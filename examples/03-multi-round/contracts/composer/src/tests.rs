use alloc::vec::Vec as StdVec;
use soroban_sdk::testutils::{Address as _, Events as _, Ledger};
use soroban_sdk::xdr::{ContractEventBody, ScSymbol, ScVal, ToXdr};
use soroban_sdk::{Address, Bytes, BytesN, Env, Vec};
use warpdrive_shared::interfaces::handler::XlmEnvelope;
use warpdrive_shared::testutils::{
    ed25519_pubkey, ed25519_sign_envelope, make_ed25519_key, Ed25519SigningKey,
};

use ed25519_security::Ed25519Security;
use ed25519_verification::Ed25519Verification;

use crate::{
    Composer, ComposerClient, ComposerError, Ed25519SignatureData, FinalPayload, Round1Bundle,
    Round1Payload, SubmissionPayload,
};

// Two distinct ledger sequences so the verifier can prove the
// reference_block (when signers were registered) is in the past
// relative to the current verification call — same convention as
// hodlers-app's test suite.
const REGISTRATION_BLOCK: u32 = 10;
const CURRENT_BLOCK: u32 = 100;

struct TestSetup<'a> {
    env: Env,
    composer: ComposerClient<'a>,
    keys: StdVec<(Ed25519SigningKey, BytesN<32>)>,
}

/// Build the wire-format envelope the off-chain circuits emit: an
/// `XlmEnvelope` whose `payload` is the XDR-serialised
/// `SubmissionPayload` enum. `event_id` varies per test to isolate
/// the per-event_id seen-set between cases — `payload` is the
/// already-built inner variant the caller wants on the wire.
fn build_envelope_bytes(env: &Env, event_id: u8, payload: SubmissionPayload) -> Bytes {
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
    // Composer takes the verification address + a quorum fraction at
    // construct time. We pass the same num/denom as the security
    // contract so the Round 1 attestation-count threshold and the
    // verifier's weight threshold move in lockstep — that's the
    // intended shape for production deploys too.
    let composer_id = env.register(
        Composer,
        (
            &verification_id,
            threshold_num as u32,
            threshold_denom as u32,
        ),
    );

    env.ledger().set_sequence_number(CURRENT_BLOCK);
    // verify_xlm dispatches into the verification module which in
    // turn reads the security module — those nested cross-contract
    // calls inherit the test's mock-auth context. `_allowing_non_root_auth`
    // is the documented escape hatch for any nested `require_auth`
    // along that path; we use it here defensively even though the
    // verifier's read-only entry points don't currently call it.
    env.mock_all_auths_allowing_non_root_auth();

    TestSetup {
        env: env.clone(),
        composer: ComposerClient::new(&env, &composer_id),
        keys,
    }
}

/// Build an `Ed25519SignatureData` from real ed25519 signatures over
/// `envelope`. The verifier rejects unordered signer lists with
/// `SignersNotOrdered`, so we sort the (sig, pubkey) pairs by pubkey
/// bytes before packing — the test doesn't care about the encoding
/// order of the input slice.
fn sign(
    env: &Env,
    envelope: &Bytes,
    keys: &[(Ed25519SigningKey, BytesN<32>)],
) -> Ed25519SignatureData {
    let envelope_vec = envelope.to_alloc_vec();
    let mut pairs: StdVec<([u8; 64], BytesN<32>)> = StdVec::new();
    for (sk, pk) in keys {
        let raw = ed25519_sign_envelope(sk, &envelope_vec);
        pairs.push((raw, pk.clone()));
    }
    pairs.sort_by(|a, b| a.1.to_array().cmp(&b.1.to_array()));

    let mut signatures: Vec<BytesN<64>> = Vec::new(env);
    let mut signers: Vec<BytesN<32>> = Vec::new(env);
    for (raw, pk) in pairs {
        signatures.push_back(BytesN::from_array(env, &raw));
        signers.push_back(pk);
    }
    Ed25519SignatureData {
        signatures,
        signers,
        reference_block: REGISTRATION_BLOCK,
    }
}

/// Returns how many `r1ready` topic events the composer has emitted,
/// matched by the static topic symbol the `#[contractevent]` macro
/// derives from `Round1Ready` (overridden to `r1ready` in
/// contract.rs). Counting rather than asserting equal-to-1 lets the
/// caller distinguish "fired once" from "fired multiple times" in
/// the threshold-latch test.
fn count_topic(env: &Env, topic_str: &str) -> usize {
    let want = ScVal::Symbol(ScSymbol(topic_str.try_into().expect("topic fits")));
    env.events()
        .all()
        .events()
        .iter()
        .filter(|ev| {
            let ContractEventBody::V0(body) = &ev.body;
            body.topics.first().map(|t| t == &want).unwrap_or(false)
        })
        .count()
}

/// Build + sign + submit a Round 1 envelope by a single signer,
/// asserting the call succeeded. Tests that need to inspect a
/// specific failure call `composer.try_verify_xlm` directly.
fn submit_round1(
    env: &Env,
    composer: &ComposerClient<'_>,
    keys: &[(Ed25519SigningKey, BytesN<32>)],
    signer_idx: usize,
    event_id: u8,
    round_id: u64,
    signer_value: u64,
) {
    let payload = SubmissionPayload::Round1(Round1Payload {
        round_id,
        signer_value,
    });
    let envelope = build_envelope_bytes(env, event_id, payload);
    let sig = sign(env, &envelope, &keys[signer_idx..=signer_idx]);
    composer.verify_xlm(&envelope, &sig);
}

