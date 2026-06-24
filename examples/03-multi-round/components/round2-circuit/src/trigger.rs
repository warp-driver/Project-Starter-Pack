//! Decode the composer's `Round1Ready` event into the typed bundle the
//! round2-circuit reduces.
//!
//! Event shape (set by `Composer::apply_round1` once the bundle crosses
//! quorum):
//!   topic 0: `ScVal::Symbol("r1ready")`
//!   topic 1: `ScVal::U64(round_id)`
//!   value:   `ScVal::Map(Round1Bundle)` — one entry, `attestations:
//!            Vec<Round1Attestation>`. Each `Round1Attestation` is itself
//!            an `ScVal::Map` with alphabetic fields:
//!            `signer: BytesN<32>`, `value: U64`.
//!
//! WarpDrive's Stellar event poller forwards `topic_segments` and `value`
//! as opaque strings. Default `xdrFormat` for `getEvents` is `base64`, but
//! the engine sometimes pre-decodes to the stellar-xdr `serde_json` shape;
//! [`parse_scval`] tries JSON first, then XDR-base64, so the circuit works
//! against either flavour without runtime configuration. Same helper every
//! other WarpDrive Stellar-event circuit ships (see oracle-demo's
//! median-circuit/trigger.rs and 02-event-watcher's event-watcher-circuit).

use anyhow::{anyhow, bail, Context, Result};
use base64::{engine::general_purpose::STANDARD, Engine as _};
use stellar_xdr::curr::{Limits, ReadXdr, ScMapEntry, ScSymbol, ScVal};

/// One Round 1 attestation, decoded from the on-chain `Round1Attestation`
/// struct carried inside the `Round1Ready` event bundle. Field types are
/// the native ones a reducer needs — fixed-size key array for the 32-byte
/// ed25519 signer, `u64` for the per-operator scalar.
///
/// `signer` is parsed but unused by the current reduce (Round 2 only takes
/// `min` over `value`). Kept on the type so the decoder mirrors the
/// on-chain wire shape 1:1 — drop only if you also drop the field from the
/// composer's `Round1Attestation` contracttype, since reading by name
/// elsewhere would then fail. Re-introducing an off-chain ed25519
/// re-verify (oracle-demo's median-circuit/verify.rs pattern) lights this
/// field up without changing the wire format.
#[allow(dead_code)]
pub struct DecodedAttestation {
    pub signer: [u8; 32],
    pub value: u64,
}

/// Decoded `Round1Ready` event payload, plus the `round_id` extracted from
/// topic 1 so the caller can build a stable ordering / salt without
/// re-parsing the topics.
pub struct DecodedBundle {
    pub round_id: u64,
    pub attestations: Vec<DecodedAttestation>,
}

pub fn parse_r1ready(topic_segments: &[String], value: &str) -> Result<DecodedBundle> {
    // The composer publishes two topics (symbol + round_id) and `event.value`
    // is the bundle map. Anything shorter means the server-side topic filter
    // let through an event of a different shape — bail rather than guess.
    if topic_segments.len() < 2 {
        bail!(
            "expected >=2 topic segments (symbol + round_id), got {}",
            topic_segments.len()
        );
    }

    let topic0 = parse_scval(&topic_segments[0]).context("decode topic[0]")?;
    let symbol = match topic0 {
        ScVal::Symbol(ScSymbol(s)) => String::from_utf8(s.to_vec())
            .context("topic[0]: ScSymbol not valid UTF-8")?,
        other => bail!("topic[0]: expected ScVal::Symbol, got {other:?}"),
    };
    if symbol != "r1ready" {
        bail!("topic[0]: expected symbol \"r1ready\", got {symbol:?}");
    }

    let topic1 = parse_scval(&topic_segments[1]).context("decode topic[1]")?;
    let round_id = match topic1 {
        ScVal::U64(n) => n,
        other => bail!("topic[1]: expected ScVal::U64, got {other:?}"),
    };

    let body = parse_scval(value).context("decode event.value")?;
    let entries = expect_map(&body).context("event.value not a Map")?;
    let attestations_vec = expect_vec(get_field(entries, "attestations")?)?;
    let attestations = attestations_vec
        .iter()
        .map(decode_attestation)
        .collect::<Result<Vec<_>>>()?;

    Ok(DecodedBundle { round_id, attestations })
}

fn decode_attestation(val: &ScVal) -> Result<DecodedAttestation> {
    let entries = expect_map(val).context("attestation not a Map")?;
    // Alphabetic: `signer` < `value`. The composer's #[contracttype] derive
    // emits them in that order; reading by name is order-insensitive but
    // catches a renamed field loudly.
    let signer_bytes = expect_bytes(get_field(entries, "signer")?)?;
    let value = expect_u64(get_field(entries, "value")?)?;

    let signer: [u8; 32] = signer_bytes
        .try_into()
        .map_err(|v: Vec<u8>| anyhow!("signer not 32 bytes (got {})", v.len()))?;

    Ok(DecodedAttestation { signer, value })
}

// ── ScVal helpers ────────────────────────────────────────────────────────

/// Decode a string-encoded `ScVal` the WarpDrive engine handed us on a
/// `StellarEvent` field. The host emits either the JSON form (stellar-xdr
/// `serde` representation) or the XDR-base64 form (Stellar RPC default);
/// try JSON first so we keep the typed value path, then fall back.
fn parse_scval(raw: &str) -> Result<ScVal> {
    if let Ok(v) = serde_json::from_str::<ScVal>(raw) {
        return Ok(v);
    }
    // Two-step on purpose so each failure point stays distinct: base64
    // decode failure vs XDR-shape failure. `from_xdr_base64` folds both
    // but loses that distinction in error messages.
    let bytes = STANDARD
        .decode(raw)
        .map_err(|e| anyhow!("ScVal decode (neither JSON nor base64): {e}"))?;
    ScVal::from_xdr(&bytes, Limits::none())
        .map_err(|e| anyhow!("ScVal XDR decode: {e}"))
}

fn get_field<'a>(entries: &'a [ScMapEntry], field: &str) -> Result<&'a ScVal> {
    entries
        .iter()
        .find(|e| {
            matches!(
                &e.key,
                ScVal::Symbol(ScSymbol(s)) if s.as_slice() == field.as_bytes()
            )
        })
        .map(|e| &e.val)
        .ok_or_else(|| anyhow!("missing field {field:?} in ScMap"))
}

fn expect_map(val: &ScVal) -> Result<&[ScMapEntry]> {
    match val {
        ScVal::Map(Some(m)) => Ok(m.0.as_slice()),
        other => bail!("expected ScVal::Map, got {other:?}"),
    }
}

fn expect_vec(val: &ScVal) -> Result<&[ScVal]> {
    match val {
        ScVal::Vec(Some(v)) => Ok(v.0.as_slice()),
        other => bail!("expected ScVal::Vec, got {other:?}"),
    }
}

fn expect_u64(val: &ScVal) -> Result<u64> {
    match val {
        ScVal::U64(n) => Ok(*n),
        other => bail!("expected ScVal::U64, got {other:?}"),
    }
}

fn expect_bytes(val: &ScVal) -> Result<Vec<u8>> {
    match val {
        ScVal::Bytes(b) => Ok(b.0.to_vec()),
        // Soroban's `BytesN<32>` is also exposed as ScVal::Bytes; the only
        // distinction is the on-chain type's fixed length, enforced by the
        // `try_into::<[u8;32]>` in `decode_attestation`.
        other => bail!("expected ScVal::Bytes, got {other:?}"),
    }
}
