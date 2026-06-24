## 02-event-watcher

The smallest WarpDrive integration whose trigger is a Soroban contract
event — the most common production shape, and the natural next step
after 01-counter. A wallet calls `message_board.publish("hello")`; the
contract emits an event the warpdrive operators are subscribed to;
they decode it into a `RecordPayload { msg, msg_id }`, sign it by
quorum, and the same handler-verify-and-dispatch chain you saw in the
counter writes the verified message back into the same contract via
`record_signed`. One source contract owns both ends of the loop —
`publish` and `record_signed` are sibling methods on `MessageBoard` —
so the demo runs self-contained on testnet, no mainnet dependency,
no second project to deploy.

## Architecture

```
wallet (any G-address) ── stellar contract invoke ──┐
                                                    │
                                                    ▼
┌──────────────────────────────────────────────────────────────┐
│ contracts/message-board (Soroban, testnet)                   │
│   publish(msg) -> u64                                        │
│     emits Soroban event:                                     │
│       topic[0] = Symbol("msg")                               │
│       topic[1] = U64(msg_id)                                 │
│       value    = String(msg)                                 │
└──────────────────────────────────────────────────────────────┘
   │  warpdrive operators observe via the Stellar chain-event
   │  subscription pinned to this contract id
   ▼
┌──────────────────────────────────────────────────────────────┐
│ components/event-watcher-circuit   - WASI 0.2                │
│   decodes ScVal::Symbol("msg") topic, U64 topic, String value│
│   emits XDR ScMap                                            │
│   payload: RecordPayload { msg: String, msg_id: u64 }        │
│   event_id_salt: msg_id.to_le_bytes() (deterministic across  │
│   operators — same on-chain event, same salt, quorum-collapse│
│   merges the identical signatures into one submission)       │
└──────────────────────────────────────────────────────────────┘
   │
   ▼
┌──────────────────────────────────────────────────────────────┐
│ components/aggregator              - WASI 0.2                │
│   emits one Stellar SubmitAction targeting service_handler   │
│   chain + service_handler come from service.json config      │
└──────────────────────────────────────────────────────────────┘
   │  ed25519 quorum over libp2p; one operator wins the
   │  submission race, the rest back off on EventAlreadySeen
   ▼
┌──────────────────────────────────────────────────────────────┐
│ contracts/stellar-handler (Soroban, testnet)                 │
│   verify_xlm(envelope_bytes, sig_data):                      │
│     ed25519-verification.try_verify (quorum check)           │
│     decode RecordPayload from envelope.payload               │
│     invoke message_board.record_signed(msg_id, msg)          │
└──────────────────────────────────────────────────────────────┘
   │
   ▼
┌──────────────────────────────────────────────────────────────┐
│ contracts/message-board   (same contract as the source)      │
│   record_signed(msg_id, msg)                                 │
│     require_auth(handler), idempotent insert into ledger     │
│   recorded(msg_id) -> Option<String>   public query          │
└──────────────────────────────────────────────────────────────┘
```

The trust chain is **wallet → message-board event → operator quorum →
`handler.verify_xlm` → `message_board.record_signed`**. The
`record_signed` method `require_auth`s the registered handler
address, so the only path that lands a verified record is via the
handler, and the only path through the handler is via a quorum
signature checked by `ed25519-verification`. Anyone can call
`publish` (that's the point — it's the user-facing entry point), but
no one can forge a signed record.

## Contracts

