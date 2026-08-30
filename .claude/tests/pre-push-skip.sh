#!/usr/bin/env bash
# The pre-push hook decides whether a push has to pay for ci.sh, and a wrong
# answer is silent in the direction that matters: a skipped run reports a clean
# push and gates nothing. Its inputs are git's stdin protocol and a stamp file,
# both outside cargo's reach, so this is a gate rather than a habit.
#
# Each case builds a scratch repo whose `ci.sh` is a stub that leaves a marker,
# so "did the gate run" is a file test rather than a build.
set -euo pipefail

# git exports GIT_DIR, GIT_INDEX_FILE and the rest to its hooks, and ci.sh runs
# from .githooks/pre-push — inherited, they aim every git call below at the repo
# being PUSHED rather than the scratch one, and `git -C` does not override them.
# What that costs is not a failed test: the commits land on the real branch, the
# real index is replaced by the scratch one, and `update-ref origin/main` moves a
# ref every other worktree shares. The same unset guards reclaim-locks.sh and
# plugin-swap.sh, for the same reason.
for v in $(env | sed -n 's/^\(GIT_[A-Z_]*\)=.*/\1/p'); do
  unset "$v"
done

HOOK=$(cd "$(dirname "$0")/../.." && pwd)/.githooks/pre-push
tmp=$(mktemp -d)
trap 'rm -rf "$tmp"' EXIT

fail=0
ZERO=0000000000000000000000000000000000000000

