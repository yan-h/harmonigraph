#!/usr/bin/env bash
# Canonical full CI gate: formatting, markdown clause breaks, workspace clippy with warnings denied,
# workspace tests, the plugin package check, harmonigraph-render's own tests,
# vendored GUI crates' tests, the optional CLAP probe fixture, doc links, the harmonigraph-core dependency
# guard, worktree reclaim safety, and the registered-worktree bundle swap.
#
# GitHub Actions invokes this script unchanged on the toolchain pinned by
# rust-toolchain.toml. It remains available locally when a full run is useful.
#
# Run it directly: ./ci.sh
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

# The same idea for markdown, which rustfmt never opens. Prose here is laid out
# one clause per line, so a line break only ever falls where the text already
# had punctuation. That buys three mechanical properties, none of them about
# how it looks: an edit rewrites the clause it edits instead of reflowing the
# rest of the paragraph, two sessions editing neighbouring sentences of one
# paragraph touch different lines instead of colliding, and a line is a whole
# unit of text, so splicing paragraphs cannot strand a fragment on its own line
# — which is the defect that prompted this.
#
# A break inside a paragraph renders as a space, so none of this changes a
# rendered byte. `--write` fixes every failure this reports.
run .claude/semantic-breaks.py --check

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

# #615's optional apparatus exercises the actual CLAP boundary and callback
# allocation guard. Default workspace tests cannot see this feature.
run cargo clippy -p harmonigraph-plugin --all-targets --features tuning-probe -- -D warnings
run cargo test -p harmonigraph-plugin --features tuning-probe,nice-plug/assert_process_allocs \
  probe::tests::

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
# dependencies and runs none of their tests. The GUI patches carry focused
# tests: baseview's macOS view decides when a gesture ends, and egui-baseview
# tests the font-atlas bound. nice-plug's upstream tests exercise macros and
# serialization; its patched CLAP boundary is exercised by the exported-CLAP
# fixture above, including activation latency and allocation guards. The nested
# crate cannot see the workspace's baseview patch, so its command repeats that
# path explicitly; WGPU is the backend the plugin ships.
#
# Each crate is its own workspace root, so keep both of their targets under
# `target/debug`: the idle-worktree reclaimer owns that whole subtree.
run cargo test --manifest-path vendor/baseview/Cargo.toml --target-dir target/debug/vendor-baseview
run cargo test --manifest-path vendor/egui-baseview/Cargo.toml \
  --no-default-features --features wgpu,tracing \
  --config 'patch.crates-io.baseview.path="vendor/baseview"' \
  --target-dir target/debug/vendor-egui-baseview

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

# The security audit has trigger-dependent behavior that no Rust test reaches:
# scheduled and manual runs always scan, while a main push may legitimately
# skip. Keep unlike triggers out of each other's cancellation groups so the
# skip cannot erase the scan it was meant to complement.
run .claude/tests/audit-workflow.sh

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

# The only gate here that guards a script rather than the crates. Its subjects
# — which worktrees Claude owns and which of their locks are live — decide
# whether the tree can delete a directory, and their inputs (a path, a pid, its
# argv, and the spare pool's sockets) are outside the repo, so no cargo test can
# reach them. The lock has been wrong in both directions, most recently eagerly
# enough to make a live session's worktree removable, which is why this is a
# gate and not a habit.
run .claude/tests/reclaim-locks.sh

# The other gate that guards a script, and for the same reason: its subjects
# are Git's registered worktree paths, an inode and an ad-hoc signature, all
# outside the repo. Getting any one wrong is silent in the worst way: a branch
# build is absent from the menu, or every swap step reports success while the
# DAW still draws the previous build.
run .claude/tests/plugin-swap.sh

echo
echo "✅ full CI passed (fmt + markdown breaks + workspace clippy + workspace tests + plugin check + render tests + vendored tests + doc links + harmonigraph-core dep guard + reclaim safety + plugin swap)"