| Crate | Role |
|---|---|
| `contracts/message-board` | Both ends of the loop. `publish(msg)` is open — any wallet calls it, the contract emits the indexed Soroban event, returns the assigned `msg_id`. `record_signed(msg_id, msg)` is gated by `require_auth(handler)` and writes the verified payload into the ledger. `recorded(msg_id)` is an open query. |
| `contracts/stellar-handler` | Cookie-cutter handler. Decodes `XlmEnvelope`, calls `ed25519-verification.try_verify` to enforce quorum, decodes a `RecordPayload { msg, msg_id }` from `envelope.payload`, then invokes `message_board.record_signed(msg_id, msg)`. ~150 LOC. |
| `contracts/ed25519-security` | Vendored verbatim from [warpdrive-contracts](https://github.com/warp-driver/warpdrive-contracts). Holds the operator signer set + threshold. Admin is `project_root` once `deploy-middleware` finishes. |
| `contracts/ed25519-verification` | Vendored verbatim. Looks up signers on `ed25519-security` and runs the BFT quorum verification over the SEP-53 envelope hash. |

The two `ed25519-*` contracts are copies of the audited contracts in
`warpdrive-contracts`. No git submodule, no version drift — when a
new release of `warpdrive-contracts` lands, regenerate the vendored
copies (see `vendor/README.md` at the pack root).

## Components

| Crate | Role |
|---|---|
| `components/event-watcher-circuit` | WASI 0.2 circuit. Receives `TriggerData::StellarContractEvent` from the message-board subscription, validates `topic_segments[0]` decodes to `ScVal::Symbol("msg")`, extracts `msg_id: u64` from `topic_segments[1]`, decodes the event value as `ScVal::String` → UTF-8 → `String`, then hand-builds a `RecordPayload { msg, msg_id }` as an XDR `ScMap` with alphabetically sorted keys (`msg` before `msg_id` — Soroban's encoding order, which the handler decodes against). `event_id_salt = msg_id.to_le_bytes()`: same across every operator for the same on-chain event, so the host quorum-collapses their identical signatures into one. |
| `components/aggregator` | WASI 0.2 aggregator. Standard Stellar `SubmitAction` emitter — reads `chain` and `service_handler` from the service spec's `component config` and constructs the `verify_xlm` invocation. Byte-identical to the aggregator used by 01-counter, hodlers-app, oracle-demo, and phoenix-blend-pool. |

The wire format between circuit and handler is the locked
cross-agent contract:

```rust
// Soroban side:
#[contracttype]
pub struct RecordPayload {
    pub msg: String,
    pub msg_id: u64,
}

// Circuit side: an ScMap with two entries, alphabetically sorted by
// (symbolic) key. `msg` precedes `msg_id` — that's what the handler's
// `RecordPayload::from_xdr` decodes against, byte for byte.
ScVal::Map(ScMap(vec![
    ScMapEntry { key: ScVal::Symbol("msg".try_into().unwrap()),
                 val: ScVal::String(msg.try_into().unwrap()) },
    ScMapEntry { key: ScVal::Symbol("msg_id".try_into().unwrap()),
                 val: ScVal::U64(msg_id) },
]))
```

## Quickstart

A funded testnet key and one warpdrive node on your laptop. About
five minutes from clone to first signed record. Multi-op walkthrough
below.

```bash
# 0. Working directory.
cd Project-Starter-Pack/examples/02-event-watcher

# 1. WIT deps (one-time per clone). Fetches warpdrive-vectr and the
#    aggregator world into wit-definitions/wit/deps/.
task fetch-wit

# 2. Mint a funded testnet deployer + one operator mnemonic into .env.
#    Re-run with OPERATORS=N for N operator keys.
../../scripts/bootstrap-keys.sh > .env
set -a; source .env; set +a

# 3. Build contracts + components, deploy ed25519 stack, message-board,
#    handler, then wire the two together. Writes out/deploy.json +
#    out/message-board.json + out/handler.json.
task deploy

# 4. Start the operator. Leave running in a SECOND terminal:
#       cd Project-Starter-Pack/examples/02-event-watcher
#       set -a; source .env; set +a
#       task run-node
#    Wait for "Stellar chain [stellar:testnet] is healthy" plus
#    "HTTP server bound to port 8000".

# 5. Upload components, build service.json, activate it on the node.
task wire-service

# 6. Register this operator's pubkey on ed25519-security at the
#    default 1/1 threshold (any single quorum sig suffices).
task register-signer
```

The pipeline is event-driven: nothing happens until a wallet calls
`publish`. Fire one yourself and watch it round-trip:

```bash
MESSAGE_BOARD=$(jq -r .message_board out/message-board.json)
RPC=https://soroban-testnet.stellar.org
PASS="Test SDF Network ; September 2015"

# Publish — emits the Soroban event the operator is subscribed to.
# Returns the assigned msg_id (0 for the very first publish).
stellar contract invoke --id "$MESSAGE_BOARD" \
  --rpc-url "$RPC" --network-passphrase "$PASS" \
  --source "$DEPLOYER_SECRET" \
  -- publish --msg '"hello"'
# → 0

# Wait ~5–15 s for the warpdrive node to observe the event, sign it,
# and land record_signed on chain. Watch the run-node log for
# "submitted Stellar tx".
sleep 15

# Read the verified record back. The handler-gated write is the only
# path that could have populated this slot.
stellar contract invoke --id "$MESSAGE_BOARD" \
  --rpc-url "$RPC" --network-passphrase "$PASS" \
  --source "$DEPLOYER_SECRET" \
  -- recorded --msg_id 0
# → "hello"
```

### Multi-operator (OPERATORS=2)

Same flow, two terminals for the two nodes:

```bash
# 1. Mint two mnemonics + funded deployer.
OPERATORS=2 ../../scripts/bootstrap-keys.sh > .env
set -a; source .env; set +a

# 2. Deploy as before.
OPERATORS=2 task deploy

# 3. Terminal A: op 1 on :8000.
OPERATORS=2 task run-node
# 4. Terminal B: op 2 on :8010.
OP=2 OPERATORS=2 task run-node

# 5. Wire both nodes in one shot (uploads + service.json + register on
#    every op).
OPERATORS=2 task wire-service

# 6. Sync the ed25519-security signer set to exactly both operators,
#    apply threshold (default 1/1; set 2/2 for a strict quorum).
OPERATORS=2 THRESHOLD_NUM=2 THRESHOLD_DEN=2 task register-signers
```

`run-node` materialises a per-operator copy of `warpdrive.toml` under
`out/op${N}-home/` with `port` and `listen_port` bumped by `10*(N-1)`,
so two nodes coexist on one host without further config. With 2/2
quorum both nodes must have signed before `record_signed` lands; the
quorum-collapse on `event_id_salt = msg_id.to_le_bytes()` means the
chain still sees exactly one submission, not two.

## Where to go next

Three real WarpDrive apps the example is distilled from. Each
demonstrates one extra dimension the watcher deliberately leaves out
— jump to the one whose shape matches what you want to build.

| Project | What it adds on top of the event-watcher |
|---|---|
| [hodlers-app](https://github.com/warp-driver/hodlers-app) | Mainnet event source (Phoenix XLM-USDC swap on Stellar mainnet). CAS-accumulator state in `wasi:keyvalue/atomics` for exactly-once delivery when one logical swap fires 8 sub-events. Smallest real app — the natural next step once you've replaced this example's `message-board` with whatever contract you actually want to watch. |
| [oracle-demo](https://github.com/warp-driver/oracle-demo) | Three-round composition (cron → `twapreq` event → `r2ready` event), plus a Sepolia EVM-bridge workflow. Two-operator quorum, MetaMask trigger path, React + Vite frontend wired against both Freighter and Sepolia wallets. The reference for multi-round and EVM patterns. |
| [phoenix-blend-pool](https://github.com/warp-driver/phoenix-blend-pool) | Blend pool rebalancer with full production multi-host quorum: dedicated `DEPLOY.md`, `OPERATORS.md`, `ARCHITECTURE.md`, systemd unit templates. The reference for taking a Starter Pack fork into production. |

See `ARCHITECTURE.md` at the pack root for the Vectr / circuit /
aggregator / handler / security explainer, and `DEPLOY.md` for the
detailed single-op and multi-op walkthroughs covering troubleshooting,
log lines to look for, and what to do when the operator-quorum
verification fails on chain.
