#!/usr/bin/env bash
# Local mirror of .github/workflows/ci.yml: the exact two gates the cloud CI
# runs — clippy across all targets with warnings denied, then the full test
# suite. Nothing more, nothing less, on the toolchain pinned by
# rust-toolchain.toml — so a green run here means a green run there.
#
# Run it directly:              ./ci.sh
# Or gate every push on it:     git config core.hooksPath .githooks
#                               (then `git push --no-verify` to skip one-off)
set -euo pipefail
cd "$(dirname "$0")"

run() { echo; echo "▶ $*"; "$@"; }

run cargo clippy --workspace --all-targets -- -D warnings
run cargo test --workspace

echo
echo "✅ local CI passed (clippy + tests)"