#[test]
fn round1_attestations_accumulate_and_emit_r1ready() {
    // 2 signers, 1/1 quorum ⇒ Round 1 release threshold = ceil(2*1/1) = 2.
    // First submission must NOT fire `r1ready`; second submission MUST.
    let s = setup(2, 1, 1);

    submit_round1(&s.env, &s.composer, &s.keys, 0, 1, 7, 100);
    assert_eq!(
        count_topic(&s.env, "r1ready"),
        0,
        "release event must not fire before threshold"
    );

    submit_round1(&s.env, &s.composer, &s.keys, 1, 2, 7, 200);
    assert_eq!(
        count_topic(&s.env, "r1ready"),
        1,
        "release event must fire exactly once when threshold crossed"
    );

    // Bundle observable on chain — proves the (signer, value) pairs
    // were stored and would be readable by the Round 2 circuit.
    let bundle = s.composer.round1_bundle(&7).expect("bundle present");
    assert_eq!(bundle.attestations.len(), 2);
    // Final not yet stored.
    assert_eq!(s.composer.final_result(&7), None);
}

#[test]
fn round2_final_with_quorum_signs() {
    // Same 1/1, 2-signer shape; release Round 1 first (apply_final
    // gates on round1_released), then submit a Final signed by both.
    let s = setup(2, 1, 1);
    submit_round1(&s.env, &s.composer, &s.keys, 0, 1, 9, 50);
    submit_round1(&s.env, &s.composer, &s.keys, 1, 2, 9, 70);
    assert!(s.composer.round1_bundle(&9).is_some());

    let final_payload = SubmissionPayload::Final(FinalPayload {
        aggregate: 50,
        round_id: 9,
    });
    let envelope = build_envelope_bytes(&s.env, 3, final_payload);
    let sig = sign(&s.env, &envelope, &s.keys);

    s.composer.verify_xlm(&envelope, &sig);
    assert_eq!(
        count_topic(&s.env, "final"),
        1,
        "Finalized event must fire on persistence"
    );
    // Final aggregate observable via the read API.
    assert_eq!(s.composer.final_result(&9), Some(50));
}

#[test]
fn replay_rejected() {
    // Same envelope (same event_id) submitted twice — the second call
    // must trip the persistent EventSeen entry even though the
    // signatures still verify.
    let s = setup(2, 1, 1);
    let payload = SubmissionPayload::Round1(Round1Payload {
        round_id: 11,
        signer_value: 100,
    });
    let envelope = build_envelope_bytes(&s.env, 5, payload);
    let sig = sign(&s.env, &envelope, &s.keys[0..1]);

    s.composer.verify_xlm(&envelope, &sig);
    let result = s.composer.try_verify_xlm(&envelope, &sig);
    assert_eq!(result, Err(Ok(ComposerError::EventAlreadySeen)));
}

#[test]
fn final_requires_two_sigs_for_1_of_1() {
    // 1/1 with 2 signers (each weight 100) ⇒ required_weight = 200.
    // A single signature reaches only 100 ⇒ try_verify reports
    // InsufficientWeight, which the composer surfaces as
    // InsufficientQuorum. apply_final never runs, so the missing
    // Round 1 release doesn't matter for this assertion.
    let s = setup(2, 1, 1);
    let payload = SubmissionPayload::Final(FinalPayload {
        aggregate: 1,
        round_id: 13,
    });
    let envelope = build_envelope_bytes(&s.env, 6, payload);
    let sig = sign(&s.env, &envelope, &s.keys[0..1]);

    let result = s.composer.try_verify_xlm(&envelope, &sig);
    assert_eq!(result, Err(Ok(ComposerError::InsufficientQuorum)));
}

#[test]
fn round1_duplicate_signer_dedups() {
    // Same signer submits Round 1 twice for the same round_id under
    // distinct event_ids (different envelope bytes ⇒ different
    // event_id ⇒ both pass is_event_seen). The bundle MUST refuse
    // the second one with DuplicateAttestation — otherwise a buggy
    // operator could pad the bundle and skew the Round 2 reduce.
    //
    // Quorum widened to 1/2 so two signers with weight 100 each (200
    // total) yield threshold ceil(2*1/2) = 1, releasing on the
    // FIRST attestation — that lets us prove the dup-reject is the
    // bundle's own check, not a side effect of a closed-once-latched
    // bundle.
    let s = setup(2, 1, 2);

    submit_round1(&s.env, &s.composer, &s.keys, 0, 1, 21, 7);
    assert_eq!(count_topic(&s.env, "r1ready"), 1);

    // Same signer (idx 0), DIFFERENT event_id (8 vs 1), same round_id.
    let payload = SubmissionPayload::Round1(Round1Payload {
        round_id: 21,
        signer_value: 99,
    });
    let envelope = build_envelope_bytes(&s.env, 8, payload);
    let sig = sign(&s.env, &envelope, &s.keys[0..1]);
    let result = s.composer.try_verify_xlm(&envelope, &sig);
    assert_eq!(result, Err(Ok(ComposerError::DuplicateAttestation)));

    // Bundle still single-entry: the dup didn't get persisted.
    let bundle: Round1Bundle = s.composer.round1_bundle(&21).expect("bundle");
    assert_eq!(bundle.attestations.len(), 1);
    assert_eq!(bundle.attestations.get(0).unwrap().value, 7);
}
