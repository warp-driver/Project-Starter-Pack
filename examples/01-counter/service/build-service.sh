#!/usr/bin/env bash
#
# build-service.sh — assemble service/service.json declaratively via
# warpdrive-cli for 01-counter's single-workflow pipeline:
#
#   tick           cron @ "*/30 * * * * *"
#                  -> tick-circuit (no http, no fs — just emits ts)
#                  -> submit aggregator (chain + service_handler config)
#                  -> stellar-handler.verify_xlm → counter.tick(ts)
#
# Then sets the stellar service manager to project_root and appends the
# ed25519 / sep53 signature_kind via the warpdrive-cli signature subcommand
# (matches the on-chain ed25519-verification contract's prefix).
#
# Why only one workflow? The cookbook example is deliberately the smallest
# possible end-to-end pipeline. Real apps fan out from here:
#
#   hodlers-app           one workflow, but the trigger is a Stellar
#                         contract event (swap) instead of cron, and the
#                         circuit accumulates state across multiple
#                         events per logical swap.
#   oracle-demo           three workflows (fetch_prices / compute_twap /
#                         compute_median) chained via Stellar event
#                         composition + a 4th bridging Sepolia events.
#   phoenix-blend-pool    one workflow, multi-host quorum, production
#                         systemd deployment.
#
# Required inputs (relative to the example root):
#
#   out/handler.json      { "handler": "C..." }                (from `task deploy-handler`)
#   out/deploy.json       { "contracts": { "project_root": "C...", ... } }
#                                                              (from `task deploy-middleware`)
#   out/tick-circuit.digest    64-hex content digest           (from `task upload-tick-circuit`)
#   out/aggregator.digest      64-hex content digest           (from `task upload-aggregator`)
#   SERVICE_FILE          target path (default: service/service.json)
#   TRIGGER_CHAIN         chain key the circuit submits envelopes from
#                         (default: stellar:testnet — the handler lives there)
#   MANAGER_CHAIN         chain key the service manager (project_root)
#                         lives on (default: stellar:testnet)
#   CRON_SCHEDULE         cron expression (default: every 30 s for
#                         demo responsiveness; tighten or relax to taste).

set -euo pipefail

SERVICE_FILE="${SERVICE_FILE:-service/service.json}"
TRIGGER_CHAIN="${TRIGGER_CHAIN:-stellar:testnet}"
MANAGER_CHAIN="${MANAGER_CHAIN:-stellar:testnet}"
CRON_SCHEDULE="${CRON_SCHEDULE:-*/30 * * * * *}"

# ── prerequisite files ────────────────────────────────────────────────

for f in out/handler.json out/deploy.json \
         out/tick-circuit.digest out/aggregator.digest; do
    test -s "$f" || { echo "missing $f — run the prerequisite task first" >&2; exit 1; }
done

HANDLER=$(jq -r .handler out/handler.json)
PROJECT_ROOT=$(jq -r .contracts.project_root out/deploy.json)
TICK_DIGEST=$(cat out/tick-circuit.digest)
AGG_DIGEST=$(cat out/aggregator.digest)

for v in HANDLER PROJECT_ROOT TICK_DIGEST AGG_DIGEST; do
    test -n "${!v}" && test "${!v}" != "null" \
        || { echo "$v is empty — check the corresponding artefact under out/" >&2; exit 1; }
done

# ── initialise ─────────────────────────────────────────────────────────

mkdir -p "$(dirname "$SERVICE_FILE")"
rm -f "$SERVICE_FILE"

warpdrive-cli service -f "$SERVICE_FILE" init --name counter

# ── workflow: tick (cron → tick-circuit → aggregator → handler) ───────

warpdrive-cli service -f "$SERVICE_FILE" workflow add --id tick

warpdrive-cli service -f "$SERVICE_FILE" workflow trigger \
    --id tick set-cron \
    --schedule "$CRON_SCHEDULE"

warpdrive-cli service -f "$SERVICE_FILE" workflow component \
    --id tick set-source-digest --digest "$TICK_DIGEST"

# tick-circuit needs no host capabilities — no HTTP, no filesystem, no
# keyvalue. Cron supplies the timestamp directly in the trigger payload
# and the circuit serialises it straight into a TickPayload. Add
# `workflow component permissions --http-hosts ...` here if a fork
# ever needs to enrich the payload from an external source.

warpdrive-cli service -f "$SERVICE_FILE" workflow submit \
    --id tick set-aggregator

warpdrive-cli service -f "$SERVICE_FILE" workflow submit \
    --id tick component set-source-digest --digest "$AGG_DIGEST"

# The aggregator reads two config values from the service spec:
#   chain             where the SubmitAction lands (= the handler's chain)
#   service_handler   the Soroban contract id of the stellar-handler;
#                     the aggregator builds a `verify_xlm` invocation
#                     targeting this address.
warpdrive-cli service -f "$SERVICE_FILE" workflow submit \
    --id tick component config \
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
