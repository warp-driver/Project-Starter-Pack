//! Composer contract — the one on-chain endpoint for the multi-round
//! WarpDrive demo.
//!
//! Round flow:
//!
//! 1. A cron tick fires every 30 s. Each Vectr's *Round 1* circuit
//!    samples its wall clock, encodes a `Round1Payload { round_id,
//!    signer_value }`, wraps it as `SubmissionPayload::Round1`, and
//!    submits a SINGLE-signer envelope. The composer routes on the
//!    enum variant, validates the lone signer via
//!    `Ed25519VerificationClient::check_one` (which only checks the
//!    sig is from a registered signer with non-zero weight — exact-
//!    match attestations would be impossible because every operator's
//!    wall clock differs), and appends to a per-round bundle. When the
//!    bundle reaches the configured quorum threshold the composer
//!    latches a one-shot `Round1Released` flag and emits a
//!    `Round1Ready` Soroban event carrying the full bundle — the
//!    composition event the *Round 2* circuits listen on.
//!
//! 2. Each Vectr's *Round 2* circuit decodes the bundle, reduces it
//!    (this demo: `min`), and submits a QUORUM-signed envelope
//!    carrying `SubmissionPayload::Final { round_id, aggregate }`.
//!    The composer validates via `Ed25519VerificationClient::verify`
//!    (full sum-of-weights threshold) and stores the aggregate.
//!
//! See `Composer::verify_xlm` for the dispatcher and `apply_round1`
//! / `apply_final` for the per-variant state transitions.

use soroban_sdk::{
    contract, contracterror, contractevent, contractimpl, contracttype, xdr::FromXdr, Address,
    Bytes, BytesN, Env, String, Vec,
};
use warpdrive_shared::interfaces::{
    handler::{Ed25519SignatureData, XlmEnvelope},
    verification::Ed25519VerificationClient,
};

use crate::storage;

// ─── error type ───────────────────────────────────────────────────────

#[contracterror]
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
#[repr(u32)]
pub enum ComposerError {
    QuorumOutOfRange = 1,
    InvalidEnvelope = 2,
    EventAlreadySeen = 3,
    LengthMismatch = 4,
    UnknownVerificationError = 5,
    OtherInvocationError = 6,
    SignerNotRegistered = 7,
    InsufficientQuorum = 8,
    DuplicateAttestation = 9,
    AlreadyFinalized = 10,
    Round1NotReady = 11,
    Round1MismatchedId = 12,
}

impl From<warpdrive_shared::interfaces::verification::VerifyError> for ComposerError {
    fn from(e: warpdrive_shared::interfaces::verification::VerifyError) -> Self {
        use warpdrive_shared::interfaces::verification::VerifyError as V;
        match e {
            V::SignerNotRegistered => ComposerError::SignerNotRegistered,
            V::InsufficientWeight => ComposerError::InsufficientQuorum,
            V::LengthMismatch => ComposerError::LengthMismatch,
            // The remaining VerifyError variants (InvalidSignature,
            // EmptySignatures, SignersNotOrdered, ZeroRequiredWeight)
            // collapse to a single bucket — the composer can't act on
            // them differently and the off-chain operator only needs
            // "the verifier said no". Detailed diagnostics live in
            // the verifier's own event log.
            _ => ComposerError::UnknownVerificationError,
        }
    }
}

// ─── public payload types ─────────────────────────────────────────────
//
// Field declaration order is alphabetical INSIDE each struct because
// Soroban's `#[contracttype]` derive sorts ScMap entries by key when
// encoding to XDR. The off-chain circuits hand-build the matching
// ScMap and MUST emit entries in the same alphabetical order, or the
// contract's `from_xdr` rejects with `InvalidEnvelope`. Adding a
// field here means adding it to the circuit's map at the same time
// AND keeping the keys sorted.

/// The XDR payload a Vectr's Round 1 circuit emits, wrapped in
/// `SubmissionPayload::Round1` before the host signs the surrounding
/// `XlmEnvelope`.
#[contracttype]
#[derive(Clone)]
pub struct Round1Payload {
    /// Tick id, derived deterministically from the cron schedule
    /// (`trigger_time.nanos / 30_000_000_000`) so every operator
    /// agrees on which round this submission belongs to.
    pub round_id: u64,
    /// This operator's per-tick value — wall-clock-derived, so it
    /// differs across operators by design. The off-chain submission
    /// manager MUST NOT quorum-collapse two Vectrs' Round 1 payloads;
    /// the circuit uses the payload bytes themselves as the
    /// `event_id_salt`, guaranteeing distinct event_ids.
    pub signer_value: u64,
}

