use alloc::vec::Vec as StdVec;
use soroban_sdk::testutils::{Address as _, Ledger};
use soroban_sdk::xdr::ToXdr;
use soroban_sdk::{Address, Bytes, BytesN, Env, Vec};
use warpdrive_shared::interfaces::handler::XlmEnvelope;
use warpdrive_shared::testutils::{
    ed25519_pubkey, ed25519_sign_envelope, make_ed25519_key, Ed25519SigningKey,
};

use counter::CounterContract;
use ed25519_security::Ed25519Security;
use ed25519_verification::Ed25519Verification;

use crate::{Ed25519SignatureData, HandlerError, StellarHandler, StellarHandlerClient, TickPayload};

// Two distinct ledger sequences so the verifier can prove the
// reference_block (when signers were registered) is in the past
// relative to the current verification call — same convention as
// hodlers-app's test suite.
const REGISTRATION_BLOCK: u32 = 10;
const CURRENT_BLOCK: u32 = 100;

struct TestSetup<'a> {
    env: Env,
    handler: StellarHandlerClient<'a>,
    counter: counter::CounterContractClient<'a>,
    keys: StdVec<(Ed25519SigningKey, BytesN<32>)>,
}

/// Build the same wire-format envelope the circuit emits: an
/// `XlmEnvelope` whose `payload` is the XDR-serialised `TickPayload`.
/// Varying `event_id` per test isolates replay-protection state
/// between cases without rebuilding the entire setup.
fn build_envelope_bytes(env: &Env, event_id: u8, ts: u64) -> Bytes {
    let payload = TickPayload { ts };
    let payload_bytes = payload.to_xdr(env);

    let mut event_id_bytes = [0u8; 20];
    event_id_bytes[19] = event_id;

    let envelope = XlmEnvelope {
        event_id: BytesN::from_array(env, &event_id_bytes),
        ordering: BytesN::from_array(env, &[0u8; 12]),
        payload: payload_bytes,
    };
    envelope.to_xdr(env)
}

fn setup(num_signers: usize, threshold_num: u64, threshold_denom: u64) -> TestSetup<'static> {
    let env = Env::default();
    let admin = Address::generate(&env);

    // Register signers at REGISTRATION_BLOCK so the verification call
    // at CURRENT_BLOCK sees them as already-active.
    env.ledger().set_sequence_number(REGISTRATION_BLOCK);

    let security_id = env.register(Ed25519Security, (&admin, threshold_num, threshold_denom));
    let security = ed25519_security::Ed25519SecurityClient::new(&env, &security_id);

    let mut keys: StdVec<(Ed25519SigningKey, BytesN<32>)> = StdVec::new();
    for i in 0..num_signers {
        let sk = make_ed25519_key((i as u8) + 1);
        let pk = ed25519_pubkey(&env, &sk);
        // mock_all_auths so the security contract accepts admin writes
        // without a separate sign step — admin auth isn't what we're
        // testing here.
        env.mock_all_auths();
        security.add_signer(&pk, &100);
        keys.push((sk, pk));
    }

    let verification_id = env.register(Ed25519Verification, (&admin, &security_id));
    // Counter takes the handler address in its constructor — but the
    // handler hasn't been deployed yet. Register the handler first
    // with a placeholder, then re-register the counter with the real
    // handler id? Simpler: register counter pointing at where the
    // handler will be (Soroban's `env.register` is deterministic in
    // tests, but easier to flip the order: deploy counter against a
    // dummy address, then mock_all_auths covers cross-contract auth
    // in the verify_xlm path).
    //
    // We pick the second approach: counter trusts a generated dummy
    // address; `env.mock_all_auths` (re-armed below) makes
    // `handler.require_auth()` inside counter accept the cross-contract
    // call regardless of which address Soroban thinks initiated it.
    let counter_dummy_handler = Address::generate(&env);
    let counter_id = env.register(CounterContract, (&counter_dummy_handler,));
    let handler_id = env.register(StellarHandler, (&verification_id, &counter_id));

    env.ledger().set_sequence_number(CURRENT_BLOCK);
    // Re-arm so verify_xlm's nested counter.tick auth is satisfied.
    env.mock_all_auths();

    TestSetup {
        env: env.clone(),
        handler: StellarHandlerClient::new(&env, &handler_id),
        counter: counter::CounterContractClient::new(&env, &counter_id),
        keys,
    }
}

