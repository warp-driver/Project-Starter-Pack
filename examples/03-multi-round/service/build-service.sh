#!/usr/bin/env bash
#
# build-service.sh — assemble service/service.json declaratively via
# warpdrive-cli for 03-multi-round's two-workflow pipeline:
#
#   round1   cron @ "$CRON_SCHEDULE"
#            -> round1-circuit (no http, no fs — emits
#               SubmissionPayload::Round1 { round_id, signer_value }
#               with per-Vectr salt = the payload bytes themselves so
#               the host's QuorumQueue keeps every operator's envelope
#               distinct)
#            -> submit aggregator (chain + service_handler = composer)
#            -> composer.verify_xlm dispatches to apply_round1, which
#               dedups by signer, accumulates a Round1Bundle, and
#               emits the `r1ready` composition event when the bundle
#               crosses ceil(N * quorum_num / quorum_denom)
#
#   round2   stellar event (composer, topic[0]=symbol "r1ready", topic[1]=*)
#            -> round2-circuit (no http, no fs — decodes the on-chain
#               Round1Bundle, reduces with min(), emits
#               SubmissionPayload::Final { round_id, aggregate } with
#               deterministic salt = round_id.to_le_bytes() ++ b"-r2"
#               so every operator produces byte-identical envelopes and
#               the host quorum-collapses them into a single submission)
#            -> submit aggregator (chain + service_handler = composer)
#            -> composer.verify_xlm dispatches to apply_final, which
#               stores Final(round_id) and emits the `final` event
#
# Then sets the stellar service manager to project_root and appends the
# ed25519 / sep53 signature_kind via the warpdrive-cli signature
# subcommand (matches the on-chain ed25519-verification contract's
# prefix).
#
# Why two workflows? The submission manager only calls one handler
# method per service (`verify_xlm`), so multi-round pipelines wrap
# each round's payload in a tagged enum and dispatch INSIDE the
# contract. Round 1 fans out (per-Vectr value, per-Vectr salt — the
# host keeps the envelopes separate). Round 2 fans in (deterministic
# input, deterministic salt — the host collapses to one quorum-signed
# envelope). The same `service_handler` address (the composer)
# receives both rounds; the tagged enum tells it which arm to take.
#
# Required inputs (relative to the example root):
#
#   out/composer.json        { "composer": "C..." }              (from `task deploy-composer`)
#   out/deploy.json          { "contracts": { "project_root": "C...", ... } }
#                                                                 (from `task deploy-middleware`)
#   out/round1-circuit.digest   64-hex content digest             (from `task upload-round1-circuit`)
#   out/round2-circuit.digest   64-hex content digest             (from `task upload-round2-circuit`)
#   out/aggregator.digest       64-hex content digest             (from `task upload-aggregator`)
#   SERVICE_FILE          target path (default: service/service.json)
#   TRIGGER_CHAIN         chain key the round2 circuit subscribes to AND
#                         both rounds submit envelopes from — same chain
#                         because the composer hosts both the trigger
#                         contract (emits `r1ready`) and the
#                         `verify_xlm` sink
#                         (default: stellar:testnet)
#   MANAGER_CHAIN         chain key the service manager (project_root)
#                         lives on (default: stellar:testnet)
#   CRON_SCHEDULE         cron expression driving Round 1 ticks
#                         (default: every 30 s)

set -euo pipefail

SERVICE_FILE="${SERVICE_FILE:-service/service.json}"
TRIGGER_CHAIN="${TRIGGER_CHAIN:-stellar:testnet}"
MANAGER_CHAIN="${MANAGER_CHAIN:-stellar:testnet}"
CRON_SCHEDULE="${CRON_SCHEDULE:-*/30 * * * * *}"

# ── prerequisite files ────────────────────────────────────────────────

for f in out/composer.json out/deploy.json \
         out/round1-circuit.digest out/round2-circuit.digest out/aggregator.digest; do
    test -s "$f" || { echo "missing $f — run the prerequisite task first" >&2; exit 1; }
done

COMPOSER=$(jq -r .composer out/composer.json)
PROJECT_ROOT=$(jq -r .contracts.project_root out/deploy.json)
ROUND1_DIGEST=$(cat out/round1-circuit.digest)
ROUND2_DIGEST=$(cat out/round2-circuit.digest)
AGG_DIGEST=$(cat out/aggregator.digest)

for v in COMPOSER PROJECT_ROOT ROUND1_DIGEST ROUND2_DIGEST AGG_DIGEST; do
    test -n "${!v}" || { echo "$v empty after parsing prerequisites" >&2; exit 1; }
