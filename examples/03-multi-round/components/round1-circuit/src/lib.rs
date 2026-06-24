//! round1-circuit — every cron tick, emit one single-signer `Round1Payload`.
//!
//! Trigger: cron. The schedule (e.g. `*/30 * * * * *`) is set externally in
//! the service workflow config; the only field this code reads from the
//! trigger is `trigger_time`.
//!
//! Asymmetry note (spec § 2): Round 1 is the OPPOSITE of 01-counter /
//! 02-event-watcher. There, every operator must emit byte-identical bytes so
//! the host quorum-collapses the signatures into one envelope. Here, each
//! operator MUST emit a DIFFERENT payload so the composer accumulates a
//! bundle of distinct single-signer attestations before reducing in Round 2.
//! `wall_clock::now()` is the cheapest source of inter-operator drift — its
//! sub-second nanoseconds differ across machines by definition.
//!
//! Sources of (un-)determinism here:
//!   * `round_id` — `trigger_time.nanos / 30_000_000_000` is the scheduled
//!     30 s bucket, IDENTICAL across operators. Used as the bundle key on
//!     chain (`Attestations(round_id)`) and as `ordering`.
//!   * `signer_value` — `wall_clock::now().nanoseconds % 1000`. PER-OPERATOR
//!     by design; the composer dedups by signer, so two operators landing
//!     on the same value is fine, but the SALT must still be unique per
//!     operator so the off-chain submission manager doesn't collapse the
//!     envelopes.
//!
//! `event_id_salt = payload_bytes.clone()` — the payload bytes already
//! encode the per-operator `signer_value`, so reusing them is the cheapest
//! unique fingerprint (same trick as oracle-demo's twap-circuit).
//!
//! `ordering = Some(round_id)` keeps a stable per-bucket ordering for the
//! host's submission queue.

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
        run_inner(trigger_action).map_err(|e| format!("round1-circuit: {e:#}"))
    }
}

fn run_inner(trigger_action: TriggerAction) -> anyhow::Result<Vec<WasmResponse>> {
    // The workflow trigger is cron; refuse any other variant rather than
    // silently signing arbitrary upstream events.
    let cron = match trigger_action.data {
        TriggerData::Cron(c) => c,
        other => anyhow::bail!("expected Cron trigger, got {other:?}"),
    };

    // 30 s bucket, stable across operators on the same scheduled tick. Same
    // arithmetic the composer uses to key `Attestations(round_id)`.
    let round_id = cron.trigger_time.nanos / 30_000_000_000;

    // PER-VECTR by design (spec § 2): wall_clock drifts by milliseconds
    // between operator hosts, so each Vectr ends up with a different
    // `signer_value`. `% 1000` keeps the demo number small and readable;
    // the composer just stores it as an opaque u64.
    let signer_value =
        (crate::wasi::clocks::wall_clock::now().nanoseconds % 1000) as u64;

    let payload_bytes = payload::encode_round1(round_id, signer_value)?;

    Ok(vec![WasmResponse {
        payload: payload_bytes.clone(),
        ordering: Some(round_id),
        // Payload already differs per operator (signer_value) — reuse it as
        // the unique fingerprint rather than minting fresh entropy.
        event_id_salt: Some(payload_bytes),
    }])
}

export!(Component);
