## 03-multi-round

The smallest WarpDrive integration that demonstrates **multi-round
composition**: a per-Vectr Round 1 attestation phase that the operators
all sign distinctly, an on-chain accumulator that emits a composition
event when the bundle is full, and a Round 2 reduce phase where every
operator computes the same answer from the immutable on-chain bundle
and the host quorum-collapses their identical envelopes into a single
quorum-signed final. One composer contract owns the whole loop — it's
both the handler (`verify_xlm` dispatches on a tagged payload variant)
and the application state (Round1Bundle + Final live in its
storage) — so the demo runs self-contained on testnet with no second
project to deploy.

This is the natural next step after 01-counter (cron trigger) and
02-event-watcher (Stellar-event trigger): the same five-layer pattern
plus the two load-bearing primitives that every multi-round WarpDrive
service relies on — a tagged `SubmissionPayload` dispatcher and a
per-Vectr-vs-deterministic salt asymmetry between the rounds.

## Architecture

```
                        cron @ */30 s
                              │
              ┌───────────────┴───────────────┐
              ▼                               ▼
┌──────────────────────────┐    ┌──────────────────────────┐
│ op 1: round1-circuit     │    │ op 2: round1-circuit     │
│   round_id = ts/30s      │    │   round_id = ts/30s      │  ← same
│   signer_value = wall%1k │    │   signer_value = wall%1k │  ← differs!
│   payload = ScVal::Map { │    │   payload = ScVal::Map { │
│     round_id, signer_v } │    │     round_id, signer_v } │
│   event_id_salt          │    │   event_id_salt          │
│     = payload bytes      │    │     = payload bytes      │  ← per-Vectr
└──────────────────────────┘    └──────────────────────────┘
              │ one ed25519 sig per op (salts differ → no collapse)
              ▼                               ▼
┌──────────────────────────────────────────────────────────┐
│ composer.verify_xlm(SubmissionPayload::Round1)           │
│   try_check_one (one sig, "is this signer registered?")  │
│   apply_round1: dedup by signer, push attestation        │
│   if bundle.len() ≥ ceil(N·num/den): emit r1ready event  │
│     topic[0]=Symbol("r1ready"), topic[1]=U64(round_id)   │
│     value   = Round1Bundle { attestations: [...] }       │
└──────────────────────────────────────────────────────────┘
              │  Stellar contract event, observed by all ops
              ▼
      ┌───────┴────────┐
      ▼                ▼
┌──────────────────────────┐    ┌──────────────────────────┐
│ op 1: round2-circuit     │    │ op 2: round2-circuit     │
│   decode bundle (ScVal)  │    │   decode bundle (ScVal)  │
│   aggregate = bundle.min │    │   aggregate = bundle.min │  ← same!
│   payload = ScVal::Map { │    │   payload = ScVal::Map { │
│     aggregate, round_id }│    │     aggregate, round_id }│
│   event_id_salt          │    │   event_id_salt          │
│     = round_id ++ "-r2"  │    │     = round_id ++ "-r2"  │  ← deterministic
└──────────────────────────┘    └──────────────────────────┘
              │  N envelopes, identical bytes + salt
              │  → host QuorumQueue collapses to 1 envelope w/ N sigs
              ▼
┌──────────────────────────────────────────────────────────┐
│ composer.verify_xlm(SubmissionPayload::Final)            │
│   try_verify (sum-of-weights ≥ required at ref_block)    │
│   apply_final: store Final(round_id, aggregate)          │
│   emit Symbol("final"), U64(round_id) → U64(aggregate)   │
└──────────────────────────────────────────────────────────┘
              │
              ▼
   stellar contract invoke -- final_result --round_id N
                                            → Some(min)
```

The trust chain is **cron tick → N parallel per-Vectr attestations →
composer accumulator → quorum-signed reduce → composer final**.
`apply_round1` and `apply_final` are both private — the only way to
reach them is through `verify_xlm`, which gates on
`ed25519-verification.try_check_one` (Round 1) or `try_verify`
(Final). Anyone can read `final_result(round_id)` and
`round1_bundle(round_id)`, but no one can forge an entry on either.

