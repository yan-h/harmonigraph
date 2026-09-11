#!/usr/bin/env bash
#
# update-plugin.sh — BUILD the current checkout (works from a git worktree) and
# load the result into the ONE bundle slot Bitwig scans. Build-and-load in a
# single shot; `load-plugin.sh` is the same swap without the build.
#
# Run this after ANY change you want to see in the DAW, then deactivate and
# reactivate Bitwig's audio engine. Works from the main checkout or any worktree.
#
# Why this script exists (two footguns it sidesteps):
#   1. `cargo xtask bundle` picks the *topmost* Cargo.toml ancestor, so run
#      from a nested worktree it silently bundles the MAIN checkout's
#      sources, not the branch (nice-plug-xtask's chdir_workspace_root).
#   2. Each worktree has its own target/, but Bitwig only scans the main
#      checkout's target/bundled/, so a branch build is invisible there
#      until its binary is copied over.
# Plain `cargo build` resolves the *nearest* workspace root (= the branch);
# `load-plugin.sh` then puts that artifact into the shared slot.
#
# The SWAP is not here. load-plugin.sh signs a staging bundle and installs its
# executable by atomic rename from a fresh sibling inode. Existing host mappings
# keep their old code until the process exits. Both entry points use that one
# implementation so signing, replacement and discovery cannot drift apart.
# That shared swap also refreshes Info.plist's discovery timestamp (#631),
# so Bitwig can enumerate changed CLAP classes after either install path.
set -euo pipefail

PKG="harmonigraph-plugin"
HERE="$(git rev-parse --show-toplevel)"
LOADER="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)/load-plugin.sh"
[ -x "$LOADER" ] || { echo "ERROR: $LOADER not found or not executable" >&2; exit 1; }

# Both packages, and the second is the easy half to skip. The offline renderer
# draws through harmonigraph-ui and harmonigraph-render — the same crates the
# editor does — so a change to what any pane looks like is a change to what an
# mp4 looks like, and a worktree that built only the plugin hands the loader
# whatever renderer it happens to be holding. `load-plugin.sh` warns when the
# renderer it installs predates HEAD; building it here is what keeps that
# warning quiet for the right reason.
echo "Building $PKG + harmonigraph-offline (release) from $HERE ..."
( cd "$HERE" && cargo build --release -p "$PKG" -p harmonigraph-offline )

# The branch this checkout stands on, which is how load-plugin.sh names a
# build. A detached HEAD has no branch to match, so say so here rather than
# letting the loader fail on a table lookup that cannot succeed.
BRANCH="$(git -C "$HERE" symbolic-ref --short -q HEAD || true)"
if [ -z "$BRANCH" ]; then
  echo "ERROR: $HERE is on a detached HEAD, which load-plugin.sh matches by" >&2
  echo "       branch name. Create a branch here in Codex or check one out, then" >&2
  echo "       rerun this command. To inspect existing named builds instead:" >&2
  echo "       $LOADER --list" >&2
  exit 1
fi

exec "$LOADER" "$BRANCH"
