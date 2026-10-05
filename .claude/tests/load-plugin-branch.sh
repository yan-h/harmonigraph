#!/usr/bin/env bash
# A full branch name selects that branch even when it is a substring of another.
# Retained handoffs keep `codex/foo-2` beside `codex/foo` after both worktrees
# are gone, and the full name is what a handover and update-plugin.sh pass.
# `--tag` resolves the argument through the same lookup as loading and needs
# no codesign, so this runs everywhere `ci.sh` does.
#
#   .claude/tests/load-plugin-branch.sh          # run it
#
# Written for bash 3.2 (macOS system bash), like the scripts it tests.
set -uo pipefail

for v in $(env | sed -n 's/^\(GIT_[A-Z_]*\)=.*/\1/p'); do
  unset "$v"
done

ROOT=$(cd "$(dirname "$0")/../.." && pwd)
TMP=$(mktemp -d "${TMPDIR:-/tmp}/load-plugin-branch.XXXXXX") || exit 1
trap 'rm -rf "$TMP"' EXIT

repo="$TMP/main"
git init -q -b main "$repo"
git -C "$repo" config user.email t@example.com
git -C "$repo" config user.name t
mkdir -p "$repo/.claude"
cp "$ROOT/load-plugin.sh" "$ROOT/session-lifecycle.sh" "$repo/"
cp "$ROOT/.claude/build-handoffs.sh" "$repo/.claude/"
git -C "$repo" add .
git -C "$repo" commit -q -m seed
sha=$(git -C "$repo" rev-parse --short HEAD)

for branch in codex/foo codex/foo-2; do
  wt="$TMP/${branch//\//-}"
  git -C "$repo" worktree add -q -b "$branch" "$wt" HEAD
  mkdir -p "$wt/target/release"
  printf 'x\0%s @%s\0' "$branch" "$sha" > "$wt/target/release/libharmonigraph_plugin.dylib"
done

failures=0
expect() {
  local query="$1" want="$2" got
  got=$(cd "$repo" && ./load-plugin.sh --tag "$query" 2>&1)
  if [ "$got" = "$want" ]; then
    echo "✓ --tag $query"
  else
    echo "✗ --tag $query: expected '$want', got '$got'" >&2
    failures=$((failures + 1))
  fi
}
expect codex/foo "codex/foo @$sha"
expect codex/foo-2 "codex/foo-2 @$sha"
expect foo-2 "codex/foo-2 @$sha"
expect foo "'foo' matches multiple branches:
  codex/foo
  codex/foo-2"

# Every detached worktree shares one label, so even an exact match refuses.
git -C "$repo" worktree add -q --detach "$TMP/detached-a" HEAD
git -C "$repo" worktree add -q --detach "$TMP/detached-b" HEAD
expect "(detached)" "'(detached)' matches multiple branches:
  (detached)
  (detached)"

# A handoff record the loader cannot read prints '?' rather than ending the
# loader partway through a load, whether or not the caller sets pipefail.
mkdir -p "$TMP/unreadable" && echo '{}' > "$TMP/unreadable/handoff.json"
got=$(set -eu; . "$ROOT/.claude/build-handoffs.sh"; short="$(build_short "$TMP/unreadable")"; echo "$short")
if [ "$got" = "?" ]; then
  echo "✓ unreadable handoff reads '?'"
else
  echo "✗ unreadable handoff: expected '?', got '$got'" >&2
  failures=$((failures + 1))
fi

exit "$failures"
