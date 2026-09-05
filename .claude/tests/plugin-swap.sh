#!/usr/bin/env bash
# Does a swap into the shared bundle slot reach a host that already has the
# plugin open, including when the source build lives in a Codex-managed
# worktree outside the main checkout?
#
# The swap is the one sequence in the tree whose failure is completely silent.
# Bitwig's plugin host keeps the library it opened mapped for as long as its
# sandbox PROCESS lives, so the only swap such a host can see is one that
# changes the bytes behind the inode it is already holding:
#
#   - `cp` writes THROUGH the inode already at the path. Good.
#   - `codesign` does NOT. It writes a new file and renames it into place on
#     every run, whether or not the signature changes size — so signing the
#     LIVE bundle discards the inode a preceding `cp` just wrote.
#
# Get the order wrong and every step still succeeds: the copy reports success,
# `codesign --verify` passes, the bundle on disk is correct, and the DAW goes
# on drawing the previous build with nothing on screen saying so. No cargo test
# can reach this — the subject is an inode and an ad-hoc signature — which is
# why it is a gate rather than a habit, like the reclaim-lock cases beside it.
#
#   .claude/tests/plugin-swap.sh          # run it
#
# Written for bash 3.2 (macOS system bash), like the scripts it tests.
set -uo pipefail

# This test creates and enters a throwaway repository, so inherited GIT_DIR
# and related variables would point every git call below at the caller's repo.
# Same guard, same reason, as `.claude/tests/reclaim-locks.sh`.
for v in $(env | sed -n 's/^\(GIT_[A-Z_]*\)=.*/\1/p'); do
  unset "$v"
done

ROOT=$(cd "$(dirname "$0")/../.." && pwd)
for s in load-plugin.sh update-plugin.sh; do
  [ -x "$ROOT/$s" ] || { echo "✗ not executable: $ROOT/$s" >&2; exit 1; }
done

command -v codesign >/dev/null 2>&1 || {
  echo "- skipped: no codesign on this machine (the swap is macOS-only)"
  exit 0
}
# A real Mach-O is not a detail of the fixture. An ad-hoc signature lives
# INSIDE a Mach-O and beside a script, so a bundle whose executable is a shell
# script is sealed in a way the real one is not, and the copy that is correct
# for a plugin fails for it.
command -v cc >/dev/null 2>&1 || {
  echo "- skipped: no cc, so no Mach-O to sign (needs the Xcode command line tools)"
  exit 0
}

TMP=$(mktemp -d "${TMPDIR:-/tmp}/plugin-swap.XXXXXX") || exit 1
trap 'rm -rf "$TMP"' EXIT

NAME="Harmonigraph"
failures=0

# A throwaway checkout that looks enough like the real one for both scripts:
# a git repo with a branch and a commit (they read `git worktree list`, HEAD's
# sha and its date), the bundle slot with a signed bundle in it, and a build in
# target/release for the loader to find.
repo="$TMP/main"
mkdir -p "$repo"
git init -q "$repo"
git -C "$repo" checkout -q -b main 2>/dev/null || true
git -C "$repo" config user.email t@example.com
git -C "$repo" config user.name t
: > "$repo/seed"
git -C "$repo" add seed
git -C "$repo" commit -q -m seed
sha=$(git -C "$repo" rev-parse --short HEAD)

cp "$ROOT/load-plugin.sh" "$ROOT/update-plugin.sh" "$repo/"

# Codex-managed worktrees normally live under $CODEX_HOME rather than inside
# the checkout. The loader must discover this one through Git registration,
# retain its `codex/` branch name, and load it on the same terms as main.
codex_branch="codex/app-worktree"
codex_wt="$TMP/codex/worktrees/a1b2/harmonigraph"
mkdir -p "$(dirname "$codex_wt")"
git -C "$repo" worktree add -q -b "$codex_branch" "$codex_wt" HEAD 2>/dev/null || {
  echo "✗ could not create the Codex-managed worktree fixture" >&2; exit 1; }

# The artifacts a build would have left. The dylib carries the build tag the
# loader reads back out of it with `strings`, in the shape build.rs stamps it.
mkdir -p "$repo/target/release"
cat > "$TMP/new.c" <<C
const char *tag = "main @$sha";
int main(void) { return 0; }
C
cc -o "$repo/target/release/libharmonigraph_plugin.dylib" "$TMP/new.c" 2>/dev/null || {
  echo "✗ could not build the fixture's Mach-O" >&2; exit 1; }
printf '#!/bin/sh\ntrue\n' > "$repo/target/release/harmonigraph-offline"
chmod +x "$repo/target/release/harmonigraph-offline"

mkdir -p "$codex_wt/target/release"
cat > "$TMP/codex.c" <<C
const char *tag = "$codex_branch @$sha";
int main(void) { return 0; }
C
cc -o "$codex_wt/target/release/libharmonigraph_plugin.dylib" \
  "$TMP/codex.c" 2>/dev/null || {
  echo "✗ could not build the Codex worktree fixture's Mach-O" >&2; exit 1; }
printf '#!/bin/sh\ntrue\n' > "$codex_wt/target/release/harmonigraph-offline"
chmod +x "$codex_wt/target/release/harmonigraph-offline"

