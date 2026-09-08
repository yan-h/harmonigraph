#!/usr/bin/env bash
# Install distinct signed builds from main and an external registered worktree.
# Every executable replacement must use a fresh inode, preserve an old open
# descriptor's bytes, and load the new dylib in a fresh process. On-disk signature
# verification alone did not catch Bitwig's CODESIGNING Invalid Page scans (#705).
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
int fixture_value(void) { return 0; }
C
cc -dynamiclib -o "$repo/target/release/libharmonigraph_plugin.dylib" "$TMP/new.c" 2>/dev/null || {
  echo "✗ could not build the fixture's Mach-O" >&2; exit 1; }
printf '#!/bin/sh\ntrue\n' > "$repo/target/release/harmonigraph-offline"
chmod +x "$repo/target/release/harmonigraph-offline"

mkdir -p "$codex_wt/target/release"
cat > "$TMP/codex.c" <<C
const char *tag = "$codex_branch @$sha";
int fixture_value(void) { return 0; }
C
cc -dynamiclib -o "$codex_wt/target/release/libharmonigraph_plugin.dylib" \
  "$TMP/codex.c" 2>/dev/null || {
  echo "✗ could not build the Codex worktree fixture's Mach-O" >&2; exit 1; }
printf '#!/bin/sh\ntrue\n' > "$codex_wt/target/release/harmonigraph-offline"
chmod +x "$codex_wt/target/release/harmonigraph-offline"

# A DIFFERENT binary in the slot to begin with, so "the new bytes arrived" is
# a real question rather than one the fixture answers by construction.
cat > "$TMP/old.c" <<C
const char *tag = "main @0000000";
int fixture_value(void) { return 0; }
C
cc -dynamiclib -o "$TMP/old-bin" "$TMP/old.c" 2>/dev/null || {
  echo "✗ could not build the fixture's Mach-O" >&2; exit 1; }

# A fresh process goes through the dynamic loader and reads the loaded tag.
cat > "$TMP/dlopen.c" <<'C'
#include <dlfcn.h>
#include <stdio.h>
int main(int argc, char **argv) {
    if (argc != 2) return 1;
    void *handle = dlopen(argv[1], RTLD_NOW | RTLD_LOCAL);
    if (!handle) { fprintf(stderr, "%s\n", dlerror()); return 1; }
    const char **tag = dlsym(handle, "tag");
    if (!tag) { fprintf(stderr, "%s\n", dlerror()); return 1; }
    puts(*tag);
    return dlclose(handle) != 0;
}
C
cc -o "$TMP/dlopen" "$TMP/dlopen.c" || exit 1
check_loads() {
  local caller="$1" expected="$2" ext actual
  for ext in clap vst3; do
    actual=$("$TMP/dlopen" "$repo/target/bundled/$NAME.$ext/Contents/MacOS/$NAME")
    if [ "$?" -ne 0 ] || [ "$actual" != "$expected" ]; then
      echo "✗ $caller $ext dlopen returned '${actual:-no tag}', expected $expected" >&2
      failures=$((failures + 1))
    fi
  done
}

# The initial signed slot is loaded before replacement, exercising a previously
# used executable rather than a never-opened file that cannot carry old state.
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

check_loads initial "main @0000000"
exec 3< "$repo/target/bundled/$NAME.clap/Contents/MacOS/$NAME"
cp "$repo/target/bundled/$NAME.clap/Contents/MacOS/$NAME" "$TMP/old-open-bin"

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

# Deterministic process snapshots: never inspect/control the real Bitwig hosts.
# lsof field records match macOS 4.91 output observed from a scratch dlopen
# process across an atomic rename: same n pathname, different D/i identity.
export PLUGIN_SWAP_REPO="$(cd "$repo" && pwd -P)" PLUGIN_SWAP_QUERY_LOG="$TMP/queries"
cat > "$TMP/bin/ps" <<'SH'
#!/bin/bash
echo ps >> "$PLUGIN_SWAP_QUERY_LOG"
case "${PLUGIN_SWAP_DIAGNOSTIC_CASE:-closed}" in
  closed) exit 0 ;;
  ps-failed) exit 1 ;;
esac
for pid in 7101 7102 7103 7104 7105 7106; do
  start="Mon Sep  7 10:40:18 2026"
  if [[ "$1" == -p && "$pid" == 7104 ]]; then
    start="Mon Sep  7 10:41:19 2026"  # PID reused while lsof ran.
  fi
  name=BitwigPluginHost-ARM64-NEON
  [[ "$pid" == 7102 ]] && name=BitwigAudioEngine-ARM64-NEON
  printf '%s %s /Applications/Bitwig Studio.app/Contents/MacOS/%s\n' "$pid" "$start" "$name"
