//! tick-circuit — every cron tick, emit a `TickPayload { ts }` payload.
//!
//! Trigger: cron. The schedule (e.g. `*/30 * * * * *`) is set externally in
//! the service workflow config; the only field this code reads from the
//! trigger is `trigger_time`.
//!
//! Submit kind: `Submit::Aggregator(...)` is set in the workflow, so each
//! successful tick returns `Ok(vec![one WasmResponse])` and the aggregator
//! sees an `AggregatorInput` per operator.
//!
//! Determinism note: every operator MUST emit byte-identical payload bytes
//! so their signatures quorum-collapse on the handler side. Sources of
//! determinism here:
//!   * `trigger_time` — the SCHEDULED firing time injected by the warpdrive
//!     scheduler, identical across operators.
//!   * `payload::encode_tick` — pure function of `ts`.
//! Do NOT reach for `wasi:clocks/wall-clock::now()` here — that would drift
//! per-operator by milliseconds and shred the quorum.
//!
//! `event_id_salt` is the 8-byte LE encoding of `ts`. The host uses
//! `(workflow_id, salt) -> event_id` to deduplicate operator submissions
//! belonging to the same scheduled tick; picking the scheduled `ts` as the
//! salt makes the same tick collapse to the same event-id across the network.
//!
//! `ordering = Some(ts)` lets the host preserve monotonic order if multiple
//! ticks accumulate before submission.

mod payload;

wit_bindgen::generate!({
    world: "circuit-world",
    path: "../../wit-definitions/wit",
    generate_all,
});

use warpdrive::vectr::input::TriggerData;

struct Component;

impl Guest for Component {
    fn run(trigger_action: TriggerAction) -> Result<Vec<WasmResponse>, String> {
        run_inner(trigger_action).map_err(|e| format!("tick-circuit: {e:#}"))
    }
}

fn run_inner(trigger_action: TriggerAction) -> anyhow::Result<Vec<WasmResponse>> {
    // The workflow trigger is cron; refuse any other variant rather than
    // silently signing arbitrary upstream events.
    let cron = match trigger_action.data {
        TriggerData::Cron(c) => c,
        other => anyhow::bail!("expected Cron trigger, got {other:?}"),
    };

    // `timestamp.nanos` is unix-nanoseconds (see warpdrive:types/core). The
    // handler stores and exposes a unix-seconds counter, so floor at the
    // boundary; integer division is exact and identical across operators.
    let ts = cron.trigger_time.nanos / 1_000_000_000;

    let payload = payload::encode_tick(ts)?;

    Ok(vec![WasmResponse {
        payload,
        ordering: Some(ts),
        event_id_salt: Some(ts.to_le_bytes().to_vec()),
    }])
}

export!(Component);
