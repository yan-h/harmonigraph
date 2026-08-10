#!/usr/bin/env bash
# Does reclaim-worktrees.sh tell a live session's lock from an abandoned one?
#
# This is the one decision in that script that DELETES a directory, and it is
# the only one whose inputs live outside the repo — a pid, its argv, and the
# spare pool's socket files — so nothing else in the tree can catch it going
# wrong. It has gone wrong in both directions: too conservative (a returned
# spare kept its lock forever and 36.6G of cache with it), then too eager (a
# discriminator that matched every live session, so a paused session's whole
# worktree was removable out from under it).
#
# The script has no main guard and does real work at load, so these drive it
# end to end: a throwaway repo, a worktree locked with a reason naming a pid,
# a `ps` shim ahead of the real one on PATH, and DRY_RUN so every decision is
# printed rather than taken. RECLAIM_FORCE=1 skips the df gate — the reading
# on the test machine's disk is not part of what is under test.
#
#   .claude/tests/reclaim-locks.sh          # run it
#
# Written for bash 3.2 (macOS system bash), like the script it tests.
set -uo pipefail

SCRIPT=$(cd "$(dirname "$0")/../.." && pwd)/.claude/reclaim-worktrees.sh
[ -x "$SCRIPT" ] || { echo "✗ not executable: $SCRIPT" >&2; exit 1; }

TMP=$(mktemp -d "${TMPDIR:-/tmp}/reclaim-locks.XXXXXX") || exit 1
trap 'rm -rf "$TMP"' EXIT

failures=0

# One case: build a repo whose single worktree is locked by $pid, put a `ps`
# shim on PATH that gives that pid the cmdline $cmd, and assert the script's
# own note for that worktree matches $want.
#
# $sock is the claim-socket path the shim reports in the cmdline; it is
# created on disk only when $sock_exists is 1, which is the whole signal —
# see the script's own comment at the lock branch.
check() {
  desc=$1; cmd=$2; sock_exists=$3; want=$4

  work="$TMP/case$((++case_n))"
  mkdir -p "$work/main" "$work/bin"
  sock="$work/spare.claim.sock"
  [ "$sock_exists" = 1 ] && : > "$sock"

  # The shim answers the two forms the script uses: a liveness probe, and a
  # cmdline read. Anything else is not this script's business and exits 1.
  cat > "$work/bin/ps" <<SHIM
#!/usr/bin/env bash
case "\$*" in
  *"-o command="*) printf '%s\n' "${cmd//SOCK/$sock}" ;;
  *) exit 0 ;;
esac
SHIM
  chmod +x "$work/bin/ps"

  (
    cd "$work/main" || exit 1
    git init -q . 2>/dev/null
    git config user.email t@t; git config user.name t
    git commit -q --allow-empty -m base
    # The branch sits ON main's commit, so the ancestor test reads it as
    # merged and the run reaches the removal gate rather than stopping short.
    git worktree add -q -b w1 .claude/worktrees/w1 HEAD 2>/dev/null
    git worktree lock --reason "claude session w1 (pid 4242 start Mon Aug 10 02:00:00 2026)" \
      .claude/worktrees/w1 2>/dev/null
  ) || { echo "✗ $desc: could not build the fixture" >&2; failures=$((failures + 1)); return; }

  got=$(cd "$work/main" && PATH="$work/bin:$PATH" CLAUDE_PROJECT_DIR="$work/main" \
    RECLAIM_DRY_RUN=1 RECLAIM_FORCE=1 RECLAIM_NO_NETWORK=1 \
    "$SCRIPT" </dev/null 2>&1 >/dev/null | grep -c "$want")

  if [ "${got:-0}" -ge 1 ]; then
    echo "✓ $desc"
  else
    echo "✗ $desc" >&2
    echo "    expected a line matching: $want" >&2
    (cd "$work/main" && PATH="$work/bin:$PATH" CLAUDE_PROJECT_DIR="$work/main" \
      RECLAIM_DRY_RUN=1 RECLAIM_FORCE=1 RECLAIM_NO_NETWORK=1 \
      "$SCRIPT" </dev/null 2>&1 >/dev/null | sed 's/^/    got: /') >&2
    failures=$((failures + 1))
  fi
}

case_n=0

# A CLAIMED spare is a live session. Its argv is fixed at exec and still names
# the claim socket it was offered on, so argv alone cannot say it was taken —
# the socket being GONE from disk is what says so. This is the case that makes
# the difference between skipping a worktree and deleting one a session is
# paused in, and it is the reason a flag test is not enough.
check "a claimed spare's lock is live" \
  "claude bg-spare --bg-spare SOCK" 0 \
  "locked by live pid"

# An UNCLAIMED spare is still advertising itself on that socket, so the socket
# is on disk and the lock protects nothing.
check "an unclaimed spare's lock is stale" \
  "claude bg-spare --bg-spare SOCK" 1 \
  "stale lock"

# A process that is not a spare at all — a hand-run session, another tool —
# is taken at its word. Unrecognised means live, which is the safe direction.
check "an unrecognised holder's lock is live" \
  "some-other-program --with args" 0 \
  "locked by live pid"

echo
if [ "$failures" -gt 0 ]; then
  echo "✗ $failures lock case(s) failed" >&2
  exit 1
fi
echo "✅ reclaim-worktrees.sh reads every lock case correctly"