done
echo '7199 Mon Sep  7 10:40:18 2026 /tmp/NotBitwigPluginHost'
SH
cat > "$TMP/bin/lsof" <<'SH'
#!/bin/bash
echo lsof >> "$PLUGIN_SWAP_QUERY_LOG"
# Assert one batch covers only the selected hosts and mapped text. A filename
# selection would hide precisely the old inode this regression needs to reach.
[[ "$*" == '-nP -b -a -p 7101,7102,7103,7104,7105,7106 -d txt -FpfDin' ]] || exit 2
[[ "$PLUGIN_SWAP_DIAGNOSTIC_CASE" == unavailable ]] && exit 127
record() {
  printf 'ftxt\nD%s\ni%s\nn%s\n' "${2%:*}" "${2#*:}" "$1"
}
for pid in 7101 7102 7103 7104 7105 7106; do
  [[ "$pid" == 7105 && "$PLUGIN_SWAP_DIAGNOSTIC_CASE" != no-match ]] && continue
  echo "p$pid"
  record /usr/lib/dyld 0x100000d:1152921500312573255
  [[ "$PLUGIN_SWAP_DIAGNOSTIC_CASE" == no-match ]] && continue
  for ext in clap vst3; do
    live="$PLUGIN_SWAP_REPO/target/bundled/Harmonigraph.$ext/Contents/MacOS/Harmonigraph"
    old=$(cat "$PLUGIN_SWAP_REPO/old-$ext")
    current=$(stat -f '0x%Xd:%i' "$live")
    case "$pid" in
      7101) record "$live" "$old"; record "$live" "$old" ;; # deduplicated
      7102) record "$live" "$current" ;;
      7103)
        if [[ "$ext" == clap ]]; then
          printf 'ftxt\nn%s\n' "$live"  # no identity: pathname is insufficient
        else
          record "$live" "0x999:${current#*:}"  # same inode, different device
        fi ;;
      7104) record "$live" "$old" ;;
    esac
  done
done
[[ "$PLUGIN_SWAP_DIAGNOSTIC_CASE" == failed ]] && exit 1
exit 0
SH
chmod +x "$TMP/bin/ps" "$TMP/bin/lsof"

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

# 1. Both installed paths move to fresh inodes; an old open descriptor keeps
#    the old bytes intact instead of seeing a different executable underneath.
if [ "$before_clap" = "$after_clap" ] || [ "$before_vst3" = "$after_vst3" ]; then
  echo "✗ update-plugin.sh reused an executable inode" >&2
  failures=$((failures + 1))
fi
cat <&3 > "$TMP/old-open-after"
exec 3<&-
if ! cmp -s "$TMP/old-open-bin" "$TMP/old-open-after"; then
  echo "✗ update-plugin.sh changed the bytes behind an old open descriptor" >&2
  failures=$((failures + 1))
fi
check_loads update-plugin.sh "main @$sha"

# 2. The selected build's bytes actually arrived.
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

before_clap=$(stat -f %i "$repo/target/bundled/$NAME.clap/Contents/MacOS/$NAME")
before_vst3=$(stat -f %i "$repo/target/bundled/$NAME.vst3/Contents/MacOS/$NAME")
exec 3< "$repo/target/bundled/$NAME.clap/Contents/MacOS/$NAME"
cp "$repo/target/bundled/$NAME.clap/Contents/MacOS/$NAME" "$TMP/old-open-bin"
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
if [ "$before_clap" = "$(stat -f %i "$repo/target/bundled/$NAME.clap/Contents/MacOS/$NAME")" ] \
  || [ "$before_vst3" = "$(stat -f %i "$repo/target/bundled/$NAME.vst3/Contents/MacOS/$NAME")" ]; then
  echo "✗ load-plugin.sh reused an executable inode" >&2
  failures=$((failures + 1))
fi

cat <&3 > "$TMP/old-open-after"
exec 3<&-
if ! cmp -s "$TMP/old-open-bin" "$TMP/old-open-after"; then
  echo "✗ load-plugin.sh changed the bytes behind an old open descriptor" >&2
  failures=$((failures + 1))
fi
check_loads load-plugin.sh "$codex_branch @$sha"

