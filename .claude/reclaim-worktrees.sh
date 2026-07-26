#!/usr/bin/env bash
#
# Keep the disk from filling up without taxing every session start.
#
# TWO TIERS, because "reclaim disk" and "remove a worktree" are different
# questions and only the second one needs to know whether work is merged:
#
#   1. PRUNE `target/debug` and `target/doc` from any idle worktree. They are
#      regenerable build caches that hold no work, and they are the bulk of
#      the footprint: 27G of 33G across nine worktrees, measured 2026-07-26.
#      Needing no merge detection is the whole point — see WHY TIER 1 below.
#
#   2. REMOVE a whole worktree once its work is provably merged and its tree
#      is clean. Tier 1 has already reclaimed the space by then, so this is
#      a tidiness and inode win rather than a disk win.
#
# TIER 1 CANNOT BREAK THE HANDOVER CONTRACT, which is what makes it safe to
# run on worktrees whose work is unfinished. CLAUDE.md's contract is that a
# paused session leaves a build loadable via `./load-plugin.sh <branch>`, and
# `load-plugin.sh` reads `target/release/libmidi_lattice_3d.dylib` — 11M, in
# `release`, never in `debug`. `target/release` is therefore never pruned.
# `debug` holds test and clippy output that only `ci.sh` consumes.
#
# WHY TIER 1 EXISTS: `git merge-base --is-ancestor` cannot see a SQUASH
# merge, and CLAUDE.md makes squashing the default. A squash-merged branch's
# commits are never ancestors of main, so the merged-only rule retained 14.5G
# across four already-merged PRs (#101, #103, #104, #105) with no expiry —
# the disk filled while every gate reported "unmerged, keep". Patch-id
# containment was tried as a fix and rejected: it caught one of those four,
# because main modified the same files afterwards, and cost 1.9s. Tier 1 does
# not ask the question at all.
#
# COST. A `df` check gates everything and takes ~6ms, so a session with room
# to spare pays that and exits. Only under FREE_LOW_WATER_GB does the scan
# run (~185ms per worktree). The `rm -rf` is detached: a `target/debug` holds
# ~51k files, so deleting several synchronously would stall session start for
# tens of seconds. Each dir is renamed aside (atomic, same volume) and
# deleted by a background process; a leftover staging dir from a killed run
# is swept by the next one.
#
# A worktree is REMOVED (tier 2) only when ALL of these hold:
#   - it lives under .claude/worktrees/ (never touch a hand-made worktree)
#   - it is not the main checkout
#   - it is not the worktree this session is running in
#   - its HEAD is an ancestor of main, so the work is merged and nothing is lost
#   - `git status --porcelain` is empty: no uncommitted and no untracked files
#   - it is not locked by a process that is still alive
#   - nothing near its top level was touched in the last MIN_IDLE_MINUTES
#
# A worktree's cache is PRUNED (tier 1) on the same ownership, session and
# live-lock checks, plus: `target/debug` itself has not been written in
# PRUNE_IDLE_MINUTES. Merge state is deliberately not consulted.
#
# `git worktree remove` keeps the branch ref, so merged commits stay reachable
# and the branch can be checked out again later.
#
# Runs automatically at SessionStart, wired up in .claude/settings.json. Also
# safe to run by hand, from the main checkout OR any worktree — it locates the
# main checkout through `git worktree list` rather than trusting $PWD:
#
#   RECLAIM_DRY_RUN=1 .claude/reclaim-worktrees.sh   # explain, change nothing
#   RECLAIM_FORCE=1   .claude/reclaim-worktrees.sh   # ignore the df gate
#   RECLAIM_DRY_RUN=1 RECLAIM_FORCE=1 RECLAIM_PRUNE_IDLE_MINUTES=1 \
#     .claude/reclaim-worktrees.sh                   # see it find things NOW
#
# DRY_RUN prints one line per decision, on stderr, because stdout is reserved
# for the systemMessage JSON. It is verbose on purpose: the failure mode this
# guards against is a hand-run that silently does nothing and gives no clue
# why — which is exactly what the earlier $PWD-derived path produced.
#
# A no-op prints nothing, so a session that reclaims nothing stays quiet; when
# it does free something it reports the total as a systemMessage.
#
# Written for bash 3.2 (macOS system bash): no mapfile, no associative arrays.