/// The XDR payload a Vectr's Round 2 circuit emits, wrapped in
/// `SubmissionPayload::Final` for quorum submission. Bytes are
/// byte-identical across operators (every operator runs the same
/// reduce over the same on-chain bundle), so the host's QuorumQueue
/// collapses N envelopes into one with N signatures.
#[contracttype]
#[derive(Clone)]
pub struct FinalPayload {
    /// Reduce of the Round 1 bundle — this demo uses `min(values)`
    /// because it is trivially deterministic (no overflow, no
    /// rounding choices), which keeps "what went wrong" cheap to
    /// diagnose when the multi-round pattern is the thing under
    /// study.
    pub aggregate: u64,
    pub round_id: u64,
}

/// One Vectr's Round 1 contribution as recorded on chain — what's
/// bundled into the `Round1Ready` event for the Round 2 circuits.
/// We do NOT store the envelope or signature alongside: the composer
/// already verified the sig at the door (via `check_one`) and the
/// only thing Round 2 needs is the (signer, value) pair to feed its
/// reduce. Trimming the bundle keeps the composition event payload
/// small and dodges the cost of re-decoding ed25519 material in the
/// downstream circuit.
#[contracttype]
#[derive(Clone)]
pub struct Round1Attestation {
    pub signer: BytesN<32>,
    pub value: u64,
}

#[contracttype]
#[derive(Clone)]
pub struct Round1Bundle {
    pub attestations: Vec<Round1Attestation>,
}

/// Tagged payload the off-chain Vectrs put inside the signed
/// envelope. The warpdrive node's submission manager always invokes
/// `verify_xlm` on the handler — there is no per-round entrypoint to
/// route on — so we wrap each round's struct in a tagged enum and
/// dispatch on the variant inside the composer.
#[contracttype]
#[derive(Clone)]
pub enum SubmissionPayload {
    /// Single-Vectr Round 1 attestation.
    Round1(Round1Payload),
    /// Quorum-signed Round 2 aggregate.
    Final(FinalPayload),
}

// ─── events ───────────────────────────────────────────────────────────

/// Composition event emitted exactly once per round, when the
/// attestation bundle first crosses the release threshold. Topic
/// `"r1ready"` is the symbol the Round 2 circuits filter on; the
/// `round_id` topic lets a subscriber replay a specific round's
/// composition without scanning the full event log. The `Round1Bundle`
/// rides in the event data so downstream circuits can fold it without
/// a separate ledger read.
#[contractevent(topics = ["r1ready"], data_format = "single-value")]
pub struct Round1Ready {
    #[topic]
    pub round_id: u64,
    pub bundle: Round1Bundle,
}

/// Terminal event for a round — emitted after the quorum-signed Final
/// envelope lands and `Final(round_id)` is persisted. Topic `"final"`
/// + `round_id` so a subscriber can dedup against a specific round
/// without re-scanning.
#[contractevent(topics = ["final"], data_format = "single-value")]
pub struct Finalized {
    #[topic]
    pub round_id: u64,
    pub aggregate: u64,
}

// ─── contract ─────────────────────────────────────────────────────────

#[contract]
pub struct Composer;

#[contractimpl]
impl Composer {
    /// Wires the composer to the project's ed25519 verification
    /// module and sets the initial quorum fraction. The default for
    /// the 2-operator demo is `1/1` — full quorum, both operators
    /// MUST sign each Final — but the constructor takes the fraction
    /// explicitly so the same contract code runs the larger-swarm
    /// integration suites.
    pub fn __constructor(
        env: Env,
        verification_contract: Address,
        quorum_numerator: u32,
        quorum_denominator: u32,
    ) -> Result<(), ComposerError> {
        if quorum_denominator == 0
            || quorum_numerator == 0
            || quorum_numerator > quorum_denominator
        {
            return Err(ComposerError::QuorumOutOfRange);
        }
        storage::set_verification_contract(&env, &verification_contract);
        storage::set_quorum(&env, quorum_numerator, quorum_denominator);
        storage::set_version(&env, &String::from_str(&env, env!("CARGO_PKG_VERSION")));
        storage::extend_instance_ttl(&env);
        Ok(())
    }

