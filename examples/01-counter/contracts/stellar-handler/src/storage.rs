use soroban_sdk::{contracttype, Address, BytesN, Env, String};

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
    /// the verifier means redeploying the handler so the audit trail
    /// reflects a configuration change.
    VerificationContract,
    /// Address of the downstream `counter` contract we relay to once
    /// the signature check passes.
    CounterContract,
    /// CARGO_PKG_VERSION snapshot captured at deploy. Read by tooling
    /// to confirm which build is live on-chain without trusting the
    /// deployer's notes.
    Version,
    /// Replay-protection set: any 20-byte `event_id` that's already
    /// been processed. Persistent so it survives instance archival;
    /// `EventSeen` is the only state we keep beyond the small
    /// instance-storage configuration.
    EventSeen(BytesN<20>),
}

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

pub fn set_counter_contract(env: &Env, addr: &Address) {
    env.storage()
        .instance()
        .set(&DataKey::CounterContract, addr);
}

pub fn get_counter_contract(env: &Env) -> Address {
    env.storage()
        .instance()
        .get(&DataKey::CounterContract)
        .expect("counter contract not set")
}

pub fn set_version(env: &Env, v: &String) {
    env.storage().instance().set(&DataKey::Version, v);
}

pub fn is_event_seen(env: &Env, event_id: &BytesN<20>) -> bool {
    env.storage()
        .persistent()
        .has(&DataKey::EventSeen(event_id.clone()))
}

/// Persist the event_id with a 30-day TTL. Why persistent and not
/// temporary: the operator quorum can re-broadcast a signed envelope
/// at any time within the chain's history, and we MUST refuse it on
/// every re-submission, not just within a short window.
pub fn mark_event_seen(env: &Env, event_id: &BytesN<20>) {
    let key = DataKey::EventSeen(event_id.clone());
    env.storage().persistent().set(&key, &true);
    env.storage().persistent().extend_ttl(
        &key,
        PERSISTENT_RENEWAL_THRESHOLD,
        PERSISTENT_TARGET_TTL,
    );
}

pub fn extend_instance_ttl(env: &Env) {
    env.storage()
        .instance()
        .extend_ttl(INSTANCE_RENEWAL_THRESHOLD, INSTANCE_TARGET_TTL);
}