set -uo pipefail

MIN_IDLE_MINUTES=${RECLAIM_MIN_IDLE_MINUTES:-120}
PRUNE_IDLE_MINUTES=${RECLAIM_PRUNE_IDLE_MINUTES:-480}
# 80G is about ten concurrent release builds' headroom (~5G each, and a
# disk-full mid-build fails every running agent, not just the newest). Above
# it there is room to spare and the scan is not worth its ~185ms per
# worktree; below it, reclaiming is worth more than the time it costs.
FREE_LOW_WATER_GB=${RECLAIM_FREE_LOW_WATER_GB:-80}
DRY_RUN=${RECLAIM_DRY_RUN:-0}
FORCE=${RECLAIM_FORCE:-0}

# SessionStart delivers its payload as JSON on stdin; a hand-run has a tty and
# must not block waiting for input that never comes.
SESSION_CWD=""
if [ ! -t 0 ]; then
  payload=$(cat 2>/dev/null || true)
  if [ -n "$payload" ] && command -v jq >/dev/null 2>&1; then
    SESSION_CWD=$(printf '%s' "$payload" | jq -r '.cwd // empty' 2>/dev/null || true)
  fi
fi
[ -n "$SESSION_CWD" ] || SESSION_CWD=$PWD

ROOT=${CLAUDE_PROJECT_DIR:-$SESSION_CWD}
git -C "$ROOT" rev-parse --git-dir >/dev/null 2>&1 || exit 0

WOULD_DO=0

# Diagnostics for a hand-run. Silence is the right behaviour for the hook —
# SessionStart fires on every session — but it is a terrible answer to
# "why did nothing happen?", so DRY_RUN explains every decision it makes.
# These go to stderr; stdout stays reserved for the systemMessage JSON.
note() {
  [ "$DRY_RUN" = 1 ] || return 0
  printf '%s\n' "$1" >&2
}

# A note that also counts as "this run would have changed something", which is
# what separates "examined everything, all of it in use" from "did nothing".
act() {
  WOULD_DO=$((WOULD_DO + 1))
  note "$1"
}

# The first record `git worktree list` prints is always the main checkout, and
# that holds from ANY worktree of the repo. Derive both paths from it rather
# than from ROOT: ROOT falls back to $PWD, so a hand-run from inside
# .claude/worktrees/<branch>/ would otherwise look for
# <branch>/.claude/worktrees and find nothing.
MAIN_WT=$(git -C "$ROOT" worktree list --porcelain | awk '/^worktree /{print substr($0, 10); exit}')
if [ -z "$MAIN_WT" ]; then
  note 'could not resolve the main checkout from git worktree list'
  exit 0
fi

WT_DIR="$MAIN_WT/.claude/worktrees"
if [ ! -d "$WT_DIR" ]; then
  note "no worktree dir at $WT_DIR — nothing this script manages"
  exit 0
fi

