# Deploying a WarpDrive integration

This guide takes you from `git clone` to a finalized on-chain transaction
on Stellar testnet. About ten minutes for the single-operator dev
shape, twenty for the 2-operator local quorum demo. Both end at the
same on-chain state: a `counter` contract whose `count()` is being
advanced once every 30 s by quorum-signed `verify_xlm` envelopes that
originate inside a WASI circuit on the operator nodes.

If you have not read `ARCHITECTURE.md` yet, read it first. The
sections below assume you know what a circuit, an aggregator, a
handler, and an `ed25519-verification` quorum are.

The walkthrough drives `examples/01-counter/` — the smallest end-to-end
integration the Pack ships. Once you have it producing ticks on chain,
copy the directory (`./scripts/new-project.sh 01-counter ../my-thing`)
and swap the cron-circuit + counter contract for your own logic.

`examples/02-event-watcher/` and `examples/03-multi-round/` follow
the EXACT same task surface (`task deploy`, `task run-node`,
`task wire-service`, `task register-signer`/`register-signers`).
02 differs from 01 only in trigger (Stellar contract event vs cron)
and application contract (message-board vs counter). 03 adds a
second workflow and a second circuit but the deploy commands are
the same — and 03 specifically needs `OPERATORS=2` to demonstrate
the multi-round accumulator (1 operator + 1/1 quorum collapses the
demo). Once you can drive 01, 02 and 03 need no new commands.

---

## Prerequisites

Linux or macOS with a recent glibc / clang. Everything below must be on
`PATH` before you start.

| Tool                    | Why                                                                        | Install                                                                                       |
| ----------------------- | -------------------------------------------------------------------------- | --------------------------------------------------------------------------------------------- |
| `curl`, `jq`, `python3` | Shell glue + JSON munging in the Taskfile                                  | `apt install curl jq python3` / `brew install jq`                                             |
| `docker`                | Runs `warpdrive-stellar-middleware` (deploys the ed25519 stack)            | https://docs.docker.com/engine/install/                                                       |
| Rust 1.95               | Pinned by `rust-toolchain.toml`; required for component + contract builds | https://rustup.rs                                                                             |
| `cargo-component`       | Builds WASI 0.2 components                                                 | `cargo install cargo-component`                                                               |
| `wkg`                   | Resolves WIT deps from `wa.dev`                                            | `cargo install wkg`                                                                           |
| `stellar` CLI           | Soroban deploys, key management, RPC                                       | https://developers.stellar.org/docs/tools/developer-tools/cli/install-cli                     |
| `warpdrive`             | The operator runtime                                                       | `cargo install --path ../warpdrive/packages/warpdrive --locked`                               |
| `warpdrive-cli`         | Service registration, signer queries, component uploads                    | `cargo install --path ../warpdrive/packages/cli --locked`                                     |
| `task` (go-task)        | Runs the example's `Taskfile.yml`                                          | https://taskfile.dev/installation/                                                            |
| Node 20+ + `yarn`       | Only if your integration has a frontend (the bundled 01-counter does not) | https://nodejs.org/                                                                           |

Once `rustup` is installed, add the two wasm targets the toolchain
file references:

```bash
rustup target add wasm32-wasip1 wasm32v1-none
```

Point `wkg` at the `wa.dev` registry so it can resolve the
`warpdrive:` namespace WIT packages:

```bash
mkdir -p ~/.config/wasm-pkg
cat > ~/.config/wasm-pkg/config.toml <<'EOF'
default_registry = "wa.dev"

[namespace_registries]
warpdrive = "warg.wa.dev"
EOF
```

---

## One-time setup

```bash
git clone <pack-url> Project-Starter-Pack
cd Project-Starter-Pack/examples/01-counter

# Pull warpdrive's WIT definitions into wit-definitions/wit/deps/.
# One-time per clone; the WIT deps are not vendored because they evolve
# with warpdrive itself.
task fetch-wit
```

`task fetch-wit` is the only task that talks to `wa.dev`; if it fails,
your shell never gets a chance to build the components. See the
Troubleshooting section if `wkg` complains about the `warpdrive`
namespace.

The rest of the guide stays in `examples/01-counter/`. If you cloned
the pack into a different name, substitute accordingly.

---

## Single-operator quickstart

Everything on one host, quorum 1-of-1, no IPFS. The fastest path to
seeing a tick land on chain.

