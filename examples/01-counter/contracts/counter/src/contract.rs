use soroban_sdk::{contract, contracterror, contractevent, contractimpl, Address, Env};

use crate::storage;

#[contracterror]
#[derive(Copy, Clone, Debug, Eq, PartialEq, PartialOrd, Ord)]
pub enum CounterError {
    /// Reserved for future explicit-auth paths. Today the unauthorised
    /// case is caught by `handler.require_auth()`, which panics in the
    /// host before we get a chance to return — keeping the variant in
    /// the published spec gives downstream callers a stable error code
    /// to switch on once we surface auth failures non-fatally.
    UnauthorizedCaller = 1,
}

#[contract]
pub struct CounterContract;

#[contractimpl]
impl CounterContract {
    /// Pin the trusted handler at deploy time. The constructor argument
    /// is consumed exactly once; subsequent reads of `Handler` go
    /// straight to instance storage with no public setter — rotating
    /// trust means redeploying.
    pub fn __constructor(env: Env, handler: Address) {
        storage::set_handler(&env, &handler);
        storage::extend_instance_ttl(&env);
    }

    /// Open call signature, but the body gates on
    /// `handler.require_auth()`. Trust flows upstream from the operator
    /// quorum:
    ///   operator signatures → ed25519-verification → stellar-handler
    ///                       → counter.tick (this method)
    /// `require_auth` succeeds only when the current invocation was
    /// initiated by the registered handler contract — Soroban grants
    /// contract addresses implicit auth when they're the immediate
    /// caller. A direct tick from any other address panics here.
    pub fn tick(env: Env, ts: u64) -> Result<u64, CounterError> {
        let handler = storage::get_handler(&env);
        handler.require_auth();

        // `+ 1` not `checked_add`: with overflow-checks=true a host
        // panic is the right failure mode — this counter would have to
        // run for ~10^13 years at 30s/tick to wrap, so reaching it
        // means something else is profoundly wrong and we want the tx
        // to abort, not silently saturate.
        let count = storage::get_count(&env) + 1;
        storage::set_count(&env, count);
        storage::set_last_tick(&env, ts);
        storage::extend_instance_ttl(&env);

        // Topic = `count`; data = `ts`. Indexing on `count` lets a
        // subscriber resume from the last seen value with a single
        // topic filter, rather than scanning every event since deploy.
        Ticked { count, ts }.publish(&env);
        Ok(count)
    }

    pub fn count(env: Env) -> u64 {
        storage::get_count(&env)
    }

    pub fn last_tick(env: Env) -> u64 {
        storage::get_last_tick(&env)
    }

    pub fn handler(env: Env) -> Address {
        storage::get_handler(&env)
    }
}

/// On-chain audit trail of every successful tick. Subscribers (a UI,
/// an indexer, a chain-of-custody auditor) consume this stream to
/// reconstruct history without re-querying the contract for each
/// data point.
#[contractevent]
pub struct Ticked {
    #[topic]
    pub count: u64,
    pub ts: u64,
}