# A DIFFERENT binary in the slot to begin with, so "the new bytes arrived" is
# a real question rather than one the fixture answers by construction.
cat > "$TMP/old.c" <<C
const char *tag = "main @0000000";
int main(void) { return 0; }
C
cc -o "$TMP/old-bin" "$TMP/old.c" 2>/dev/null || {
  echo "✗ could not build the fixture's Mach-O" >&2; exit 1; }

# The slot, holding a signed bundle — which is the state that matters: the
# inode under test is the one a running host would already be mapped to.
for ext in clap vst3; do
  bundle="$repo/target/bundled/$NAME.$ext"
  mkdir -p "$bundle/Contents/MacOS"
  cp "$TMP/old-bin" "$bundle/Contents/MacOS/$NAME"
  cat > "$bundle/Contents/Info.plist" <<PLIST
<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0"><dict>
<key>CFBundleExecutable</key><string>$NAME</string>
<key>CFBundleIdentifier</key><string>test.$NAME</string>
</dict></plist>
PLIST
  codesign --force --sign - "$bundle" >/dev/null 2>&1
done

# Bitwig's class-discovery cache fingerprints Info.plist's mtime and size,
# not the executable (#631). Keep the signed bytes for comparison and give
# each install an old fingerprint: otherwise a test can pass just because
# bundle setup happened in a different clock tick.
cp "$repo/target/bundled/$NAME.clap/Contents/Info.plist" "$TMP/original.plist"
prime_discovery_metadata() {
  for ext in clap vst3; do
    touch -m -t 200101010000 "$repo/target/bundled/$NAME.$ext/Contents/Info.plist"
  done
}
check_discovery_metadata() {
  local caller="$1" ext bundle plist
  for ext in clap vst3; do
    bundle="$repo/target/bundled/$NAME.$ext"
    plist="$bundle/Contents/Info.plist"
    if [ "$(stat -f %m "$plist")" -le "$metadata_before" ]; then
      echo "✗ $caller did not refresh $ext discovery metadata" >&2
      failures=$((failures + 1))
    fi
    if ! cmp -s "$TMP/original.plist" "$plist"; then
      echo "✗ $caller changed $ext signed plist contents" >&2
      failures=$((failures + 1))
    fi
    if ! codesign --verify --verbose=1 "$bundle" >/dev/null 2>&1; then
      echo "✗ $caller left an invalid $ext signature after metadata refresh" >&2
      failures=$((failures + 1))
    fi
  done
}

# A `cargo` that builds nothing: the artifacts are already staged above, and
# what is under test is the swap rather than the compile.
mkdir -p "$TMP/bin"
printf '#!/bin/sh\nexit 0\n' > "$TMP/bin/cargo"
chmod +x "$TMP/bin/cargo"

# Model a timestamp tick collision deterministically: the first metadata touch
# per bundle leaves mtime unchanged, just as a repeat install within the same
# clock tick can. Subsequent calls use the real touch. The direct-loader case
# must reach the retry and still refresh both fingerprints; no timing race in
# fixture compilation/signing decides whether that branch gets exercised.
real_touch=$(command -v touch)
cat > "$TMP/bin/touch" <<'SH'
#!/usr/bin/env bash
if [[ -n "${PLUGIN_SWAP_COLLISION_DIR:-}" && "$1" == -m ]]; then
  marker="$PLUGIN_SWAP_COLLISION_DIR/$(basename "$(dirname "$(dirname "$2")")")"
  if [[ ! -f "$marker" ]]; then
    : > "$marker"
    exit 0
  fi
fi
exec "$PLUGIN_SWAP_REAL_TOUCH" "$@"
SH
chmod +x "$TMP/bin/touch"
export PLUGIN_SWAP_REAL_TOUCH="$real_touch"

# The inodes a host would be holding, read before the swap.
before_clap=$(stat -f %i "$repo/target/bundled/$NAME.clap/Contents/MacOS/$NAME")
before_vst3=$(stat -f %i "$repo/target/bundled/$NAME.vst3/Contents/MacOS/$NAME")

# HOME redirected: the scripts install the offline renderer under
# ~/Library/Application Support, and a test must not write to the real one.
out="$TMP/run.log"
prime_discovery_metadata
metadata_before=$(stat -f %m "$repo/target/bundled/$NAME.clap/Contents/Info.plist")
( cd "$repo" && PATH="$TMP/bin:$PATH" HOME="$TMP/home" ./update-plugin.sh ) \
  > "$out" 2>&1
status=$?

if [ "$status" -ne 0 ]; then
  echo "✗ update-plugin.sh exited $status" >&2
  sed 's/^/    /' "$out" >&2
  failures=$((failures + 1))
fi
check_discovery_metadata update-plugin.sh

after_clap=$(stat -f %i "$repo/target/bundled/$NAME.clap/Contents/MacOS/$NAME")
after_vst3=$(stat -f %i "$repo/target/bundled/$NAME.vst3/Contents/MacOS/$NAME")