```bash
# 1. Mint a funded testnet deployer + one operator BIP39 mnemonic,
#    write them to .env. OPERATORS=1 keeps it to a single signer.
OPERATORS=1 ./scripts/bootstrap-keys.sh > .env
set -a; source .env; set +a

# 2. Phase 1 — build contracts + components, deploy the ed25519 stack
#    + project_root, deploy the counter + stellar-handler, register the
#    handler against project_root. Writes:
#      out/deploy.json   (ed25519_security, ed25519_verification, project_root)
#      out/counter.json  (the counter C-address)
#      out/handler.json  (the stellar-handler C-address)
task deploy

# 3. In a SECOND terminal, start the operator node. Leave it running.
#       cd Project-Starter-Pack/examples/01-counter
#       set -a; source .env; set +a
#       task run-node
#    Wait for "Stellar chain [stellar:testnet] is healthy" and
#    "HTTP server bound to port 8000".

# 4. Phase 2 — upload the cron-circuit + aggregator wasms to the local
#    node, assemble service/service.json, POST it to the dispatcher.
task wire-service

# 5. Register the operator's pubkey on the security contract at
#    weight 100 and set threshold 1/1.
task register-signer
```

That is the entire flow. Within ~30 s the node log should show the
cron firing, the circuit emitting a `TickPayload`, the aggregator
signing a `verify_xlm` envelope, and the stellar-handler invoking
`counter.tick(ts)` on testnet.

To confirm directly from the shell:

```bash
# Wait ~30 s after `task register-signer` returns, then:
COUNTER=$(jq -r .counter out/counter.json)
stellar contract invoke --id "$COUNTER" \
  --rpc-url https://soroban-testnet.stellar.org \
  --network-passphrase "Test SDF Network ; September 2015" \
  --source "$DEPLOYER_SECRET" \
  -- count
```

`count()` returns the number of ticks the handler has accepted.
Run it twice with a 30 s gap; it should grow by one or more.
`last_tick()` returns the Unix-seconds timestamp the circuit emitted
in the most recent payload.

---

## 2-operator local quorum demo

Three terminals on one host. Terminal 1 drives the deploy, terminals 2
and 3 each run a warpdrive node. Both nodes discover each other over
mDNS on the loopback; both fetch the same cron, both sign their own
`verify_xlm` envelopes. With threshold 1/1 at weight 100/200 a single
signature is short of quorum, so the aggregator must collect both
signatures before submitting — that is the quorum check you are
demoing.

The OPERATORS=2 path is what `bootstrap-keys.sh` produces by default,
so you can drop the explicit setting after the first run.

```bash
# ── Terminal 1 (driver) ──────────────────────────────────────────
cd Project-Starter-Pack/examples/01-counter

# Two operators, two mnemonics. The script writes
# WARPDRIVE_SIGNING_MNEMONIC (op 1) and WARPDRIVE_SIGNING_MNEMONIC_2
# (op 2) plus the same DEPLOYER_SECRET both nodes submit txs from.
OPERATORS=2 ./scripts/bootstrap-keys.sh > .env
set -a; source .env; set +a

# Phase 1 — same as the single-op path. The on-chain stack does not
# care how many operators run; quorum is enforced at signer-registration
# time, not at deploy time.
task deploy
```

```bash
# ── Terminal 2 (operator 1) ──────────────────────────────────────
cd Project-Starter-Pack/examples/01-counter
set -a; source .env; set +a
task run-node
# Listens on HTTP :8000, libp2p :9000, data dir out/node-data.
# Wait for "Stellar chain [stellar:testnet] is healthy" and
# "HTTP server bound to port 8000". Leave running.
```

```bash
# ── Terminal 3 (operator 2) ──────────────────────────────────────
cd Project-Starter-Pack/examples/01-counter
set -a; source .env; set +a
OP=2 task run-node
# Listens on HTTP :8010, libp2p :9010, data dir out/node-data-2.
# scripts/op-env.sh maps OP=N to port (8000 + 10*(N-1)) and the
# matching data dir + mnemonic env var. Wait for the same two
# health/HTTP lines, then "HTTP server bound to port 8010". Leave
# running.
```

