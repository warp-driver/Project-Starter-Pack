use soroban_sdk::{contracttype, Address, Env, String};

// TTL constants inlined (instead of importing `warpdrive_shared::ttl`)
// so this crate stays standalone — a developer can lift just the
// `message-board/` directory into their own project and build it
// without pulling the WarpDrive ecosystem. Same numbers warpdrive-shared
// uses.
const DAY_IN_LEDGERS: u32 = 17_280;
const INSTANCE_TARGET_TTL: u32 = 7 * DAY_IN_LEDGERS;
const INSTANCE_RENEWAL_THRESHOLD: u32 = INSTANCE_TARGET_TTL - DAY_IN_LEDGERS;
const PERSISTENT_TARGET_TTL: u32 = 30 * DAY_IN_LEDGERS;
const PERSISTENT_RENEWAL_THRESHOLD: u32 = PERSISTENT_TARGET_TTL - DAY_IN_LEDGERS;

#[contracttype]
pub enum DataKey {
    /// Address of the stellar-handler contract — the only caller we
    /// honour for `record_signed`. Set once in `__constructor` and
    /// never rewritten; rotating the handler would mean redeploying
    /// the message-board (intentional: keeps the trust chain auditable
    /// from the on-chain history alone).
    Handler,
    /// Monotonically increasing id handed out by `publish`. Lives in
    /// instance storage because it's a single small value the contract
    /// reads on every publish — cheaper than a persistent fetch.
    NextMsgId,
    /// `msg_id -> msg` mapping. Persistent because the recorded
    /// message is the canonical output of the whole quorum pipeline
    /// and MUST survive instance archival. Each entry lives ~30 days
    /// past last touch, which is plenty for a demo and easy to bump
    /// on read with `extend_ttl`.
    Record(u64),
}

pub fn set_handler(env: &Env, addr: &Address) {
    env.storage().instance().set(&DataKey::Handler, addr);
}

pub fn get_handler(env: &Env) -> Address {
    env.storage()
        .instance()
        .get(&DataKey::Handler)
        .expect("handler not set — __constructor must have run")
}

/// Atomically claim the next id and persist the bumped counter. Callers
/// (only `publish`) get back a value they can safely treat as unique
/// for the lifetime of the contract.
pub fn next_msg_id(env: &Env) -> u64 {
    // `+ 1` not `checked_add`: with overflow-checks=true a host panic
    // is the right failure mode — at one publish per second this would
    // take ~580 billion years to wrap, so reaching it means something
    // else is profoundly wrong and we want the tx to abort.
    let id = peek_next_msg_id(env);
    env.storage()
        .instance()
        .set(&DataKey::NextMsgId, &(id + 1));
    id
}

pub fn peek_next_msg_id(env: &Env) -> u64 {
    env.storage()
        .instance()
        .get(&DataKey::NextMsgId)
        .unwrap_or(0)
}

pub fn has_record(env: &Env, msg_id: u64) -> bool {
    env.storage().persistent().has(&DataKey::Record(msg_id))
}

pub fn get_record(env: &Env, msg_id: u64) -> Option<String> {
    let key = DataKey::Record(msg_id);
    let value: Option<String> = env.storage().persistent().get(&key);
    // Touch the entry on successful reads so a frequently-queried
    // record stays live without a write. Soroban silently ignores
    // extend_ttl on a missing entry, so the guard isn't strictly
    // necessary, but skipping the host call when there's nothing to
    // extend is the boring win.
    if value.is_some() {
        env.storage().persistent().extend_ttl(
            &key,
            PERSISTENT_RENEWAL_THRESHOLD,
            PERSISTENT_TARGET_TTL,
        );
    }
    value
}

pub fn set_record(env: &Env, msg_id: u64, msg: &String) {
    let key = DataKey::Record(msg_id);
    env.storage().persistent().set(&key, msg);
    env.storage().persistent().extend_ttl(
        &key,
        PERSISTENT_RENEWAL_THRESHOLD,
        PERSISTENT_TARGET_TTL,
    );
}

/// Bump the instance TTL on every state-changing call. Without this
/// the contract instance would archive after ~7 days of inactivity and
/// the next call would fail until someone manually restored it. Cheap
/// enough to do per call: instance TTL extension is one host op.
pub fn extend_instance_ttl(env: &Env) {
    env.storage()
        .instance()
        .extend_ttl(INSTANCE_RENEWAL_THRESHOLD, INSTANCE_TARGET_TTL);
}