# 1. The inode survives — the whole point. A host mapped to the old one keeps
#    serving the old build, and every other check here still passes.
if [ "$before_clap" != "$after_clap" ]; then
  echo "✗ the .clap executable changed inode ($before_clap -> $after_clap):" >&2
  echo "    a running host stays on the old build until Bitwig restarts" >&2
  failures=$((failures + 1))
fi
if [ "$before_vst3" != "$after_vst3" ]; then
  echo "✗ the .vst3 executable changed inode ($before_vst3 -> $after_vst3)" >&2
  failures=$((failures + 1))
fi

# 2. The new bytes actually arrived. Writing through the inode is only right if
#    it is the BUILD that is written; a swap that preserves the inode by not
#    copying at all would pass the check above.
if ! grep -q "main @$sha" "$repo/target/bundled/$NAME.clap/Contents/MacOS/$NAME"; then
  echo "✗ the .clap executable does not carry the build that was loaded" >&2
  failures=$((failures + 1))
fi

# 3. And the bundle is still valid: an ad-hoc signature that no longer matches
#    the bytes makes the dynamic loader refuse the binary on Apple Silicon,
#    which is a different silent failure with the same symptom.
for ext in clap vst3; do
  if ! codesign --verify --verbose=1 "$repo/target/bundled/$NAME.$ext" >/dev/null 2>&1; then
    echo "✗ $NAME.$ext does not verify after the swap" >&2
    failures=$((failures + 1))
  fi
done

# 4. The slot records which build is in it, tag included. `commit` is where the
#    worktree stands and `tag` is what the binary says about itself; they
#    disagree whenever a session committed after building, and only `tag`
#    answers "which build am I looking at?".
loaded="$repo/target/bundled/.loaded"
if [ ! -f "$loaded" ]; then
  echo "✗ no .loaded written, so nothing records what is in the slot" >&2
  failures=$((failures + 1))
elif ! grep -q "^tag=main @$sha" "$loaded"; then
  echo "✗ .loaded does not name the build's own tag:" >&2
  sed 's/^/    /' "$loaded" >&2
  failures=$((failures + 1))
fi

# 5. A registered Codex-managed worktree outside main is visible by branch,
# reports its own overlay tag, and can be selected for the same swap. This is
# the path contract; matching only `.claude/worktrees` would make the task's
# finished build invisible even though Git knows exactly where it is.
codex_tag=$(cd "$repo" && ./load-plugin.sh --tag "$codex_branch" 2>/dev/null)
if [ "$codex_tag" != "$codex_branch @$sha" ]; then
  echo "✗ Codex worktree tag was '${codex_tag:-missing}', expected $codex_branch @$sha" >&2
  failures=$((failures + 1))
fi

codex_list=$(cd "$repo" && ./load-plugin.sh --list 2>/dev/null)
if ! printf '%s\n' "$codex_list" | grep -Fq "$codex_branch"; then
  echo "✗ --list does not include the Codex-managed worktree" >&2
  failures=$((failures + 1))
fi

codex_out="$TMP/codex-load.log"
prime_discovery_metadata
metadata_before=$(stat -f %m "$repo/target/bundled/$NAME.clap/Contents/Info.plist")
mkdir -p "$TMP/collisions"
(cd "$repo" && PATH="$TMP/bin:$PATH" HOME="$TMP/home" \
  PLUGIN_SWAP_COLLISION_DIR="$TMP/collisions" ./load-plugin.sh "$codex_branch") \
  >"$codex_out" 2>&1
codex_status=$?
if [ "$codex_status" -ne 0 ]; then
  echo "✗ loading the Codex-managed worktree exited $codex_status" >&2
  sed 's/^/    /' "$codex_out" >&2
  failures=$((failures + 1))
elif ! grep -Fq "$codex_branch @$sha" \
  "$repo/target/bundled/$NAME.clap/Contents/MacOS/$NAME"; then
  echo "✗ the Codex-managed worktree's bytes did not reach the bundle" >&2
  failures=$((failures + 1))
elif ! grep -Fq "branch=$codex_branch" "$loaded" \
  || ! grep -Fq "tag=$codex_branch @$sha" "$loaded"; then
  echo "✗ .loaded does not identify the Codex-managed build:" >&2
  sed 's/^/    /' "$loaded" >&2
  failures=$((failures + 1))
fi
check_discovery_metadata load-plugin.sh
for ext in clap vst3; do
  if [ ! -f "$TMP/collisions/$NAME.$ext" ]; then
    echo "✗ the $ext timestamp collision fixture was not reached" >&2
    failures=$((failures + 1))
  fi
done
if [ "$before_clap" != "$(stat -f %i "$repo/target/bundled/$NAME.clap/Contents/MacOS/$NAME")" ] \
  || [ "$before_vst3" != "$(stat -f %i "$repo/target/bundled/$NAME.vst3/Contents/MacOS/$NAME")" ]; then
  echo "✗ load-plugin.sh changed an executable inode" >&2
  failures=$((failures + 1))
fi

if [ "$failures" -eq 0 ]; then
  echo "  ok — registered worktrees reach a host that already has the plugin open"
else
  echo "✗ $failures plugin-swap check(s) failed" >&2
  exit 1
fi
