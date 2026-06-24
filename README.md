# WarpDrive Project Starter Pack

A scaffold for Soroban developers building a WarpDrive integration:
three end-to-end examples covering the three triggers most
integrations use (cron, Stellar contract event, multi-round
composition), the security contracts that make on-chain quorum
verification work, and the deploy / wire-up scripts factored out
of three real WarpDrive apps in production. Clone it, run
`./scripts/new-project.sh 01-counter ../my-thing`, and you have a
working integration to iterate on instead of a blank repo and a
wiki tab.

## What this gives you

- **Three working examples** — `examples/01-counter/` (cron trigger →
  counter contract), `examples/02-event-watcher/` (Stellar contract
  event → message-board contract), and `examples/03-multi-round/`
  (cron-driven multi-round composition with a per-Vectr Round 1
  bundle accumulator and a quorum-collapsed Round 2 reduce). All
  three ship WASI 0.2 circuit(s) + aggregator + Soroban handler,
  signed by an ed25519 operator quorum, settled on Stellar testnet.
  ~600–800 LOC each end-to-end.
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
    ├── 01-counter/                 # cron → counter contract
    │   ├── README.md
    │   ├── Taskfile.yml
    │   ├── warpdrive.toml          # copied from ../../warpdrive.toml.template
    │   ├── rust-toolchain.toml
    │   ├── contracts/
    │   │   ├── counter/            # tick(ts) + count() + last_tick()
    │   │   ├── stellar-handler/    # decodes envelope, calls counter.tick
    │   │   ├── ed25519-security/   # copy of ../../vendor/contracts/
    │   │   └── ed25519-verification/
    │   ├── components/
    │   │   ├── tick-circuit/       # WASI 0.2: cron → TickPayload { ts }
    │   │   └── aggregator/         # WASI 0.2: emits Stellar SubmitAction
    │   ├── service/
    │   │   └── build-service.sh    # one workflow: cron → tick-circuit → aggregator → handler
    │   └── wit-definitions/        # warpdrive-vectr + aggregator worlds
    ├── 02-event-watcher/           # Stellar contract event → message-board (same layout, set-stellar trigger)
    └── 03-multi-round/             # cron → Round 1 attestation → Round1Ready event → Round 2 reduce → Final
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

## What's in `examples/02-event-watcher/`

A self-contained Stellar contract event watcher. The same
`MessageBoard` contract owns BOTH ends of the loop: anyone calls
`publish(msg)` from any wallet, the contract emits a Soroban event
with topic `(Symbol("msg"), U64(msg_id))` and value `String(msg)`,
the operator quorum observes the event, decodes it in a WASI circuit,
signs the result with ed25519, and the stellar-handler verifies the
quorum and calls `MessageBoard.record_signed(msg_id, msg)` on the
same contract. The recorded message is then queryable by id from
anyone. Round-trip latency: ~5–15 s after `publish` (one ledger
close per direction).

```
user wallet (Freighter / CLI)
   │  publish(msg)
   ▼
┌──────────────────────────────────────────────────────────────┐
│ contracts/message-board  (Soroban, testnet)                  │
│   publish(msg) → mints msg_id, emits Soroban event with      │
│     topic[0]=Symbol("msg"), topic[1]=U64(msg_id),            │
│     value=String(msg)                                        │
└──────────────────────────────────────────────────────────────┘
   │  topic-filter match (set-stellar --topic-symbol msg --topic wildcard)
   ▼
┌──────────────────────────────────────────────────────────────┐
│ components/event-watcher-circuit  - WASI 0.2                 │
│   • decodes topic_segments + event.value                     │
│   • emits XDR-encoded RecordPayload { msg, msg_id }          │
└──────────────────────────────────────────────────────────────┘
   │  ed25519 quorum signs, host quorum-collapses by salt=msg_id
   ▼
┌──────────────────────────────────────────────────────────────┐
│ contracts/stellar-handler.verify_xlm                         │
│   → contracts/message-board.record_signed(msg_id, msg)       │
│   (handler.require_auth() gates the cross-contract call)     │
└──────────────────────────────────────────────────────────────┘
   │
   ▼
anyone: recorded(msg_id) → Some(msg)
```

