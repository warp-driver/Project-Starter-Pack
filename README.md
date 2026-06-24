# WarpDrive Project Starter Pack

A scaffold for Soroban developers building a WarpDrive integration: one
tiny end-to-end example (cron → quorum-signed envelope → on-chain
counter), the security contracts that make on-chain quorum verification
work, and the deploy / wire-up scripts factored out of three real
WarpDrive apps in production. Clone it, run `./scripts/new-project.sh
01-counter ../my-thing`, and you have a working integration to iterate
on instead of a blank repo and a wiki tab.

## What this gives you

- **A working example** — `examples/01-counter/`. WASI 0.2
  tick-circuit + aggregator + Soroban handler + counter contract,
  driven by a 30-second cron, signed by an ed25519 operator quorum,
  settled on Stellar testnet. ~600 LOC end-to-end.
- **The security stack, vendored** — `vendor/contracts/ed25519-security`
  and `vendor/contracts/ed25519-verification`, byte-identical copies of
  the audited contracts from
  [warpdrive-contracts](https://github.com/warp-driver/warpdrive-contracts).
  No git submodule, no version drift, no upstream API surprises.
- **Deploy / wire-up scripts** — `bootstrap-keys.sh` (mints deployer +
  N operator mnemonics into `.env`), `op-env.sh` (`OP=N` → distinct
  port / data dir / mnemonic), `upload-component.sh` (POSTs to
  `/dev/components` directly, bypassing a broken cli flag),
  `middleware.sh` (wraps the `warpdrive-stellar-middleware` container),
  `new-project.sh` (forks any `examples/NN-name/` into a new repo and
  rewrites crate names).
- **Skeletons + pinned tooling** — `rust-toolchain.toml` pinned to
  Rust 1.95 with both `wasm32-wasip1` and `wasm32v1-none` targets, and
  `warpdrive.toml.template` with single-op + multi-op blocks.
- **The docs** — [`ARCHITECTURE.md`](./ARCHITECTURE.md) (how the
  Vectr / circuit / aggregator / handler / security pieces wire up),
  [`DEPLOY.md`](./DEPLOY.md) (single-op and multi-op walkthroughs,
  same shape as `hodlers-app/DEPLOY.md`),
  [`CONTRIBUTING.md`](./CONTRIBUTING.md) (where to ask for help, how
  to land a new example).

## Choose your path

- **I want to understand the architecture first** →
  [`ARCHITECTURE.md`](./ARCHITECTURE.md).
- **I want to deploy and run something** →
  [`DEPLOY.md`](./DEPLOY.md), then the quickstart below.
- **I want to fork and start coding** →
  `./scripts/new-project.sh 01-counter ../my-counter` and start
  editing `contracts/` and `components/`.

## 5-minute quickstart

A funded testnet key plus one warpdrive node on your laptop. About
five minutes from clone to first signed counter tick.

```bash
# 0. Clone the pack and pick an example to work on.
git clone https://github.com/warp-driver/Project-Starter-Pack
cd Project-Starter-Pack/examples/01-counter

# 1. WIT deps (one-time per clone). Fetches warpdrive-vectr and the
#    aggregator world from wa.dev into wit-definitions/wit/deps/.
task fetch-wit

# 2. Mint a funded testnet deployer + one operator mnemonic. Writes
#    DEPLOYER_SECRET / DEPLOYER_ADDRESS / WARPDRIVE_SIGNING_MNEMONIC
#    to .env. Re-run with OPERATORS=N for N operator keys.
../../scripts/bootstrap-keys.sh > .env
set -a; source .env; set +a

# 3. Build the contracts + components and deploy the on-chain stack
#    (ed25519-security, ed25519-verification, counter, stellar-handler).
#    Output: out/deploy.json + out/counter.json.
task deploy

# 4. Start the operator. Leave running in a SECOND terminal.
#       cd Project-Starter-Pack/examples/01-counter
#       set -a; source .env; set +a
#       task run-node
#    Wait for "Stellar chain [stellar:testnet] is healthy" and
#    "HTTP server bound to port 8000".

# 5. Upload components, build service.json (one workflow: cron →
#    tick-circuit → aggregator → handler), activate it on the node.
task wire-service

# 6. Register this operator's pubkey on ed25519-security at the
#    default 1/1 threshold (any single quorum sig is sufficient).
task register-signer
```

The cron fires every 30 seconds. Wait one cycle, then read the counter:

```bash
COUNTER=$(jq -r .counter out/counter.json)
stellar contract invoke --id "$COUNTER" \
  --rpc-url https://soroban-testnet.stellar.org \
  --network-passphrase "Test SDF Network ; September 2015" \
  --source $DEPLOYER_SECRET \
  -- count
# → 1, then 2, then 3 …
```

If any task complains about a missing env var, you forgot
`set -a; source .env; set +a` in that shell. Full step-by-step
(prerequisites, multi-op variant, troubleshooting) is in
[`DEPLOY.md`](./DEPLOY.md).

## Repo layout

```
Project-Starter-Pack/
├── README.md                       # this file
├── ARCHITECTURE.md                 # Vectr / circuit / aggregator / handler / security
├── DEPLOY.md                       # single-op + multi-op walkthrough
├── CONTRIBUTING.md                 # public chat, how to add an example
├── LICENSE                         # GPL-3.0
├── rust-toolchain.toml             # 1.95 + wasm32-wasip1 + wasm32v1-none
├── warpdrive.toml.template         # node config skeleton, copied per example
├── scripts/
│   ├── bootstrap-keys.sh           # mint deployer + N operator mnemonics → .env
│   ├── op-env.sh                   # OP=N → port / data / mnemonic mapper
│   ├── upload-component.sh         # POST /dev/components (bypasses broken cli flag)
│   ├── middleware.sh               # wraps warpdrive-stellar-middleware container
│   └── new-project.sh              # generator: copy an example, rewrite names
├── vendor/
│   └── contracts/
│       ├── ed25519-security/       # verbatim from warpdrive-contracts
│       └── ed25519-verification/
└── examples/
    └── 01-counter/                 # MVP starter — cron → counter contract
        ├── README.md
        ├── Taskfile.yml
        ├── warpdrive.toml          # copied from ../../warpdrive.toml.template
        ├── rust-toolchain.toml
        ├── contracts/
        │   ├── counter/            # tick(ts) + count() + last_tick()
        │   ├── stellar-handler/    # decodes envelope, calls counter.tick
        │   ├── ed25519-security/   # copy of ../../vendor/contracts/
        │   └── ed25519-verification/
        ├── components/
        │   ├── tick-circuit/       # WASI 0.2: cron → TickPayload { ts }
        │   └── aggregator/         # WASI 0.2: emits Stellar SubmitAction
        ├── service/
        │   └── build-service.sh    # one workflow: cron → tick-circuit → aggregator → handler
        └── wit-definitions/        # warpdrive-vectr + aggregator worlds
```

Each example is self-contained: its own `Taskfile.yml`,
`warpdrive.toml`, contracts, components, and WIT. The pack's own
top-level `scripts/` and `vendor/contracts/` are the only things
shared across examples, and they are copied (not symlinked) into a
generated project by `new-project.sh` so the fork stands alone.

## What's in `examples/01-counter/`

A cron-driven counter. Every 30 seconds the WarpDrive node fires the
`*/30 * * * * *` cron trigger, the operator (or operators) sign the
current Unix timestamp into a `TickPayload`, the aggregator collects
the quorum into a single `XlmEnvelope`, and the stellar-handler
verifies the envelope on-chain and forwards the timestamp to a
`CounterContract.tick(ts)` call. The counter increments and stores
the last tick timestamp. End-to-end latency is one Stellar ledger
close (~5 s) after the cron fires.

```
cron tick (*/30 * * * * *)
   │
   ▼
┌──────────────────────────────────────────────────────────────┐
│ components/tick-circuit  - WASI 0.2                          │
│   • decodes the cron trigger payload                         │
│   • emits XDR-encoded TickPayload { ts: u64 }                │
└──────────────────────────────────────────────────────────────┘
   │
   ▼
┌──────────────────────────────────────────────────────────────┐
│ components/aggregator  - WASI 0.2                            │
│   • emits one Stellar SubmitAction at the handler            │
│     (chain + service_handler read from service.json)         │
└──────────────────────────────────────────────────────────────┘
   │  ed25519 quorum signs over libp2p, one operator wins the
   │  submission race, the rest back off
   ▼
┌──────────────────────────────────────────────────────────────┐
│ contracts/stellar-handler  (Soroban, testnet)                │
│   verify_xlm(envelope_bytes, sig_data):                      │
│     • XDR-decodes XlmEnvelope                                │
│     • delegates the ed25519 quorum check to                  │
│       contracts/ed25519-verification                          │
│     • XDR-decodes inner TickPayload { ts }                   │
│     • forwards ts to contracts/counter.tick                  │
└──────────────────────────────────────────────────────────────┘
   │
   ▼
┌──────────────────────────────────────────────────────────────┐
│ contracts/counter  (Soroban, testnet)                        │
│   tick(ts)         - asserts caller == registered handler,   │
│                       increments count, stores ts             │
│   count()          - public query                            │
│   last_tick()      - public query                            │
└──────────────────────────────────────────────────────────────┘
```

The counter contract exposes four entries:

| Entry | Purpose |
|---|---|
| `__constructor(handler: Address)` | Stores the handler address in instance storage. Set once at deploy. |
| `tick(ts: u64) -> u64` | Asserts `env.current_contract_address()`-via-`require_auth` matches the registered handler, increments the count, stores `ts`, returns the new count. |
| `count() -> u64` | Public query. |
| `last_tick() -> u64` | Public query — the latest `ts` the handler delivered. |

The trust chain is **operator quorum → `handler.verify_xlm` →
`counter.tick`**: the counter trusts only the handler, the handler
trusts only envelopes that pass the ed25519 quorum check, and the
quorum is exactly the set of operator pubkeys registered on
`ed25519-security`. Nobody else can advance the counter.

The `tick-circuit` is deliberately trivial. Its one job is to decode
the cron trigger payload (which contains nothing useful — cron just
fires the workflow on a schedule) and produce a `TickPayload { ts:
u64 }` containing the current Unix time. The wire format is
XDR-encoded `ScVal::Map([("ts", ScVal::U64(unix_secs))])`. The
handler decodes that same shape on the contract side, so the payload
definition is a single source of truth (Soroban `contracttype` on
the contract, hand-built `ScMap` matching that type in the WASI
component). Real workloads will swap the cron trigger for a Stellar
event, an EVM event, or a Round 2 composition event, and replace the
`{ts}` payload with whatever shape their handler expects — the rest
of the wiring stays the same.

## Where to go for more advanced patterns

Three real WarpDrive apps the pack is distilled from. Each
demonstrates one extra dimension the 01-counter example deliberately
leaves out:

| Project | What it adds on top of the counter |
|---|---|
| [hodlers-app](https://github.com/warp-driver/hodlers-app) | Stellar mainnet contract-event trigger; CAS accumulator over `wasi:keyvalue/atomics` for exactly-once delivery when one logical event fires as N concurrent sub-events. Smallest real app. |
| [oracle-demo](https://github.com/warp-driver/oracle-demo) | Cron + Stellar event + EVM (MetaMask, Sepolia) triggers; 2- and 3-round composition; per-operator vs quorum signing; React frontend talking to both wallets. |
| [phoenix-blend-pool](https://github.com/warp-driver/phoenix-blend-pool) | Blend pool rebalancer with the full multi-operator deploy plumbing (DEPLOY.md, OPERATORS.md, ARCHITECTURE.md, systemd units). The reference for taking a Starter Pack fork to production. |

When the answer to "how do I do X" is "go look at oracle-demo /
hodlers-app / phoenix-blend-pool", the pack will say so explicitly
rather than re-explaining a pattern that already exists upstream.

## Project status

**What works today.** End-to-end testnet pipeline for the 01-counter
example: cron tick → quorum-signed `XlmEnvelope` → `verify_xlm` →
`counter.tick`. Single-operator and N-of-N multi-operator
deployments. The vendored ed25519-security / ed25519-verification
contracts are unchanged from upstream and pass their existing test
suites. Scripts (`bootstrap-keys`, `op-env`, `upload-component`,
`middleware`, `new-project`) all run.

**Planned.** A second example demonstrating a Stellar-event trigger
(distilled from hodlers-app's circuit), a third demonstrating
multi-round composition (distilled from oracle-demo). An integration
test harness that spins up a node, runs a full cron cycle, and
asserts the counter advances. Frontend skeleton (React + Vite +
Freighter wiring) once a real example needs one.

**Out of scope for the first cut.** Web2 / atproto event triggers
(planned but not designed yet), EVM bridge example (use oracle-demo's
`eth-bridge-circuit` directly until a distilled version lands), and
production-grade observability (Prometheus exporters, log shipping,
alerting) — `DEPLOY.md` covers what to run in a tmux/systemd setup
and what to monitor manually.

## Community

Help, design discussion, and "is this the right pattern for X?"
questions belong in the public WarpDrive chat — link, channel
conventions, and office-hours cadence are in
[`CONTRIBUTING.md`](./CONTRIBUTING.md). Bug reports and feature
requests go to the WarpDrive GitHub org
([warp-driver](https://github.com/warp-driver)) — issues filed
against the pack itself live on this repo; issues about the
underlying engine, contracts, or middleware belong on
[`warpdrive`](https://github.com/warp-driver/warpdrive),
[`warpdrive-contracts`](https://github.com/warp-driver/warpdrive-contracts),
or
[`warpdrive-stellar-middleware`](https://github.com/warp-driver/warpdrive-stellar-middleware)
respectively.

## License

GPL-3.0 — see [`LICENSE`](./LICENSE).
