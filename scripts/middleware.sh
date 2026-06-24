#!/usr/bin/env bash
# Run a `warpdrive-deployer` subcommand inside the
# warpdrive-stellar-middleware container, mounting ./out into /out so
# subcommands can read/write the shared deploy manifest there.
#
# Usage:
#   scripts/middleware.sh <subcommand> [--flag value ...]
#
# Example:
#   scripts/middleware.sh deploy --output-path /out/deploy.json --variant stellar
#
# Required env vars: DEPLOYER_SECRET (a funded testnet S-seed).
# Honoured env vars : RPC_URL, NETWORK_PASSPHRASE, MIDDLEWARE_IMAGE.
#
# The image entrypoint used to be a shell wrapper at /warpdrive/cli.sh.
# As of late 2025 it's been replaced by a Rust binary `warpdrive-deployer`
# on PATH; this script targets the new layout and works against any
# image tag that ships that binary.
#
# We use `--pull=missing`, NOT `--pull=always`: once an image is on
# disk we keep using it across runs so an upstream re-tag of `:latest`
# can't silently break a working local deploy mid-session. Update with
# an explicit `docker pull ghcr.io/warp-driver/warpdrive-stellar-middleware:latest`.

set -euo pipefail

: "${DEPLOYER_SECRET:?DEPLOYER_SECRET is required (a funded testnet S-seed)}"

RPC_URL="${RPC_URL:-https://soroban-testnet.stellar.org}"
NETWORK_PASSPHRASE="${NETWORK_PASSPHRASE:-Test SDF Network ; September 2015}"
MIDDLEWARE_IMAGE="${MIDDLEWARE_IMAGE:-ghcr.io/warp-driver/warpdrive-stellar-middleware:latest}"

if [[ $# -eq 0 ]]; then
    echo "usage: $0 <subcommand> [args...]" >&2
    echo "  e.g. $0 deploy --output-path /out/deploy.json --variant stellar" >&2
    exit 2
fi

mkdir -p out

exec docker run --rm \
    --pull=missing \
    --user "$(id -u):$(id -g)" \
    -e HOME=/tmp \
    -e RPC_URL="$RPC_URL" \
    -e NETWORK_PASSPHRASE="$NETWORK_PASSPHRASE" \
    -v "$PWD/out:/out" \
    --entrypoint warpdrive-deployer \
    "$MIDDLEWARE_IMAGE" \
    "$@" \
    --secret "$DEPLOYER_SECRET"
