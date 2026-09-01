#!/usr/bin/env bash
# Does reclaim-worktrees.sh keep its ownership boundary and tell a live
# session's lock from an abandoned one?
#
# These are the decisions in that script that can DELETE a directory: whether
# the worktree belongs to Claude, and whether a Claude lock is live. Their
# inputs live outside the repo — a path, a pid, its argv, and the spare pool's
# socket files — so nothing else in the tree can catch them going wrong. The
# lock has gone wrong in both directions: too conservative (a returned spare
# kept its lock forever and 36.6G of cache with it), then too eager (a
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

# git exports GIT_DIR, GIT_INDEX_FILE and the rest to its hooks, and ci.sh runs
# from .githooks/pre-push — so inherited, they point every git call below at the
# repo being PUSHED instead of the throwaway one. That is not a near miss:
# `git init` with GIT_DIR set and no work tree sets `core.bare=true` on the real
# repo, `git commit` lands on the real branch, and `git worktree add` registers
# a worktree in a temp directory this script then deletes.
for v in $(env | sed -n 's/^\(GIT_[A-Z_]*\)=.*/\1/p'); do
  unset "$v"
done

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
    # Belt and braces over the unset above, and the guard that matters: prove
    # git resolves INSIDE this throwaway repo before committing to it or
    # adding worktrees to it. Both paths are physical (`pwd -P` and
    # `--absolute-git-dir`), which is what makes the prefix test hold under
    # macOS's /var -> /private/var symlink.
    real=$(pwd -P)
    here=$(git rev-parse --absolute-git-dir 2>/dev/null)
    case "$here" in
      "$real"/*) ;;
      *) echo "refusing: git resolves to ${here:-nothing}, not $real" >&2; exit 1 ;;
    esac
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

# A Codex Worktree root is configurable. If it is placed below
# `.claude/worktrees`, the app still creates its own `<id>/<repo>` level below
# that root; a prefix-only ownership check mistakes the nested worktree for a
# Claude one and removes it. Run the reclaimer for real in a throwaway repo and
# prove both the worktree and its prunable cache survive.
check_codex_ownership() {
  work="$TMP/ownership"
  main="$work/main"
  codex_wt="$main/.claude/worktrees/codex-id/harmonigraph"
  claude_wt="$main/.claude/worktrees/claude-w"
  mkdir -p "$main" "$(dirname "$codex_wt")"

  (
    cd "$main" || exit 1
    git init -q . 2>/dev/null || exit 1
    # This case runs the reclaimer without DRY_RUN, so prove every git command
    # resolves inside the disposable repo before creating removable worktrees.
    real=$(pwd -P)
    here=$(git rev-parse --absolute-git-dir 2>/dev/null)
    case "$here" in
      "$real"/*) ;;
      *) echo "refusing: git resolves to ${here:-nothing}, not $real" >&2; exit 1 ;;
    esac
    git checkout -q -b main 2>/dev/null || true
    git config user.email t@t; git config user.name t
    printf '/.claude/worktrees/\n/target/\n' > .gitignore
    git add .gitignore
    git commit -q -m base || exit 1
    git worktree add -q -b codex/app "$codex_wt" HEAD 2>/dev/null || exit 1
    git worktree add -q -b claude/w "$claude_wt" HEAD 2>/dev/null || exit 1
    mkdir -p "$codex_wt/target/debug" "$claude_wt/target/debug" || exit 1
    : > "$codex_wt/target/debug/sentinel"
    : > "$claude_wt/target/debug/sentinel"
    # The direct child is the positive control: make it unambiguously idle so
    # both zero-minute cutoffs reach the removal decision on every `find`.
    find "$claude_wt" -depth -exec touch -t 200001010000 {} \; || exit 1
  ) || {
    echo "✗ a Codex-managed worktree: could not build the fixture" >&2
    failures=$((failures + 1))
    return
  }

  if [ ! -e "$codex_wt/.git" ] || [ ! -e "$claude_wt/.git" ]; then
    echo "✗ a Codex-managed worktree: fixture worktrees are not registered" >&2
    failures=$((failures + 1))
    return
  fi

  out="$work/run.log"
  (cd "$main" && CLAUDE_PROJECT_DIR="$main" RECLAIM_FORCE=1 \
    RECLAIM_NO_NETWORK=1 RECLAIM_MIN_IDLE_MINUTES=0 \
    RECLAIM_PRUNE_IDLE_MINUTES=0 "$SCRIPT" </dev/null) >"$out" 2>&1
  status=$?

  if [ "$status" -eq 0 ] && [ -f "$codex_wt/target/debug/sentinel" ] &&
    [ ! -d "$claude_wt" ]; then
    echo "✓ a Codex-managed worktree stays app-owned"
  else
    echo "✗ the reclaimer did not distinguish Codex and Claude ownership" >&2
    sed 's/^/    /' "$out" >&2
    failures=$((failures + 1))
  fi
}

# A lock this script did not write names nobody it can check, so it is left
# alone whatever its text happens to contain. The reason below carries a dead
# pid in prose, which is the shape that reads as attributable without being so:
# only a lock in the harness's own format says which session is holding it.
#
# CLAUDE.md sends people here — it tells a session that a hand-written lock
# stands until a human clears it, and a hand-locked worktree that the reclaimer
# deletes at the next SessionStart is that promise broken with work inside it.
check_handmade_lock() {
  desc="a hand-written lock is left for a human"
  work="$TMP/handmade"
  mkdir -p "$work/main" "$work/bin"

  # The pid is dead: the liveness probe fails, so nothing but the reason's own
  # shape stands between this worktree and the removal gate.
  printf '#!/usr/bin/env bash\nexit 1\n' > "$work/bin/ps"
  chmod +x "$work/bin/ps"

  (
    cd "$work/main" || exit 1
    git init -q . 2>/dev/null
    real=$(pwd -P)
    here=$(git rev-parse --absolute-git-dir 2>/dev/null)
    case "$here" in
      "$real"/*) ;;
      *) echo "refusing: git resolves to ${here:-nothing}, not $real" >&2; exit 1 ;;
    esac
    git config user.email t@t; git config user.name t
    git commit -q --allow-empty -m base
    git worktree add -q -b w1 .claude/worktrees/w1 HEAD 2>/dev/null
    git worktree lock --reason "held by hand for pid 4242, mid-investigation" \
      .claude/worktrees/w1 2>/dev/null
  ) || { echo "✗ $desc: could not build the fixture" >&2; failures=$((failures + 1)); return; }

  # A worktree made a moment ago is held back by the idle guard, which sits
  # AFTER the lock check and would carry this case on a reason that has nothing
  # to do with the lock. Backdating it puts the lock on its own.
  find "$work/main/.claude/worktrees/w1" -maxdepth 2 -exec touch -t 202001010000 {} + 2>/dev/null
  touch -t 202001010000 "$work/main/.claude/worktrees/w1"

  out=$(cd "$work/main" && PATH="$work/bin:$PATH" CLAUDE_PROJECT_DIR="$work/main" \
    RECLAIM_DRY_RUN=1 RECLAIM_FORCE=1 RECLAIM_NO_NETWORK=1 \
    "$SCRIPT" </dev/null 2>&1)

  if printf '%s' "$out" | grep -q "would remove"; then
    echo "✗ $desc" >&2
    printf '%s' "$out" | sed 's/^/    got: /' >&2
    failures=$((failures + 1))
  else
    echo "✓ $desc"
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

check_codex_ownership
check_handmade_lock

echo
if [ "$failures" -gt 0 ]; then
  echo "✗ $failures lock case(s) failed" >&2
  exit 1
fi
echo "✅ reclaim-worktrees.sh keeps ownership and lock boundaries"