Same five-layer pattern as `01-counter`. The only thing that
changed is the trigger: `set-cron` becomes `set-stellar
--topic-symbol msg --topic wildcard`, and the circuit's job switches
from "read scheduled time" to "decode Soroban event topics + value".
The handler dispatcher, aggregator, and operator/quorum plumbing
are byte-identical to 01-counter.

One thing this example demonstrates that's worth calling out:
**predict-then-deploy bootstrapping**. `MessageBoard` takes the
handler address at construction, the handler takes the
`MessageBoard` address at construction — a cyclic dependency. The
Taskfile resolves it by predicting the handler's address from
`stellar contract id wasm --salt $SALT --source-account
$DEPLOYER_ADDRESS` BEFORE either deploys, passing the predicted
address to `MessageBoard`'s constructor, and then deploying the
handler with the same salt so it lands at exactly the predicted
address. A post-deploy assertion catches salt/source drift between
the two steps. The pattern is reusable for any pair of contracts
that need to know each other's addresses at construction.

## What's in `examples/03-multi-round/`

A cron-driven two-round composition. Every 30 s each operator's
Round 1 circuit emits a per-Vectr value (`signer_value` = something
that legitimately differs across operators — `wall_clock::now()
nanoseconds % 1000` in the demo). The composer contract accumulates
each operator's attestation into a per-`round_id` bundle until the
bundle crosses `ceil(N · quorum_num / quorum_denom)`, at which point
it emits a `Round1Ready` Soroban event carrying the whole bundle.
Each operator's Round 2 circuit then observes that event, folds the
bundle to a `min` (the simplest pure-deterministic reduce — every
operator produces byte-identical bytes), and the host quorum-
collapses their signatures into one envelope. `verify_xlm` dispatches
to the Final arm, saves the aggregate, and emits `Finalized`.

```
cron tick (*/30 * * * * *)
   │                  │
   ▼ op 1             ▼ op 2
┌─────────────┐    ┌─────────────┐
│ round1-circ │    │ round1-circ │
│ value: 421  │    │ value: 837  │   ← `signer_value` differs per op
└──────┬──────┘    └──────┬──────┘
       │                  │            (per-Vectr salt: payload bytes
       │ envelope_A       │ envelope_B   themselves, distinct per op)
       ▼                  ▼
┌──────────────────────────────────────────────────────────────┐
│ composer.verify_xlm(SubmissionPayload::Round1(...))          │
│   try_check_one → push to Attestations(round_id)             │
│   bundle.len() ≥ ceil(N·num/denom) → emit Round1Ready        │
└────────────────────────┬─────────────────────────────────────┘
                         │ Round1Ready event {round_id, bundle}
                         ▼
┌─────────────┐    ┌─────────────┐
│ round2-circ │    │ round2-circ │
│ aggregate:  │    │ aggregate:  │   ← every op reads SAME on-chain
│   min=421   │    │   min=421   │     bundle → SAME bytes out
└──────┬──────┘    └──────┬──────┘
       │                  │            (deterministic salt: round_id
       │   identical      │ identical    ++ "-r2", collides per op)
       ▼                  ▼
        host QuorumQueue collapses N envelopes into ONE with N sigs
                         │
                         ▼
┌──────────────────────────────────────────────────────────────┐
│ composer.verify_xlm(SubmissionPayload::Final(...))           │
│   try_verify → save Final(round_id, aggregate)               │
│   emit Finalized                                             │
└──────────────────────────────────────────────────────────────┘
   │
   ▼
anyone: final_result(round_id) → Some(min value)
```

The **salt asymmetry** is the load-bearing trick of multi-round
WarpDrive composition:

