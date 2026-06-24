//! event-watcher-circuit — turn one Soroban `("msg", msg_id, String)` event
//! into one verified record submission.
//!
//! Trigger: `StellarContractEvent` on the message-board contract, filtered
//! server-side by `set-stellar --topic-symbol msg --topic wildcard`. The
//! circuit re-validates the topic shape, decodes the payload, and emits a
//! single `WasmResponse` carrying a `RecordPayload { msg, msg_id }`.
//!
//! Determinism note: every operator MUST emit byte-identical payload bytes
//! so their signatures quorum-collapse on the handler side. Sources of
//! determinism here:
//!   * `topic_segments` + `value` — the on-chain event is identical for
//!     every operator subscribing to the same contract.
//!   * `payload::encode_record` — pure function of `(msg, msg_id)`.
//! Do NOT reach for `wasi:clocks/wall-clock::now()` or any per-host state;
//! that would drift across operators and shred the quorum.
//!
//! `event_id_salt` is the 8-byte LE encoding of `msg_id`. The host uses
//! `(workflow_id, salt) -> event_id` to deduplicate operator submissions
//! belonging to the same on-chain event; the message id is the natural
//! deterministic dedup key here.
//!
//! `ordering = Some(msg_id)` lets the host preserve monotonic order if
//! multiple events accumulate before submission.

mod payload;
mod trigger;

wit_bindgen::generate!({
    world: "circuit-world",
    path: "../../wit-definitions/wit",
    generate_all,
});

use warpdrive::vectr::input::TriggerData;

struct Component;

impl Guest for Component {
    fn run(trigger_action: TriggerAction) -> Result<Vec<WasmResponse>, String> {
        run_inner(trigger_action).map_err(|e| format!("event-watcher-circuit: {e:#}"))
    }
}

fn run_inner(trigger_action: TriggerAction) -> anyhow::Result<Vec<WasmResponse>> {
    // The workflow trigger is a Soroban contract event; refuse any other
    // variant rather than silently signing arbitrary upstream data.
    let event = match trigger_action.data {
        TriggerData::StellarContractEvent(e) => e.event,
        other => anyhow::bail!("expected StellarContractEvent trigger, got {other:?}"),
    };

    let decoded = trigger::parse_msg_event(&event.topic_segments, &event.value)?;
    let bytes = payload::encode_record(&decoded.msg, decoded.msg_id)?;

    Ok(vec![WasmResponse {
        payload: bytes,
        ordering: Some(decoded.msg_id),
        event_id_salt: Some(decoded.msg_id.to_le_bytes().to_vec()),
    }])
}

export!(Component);