```bash
# ── Terminal 1 (driver, continued) ───────────────────────────────
# Phase 2 — wire-service walks OP=1..OPERATORS, uploading the cron +
# aggregator wasms to each node's /dev/components, then assembling
# service/service.json and POSTing it to each node's /dev/services.
# Each node logs "Initializing dispatcher: services=1, workflows=1,
# components=2" once it accepts the spec.
task wire-service

# register-signers (plural) walks the same range: fetches each node's
# ed25519 pubkey via /services/signer, drops anything not in the
# resulting set from the on-chain signer table, adds the two we just
# fetched at weight 100 each, then applies threshold 1/1 (which
# against total weight 200 means both signatures are required).
task register-signers
```

Within ~30 s of `register-signers` returning, both node logs should
start showing matching `payload_size=…` and `submitting verify_xlm`
lines. Exactly one node will land the on-chain tx; the other will see
`EventAlreadySeen` and back off — that redundancy is by design (see
`ARCHITECTURE.md` § "Redundant submission"). Read `counter.count()` the
same way as in the single-op path:

```bash
COUNTER=$(jq -r .counter out/counter.json)
stellar contract invoke --id "$COUNTER" \
  --rpc-url https://soroban-testnet.stellar.org \
  --network-passphrase "Test SDF Network ; September 2015" \
  --source "$DEPLOYER_SECRET" \
  -- count
```

---

## Multi-host multi-operator deployment

The single-host demos above stop short of two things production needs:
each operator's box belongs to a different team, and the dispatcher
spec must reach every node without anyone copying files around.

The Pack ships the same `scripts/middleware.sh`, `scripts/bootstrap-keys.sh`,
and `scripts/op-env.sh` that `oracle-demo`, `hodlers-app`, and
`phoenix-blend-pool` use, so the pattern is identical to those projects'
multi-host walkthroughs:

- IPFS-pin `service/service.json` via Pinata, store the CID on chain
  in `project_root.service_uri()`, and let each operator's node fetch
  the spec on `register-manager` startup. No file copies between
  hosts.
- Each operator runs their own `WARPDRIVE_SIGNING_MNEMONIC` and
  exchanges only a derived ed25519 pubkey with the admin.
- The bootstrap operator publishes a libp2p multiaddr; everyone else
  pins it in `warpdrive.toml`'s `[warpdrive.p2p.remote] bootstrap_nodes`.

Rather than duplicate those steps here, the canonical walkthroughs
live alongside the projects that actually run them in production:

- `hodlers-app/DEPLOY.md` § "Multi-operator production deploy" — the
  cleanest small-example version. Step-by-step for a 3-operator
  2-of-3 quorum, with Pinata, p2p bootstrap, signer rotation, and an
  end-to-end smoke test.
- `phoenix-blend-pool/OPERATORS.md` — the operator inventory shape,
  per-operator provisioning checklist, hot-running checklist, and
  onboarding / off-boarding / signer-rotation procedures.
- `phoenix-blend-pool/DEPLOY.md` — the long-form deploy guide that
  goes with the operator inventory.

The Pack itself is intentionally single-host-focused so that the
quickstart stays under twenty minutes. Once you have your 01-counter
clone running multi-op locally, those three docs are the next stop.

---

## What each task does

The example's `Taskfile.yml` is the source of truth (each task has a
multi-line `desc:`). The table below summarises just the ones the
quickstart calls.

| Task                | What it does                                                                                              |
| ------------------- | --------------------------------------------------------------------------------------------------------- |
| `fetch-wit`         | One-time per clone. Calls `wkg wit fetch` to pull `warpdrive:vectr` and `warpdrive:aggregator` into `wit-definitions/wit/deps/`. |
| `deploy`            | Phase 1 composite. Runs `build-contracts` + `build-components`, then `deploy-middleware` (ed25519 stack + project_root via Docker), `deploy-counter`, `deploy-handler`, `register-handler`. Requires no node. |
| `run-node`          | Starts operator `OP`'s warpdrive node (default OP=1). Resolves port + data dir + mnemonic via `scripts/op-env.sh`. Foreground process; Ctrl-C stops it. |
| `wire-service`      | Phase 2 composite. Walks `OP=1..OPERATORS`, uploading the cron-circuit + aggregator wasms via `scripts/upload-component.sh`, then runs `build-service` (assemble `service/service.json` from on-chain addresses + digests) and `register-service` against each node. |
| `register-signer`   | Single-op convenience: fetches the local node's ed25519 pubkey via `/services/signer`, calls `ed25519-security.add_signer` at `SIGNER_WEIGHT` (default 100), then `set-threshold` at `THRESHOLD_NUM/DEN` (default 1/1). |
| `register-signers`  | Multi-op variant. Walks `OP=1..OPERATORS`, fetches each pubkey, resets the on-chain signer set to exactly those, then applies threshold. Idempotent — re-run anytime the operator set changes. |
| `set-threshold`     | Routes a threshold update through `project_root` to the security contract. Defaults to `THRESHOLD_NUM/THRESHOLD_DEN` env vars. |
| `register-manager`  | Multi-host only. Tells the local node to start watching `project_root` on chain and fetch the service spec from `project_root.service_uri()`. |

