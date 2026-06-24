## vendor/

Third-party Soroban contracts bundled with the Starter Pack. Vendored so a
clone of this repo builds offline against pinned source, with no `git
submodule` ceremony.

## contracts/

`ed25519-security` and `ed25519-verification` are copied verbatim from

  https://github.com/warp-driver/warpdrive-contracts/tree/main/packages/contracts

These are the two contracts a WarpDrive handler links against to validate
an operator-signed `XlmEnvelope`:

- `ed25519-security` — owns the signer set (public keys + weights) and the
  threshold. Deployed once per integration; the deployer is the admin.
- `ed25519-verification` — stateless verifier. Calls into `ed25519-security`
  to fetch the signer set, then checks the supplied ed25519 signatures
  against the envelope hash and sums weights against the threshold. The
  consumer handler invokes `try_verify` (quorum) or `try_check_one`.

You SHOULD NOT modify these. They are the cryptographic trust root of every
WarpDrive integration; a local fork drifts from upstream audits and makes
operator-key rotation incompatible with sibling deployments. The only
reason to edit is a different signing scheme (e.g. secp256k1), in which
case fork upstream rather than diverge silently.

## Syncing to a newer upstream

```bash
git clone https://github.com/warp-driver/warpdrive-contracts /tmp/wc
cp -r /tmp/wc/packages/contracts/ed25519-security    vendor/contracts/
cp -r /tmp/wc/packages/contracts/ed25519-verification vendor/contracts/
rm -rf vendor/contracts/*/target vendor/contracts/*/Cargo.lock
```

Then bump the consumer `Cargo.lock` (`cargo update -p ed25519-security
-p ed25519-verification`) and re-run the integration tests.

## Why examples/ has its own copy

`examples/01-counter/contracts/ed25519-{security,verification}/` is a
fresh copy of these directories, not a symlink. Each example is a
self-contained Cargo workspace so `cargo build` in the example folder
Just Works without escaping into the parent tree.

Cargo's `[profile.release]` (wasm optimisation, panic = abort, LTO) only
applies at the workspace root — vendoring the contracts at root + linking
from examples by `path = "../../vendor/..."` would silently drop those
release settings when an example builds standalone. Keeping a per-example
copy preserves the optimisation profile end-to-end. See ARCHITECTURE.md
("per-contract Cargo.toml" note) for the full rationale.

When you sync upstream, refresh both locations:

```bash
cp -r vendor/contracts/ed25519-security    examples/01-counter/contracts/
cp -r vendor/contracts/ed25519-verification examples/01-counter/contracts/
```
