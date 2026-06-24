use soroban_sdk::{contract, contracterror, contractevent, contractimpl, Address, Env, String};

use crate::storage;

#[contracterror]
#[derive(Copy, Clone, Debug, Eq, PartialEq, PartialOrd, Ord)]
pub enum MessageBoardError {
    /// Reserved for future explicit-auth paths. Today the unauthorised
    /// case is caught by `handler.require_auth()`, which panics in the
    /// host before we get a chance to return — keeping the variant in
    /// the published spec gives downstream callers a stable error code
    /// to switch on once we surface auth failures non-fatally.
    UnauthorizedCaller = 1,
    /// `record_signed` was called twice for the same `msg_id`. The
    /// quorum can legitimately re-broadcast a signed envelope (network
    /// retries, late operator catch-up), so this is an expected,
    /// non-fatal outcome: the caller treats it as success — the record
    /// is already on-chain — and the second tx records no state
    /// changes. Distinct from `EventAlreadySeen` on the handler, which
    /// stops the verification work earlier; this is the contract-level
    /// idempotency net for the case where two operators race past the
    /// handler's seen-set check inside the same ledger.
    AlreadyRecorded = 2,
}

#[contract]
pub struct MessageBoard;

#[contractimpl]
impl MessageBoard {
    /// Pin the trusted handler at deploy time. Same pattern as
    /// 01-counter: the constructor argument is consumed exactly once;
    /// subsequent reads of `Handler` go straight to instance storage
    /// with no public setter — rotating trust means redeploying.
    pub fn __constructor(env: Env, handler: Address) {
        storage::set_handler(&env, &handler);
        storage::extend_instance_ttl(&env);
    }

    /// Open call: any wallet may publish a message. Increments the
    /// monotonic id, emits a `Published` event topic-keyed on `msg_id`
    /// (the warpdrive operator nodes are subscribed to this topic),
    /// and returns the id so the caller can correlate the eventual
    /// `recorded(msg_id)` read.
    ///
    /// No auth on the open side — the trust gate sits on the *output*
    /// path (`record_signed`), so anyone can feed the loop but only
    /// the quorum-signed record lands in canonical storage.
    pub fn publish(env: Env, msg: String) -> u64 {
        let msg_id = storage::next_msg_id(&env);
        storage::extend_instance_ttl(&env);
        Published {
            msg_id,
            msg: msg.clone(),
        }
        .publish(&env);
        msg_id
    }

    /// Handler-only — the verified payload from the warpdrive quorum
    /// makes its way here after `stellar-handler.verify_xlm` succeeds.
    /// Trust flows upstream:
    ///   operator signatures → ed25519-verification → stellar-handler
    ///                       → message_board.record_signed (this method)
    /// `require_auth` succeeds only when the immediate caller is the
    /// registered handler contract.
    ///
    /// Idempotent: a second call with the same `msg_id` returns
    /// `AlreadyRecorded` without mutating state. Two operators racing
    /// past the handler's replay-seen check within one ledger would
    /// otherwise both reach this method — returning an error (rather
    /// than panicking) lets the loser's tx complete cleanly and keeps
    /// the canonical record exactly what the first writer stored.
    pub fn record_signed(
        env: Env,
        msg_id: u64,
        msg: String,
    ) -> Result<(), MessageBoardError> {
        let handler = storage::get_handler(&env);
        handler.require_auth();

        if storage::has_record(&env, msg_id) {
            return Err(MessageBoardError::AlreadyRecorded);
        }

        storage::set_record(&env, msg_id, &msg);
        storage::extend_instance_ttl(&env);

        // Output-stream event: frontends and indexers subscribe to
        // `recorded` to react only to quorum-signed messages, skipping
        // the noisier raw `msg` topic from `publish`.
        Recorded { msg_id, msg }.publish(&env);
        Ok(())
    }

    pub fn recorded(env: Env, msg_id: u64) -> Option<String> {
        storage::get_record(&env, msg_id)
    }

    /// Peek at the id `publish` would hand out next. Useful for a UI
    /// that wants to show the live counter without bumping it.
    pub fn next_id(env: Env) -> u64 {
        storage::peek_next_msg_id(&env)
    }

    pub fn handler(env: Env) -> Address {
        storage::get_handler(&env)
    }
}

/// Input-stream event the warpdrive circuit subscribes to. Topic
/// layout is locked: `topic[0] = Symbol("msg")`, `topic[1] = U64(msg_id)`,
/// `data = String(msg)`. Operators set the filter as
/// `[Exact("msg"), Wildcard]` so every publish matches regardless of id.
#[contractevent(topics = ["msg"])]
pub struct Published {
    #[topic]
    pub msg_id: u64,
    pub msg: String,
}

/// Output-stream event emitted after a quorum-signed record lands.
/// Topic-keyed on `msg_id` so a subscriber can replay a specific
/// message's confirmation without scanning the full log.
#[contractevent(topics = ["recorded"])]
pub struct Recorded {
    #[topic]
    pub msg_id: u64,
    pub msg: String,
}