Anything else (per-component `build-*`, `upload-*`, `register-service`)
is reachable directly when you want to iterate on a single piece
without re-running the composite. `task --list` enumerates everything.

---

## BYOK env vars

`scripts/bootstrap-keys.sh` writes `.env` for you, but the variables
themselves are stable across the example projects and worth knowing
directly — particularly because production deploys swap the script's
freshly-minted demo keys for keys you control.

| Variable                          | Purpose                                                                                                                                             |
| --------------------------------- | --------------------------------------------------------------------------------------------------------------------------------------------------- |
| `DEPLOYER_SECRET`                 | Funded testnet `S…` seed. Used as `--source` for every Soroban deploy/invoke AND as the aggregator credential the node submits `verify_xlm` from.   |
| `DEPLOYER_ADDRESS`                | `G…` address matching `DEPLOYER_SECRET`. Read by the middleware Docker container and asserted against in a few places.                              |
| `OPERATORS`                       | Number of operators the multi-op tasks should walk over. Defaults to 2 in `bootstrap-keys.sh`; set to 1 for the single-op quickstart.                |
| `WARPDRIVE_SIGNING_MNEMONIC`      | BIP39 mnemonic operator 1 HD-derives its ed25519 signing key from. MUST be a BIP39 phrase — a raw hex secret won't satisfy non-zero HD indices.     |
| `WARPDRIVE_SIGNING_MNEMONIC_N`    | Same shape, for operator N>1. `bootstrap-keys.sh` writes one of these per operator (N=2, 3, …) when `OPERATORS>1`.                                  |
| `OP`                              | Per-shell selector (1..OPERATORS). Resolved by `scripts/op-env.sh` into `OP_PORT = 8000 + 10*(OP-1)`, `OP_P2P_PORT = 9000 + 10*(OP-1)`, the matching `out/node-data[-N]` dir, and `OP_MNEMONIC` from the variable above. Default 1. |

`bootstrap-keys.sh > .env` mints all of these in one shot: it deletes
any prior `stellar keys` aliases under `counter-deployer` /
`warpdrive-operator-N`, generates fresh keys, funds the deployer via
Friendbot, then prints the variable block on stdout for redirection
into `.env`. Sourcing `.env` is up to you (`set -a; source .env; set +a`).

This is fine for the demo because every run starts from a fresh
empty on-chain state anyway. For a real deployment you typically:

- generate the deployer key once, fund it through your own faucet /
  treasury, and keep it in a secrets manager rather than in `.env`;
- generate each operator's mnemonic on the operator's box (the
  mnemonic never leaves it; admin only ever sees the derived ed25519
  pubkey, exactly as in `phoenix-blend-pool/OPERATORS.md`);
- skip `bootstrap-keys.sh` entirely and write `.env` from those
  long-lived secrets instead.

The Taskfile reads every variable through `${VAR:?…}` guards, so a
missing one fails the task with a one-line error rather than silently
deploying with the wrong key.

---

## Troubleshooting

- **`task register-signer` fails with `500 missing key for service …`.**
  `wire-service` did not complete on that operator's node, so the
  dispatcher has no service to derive a signer for yet. Re-run
  `task wire-service` (or for a specific op, `OP=N task register-service`),
  watch the node log for `Adding service: counter` and
  `services=1, workflows=1, components=2`, then retry `register-signer`.

