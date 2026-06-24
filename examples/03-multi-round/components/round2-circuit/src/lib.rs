//! round2-circuit — observe `Round1Ready` on the composer, reduce the bundle
//! to its min, emit one byte-identical `FinalPayload` per operator.
//!
//! Trigger: `StellarContractEvent` on the composer, filtered server-side by
//! `set-stellar --topic-symbol r1ready --topic wildcard`. The circuit
//! re-validates the topic shape, decodes the on-chain `Round1Bundle`, and
//! emits a single `WasmResponse` carrying a tagged
//! `SubmissionPayload::Final(FinalPayload)`.
//!
//! Determinism note (spec § 2, OPPOSITE of round1-circuit): every operator
//! MUST emit byte-identical payload bytes here so the host quorum-collapses
//! their signatures into one envelope for the composer's `try_verify` call.
//! The bundle the circuit reduces is on-chain (immutable, observed
//! identically by every operator subscribing to the composer's events), and
//! the reduction is a pure `min` — no rounding choices, no overflow. The
//! only forbidden tool in this file is `wasi:clocks/wall-clock::now()`:
//! anything time-derived would drift between operators and shred the
//! quorum. If you ever need a timestamp here, take it from the bundle.

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
        run_inner(trigger_action).map_err(|e| format!("round2-circuit: {e:#}"))
    }
}

fn run_inner(trigger_action: TriggerAction) -> anyhow::Result<Vec<WasmResponse>> {
    // The workflow trigger is a Soroban contract event; refuse any other
    // variant rather than silently signing arbitrary upstream data.
    let event = match trigger_action.data {
        TriggerData::StellarContractEvent(e) => e.event,
        other => anyhow::bail!("expected StellarContractEvent trigger, got {other:?}"),
    };

    let bundle = trigger::parse_r1ready(&event.topic_segments, &event.value)?;
    if bundle.attestations.is_empty() {
        // The composer only emits Round1Ready once the bundle crosses the
        // quorum threshold, so empty would mean a malformed event. Bail
        // rather than sign over a vacuous Final.
        anyhow::bail!("Round1 bundle is empty");
    }

    // `min` is the simplest pure reduce: trivially deterministic, no
    // overflow, no rounding choices. Every operator sees the same on-chain
    // bundle and so picks the same minimum.
    let aggregate = bundle
        .attestations
        .iter()
        .map(|a| a.value)
        .min()
        .expect("non-empty by check above");

    let payload_bytes = payload::encode_final(aggregate, bundle.round_id)?;

    // Deterministic across operators (same round_id, same suffix). Distinct
    // from round1's per-Vectr salt by design — here we WANT the host's
    // QuorumQueue to collapse N envelopes into one with N signatures.
    let mut salt = bundle.round_id.to_le_bytes().to_vec();
    salt.extend_from_slice(b"-r2");

    Ok(vec![WasmResponse {
        payload: payload_bytes,
        ordering: Some(bundle.round_id),
        event_id_salt: Some(salt),
    }])
}

export!(Component);
