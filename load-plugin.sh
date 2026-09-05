#!/usr/bin/env bash
#
# load-plugin.sh — pick an already-built registered worktree build and swap it
# into the ONE bundle slot Bitwig scans (the MAIN checkout's target/bundled/),
# then re-sign ad-hoc. This is the PULL side of the workflow: parallel sessions
# each build into their own worktree's target/release/ but do NOT touch the
# shared slot; you choose which of those builds is live. Git registration, not
# a parent-directory convention, is what makes both Claude- and Codex-managed
# worktrees discoverable here.
#
# It COPIES ONLY — it never builds. A build has to exist in the worktree
# already (a session's `cargo build --release`, or `./update-plugin.sh`).
# Contrast update-plugin.sh, which BUILDS the checkout it runs from and swaps
# that one; use it when you want build-and-load in a single shot.
#
# Usage:
#   ./load-plugin.sh            # interactive menu of every worktree's build
#   ./load-plugin.sh --list     # print the table only; load nothing
#   ./load-plugin.sh --tag      # print the overlay tag of THIS worktree's build
#   ./load-plugin.sh <branch>   # load that branch's build (unique substring ok)
#
# Staleness is WARN-ONLY: a build not made at its branch HEAD is flagged but
# still loadable. The test is the commit STAMPED IN THE BINARY against HEAD,
# so "fresh" means "matches the last commit" and never "matches the working
# tree" — a build carrying uncommitted edits reads as fresh, because the tag
# names the commit underneath them. The renderer it installs alongside has no
# such stamp and is judged on its mtime, and it is the half that actually goes
# stale: nothing rebuilds it unless a session names `-p harmonigraph-offline`.
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

# Scratch bundle that load_build signs before writing the result across. Held at
# script scope rather than in the function, so the trap that removes it is still
# looking at a variable that exists once the function has returned.
STAGE=""
cleanup() { [[ -n "$STAGE" ]] && rm -rf "$STAGE"; return 0; }
trap cleanup EXIT

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

# Echo the build tag stamped into dylib $1, whose worktree is on branch $2.
#
# Read out of the BINARY rather than predicted from a log, because the two
# disagree routinely and only this one is what the overlay will show.
# `harmonigraph-perf`'s build.rs stamps the commit the build sat on, and the
# ordinary session order is edit -> build -> commit -> hand over: the commit
# lands AFTER the build it is meant to describe, so a tag quoted from the log
# names a commit the binary has never heard of. An amend or a rebase breaks
# the prediction the other way, leaving a stamped sha that is no longer an
# object in the worktree at all.
build_tag() {
  local name="${2#worktree-}"
  strings -a "$1" 2>/dev/null | grep -oE "$name @[0-9a-f]{7,40}" | head -1
}

# Echo the display fields for worktree index $1 as:
#   built<TAB>overlay<TAB>vshead<TAB>marker
build_info() {
  local path="${WT_PATH[$1]}" dylib built overlay vshead marker
  dylib="$(find_dylib "$path")" || dylib=""
  if [[ -n "$dylib" ]]; then
    local mtime head_ct head_short tag sha
    mtime="$(stat -f %m "$dylib")"
    built="$(ago "$mtime")"
    head_ct="$(git -C "$path" show -s --format=%ct HEAD 2>/dev/null || echo 0)"
    head_short="$(git -C "$path" show -s --format=%h HEAD 2>/dev/null || echo '?')"
    tag="$(build_tag "$dylib" "${WT_BRANCH[$1]}")"
    sha="${tag##*@}"
    [[ "$sha" == "$tag" ]] && sha=""   # no tag read; ${..##*@} echoes the input
    overlay="${sha:+@$sha}"; overlay="${overlay:-@?}"
    # Judged on the stamped sha ahead of the mtime: it names the commit the
    # binary was actually built at, where an mtime only says which of the two
    # came first. An amend or a rebase moves HEAD without moving the clock, and
    # reads as fresh on time alone. The mtime stays as the fallback for a build
    # carrying no readable tag (a source tarball, or no git when it compiled).
    if [[ -n "$sha" && "$head_short" != '?' ]]; then
      if [[ "$head_short" == "$sha"* || "$sha" == "$head_short"* ]]; then
        vshead="ok fresh"
      elif git -C "$path" cat-file -e "$sha^{commit}" 2>/dev/null; then
        vshead="! built at $sha, HEAD $head_short"
      else
        # The stamped commit is not an object here at all: the branch was
        # amended, rebased or reset out from under the build.
        vshead="! built at $sha, gone from branch"
      fi
    elif (( head_ct > mtime )); then
      vshead="! built before $head_short"
    else
      vshead="ok fresh"
    fi
  else
    built="- not built -"; overlay=""; vshead=""
  fi
  marker=""; [[ -n "$loaded_wt" && "$path" == "$loaded_wt" ]] && marker="<- now"
  printf '%s\t%s\t%s\t%s' "$built" "$overlay" "$vshead" "$marker"
}