- **Port collides when starting operator 2.** Default port allocation
  is `OP_PORT = 8000 + 10*(OP-1)`, so op 2 wants `:8010` and op 3
  wants `:8020`. If another process already owns one of those:
  ```bash
  ss -ltn | grep -E ':80(00|10|20)'
  ```
  Kill the offender or bump every operator's index past it (`OP=11
  task run-node` puts that operator on `:8100`/`:9100`).

- **Cron triggers but a third-party API returns 429.** Not relevant
  to 01-counter (it has no outbound HTTP — the cron payload is just a
  timestamp), but this is the generic shape: edit `.env` to add the
  relevant API key, then re-bake it into the dispatcher with
  `task build-service && OP=N task register-service` per operator.
  The `oracle-demo`'s `COINGECKO_API_KEY` flow in `bootstrap-keys.sh`
  is the worked example. No node restart is needed.

- **Node restart wipes the dispatcher state.** Ctrl-C-ing a
  `task run-node` and starting it again leaves an empty `/dev/services`
  in that node. Re-run `task wire-service` (uploads + registration are
  idempotent), and if you raised the threshold or changed the signer
  set, `task register-signers` after. The on-chain state is unaffected
  by node restarts; only the dispatcher's in-memory + `out/node-data*/`
  state is.

- **`task fetch-wit` fails with `package warpdrive:vectr was not
  found`.** `wkg` does not know how to resolve the `warpdrive`
  namespace. Write `~/.config/wasm-pkg/config.toml` exactly as shown
  under Prerequisites, then retry. If you are behind a corporate
  proxy, set `HTTPS_PROXY` before re-running.

- **`stellar contract build` fails with `target wasm32v1-none not
  installed`.** `rust-toolchain.toml` declares both wasm targets, but
  rustup only installs them lazily. Force it:
  ```bash
  rustup target add wasm32v1-none --toolchain $(grep channel \
      rust-toolchain.toml | cut -d'"' -f2)
  ```

- **`task register-signers` says `op N pubkey is empty`.** The service
  hasn't been registered on op N's dispatcher yet, so
  `/services/signer` has nothing to return. Run `OP=N task
  register-service` first, then re-run `register-signers`.

- **Node log shows `evm:sepolia chain failed to connect` (or similar
  for a chain you are not using).** The shipped `warpdrive.toml.template`
  enables only `stellar:testnet`; other chains in the template are
  comment-blocked. If you uncommented one, either fix the endpoint or
  re-comment it. The node refuses to fully start until every enabled
  chain reports healthy.

- **`--warpdrive-endpoint` on `warpdrive-cli` is silently ignored.**
  Known upstream issue: the multi-op uploads went to whatever the cli
  considered the default node, not the per-operator port we wanted.
  The Pack works around it by shipping `scripts/upload-component.sh`,
  which POSTs directly to `$OP_URL/dev/components` so each operator's
  upload lands on the right node. If you replace the script with a
  direct `warpdrive-cli upload-component --warpdrive-endpoint …`
  call, expect op-2 uploads to land on op-1.

- **`verify_xlm` reverts with `EventAlreadySeen`.** Expected in
  multi-op. After quorum is reached every operator independently
  submits; the handler's on-chain dedup table accepts exactly one and
  rejects the rest. The losing operators back off and the system makes
  forward progress.

---

## Next steps

- **Integration tests.** Planned. The current verification path is the
  end-to-end check above (`task deploy` + `task wire-service` +
  `task register-signer{,s}` + a `stellar contract invoke -- count`
  read). A future `task e2e-test` will run that loop unattended and
  assert `count() > before` inside a wrapper.

- **Multi-host production.** Working today, via the Pinata-pinned
  `service.json` + `project_root.service_uri()` pattern. Use
  `hodlers-app/DEPLOY.md` and `phoenix-blend-pool/OPERATORS.md` as the
  canonical walkthroughs — they distil the operational rituals
  (operator inventory, threshold rotation, signer revocation) the
  Pack itself does not need.

- **Other trigger shapes.** 01-counter only demonstrates the cron
  trigger because it's the smallest. The same handler + aggregator
  shape carries over to:
  - Stellar contract events — see `hodlers-app`'s circuit, which
    watches Phoenix swap events on mainnet and emits a XLM-USDC
    `SwapPayload` to a testnet handler.
  - EVM contract events — see `oracle-demo`'s `eth-bridge-circuit`,
    which watches Sepolia for `TwapRequest` events and bridges them
    into a Stellar `request_twap` call via the same quorum-signed
    `verify_xlm` envelope.

  Both patterns drop straight into `examples/01-counter/components/`
  by replacing the cron-circuit; the aggregator + handler do not
  change.

- **Composition events.** `oracle-demo` shows a 3-round pipeline
  where Round 1 (cron) writes kv samples, Round 2 (event) emits
  per-operator attestations, Round 3 (event on Round 2's output)
  emits a quorum-signed median. The same `aggregator` and
  `ed25519-verification` contracts ship with the Pack, so you can
  build that pattern without any extra vendoring.
