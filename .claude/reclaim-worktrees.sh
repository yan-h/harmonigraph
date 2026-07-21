#!/usr/bin/env bash
#
# Remove Claude worktrees that are provably finished, so their per-worktree Rust
# `target/` dirs (1.5-8 GB each; no sccache, no shared CARGO_TARGET_DIR) stop
# filling the disk. On 2026-07-19 thirty worktrees held 99 GB and the volume hit
# 100% — a disk-full mid-build fails every concurrent agent, not just the newest.
#
# A worktree is removed only when ALL of these hold:
#   - it lives under .claude/worktrees/ (never touch a hand-made worktree)
#   - it is not the main checkout
#   - it is not the worktree this session is running in
#   - its HEAD is an ancestor of main, so the work is merged and nothing is lost
#   - `git status --porcelain` is empty: no uncommitted and no untracked files
#   - it is not locked by a process that is still alive
#   - nothing near its top level was touched in the last MIN_IDLE_MINUTES
#
# `git worktree remove` keeps the branch ref, so merged commits stay reachable
# and the branch can be checked out again later.
#
# Safe to run by hand:
#   RECLAIM_DRY_RUN=1 .claude/reclaim-worktrees.sh   # report, remove nothing
#
# To run it automatically, add this to .claude/settings.json:
#   {
#     "hooks": {
#       "SessionStart": [
#         {
#           "hooks": [
#             {
#               "type": "command",
#               "command": "S=\"$CLAUDE_PROJECT_DIR/.claude/reclaim-worktrees.sh\"; [ -x \"$S\" ] && \"$S\" || true",
#               "timeout": 60,
#               "statusMessage": "Reclaiming finished worktrees"
#             }
#           ]
#         }
#       ]
#     }
#   }
#
# Written for bash 3.2 (macOS system bash): no mapfile, no associative arrays.

set -uo pipefail

MIN_IDLE_MINUTES=${RECLAIM_MIN_IDLE_MINUTES:-120}
DRY_RUN=${RECLAIM_DRY_RUN:-0}

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

# Resolve main once. Without it "merged" is unanswerable, so do nothing.
MAIN_REF=""
for ref in main origin/main; do
  if git -C "$ROOT" rev-parse --verify --quiet "$ref" >/dev/null 2>&1; then
    MAIN_REF=$ref
    break
  fi
done
[ -n "$MAIN_REF" ] || exit 0

# The first record `git worktree list` prints is always the main checkout.
MAIN_WT=$(git -C "$ROOT" worktree list --porcelain | awk '/^worktree /{print substr($0, 10); exit}')

freed_kb=0
removed=0
names=""

human() {
  awk -v kb="$1" 'BEGIN {
    if (kb >= 1048576) printf "%.1fG", kb / 1048576
    else if (kb >= 1024) printf "%.0fM", kb / 1024
    else printf "%dK", kb
  }'
}

consider() {
  path=$1
  head=$2
  locked=$3
  reason=$4

  [ -d "$path" ] || return 0
  [ "$path" = "$MAIN_WT" ] && return 0

  # Only ever reclaim worktrees Claude created.
  case "$path" in
    */.claude/worktrees/*) ;;
    *) return 0 ;;
  esac

  # Never saw off the branch we are sitting on.
  case "$SESSION_CWD/" in
    "$path"/*) return 0 ;;
  esac

  # Merged into main? Unmerged work is never removed.
  git -C "$ROOT" merge-base --is-ancestor "$head" "$MAIN_REF" 2>/dev/null || return 0

  # Clean? --porcelain lists untracked files too, so a stray scratch file saves it.
  status=$(git -C "$path" status --porcelain 2>/dev/null) || return 0
  [ -z "$status" ] || return 0

  # A lock naming a live pid means a session still owns this worktree. A lock we
  # cannot attribute to a pid is left alone rather than guessed at.
  if [ "$locked" = 1 ]; then
    pid=$(printf '%s' "$reason" | sed -n 's/.*pid \([0-9][0-9]*\).*/\1/p')
    [ -n "$pid" ] || return 0
    ps -p "$pid" >/dev/null 2>&1 && return 0
  fi

  # Belt and braces for work that is committed but still being used: maxdepth
  # keeps this cheap, and catches both source edits and a running build's
  # writes to target/{debug,release}.
  if [ -n "$(find "$path" -maxdepth 2 -newermt "-${MIN_IDLE_MINUTES} minutes" -print -quit 2>/dev/null)" ]; then
    return 0
  fi

  size_kb=$(du -sk "$path" 2>/dev/null | awk '{print $1}')
  [ -n "$size_kb" ] || size_kb=0

  if [ "$DRY_RUN" = 1 ]; then
    printf 'would remove %s (%s)\n' "$path" "$(human "$size_kb")" >&2
    return 0
  fi

  [ "$locked" = 1 ] && git -C "$ROOT" worktree unlock "$path" >/dev/null 2>&1
  if git -C "$ROOT" worktree remove "$path" >/dev/null 2>&1; then
    removed=$((removed + 1))
    freed_kb=$((freed_kb + size_kb))
    names="$names $(basename "$path" | tr -cd 'A-Za-z0-9._-')"
  fi
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

# Stay silent on a no-op; SessionStart runs on every single session.
if [ "$removed" -gt 0 ]; then
  printf '{"systemMessage":"Reclaimed %s of disk from %d merged worktree(s):%s"}\n' \
    "$(human "$freed_kb")" "$removed" "$names"
fi

exit 0