print_table() {
  printf '\n  %-3s %-30s %-13s %-10s %-30s %s\n' '#' 'branch' 'built' 'overlay' 'vs HEAD' 'loaded'
  local rule; rule="$(printf '%*s' 101 '')"; printf '  %s\n' "${rule// /-}"
  local i built overlay vshead marker
  for i in "${!WT_PATH[@]}"; do
    IFS=$'\t' read -r built overlay vshead marker <<<"$(build_info "$i")"
    printf '  %-3s %-30s %-13s %-10s %-30s %s\n' \
      "$((i + 1))" "${WT_BRANCH[$i]}" "$built" "$overlay" "$vshead" "$marker"
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
  # Somewhere for the signature to be produced that the host is not looking at.
  STAGE="$(mktemp -d)"

  local updated=0 ext bundle staged live_bin ino_before ino_after
  for ext in clap vst3; do
    bundle="$BUNDLED/$NAME.$ext"
    if [[ ! -d "$bundle" ]]; then
      echo "WARNING: $bundle missing — create the bundles once from main:" >&2
      echo "         (cd \"$MAIN\" && cargo xtask bundle $PKG --release)" >&2
      continue
    fi
    # Sign a staging copy FIRST, then write the finished bytes through the
    # destination's own inode. The order is load-bearing in both directions, and
    # neither half is the tidier equivalent of the other:
    #
    #   - `cp` writes through the inode already at the path. Bitwig's plugin host
    #     keeps the library it opened mapped for as long as its sandbox PROCESS
    #     lives, so the only swap such a host can see is one that changes the
    #     bytes behind the inode it is already holding. Give it a fresh inode at
    #     the same path and it stays on the old build until that process dies.
    #   - `codesign` does NOT write through an inode. It writes a new file and
    #     renames it into place on every run, whether or not the signature
    #     changes size. Signing the LIVE bundle therefore discards the very inode
    #     a preceding `cp` just wrote, and leaves a running host mapped to an
    #     unlinked file — the silent way this swap stops working, with a bundle
    #     that still verifies and a DAW that still shows the previous build.
    staged="$STAGE/$NAME.$ext"
    cp -R "$bundle" "$staged"
    cp "$dylib" "$staged/Contents/MacOS/$NAME"
    codesign --force --sign - "$staged"
    codesign --verify --verbose=1 "$staged"

    live_bin="$bundle/Contents/MacOS/$NAME"
    ino_before="$(stat -f %i "$live_bin")"
    cp "$staged/Contents/MacOS/$NAME" "$live_bin"
    # The seal names the bundle's other files; the executable seals itself, so
    # this moves only when a resource does. Copied unconditionally rather than
    # reasoned about per change — it is one small file.
    cp "$staged/Contents/_CodeSignature/CodeResources" \
       "$bundle/Contents/_CodeSignature/CodeResources"
    ino_after="$(stat -f %i "$live_bin")"

    # A tripwire on the paragraph above, because its failure is silent: every
    # step here still succeeds when the inode moves, and only the DAW knows.
    if [[ "$ino_before" != "$ino_after" ]]; then
      echo "WARNING: $live_bin changed inode ($ino_before -> $ino_after)." >&2
      echo "         A host holding the old one keeps serving the old build until its" >&2
      echo "         sandbox process exits; restart Bitwig to be sure of this build." >&2
    fi
    # Bitwig caches the bundle's class list by Info.plist's mtime and size,
    # not by the executable (#631). Refresh that fingerprint on every install,
    # including a rollback to a build with fewer classes. Only the timestamp
    # moves: codesign seals the plist's bytes, so its signature stays valid.
    # Cross a whole-second boundary even on a rapid repeat install; touching
    # twice within one timestamp tick must not leave discovery on the old list.
    local plist="$bundle/Contents/Info.plist" plist_mtime
    plist_mtime="$(stat -f %m "$plist")"
    touch -m "$plist"
    while [[ "$(stat -f %m "$plist")" == "$plist_mtime" ]]; do
      sleep 1
      touch -m "$plist"
    done
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

  # `commit` is where the WORKTREE stands; `tag` is what the binary in the slot
  # says about itself. They are different facts and they disagree whenever a
  # session committed after building, so both are recorded and only `tag`
  # answers "which build am I looking at?".
  local loaded_tag; loaded_tag="$(build_tag "$dylib" "$branch")"
  { echo "worktree=$path"
    echo "branch=$branch"
    echo "commit=$(git -C "$path" show -s --format=%h HEAD 2>/dev/null || echo '?')"
    echo "tag=${loaded_tag:-unknown}"
    echo "loaded_at=$(date +%s)"
  } > "$LOADED"

  echo
  echo "Now loaded: $branch. Deactivate + reactivate the plugin in Bitwig to pick it up."
  if [[ -n "$loaded_tag" ]]; then
    echo "The performance overlay will read:  build  $loaded_tag"
    local head_short loaded_sha
    head_short="$(git -C "$path" show -s --format=%h HEAD 2>/dev/null || echo '?')"
    loaded_sha="${loaded_tag##*@}"
    if [[ "$head_short" != '?' \
       && "$head_short" != "$loaded_sha"* && "$loaded_sha" != "$head_short"* ]]; then
      echo "  (that is NOT the branch's HEAD, $head_short — this build predates the last" >&2
      echo "   commit on it, which is what building before committing leaves behind.)" >&2
    fi
  else
    echo "WARNING: no build tag readable in that dylib, so the overlay cannot confirm" >&2
    echo "         which build is live. Expect it to read 'unknown'." >&2
  fi
  # That reload only re-reads the file if Harmonigraph's sandbox process exits when
  # the plugin unloads, which is a question of Bitwig's Plug-in Hosting Mode under
  # Settings -> Plug-ins. "by Vendor" puts every plug-in sharing a VENDOR string
  # into ONE process, and that process lives as long as ANY of them is loaded — so
  # a second plug-in of your own pins the old Harmonigraph image in memory and the
  # reload is a no-op. "by Plug-in" and "Individually" each give it a process of its
  # own; "with Bitwig" loads it into the audio engine, which unloads nothing.
  echo "(Needs a sandbox process of its own to re-read the file: Bitwig's 'by Vendor'"
  echo " hosting mode shares one process across every plug-in with your VENDOR string.)"
}

