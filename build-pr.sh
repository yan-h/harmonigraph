#!/usr/bin/env bash
#
# build-pr.sh — bring a pushed branch, usually one a cloud session opened as a PR,
# into a local worktree and build it, so load-plugin.sh can offer it.
# A cloud session builds on a remote machine and hands back only the branch;
# load-plugin.sh only sees builds that sit in LOCAL worktrees. This is the step
# between the two.
#
# Usage:
#   ./build-pr.sh 1079              # a PR number
#   ./build-pr.sh 1079 1081 1083    # several, built one after another
#   ./build-pr.sh claude/some-branch
#   ./build-pr.sh --load 1079       # build, then load it (one target only)
#
# Rerunning it after the PR gets new commits fast-forwards the same worktree and
# rebuilds. It never discards anything: a worktree with uncommitted edits, or a
# branch that was force-pushed past its local commits, stops with instructions.
#
# The worktree is .claude/worktrees/<last branch segment>, a direct child of the
# directory `.claude/reclaim-worktrees.sh` manages, so it is cleaned up like a
# session's. It takes no lock (see CLAUDE.md on hand-written locks).
set -euo pipefail

LOAD=0
TARGETS=()
for arg in "$@"; do
  case "$arg" in
    --load) LOAD=1 ;;
    -h|--help) awk 'NR == 1 { next } !/^#/ { exit } { sub(/^# ?/, ""); print }' "$0"; exit 0 ;;
    *) TARGETS+=("$arg") ;;
  esac
done
(( ${#TARGETS[@]} )) || { echo "usage: $0 [--load] <pr-number|branch>..." >&2; exit 2; }
(( LOAD == 0 || ${#TARGETS[@]} == 1 )) || { echo "ERROR: --load takes one target; the slot holds one build" >&2; exit 2; }

MAIN="$(git worktree list --porcelain | awk '/^worktree /{print substr($0, 10); exit}')"

# Path of the registered worktree that has $1 checked out, or nothing.
worktree_of() {
  git worktree list --porcelain | awk -v want="branch refs/heads/$1" '
    /^worktree /{ path = substr($0, 10) }
    $0 == want { print path; exit }'
}

build_one() {
  local target="$1" branch
  if [[ "$target" =~ ^[0-9]+$ ]]; then
    local info
    info="$(gh pr view "$target" --json headRefName,isCrossRepository --jq '"\(.isCrossRepository) \(.headRefName)"')"
    [[ "${info%% *}" == "false" ]] || { echo "ERROR: PR #$target is from a fork; not supported" >&2; return 1; }
    branch="${info#* }"
  else
    branch="$target"
  fi

  echo "==> $branch"
  git -C "$MAIN" fetch --quiet origin "$branch"
  local remote="origin/$branch"

  local wt
  wt="$(worktree_of "$branch")"
  if [[ -z "$wt" ]]; then
    local name="${branch##*/}"
    wt="$MAIN/.claude/worktrees/$name"
    [[ ! -e "$wt" ]] || { echo "ERROR: $wt exists but does not hold $branch" >&2; return 1; }
    if git -C "$MAIN" show-ref --verify --quiet "refs/heads/$branch"; then
      git -C "$MAIN" worktree add --quiet "$wt" "$branch"
    else
      git -C "$MAIN" worktree add --quiet "$wt" -b "$branch" --track "$remote"
    fi
    echo "    new worktree $wt"
    # Seed the build with main's artifacts. `cp -c` is an APFS clone: ~3s and no
    # disk for 7G. sccache already serves the deps, but not build scripts, proc
    # macros or links; with this a fresh build went 1m26s -> 42s. Cargo's own
    # fingerprints decide what is stale, so a stale copy only costs rebuilds.
    if [[ -d "$MAIN/target/release" ]]; then
      mkdir -p "$wt/target"
      cp -c -R -p "$MAIN/target/release" "$wt/target/" || rm -rf "$wt/target/release"
      # Main's finished plugin and renderer came along too. A build that fails
      # or is interrupted would leave them for load-plugin.sh to list as this
      # branch's, untagged and "ok fresh" by mtime; cargo relinks both anyway.
      rm -f "$wt/target/release/libharmonigraph_plugin.dylib" "$wt/target/release/harmonigraph-offline"
    fi
  fi

  if [[ -n "$(git -C "$wt" status --porcelain --untracked-files=no)" ]]; then
    echo "ERROR: $wt has uncommitted edits; commit or discard them, then rerun" >&2
    return 1
  fi
  if git -C "$wt" merge-base --is-ancestor HEAD "$remote"; then
    git -C "$wt" merge --quiet --ff-only "$remote"
  elif ! git -C "$wt" merge-base --is-ancestor "$remote" HEAD; then
    echo "ERROR: $branch and $remote have diverged (a force-push, or local commits)." >&2
    echo "       Inspect with: git -C $wt log --oneline --left-right HEAD...$remote" >&2
    echo "       To take the remote: git -C $wt reset --hard $remote" >&2
    return 1
  fi
  echo "    at $(git -C "$wt" log -1 --format='%h %s')"

  if (( LOAD )); then
    ( cd "$wt" && ./update-plugin.sh )
  else
    ( cd "$wt" && cargo build --release -p harmonigraph-plugin -p harmonigraph-offline )
    echo "    loadable: ./load-plugin.sh $branch"
  fi
}

# Not `build_one "$t" || ...`: bash switches errexit off for everything under a
# `||`, so a failed fetch or cargo build would fall through to the next step.
# A plain subshell keeps errexit, and one target failing still leaves the rest.
failed=()
for t in "${TARGETS[@]}"; do
  set +e
  ( set -e; build_one "$t" )
  rc=$?
  set -e
  (( rc == 0 )) || failed+=("$t")
done
if (( ${#failed[@]} )); then
  echo "FAILED: ${failed[*]}" >&2
  exit 1
fi