    /// `StellarHandlerInterface::verify_xlm` — the single entry point
    /// the warpdrive node's submission manager invokes for every
    /// aggregated submission. The envelope payload is a tagged
    /// `SubmissionPayload` whose variant decides whether this is a
    /// per-Vectr Round 1 attestation (single-signer, `check_one`) or
    /// a quorum-signed Round 2 final (`verify`).
    ///
    /// We decode the payload BEFORE running the crypto check so we
    /// know which check the variant wants — Round 1 envelopes are
    /// single-signer and would fail a full `verify` even when valid.
    pub fn verify_xlm(
        env: Env,
        envelope_bytes: Bytes,
        sig_data: Ed25519SignatureData,
    ) -> Result<(), ComposerError> {
        let envelope = XlmEnvelope::from_xdr(&env, &envelope_bytes)
            .map_err(|_| ComposerError::InvalidEnvelope)?;
        let event_id = envelope.event_id.clone();
        if storage::is_event_seen(&env, &event_id) {
            return Err(ComposerError::EventAlreadySeen);
        }

        let payload = SubmissionPayload::from_xdr(&env, &envelope.payload)
            .map_err(|_| ComposerError::InvalidEnvelope)?;
        let verification_addr = storage::get_verification_contract(&env);
        let verification = Ed25519VerificationClient::new(&env, &verification_addr);

        match payload {
            SubmissionPayload::Round1(p) => {
                // Single-signer envelope: enforce the shape before we
                // try to pull the lone signer/signature out.
                if sig_data.signatures.len() != 1 || sig_data.signers.len() != 1 {
                    return Err(ComposerError::LengthMismatch);
                }
                let signer = sig_data.signers.get(0).expect("len==1");
                let signature = sig_data.signatures.get(0).expect("len==1");
                // try_check_one returns Result<Result<weight, VerifyError>, InvokeError>:
                // outer = host-level invocation failure, inner = the
                // verifier's typed error. Flatten both to ComposerError.
                match verification.try_check_one(
                    &envelope_bytes,
                    &signature,
                    &signer,
                    &Some(sig_data.reference_block),
                ) {
                    Ok(Ok(_weight)) => {}
                    Ok(Err(_)) => return Err(ComposerError::UnknownVerificationError),
                    Err(Ok(e)) => return Err(ComposerError::from(e)),
                    Err(Err(_)) => return Err(ComposerError::OtherInvocationError),
                }
                Self::apply_round1(&env, &verification_addr, &signer, &event_id, p)
            }
            SubmissionPayload::Final(p) => {
                // Sum-of-weights ≥ required_weight at reference_block.
                match verification.try_verify(
                    &envelope_bytes,
                    &sig_data.signatures,
                    &sig_data.signers,
                    &sig_data.reference_block,
                ) {
                    Ok(Ok(())) => {}
                    Ok(Err(_)) => return Err(ComposerError::UnknownVerificationError),
                    Err(Ok(e)) => return Err(ComposerError::from(e)),
                    Err(Err(_)) => return Err(ComposerError::OtherInvocationError),
                }
                Self::apply_final(&env, &event_id, p)
            }
        }
    }

    // ── internal round 1 logic ────────────────────────────────────────

