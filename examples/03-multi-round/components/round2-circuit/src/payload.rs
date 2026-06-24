//! XDR encoder for `SubmissionPayload::Final(FinalPayload)` — the bytes
//! the composer's `verify_xlm` decodes via `SubmissionPayload::from_xdr`
//! and dispatches to `apply_final`.
//!
//! Why hand-rolled XDR instead of the Soroban SDK: this crate is a WASI 0.2
//! component, not a Soroban contract — pulling `soroban-sdk` in would drag
//! a wasm32v1-none runtime into a wasm32-wasip1 module. The XDR wire format
//! is stable and the surface we need is one tagged enum + one map.
//!
//! Soroban serialises a `#[contracttype]` tuple-variant enum as
//!     ScVal::Vec(Some([ ScVal::Symbol("VariantName"), <inner ScVal> ]))
//! and the inner `FinalPayload` struct as an `ScVal::Map` with entries
//! sorted by key bytes (ScVal::Symbol of the field name) in ascending
//! order — alphabetic for ASCII names. Mis-ordering either layer makes
//! the on-chain `from_xdr` reject the envelope as `InvalidEnvelope`, so
//! this module is the single source of truth for the wire format.
//!
//! FinalPayload field order (alphabetic): `aggregate` < `round_id`.
//!
//! Wire shape:
//!     ScVal::Vec(Some([
//!         ScVal::Symbol("Final"),
//!         ScVal::Map(Some(ScMap([
//!             { Symbol("aggregate"), U64(aggregate) },
//!             { Symbol("round_id"),  U64(round_id)  },
//!         ]))),
//!     ]))

use anyhow::{Context, Result};
use stellar_xdr::curr::{
    Limits, ScMap, ScMapEntry, ScSymbol, ScVal, ScVec, StringM, VecM, WriteXdr,
};

pub fn encode_final(aggregate: u64, round_id: u64) -> Result<Vec<u8>> {
    // Inner struct: FinalPayload as ScVal::Map, fields alphabetised.
    let entries = vec![
        ScMapEntry {
            key: symbol_val("aggregate")?,
            val: ScVal::U64(aggregate),
        },
        ScMapEntry {
            key: symbol_val("round_id")?,
            val: ScVal::U64(round_id),
        },
    ];
    let map_entries: VecM<ScMapEntry> = entries
        .try_into()
        .context("ScMap construction (entry count > vec capacity)")?;
    let inner = ScVal::Map(Some(ScMap(map_entries)));

    // Outer tagged enum: SubmissionPayload::Final(FinalPayload).
    let variant: VecM<ScVal> = vec![symbol_val("Final")?, inner]
        .try_into()
        .context("ScVec construction")?;

    // `Limits::none()` skips the depth/length caps used for untrusted input;
    // safe here because we built the value ourselves and its shape is fixed.
    ScVal::Vec(Some(ScVec(variant)))
        .to_xdr(Limits::none())
        .context("xdr-encode SubmissionPayload::Final")
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
