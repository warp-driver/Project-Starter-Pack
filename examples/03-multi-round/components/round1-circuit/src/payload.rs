//! XDR encoder for `SubmissionPayload::Round1(Round1Payload)` — the bytes
//! the composer's `verify_xlm` decodes via `SubmissionPayload::from_xdr`
//! and dispatches to `apply_round1`.
//!
//! Why hand-rolled XDR instead of the Soroban SDK: this crate is a WASI 0.2
//! component, not a Soroban contract — pulling `soroban-sdk` in would drag
//! a wasm32v1-none runtime into a wasm32-wasip1 module. The XDR wire format
//! is stable and the surface we need is one tagged enum + one map.
//!
//! Soroban serialises a `#[contracttype]` tuple-variant enum as
//!     ScVal::Vec(Some([ ScVal::Symbol("VariantName"), <inner ScVal> ]))
//! and the inner `Round1Payload` struct as an `ScVal::Map` with entries
//! sorted by key bytes (ScVal::Symbol of the field name) in ascending
//! order — alphabetic for ASCII names. Mis-ordering either layer makes
//! the on-chain `from_xdr` reject the envelope as `InvalidEnvelope`, so
//! this module is the single source of truth for the wire format.
//!
//! Round1Payload field order (alphabetic): `round_id` < `signer_value`.
//!
//! Wire shape:
//!     ScVal::Vec(Some([
//!         ScVal::Symbol("Round1"),
//!         ScVal::Map(Some(ScMap([
//!             { Symbol("round_id"),     U64(round_id)     },
//!             { Symbol("signer_value"), U64(signer_value) },
//!         ]))),
//!     ]))

use anyhow::{Context, Result};
use stellar_xdr::curr::{
    Limits, ScMap, ScMapEntry, ScSymbol, ScVal, ScVec, StringM, VecM, WriteXdr,
};

pub fn encode_round1(round_id: u64, signer_value: u64) -> Result<Vec<u8>> {
    // Inner struct: Round1Payload as ScVal::Map, fields alphabetised.
    let entries = vec![
        ScMapEntry {
            key: symbol_val("round_id")?,
            val: ScVal::U64(round_id),
        },
        ScMapEntry {
            key: symbol_val("signer_value")?,
            val: ScVal::U64(signer_value),
        },
    ];
    let map_entries: VecM<ScMapEntry> = entries
        .try_into()
        .context("ScMap construction (entry count > vec capacity)")?;
    let inner = ScVal::Map(Some(ScMap(map_entries)));

    // Outer tagged enum: SubmissionPayload::Round1(Round1Payload).
    let variant: VecM<ScVal> = vec![symbol_val("Round1")?, inner]
        .try_into()
        .context("ScVec construction")?;

    // `Limits::none()` skips the depth/length caps used for untrusted input;
    // safe here because we built the value ourselves and its shape is fixed.
    ScVal::Vec(Some(ScVec(variant)))
        .to_xdr(Limits::none())
        .context("xdr-encode SubmissionPayload::Round1")
}

fn symbol_val(s: &str) -> Result<ScVal> {
    // `ScSymbol` wraps `StringM<32>` — Soroban field-name symbols are capped
    // at 32 bytes. All names here fit comfortably, but keep the fallible
    // conversion so future renames fail loudly here rather than producing
    // malformed XDR the composer silently rejects.
    let inner: StringM<32> = s
        .as_bytes()
        .try_into()
        .context("field-name symbol too long for ScSymbol (≤32 bytes)")?;
    Ok(ScVal::Symbol(ScSymbol(inner)))
}