# Echo the index of the worktree containing directory $1, or nothing.
#
# LONGEST match, not the first one that fits: Claude worktrees live under the
# main checkout's own `.claude/worktrees/`, so their paths also fit main. Codex
# worktrees normally live elsewhere, but both kinds travel through this one
# lookup.
wt_containing() {
  local dir="$1" i best="" best_len=0 len
  for i in "${!WT_PATH[@]}"; do
    if [[ "$dir" == "${WT_PATH[$i]}" || "$dir" == "${WT_PATH[$i]}"/* ]]; then
      len=${#WT_PATH[$i]}
      (( len > best_len )) && { best="$i"; best_len=$len; }
    fi
  done
  [[ -n "$best" ]] && { echo "$best"; return 0; }
  return 1
}

# --- dispatch ----------------------------------------------------------------
case "${1:-}" in
  --list|-l)
    print_table
    ;;
  --tag|-t)
    # Print the tag a build will show in the overlay. With no branch, the one
    # belonging to the worktree you are standing in — which is what a session
    # handing over its own build wants.
    if [[ -n "${2:-}" ]]; then
      # Unique substring, on the same terms as loading: a query that silently
      # picks one of several matches reports a tag for a build you did not name,
      # which is the exact failure this mode exists to remove.
      tag_matches=()
      for i in "${!WT_BRANCH[@]}"; do
        [[ "${WT_BRANCH[$i]}" == *"$2"* ]] && tag_matches+=("$i")
      done
      if (( ${#tag_matches[@]} == 0 )); then
        echo "No worktree branch matching '$2'." >&2; exit 1
      elif (( ${#tag_matches[@]} > 1 )); then
        echo "'$2' matches multiple branches:" >&2
        for i in "${tag_matches[@]}"; do echo "  ${WT_BRANCH[$i]}" >&2; done
        exit 1
      fi
      tag_idx="${tag_matches[0]}"
    else
      tag_idx="$(wt_containing "$PWD")" || {
        echo "Not inside a known worktree; pass a branch: ./load-plugin.sh --tag <branch>" >&2
        exit 1
      }
    fi
    tag_dylib="$(find_dylib "${WT_PATH[$tag_idx]}")" || {
      echo "No build in ${WT_PATH[$tag_idx]}/target/release." >&2; exit 1; }
    tag_out="$(build_tag "$tag_dylib" "${WT_BRANCH[$tag_idx]}")"
    [[ -n "$tag_out" ]] || { echo "No build tag readable in $tag_dylib." >&2; exit 1; }
    echo "$tag_out"
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
    echo "Usage: ./load-plugin.sh [--list | --tag [branch] | <branch>]" >&2
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
