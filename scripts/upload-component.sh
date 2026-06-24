#!/usr/bin/env bash
# Upload a WASI component wasm to a specific warpdrive node and print its
# digest on stdout. `warpdrive-cli upload-component --warpdrive-endpoint`
# accepts the flag but ignores it (the flag is declared in args.rs but
# never threaded into Config::wavs_endpoint), so every upload via the CLI
# silently lands on the default :8000. We POST the bytes directly instead.
#
# Usage: upload-component.sh <endpoint> <wasm-path>
set -euo pipefail

ENDPOINT="$1"
WASM="$2"

test -f "$WASM" || { echo "wasm not found: $WASM" >&2; exit 1; }

RESPONSE="$(curl -sS --fail-with-body \
    -X POST "$ENDPOINT/dev/components" \
    -H "Content-Type: application/octet-stream" \
    --data-binary "@$WASM")"

DIGEST="$(printf '%s' "$RESPONSE" | jq -r .digest)"
test -n "$DIGEST" && test "$DIGEST" != "null" \
    || { echo "no digest in response: $RESPONSE" >&2; exit 1; }

printf '%s\n' "$DIGEST"