/// Build a real Ed25519SignatureData by signing the wire bytes with
/// every key in `keys`. The verifier checks each signature against the
/// signer set as of `reference_block`.
fn sign(env: &Env, envelope: &Bytes, keys: &[(Ed25519SigningKey, BytesN<32>)]) -> Ed25519SignatureData {
    let envelope_vec = envelope.to_alloc_vec();
    let mut signatures: Vec<BytesN<64>> = Vec::new(env);
    let mut signers: Vec<BytesN<32>> = Vec::new(env);
    for (sk, pk) in keys {
        let raw = ed25519_sign_envelope(sk, &envelope_vec);
        signatures.push_back(BytesN::from_array(env, &raw));
        signers.push_back(pk.clone());
    }
    Ed25519SignatureData {
        signatures,
        signers,
        reference_block: REGISTRATION_BLOCK,
    }
}

#[test]
fn happy_path_verifies_and_ticks_counter() {
    let s = setup(2, 55, 100);
    let envelope = build_envelope_bytes(&s.env, 1, 1_700_000_000);
    let sig = sign(&s.env, &envelope, &s.keys);

    s.handler.verify_xlm(&envelope, &sig);

    // Full pipeline observable through counter state — proves the
    // envelope decoded, signatures verified, payload re-decoded, and
    // the cross-contract tick landed.
    assert_eq!(s.counter.count(), 1);
    assert_eq!(s.counter.last_tick(), 1_700_000_000);
}

#[test]
fn replay_is_rejected() {
    let s = setup(2, 55, 100);
    let envelope = build_envelope_bytes(&s.env, 1, 1_700_000_000);
    let sig = sign(&s.env, &envelope, &s.keys);

    s.handler.verify_xlm(&envelope, &sig);
    // Same event_id ⇒ EventAlreadySeen on the second call, even though
    // the signatures are still valid. The persistent EventSeen entry
    // is what blocks it.
    let result = s.handler.try_verify_xlm(&envelope, &sig);
    assert_eq!(result, Err(Ok(HandlerError::EventAlreadySeen)));
}

#[test]
fn insufficient_quorum_rejected() {
    // 2 signers, 55% threshold ⇒ need both signatures. Submit only one.
    let s = setup(2, 55, 100);
    let envelope = build_envelope_bytes(&s.env, 2, 1_700_000_000);
    let sig = sign(&s.env, &envelope, &s.keys[..1]);

    let result = s.handler.try_verify_xlm(&envelope, &sig);
    assert_eq!(result, Err(Ok(HandlerError::InsufficientWeight)));
}

#[test]
fn malformed_envelope_rejected() {
    // Valid XDR but the wrong shape — `from_xdr::<XlmEnvelope>`
    // returns ConversionError, which we surface as InvalidEnvelope.
    // Bytes that are not valid XDR at all panic in the host before our
    // error mapping runs, so we deliberately test only the in-shape
    // failure here (same caveat as hodlers-app's test suite).
    let s = setup(2, 55, 100);
    let envelope = 7u32.to_xdr(&s.env);
    let sig = Ed25519SignatureData {
        signatures: Vec::new(&s.env),
        signers: Vec::new(&s.env),
        reference_block: REGISTRATION_BLOCK,
    };

    let result = s.handler.try_verify_xlm(&envelope, &sig);
    assert_eq!(result, Err(Ok(HandlerError::InvalidEnvelope)));
}
