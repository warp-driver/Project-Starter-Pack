#!/usr/bin/env bash
#
# build-service.sh — assemble service/service.json declaratively via
# warpdrive-cli for 02-event-watcher's single-workflow pipeline:
#
#   record         stellar event (message-board, topic[0]=symbol "msg", topic[1]=*)
#                  -> event-watcher-circuit (no http, no fs — just decodes the event)
#                  -> submit aggregator (chain + service_handler config)
#                  -> stellar-handler.verify_xlm → message-board.record_signed(msg_id, msg)
#
# Then sets the stellar service manager to project_root and appends the
# ed25519 / sep53 signature_kind via the warpdrive-cli signature subcommand
# (matches the on-chain ed25519-verification contract's prefix).
#
# Why a Stellar event instead of cron? This is the most common production
# trigger shape: an on-chain user action (here: a `publish` call from any
# wallet) emits a Soroban event the warpdrive operators are subscribed to,
# they decode it deterministically, sign by quorum, and a handler dispatches
# the verified payload back into the application contract. The full loop
# lives on a single contract — `publish` and `record_signed` are both
# message-board methods — so the demo runs self-contained on testnet
# with no second project to deploy. Production fans out from here:
#
#   hodlers-app           same shape, but the trigger contract is
#                         Phoenix's mainnet AMM and the circuit accumulates
#                         multiple swap-sub-events per logical swap (CAS in
#                         wasi:keyvalue/atomics for exactly-once delivery).
#   oracle-demo           three workflows (fetch_prices / compute_twap /
#                         compute_median) chained via Stellar event
#                         composition + a 4th bridging Sepolia events.
#   phoenix-blend-pool    one workflow, multi-host quorum, production
#                         systemd deployment.
#
# Required inputs (relative to the example root):
#
#   out/handler.json          { "handler": "C..." }              (from `task deploy-handler`)
#   out/message-board.json    { "message_board": "C..." }        (from `task deploy-message-board`)
#   out/deploy.json           { "contracts": { "project_root": "C...", ... } }
#                                                                (from `task deploy-middleware`)
#   out/event-watcher-circuit.digest   64-hex content digest     (from `task upload-event-watcher-circuit`)
#   out/aggregator.digest              64-hex content digest     (from `task upload-aggregator`)
#   SERVICE_FILE          target path (default: service/service.json)
#   TRIGGER_CHAIN         chain key the circuit subscribes to AND submits
#                         envelopes from — same chain because message-board
#                         hosts both ends of the loop
#                         (default: stellar:testnet)
#   MANAGER_CHAIN         chain key the service manager (project_root)
#                         lives on (default: stellar:testnet)

set -euo pipefail

SERVICE_FILE="${SERVICE_FILE:-service/service.json}"
TRIGGER_CHAIN="${TRIGGER_CHAIN:-stellar:testnet}"
MANAGER_CHAIN="${MANAGER_CHAIN:-stellar:testnet}"

# ── prerequisite files ────────────────────────────────────────────────

for f in out/handler.json out/message-board.json out/deploy.json \
         out/event-watcher-circuit.digest out/aggregator.digest; do
    test -s "$f" || { echo "missing $f — run the prerequisite task first" >&2; exit 1; }
done

HANDLER=$(jq -r .handler out/handler.json)
MESSAGE_BOARD=$(jq -r .message_board out/message-board.json)
PROJECT_ROOT=$(jq -r .contracts.project_root out/deploy.json)
CIRCUIT_DIGEST=$(cat out/event-watcher-circuit.digest)
AGG_DIGEST=$(cat out/aggregator.digest)

for v in HANDLER MESSAGE_BOARD PROJECT_ROOT CIRCUIT_DIGEST AGG_DIGEST; do
    test -n "${!v}" || { echo "$v empty after parsing prerequisites" >&2; exit 1; }
done

# ── initialise ─────────────────────────────────────────────────────────

mkdir -p "$(dirname "$SERVICE_FILE")"
rm -f "$SERVICE_FILE"

warpdrive-cli service -f "$SERVICE_FILE" init --name event-watcher

# ── workflow: record (stellar event → event-watcher-circuit → aggregator → handler) ─

warpdrive-cli service -f "$SERVICE_FILE" workflow add --id record

# Subscribe to message-board's `msg` event. Two topic segments:
#   topic[0] = ScVal::Symbol("msg")   — exact-match symbol filter
#   topic[1] = ScVal::U64(msg_id)     — wildcard (any msg_id matches)
# `set-stellar` translates this into the operator's chain-event filter; the
# matching shape comes back to the circuit as TriggerData::StellarContractEvent
# with both topic segments and the event value (the published String) still
# in ScVal form for the circuit to decode.
warpdrive-cli service -f "$SERVICE_FILE" workflow trigger \
    --id record set-stellar \
    --contract-id "$MESSAGE_BOARD" \
    --chain "$TRIGGER_CHAIN" \
    --topic-symbol msg \
    --topic wildcard

warpdrive-cli service -f "$SERVICE_FILE" workflow component \
    --id record set-source-digest --digest "$CIRCUIT_DIGEST"

# event-watcher-circuit needs no host capabilities — no HTTP, no filesystem,
# no keyvalue. The Soroban event carries every field the RecordPayload
# needs (msg_id from topic[1], msg from value) and the circuit serialises
# it straight into a hand-built XDR ScMap. Add
# `workflow component permissions --http-hosts ...` here if a fork ever
# needs to enrich the payload from an external source.

warpdrive-cli service -f "$SERVICE_FILE" workflow submit \
    --id record set-aggregator

warpdrive-cli service -f "$SERVICE_FILE" workflow submit \
    --id record component set-source-digest --digest "$AGG_DIGEST"

# The aggregator reads two config values from the service spec:
#   chain             where the SubmitAction lands (= the handler's chain,
#                     = the trigger chain in this example since message-board
#                     hosts the whole loop)
#   service_handler   the Soroban contract id of the stellar-handler;
#                     the aggregator builds a `verify_xlm` invocation
#                     targeting this address.
warpdrive-cli service -f "$SERVICE_FILE" workflow submit \
    --id record component config \
    --values "chain=$TRIGGER_CHAIN" \
    --values "service_handler=$HANDLER"

# ── service manager points at on-chain project_root ───────────────────

warpdrive-cli service -f "$SERVICE_FILE" manager set-stellar \
    --chain "$MANAGER_CHAIN" \
    --address "$PROJECT_ROOT"

# ── signature_kind: ed25519 / sep53 (matches the on-chain verifier) ───
#
# sep53 is the SEP-53 envelope-signing prefix the ed25519-verification
# contract expects on the signed bytes. Changing this here without
# changing the on-chain verifier will cause every submission to fail
# with `InvalidSignature`.

warpdrive-cli service -f "$SERVICE_FILE" signature set ed25519 --prefix sep53

echo "wrote $SERVICE_FILE"