## Contracts

| Crate | Role |
|---|---|
| `contracts/composer` | The whole application. `__constructor(verification, quorum_num, quorum_den)` wires the trust chain at deploy time. `verify_xlm(envelope, sig_data)` decodes `SubmissionPayload::from_xdr` and dispatches: `Round1` → `try_check_one` + `apply_round1` (dedup, accumulate, latch + emit on threshold); `Final` → `try_verify` + `apply_final` (store + emit). Reads: `final_result(round_id) -> Option<u64>`, `round1_bundle(round_id) -> Option<Round1Bundle>`. ~250 LOC. |
| `contracts/ed25519-security` | Vendored verbatim from [warpdrive-contracts](https://github.com/warp-driver/warpdrive-contracts). Holds the operator signer set + threshold. Admin is `project_root` once `deploy-middleware` finishes. |
| `contracts/ed25519-verification` | Vendored verbatim. Two entry points the composer uses: `try_check_one` (Round 1 — exactly one signer, "is this key registered?") and `try_verify` (Final — sum-of-weights ≥ required at the reference block). |

Composer is both the handler **and** the application, so there's no
predict-then-deploy dance (unlike 01/02 where `message_board`'s
constructor took the handler's address before the handler existed).
One contract, one deploy, one `register_handler` call against
`project_root`.

## Components

| Crate | Role |
|---|---|
| `components/round1-circuit` | WASI 0.2 circuit, cron-triggered. Each tick: `round_id = trigger_time.nanos / 30_000_000_000` (stable across operators on the same tick), `signer_value = wall_clock::now().nanos % 1000` (differs per operator — that's the point). Emits `SubmissionPayload::Round1 { round_id, signer_value }` as an alphabetically-sorted XDR ScMap. **`event_id_salt = payload_bytes.clone()`** — the payload itself differs per Vectr, so reusing it as the salt costs nothing and guarantees the host keeps every operator's envelope distinct. |
| `components/round2-circuit` | WASI 0.2 circuit, Stellar-event-triggered on `(Symbol("r1ready"), Wildcard)` against the composer's contract id. Decodes `event.value` as `Round1Bundle { attestations: Vec<{signer, value}> }`, reduces with `bundle.attestations.iter().map(|a| a.value).min()`, emits `SubmissionPayload::Final { round_id, aggregate }`. **`event_id_salt = round_id.to_le_bytes() ++ b"-r2"`** — deterministic across operators (every Vectr sees the same on-chain bundle, computes the same min, builds the same payload bytes), so the host's QuorumQueue batches all N envelopes into one. No wall-clock or other time-derived reads in Round 2's path. |
| `components/aggregator` | WASI 0.2 aggregator. Standard Stellar `SubmitAction` emitter — reads `chain` and `service_handler` from the service spec's `component config` and constructs the `verify_xlm` invocation. Both workflows share the same aggregator pointing at the same composer address; the SubmissionPayload tag does the rest. |

## Salt asymmetry: the load-bearing trick

The two rounds use the same `verify_xlm` entry point on the same
contract, but their `event_id_salt` rules are deliberately opposite.
Get this wrong and the pipeline silently produces the wrong answers
(or no answers at all). It's a one-paragraph rule whose every word
has teeth — worth reading once before forking.

**Round 1 is fan-out.** Each operator legitimately observes something
slightly different about the world — its own wall clock, its own
sampling of an external data source, its own local view of a
not-yet-finalised chain state. `signer_value` here is contrived
(`wall_clock.nanos % 1000` always differs), but in production it's
"the CoinGecko price tick I saw" or "the swap volume in the last 30
seconds on my mempool view". The whole point of an attestation phase
is to **collect** these per-operator observations, not collapse them
into one. The host's submission manager looks at
`(envelope_bytes, event_id_salt)` to decide whether two operators'
envelopes are talking about "the same event" — if they match, it
quorum-collapses them into one submission with N signatures. If we
let the salts match in Round 1, the host would treat the N distinct
observations as duplicates and drop all but one. So Round 1 uses a
**per-Vectr salt** — here, the payload bytes themselves, which
differ because `signer_value` differs. Every operator's envelope
lands on chain as its own submission, the composer's
`apply_round1` dedups by signer pubkey, and the bundle grows by
one attestation per operator until it crosses
`ceil(N · quorum_num / quorum_denom)` — at which point it latches and
emits `r1ready`.

**Round 2 is fan-in.** Every operator subscribes to the same on-chain
event, reads the same immutable `Round1Bundle`, and computes the same
reduction (`min` here; `geomean` in oracle-demo). The payload bytes
are identical across operators by construction — anything time-derived
in Round 2's path would break that invariant and is forbidden. So
Round 2 uses a **deterministic salt** — here,
`round_id.to_le_bytes() ++ b"-r2"`. With identical
`(envelope_bytes, event_id_salt)` across N operators, the host's
QuorumQueue batches their signatures into one envelope, the composer
sees a single `verify_xlm` call with N signatures, and `try_verify`
checks sum-of-weights against the threshold. One on-chain
transaction, N attesting signers, exactly the property a quorum
signature exists to provide.

The on-chain manifestation of this asymmetry is the two-arm dispatch
inside `verify_xlm`: `Round1` calls `try_check_one` (the per-signer
"is this key registered?" check); `Final` calls `try_verify` (the
quorum threshold check). Mismatch the salts and you'll hit one of
two failure modes: Round 1 with deterministic salt → host collapses
N attestations to 1, `apply_round1`'s bundle never fills, `r1ready`
never fires. Round 2 with per-Vectr salt → host keeps N envelopes
distinct, each fails `try_verify` for not meeting the threshold on
its own.

The same asymmetry shows up in every production multi-round WarpDrive
service. oracle-demo splits a three-round pipeline (cron prefetch →
per-operator TWAP → quorum-collapsed median) along exactly these
seams. hodlers-app deliberately keeps to a single round and uses a
deterministic salt throughout, because there's no per-operator
observation worth collecting separately. Phoenix's blend-pool
rebalancer is also single-round-deterministic. Two rounds is the
inflection point where the salt rule starts paying its rent.

## Quickstart

A funded testnet key and **two warpdrive nodes** on your laptop.
About six minutes from clone to first finalised round. The demo
**requires `OPERATORS=2`** to be meaningful — with one operator and
the default 1/1 quorum the bundle latches on the first attestation
and the "wait for quorum" step the example exists to teach
vanishes. Single-op walkthrough is still possible (just to verify
the deploy is wired); replace step 3's quorum with `1/1` and skip
the second-terminal node.

```bash
# 0. Working directory.
cd Project-Starter-Pack/examples/03-multi-round

# 1. WIT deps (one-time per clone). Fetches warpdrive-vectr and the
#    aggregator world into wit-definitions/wit/deps/.
task fetch-wit

# 2. Mint a funded testnet deployer + two operator mnemonics into .env.
OPERATORS=2 ../../scripts/bootstrap-keys.sh > .env
set -a; source .env; set +a

# 3. Build contracts + components, deploy the ed25519 stack and the
#    composer (constructor: verification address + 1/1 bundle quorum
#    so both ops must attest), register the composer on project_root.
#    Writes out/deploy.json + out/composer.json.
OPERATORS=2 task deploy

# 4. Start operator 1 (terminal A) — leave running:
#       cd Project-Starter-Pack/examples/03-multi-round
#       set -a; source .env; set +a
#       OPERATORS=2 task run-node
#    Wait for "Stellar chain [stellar:testnet] is healthy" plus
#    "HTTP server bound to port 8000".
#
#    Then operator 2 (terminal B) on :8010 / :9010:
#       OP=2 OPERATORS=2 task run-node

# 5. Upload both circuits + aggregator to both nodes, build service.json
#    with the two workflows (round1 + round2), activate on both nodes.
OPERATORS=2 task wire-service

# 6. Sync ed25519-security to exactly both operators at 2/2 threshold
#    so verify_xlm's Final arm needs both signatures.
OPERATORS=2 THRESHOLD_NUM=2 THRESHOLD_DEN=2 task register-signers
```

The pipeline is cron-driven from here — nothing else to do. Wait one
tick (~30 s), watch one round complete, then read the answer:

```bash
COMPOSER=$(jq -r .composer out/composer.json)
RPC=https://soroban-testnet.stellar.org
PASS="Test SDF Network ; September 2015"

# Both nodes' logs should show a Round 1 envelope land within ~5 s of
# the tick, then the `r1ready` event, then a single quorum-collapsed
# Round 2 envelope a few seconds later.
sleep 45

# Round-id is `(unix_seconds_at_tick / 30)` for the default cron — pick a
# tick a minute or so before now and let the node logs confirm. Each
# round1-circuit log line prints `round_id=NNNN` on the tick it observed.
ROUND_ID=$(( $(date +%s) / 30 - 2 ))

stellar contract invoke --id "$COMPOSER" \
  --rpc-url "$RPC" --network-passphrase "$PASS" \
  --source "$DEPLOYER_SECRET" \
  -- final_result --round_id "$ROUND_ID"
# → Some(NNN)   the min of the two operators' signer_value attestations
```

`round1_bundle(round_id)` is the read-only window into the
intermediate state — the two attestations the bundle held when
`r1ready` fired. Useful for debugging the round-id derivation if
the two operators land on different `round_id`s for the same tick
(they shouldn't — `cron.trigger_time` is the scheduled time, not
when each node observed the tick).

## Where to go next

Three real WarpDrive apps the example is distilled from. 03 is
deliberately the minimum demonstration of multi-round; production
services layer on persistence, more rounds, or multi-host quorum.

| Project | What it adds on top of multi-round |
|---|---|
| [oracle-demo](https://github.com/warp-driver/oracle-demo) | **Three** rounds, not two: a cron-driven CoinGecko prefetch with `wasi:keyvalue/atomics` CAS-accumulated price samples (Round 0 of sorts), a per-Vectr TWAP attestation (Round 1 with the same salt asymmetry as here), and a quorum-collapsed median reduce (Round 2). Plus a Sepolia EVM-bridge workflow that fan-ins into the same TWAP pipeline. The reference for taking the salt-asymmetry pattern to more rounds and to EVM triggers. |
| [hodlers-app](https://github.com/warp-driver/hodlers-app) | The smallest **single-round** real app — mainnet Phoenix XLM-USDC swap event into one quorum-signed record. Use this as the template when you don't actually need multi-round; lots of production WarpDrive services live there. The contrast is instructive: hodlers-app uses a deterministic salt for the whole loop because there's no per-operator observation worth collecting separately. |
| [phoenix-blend-pool](https://github.com/warp-driver/phoenix-blend-pool) | Blend pool rebalancer with full production **multi-host** quorum: dedicated `DEPLOY.md`, `OPERATORS.md`, `ARCHITECTURE.md`, systemd unit templates. Single-round-deterministic, like hodlers-app, but the operators run on three separate hosts behind real DNS. The reference for taking a Starter Pack fork into production. |

See `ARCHITECTURE.md` at the pack root for the Vectr / circuit /
aggregator / handler / security explainer, and `DEPLOY.md` for the
detailed single-op and multi-op walkthroughs covering troubleshooting,
log lines to look for, and what to do when one operator's Round 1
attestation gets dropped (most often: `try_check_one` rejecting an
unregistered signer because `register-signers` hasn't been re-run
after a mnemonic rotation).
