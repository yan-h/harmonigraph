#!/usr/bin/env bash
#
# Keep the disk from filling up without taxing every session start.
#
# TWO TIERS, because "reclaim disk" and "remove a worktree" are different
# questions and only the second one needs to know whether work is merged:
#
#   1. PRUNE `target/debug` and `target/doc` from any idle Claude worktree.
#      They are regenerable build caches that hold no work, and they are the
#      bulk of the footprint: 27G of 33G across nine worktrees, measured
#      2026-07-26. Needing no merge detection is the whole point — see WHY
#      TIER 1 below.
#
#   2. REMOVE a whole worktree once its work is provably merged and its tree
#      is clean. Tier 1 has already reclaimed the space by then, so this is
#      a tidiness and inode win rather than a disk win.
#
# TIER 1 CANNOT BREAK THE HANDOVER CONTRACT, which is what makes it safe to
# run on worktrees whose work is unfinished. CLAUDE.md's contract is that a
# paused session leaves a build loadable via `./load-plugin.sh <branch>`, and
# `load-plugin.sh` reads `target/release/libharmonigraph_plugin.dylib` — 13M, in
# `release`, never in `debug`. `target/release` is therefore never pruned.
# `debug` holds test and clippy output that only `ci.sh` consumes.
#
# HOW TIER 2 SEES A SQUASH MERGE, which is the thing that made merged
# worktrees pile up. `git merge-base --is-ancestor` cannot see one, and the
# `pr-hygiene` skill makes squashing the default, so four already-merged
# PRs (#101, #103, #104, #105) sat undeletable while every gate said
# "unmerged, keep".
# Tier 2 therefore takes THREE signals, and a worktree needs any one of:
#   - HEAD is an ancestor of main. Offline, instant, covers merge-commit PRs.
#   - a MERGED PR's head sha EQUALS this worktree's HEAD, from one
#     `gh pr list` call (~1.5s for 600 PRs, once per run, lazy). Sha equality
#     is the safety: the worktree holds exactly what merged and nothing newer,
#     so main's squash commit supersedes it.
#   - every commit reachable from HEAD is reachable from main or from some
#     RESOLVED PR head — merged or closed. See CONTAINMENT below.
# Rejected alternatives: patch-id containment caught one of the four, because
# main edited the same files afterwards, and cost 1.9s. "Remote branch is
# gone" was worse — 46 of 48 merged branches are still on the remote here,
# because --delete-branch mostly did not run.
#
# CONTAINMENT, and why equality alone left 17 worktrees standing on a disk at
# 95%. Equality answers "did exactly this merge?", which is a narrower question
# than the one tier 2 is actually asking, and it missed in two directions at
# once (measured 2026-09-07, 29 Claude worktrees, 26 of them reclaimable):
#   - BEHIND a merge. Five worktrees (#375, #390, #444, #505, #506) held a
#     strict ANCESTOR of the sha that merged: the branch was pushed to, or
#     amended, after the worktree last built, so the local ref trailed the
#     remote. Every commit in the tree was in main, and equality still said no.
#   - CLOSED unmerged. Sixteen more (the rejected #568 crease set, #453, #493,
#     #683, #687, #581) belonged to PRs a human explicitly closed. Under a
#     merged-only rule those are PERMANENT residents — no future event can ever
#     make them eligible — so rejected experiments accumulate forever, one
#     `target/release` apiece.
# A closed PR is as final a verdict as a merged one; both mean nobody is coming
# back to that worktree. What makes acting on either safe is that removal loses
# no commits at all: `git worktree remove` KEEPS the branch ref, the remote
# still has the branch, and GitHub keeps `refs/pull/<n>/head` even for a closed
# PR. The only things a removal can destroy are uncommitted files and the
# built `target/release` — which is why the clean-tree gate below stays strict,
# and why a branch with no PR at all is still kept: an unresolved branch is
# work in flight, and there is no verdict to read.
#
# Containment is ONE `git rev-list --ignore-missing --stdin --count` per
# worktree, negating main and every resolved PR head at once. `--ignore-missing`
# is what makes it safe to feed shas this clone may never have fetched (a
# deleted remote branch), and `--stdin` keeps 600 shas off the argv limit. Zero
# means the tree adds nothing to what is already resolved. A per-sha
# `merge-base --is-ancestor` loop answers the same question and costs 300+ git
# invocations per worktree, which is why it is not what runs here.
#
# WHY TIER 1 STILL EXISTS once tier 2 can see squashes: an UNFINISHED branch
# is never removable, and its cache is still dead weight. The three fade-*
# worktrees had no PR at all and held 8.6G of `debug` between them. Tier 1
# also needs no network, so it keeps working when gh is unavailable.
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
#   - it is a direct child of .claude/worktrees/ (never touch a hand-made or
#     Codex-managed worktree)
#   - it is not the main checkout
#   - it is not the worktree this session is running in
#   - its work is RESOLVED by one of the three signals above, so the commits
#     survive the removal in main, on the remote, or on the kept branch ref
#   - `git status --porcelain` is empty: no uncommitted and no untracked files
#   - it is not locked by a live SESSION — a lock whose pid is sitting in the
#     `claude bg-spare` pool, which its claim socket still being on disk is
#     what says, is stale and does not protect anything. Every other reading
#     of a live pid counts as live; `.claude/tests/reclaim-locks.sh` is the
#     gate on that, because getting it wrong deletes a directory
#   - nothing near its top level was touched in the last MIN_IDLE_MINUTES
#
# A worktree's cache is PRUNED (tier 1) on the same ownership, session and
# live-lock checks, plus: `target/debug` itself has not been written in
# PRUNE_IDLE_MINUTES. Merge state is deliberately not consulted.
#
# ORPHANS are swept last, and they are the miss no gate above could catch:
# both tiers walk `git worktree list`, so a directory git no longer counts as a
# worktree is not skipped for a reason — it is never looked at. Six of them
# were sitting under .claude/worktrees on 2026-09-07, invisible to every
# dry-run line. They arise when the admin entry in .git/worktrees is pruned
# while the directory survives: a `git worktree prune` after the tree was moved
# or partly deleted, or a session killed mid-teardown. The sweep runs AFTER the
# `git worktree prune` below, so a stale admin entry is resolved into either a
# real record or an orphan before anything is judged, and it removes only a
# direct child that git does not list, holds no live `.git`, and has been idle
# for MIN_IDLE_MINUTES. There is no merge question to ask: git has already
# forgotten the worktree, so any branch it once held is reachable by name or
# not at all, and nothing this script does changes that.
#
# `git worktree remove` keeps the branch ref, so merged commits stay reachable
# and the branch can be checked out again later.
#
# Codex owns cleanup and snapshots for its managed worktrees, so both tiers
# leave those alone even though `git worktree list` includes them. Exact
# direct-child ownership also protects a Codex root configured below
# `.claude/worktrees`: its `<id>/<repo>` worktrees are not Claude's to release.
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