# Sweep staging dirs a previous run left behind BEFORE the df gate: a killed
# background delete is exactly the case where space is still held, so exiting
# early on a "roomy" reading would strand it forever.
for stale in "$WT_DIR"/*/target/.reclaiming-*; do
  [ -d "$stale" ] || continue
  if [ "$DRY_RUN" = 1 ]; then
    act "would sweep leftover $stale"
  else
    nohup rm -rf "$stale" >/dev/null 2>&1 &
  fi
done

# The cheap gate. Everything below costs real time, so a roomy disk stops here.
free_gb=$(df -k "$ROOT" 2>/dev/null | awk 'NR==2 {printf "%d", $4 / 1048576}')
[ -n "$free_gb" ] || free_gb=0
if [ "$FORCE" != 1 ] && [ "$free_gb" -ge "$FREE_LOW_WATER_GB" ]; then
  note "df gate: ${free_gb}G free >= ${FREE_LOW_WATER_GB}G low water, so nothing to do (RECLAIM_FORCE=1 overrides)"
  exit 0
fi
note "df gate: ${free_gb}G free, low water ${FREE_LOW_WATER_GB}G$([ "$FORCE" = 1 ] && printf ' (forced)')"

# Resolve main once. Without it "merged" is unanswerable, so tier 2 is skipped
# while tier 1 — which never asks — still runs.
MAIN_REF=""
for ref in main origin/main; do
  if git -C "$ROOT" rev-parse --verify --quiet "$ref" >/dev/null 2>&1; then
    MAIN_REF=$ref
    break
  fi
done

freed_kb=0
removed=0
pruned=0
names=""

human() {
  awk -v kb="$1" 'BEGIN {
    if (kb >= 1048576) printf "%.1fG", kb / 1048576
    else if (kb >= 1024) printf "%.0fM", kb / 1024
    else printf "%dK", kb
  }'
}

# Rename aside, then delete detached. The rename is atomic within the volume,
# so cargo never observes a half-deleted cache even though the delete outlives
# this script.
detach_delete() {
  victim=$1
  staging="$(dirname "$victim")/.reclaiming-$$-$(basename "$victim")"
  mv "$victim" "$staging" 2>/dev/null || return 1
  nohup rm -rf "$staging" >/dev/null 2>&1 &
  return 0
}

# Ownership checks shared by both tiers. Non-zero means leave this worktree
# completely alone.
usable() {
  path=$1
  locked=$2
  reason=$3
  name=$(basename "$path")

  [ -d "$path" ] || return 1
  [ "$path" = "$MAIN_WT" ] && return 1

  # Only ever touch worktrees Claude created.
  case "$path" in
    */.claude/worktrees/*) ;;
    *) note "skip $name: not under .claude/worktrees"; return 1 ;;
  esac

  # Never saw off the branch we are sitting on.
  case "$SESSION_CWD/" in
    "$path"/*) note "skip $name: this session is running in it"; return 1 ;;
  esac

  # A lock naming a live pid means a session still owns this worktree. A lock we
  # cannot attribute to a pid is left alone rather than guessed at.
  if [ "$locked" = 1 ]; then
    pid=$(printf '%s' "$reason" | sed -n 's/.*pid \([0-9][0-9]*\).*/\1/p')
    if [ -z "$pid" ]; then
      note "skip $name: locked, no pid in the reason to check"
      return 1
    fi
    if ps -p "$pid" >/dev/null 2>&1; then
      note "skip $name: locked by live pid $pid"
      return 1
    fi
  fi
  return 0
}

# Tier 1: regenerable caches out of an idle worktree. Never touches release.
prune_caches() {
  path=$1
  for sub in debug doc; do
    victim="$path/target/$sub"
    [ -d "$victim" ] || continue

    # Idle by the cache's own recent writes, which is a truer "nobody is using
    # this" signal than the worktree's top level. maxdepth 2 rather than 0
    # because an incremental build can touch only `deps/` or `.fingerprint/`
    # without ever updating `debug/`'s own mtime, and pruning mid-build would
    # break that build. The live-lock check above is the primary guard; this is
    # depth behind it, and `-quit` keeps it to a few hundred stats.
    if [ -n "$(find "$victim" -maxdepth 2 -newermt "-${PRUNE_IDLE_MINUTES} minutes" -print -quit 2>/dev/null)" ]; then
      note "keep $(basename "$path")/target/$sub: written in the last ${PRUNE_IDLE_MINUTES}m"
      continue
    fi

    size_kb=$(du -sk "$victim" 2>/dev/null | awk '{print $1}')
    [ -n "$size_kb" ] || size_kb=0

    if [ "$DRY_RUN" = 1 ]; then
      act "would prune $victim ($(human "$size_kb"))"
      continue
    fi

    if detach_delete "$victim"; then
      pruned=$((pruned + 1))
      freed_kb=$((freed_kb + size_kb))
    fi
  done
}