# A scratch repo with one commit holding both a .rs and a .md, an `origin/main`
# to serve as the base for a new branch, and a stub ci.sh.
new_repo() {
  rm -rf "$tmp/r" && mkdir -p "$tmp/r"
  git -C "$tmp/r" init -q
  # Belt and braces over the unset above, and the guard that matters: prove git
  # resolves INSIDE the scratch repo before anything commits to it. Both paths
  # are physical, which is what makes the prefix test hold under macOS's
  # /var -> /private/var symlink.
  real=$(cd "$tmp/r" && pwd -P)
  here=$(git -C "$tmp/r" rev-parse --absolute-git-dir 2>/dev/null)
  case "$here" in
    "$real"/*) ;;
    *)
      echo "refusing: git resolves to ${here:-nothing}, not $real" >&2
      exit 1
      ;;
  esac
  git -C "$tmp/r" config user.email t@t && git -C "$tmp/r" config user.name t
  mkdir -p "$tmp/r/crates"
  echo "fn main() {}" >"$tmp/r/crates/a.rs"
  echo "prose" >"$tmp/r/README.md"
  printf '#!/bin/sh\ntouch "$(dirname "$0")/CI_RAN"\n' >"$tmp/r/ci.sh"
  chmod +x "$tmp/r/ci.sh"
  git -C "$tmp/r" add -A && git -C "$tmp/r" commit -qm base
  git -C "$tmp/r" update-ref refs/remotes/origin/main HEAD
}

# Feed one ref line to the hook, and report both whether the stub fired and
# whether the hook let the push through. Both halves are needed: a hook that
# dies also leaves no marker, so "ran=no" alone cannot tell a deliberate skip
# from a crash — and the crash blocks the push it was supposed to wave past.
ran() {
  rm -f "$tmp/r/CI_RAN"
  echo "$1" | (cd "$tmp/r" && "$HOOK" >/dev/null 2>&1)
  rc=$?
  [ -f "$tmp/r/CI_RAN" ]
}

check() { # check <expect ran: yes|no> <label> <ref line>
  local want=$1 label=$2 line=$3 got=no
  ran "$line" && got=yes
  if [ "$got" != "$want" ]; then
    echo "✗ $label — ci.sh ran=$got, expected ran=$want" >&2
    fail=1
  elif [ "$want" = no ] && [ "$rc" != 0 ]; then
    echo "✗ $label — the gate was skipped by exiting $rc, which rejects the push" >&2
    fail=1
  else
    echo "✓ $label"
  fi
}

# Prints the new sha, and refuses to return one for a commit that changed
# nothing: an empty range skips the gate for a reason no case here is testing,
# which would read as a pass and measure nothing.
commit_change() { # commit_change <path> <content> -> prints new sha
  mkdir -p "$(dirname "$tmp/r/$1")"
  echo "$2" >"$tmp/r/$1"
  git -C "$tmp/r" add -A
  git -C "$tmp/r" commit -qm "change $1" >/dev/null || {
    echo "fixture did not change $1" >&2
    exit 1
  }
  git -C "$tmp/r" rev-parse HEAD
}

# A deletion carries an all-zero local sha and sends no objects.
new_repo
head=$(git -C "$tmp/r" rev-parse HEAD)
check no "a branch deletion does not run the gate" \
  "(delete) $ZERO refs/heads/gone $head"

# Prose alone cannot reach any check ci.sh makes.
new_repo
base=$(git -C "$tmp/r" rev-parse HEAD)
sha=$(commit_change README.md "more prose")
check no "a prose-only range does not run the gate" \
  "refs/heads/b $sha refs/heads/b $base"

# ...and code in the same range brings it straight back.
new_repo
base=$(git -C "$tmp/r" rev-parse HEAD)
sha=$(commit_change crates/a.rs "fn main() { let _ = 1; }")
check yes "a code change runs the gate" \
  "refs/heads/b $sha refs/heads/b $base"

new_repo
base=$(git -C "$tmp/r" rev-parse HEAD)
commit_change README.md "more prose" >/dev/null
sha=$(commit_change crates/a.rs "fn main() { let _ = 2; }")
check yes "code alongside prose runs the gate" \
  "refs/heads/b $sha refs/heads/b $base"

# A new branch has no remote counterpart, so the base is what it adds to main.
new_repo
sha=$(commit_change README.md "prose the base does not have")
check no "a new prose-only branch measures against origin/main" \
  "refs/heads/b $sha refs/heads/b $ZERO"

# `docs/` needs its own case rather than a `.md` under it: a markdown file
# there is already covered by the extension, so a `docs/*.md` fixture leaves
# the arm that carries the screenshots unexecuted.
new_repo
base=$(git -C "$tmp/r" rev-parse HEAD)
sha=$(commit_change docs/images/shot.png "not really a png")
check no "a non-markdown file under docs/ does not run the gate" \
  "refs/heads/b $sha refs/heads/b $base"

# With no base to measure against, the gate runs rather than guessing.
new_repo
git -C "$tmp/r" update-ref -d refs/remotes/origin/main
sha=$(commit_change README.md "more prose")
check yes "a new branch with no origin/main runs the gate" \
  "refs/heads/b $sha refs/heads/b $ZERO"

# A tree ci.sh already cleared is not asked about twice, even for code.
new_repo
base=$(git -C "$tmp/r" rev-parse HEAD)
sha=$(commit_change crates/a.rs "fn main() { let _ = 3; }")
git -C "$tmp/r" rev-parse "$sha^{tree}" >"$tmp/r/.git/ci-passed-trees"
check no "a tree already recorded as passing does not run the gate" \
  "refs/heads/b $sha refs/heads/b $base"

# The stamp names a TREE, so a different tree with a stamped neighbour still pays.
new_repo
base=$(git -C "$tmp/r" rev-parse HEAD)
git -C "$tmp/r" rev-parse "HEAD^{tree}" >"$tmp/r/.git/ci-passed-trees"
sha=$(commit_change crates/a.rs "fn main() { let _ = 4; }")
check yes "a stamped ancestor does not clear its descendant" \
  "refs/heads/b $sha refs/heads/b $base"

# Every case above invokes the hook the way a person tests it: from a repo
# root, with no GIT_DIR. Git runs it the other way — cwd in the pushing
# worktree, GIT_DIR exported — and almost every push here comes from a linked
# worktree, whose GIT_DIR is not the common dir the stamp lives in. This is the
# shape that ships, so it is the shape worth pinning.
new_repo
base=$(git -C "$tmp/r" rev-parse HEAD)
git -C "$tmp/r" worktree add -q "$tmp/wt" -b wt HEAD
echo "fn main() { let _ = 9; }" >"$tmp/wt/crates/a.rs"
git -C "$tmp/wt" add -A
git -C "$tmp/wt" commit -qm "code, in a linked worktree"
sha=$(git -C "$tmp/wt" rev-parse HEAD)
git -C "$tmp/wt" rev-parse "$sha^{tree}" >"$tmp/r/.git/ci-passed-trees"

rm -f "$tmp/wt/CI_RAN"
gitdir=$(git -C "$tmp/wt" rev-parse --absolute-git-dir)
echo "refs/heads/wt $sha refs/heads/wt $base" |
  (cd "$tmp/wt" && GIT_DIR=$gitdir "$HOOK" >/dev/null 2>&1)
rc=$?
if [ -f "$tmp/wt/CI_RAN" ]; then
  echo "✗ a stamped tree pushed from a linked worktree still ran the gate" >&2
  fail=1
elif [ "$rc" != 0 ]; then
  echo "✗ the linked-worktree push was rejected with exit $rc" >&2
  fail=1
else
  echo "✓ the stamp is found from a linked worktree, with GIT_DIR set"
fi

echo
if [ "$fail" = 0 ]; then
  echo "✅ pre-push skips only pushes that cannot change ci.sh's answer"
else
  echo "❌ pre-push gate is wrong" >&2
  exit 1
fi
