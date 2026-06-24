use soroban_sdk::testutils::Address as _;
use soroban_sdk::{Address, Env};

use crate::{CounterContract, CounterContractClient};

fn setup(env: &Env) -> (CounterContractClient<'_>, Address) {
    // The handler is just an address here — we never deploy a real
    // handler contract for these unit tests. `mock_all_auths` (called
    // per-test) makes Soroban accept `handler.require_auth()` regardless
    // of who's actually authenticated, which lets us exercise the tick
    // path in isolation from the full envelope/verification flow.
    let handler = Address::generate(env);
    let id = env.register(CounterContract, (&handler,));
    (CounterContractClient::new(env, &id), handler)
}

#[test]
fn tick_increments_count_and_stores_ts() {
    let env = Env::default();
    env.mock_all_auths();
    let (client, handler) = setup(&env);

    assert_eq!(client.count(), 0);
    assert_eq!(client.last_tick(), 0);
    assert_eq!(client.handler(), handler);

    assert_eq!(client.tick(&1_700_000_000), 1);
    assert_eq!(client.tick(&1_700_000_030), 2);
    assert_eq!(client.tick(&1_700_000_060), 3);

    // Final state reflects the last call — the previous timestamps are
    // not retained on-chain; subscribers reconstruct history from the
    // `tick` event log.
    assert_eq!(client.count(), 3);
    assert_eq!(client.last_tick(), 1_700_000_060);
}

#[test]
#[should_panic]
fn tick_without_handler_auth_panics() {
    // No `mock_all_auths` — `handler.require_auth()` has no satisfied
    // entry in the auth tree and the host aborts the invocation. This
    // is the on-chain failure mode for any caller other than the
    // registered handler.
    let env = Env::default();
    let (client, _handler) = setup(&env);
    client.tick(&1_700_000_000);
}
