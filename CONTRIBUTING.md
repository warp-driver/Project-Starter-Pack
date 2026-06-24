# Contributing

The pack is a thin layer of glue and one tiny example. The bar for a
change is "another Soroban dev would have a clearer path to a working
WarpDrive integration after this lands."

## Where to get help

Public chat: [WarpDrive thread on the Stellar Developer Discord](https://discord.com/channels/897514728459468821/1519279126610055218).
It's where design questions and "is this the right pattern for X?"
conversations happen. For anything that needs a fix — a bug, a broken
task, a docs gap — open a GitHub discussion or issue (below); chat
is for the open-ended stuff that doesn't yet have a clear ask.

## Issues / discussions

The pack is one repo in a small org. Pick the closest:

| Symptom | Where to file |
|---|---|
| The pack's docs are wrong / unclear / missing a step. | This repo. |
| `examples/*` or `scripts/*` misbehave. | This repo. |
| The `warpdrive` daemon crashes / a CLI subcommand is broken. | [warp-driver/warpdrive](https://github.com/warp-driver/warpdrive). |
| `ed25519-security` / `ed25519-verification` / `project-root` behave wrong. | [warp-driver/warpdrive-contracts](https://github.com/warp-driver/warpdrive-contracts) (the vendored copies here are byte-identical, so the fix has to land upstream first). |
| `warpdrive-stellar-middleware` container misbehaves. | [warp-driver/warpdrive-stellar-middleware](https://github.com/warp-driver/warpdrive-stellar-middleware). |

For open-ended design questions use GitHub Discussions on this repo,
not an issue — issues are for things with a fix.

## How to add an example to the pack

Examples live under `examples/NN-name/`, numbered in the order added
(`01-counter/` is the MVP, `02-event-watcher/` adds the Stellar
contract event trigger; the next slot is `03-…`). `new-project.sh`
selects by directory name and the top-level README's "Where to go for
more advanced patterns" table is sorted by complexity. Mirror the
`01-counter/` layout exactly:

```
examples/NN-your-name/
├── README.md                       # what this example demonstrates, in 1 page
├── Taskfile.yml                    # deploy / wire-service / register-signer / run-node
├── warpdrive.toml                  # copy of ../../warpdrive.toml.template + per-example overrides
├── rust-toolchain.toml             # copy of ../../rust-toolchain.toml
├── contracts/
│   ├── <your-handler-domain>/      # the actual app contract
│   ├── stellar-handler/            # verify_xlm dispatcher
│   ├── ed25519-security/           # copy of ../../vendor/contracts/ed25519-security
│   └── ed25519-verification/       # copy of ../../vendor/contracts/ed25519-verification
├── components/
│   ├── <your-circuit>/             # WASI 0.2, emits XDR payload
│   └── aggregator/                 # WASI 0.2, emits Stellar SubmitAction
├── service/
│   └── build-service.sh            # declarative warpdrive-cli → service.json
└── wit-definitions/                # warpdrive-vectr + aggregator worlds
```

To test:

```bash
cd examples/NN-your-name
task fetch-wit
../../scripts/bootstrap-keys.sh > .env
set -a; source .env; set +a
task deploy
# in another terminal: task run-node
task wire-service
task register-signer
# trigger the example's entry point (cron, event, RPC call …)
# read the handler's query entry to confirm the value lands
```

A new example PR needs the layout above, an
`examples/NN-your-name/README.md` shaped like `01-counter/README.md`
(trigger / circuit / handler / contract on one page), and one row
added to the top-level README's "Where to go for more advanced
patterns" table.

## Style guide

- **No emojis** anywhere — prose, commits, code comments, CLI output.
- **Terse comments that explain WHY, not WHAT.** "// CAS retry because
  WarpDrive fans out the 8 swap events in parallel and last-write-wins
  would clobber earlier accumulators" is useful. "// increment counter"
  is not.
- **BYOK env vars, never hard-coded secrets.** `DEPLOYER_SECRET`,
  `WARPDRIVE_SIGNING_MNEMONIC[_N]`, `COINGECKO_API_KEY`, `PINATA_JWT`,
  `SEPOLIA_DEPLOYER_KEY` — read from `.env` via `set -a; source .env;
  set +a`. `.env` is gitignored; `bootstrap-keys.sh` mints it.
- **One Cargo.toml per contract crate. No workspace parent.** Soroban's
  `[profile.release]` (`overflow-checks`, `lto`, `codegen-units`) only
  takes effect at the workspace root, so each contract crate IS its
  own root — a workspace-level `[profile.release]` silently fails to
  propagate. Components MAY share a workspace; contracts MUST NOT.
- **Markdown headings start at `##` after the title.** Single `#` only
  for the document title. Commands always in fenced bash blocks, never
  inline.
- **Commit messages: short, all lowercase, no `feat:` / `fix:` /
  conventional-commit prefixes.** Match the existing log.

## Coding patterns to preserve

Lifting these from the source apps (`hodlers-app`, `oracle-demo`,
`phoenix-blend-pool`) is what makes the pack a starter pack and not
just a list of links. PRs that quietly diverge will get pushback.

- **Single source of truth for inner payload shapes.** Define the
  payload as a Soroban `contracttype` (e.g. `TickPayload { ts: u64 }`).
  In the WASI circuit, hand-build the matching XDR `ScMap` via
  `stellar-xdr` (Soroban SDK doesn't compile to `wasm32-wasip1`). One
  shared shape, not duplicated logic. Map field names MUST match the
  struct fields in **alphabetical** order — the contract's XDR
  decoder is strict about it.
- **`XlmEnvelope` is the outer wrapper, always.** The handler decodes
  `XlmEnvelope { event_id, ordering, payload: Bytes, ... }`, calls
  `ed25519-verification.try_verify(envelope, sig_data, ...)`, then
  decodes the inner payload from `envelope.payload`. Never invent a
  bespoke outer shape — the verification contract only accepts the
  canonical envelope.
- **CAS via `wasi:keyvalue/atomics` for any event accumulator.** When
  one logical event arrives as N parallel sub-events (mainnet swaps,
  EVM logs with multi-topic decomposition, anything bundled), the
  circuit MUST use compare-and-swap, not blind writes — otherwise
  later writers clobber earlier ones and the accumulator never
  finalises. Reference: `hodlers-app/components/circuit/src/state.rs`.
  A `finalized: bool` tombstone after the final write gives
  exactly-once semantics.
- **Deterministic salt per workflow.** The aggregator's
  `SubmitAction` salt MUST be a deterministic function of the
  workflow-unique identifier (typically the trigger event id, or
  `tx_hash:op_index` for Stellar events). Random salt breaks the
  on-chain `event_id` dedup the handler relies on.
- **Open handler call, trust upstream.** `counter.tick`,
  `hodlers.add_points`, `oracle.submit_round2` etc. are open calls —
  no admin gate. Trust is upstream: only the handler can produce
  envelopes the verification contract accepts, and only the operator
  quorum can produce envelopes the handler accepts. An admin gate on
  the inner contract is redundant and an operational footgun.