# 6. Diagnostics use executable identity, tolerate incomplete OS queries, and
# never turn a successful swap into failure. The first two swaps above model
# Bitwig closed; even lsof must stay uncalled in that case.
if grep -Eq 'PID |Snapshot only|process diagnostic|Process mapping query' "$out" "$codex_out" \
  || grep -q lsof "$PLUGIN_SWAP_QUERY_LOG"; then
  echo "✗ closed Bitwig produced extra diagnostics or an lsof query" >&2
  failures=$((failures + 1))
fi

diagnostic_check() {
  local pattern="$1" expected="$2" actual
  actual=$(grep -Ec "$pattern" "$diagnostic_out")
  if [[ "$actual" != "$expected" ]]; then
    echo "✗ $diagnostic_case: expected $expected matches for $pattern, got $actual" >&2
    sed 's/^/    /' "$diagnostic_out" >&2
    failures=$((failures + 1))
  fi
}
for diagnostic_case in identities no-match failed unavailable ps-failed; do
  # Keep both old inodes alive as a mapped host would, preventing inode reuse
  # from making the fixture claim a different file is the replaced image.
  exec 3< "$repo/target/bundled/$NAME.clap/Contents/MacOS/$NAME"
  exec 4< "$repo/target/bundled/$NAME.vst3/Contents/MacOS/$NAME"
  for ext in clap vst3; do
    stat -f '0x%Xd:%i' "$repo/target/bundled/$NAME.$ext/Contents/MacOS/$NAME" > "$repo/old-$ext"
  done
  : > "$PLUGIN_SWAP_QUERY_LOG"
  diagnostic_out="$TMP/diagnostic-$diagnostic_case.log"
  prime_discovery_metadata
  (cd "$repo" && PATH="$TMP/bin:$PATH" HOME="$TMP/home" \
    PLUGIN_SWAP_DIAGNOSTIC_CASE="$diagnostic_case" /bin/bash ./load-plugin.sh "$codex_branch") \
    > "$diagnostic_out" 2>&1
  if [[ "$?" != 0 ]]; then
    echo "✗ $diagnostic_case: process diagnostic failed the install" >&2
    sed 's/^/    /' "$diagnostic_out" >&2
    failures=$((failures + 1))
  fi
  diagnostic_check 'Installed: codex/app-worktree' 1
  diagnostic_check "The performance overlay will read:  build  $codex_branch @$sha" 1
  if [[ "$diagnostic_case" == ps-failed ]]; then
    diagnostic_check 'NOTE: Bitwig process diagnostic unavailable; installation succeeded' 1
    expected_queries=ps
  else
    expected_queries=$'ps\nlsof\nps'
    diagnostic_check 'Snapshot only: unseen mappings and the next load are not verified' 1
    case "$diagnostic_case" in
      identities|failed)
        diagnostic_check 'PID 7101 \(started Mon Sep 7 10:40:18 2026\): observed old image' 2
        diagnostic_check 'PID 7102 .*observed current image' 2
        diagnostic_check 'PID 7103 .*uncertain.*pathname matches' 2
        diagnostic_check 'PID 7104 \(start unavailable or changed\): uncertain' 2
        diagnostic_check 'PID 710[12].*uncertain|PID 710[345].*observed (old|current)' 0
        ;;
      no-match)
        diagnostic_check 'observed old image|observed current image' 0
        diagnostic_check 'No Harmonigraph image observed in the queried Bitwig hosts' 1
        ;;
      unavailable)
        diagnostic_check 'PID 710[1-6].*uncertain' 6
        ;;
    esac
    diagnostic_check 'PID 7105 .*uncertain.*inaccessible or have exited' \
      "$([[ "$diagnostic_case" == no-match ]] && echo 0 || echo 1)"
    diagnostic_check 'PID 7106|PID 7199' "$([[ "$diagnostic_case" == unavailable ]] && echo 1 || echo 0)"
    diagnostic_check 'Process mapping query unavailable or incomplete; installation succeeded' \
      "$([[ "$diagnostic_case" == failed || "$diagnostic_case" == unavailable ]] && echo 1 || echo 0)"
  fi
  if [[ "$(cat "$PLUGIN_SWAP_QUERY_LOG")" != "$expected_queries" ]]; then
    echo "✗ $diagnostic_case did not use the expected bounded process queries" >&2
    failures=$((failures + 1))
  fi
  exec 3<&- 4<&-
done

if [ "$failures" -eq 0 ]; then
  echo "  ok — fresh signed inodes preserve old files; host diagnostics distinguish identities and tolerate query failures"
else
  echo "✗ $failures plugin-swap check(s) failed" >&2
  exit 1
fi
