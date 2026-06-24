//! XDR encoder for the handler's `RecordPayload { msg: String, msg_id: u64 }`
//! `#[contracttype]`.
//!
//! Why hand-rolled XDR instead of the Soroban SDK: this crate is a WASI 0.2
//! component, not a Soroban contract — pulling `soroban-sdk` in would drag
//! a wasm32v1-none runtime into a wasm32-wasip1 module. The XDR wire format
//! is stable and the surface we need is one `ScVal::Map`.
//!
//! Wire shape:
//!     ScVal::Map(Some(ScMap([
//!         ScMapEntry { key: Symbol("msg"),    val: String(msg)    },
//!         ScMapEntry { key: Symbol("msg_id"), val: U64(msg_id)    },
//!     ])))
//!
//! Soroban serialises a `#[contracttype] struct { msg, msg_id }` as exactly
//! the above. Entries inside an `ScMap` MUST be sorted ascending by key
//! bytes — alphabetically that's `msg` before `msg_id`, which is what the
//! handler's `RecordPayload::from_xdr` expects on the other end. Get this
//! ordering wrong and `from_xdr` succeeds locally but every operator's
//! bytes diverge from each other for any given event, collapsing the
//! quorum.

use anyhow::{Context, Result};
use stellar_xdr::curr::{
    Limits, ScMap, ScMapEntry, ScString, ScSymbol, ScVal, StringM, VecM, WriteXdr,
};

pub fn encode_record(msg: &str, msg_id: u64) -> Result<Vec<u8>> {
    // `StringM` is XDR's length-prefixed bytestring; the default bound is
    // u32::MAX so a runaway `msg` would balloon the payload before failing.
    // The handler's `String` field has the same effective bound, so deferring
    // to its `from_xdr` for size policy keeps the contract single-sourced.
    let msg_string: StringM = msg
        .as_bytes()
        .try_into()
        .context("msg string too long for XDR StringM")?;
    let msg_entry = ScMapEntry {
        key: symbol_val("msg")?,
        val: ScVal::String(ScString(msg_string)),
    };
    let msg_id_entry = ScMapEntry {
        key: symbol_val("msg_id")?,
        val: ScVal::U64(msg_id),
    };

    let entries: VecM<ScMapEntry> = vec![msg_entry, msg_id_entry]
        .try_into()
        .context("ScMap construction")?;
    let map = ScVal::Map(Some(ScMap(entries)));

    // `Limits::none()` skips the depth/length caps used for untrusted input;
    // safe here because we built the value ourselves and its shape is fixed.
    map.to_xdr(Limits::none()).context("xdr-encode RecordPayload")
}

fn symbol_val(s: &str) -> Result<ScVal> {
    // `ScSymbol` wraps `StringM<32>` — Soroban field-name symbols are capped
    // at 32 bytes. Both "msg" and "msg_id" fit comfortably, but keep the
    // fallible conversion so future renames fail loudly here instead of
    // producing malformed XDR.
    let inner: StringM<32> = s.as_bytes().try_into().context("symbol too long")?;
    Ok(ScVal::Symbol(ScSymbol(inner)))
}
