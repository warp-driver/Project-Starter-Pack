use alloc::vec::Vec as StdVec;
use soroban_sdk::testutils::{Address as _, Ledger};
use soroban_sdk::xdr::ToXdr;
use soroban_sdk::{Address, Bytes, BytesN, Env, String, Vec};
use warpdrive_shared::interfaces::handler::XlmEnvelope;
use warpdrive_shared::testutils::{
    ed25519_pubkey, ed25519_sign_envelope, make_ed25519_key, Ed25519SigningKey,
};

use ed25519_security::Ed25519Security;
use ed25519_verification::Ed25519Verification;
use message_board::MessageBoard;

use crate::{Ed25519SignatureData, HandlerError, RecordPayload, StellarHandler, StellarHandlerClient};

// Two distinct ledger sequences so the verifier can prove the
// reference_block (when signers were registered) is in the past
// relative to the current verification call — same convention as
// hodlers-app's test suite.
const REGISTRATION_BLOCK: u32 = 10;
const CURRENT_BLOCK: u32 = 100;

struct TestSetup<'a> {
    env: Env,
    handler: StellarHandlerClient<'a>,
    message_board: message_board::MessageBoardClient<'a>,
    keys: StdVec<(Ed25519SigningKey, BytesN<32>)>,
}

/// Build the same wire-format envelope the circuit emits: an
/// `XlmEnvelope` whose `payload` is the XDR-serialised `RecordPayload`.
/// Varying `event_id` per test isolates replay-protection state
/// between cases without rebuilding the entire setup. Varying `msg_id`
/// also isolates the message-board's per-id idempotency — two
/// different envelopes with the same msg_id would trip AlreadyRecorded
/// on the second submission even with different event_ids, which we
/// don't want to entangle with the handler-level replay test.
fn build_envelope_bytes(env: &Env, event_id: u8, msg_id: u64, msg: &str) -> Bytes {
    let payload = RecordPayload {
        msg: String::from_str(env, msg),
        msg_id,
    };
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
    // `record_signed` lives one level deep — verify_xlm is the root
    // contract call, the cross-contract call into message-board
    // happens beneath it. Stock `mock_all_auths` only satisfies
    // root-level auth contexts, which makes a non-root
    // `handler.require_auth()` panic with `Auth, InvalidAction`. The
    // `_allowing_non_root_auth` variant is the documented escape
    // hatch for cross-contract require_auth in tests. We use a dummy
    // address for the handler the message-board trusts because the
    // real handler isn't deployable yet (constructor cycle); the
    // permissive mock makes the cross-call go through regardless.
    let mb_dummy_handler = Address::generate(&env);
    let mb_id = env.register(MessageBoard, (&mb_dummy_handler,));
    let handler_id = env.register(StellarHandler, (&verification_id, &mb_id));

    env.ledger().set_sequence_number(CURRENT_BLOCK);
    env.mock_all_auths_allowing_non_root_auth();

    TestSetup {
        env: env.clone(),
        handler: StellarHandlerClient::new(&env, &handler_id),
        message_board: message_board::MessageBoardClient::new(&env, &mb_id),
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
fn happy_path_verifies_and_records_message() {
    let s = setup(2, 55, 100);
    let envelope = build_envelope_bytes(&s.env, 1, 42, "hello");
    let sig = sign(&s.env, &envelope, &s.keys);

    s.handler.verify_xlm(&envelope, &sig);

    // Full pipeline observable through message-board state — proves
    // the envelope decoded, signatures verified, payload re-decoded,
    // and the cross-contract record_signed landed. Asserting the
    // string round-trips end-to-end catches any XDR field-order drift
    // between circuit and handler.
    assert_eq!(s.message_board.recorded(&42), Some(String::from_str(&s.env, "hello")));
}

#[test]
fn replay_is_rejected() {
    let s = setup(2, 55, 100);
    let envelope = build_envelope_bytes(&s.env, 1, 42, "hello");
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
    let envelope = build_envelope_bytes(&s.env, 2, 43, "hello");
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
