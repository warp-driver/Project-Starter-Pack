//! Soroban event decoder for the `("msg", msg_id) -> String` shape emitted
//! by the demo message-board contract:
//!
//!     env.events().publish((symbol_short!("msg"), msg_id), msg)
//!
//! On the wire, each `topic-segments` entry is the base64-encoded XDR of a
//! single ScVal; `event.value` is the base64-encoded XDR of the body ScVal
//! (see `warpdrive:types` stellar-event record). `set-stellar
//! --topic-symbol msg` (in build-service.sh) already filters by
//! topic[0] == Symbol("msg") server-side, but we re-check defensively so a
//! mis-wired filter never gets a forged payload signed by the quorum.

use anyhow::{Context, Result};
use base64::{engine::general_purpose::STANDARD, Engine as _};
use stellar_xdr::curr::{Limits, ReadXdr, ScSymbol, ScVal};

pub struct DecodedMsg {
    pub msg: String,
    pub msg_id: u64,
}

pub fn parse_msg_event(topic_segments: &[String], value: &str) -> Result<DecodedMsg> {
    // `symbol_short!("msg")` + the `msg_id` literal land as two separate
    // topic segments. Anything shorter means the filter let through an
    // event from a different shape — bail rather than guess.
    if topic_segments.len() < 2 {
        anyhow::bail!(
            "expected >=2 topic segments (symbol + msg_id), got {}",
            topic_segments.len()
        );
    }

    let topic0 = decode_scval(&topic_segments[0]).context("decode topic[0]")?;
    let symbol = match topic0 {
        ScVal::Symbol(ScSymbol(s)) => s.to_string(),
        other => anyhow::bail!("topic[0]: expected ScVal::Symbol, got {other:?}"),
    };
    if symbol != "msg" {
        anyhow::bail!("topic[0]: expected symbol \"msg\", got {symbol:?}");
    }

    let topic1 = decode_scval(&topic_segments[1]).context("decode topic[1]")?;
    let msg_id = match topic1 {
        ScVal::U64(n) => n,
        other => anyhow::bail!("topic[1]: expected ScVal::U64, got {other:?}"),
    };

    let body = decode_scval(value).context("decode event.value")?;
    let msg = match body {
        // Soroban `String` carries UTF-8 by convention, but the XDR layer
        // doesn't enforce it — validate here so a malformed publisher
        // fails loud instead of pushing non-UTF-8 bytes through to the
        // handler's `RecordPayload::from_xdr`.
        ScVal::String(s) => {
            let bytes: Vec<u8> = s.0.into();
            String::from_utf8(bytes).context("event.value: ScVal::String not valid UTF-8")?
        }
        other => anyhow::bail!("event.value: expected ScVal::String, got {other:?}"),
    };

    Ok(DecodedMsg { msg, msg_id })
}

fn decode_scval(b64: &str) -> Result<ScVal> {
    // Two-step on purpose: base64 -> raw XDR -> ScVal. `Limits::none()` is
    // safe because the upstream RPC has already bounded the event size; we
    // just need to read what it sent. `from_xdr_base64` would fold both
    // steps but matches hodlers-app exactly only via the inherent method
    // — keep the explicit pair so the failure points stay distinct.
    let bytes = STANDARD.decode(b64).context("base64 decode")?;
    ScVal::from_xdr(&bytes, Limits::none()).context("xdr decode")
}
