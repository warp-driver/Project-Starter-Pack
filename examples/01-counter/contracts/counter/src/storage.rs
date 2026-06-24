use soroban_sdk::{contracttype, Address, Env};

// TTL constants inlined (instead of importing `warpdrive_shared::ttl`)
// so this crate stays standalone — a developer can lift just the
// `counter/` directory into their own project and build it without
// pulling the WarpDrive ecosystem. Same numbers warpdrive-shared uses.
const DAY_IN_LEDGERS: u32 = 17_280;
const INSTANCE_TARGET_TTL: u32 = 7 * DAY_IN_LEDGERS;
const INSTANCE_RENEWAL_THRESHOLD: u32 = INSTANCE_TARGET_TTL - DAY_IN_LEDGERS;

#[contracttype]
pub enum DataKey {
    /// Address of the stellar-handler contract — the only caller we
    /// honour. Set once in `__constructor` and never rewritten; rotating
    /// the handler would mean redeploying the counter (intentional: keeps
    /// the trust chain auditable from the on-chain history alone).
    Handler,
    /// Monotonically increasing tick count.
    Count,
    /// Unix-seconds timestamp of the most recent tick — useful for a UI
    /// that wants to show "last seen N seconds ago" without scraping
    /// events. Kept as `u64` to match the circuit's `TickPayload.ts`.
    LastTick,
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

pub fn get_count(env: &Env) -> u64 {
    env.storage().instance().get(&DataKey::Count).unwrap_or(0)
}

pub fn set_count(env: &Env, count: u64) {
    env.storage().instance().set(&DataKey::Count, &count);
}

pub fn get_last_tick(env: &Env) -> u64 {
    env.storage().instance().get(&DataKey::LastTick).unwrap_or(0)
}

pub fn set_last_tick(env: &Env, ts: u64) {
    env.storage().instance().set(&DataKey::LastTick, &ts);
}

/// Bump the instance TTL on every state-changing call. Without this the
/// contract instance would archive after ~7 days of inactivity and the
/// next tick would fail until someone manually restored it. Cheap enough
/// to do per call: instance TTL extension is one host op.
pub fn extend_instance_ttl(env: &Env) {
    env.storage()
        .instance()
        .extend_ttl(INSTANCE_RENEWAL_THRESHOLD, INSTANCE_TARGET_TTL);
}
