//! Persistent + instance storage for the composer.
//!
//! Split deliberately:
//!
//! - **Instance** holds singleton config (the address of the ed25519
//!   verification module, the current quorum fraction, the pinned
//!   build version). The instance footprint is tiny so we extend its
//!   TTL on every mutation.
//!
//! - **Persistent** holds per-round state — the Round 1 attestation
//!   bundle, the one-shot release latch, and the final aggregate.
//!   Each entry uses a uniquely-keyed `DataKey` variant so we never
//!   need a `Map<round_id, T>` (which would pull every key into a
//!   single read on access — bounded keys keep reads O(1) as the
//!   round set grows).
//!
//! - **Replay protection** uses the same per-`event_id` key pattern
//!   as the reference `stellar-handler` so a re-broadcast of an
//!   already-processed Round 1 attestation or Final envelope is
//!   refused at the door.

use soroban_sdk::{contracttype, Address, BytesN, Env, String, Vec};

use crate::contract::{Round1Attestation, Round1Bundle};

// Inlined TTL constants — same values warpdrive-shared::ttl exports;
// inlined so this crate stays standalone and copy-paste-friendly.
const DAY_IN_LEDGERS: u32 = 17_280;
const INSTANCE_TARGET_TTL: u32 = 7 * DAY_IN_LEDGERS;
const INSTANCE_RENEWAL_THRESHOLD: u32 = INSTANCE_TARGET_TTL - DAY_IN_LEDGERS;
const PERSISTENT_TARGET_TTL: u32 = 30 * DAY_IN_LEDGERS;
const PERSISTENT_RENEWAL_THRESHOLD: u32 = PERSISTENT_TARGET_TTL - DAY_IN_LEDGERS;

#[contracttype]
pub enum DataKey {
    /// Address of the `ed25519-verification` contract — performs the
    /// quorum check on every envelope. Pinned at deploy time; rotating
    /// the verifier means redeploying the composer so the audit trail
    /// reflects a configuration change.
    VerificationContract,
    /// CARGO_PKG_VERSION snapshot captured at deploy. Read by tooling
    /// to confirm which build is live on-chain without trusting the
    /// deployer's notes.
    Version,
    /// Quorum fraction (`num/denom`) applied to BOTH the Round 1
    /// attestation-count threshold and the off-chain verifier's
    /// weight threshold. Stored split because Soroban instance keys
    /// are scalar and we want each tunable individually upgradable.
    QuorumNumerator,
    QuorumDenominator,
    /// Replay-protection set: any 20-byte `event_id` that's already
    /// been processed. Persistent so it survives instance archival;
    /// `EventSeen` is the only state we keep beyond instance config
    /// and per-round bundles.
    EventSeen(BytesN<20>),
    /// Round 1 bundle for `round_id` — accumulates one
    /// `Round1Attestation` per registered signer.
    Attestations(u64),
    /// One-shot latch: `true` once the `Round1Ready` composition event
    /// has been emitted for this `round_id`. Prevents a late
    /// attestation that pushes the bundle past threshold from firing
    /// the event a second time and confusing the Round 2 circuits.
    Round1Released(u64),
    /// Final aggregate for `round_id`, written by the Round 2 path
    /// after `try_verify` accepts the quorum-signed envelope.
    Final(u64),
}

// ─── instance ─────────────────────────────────────────────────────────

pub fn set_verification_contract(env: &Env, addr: &Address) {
    env.storage()
        .instance()
        .set(&DataKey::VerificationContract, addr);
}

pub fn get_verification_contract(env: &Env) -> Address {
    env.storage()
        .instance()
        .get(&DataKey::VerificationContract)
        .expect("verification contract not set")
}

pub fn set_version(env: &Env, v: &String) {
    env.storage().instance().set(&DataKey::Version, v);
}

pub fn set_quorum(env: &Env, num: u32, denom: u32) {
    env.storage().instance().set(&DataKey::QuorumNumerator, &num);
    env.storage()
        .instance()
        .set(&DataKey::QuorumDenominator, &denom);
}

pub fn get_quorum_numerator(env: &Env) -> u32 {
    env.storage()
        .instance()
        .get(&DataKey::QuorumNumerator)
        .expect("quorum numerator not set")
}

pub fn get_quorum_denominator(env: &Env) -> u32 {
    env.storage()
        .instance()
        .get(&DataKey::QuorumDenominator)
        .expect("quorum denominator not set")
}

pub fn extend_instance_ttl(env: &Env) {
    env.storage()
        .instance()
        .extend_ttl(INSTANCE_RENEWAL_THRESHOLD, INSTANCE_TARGET_TTL);
}

// ─── replay protection ────────────────────────────────────────────────

pub fn is_event_seen(env: &Env, event_id: &BytesN<20>) -> bool {
    env.storage()
        .persistent()
        .has(&DataKey::EventSeen(event_id.clone()))
}

/// Persist the event_id with a 30-day TTL. Persistent (not temporary)
/// because the operator quorum can re-broadcast a signed envelope at
/// any time within the chain's history, and we MUST refuse it on every
/// re-submission — not just within a short window.
pub fn mark_event_seen(env: &Env, event_id: &BytesN<20>) {
    let key = DataKey::EventSeen(event_id.clone());
    env.storage().persistent().set(&key, &true);
    env.storage().persistent().extend_ttl(
        &key,
        PERSISTENT_RENEWAL_THRESHOLD,
        PERSISTENT_TARGET_TTL,
    );
}

// ─── round 1 attestation bundle ───────────────────────────────────────

pub fn load_bundle(env: &Env, id: u64) -> Round1Bundle {
    env.storage()
        .persistent()
        .get(&DataKey::Attestations(id))
        .unwrap_or_else(|| Round1Bundle {
            attestations: Vec::<Round1Attestation>::new(env),
        })
}

pub fn save_bundle(env: &Env, id: u64, bundle: &Round1Bundle) {
    let key = DataKey::Attestations(id);
    env.storage().persistent().set(&key, bundle);
    env.storage().persistent().extend_ttl(
        &key,
        PERSISTENT_RENEWAL_THRESHOLD,
        PERSISTENT_TARGET_TTL,
    );
}

pub fn round1_released(env: &Env, id: u64) -> bool {
    env.storage()
        .persistent()
        .get(&DataKey::Round1Released(id))
        .unwrap_or(false)
}

pub fn mark_round1_released(env: &Env, id: u64) {
    let key = DataKey::Round1Released(id);
    env.storage().persistent().set(&key, &true);
    env.storage().persistent().extend_ttl(
        &key,
        PERSISTENT_RENEWAL_THRESHOLD,
        PERSISTENT_TARGET_TTL,
    );
}

// ─── final results ────────────────────────────────────────────────────

pub fn save_final(env: &Env, id: u64, aggregate: u64) {
    let key = DataKey::Final(id);
    env.storage().persistent().set(&key, &aggregate);
    env.storage().persistent().extend_ttl(
        &key,
        PERSISTENT_RENEWAL_THRESHOLD,
        PERSISTENT_TARGET_TTL,
    );
}

pub fn get_final(env: &Env, id: u64) -> Option<u64> {
    env.storage().persistent().get(&DataKey::Final(id))
}
