#!/usr/bin/env bash
# Local mirror of .github/workflows/ci.yml: the exact three gates the cloud CI
# runs — clippy across all targets with warnings denied, the full test suite,
# then the lattice-core dependency guard. Nothing more, nothing less, on the
# toolchain pinned by rust-toolchain.toml — so a green run here means a green
# run there.
#
# Run it directly:              ./ci.sh
# Or gate every push on it:     git config core.hooksPath .githooks
#                               (then `git push --no-verify` to skip one-off)
set -euo pipefail
cd "$(dirname "$0")"

run() { echo; echo "▶ $*"; "$@"; }

run cargo clippy --workspace --all-targets -- -D warnings
run cargo test --workspace

# lattice-core is MIT OR Apache-2.0 while the rest of the workspace is GPL.
# That split is only defensible while the crate stays a self-contained
# library, so its dependency list must stay empty: a GPL (or otherwise
# restrictive) dependency would silently contradict its stated license.
# Adding one is allowed, but must be a deliberate edit here, not a drive-by.
echo
echo "▶ lattice-core dependency guard"
deps=$(cargo metadata --no-deps --format-version 1 \
  | python3 -c 'import json,sys; p=[x for x in json.load(sys.stdin)["packages"] if x["name"]=="lattice-core"][0]; print(" ".join(sorted(d["name"] for d in p["dependencies"])))')
if [ -n "$deps" ]; then
  echo "✗ lattice-core must stay dependency-free (it is MIT OR Apache-2.0," >&2
  echo "  unlike the GPL workspace around it). Found: $deps" >&2
  echo "  If the dependency is intended and permissively licensed, update" >&2
  echo "  this guard in ci.sh and the rationale in crates/lattice-core/README.md." >&2
  exit 1
fi
echo "  ok — no dependencies"

echo
echo "✅ local CI passed (clippy + tests + lattice-core dep guard)"
