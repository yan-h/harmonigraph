#!/usr/bin/env bash
#
# load-plugin.sh — pick an already-built worktree build and swap it into the
# ONE bundle slot Bitwig scans (the MAIN checkout's target/bundled/), then
# re-sign ad-hoc. This is the PULL side of the workflow: parallel sessions
# each build into their own worktree's target/release/ but do NOT touch the
# shared slot; you choose which of those builds is live.
#
# It COPIES ONLY — it never builds. A build has to exist in the worktree
# already (a session's `cargo build --release`, or `./update-plugin.sh`).
# Contrast update-plugin.sh, which BUILDS the checkout it runs from and swaps
# that one; use it when you want build-and-load in a single shot.
#
# Usage:
#   ./load-plugin.sh            # interactive menu of every worktree's build
#   ./load-plugin.sh --list     # print the table only; load nothing
#   ./load-plugin.sh <branch>   # load that branch's build (unique substring ok)
#
# Staleness is WARN-ONLY: a build whose dylib predates its branch HEAD is
# flagged "built before <hash>" but still loadable — the mtime check can't
# see uncommitted edits, so treat "fresh" as "matches the last commit", not
# "matches the working tree".
set -euo pipefail

PKG="harmonigraph-plugin"
NAME="Harmonigraph"
# Cargo names a LIB artifact after the lib target, which is the package name
# with dashes folded to underscores — so the dylib is libharmonigraph_plugin,
# not libharmonigraph-plugin. `cargo build -p` and `cargo xtask bundle` both
# want $PKG itself, so the two spellings have to be kept apart.
LIB="${PKG//-/_}"

# `substr($0, 10)` rather than `$2`: awk splits on whitespace, so a checkout
# under a path with a space in it ("~/My Drive/projects/...", which this repo
# has lived under) would come back truncated at the space — and a truncated
# MAIN makes BUNDLED a path that simply is not there, so the copy below goes
# quiet instead of failing. `.claude/reclaim-worktrees.sh` reads the same
# record the same way.
MAIN="$(git worktree list --porcelain | awk '/^worktree /{print substr($0, 10); exit}')"
BUNDLED="$MAIN/target/bundled"
LOADED="$BUNDLED/.loaded"   # records which worktree build is currently in the slot

# --- gather worktrees (parallel indexed arrays; no bash-4 assoc arrays) ------
WT_PATH=(); WT_BRANCH=()
while IFS= read -r line; do
  case "$line" in
    "worktree "*) WT_PATH+=("${line#worktree }") ;;
    "branch refs/heads/"*) WT_BRANCH+=("${line#branch refs/heads/}") ;;
    "detached") WT_BRANCH+=("(detached)") ;;
  esac
done < <(git worktree list --porcelain)

loaded_wt=""
[[ -f "$LOADED" ]] && loaded_wt="$(awk -F= '/^worktree=/{print $2}' "$LOADED")"

ago() {  # $1 = epoch seconds -> compact "3m ago"
  local now delta; now="$(date +%s)"; delta=$(( now - $1 ))
  if   (( delta < 60 ));    then echo "${delta}s ago"
  elif (( delta < 3600 ));  then echo "$(( delta / 60 ))m ago"
  elif (( delta < 86400 )); then echo "$(( delta / 3600 ))h ago"
  else                           echo "$(( delta / 86400 ))d ago"
  fi
}

# Echo the release dylib worktree $1 built, or nothing.
find_dylib() {
  local cand="$1/target/release/lib${LIB}.dylib"
  [[ -f "$cand" ]] && { echo "$cand"; return 0; }
  return 1
}

# Put $1 where $2 is, without ever rewriting $2's own bytes.
#
# A plain `cp` opens the destination with O_TRUNC and writes through the SAME
# inode, and a Bitwig plugin host demand-pages the plugin's code from exactly
# that inode for as long as the plugin is loaded. Overwriting it under a live
# host therefore leaves that host reading a file that is half one build and
# half another for every page it has not faulted in yet — and which pages
# those are is decided by whatever the kernel happened to evict, so the
# failure is undefined and arrives whenever the host next draws something new.
#
# A rename gives the incoming build an inode of its own and leaves the old one
# whole and unlinked underneath the running host, which keeps running the
# build it started with until Bitwig restarts the plugin. That is the
# behaviour the "Rescan/restart" line below already promises. The temp file
# is in the destination's own directory so the rename stays within one
# filesystem and is atomic.
swap_exe() {
  local src="$1" dst="$2" tmp="$2.incoming"
  cp "$src" "$tmp"
  mv -f "$tmp" "$dst"
}

# Echo the display fields for worktree index $1 as: built<TAB>vshead<TAB>marker
build_info() {
  local path="${WT_PATH[$1]}" dylib built vshead marker
  dylib="$(find_dylib "$path")" || dylib=""
  if [[ -n "$dylib" ]]; then
    local mtime head_ct head_short
    mtime="$(stat -f %m "$dylib")"
    built="$(ago "$mtime")"
    head_ct="$(git -C "$path" show -s --format=%ct HEAD 2>/dev/null || echo 0)"
    head_short="$(git -C "$path" show -s --format=%h HEAD 2>/dev/null || echo '?')"
    if (( head_ct > mtime )); then vshead="! built before $head_short"; else vshead="ok fresh"; fi
  else
    built="- not built -"; vshead=""
  fi
  marker=""; [[ -n "$loaded_wt" && "$path" == "$loaded_wt" ]] && marker="<- now"
  printf '%s\t%s\t%s' "$built" "$vshead" "$marker"
}

