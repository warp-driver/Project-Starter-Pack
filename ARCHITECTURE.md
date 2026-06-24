# WarpDrive Architecture (for Starter Pack users)

WarpDrive is a network of off-chain compute nodes (Vectrs) that produce
quorum-signed attestations a Soroban contract can verify on-chain. It is
not a blockchain. It is not a rollup. It is a signature service: a fixed
set of operators each run the same sandboxed program against the same
trigger, collect each other's results over libp2p, and ship one ed25519 /
SEP-53 envelope to Stellar that a vendored verification contract checks
against a registered signer set with a configured weight threshold.

Everything downstream of that envelope is a normal Soroban contract call.
This document describes the five layers a WarpDrive integration is built
from, the trust assumptions you inherit, and the few cross-layer
conventions you have to respect for the bytes to line up.

The reference reading is `/oracle-demo`, `/hodlers-app`, and
`/phoenix-blend-pool` in the same parent directory as this repo. Every
pattern here is lifted verbatim from one of them.

## The five layers

A WarpDrive integration has five layers. Each reference project has the
same five — only the trigger source, payload shape, and final on-chain
side-effect differ.

### 1. Trigger

What wakes a Vectr up and feeds it the data its circuit will react to.
The Vectr daemon owns the trigger plumbing — you describe what you want
in `warpdrive.toml` / the service spec, and your circuit receives one
`TriggerAction` per fire.

The kinds the Vectr supports today:

- `Cron` — a cron expression. Fires on each operator independently; no
  external data is attached. Use it for sampling external state on a
  cadence (oracle-demo's `cron-circuit` polls CoinGecko every 30 s) or
  for periodic on-chain housekeeping (`phoenix-blend-pool` harvests
  Blend yield every 4 h).
- `StellarContractEvent` — subscribes to events emitted by a contract
  on a configured Stellar network. The event's `topic_segments` + `value`
  arrive as XDR you decode in the circuit. The pubnet pool that
  hodlers-app watches and the testnet `Round2Ready` composition event
  oracle-demo listens for are both this kind.
- `EvmContractEvent` — subscribes to logs from a contract on a
  configured EVM chain. Oracle-demo's `eth-bridge-circuit` listens for
  Sepolia `TwapRequested(string,uint32,address)` logs and turns them
  into Stellar requests.

`AtprotoEvent` and `BlockInterval` triggers also exist. Atproto wires
a firehose subscription; block-interval fires every N ledgers of a
configured chain. Neither shows up in the Starter Pack's examples —
the three above cover almost every integration. The pack ships two
of the three out of the box: `01-counter` (Cron) and `02-event-watcher`
(StellarContractEvent).

Which one to use:

- Reacting to an on-chain action on Stellar → `StellarContractEvent`.
- Reacting to an on-chain action on Ethereum or another EVM chain →
  `EvmContractEvent`.
- Polling something external, sampling a price, or running a heartbeat
  → `Cron`.

### 2. Circuit (WASI 0.2)

A sandboxed wasm component built with `cargo-component` and targeting
`wasm32-wasip1`. The Vectr loads it from its local component store and
calls a single export per trigger fire. Each operator runs the circuit
inside its own Vectr; the function fires once per trigger per operator.

The imports the host gives you:

- `wasi:keyvalue` — persistent per-Vectr KV store, plus `atomics` for
  compare-and-swap when one trigger fans out into multiple concurrent
  events the circuit must merge (the canonical example is the hodlers
  swap-accumulator, where eight events arrive in parallel for one logical
  swap and CAS keeps the last writer from clobbering the others).
- `wasi:clocks/wall-clock` — wall time. `std::time::SystemTime` is
  unreliable inside the component; always read time through this
  interface.
- `wasi:http` — outbound HTTP, restricted to hosts the node operator has
  whitelisted in `warpdrive.toml`. CoinGecko, Blend RPC, an
  application-specific REST API all go through this surface.
- `warpdrive:vectr/input` — the trigger data the host gives you for this
  fire (`TriggerData::StellarContractEvent`, `Cron`, …).
- `warpdrive:vectr/output` — the `WasmResponse` type you return, plus
  `host::config_var(...)` for service-level configuration the deployer
  bakes in (API keys, contract addresses, chain selectors).

The export:

- A single `Guest::run(t: TriggerAction) -> Result<Vec<WasmResponse>, String>`.
  Returning an empty vector is the supported "no-op for this fire" signal
  (oracle-demo's cron circuit returns `Submit::None`; hodlers' circuit
  returns `vec![]` until enough event fragments have accumulated).

Canonical lib.rs shape (the twap-circuit from oracle-demo, stripped of
its mod re-exports):

```rust
wit_bindgen::generate!({
    world: "circuit-world",
    path: "../../wit-definitions/wit",
    generate_all,
});

use warpdrive::vectr::input::TriggerData;

struct Component;

impl Guest for Component {
    fn run(t: TriggerAction) -> Result<Vec<WasmResponse>, String> {
        run_inner(t).map_err(|e| format!("twap-circuit: {e:#}"))
    }
}

fn run_inner(action: TriggerAction) -> anyhow::Result<Vec<WasmResponse>> {
    let event = match action.data {
        TriggerData::StellarContractEvent(e) => e.event,
        _ => anyhow::bail!("expected StellarContractEvent trigger"),
    };
    let payload_bytes = encode_payload(&event)?; // XDR-encoded ScVal::Map
    Ok(vec![WasmResponse {
        payload: payload_bytes,
        ordering: None,
        event_id_salt: None,
    }])
}

export!(Component);
```

`payload` is opaque XDR the host never inspects — it is whatever bytes
the handler will decode on the other end. `ordering` is an optional u64
the host uses to serialise submissions for the same handler.
`event_id_salt` controls how the host derives the deduplication key for
this submission (covered below in "Salt vs event_id").

### 3. Aggregator (WASI 0.2)

A second WASI 0.2 component, built the same way as a circuit but against
the `aggregator-world`. After every operator's circuit has emitted its
`WasmResponse` (or after the operator quorum has produced one), the
Vectr feeds the result into the aggregator and the aggregator decides
what host action to take.

For Stellar handlers the aggregator's job is trivial: emit one
`SubmitAction::Stellar { chain, address }` per signed envelope, pointing
at the handler contract. The actual envelope-shipping — XDR-encoding
`XlmEnvelope`, peering with the other operators over libp2p to collect
signatures, packing them into `Ed25519SignatureData`, and finally
calling `verify_xlm` on the handler — happens in the Vectr's submission
manager. The aggregator only names the destination.

This is why the same aggregator wasm is reused verbatim across the
three reference projects: it reads `chain` and `service_handler` from
its config and emits one `SubmitAction`. Nothing project-specific lives
there. The Starter Pack ships the same one under
`examples/01-counter/components/aggregator/`.

Where signing happens:

- Each operator's Vectr signs the envelope it would submit with the
  ed25519 key derived from `WARPDRIVE_SIGNING_MNEMONIC`.
- Operators gossip their signatures over libp2p.
- For full-quorum workflows the Vectr's submission manager collapses
  the per-operator envelopes into one envelope whose
  `Ed25519SignatureData` carries N signatures over the same payload.
- For single-signer workflows (oracle-demo's Round 2) each operator
  submits its own envelope; the handler accumulates them on chain.

The aggregator never sees private key material. It runs deterministically
against the post-quorum result.

### 4. Handler (Soroban contract)

A Soroban contract with one envelope-entry: `verify_xlm(envelope_bytes,
sig_data)`. The Vectr submission manager invokes this on every
aggregated submission against this handler — there is no per-round or
per-event entrypoint to dispatch to, so the handler decides what the
envelope means by decoding its payload.

The handler's job is fixed:

1. XDR-decode `envelope_bytes` into `XlmEnvelope`.
2. Check `envelope.event_id` against the seen-set; reject duplicates.
3. Call `Ed25519VerificationClient::try_verify` (full quorum) or
   `try_check_one` (single signer) with the envelope and `sig_data`.
4. Decode the inner payload from `envelope.payload`.
5. Dispatch to your application logic, passing the decoded fields.
6. Mark `event_id` seen and publish a `Verified` event.

The single-source-of-truth for the inner payload shape is a Soroban
`#[contracttype]` enum living on the handler. When the handler decodes
the envelope, it matches on the variant and dispatches; the circuit
constructs the matching XDR by hand using `stellar-xdr` (it has no
Soroban SDK — it is a WASI component, not a contract).

Dispatcher arm shape (from oracle-demo's `OracleContract::verify_xlm`):

```rust
let envelope = XlmEnvelope::from_xdr(&env, &envelope_bytes)
    .map_err(|_| HandlerError::InvalidEnvelope)?;
let event_id = envelope.event_id.clone();
if storage::is_event_seen(&env, &event_id) {
    return Err(HandlerError::EventAlreadySeen);
}

let payload = SubmissionPayload::from_xdr(&env, &envelope.payload)
    .map_err(|_| HandlerError::InvalidEnvelope)?;
let verification = Ed25519VerificationClient::new(
    &env, &storage::get_verification_contract(&env));

match payload {
    SubmissionPayload::Round2(p) => {
        // Single-signer attestation: each operator's value differs,
        // so the host cannot quorum-collapse. try_check_one only
        // verifies one signature against the registered signer set.
        verification.try_check_one(/* envelope, sig, signer, ref_block */)?;
        Self::apply_round2(&env, p, &event_id)?;
    }
    SubmissionPayload::Final(p) => {
        // Quorum-signed: deterministic median over the bundle, every
        // operator agrees, the host collapses N envelopes into one.
        verification.try_verify(/* envelope, sigs, signers, ref_block */)?;
        Self::apply_final(&env, p, &event_id)?;
    }
}
storage::mark_event_seen(&env, &event_id);
Verified::new(event_id).publish(&env);
```

The Starter Pack's `01-counter` handler is a degenerate case of the
same shape: one variant (`Tick { ts }`), one call
(`counter.tick(ts)`), ~150 LOC of contract. `02-event-watcher`'s
handler is the same skeleton with a different inner payload
(`Record { msg, msg_id }`) and a different downstream call
(`message_board.record_signed(msg_id, msg)`) — the dispatch
structure carries over.

### 5. Security (ed25519-security + ed25519-verification)

Two vendored contracts from `warpdrive-contracts`. You do not write
these; you copy them into `contracts/` and configure them at deploy
time.

- `ed25519-security` — the registry. Holds the signer set (each pubkey
  with a weight) and the consensus threshold (a numerator/denominator
  pair against the sum of weights). Operators are registered after
  initial deploy via `task register-signer`. Its admin keys live with
  whoever owns the deployment (typically the project's deployer
  account); rotating the threshold or evicting a signer is a single
  admin tx, no redeploy required.
- `ed25519-verification` — the checker. Points at one
  `ed25519-security` instance. Exposes `try_verify(envelope, sigs,
  signers, ref_block)` — which sums the matched weights at the
  reference block and accepts if the total meets the threshold — and
  `try_check_one(envelope, sig, signer, ref_block)` — which only
  requires the signature to come from a registered signer with
  non-zero weight at that block, without enforcing quorum.

The handler calls verification; verification reads from security. You
usually never modify either contract. The exception is signing scheme:
the warpdrive-contracts repo also ships `secp256k1-security` /
`secp256k1-verification` for EVM-style signatures — fork those if the
handler needs to verify secp256k1 instead. The Stellar-native path
uses ed25519 / SEP-53 across the board, which is what the Starter Pack
ships.

## Data flow (one full round-trip)

```
   trigger fires  (cron / Stellar event / EVM log)
        │
        ▼          ─── parallel, one per operator ───
   ┌────────────────────────┬────────────────────────┐
   │  Vectr A               │  Vectr B               │
   │   circuit.run(action)  │   circuit.run(action)  │
   │   → WasmResponse {     │   → WasmResponse {     │
   │       payload (XDR),   │       payload (XDR),   │
   │       ordering,        │       ordering,        │
   │       event_id_salt }  │       event_id_salt }  │
   │   host builds          │   host builds          │
   │   XlmEnvelope, signs   │   XlmEnvelope, signs   │
   │   (ed25519 / SEP-53)   │   (ed25519 / SEP-53)   │
   └────────────┬───────────┴────────────┬───────────┘
                └──── libp2p gossip ─────┘
                             │
                             ▼
   submission mgr: collapse N envelopes with matching
   event_id into one envelope with N signatures;
   aggregator picks chain + handler address.
                             │
                             ▼
   handler.verify_xlm(envelope_bytes, sig_data)
     ├─ XDR-decode envelope, reject if event_id seen
     ├─ verification.try_verify(...) / try_check_one(...)
     ├─ decode payload (matches a #[contracttype])
     └─ dispatch into app contract (counter.tick / ...)
```

Where each step happens:

- **XDR encoding** of the inner payload happens in the circuit, on
  every operator, before signing. The circuit is the byte-shape
  authority.
- **Signing** happens in the Vectr submission manager with the
  operator's local ed25519 key. The circuit never touches the key.
- **`event_id`** is derived by the host as `hash(workflow_id,
  trigger_data, event_id_salt)`. Same id across operators → quorum
  collapse. Different id → independent envelopes.
- **Verification** happens in the handler, calling out to the
  `ed25519-verification` contract — which is what makes the trust
  rooted on-chain rather than in any one Vectr.

## Trust model

The on-chain handler accepts any envelope whose ed25519 signatures sum
to at least the threshold weight registered on the configured
`ed25519-security` contract at the envelope's reference block. There is
no other auth gate.

That has four consequences worth internalising:

- **Anyone can call the handler.** `verify_xlm` is an open entry point.
  Trust comes entirely from upstream: only the operator quorum can
  produce envelopes whose signatures verify. A non-operator can call
  `verify_xlm` all day; nothing they submit will verify.
- **The application contract trusts the handler via address pin.** The
  counter / hodlers / pool / oracle stores the handler address at
  deploy and checks the caller. The handler is the only contract
  allowed to advance application state, and it only does so once the
  envelope has cleared `try_verify`. There is no admin override path.
- **Replay protection lives in the handler.** Each envelope carries a
  20-byte `event_id`. The handler records every verified `event_id`
  in persistent storage; a second submission with the same id rejects
  before any signature work. Aggregator-level submission races (where
  multiple operators try to land the same envelope on chain) resolve
  cleanly: one tx succeeds, the rest get `EventAlreadySeen` and back
  off.
- **The threshold is on-chain configuration, not code.** Rotating the
  signer set, evicting a compromised operator, or moving from 1/1 dev
  to 4/5 production is a `task set-threshold` admin tx against
  `ed25519-security`. The handler does not need to be redeployed.

What the model does NOT cover:

- It does not constrain what each operator's circuit does internally.
  For full-quorum workflows the on-chain check is the defence: a rogue
  operator's envelope will not match the honest ones and will not
  collapse. For single-signer workflows (oracle-demo's Round 2) a
  rogue operator's attestation lands as one entry — the application
  must be designed so a minority of attestations cannot move state.
- It does not protect against a compromised threshold quorum. If
  4-of-5 operator keys are stolen, the attacker produces verifying
  envelopes. The defence is operator distribution, not the protocol.

## Payload conventions

Two conventions the circuit and handler have to agree on byte-for-byte.
Both are easy to break and only show up at deploy time as opaque
`InvalidEnvelope` errors.

### Single source of truth

The inner payload (whatever lives in `envelope.payload`) is a Soroban
`#[contracttype]` defined on the handler. That contract type is the
authoritative shape. The circuit constructs the matching XDR by hand
using `stellar-xdr` — the WASI component has no `soroban-sdk` (it is
not a contract, and pulling in `soroban-sdk` would balloon the wasm
binary and force a host that isn't there).

Two rules that bite if ignored:

- **Field names matter.** A `#[contracttype]` struct or enum encodes
  to an `ScVal::Map` keyed by field-name `ScVal::Symbol`s. The
  circuit must emit symbols whose UTF-8 bytes exactly match the
  field idents on the contract side. Renaming a contract field
  silently breaks decoding on the next deploy.
- **Alphabetical ordering matters.** The Soroban XDR decoder requires
  the map entries to be sorted alphabetically by key. The circuit
  builds the `ScVal::Map` by hand: emit the entries in sorted order
  or `from_xdr` returns `InvalidEnvelope`. With one field this is
  trivial; with more, it is the first thing to check when a freshly
  added field starts rejecting.

For multi-payload-shape integrations, wrap the per-shape structs in a
single `#[contracttype]` enum (oracle-demo's `SubmissionPayload`).
The variant tag is part of the envelope; the handler matches on it
and dispatches. There is no per-round entry point — every submission
goes through one `verify_xlm`.

### Salt vs event_id

The `event_id` the handler dedupes on is computed by the host as
`hash(workflow_id, trigger_data, salt)`. The circuit controls `salt`
through `WasmResponse::event_id_salt`. Different choices give
different downstream behaviours:

- **`event_id_salt = None`** (or every operator returning the same
  salt) → every operator's envelope derives the same `event_id`. The
  submission manager treats them as the same envelope and quorum-
  collapses N signatures into one. Use this for deterministic
  computations: the canonical `Tick { ts }` (where `ts` is the
  trigger's cron firing time, the same on every operator), the
  hodlers swap payload (the events are the same on mainnet for every
  operator), the Round 3 median (the same bundle yields the same
  median on every operator).
- **`event_id_salt = Some(unique_per_operator_bytes)`** → each
  operator's envelope derives a different `event_id`. No collapse.
  Each operator ships its own envelope and the handler treats them
  as independent submissions to bundle. Use this for single-signer
  attestations where every operator's payload legitimately differs:
  oracle-demo's Round 2 sets `event_id_salt = Some(payload_bytes)`
  because two operators sampling CoinGecko a second apart will never
  agree byte-for-byte on a TWAP, and quorum-collapsing them would
  throw away valid attestations.

In short: deterministic → no salt, full-quorum. Non-deterministic →
per-operator salt, single-signer (`try_check_one`), bundle on chain
until the application logic decides the bundle is final.

## When to deviate from the pattern

The five-layer baseline covers most integrations. A few situations
require deliberate deviation:

- **Per-contract `[profile.release]`.** Cargo only applies
  `[profile.release]` settings (notably `overflow-checks = true`) at
  the workspace root. Putting all four contracts in one workspace
  means overflow-checks silently does not propagate to the contract
  crates. Every reference project keeps each contract as its own
  standalone Cargo package with its own `Cargo.toml` + lockfile so
  overflow-checks actually applies. Do not refactor the Starter Pack
  into a unified workspace.
- **CAS via `wasi:keyvalue/atomics`.** When one trigger fans out into
  N concurrent fires the circuit must merge (Phoenix's 8 sub-events
  per swap, all firing in parallel), naive `get` / `set` clobbers
  earlier writers and the accumulator never finalises. Use
  `wasi:keyvalue/atomics` for last-write-wins-only-when-snapshot-
  matches, retry on conflict, and pair the record with a `finalized:
  bool` tombstone so subsequent fires for the same logical unit no-op.
- **Determinism in multi-operator payloads.** Anything that varies
  between operators breaks quorum collapse. The obvious traps:
  `host_now_secs()` (each operator reads a different wall-clock), any
  HTTP fetch (each operator's response differs by milliseconds), and
  any randomness. For deterministic payloads, derive every field from
  the trigger data (event timestamps, ledger numbers, sender
  addresses) — anything the host gives every operator identically. If
  a field must come from an external source, run the circuit as
  single-signer (`event_id_salt = Some(...)`) and bundle on chain.
- **Single-signer attestations + on-chain bundling.** When the payload
  is unavoidably non-deterministic, the application contract has to
  carry the bundling logic: accept per-operator single-signed
  attestations, accumulate them, and run the deterministic combiner
  (median, average, majority) on-chain or in a follow-up composition
  circuit. Oracle-demo splits this into Round 2 (single-signer, bundle
  on chain) and Round 3 (deterministic median over the bundle,
  full-quorum). The Starter Pack does not exercise this path.

## Where this came from

The patterns above are distilled from three working projects under the
same parent directory as this repo.

**`hodlers-app`** — a Phoenix XLM-USDC swap watcher. One circuit, one
aggregator, one handler, one ledger contract. The circuit subscribes
to swap events on Stellar mainnet, accumulates the eight per-swap
events with CAS into a single `(trader, delta)` payload, and emits one
quorum-signed `add_points` on testnet per logical swap. This is the
smallest end-to-end pattern and the template for the Starter Pack's
`01-counter` example. Read its `components/circuit/` for the
CAS-accumulator pattern and its `contracts/stellar-handler/` for the
canonical ~80-line handler.

**`oracle-demo`** — a multi-round price oracle. Three circuits
(`cron-circuit` for sampling, `twap-circuit` for the Round 2 per-Vectr
attestation, `median-circuit` for the Round 3 quorum-signed final),
plus an `eth-bridge-circuit` that turns Sepolia logs into Stellar
requests. Demonstrates: multi-round composition (Round 3 listens on a
Soroban event Round 2 emits via the handler), single-signer +
full-quorum coexistence in one handler (the `SubmissionPayload` enum
pattern), and cross-chain triggering (EVM event → Stellar contract
call). Read its `contracts/oracle/src/contract.rs` for the dispatcher
shape and its `components/twap-circuit/` for the canonical small
circuit.

**`phoenix-blend-pool`** — a Blend yield rebalancer for the Phoenix
blended pool. Two workflows on one handler (a Stellar-event-triggered
`Rebalance` action and a cron-triggered `HarvestYield`), production-
grade operator-set (5 operators, 4-of-5 quorum), and a richer handler
that holds delegate authority over a forked Phoenix pool. Demonstrates
the production deployment shape: independent operator infrastructure,
weighted quorum, on-chain configuration getters for dashboards, and
the `RebalanceAction` enum-only payload pattern (the envelope carries
intent, the handler reads live chain state to decide amounts). Read
its `ARCHITECTURE.md` for the sign-off-pack treatment of trust model
and operator-set composition.

The Starter Pack distils the byte-identical parts of those three
projects — the vendored security contracts, the scripts, the
warpdrive.toml shape, the cookie-cutter handler — and ships one
minimal example (`01-counter`: cron → counter contract) you can read
end-to-end in a sitting before reaching for any of the above as a
reference.
