#!/usr/bin/env bash
#
# op-env.sh — resolve the OP env var (1..N, default 1) into the
# concrete settings the warpdrive node for that operator listens on:
#
#   OP_INDEX    1, 2, …
#   OP_PORT     HTTP port, 8000 + 10*(OP_INDEX - 1)
#   OP_P2P_PORT libp2p port, 9000 + 10*(OP_INDEX - 1)
#   OP_URL      http://127.0.0.1:$OP_PORT
#   OP_DATA     out/node-data        (OP=1)
#               out/node-data-N      (OP>1)
#   OP_MNEMONIC the BIP39 mnemonic this operator signs with —
#               WARPDRIVE_SIGNING_MNEMONIC for OP=1,
#               WARPDRIVE_SIGNING_MNEMONIC_N for OP>1.
#
# Source from inside a `task` shell block:
#     . scripts/op-env.sh
# Sourced, so `set -euo pipefail` here persists in the caller's shell —
# that's fine: every Taskfile block we source from already runs under -e
# and benefits from the stricter -u/-o pipefail. Behaviour either way:
# missing mnemonic env var trips loud via -u so the caller can't
# silently sign with the wrong key.

set -euo pipefail

OP_INDEX="${OP:-1}"
OP_PORT=$((8000 + 10 * (OP_INDEX - 1)))
OP_P2P_PORT=$((9000 + 10 * (OP_INDEX - 1)))
OP_URL="http://127.0.0.1:$OP_PORT"
if [ "$OP_INDEX" -eq 1 ]; then
    OP_DATA="out/node-data"
    OP_MNEMONIC_VAR="WARPDRIVE_SIGNING_MNEMONIC"
else
    OP_DATA="out/node-data-$OP_INDEX"
    OP_MNEMONIC_VAR="WARPDRIVE_SIGNING_MNEMONIC_$OP_INDEX"
fi
# Indirect expansion — the variable named in $OP_MNEMONIC_VAR is what
# the operator's node will HD-derive its signing key from.
eval "OP_MNEMONIC=\${$OP_MNEMONIC_VAR:-}"

export OP_INDEX OP_PORT OP_P2P_PORT OP_URL OP_DATA OP_MNEMONIC_VAR OP_MNEMONIC
