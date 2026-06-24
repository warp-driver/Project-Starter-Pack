use soroban_sdk::testutils::Address as _;
use soroban_sdk::{Address, Env, String};

use crate::{MessageBoard, MessageBoardClient, MessageBoardError};

fn setup(env: &Env) -> (MessageBoardClient<'_>, Address) {
    // The handler is just an address here — we never deploy a real
    // handler contract for these unit tests. `mock_all_auths` (called
    // per-test) makes Soroban accept `handler.require_auth()` regardless
    // of who's actually authenticated, which lets us exercise
    // record_signed in isolation from the full envelope/verification
    // flow. The handler crate's tests deploy the whole pipeline end to
    // end; here we want fast unit coverage of the storage + idempotency
    // logic only.
    let handler = Address::generate(env);
    let id = env.register(MessageBoard, (&handler,));
    (MessageBoardClient::new(env, &id), handler)
}

#[test]
fn publish_and_record_round_trip() {
    let env = Env::default();
    env.mock_all_auths();
    let (client, handler) = setup(&env);

    assert_eq!(client.next_id(), 0);
    assert_eq!(client.handler(), handler);

    let hello = String::from_str(&env, "hello");
    let id = client.publish(&hello);
    assert_eq!(id, 0);
    assert_eq!(client.next_id(), 1);
    // Pre-record the message has no canonical entry — publish only
    // emits the trigger event; storage is written by the handler-gated
    // path.
    assert_eq!(client.recorded(&id), None);

    client.record_signed(&id, &hello);
    assert_eq!(client.recorded(&id), Some(hello));

    // Second publish keeps the counter monotonic even with no record
    // in between — id allocation is independent of recording.
    let id2 = client.publish(&String::from_str(&env, "world"));
    assert_eq!(id2, 1);
}

#[test]
fn record_signed_is_idempotent_by_id() {
    let env = Env::default();
    env.mock_all_auths();
    let (client, _handler) = setup(&env);

    let msg = String::from_str(&env, "hello");
    let _ = client.publish(&msg);
    client.record_signed(&0, &msg);

    // Second submission of the same id (different content even) is
    // rejected with AlreadyRecorded, not silently overwriting. Use
    // `try_record_signed` so the typed-client decodes the contract
    // error rather than panicking on the host return.
    let dup = client.try_record_signed(&0, &String::from_str(&env, "tampered"));
    assert_eq!(dup, Err(Ok(MessageBoardError::AlreadyRecorded)));

    // Canonical record is untouched — the loser of the race cannot
    // alter what the first writer stored.
    assert_eq!(client.recorded(&0), Some(msg));
}
