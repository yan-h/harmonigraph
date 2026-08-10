#!/usr/bin/env bash
# Local mirror of .github/workflows/ci.yml: the exact six gates the cloud CI
# runs — clippy across all targets with warnings denied, the full test suite,
# the plugin package check, baseview's own tests, the doc-link check, then
# the harmonigraph-core dependency guard. Nothing more, nothing less, on the
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

# The standalone harness enables harmonigraph-render's `hot-reload` feature,
# and cargo unifies features across a --workspace build, so every check
# above compiles harmonigraph-plugin with hot-reload on — a configuration
# the release bundle never ships. A `#[cfg(feature = "hot-reload")]`-only
# borrow could pass every gate above and still fail the plugin's own build.
# Checking the plugin package on its own resolves features from only its
# dependency edge, so it builds the same configuration the bundle does.
run cargo check -p harmonigraph-plugin

# The vendored crates are `exclude`d from the workspace (the `[workspace]`
# table's own key, in Cargo.toml), so `--workspace` compiles them as
# dependencies and runs none of their tests. The patches they carry are the
# reason to run them: the ones in baseview's macOS view decide when a gesture
# is over, which is not something to find out about in the DAW. Only baseview
# — egui-baseview's twelve patches carry no tests to run.
#
# `--target-dir` because the crate is its own workspace root and would
# otherwise open a second target/ inside vendor/, 113 MB per worktree on a
# machine that has already been filled once by target dirs.
run cargo test --manifest-path vendor/baseview/Cargo.toml --target-dir target/vendor-baseview

# Doc links, which is the only mechanical check on comments this tree has.
# Comments are ~40% of the non-blank lines under crates/ and carry the
# rationale — measurements taken, alternatives rejected, host quirks worked
# around — that the code cannot state for itself, so a reference that quietly
# stops naming anything is a real defect and not a formatting nit.
#
# `--document-private-items` is the point rather than a detail: most items here
# are private, so without it rustdoc checks only the exported surface and
# ignores the great majority of the doc comments.
#
# `private_intra_doc_links` is ALLOWED, and is the one lint whose complaint
# does not apply. It fires where a public item's docs point at a private one —
# accurate for anyone reading the source, and only a problem for published
# HTML, which nothing here produces. Everything else in the group is denied.
run env RUSTDOCFLAGS="-D rustdoc::all -A rustdoc::private_intra_doc_links" \
  cargo doc --no-deps --quiet --workspace --document-private-items

# harmonigraph-core is MIT OR Apache-2.0 while the rest of the workspace is GPL.
# That split is only defensible while the crate stays a self-contained
# library, so its dependency list must stay empty: a GPL (or otherwise
# restrictive) dependency would silently contradict its stated license.
# Adding one is allowed, but must be a deliberate edit here, not a drive-by.
echo
echo "▶ harmonigraph-core dependency guard"
deps=$(cargo metadata --no-deps --format-version 1 \
  | python3 -c 'import json,sys; p=[x for x in json.load(sys.stdin)["packages"] if x["name"]=="harmonigraph-core"][0]; print(" ".join(sorted(d["name"] for d in p["dependencies"])))')
if [ -n "$deps" ]; then
  echo "✗ harmonigraph-core must stay dependency-free (it is MIT OR Apache-2.0," >&2
  echo "  unlike the GPL workspace around it). Found: $deps" >&2
  echo "  If the dependency is intended and permissively licensed, update" >&2
  echo "  this guard in ci.sh and the rationale in crates/harmonigraph-core/README.md." >&2
  exit 1
fi
echo "  ok — no dependencies"

echo
echo "✅ local CI passed (clippy + tests + plugin check + baseview + doc links + harmonigraph-core dep guard)"
