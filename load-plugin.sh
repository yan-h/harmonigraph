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
# "matches the working tree". The renderer it installs alongside is held to
# the same test, and is the half that actually goes stale: nothing rebuilds
# it unless a session names `-p harmonigraph-offline`.
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

load_build() {  # $1 = worktree index
  local path="${WT_PATH[$1]}" branch="${WT_BRANCH[$1]}"
  local dylib
  dylib="$(find_dylib "$path")" || {
    echo "ERROR: no build in $path/target/release (looked for lib${LIB}.dylib)" >&2
    echo "       build it first:  (cd \"$path\" && cargo build --release -p $PKG)" >&2
    exit 1
  }
  local updated=0 ext bundle
  for ext in clap vst3; do
    bundle="$BUNDLED/$NAME.$ext"
    if [[ ! -d "$bundle" ]]; then
      echo "WARNING: $bundle missing — create the bundles once from main:" >&2
      echo "         (cd \"$MAIN\" && cargo xtask bundle $PKG --release)" >&2
      continue
    fi
    # Written through the destination's OWN inode, deliberately — `cp` here is
    # load-bearing and a rename-into-place is not the tidier equivalent. Bitwig's
    # plugin host keeps the library it opened mapped across a deactivate/activate,
    # so the only swap a running host can see is one that changes the bytes behind
    # the inode it already holds; hand it a fresh inode at the same path and it
    # stays on the old build until Bitwig itself is restarted. The cost is that a
    # swap under a live host rewrites code that host has not faulted in yet, so do
    # it while the plugin is deactivated if a session is worth protecting.
    cp "$dylib" "$bundle/Contents/MacOS/$NAME"
    codesign --force --sign - "$bundle"
    codesign --verify --verbose=1 "$bundle"
    echo "Loaded + signed: $bundle"
    updated=$(( updated + 1 ))
  done
  (( updated > 0 )) || { echo "No bundles updated — see warnings above." >&2; exit 1; }

  # Keep the renderer matched to the build being loaded (they share the take
  # format). Only if this worktree built one; otherwise leave the installed one.
  #
  # Matched is the INTENT and not something the copy can guarantee, which is
  # what the freshness check below is for. The renderer draws the picture through
  # harmonigraph-ui and harmonigraph-render, exactly as the plugin does, so
  # every change to what the panes look like changes an export too — but a
  # session builds `-p harmonigraph-plugin` and nothing rebuilds the renderer
  # unless it asked for `-p harmonigraph-offline` by name. A worktree that
  # built only the plugin therefore hands over whatever renderer was last left
  # in its target/release, which can be from any commit at all, and the
  # divergence shows up nowhere until an export comes out drawn the old way.
  local offline="$path/target/release/harmonigraph-offline"
  local support="$HOME/Library/Application Support/$NAME"
  if [[ -f "$offline" ]]; then
    mkdir -p "$support"; cp "$offline" "$support/harmonigraph-offline"
    echo "Loaded renderer: $support/harmonigraph-offline"
    # The freshness test the table already applies to the dylib, applied to the
    # renderer, and read the same way: "matches the last commit", not "matches
    # the working tree". Against HEAD rather than against the dylib's own mtime,
    # which is the tempting comparison and the wrong one — two artifacts built
    # from one source state can be minutes apart (separate `cargo build` runs, a
    # plugin relinked while the renderer was left alone), so a gap between them
    # flags matched pairs as often as mismatched ones.
    local offline_mtime head_ct head_short
    offline_mtime="$(stat -f %m "$offline")"
    head_ct="$(git -C "$path" show -s --format=%ct HEAD 2>/dev/null || echo 0)"
    head_short="$(git -C "$path" show -s --format=%h HEAD 2>/dev/null || echo '?')"
    if (( head_ct > offline_mtime )); then
      echo "WARNING: that renderer was built before $head_short ($(ago "$offline_mtime")). Video exports" >&2
      echo "         come out drawn by the older build while the editor shows the new one, and" >&2
      echo "         nothing on screen says so. To match them:" >&2
      echo "         (cd \"$path\" && cargo build --release -p harmonigraph-offline) && ./load-plugin.sh $branch" >&2
    fi
  else
    # Says "if any" because the support directory can hold no renderer at all:
    # the plugin resolves one fixed path under the product name, and nothing
    # puts a binary there until some worktree builds -p harmonigraph-offline
    # and a load copies it across.
    echo "NOTE: no harmonigraph-offline in this build; offline render keeps the previously-installed renderer, if any." >&2
    if [[ -f "$support/harmonigraph-offline" ]]; then
      local kept; kept="$(ago "$(stat -f %m "$support/harmonigraph-offline")")"
      echo "      The one it keeps is $kept and knows nothing this build changed." >&2
    fi
  fi

  { echo "worktree=$path"
    echo "branch=$branch"
    echo "commit=$(git -C "$path" show -s --format=%h HEAD 2>/dev/null || echo '?')"
    echo "loaded_at=$(date +%s)"
  } > "$LOADED"

  echo
  echo "Now loaded: $branch. Deactivate + reactivate the plugin in Bitwig to pick it up."
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