print_table() {
  printf '\n  %-3s %-30s %-13s %-24s %s\n' '#' 'branch' 'built' 'vs HEAD' 'loaded'
  printf '  %s\n' '---------------------------------------------------------------------------------'
  local i info built vshead marker
  for i in "${!WT_PATH[@]}"; do
    IFS=$'\t' read -r built vshead marker <<<"$(build_info "$i")"
    printf '  %-3s %-30s %-13s %-24s %s\n' "$((i + 1))" "${WT_BRANCH[$i]}" "$built" "$vshead" "$marker"
  done
  echo
}

# PIDs of anything that currently has a bundle's executable mapped — in
# practice one Bitwig plugin host. Read BEFORE the swap, because afterwards
# the host holds an inode with no name left to look it up by.
live_hosts() {
  lsof -t "$BUNDLED/$NAME.clap/Contents/MacOS/$NAME" \
          "$BUNDLED/$NAME.vst3/Contents/MacOS/$NAME" 2>/dev/null | sort -u | tr '\n' ' '
}

load_build() {  # $1 = worktree index
  local path="${WT_PATH[$1]}" branch="${WT_BRANCH[$1]}"
  local dylib
  dylib="$(find_dylib "$path")" || {
    echo "ERROR: no build in $path/target/release (looked for lib${LIB}.dylib)" >&2
    echo "       build it first:  (cd \"$path\" && cargo build --release -p $PKG)" >&2
    exit 1
  }
  local holders; holders="$(live_hosts || true)"
  local updated=0 ext bundle
  for ext in clap vst3; do
    bundle="$BUNDLED/$NAME.$ext"
    if [[ ! -d "$bundle" ]]; then
      echo "WARNING: $bundle missing — create the bundles once from main:" >&2
      echo "         (cd \"$MAIN\" && cargo xtask bundle $PKG --release)" >&2
      continue
    fi
    swap_exe "$dylib" "$bundle/Contents/MacOS/$NAME"
    codesign --force --sign - "$bundle"
    codesign --verify --verbose=1 "$bundle"
    echo "Loaded + signed: $bundle"
    updated=$(( updated + 1 ))
  done
  (( updated > 0 )) || { echo "No bundles updated — see warnings above." >&2; exit 1; }

  # Keep the renderer matched to the build being loaded (they share the take
  # format). Only if this worktree built one; otherwise leave the installed one.
  local offline="$path/target/release/harmonigraph-offline"
  local support="$HOME/Library/Application Support/$NAME"
  if [[ -f "$offline" ]]; then
    # Renamed into place for the same reason the bundles are: a render running
    # right now keeps the whole binary it started from.
    mkdir -p "$support"; swap_exe "$offline" "$support/harmonigraph-offline"
    echo "Loaded renderer: $support/harmonigraph-offline"
  else
    # Says "if any" because the support directory can hold no renderer at all:
    # the plugin resolves one fixed path under the product name, and nothing
    # puts a binary there until some worktree builds -p harmonigraph-offline
    # and a load copies it across.
    echo "NOTE: no harmonigraph-offline in this build; offline render keeps the previously-installed renderer, if any." >&2
  fi

  { echo "worktree=$path"
    echo "branch=$branch"
    echo "commit=$(git -C "$path" show -s --format=%h HEAD 2>/dev/null || echo '?')"
    echo "loaded_at=$(date +%s)"
  } > "$LOADED"

  echo
  echo "Now loaded: $branch. Rescan/restart the plugin in Bitwig to pick it up."
  # Naming the host makes the difference between "in the slot" and "in the DAW"
  # concrete: until it is restarted it is still drawing the build it opened
  # with, so the overlay tag will not be the one this swap just wrote.
  if [[ -n "$holders" ]]; then
    echo
    echo "NOTE: Bitwig has the previous build open right now (pid ${holders% })."
    echo "      It keeps drawing that build — and reporting its tag — until you do."
  fi
}

# --- dispatch ----------------------------------------------------------------
case "${1:-}" in
  --list|-l)
    print_table
    ;;
  "")
    print_table
    read -r -p "Pick # (empty to cancel): " choice
    [[ -z "$choice" ]] && { echo "Cancelled."; exit 0; }
    [[ "$choice" =~ ^[0-9]+$ ]] || { echo "Not a number: $choice" >&2; exit 1; }
    idx=$(( choice - 1 ))
    [[ -n "${WT_PATH[$idx]:-}" ]] || { echo "No build #$choice" >&2; exit 1; }
    load_build "$idx"
    ;;
  --*)
    echo "Unknown option: $1" >&2
    echo "Usage: ./load-plugin.sh [--list | <branch>]" >&2
    exit 1
    ;;
  *)
    # Match the argument as a substring of a branch name; require it to be unique.
    matches=()
    for i in "${!WT_BRANCH[@]}"; do
      [[ "${WT_BRANCH[$i]}" == *"$1"* ]] && matches+=("$i")
    done
    if (( ${#matches[@]} == 0 )); then
      echo "No worktree branch matching '$1'." >&2; print_table; exit 1
    elif (( ${#matches[@]} > 1 )); then
      echo "'$1' matches multiple branches:" >&2
      for i in "${matches[@]}"; do echo "  ${WT_BRANCH[$i]}" >&2; done
      exit 1
    fi
    load_build "${matches[0]}"
    ;;
esac