done

# ── initialise ─────────────────────────────────────────────────────────

mkdir -p "$(dirname "$SERVICE_FILE")"
rm -f "$SERVICE_FILE"

warpdrive-cli service -f "$SERVICE_FILE" init --name multi-round

# ── workflow 1: round1 (cron → round1-circuit → aggregator → composer) ─

warpdrive-cli service -f "$SERVICE_FILE" workflow add --id round1

warpdrive-cli service -f "$SERVICE_FILE" workflow trigger \
    --id round1 set-cron \
    --schedule "$CRON_SCHEDULE"

warpdrive-cli service -f "$SERVICE_FILE" workflow component \
    --id round1 set-source-digest --digest "$ROUND1_DIGEST"

# round1-circuit needs no host capabilities — round_id derives from
# `cron.trigger_time.nanos / 30_000_000_000` and signer_value from
# `wall_clock::now().nanos % 1000`. Wall clock is a WASI 0.2
# capability the warpdrive-vectr world already grants; no http/fs.

warpdrive-cli service -f "$SERVICE_FILE" workflow submit \
    --id round1 set-aggregator

warpdrive-cli service -f "$SERVICE_FILE" workflow submit \
    --id round1 component set-source-digest --digest "$AGG_DIGEST"

# The aggregator reads two config values from the service spec:
#   chain             where the SubmitAction lands (= composer's chain,
#                     = the trigger chain in this example since the
#                     composer hosts both the r1ready emitter and the
#                     verify_xlm sink)
#   service_handler   the Soroban contract id of the composer; the
#                     aggregator builds a `verify_xlm` invocation
#                     targeting this address with the Round1-tagged
#                     payload bytes from the circuit.
warpdrive-cli service -f "$SERVICE_FILE" workflow submit \
    --id round1 component config \
    --values "chain=$TRIGGER_CHAIN" \
    --values "service_handler=$COMPOSER"

# ── workflow 2: round2 (r1ready event → round2-circuit → aggregator → composer) ─

warpdrive-cli service -f "$SERVICE_FILE" workflow add --id round2

# Subscribe to the composer's `r1ready` event. Two topic segments:
#   topic[0] = ScVal::Symbol("r1ready")   — exact-match symbol filter
#   topic[1] = ScVal::U64(round_id)       — wildcard (any round_id matches)
# The matching shape comes back to the circuit as
# TriggerData::StellarContractEvent with both topic segments and the
# event value (the Round1Bundle as ScVal::Map) still in ScVal form
# for the circuit to decode.
warpdrive-cli service -f "$SERVICE_FILE" workflow trigger \
    --id round2 set-stellar \
    --contract-id "$COMPOSER" \
    --chain "$TRIGGER_CHAIN" \
    --topic-symbol r1ready \
    --topic wildcard

warpdrive-cli service -f "$SERVICE_FILE" workflow component \
    --id round2 set-source-digest --digest "$ROUND2_DIGEST"

# round2-circuit also needs no host capabilities. The bundle ships in
# event.value as an ScVal::Map; no time-derived inputs of any kind
# (anything time-dependent in Round 2 would break the deterministic-
# salt invariant the host relies on to quorum-collapse the envelopes).

warpdrive-cli service -f "$SERVICE_FILE" workflow submit \
    --id round2 set-aggregator

warpdrive-cli service -f "$SERVICE_FILE" workflow submit \
    --id round2 component set-source-digest --digest "$AGG_DIGEST"

# Same aggregator, same composer — only the inner SubmissionPayload
# variant differs (Final instead of Round1).
warpdrive-cli service -f "$SERVICE_FILE" workflow submit \
    --id round2 component config \
    --values "chain=$TRIGGER_CHAIN" \
    --values "service_handler=$COMPOSER"

# ── service manager points at on-chain project_root ───────────────────

warpdrive-cli service -f "$SERVICE_FILE" manager set-stellar \
    --chain "$MANAGER_CHAIN" \
    --address "$PROJECT_ROOT"

# ── signature_kind: ed25519 / sep53 (matches the on-chain verifier) ───
#
# sep53 is the SEP-53 envelope-signing prefix the ed25519-verification
# contract expects on the signed bytes. Changing this here without
# changing the on-chain verifier will cause every submission to fail
# with `InvalidSignature` — for both rounds, since both submit through
# the same verify_xlm entry point.

warpdrive-cli service -f "$SERVICE_FILE" signature set ed25519 --prefix sep53

echo "wrote $SERVICE_FILE"
