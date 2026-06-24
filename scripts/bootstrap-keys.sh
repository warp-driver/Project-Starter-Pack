#!/usr/bin/env bash
#
# bootstrap-keys.sh — mint the stellar identities a Starter Pack
# project needs and print them as a .env-compatible block on stdout.
#
# Usage:
#   ./scripts/bootstrap-keys.sh > .env
#   set -a; source .env; set +a
#
# See DEPLOY.md for how the emitted vars are consumed by the Taskfile
# and how to roll the keys for a real (non-throwaway) deployment.
#
# Default quickstart runs TWO operators on the same host (see DEPLOY.md
# § Multi-operator deployment), so we mint two BIP39 mnemonics in
# addition to the funded testnet deployer key. Each operator's
# warpdrive node uses its own mnemonic via WARPDRIVE_SIGNING_MNEMONIC
# (op 1) or WARPDRIVE_SIGNING_MNEMONIC_2 (op 2). Bumping OPERATORS
# adds more here — keep the names in lock-step with the Taskfile.
#
# This wipes any prior identities under these names in the local
# `stellar keys` store; they are scratch keys, regenerated on every run.
#
# --- extension pattern --------------------------------------------------
# An example that needs an optional env var should emit it uncommented
# when set and commented-out otherwise so the user can see the slot
# exists. Use this idiom (same shape oracle-demo carries for its
# CoinGecko / Sepolia keys):
#
#   if [ -n "${MY_API_KEY:-}" ]; then
#       echo "MY_API_KEY=$MY_API_KEY"
#   else
#       echo "# MY_API_KEY=...   # uncomment + paste your key"
#   fi
# ------------------------------------------------------------------------

set -euo pipefail

OPERATORS="${OPERATORS:-2}"

stellar keys rm deployer 2>/dev/null || true
stellar keys generate deployer --fund --network testnet

DEPLOYER_SECRET=$(stellar keys show deployer)
DEPLOYER_ADDRESS=$(stellar keys address deployer)

{
    echo "DEPLOYER_SECRET=$DEPLOYER_SECRET"
    echo "DEPLOYER_ADDRESS=$DEPLOYER_ADDRESS"
    echo "OPERATORS=$OPERATORS"
}

for op in $(seq 1 "$OPERATORS"); do
    name="warpdrive-operator-$op"
    stellar keys rm "$name" 2>/dev/null || true
    stellar keys generate "$name"
    phrase=$(stellar keys show "$name" --phrase)
    suffix=""
    [ "$op" -gt 1 ] && suffix="_$op"
    echo "WARPDRIVE_SIGNING_MNEMONIC${suffix}=\"$phrase\""
done
