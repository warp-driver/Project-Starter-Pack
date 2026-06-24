use soroban_sdk::{
    contract, contractimpl, contracttype, xdr::FromXdr, Address, Bytes, BytesN, Env, String,
};
use warpdrive_shared::interfaces::{
    handler::{Ed25519SignatureData, HandlerError, Verified, XlmEnvelope},
    verification::Ed25519VerificationClient,
};

use counter::CounterContractClient;

use crate::storage;

/// Payload carried inside an `XlmEnvelope.payload`. The circuit emits
/// the matching shape as a hand-built `ScMap` with one entry keyed
/// `"ts"`; Soroban serialises this struct as an `ScMap` sorted by
/// alphabetical field name, so a single-field struct trivially matches
/// the circuit's encoding. Add fields here only by adding them to the
/// circuit's map at the same time, keeping the keys sorted.
#[contracttype]
#[derive(Clone)]
pub struct TickPayload {
    pub ts: u64,
}

#[contract]
pub struct StellarHandler;

#[contractimpl]
impl StellarHandler {
    /// Pin both the verification contract and the downstream counter
    /// at deploy time. The version string lets tooling confirm what's
    /// live on-chain without trusting a deploy log.
    pub fn __constructor(env: Env, verification_contract: Address, counter_contract: Address) {
        storage::set_verification_contract(&env, &verification_contract);
        storage::set_counter_contract(&env, &counter_contract);
        storage::set_version(&env, &String::from_str(&env, env!("CARGO_PKG_VERSION")));
        storage::extend_instance_ttl(&env);
    }

    /// Single entrypoint. Anyone may call it — the security model lives
    /// in the signature check. Sequence:
    ///   1. Decode the XDR envelope. Bytes that aren't even valid XDR
    ///      panic in the host before we get here; bytes that are valid
    ///      XDR but the wrong shape return InvalidEnvelope.
    ///   2. Replay-check the event_id. Cheaper than verifying a
    ///      signature we'll then reject for being a duplicate.
    ///   3. Hand the envelope bytes to ed25519-verification — it
    ///      enforces the operator-set threshold relative to the
    ///      reference_block (so signer churn doesn't break in-flight
    ///      envelopes).
    ///   4. Decode the inner TickPayload and relay to counter.tick.
    ///   5. Mark the event seen + emit Verified so downstream indexers
    ///      and the WarpDrive runtime can dedup.
    pub fn verify_xlm(
        env: Env,
        envelope_bytes: Bytes,
        sig_data: Ed25519SignatureData,
    ) -> Result<(), HandlerError> {
        let envelope = XlmEnvelope::from_xdr(&env, &envelope_bytes)
            .map_err(|_| HandlerError::InvalidEnvelope)?;
        let event_id = envelope.event_id.clone();

        if storage::is_event_seen(&env, &event_id) {
            return Err(HandlerError::EventAlreadySeen);
        }

        // try_verify returns Result<Result<(), VerifyError>, InvokeError>:
        // outer = host-level invocation failure, inner = the verifier's
        // typed error. We collapse both layers to HandlerError so the
        // caller sees one flat error space.
        let verification_addr = storage::get_verification_contract(&env);
        match Ed25519VerificationClient::new(&env, &verification_addr).try_verify(
            &envelope_bytes,
            &sig_data.signatures,
            &sig_data.signers,
            &sig_data.reference_block,
        ) {
            Ok(Ok(())) => {}
            Ok(Err(_)) => return Err(HandlerError::UnknownVerificationError),
            Err(Ok(e)) => return Err(HandlerError::from(e)),
            Err(Err(_)) => return Err(HandlerError::OtherInvocationError),
        }

        let payload = TickPayload::from_xdr(&env, &envelope.payload)
            .map_err(|_| HandlerError::InvalidEnvelope)?;

        // Cross-contract call: counter.tick gates on
        // `handler.require_auth()`, which Soroban grants implicitly
        // because *this* contract is the immediate caller. No explicit
        // auth payload needed.
        let counter_addr = storage::get_counter_contract(&env);
        CounterContractClient::new(&env, &counter_addr).tick(&payload.ts);

        // Order matters: mark seen + extend TTL + emit Verified happen
        // only after the downstream call succeeded. A panic in counter
        // would unwind the whole transaction, so we'd never have
        // marked a failed event as processed.
        storage::mark_event_seen(&env, &event_id);
        storage::extend_instance_ttl(&env);
        Verified::new(event_id).publish(&env);
        Ok(())
    }

    pub fn verification_contract(env: Env) -> Address {
        storage::get_verification_contract(&env)
    }

    pub fn counter_contract(env: Env) -> Address {
        storage::get_counter_contract(&env)
    }

    /// WarpDrive runtime queries this hook to replay an envelope by
    /// event_id. We don't persist payloads — operators always have
    /// them in the circuit's output — so we return None and let the
    /// runtime fall back to its own caches.
    pub fn payload(_env: Env, _event_id: BytesN<20>) -> Option<Bytes> {
        None
    }
}
