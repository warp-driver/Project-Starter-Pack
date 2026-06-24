//! XDR encoder for the handler's `TickPayload { ts: u64 }` `#[contracttype]`.
//!
//! Why hand-rolled XDR instead of the Soroban SDK: this crate is a WASI 0.2
//! component, not a Soroban contract — pulling `soroban-sdk` in would drag
//! a wasm32v1-none runtime into a wasm32-wasip1 module. The XDR wire format
//! is stable and the surface we need is one `ScVal::Map`.
//!
//! Wire shape:
//!     ScVal::Map(Some(ScMap([ ScMapEntry { key: Symbol("ts"), val: U64(ts) } ])))
//!
//! Soroban serialises a `#[contracttype] struct { ts: u64 }` as exactly the
//! above. Entries inside an `ScMap` MUST be sorted ascending by key bytes;
//! with a single entry that ordering is trivial, but the convention is what
//! the handler's `TickPayload::from_xdr` is doing on the other end — if we
//! ever add a second field, sort by field name alphabetically (same byte
//! order Soroban uses).

use anyhow::{Context, Result};
use stellar_xdr::curr::{
    Limits, ScMap, ScMapEntry, ScSymbol, ScVal, StringM, VecM, WriteXdr,
};

pub fn encode_tick(ts: u64) -> Result<Vec<u8>> {
    let entry = ScMapEntry {
        key: symbol_val("ts")?,
        val: ScVal::U64(ts),
    };
    let entries: VecM<ScMapEntry> = vec![entry]
        .try_into()
        .context("ScMap construction")?;
    let map = ScVal::Map(Some(ScMap(entries)));

    // `Limits::none()` skips the depth/length caps used for untrusted input;
    // safe here because we built the value ourselves and its shape is fixed.
    map.to_xdr(Limits::none()).context("xdr-encode TickPayload")
}

fn symbol_val(s: &str) -> Result<ScVal> {
    // `ScSymbol` wraps `StringM<32>` — Soroban field-name symbols are capped
    // at 32 bytes. "ts" is well under, but keep the fallible conversion so
    // future renames fail loudly here instead of producing malformed XDR.
    let inner: StringM<32> = s.as_bytes().try_into().context("symbol too long")?;
    Ok(ScVal::Symbol(ScSymbol(inner)))
}
