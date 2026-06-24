## 01-counter

The smallest end-to-end WarpDrive integration: a 30-second cron tick
turns into a quorum-signed `XlmEnvelope`, gets verified on Stellar
testnet via the audited `ed25519-verification` contract, and lands in
a `Counter.tick(ts)` call. Nothing about it is interesting on its own
— it exists so the deploy / wire-service plumbing has something to
operate on. Real apps swap the cron trigger for a Stellar event, an
EVM event, or a multi-round composition event, and replace the
`TickPayload` with whatever shape their handler expects. Everything
else stays the same.

## Architecture

```
cron tick (*/30 * * * * *)
   │
   ▼
┌──────────────────────────────────────────────────────────────┐
│ components/tick-circuit   - WASI 0.2                         │
│   decodes cron trigger, emits XDR ScMap                      │
│   payload: TickPayload { ts: u64 }                           │
│   event_id_salt: ts.to_le_bytes() (deterministic across ops) │
└──────────────────────────────────────────────────────────────┘
   │
   ▼
┌──────────────────────────────────────────────────────────────┐
│ components/aggregator     - WASI 0.2                         │
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
│     decode TickPayload { ts } from envelope.payload          │
│     invoke counter.tick(ts)                                  │
└──────────────────────────────────────────────────────────────┘
   │
   ▼
┌──────────────────────────────────────────────────────────────┐
│ contracts/counter         (Soroban, testnet)                 │
│   tick(ts)        require_auth(handler), inc count, store ts │
│   count()         public query                               │
│   last_tick()     public query                               │
└──────────────────────────────────────────────────────────────┘
```

The trust chain is **operator quorum → `handler.verify_xlm` →
`counter.tick`**. The counter contract `require_auth`s the registered
handler address, so the only path that advances the counter is via
the handler, and the only path through the handler is via a quorum
signature checked by `ed25519-verification`.

## Contracts

| Crate | Role |
|---|---|
| `contracts/counter` | The payload sink. `tick(ts)` increments a counter and stores the latest timestamp, but only when called by the registered handler. `count()` and `last_tick()` are open queries. |
| `contracts/stellar-handler` | Cookie-cutter handler. Decodes `XlmEnvelope`, calls `ed25519-verification.try_verify` to enforce quorum, decodes a `TickPayload { ts: u64 }` from `envelope.payload`, then invokes `counter.tick(ts)`. ~150 LOC. |
| `contracts/ed25519-security` | Vendored verbatim from [warpdrive-contracts](https://github.com/warp-driver/warpdrive-contracts). Holds the operator signer set + threshold. Admin is `project_root` once `deploy-middleware` finishes. |
| `contracts/ed25519-verification` | Vendored verbatim. Looks up signers on `ed25519-security` and runs the BFT quorum verification over the SEP-53 envelope hash. |

The two `ed25519-*` contracts are copies of the audited contracts in
`warpdrive-contracts`. No git submodule, no version drift — when a
new release of `warpdrive-contracts` lands, regenerate the vendored
copies (see `vendor/README.md` at the pack root).

## Components

| Crate | Role |
|---|---|
| `components/tick-circuit` | WASI 0.2 circuit. Receives the cron trigger, builds a `TickPayload { ts: u64 }` as a hand-built XDR `ScMap` (single key `"ts"`, alphabetically sorted), returns a `WasmResponse` whose `event_id_salt` is `ts.to_le_bytes()`. The salt is the same across every operator for the same scheduled tick, so the host quorum-collapses their identical signatures into one. |
| `components/aggregator` | WASI 0.2 aggregator. Standard Stellar `SubmitAction` emitter — reads `chain` and `service_handler` from the service spec's `component config` and constructs the `verify_xlm` invocation. Byte-identical to the aggregator used by hodlers-app, oracle-demo, and phoenix-blend-pool. |

The wire format between circuit and handler is the locked
cross-agent contract:

```rust
// Soroban side:
#[contracttype]
pub struct TickPayload { pub ts: u64 }

// Circuit side: an ScMap with one entry, alphabetically sorted by
// (symbolic) key — there's only one key here, so the sort is trivial,
// but the convention is what the handler decodes against.
ScVal::Map(ScMap(vec![ScMapEntry {
    key:  ScVal::Symbol("ts".try_into().unwrap()),
    val:  ScVal::U64(unix_secs),
}]))
```

## Quickstart

A funded testnet key and one warpdrive node on your laptop. About
five minutes from clone to first signed counter tick. Multi-op
walkthrough below.

```bash
# 0. Working directory.
cd Project-Starter-Pack/examples/01-counter

# 1. WIT deps (one-time per clone). Fetches warpdrive-vectr and the
#    aggregator world into wit-definitions/wit/deps/.
task fetch-wit

# 2. Mint a funded testnet deployer + one operator mnemonic into .env.
#    Re-run with OPERATORS=N for N operator keys.
../../scripts/bootstrap-keys.sh > .env
set -a; source .env; set +a

# 3. Build contracts + components, deploy ed25519 stack, counter,
#    handler. Writes out/deploy.json + out/counter.json + out/handler.json.
task deploy

# 4. Start the operator. Leave running in a SECOND terminal:
#       cd Project-Starter-Pack/examples/01-counter
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

The cron fires every 30 seconds. Watch the counter advance:

```bash
COUNTER=$(jq -r .counter out/counter.json)
stellar contract invoke --id "$COUNTER" \
  --rpc-url https://soroban-testnet.stellar.org \
  --network-passphrase "Test SDF Network ; September 2015" \
  --source "$DEPLOYER_SECRET" \
  -- count
# → 1, then 2, then 3 …
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
so two nodes coexist on one host without further config.

## Where to go next

Three real WarpDrive apps the example is distilled from. Each
demonstrates one extra dimension the counter deliberately leaves out
— jump to the one whose shape matches what you want to build.

| Project | What it adds on top of the counter |
|---|---|
| [hodlers-app](https://github.com/warp-driver/hodlers-app) | Stellar mainnet contract-event trigger (Phoenix XLM-USDC swap) instead of cron. CAS-accumulator state in `wasi:keyvalue/atomics` for exactly-once delivery when one logical swap fires 8 sub-events. Smallest real app — closest to the counter in scope. |
| [oracle-demo](https://github.com/warp-driver/oracle-demo) | Three-round composition (cron → `twapreq` event → `r2ready` event), plus a Sepolia EVM-bridge workflow. Two-operator quorum, MetaMask trigger path, React + Vite frontend wired against both Freighter and Sepolia wallets. The reference for multi-round and EVM patterns. |
| [phoenix-blend-pool](https://github.com/warp-driver/phoenix-blend-pool) | Blend pool rebalancer with full production multi-host quorum: dedicated `DEPLOY.md`, `OPERATORS.md`, `ARCHITECTURE.md`, systemd unit templates. The reference for taking a Starter Pack fork into production. |

When the answer to "how do I do X" is "go look at oracle-demo /
hodlers-app / phoenix-blend-pool", the pack will say so explicitly
rather than re-explaining a pattern that already exists upstream.

See `ARCHITECTURE.md` at the pack root for the Vectr / circuit /
aggregator / handler / security explainer, and `DEPLOY.md` for the
detailed single-op and multi-op walkthroughs covering troubleshooting,
log lines to look for, and what to do when the operator-quorum
verification fails on chain.
