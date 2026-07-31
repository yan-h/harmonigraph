#!/usr/bin/env bash
#
# Rebuild the plugin from the CURRENT checkout (works from a git worktree)
# and swap the fresh binary into the MAIN checkout's target/bundled/ — the
# location Bitwig scans — then re-sign ad-hoc (required on Apple Silicon,
# where an invalid signature makes the dynamic loader refuse the binary).
#
# Run this after ANY change you want to see in the DAW, then rescan/restart
# the plugin in Bitwig. Works from the main checkout or any worktree.
#
# Why this script exists (two footguns it sidesteps):
#   1. `cargo xtask bundle` picks the *topmost* Cargo.toml ancestor, so run
#      from a nested worktree it silently bundles the MAIN checkout's
#      sources, not the branch (nice-plug-xtask's chdir_workspace_root).
#   2. Each worktree has its own target/, but Bitwig only scans the main
#      checkout's target/bundled/, so a branch build is invisible there
#      until its binary is copied over.
# Plain `cargo build` resolves the *nearest* workspace root (= the branch);
# we then copy the artifact into the main checkout's bundles explicitly.
set -euo pipefail

PKG="harmonigraph-plugin"
NAME="Harmonigraph"
# Cargo names a LIB artifact after the lib target, which is the package name
# with dashes folded to underscores — so the dylib is libharmonigraph_plugin,
# not libharmonigraph-plugin. `cargo build -p` below wants $PKG itself, so the
# two spellings have to be kept apart.
LIB="${PKG//-/_}"

# Current checkout (where the build output lands) and the main working tree
# (first entry of `git worktree list`, whose target/bundled/ Bitwig scans).
HERE="$(git rev-parse --show-toplevel)"
# `substr($0, 10)` rather than `$2`: awk splits on whitespace, so a checkout
# under a path with a space in it ("~/My Drive/projects/...", which this repo
# has lived under) would come back truncated at the space — and a truncated
# MAIN makes BUNDLED a path that simply is not there, so the copy below goes
# quiet instead of failing. `.claude/reclaim-worktrees.sh` reads the same
# record the same way.
MAIN="$(git worktree list --porcelain | awk '/^worktree /{print substr($0, 10); exit}')"
BUNDLED="$MAIN/target/bundled"
DYLIB="$HERE/target/release/lib${LIB}.dylib"

echo "Building $PKG (release) from $HERE ..."
( cd "$HERE" && cargo build --release -p "$PKG" )
[ -f "$DYLIB" ] || { echo "ERROR: $DYLIB not found after build" >&2; exit 1; }

updated=0
for EXT in clap vst3; do
  BUNDLE="$BUNDLED/$NAME.$EXT"
  if [ ! -d "$BUNDLE" ]; then
    echo "WARNING: $BUNDLE missing — create the bundles once from the main" >&2
    echo "         checkout: (cd \"$MAIN\" && cargo xtask bundle $PKG --release)" >&2
    continue
  fi
  cp "$DYLIB" "$BUNDLE/Contents/MacOS/$NAME"
  codesign --force --sign - "$BUNDLE"
  codesign --verify --verbose=1 "$BUNDLE"
  echo "Updated + signed: $BUNDLE"
  updated=$((updated + 1))
done

# The offline video renderer, installed where the plugin's "Render video
# when done" setting looks for it by default. A fixed location beats the
# plugin trying to guess at a host's working directory or its own bundle
# path — and it means the renderer always matches the build it shipped
# with, which matters because they share the take format.
SUPPORT="$HOME/Library/Application Support/$NAME"
( cd "$HERE" && cargo build --release -p harmonigraph-offline )
if [ -f "$HERE/target/release/harmonigraph-offline" ]; then
  mkdir -p "$SUPPORT"
  cp "$HERE/target/release/harmonigraph-offline" "$SUPPORT/harmonigraph-offline"
  echo "Updated renderer:  $SUPPORT/harmonigraph-offline"
else
  echo "WARNING: harmonigraph-offline not built; auto-render will not work" >&2
fi

echo
if [ "$updated" -gt 0 ]; then
  # Record which build is now in the shared slot so load-plugin.sh's "<- now"
  # marker (and any session asking "what's loaded?") reflects this swap too.
  {
    echo "worktree=$HERE"
    echo "branch=$(git -C "$HERE" symbolic-ref --short -q HEAD || git -C "$HERE" rev-parse --short HEAD)"
    echo "commit=$(git -C "$HERE" rev-parse --short HEAD)"
    echo "loaded_at=$(date +%s)"
  } > "$BUNDLED/.loaded"
  echo "Done ($updated bundle(s)). Rescan/restart the plugin in Bitwig."
else
  echo "No bundles updated — see warnings above." >&2
  exit 1
fi
