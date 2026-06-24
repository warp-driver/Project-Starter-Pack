#!/usr/bin/env bash
#
# new-project.sh — scaffold a fresh WarpDrive project from one of the
# bundled examples.
#
# Usage:
#   ./scripts/new-project.sh <example> <dest>
#
# Example:
#   ./scripts/new-project.sh 01-counter ../my-counter
#
# Layout copied into <dest>:
#   - examples/<example>/*       (the example skeleton becomes the project root)
#   - scripts/                   (own copy — cross-project symlinks are fragile in CI)
#   - vendor/                    (vendored ed25519-{security,verification})
#   - .gitignore
#   - rust-toolchain.toml
#   - warpdrive.toml.template → warpdrive.toml
#
# Then `git init` + one commit. We never poke ~/.gitconfig; whatever
# author git is configured with becomes the commit author.
#
# Rust crate identifiers in Cargo.toml / lib.rs are NOT auto-rewritten —
# the example README walks the user through the sed one-liner.

set -euo pipefail

if [ "$#" -ne 2 ]; then
    echo "usage: $0 <example> <dest>" >&2
    echo "  e.g. $0 01-counter ../my-counter" >&2
    exit 2
fi

EXAMPLE="$1"
DEST="$2"

# Run from the pack root regardless of where the user invoked us from,
# so all the cp sources resolve consistently.
PACK_ROOT="$(cd "$(dirname "$0")/.." && pwd)"
cd "$PACK_ROOT"

SRC="examples/$EXAMPLE"

test -d "$SRC" \
    || { echo "no such example: $SRC (looked under $PACK_ROOT)" >&2; exit 1; }
test ! -e "$DEST" \
    || { echo "destination already exists: $DEST" >&2; exit 1; }

# Pre-flight: every top-level artefact the new project needs must exist
# in the pack root. Fail before the first cp so we never leave a
# half-populated $DEST behind.
for f in scripts vendor .gitignore rust-toolchain.toml warpdrive.toml.template; do
    test -e "$f" \
        || { echo "missing pack artefact: $PACK_ROOT/$f" >&2; exit 1; }
done

echo "+ scaffolding from $SRC -> $DEST"
cp -r "$SRC" "$DEST"

echo "+ copying scripts/ -> $DEST/scripts/"
cp -r scripts "$DEST/scripts"

echo "+ copying vendor/ -> $DEST/vendor/"
cp -r vendor "$DEST/vendor"

echo "+ copying .gitignore"
cp .gitignore "$DEST/.gitignore"

echo "+ copying rust-toolchain.toml"
cp rust-toolchain.toml "$DEST/rust-toolchain.toml"

echo "+ copying warpdrive.toml.template -> warpdrive.toml"
cp warpdrive.toml.template "$DEST/warpdrive.toml"

echo "+ initialising git repo in $DEST"
(
    cd "$DEST"
    git init -q
    git add -A
    git commit -q -m "scaffold from Project-Starter-Pack"
)

ABS_DEST="$(cd "$DEST" && pwd)"

cat <<EOF

Next steps:
  cd $ABS_DEST
  edit Cargo.toml package names (see README)
  task fetch-wit
  ./scripts/bootstrap-keys.sh > .env
EOF