- Round 1 must use a unique-per-operator salt. If two operators
  produce different `signer_value`s (which they SHOULD — the whole
  point is to attest to something each Vectr observed independently),
  but the host's QuorumQueue saw the same `event_id`, it would
  quorum-collapse the two different envelopes into one and the
  contract would silently reject `len(sigs) != 1` on the
  `try_check_one` path. So Round 1 reuses the payload bytes
  themselves as the salt — guaranteed unique because the payloads
  legitimately differ.
- Round 2 must use a deterministic salt. Every operator runs the
  same pure reduce over the same on-chain bundle, so they produce
  byte-identical envelope bytes. The host quorum-collapses their
  signatures into ONE envelope with N signatures, which the
  contract's `try_verify` arm accepts. Round 2's salt is
  `round_id.to_le_bytes() ++ b"-r2"` — pure function of the trigger.

The composer contract is BOTH the handler (verify_xlm with the two
dispatch arms) AND the application state (Round1Bundle + Final live
in its storage). No separate `stellar-handler` + `application`
contract pair like 01/02 — one contract, no predict-then-deploy
dance. The same pattern recurses to more rounds if you need them
(oracle-demo's three-round flow is exactly this scaled up with a
third `request_twap` round on top).

This example REQUIRES two operators. With one operator and 1/1
quorum, the Round 1 threshold (`ceil(1 · 1 / 1) = 1`) fires on the
first attestation and the multi-round shape collapses to a single
envelope per tick. Set `OPERATORS=2` in `.env` and run two
`task run-node` terminals (one with `OP=2 task run-node`) so you
actually see the bundle accumulate before `Round1Ready` fires.

## Where to go for more advanced patterns

Three real WarpDrive apps the pack is distilled from. They pick up
where the in-pack examples stop — production multi-host quorum,
EVM bridges, mainnet event sources, three-round composition with
real financial math, and the operational tooling that goes with
running operators for real:

| Project | What it adds on top of the in-pack examples |
|---|---|
| [hodlers-app](https://github.com/warp-driver/hodlers-app) | Stellar mainnet contract-event trigger; CAS accumulator over `wasi:keyvalue/atomics` for exactly-once delivery when one logical event fires as N concurrent sub-events. Smallest real app. |
| [oracle-demo](https://github.com/warp-driver/oracle-demo) | Cron + Stellar event + EVM (MetaMask, Sepolia) triggers; 2- and 3-round composition; per-operator vs quorum signing; React frontend talking to both wallets. |
| [phoenix-blend-pool](https://github.com/warp-driver/phoenix-blend-pool) | Blend pool rebalancer with the full multi-operator deploy plumbing (DEPLOY.md, OPERATORS.md, ARCHITECTURE.md, systemd units). The reference for taking a Starter Pack fork to production. |

When the answer to "how do I do X" is "go look at oracle-demo /
hodlers-app / phoenix-blend-pool", the pack will say so explicitly
rather than re-explaining a pattern that already exists upstream.

## Project status

**What works today.** End-to-end testnet pipeline for all three
examples. `01-counter` covers cron → counter, `02-event-watcher`
covers Stellar contract event → message-board (same contract owns
both ends, predict-then-deploy bootstrap), `03-multi-round` covers
two-round composition with a per-Vectr Round 1 bundle accumulator
and a quorum-collapsed Round 2 reduce. Single-operator and N-of-N
multi-operator deployments. The vendored ed25519-security /
ed25519-verification contracts are unchanged from upstream and pass
their existing test suites. Scripts (`bootstrap-keys`, `op-env`,
`upload-component`, `middleware`, `new-project`) all run.

**Planned.** An integration test harness that spins up a node, runs
a full cron cycle for 01, and asserts the counter advances. A
frontend skeleton (React + Vite + Freighter wiring) once a real
example needs one. Additional examples covering EVM bridges,
atproto firehose triggers, and production observability are still
best read directly out of the source projects until they're
distilled.

## Community

Help, design discussion, and "is this the right pattern for X?"
questions belong in the [WarpDrive thread on the Stellar Developer Discord](https://discord.com/channels/897514728459468821/1519279126610055218)
 — channel conventions and office-hours cadence are documented in
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