    fn apply_round1(
        env: &Env,
        verification_addr: &Address,
        signer: &BytesN<32>,
        event_id: &BytesN<20>,
        payload: Round1Payload,
    ) -> Result<(), ComposerError> {
        // A Final for this round already landed? The bundle is
        // immutable from then on — late Round 1 attestations would
        // race the finalised state.
        if storage::get_final(env, payload.round_id).is_some() {
            return Err(ComposerError::AlreadyFinalized);
        }

        // One attestation per signer per round. Catches a buggy
        // operator that retries with a fresh event_id (which would
        // pass the seen-set check).
        let mut bundle = storage::load_bundle(env, payload.round_id);
        for existing in bundle.attestations.iter() {
            if &existing.signer == signer {
                return Err(ComposerError::DuplicateAttestation);
            }
        }
        bundle.attestations.push_back(Round1Attestation {
            signer: signer.clone(),
            value: payload.signer_value,
        });
        storage::save_bundle(env, payload.round_id, &bundle);
        storage::mark_event_seen(env, event_id);
        storage::extend_instance_ttl(env);

        // Threshold release: once the bundle hits (or passes)
        // ceil(total_signers · num/denom), emit `Round1Ready` exactly
        // once. The latch is the gatekeeper, not the equality check —
        // a late attestation that pushes from N to N+1 must NOT
        // re-fire the composition event and trigger the Round 2
        // circuits twice.
        if !storage::round1_released(env, payload.round_id) {
            let total = security_signer_count(env, verification_addr);
            let threshold = ceil_div(
                total * storage::get_quorum_numerator(env),
                storage::get_quorum_denominator(env),
            );
            if bundle.attestations.len() >= threshold {
                storage::mark_round1_released(env, payload.round_id);
                Round1Ready {
                    round_id: payload.round_id,
                    bundle: bundle.clone(),
                }
                .publish(env);
            }
        }
        Ok(())
    }

    // ── internal final logic ──────────────────────────────────────────

    fn apply_final(
        env: &Env,
        event_id: &BytesN<20>,
        payload: FinalPayload,
    ) -> Result<(), ComposerError> {
        // Final MUST follow a released Round 1 — otherwise we'd be
        // accepting an aggregate over a bundle no one has seen on
        // chain. The off-chain Round 2 circuit gates on the
        // composition event, so this should never fire in a healthy
        // pipeline; treating it as an error here makes the failure
        // loud if some operator skips ahead.
        if !storage::round1_released(env, payload.round_id) {
            return Err(ComposerError::Round1NotReady);
        }
        if storage::get_final(env, payload.round_id).is_some() {
            return Err(ComposerError::AlreadyFinalized);
        }
        // The bundle exists because round1_released ⇒ at least one
        // attestation was pushed; an empty bundle here would mean
        // storage corruption.
        let bundle = storage::load_bundle(env, payload.round_id);
        if bundle.attestations.is_empty() {
            return Err(ComposerError::Round1MismatchedId);
        }

        storage::save_final(env, payload.round_id, payload.aggregate);
        // Mark seen AFTER apply_final's persistence so a panic earlier
        // doesn't latch the event_id — the operator can retry the
        // same envelope.
        storage::mark_event_seen(env, event_id);
        storage::extend_instance_ttl(env);

        Finalized {
            round_id: payload.round_id,
            aggregate: payload.aggregate,
        }
        .publish(env);
        Ok(())
    }

    // ── reads ─────────────────────────────────────────────────────────

    pub fn verification_contract(env: Env) -> Address {
        storage::get_verification_contract(&env)
    }

    pub fn final_result(env: Env, round_id: u64) -> Option<u64> {
        storage::get_final(&env, round_id)
    }

    pub fn round1_bundle(env: Env, round_id: u64) -> Option<Round1Bundle> {
        let b = storage::load_bundle(&env, round_id);
        if b.attestations.is_empty() {
            None
        } else {
            Some(b)
        }
    }

    pub fn quorum(env: Env) -> (u32, u32) {
        (
            storage::get_quorum_numerator(&env),
            storage::get_quorum_denominator(&env),
        )
    }

    /// Required by `StellarHandlerInterface` — project-root uses this
    /// to recognise the contract as a handler. We don't persist
    /// payloads (operators always have them in the circuit's output),
    /// so we return None and let the runtime fall back to its own
    /// caches.
    pub fn payload(_env: Env, _event_id: BytesN<20>) -> Option<Bytes> {
        None
    }
}

// ─── helpers ──────────────────────────────────────────────────────────

fn ceil_div(num: u32, denom: u32) -> u32 {
    debug_assert!(denom > 0);
    (num + denom - 1) / denom
}

/// Count the registered signers the verification module trusts.
/// Routes through the verification module's own client so the
/// composer never needs to know the security address directly — that
/// indirection lets ops rotate the security contract by re-pointing
/// the verifier without redeploying the composer.
fn security_signer_count(env: &Env, verification: &Address) -> u32 {
    use warpdrive_shared::interfaces::security::Ed25519SecurityClient;
    let security_addr = Ed25519VerificationClient::new(env, verification).security_contract();
    Ed25519SecurityClient::new(env, &security_addr)
        .list_signers()
        .len() as u32
}
