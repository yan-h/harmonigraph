#!/usr/bin/env bash
# Local mirror of .github/workflows/ci.yml: the formatting check, clippy across
# all targets with warnings denied, the full test suite, the plugin package
# check, baseview's own tests, the doc-link check, the harmonigraph-core
# dependency guard, then the two script gates — the worktree-reclaim lock cases
# and the bundle swap.
# Nothing more, nothing less, on the toolchain pinned by rust-toolchain.toml —
# so a green run here means a green run there.
#
# Run it directly:              ./ci.sh
# Or gate every push on it:     git config core.hooksPath .githooks
#                               (then `git push --no-verify` to skip one-off)
set -euo pipefail
cd "$(dirname "$0")"

run() { echo; echo "▶ $*"; "$@"; }

# Formatting, first because it is the cheapest gate here and the only one whose
# failure is fixed without reading anything: `cargo fmt --all`. The config is
# rustfmt.toml, tuned to the density this tree already had, so the formatter
# agrees with the code rather than fighting it.
#
# The point of the line is that nobody has to hold the house style in their
# head. Before it, style was a convention kept by attention alone, which is the
# scarcest thing in a repo whose commits mostly come from parallel sessions.
run cargo fmt --all --check

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

# ...and RUN harmonigraph-render's own tests in that same configuration, which
# is the half a check cannot do. The unification above does not merely compile
# the plugin with hot-reload on, it also deletes every
# `#[cfg(not(feature = "hot-reload"))]` test from the workspace binary — so the
# arm of `text_source` the bundle actually ships (lib.rs) and the test that
# pins it to the baked concatenation (text.rs) are both invisible to the run
# above. Dropping `with_common` from that arm would pass every gate and ship a
# plugin whose glyph pipelines compile text.wgsl without the common half,
# surfacing as a pipeline panic on first paint inside the DAW.
run cargo test -p harmonigraph-render

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
#
# What this gate CANNOT see, so that nobody plans around it again: rustdoc does
# not document `#[test]` functions, so a link in one is never resolved and a
# dead one never reported. `--cfg test` does not buy it back — the function is
# still skipped — and the only thing that reaches such a path is the compiler,
# through a `use` of it. Two commits in a row have now swept `cfg(test)` doc
# links by eye and reported the sweep complete while nine were still broken.
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

# The only gate here that guards a script rather than the crates. Its subject
# — which worktree locks are live — is the one decision in the tree that
# deletes a directory, and its inputs (a pid, its argv, the spare pool's
# sockets) are all outside the repo, so no cargo test can reach it. It has
# been wrong in both directions, most recently eagerly enough to make a live
# session's worktree removable, which is why it is a gate and not a habit.
run .claude/tests/reclaim-locks.sh

# The other gate that guards a script, and for the same reason: its subject is
# an inode and an ad-hoc signature, both outside the repo, so no cargo test can
# reach it — and getting it wrong is silent in the worst way, with every step
# reporting success and the DAW still drawing the previous build.
run .claude/tests/plugin-swap.sh

echo
echo "✅ local CI passed (fmt + clippy + tests + plugin check + baseview + doc links + harmonigraph-core dep guard + reclaim locks + plugin swap)"