# Tier 2: the whole worktree, only when the work is provably safe to lose.
remove_worktree() {
  path=$1
  head=$2
  locked=$3

  name=$(basename "$path")
  [ -n "$MAIN_REF" ] || { note "no-remove $name: could not resolve main"; return 1; }

  # Merged into main? Unmerged work is never removed. This misses squash
  # merges by construction, which is why tier 1 does not depend on it.
  if ! git -C "$ROOT" merge-base --is-ancestor "$head" "$MAIN_REF" 2>/dev/null; then
    note "no-remove $name: HEAD not an ancestor of $MAIN_REF (a squash merge reads this way too)"
    return 1
  fi

  # Clean? --porcelain lists untracked files too, so a stray scratch file saves it.
  status=$(git -C "$path" status --porcelain 2>/dev/null) || return 1
  if [ -n "$status" ]; then
    note "no-remove $name: $(printf '%s' "$status" | wc -l | tr -d ' ') uncommitted/untracked file(s)"
    return 1
  fi

  # Belt and braces for work that is committed but still being used: maxdepth
  # keeps this cheap, and catches both source edits and a running build's
  # writes to target/{debug,release}.
  if [ -n "$(find "$path" -maxdepth 2 -newermt "-${MIN_IDLE_MINUTES} minutes" -print -quit 2>/dev/null)" ]; then
    note "no-remove $name: touched in the last ${MIN_IDLE_MINUTES}m"
    return 1
  fi

  size_kb=$(du -sk "$path" 2>/dev/null | awk '{print $1}')
  [ -n "$size_kb" ] || size_kb=0

  if [ "$DRY_RUN" = 1 ]; then
    act "would remove $path ($(human "$size_kb"))"
    return 0
  fi

  [ "$locked" = 1 ] && git -C "$ROOT" worktree unlock "$path" >/dev/null 2>&1
  if git -C "$ROOT" worktree remove "$path" >/dev/null 2>&1; then
    removed=$((removed + 1))
    freed_kb=$((freed_kb + size_kb))
    names="$names $(basename "$path" | tr -cd 'A-Za-z0-9._-')"
    return 0
  fi
  return 1
}

consider() {
  path=$1
  head=$2
  locked=$3
  reason=$4

  usable "$path" "$locked" "$reason" || return 0

  # Tier 2 first: a full removal makes tier 1 moot for this worktree, and
  # sizing the whole tree once beats sizing it and then its caches.
  remove_worktree "$path" "$head" "$locked" && return 0
  prune_caches "$path"
}

# --porcelain emits one blank-line-separated record per worktree:
#   worktree <path> / HEAD <sha> / branch <ref> / [locked [<reason>]]
cur_path=""
cur_head=""
cur_locked=0
cur_reason=""

flush() {
  [ -n "$cur_path" ] || return 0
  consider "$cur_path" "$cur_head" "$cur_locked" "$cur_reason"
  cur_path=""
  cur_head=""
  cur_locked=0
  cur_reason=""
}

while IFS= read -r line; do
  case "$line" in
    "worktree "*) flush; cur_path=${line#worktree } ;;
    "HEAD "*)     cur_head=${line#HEAD } ;;
    "locked"*)    cur_locked=1; cur_reason=${line#locked} ;;
  esac
done < <(git -C "$ROOT" worktree list --porcelain)
flush

git -C "$ROOT" worktree prune >/dev/null 2>&1

if [ "$DRY_RUN" = 1 ]; then
  if [ "$WOULD_DO" = 0 ]; then
    note 'nothing eligible — every worktree above is in use, recently built, or unmerged'
  else
    note "$WOULD_DO action(s) eligible; re-run without RECLAIM_DRY_RUN=1 to apply"
  fi
  exit 0
fi

# Stay silent on a no-op; SessionStart runs on every single session.
if [ "$removed" -gt 0 ] || [ "$pruned" -gt 0 ]; then
  detail=""
  [ "$removed" -gt 0 ] && detail="$removed merged worktree(s):$names"
  if [ "$pruned" -gt 0 ]; then
    [ -n "$detail" ] && detail="$detail, "
    detail="${detail}${pruned} idle build cache(s)"
  fi
  printf '{"systemMessage":"Reclaimed %s of disk from %s"}\n' \
    "$(human "$freed_kb")" "$detail"
fi

exit 0