# The squash-merge answer, and the only thing here that touches the network.
# One `gh pr list` call per run resolves every RESOLVED PR's head sha — merged
# and closed alike — which is what makes a squash-merged or explicitly rejected
# worktree removable rather than merely prunable.
# It is lazy (nothing asks until a worktree fails the ancestor check), bounded
# (killed after GH_TIMEOUT_S), and fail-safe: any failure falls back to
# ancestor-only, so a flaky network makes the script conservative, never wrong.
#
# The limit is a CEILING ON HISTORY, not a page size, and setting it too low
# fails silently in the one direction that matters: a PR older than the newest
# GH_PR_LIMIT is simply invisible, and its worktree reads as unresolved
# forever. 300 was already below this repo's 530 merged PRs on 2026-09-07 —
# everything before #261 was unreadable. Keep it comfortably above the total
# PR count; the call is lazy and costs ~1.5s at 600.
NO_NETWORK=${RECLAIM_NO_NETWORK:-0}
GH_TIMEOUT_S=${RECLAIM_GH_TIMEOUT_S:-8}
GH_PR_LIMIT=${RECLAIM_GH_PR_LIMIT:-1000}

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
HELD_LOCKED_KB=0
HELD_RECENT_KB=0

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
# Both shapes: tier 1 stages a cache inside its own worktree's `target/`, tier 3
# stages a whole orphan directory alongside its siblings in WT_DIR.
for stale in "$WT_DIR"/*/target/.reclaiming-* "$WT_DIR"/.reclaiming-*; do
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

# One sha per line for every merged PR, and one per line for every resolved PR
# (merged or closed). Loaded at most once per run.
MERGED_HEADS=""
RESOLVED_HEADS=""
MERGED_TRIED=0

load_merged_heads() {
  [ "$MERGED_TRIED" = 1 ] && return 0
  MERGED_TRIED=1

  if [ "$NO_NETWORK" = 1 ]; then
    note 'gh lookup disabled (RECLAIM_NO_NETWORK=1) — ancestor-only, so squash merges stay'
    return 0
  fi
  if ! command -v gh >/dev/null 2>&1; then
    note 'gh not on PATH — ancestor-only, so squash merges stay'
    return 0
  fi

  tmp=$(mktemp) || return 0
  # Head shas only, NOT "<branch> <sha>" pairs: keying on the branch name misses
  # a branch that was renamed, or a second local branch pointing at the same
  # merged commit (pr97 and pr98 here are aliases for the heads of #97 and
  # #98). The sha alone is the whole claim — if HEAD *is* a merged PR's head
  # commit, that work merged, whatever the branch is called.
  # --jq uses gh's embedded jq, so this needs no external jq. `--state all`
  # rather than two calls: one round trip carries both verdicts, and OPEN is
  # dropped here so an in-flight PR never counts as resolved.
  gh pr list --state all --limit "$GH_PR_LIMIT" \
    --json state,headRefOid \
    --jq '.[] | select(.state == "MERGED" or .state == "CLOSED")
              | "\(.state) \(.headRefOid)"' >"$tmp" 2>/dev/null &
  gh_pid=$!

  # Bound it by hand: macOS ships no `timeout`, and a hung network call must not
  # stall session start. Counted in half-seconds.
  halves=0
  limit=$((GH_TIMEOUT_S * 2))
  while kill -0 "$gh_pid" 2>/dev/null && [ "$halves" -lt "$limit" ]; do
    sleep 0.5
    halves=$((halves + 1))
  done
  if kill -0 "$gh_pid" 2>/dev/null; then
    kill "$gh_pid" 2>/dev/null
    wait "$gh_pid" 2>/dev/null
    note "gh pr list exceeded ${GH_TIMEOUT_S}s — ancestor-only for this run"
    rm -f "$tmp"
    return 0
  fi
  wait "$gh_pid" 2>/dev/null

  if [ -s "$tmp" ]; then
    MERGED_HEADS=$(awk '$1 == "MERGED" {print $2}' "$tmp")
    RESOLVED_HEADS=$(awk '{print $2}' "$tmp")
    note "gh: $(printf '%s\n' "$RESOLVED_HEADS" | wc -l | tr -d ' ') resolved PR head(s) ($(printf '%s\n' "$MERGED_HEADS" | wc -l | tr -d ' ') merged)"
  else
    note 'gh returned nothing (offline, or not authenticated) — ancestor-only for this run'
  fi
  rm -f "$tmp"
}

# True when a MERGED PR's head sha is exactly this worktree's HEAD. Equality is
# the safety: it means the worktree holds the work that merged and nothing
# newer, so the squash commit on main supersedes it completely. A worktree that
# has moved on past its merge fails this and is kept.
# -F -x: the sha is data, not a pattern.
pr_merged_at_head() {
  [ -n "$MERGED_HEADS" ] || return 1
  printf '%s\n' "$MERGED_HEADS" | grep -Fxq "$1"
}

# True when every commit reachable from HEAD is already reachable from main or
# from some resolved PR head — the worktree adds nothing unresolved to the
# repo. This is what catches a branch that trails the sha which merged, and a
# branch whose PR a human closed unmerged; see CONTAINMENT in the header.
#
# The head sha is verified to EXIST first, and that guard is load-bearing:
# `--ignore-missing` drops an unknown POSITIVE rev as readily as an unknown
# negative one, so an unresolvable HEAD would otherwise count zero commits and
# read as fully contained — the one input that turns this test into a
# rubber stamp. Bad sha in, "keep" out.
head_contained_in_resolved() {
  head=$1
  [ -n "$RESOLVED_HEADS" ] || return 1
  git -C "$ROOT" cat-file -e "$head^{commit}" 2>/dev/null || return 1

  count=$( { printf '%s\n' "$head"
             [ -n "$MAIN_REF" ] && printf '^%s\n' "$MAIN_REF"
             printf '^%s\n' $RESOLVED_HEADS
           } | git -C "$ROOT" rev-list --ignore-missing --stdin --count 2>/dev/null )
  [ "$count" = 0 ]
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

  # Claude owns exactly the direct children of this directory. A broader path
  # prefix also catches Codex's `<id>/<repo>` shape when its configurable root
  # sits below .claude/worktrees, and would let this hook prune or remove a
  # worktree whose lifecycle belongs to the app.
  if [ "$(dirname "$path")" != "$WT_DIR" ]; then
    note "skip $name: not a direct Claude-owned worktree"
    return 1
  fi

  # Never saw off the branch we are sitting on.
  case "$SESSION_CWD/" in
    "$path"/*)
      if [ "$DRY_RUN" = 1 ]; then
        kb=$(du -sk "$path/target/debug" "$path/target/doc" 2>/dev/null | awk '{s+=$1} END{print s+0}')
        HELD_LOCKED_KB=$((HELD_LOCKED_KB + ${kb:-0}))
        note "skip $name: this session is running in it (holding $(human "${kb:-0}") of cache)"
      fi
      return 1 ;;
  esac

  # A lock naming a live pid means a session still owns this worktree. A lock we
  # cannot attribute to a pid is left alone rather than guessed at.
  #
  # Attributable means written by the harness, in its own format, and the whole
  # string has to match — a bare `pid <n>` anywhere in the reason is not enough.
  # Prose naming a number is what a PERSON writes, and a person's lock is the
  # one this script must never answer for: CLAUDE.md tells sessions a
  # hand-written lock stands until a human clears it, so reading a pid out of
  # its prose and finding it dead would delete the worktree that promise covers.
  if [ "$locked" = 1 ]; then
    pid=$(printf '%s' "$reason" | \
      sed -n 's/^ *claude session .* (pid \([0-9][0-9]*\) start .*)$/\1/p')
    if [ -z "$pid" ]; then
      note "skip $name: locked by a reason this script did not write"
      return 1
    fi
    if ps -p "$pid" >/dev/null 2>&1; then
      # The pid being alive does NOT make the lock live. Every local session
      # runs as a `claude bg-spare` process taken from a warm pool, and a spare
      # that goes back to the pool keeps its pid, so a dead session's lock is
      # indistinguishable from a live one by pid alone.
      #
      # ARGV CANNOT SETTLE IT, and reaching for a flag is the trap here: argv
      # is fixed at exec, and a spare is claimed afterwards over the socket it
      # already names, so a claimed spare's cmdline is byte-identical to an
      # unclaimed one's. There is no `--session-id` to key on — a test for one
      # matches nothing and calls EVERY live session's lock stale, which is how
      # a worktree gets removed from under a session paused on a question.
      #
      # The SOCKET is the signal, because it records the claim that argv
      # missed: a spare advertises itself on the `.claim.sock` its own argv
      # names, and claiming it unlinks that socket. Still on disk means still
      # in the pool, so the lock protects nothing.
      #
      # This errs live in every case it cannot read — an unrecognised holder,
      # an unparseable path, a spare returned to the pool without re-
      # advertising. Holding disk costs a `rm -rf` later; releasing a live
      # session's lock costs the branch it was about to hand over.
      cmd=$(ps -p "$pid" -o command= 2>/dev/null)
      case "$cmd" in
        *--bg-spare*)
          sock=${cmd##*--bg-spare }
          sock=${sock%% *}
          if [ -n "$sock" ] && [ -e "$sock" ]; then
            note "stale lock $name: pid $pid is an unclaimed bg-spare, not a session"
            return 0
          fi ;;
      esac
      if [ "$DRY_RUN" = 1 ]; then
        kb=$(du -sk "$path/target/debug" "$path/target/doc" 2>/dev/null | awk '{s+=$1} END{print s+0}')
        HELD_LOCKED_KB=$((HELD_LOCKED_KB + ${kb:-0}))
        note "skip $name: locked by live pid $pid (holding $(human "${kb:-0}") of cache)"
      fi
      return 1
    fi
  fi
  return 0
}

# Tier 1: regenerable caches out of an idle worktree. Never touches release.
prune_caches() {
  path=$1
  found=0
  for sub in debug doc; do
    victim="$path/target/$sub"
    [ -d "$victim" ] || continue
    found=1

    # Idle by the cache's own recent writes, which is a truer "nobody is using
    # this" signal than the worktree's top level. maxdepth 2 rather than 0
    # because an incremental build can touch only `deps/` or `.fingerprint/`
    # without ever updating `debug/`'s own mtime, and pruning mid-build would
    # break that build. The live-lock check above is the primary guard; this is
    # depth behind it, and `-quit` keeps it to a few hundred stats.
    if [ -n "$(find "$victim" -maxdepth 2 -newermt "-${PRUNE_IDLE_MINUTES} minutes" -print -quit 2>/dev/null)" ]; then
      if [ "$DRY_RUN" = 1 ]; then
        kb=$(du -sk "$victim" 2>/dev/null | awk '{print $1}')
        HELD_RECENT_KB=$((HELD_RECENT_KB + ${kb:-0}))
        note "keep $(basename "$path")/target/$sub ($(human "${kb:-0}")): written in the last ${PRUNE_IDLE_MINUTES}m"
      fi
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

  # Say so explicitly. Printing nothing here is what made a correct run look
  # like a broken one: five already-pruned worktrees produced no line at all,
  # and the closing summary then guessed a reason that did not apply to them.
  [ "$found" = 0 ] && note "clean $(basename "$path"): no target/debug or target/doc to reclaim"
  return 0
}

# Tier 2: the whole worktree, only when the work is provably safe to lose.
remove_worktree() {
  path=$1
  head=$2
  locked=$3
  branch=$4

  name=$(basename "$path")
  [ -n "$MAIN_REF" ] || { note "no-remove $name: could not resolve main"; return 1; }

  # Three independent resolved-signals, widening in cost order. Each is only
  # consulted when the cheaper one above it says no, so a repo that
  # merge-commits everything never touches the network at all.
  #   ancestor    — covers merge-commit PRs, works offline, always tried first.
  #   gh sha      — covers squash merges, which the ancestor test reads as
  #                 unmerged forever.
  #   containment — covers a branch trailing the sha that merged, and a PR
  #                 closed unmerged. One rev-list; see CONTAINMENT in the header.
  how=""
  if git -C "$ROOT" merge-base --is-ancestor "$head" "$MAIN_REF" 2>/dev/null; then
    how="ancestor of $MAIN_REF"
  else
    load_merged_heads
    if pr_merged_at_head "$head"; then
      how="merged PR, head sha matches"
    elif head_contained_in_resolved "$head"; then
      how="every commit is in $MAIN_REF or a resolved PR"
    fi
  fi
  if [ -z "$how" ]; then
    note "no-remove $name: unresolved — not in $MAIN_REF and not covered by any merged or closed PR"
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
    act "would remove $path ($(human "$size_kb")) — $how"
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
  branch=$5

  usable "$path" "$locked" "$reason" || return 0

  # Tier 2 first: a full removal makes tier 1 moot for this worktree, and
  # sizing the whole tree once beats sizing it and then its caches.
  remove_worktree "$path" "$head" "$locked" "$branch" && return 0
  prune_caches "$path"
}

# --porcelain emits one blank-line-separated record per worktree:
#   worktree <path> / HEAD <sha> / branch <ref> / [locked [<reason>]]
cur_path=""
cur_head=""
cur_locked=0
cur_reason=""
cur_branch=""

flush() {
  [ -n "$cur_path" ] || return 0
  consider "$cur_path" "$cur_head" "$cur_locked" "$cur_reason" "$cur_branch"
  cur_path=""
  cur_head=""
  cur_locked=0
  cur_reason=""
  cur_branch=""
}

while IFS= read -r line; do
  case "$line" in
    "worktree "*) flush; cur_path=${line#worktree } ;;
    "HEAD "*)     cur_head=${line#HEAD } ;;
    "branch "*)   cur_branch=${line#branch refs/heads/} ;;
    "locked"*)    cur_locked=1; cur_reason=${line#locked} ;;
  esac
done < <(git -C "$ROOT" worktree list --porcelain)
flush

git -C "$ROOT" worktree prune >/dev/null 2>&1

# Tier 3: directories under .claude/worktrees that git does not list. Runs after
# the prune above so a stale admin entry has already become one or the other.
# See ORPHANS in the header for why neither tier above can reach these.
REGISTERED=$(git -C "$ROOT" worktree list --porcelain | awk '/^worktree /{print substr($0, 10)}')
for path in "$WT_DIR"/*; do
  [ -d "$path" ] || continue
  name=$(basename "$path")
  case "$name" in .reclaiming-*) continue ;; esac

  # -F -x: a path is data, and a worktree name may hold regex metacharacters
  # (the harness mints names like `bridge-cse_01UJ...` and `claude+branch`).
  printf '%s\n' "$REGISTERED" | grep -Fxq "$path" && continue

  # "git does not list THIS path" is not "git lists nothing under it", and
  # conflating the two deletes a live Codex worktree. Codex's managed shape is
  # `<id>/<repo>`, so when its root is configured below .claude/worktrees the
  # registered worktree is the CHILD and the `<id>` directory above it is
  # listed by nobody — an orphan by the test one line up, and not ours to
  # remove. Anything containing a registered worktree is somebody's tree.
  # -F: the path is data here too.
  printf '%s\n' "$REGISTERED" | grep -Fq "$path/" && {
    note "skip orphan $name: contains a registered worktree, so it is not Claude's to remove"
    continue
  }

  # Never saw off the branch we are sitting on, even when git has forgotten it.
  case "$SESSION_CWD/" in "$path"/*) continue ;; esac

  # A live `.git` means git ought to know about this and something is wrong
  # with the assumption, not with the directory. Leave it for a human.
  if [ -e "$path/.git" ]; then
    note "skip orphan $name: still has a .git, so git should be listing it"
    continue
  fi

  if [ -n "$(find "$path" -maxdepth 2 -newermt "-${MIN_IDLE_MINUTES} minutes" -print -quit 2>/dev/null)" ]; then
    note "no-remove orphan $name: touched in the last ${MIN_IDLE_MINUTES}m"
    continue
  fi

  size_kb=$(du -sk "$path" 2>/dev/null | awk '{print $1}')
  [ -n "$size_kb" ] || size_kb=0

  if [ "$DRY_RUN" = 1 ]; then
    act "would remove orphan $path ($(human "$size_kb")) — not a worktree git knows"
    continue
  fi

  if detach_delete "$path"; then
    removed=$((removed + 1))
    freed_kb=$((freed_kb + size_kb))
    names="$names $(printf '%s' "$name" | tr -cd 'A-Za-z0-9._-')"
  fi
done

if [ "$DRY_RUN" = 1 ]; then
  # Report what is held rather than asserting why nothing happened: the reasons
  # differ per worktree and the per-line notes above already carry them.
  if [ "$WOULD_DO" = 0 ]; then
    note 'nothing eligible this run'
  else
    note "$WOULD_DO action(s) eligible; re-run without RECLAIM_DRY_RUN=1 to apply"
  fi
  if [ "$HELD_LOCKED_KB" -gt 0 ]; then
    note "  $(human "$HELD_LOCKED_KB") of cache is held by live-locked worktrees; it frees when those sessions exit"
  fi
  if [ "$HELD_RECENT_KB" -gt 0 ]; then
    note "  $(human "$HELD_RECENT_KB") was built inside the last ${PRUNE_IDLE_MINUTES}m (RECLAIM_PRUNE_IDLE_MINUTES lowers that)"
  fi
  note "  ${free_gb}G free now, low water ${FREE_LOW_WATER_GB}G"
  exit 0
fi

# Stay silent on a no-op; SessionStart runs on every single session.
if [ "$removed" -gt 0 ] || [ "$pruned" -gt 0 ]; then
  detail=""
  [ "$removed" -gt 0 ] && detail="$removed resolved worktree(s):$names"
  if [ "$pruned" -gt 0 ]; then
    [ -n "$detail" ] && detail="$detail, "
    detail="${detail}${pruned} idle build cache(s)"
  fi
  printf '{"systemMessage":"Reclaimed %s of disk from %s"}\n' \
    "$(human "$freed_kb")" "$detail"
fi

exit 0
