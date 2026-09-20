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

# This test creates and enters throwaway repositories, so inherited GIT_DIR,
# GIT_INDEX_FILE and related variables would point every git call below at the
# caller's repo instead. That is not a near miss:
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

  if grep -q "would remove" <<<"$out"; then
    echo "✗ $desc" >&2
    printf '%s' "$out" | sed 's/^/    got: /' >&2
    failures=$((failures + 1))
  else
    echo "✓ $desc"
  fi
}

# Containment is the widest removal signal, so it is the one that has to be
# proved in BOTH directions from a single fixture: the two shapes equality
# missed, and a branch that is genuinely still in flight. A test that only
# showed the positives would pass just as happily against a rubber stamp — and
# `--ignore-missing` makes a rubber stamp the natural failure here, since it
# drops an unreadable rev rather than complaining.
#
# The repo below puts main at A and hangs three branches off it:
#   closed-w  at B, and a CLOSED PR whose head is B      -> resolved
#   behind-w  at B, and a MERGED PR whose head is C (B's child, not in main)
#                                                        -> resolved, and the
#                                                           case sha equality
#                                                           reads as unmerged
#   live-w    at D, named by no PR at all                -> KEPT
# None of the three is an ancestor of main, so every one of them reaches the
# gh-backed signals rather than stopping at the offline test.
check_containment() {
  work="$TMP/containment"
  main="$work/main"
  mkdir -p "$main" "$work/bin"

  shas="$work/shas"
  (
    cd "$main" || exit 1
    git init -q . 2>/dev/null || exit 1
    real=$(pwd -P)
    here=$(git rev-parse --absolute-git-dir 2>/dev/null)
    case "$here" in
      "$real"/*) ;;
      *) echo "refusing: git resolves to ${here:-nothing}, not $real" >&2; exit 1 ;;
    esac
    git checkout -q -b main 2>/dev/null || true
    git config user.email t@t; git config user.name t
    printf '/.claude/worktrees/\n' > .gitignore
    git add .gitignore
    git commit -q -m base || exit 1

    # B, then C on top of it. C is the sha that "merged"; the worktree stays
    # at B, which is exactly the drift that left five worktrees standing.
    git checkout -q -b feature 2>/dev/null || exit 1
    git commit -q --allow-empty -m B || exit 1
    b=$(git rev-parse HEAD)
    git commit -q --allow-empty -m C || exit 1
    c=$(git rev-parse HEAD)

    git checkout -q main 2>/dev/null || exit 1
    git branch -q -f closed-b "$b" || exit 1
    git branch -q -f behind-b "$b" || exit 1

    git checkout -q -b unresolved 2>/dev/null || exit 1
    git commit -q --allow-empty -m D || exit 1
    git checkout -q main 2>/dev/null || exit 1

    git worktree add -q .claude/worktrees/closed-w closed-b 2>/dev/null || exit 1
    git worktree add -q .claude/worktrees/behind-w behind-b 2>/dev/null || exit 1
    git worktree add -q .claude/worktrees/live-w unresolved 2>/dev/null || exit 1
    printf 'CLOSED %s\nMERGED %s\n' "$b" "$c" > "$shas"
  ) || { echo "✗ containment: could not build the fixture" >&2; failures=$((failures + 1)); return; }

  # The script asks gh for `--json state,headRefOid --jq ...`; the shim stands
  # in for the whole call and emits what that jq would have produced.
  cat > "$work/bin/gh" <<SHIM
#!/usr/bin/env bash
cat "$shas"
SHIM
  chmod +x "$work/bin/gh"

  # The idle guard sits after the resolved-signal check and would otherwise
  # hold all three back on a reason that is not what this case is about.
  find "$main/.claude/worktrees" -depth -exec touch -t 200001010000 {} \; 2>/dev/null

  out=$(cd "$main" && PATH="$work/bin:$PATH" CLAUDE_PROJECT_DIR="$main" \
    RECLAIM_DRY_RUN=1 RECLAIM_FORCE=1 RECLAIM_MIN_IDLE_MINUTES=0 \
    "$SCRIPT" </dev/null 2>&1)

  # Herestrings, not `printf | grep -q`, for the reason the script itself
  # spells out at its REGISTERED check: under `set -o pipefail` a matching
  # `grep -q` exits while printf is still writing, printf takes SIGPIPE, and
  # the pipeline reports 141 even though the match succeeded — so the assertion
  # inverts. Every guard below is a matcher over captured output, so this is
  # the shape all of them take. It was not hypothetical here: this loop failed
  # on `behind-w` with `printf: write error: Broken pipe` while its own failure
  # dump printed the line it had just been told was missing.
  for case in "closed-w:a PR closed unmerged" "behind-w:a branch behind the sha that merged"; do
    wt=${case%%:*}
    if grep -q "would remove .*$wt" <<<"$out"; then
      echo "✓ ${case#*:} is resolved by containment"
    else
      echo "✗ containment missed ${case#*:} ($wt)" >&2
      printf '%s\n' "$out" | sed 's/^/    /' >&2
      failures=$((failures + 1))
    fi
  done

  # The one that must survive. Asserting on "no-remove" rather than on the
  # absence of "would remove" keeps a fixture that never reached the decision
  # from passing as a success.
  if grep -q "no-remove live-w: unresolved" <<<"$out" &&
    ! grep -q "would remove .*live-w" <<<"$out"; then
    echo "✓ a branch in no PR is still kept"
  else
    echo "✗ containment removed a branch that no PR resolves" >&2
    printf '%s\n' "$out" | sed 's/^/    /' >&2
    failures=$((failures + 1))
  fi
}

# A directory git no longer lists is reachable by neither tier above — they
# both walk `git worktree list` — so it is not skipped for a reason, it is
# never examined. Six were on disk on 2026-09-07 and no dry-run line mentioned
# them. The fixture is a plain directory: no `.git`, nothing registered.
#
# BOTH halves are asserted, and the second one carries the history: a
# reviewed-away version of this tier deleted what it found, and "git does not
# list it" turned out to be the wrong aim for an `rm -rf` in three separate
# ways (see ORPHANS in the script). So "it is named" is paired with "it is
# still there", and the pairing is what stops a future change from quietly
# turning a report back into a deletion.
check_orphan_report() {
  desc="an unregistered directory is named, not removed"
  work="$TMP/orphan"
  main="$work/main"
  orphan="$main/.claude/worktrees/left-behind"
  mkdir -p "$main"

  (
    cd "$main" || exit 1
    git init -q . 2>/dev/null || exit 1
    real=$(pwd -P)
    here=$(git rev-parse --absolute-git-dir 2>/dev/null)
    case "$here" in
      "$real"/*) ;;
      *) echo "refusing: git resolves to ${here:-nothing}, not $real" >&2; exit 1 ;;
    esac
    git checkout -q -b main 2>/dev/null || true
    git config user.email t@t; git config user.name t
    printf '/.claude/worktrees/\n' > .gitignore
    git add .gitignore
    git commit -q -m base || exit 1
    mkdir -p "$orphan/target/release" || exit 1
    : > "$orphan/target/release/sentinel"
  ) || { echo "✗ $desc: could not build the fixture" >&2; failures=$((failures + 1)); return; }

  if git -C "$main" worktree list --porcelain | grep -q "left-behind"; then
    echo "✗ $desc: fixture is registered, so it is not an orphan" >&2
    failures=$((failures + 1))
    return
  fi

  find "$orphan" -depth -exec touch -t 200001010000 {} \; 2>/dev/null
  touch -t 200001010000 "$orphan"

  out=$(cd "$main" && CLAUDE_PROJECT_DIR="$main" RECLAIM_FORCE=1 \
    RECLAIM_NO_NETWORK=1 RECLAIM_MIN_IDLE_MINUTES=0 \
    "$SCRIPT" </dev/null 2>&1)

  # A detached `rm -rf` would land after the script exits, so give one a chance
  # to run before concluding the directory survived.
  n=0
  while [ -d "$orphan" ] && [ "$n" -lt 20 ]; do sleep 0.1; n=$((n + 1)); done

  if [ ! -d "$orphan" ] || [ ! -f "$orphan/target/release/sentinel" ]; then
    echo "✗ $desc: the orphan was REMOVED; this tier must only report" >&2
    printf '%s\n' "$out" | sed 's/^/    /' >&2
    failures=$((failures + 1))
    return
  fi

  if grep -q "left-behind" <<<"$out"; then
    echo "✓ $desc"
  else
    echo "✗ $desc: survived but was never named, so it stays invisible" >&2
    printf '%s\n' "$out" | sed 's/^/    /' >&2
    failures=$((failures + 1))
  fi
}

# A throwaway superproject with a real submodule at `.shared-skills` and one
# registered worktree under `.claude/worktrees` whose submodule is CHECKED OUT,
# sitting on main's commit so the ancestor signal resolves it. Sets $main and
# $wt for the caller.
#
# The checkout is the whole fixture. `git worktree remove` refuses on a
# POPULATED submodule; an uninitialised gitlink removes plainly, which is both
# why tier 2 ever worked and why a fixture that skipped `submodule update`
# would pass against the unfixed script — the shape #450 warns about. The
# assertion at the bottom is there to keep it honest.
build_submodule_fixture() {
  work=$1
  main="$work/main"
  sub="$work/sub"
  wt="$main/.claude/worktrees/w1"
  mkdir -p "$main" "$sub" "$work/bin" || return 1

  (
    cd "$sub" || exit 1
    git init -q . 2>/dev/null || exit 1
    git checkout -q -b main 2>/dev/null || true
    git config user.email t@t; git config user.name t
    : > skill.md
    git add skill.md || exit 1
    git commit -q -m base || exit 1
  ) || return 1

  (
    cd "$main" || exit 1
    git init -q . 2>/dev/null || exit 1
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
    # file:// submodules are refused by default since git 2.38, and a local
    # path is the only kind a hermetic test can have.
    git -c protocol.file.allow=always submodule add -q "$sub" .shared-skills || exit 1
    git commit -q -m submodule || exit 1
    git worktree add -q -b w1 .claude/worktrees/w1 HEAD 2>/dev/null || exit 1
    git -C .claude/worktrees/w1 -c protocol.file.allow=always \
      submodule update --init --quiet || exit 1
    mkdir -p .claude/worktrees/w1/target/debug || exit 1
    : > .claude/worktrees/w1/target/debug/sentinel
  ) || return 1

  # ` <sha> .shared-skills (heads/main)` is populated; `-<sha>` is a gitlink
  # nobody checked out, and git removes that worktree without complaint.
  case "$(git -C "$wt" submodule status 2>/dev/null)" in
    " "*) return 0 ;;
    *) echo "fixture: .shared-skills is not populated in the worktree" >&2; return 1 ;;
  esac
}

# The removal that silently did not happen. git refuses a worktree containing a
# submodule, the script discarded that stderr and returned 1, so a dry run
# promising three removals was followed by a real run that took none of them and
# said nothing (#898). Older worktrees still hold `.shared-skills`, so keep
# the `--force` retry covered after removing the pin from new checkouts.
check_submodule_removal() {
  desc="a worktree with a populated submodule is removed"
  work="$TMP/submodule"
  build_submodule_fixture "$work" || {
    echo "✗ $desc: could not build the fixture" >&2
    failures=$((failures + 1))
    return
  }

  # The idle guard sits after the removal decision's other gates and would hold
  # a just-built worktree back for a reason that is not what this case is about.
  find "$wt" -depth -exec touch -t 200001010000 {} \; 2>/dev/null
  touch -t 200001010000 "$wt"

  out=$(cd "$main" && CLAUDE_PROJECT_DIR="$main" RECLAIM_FORCE=1 \
    RECLAIM_NO_NETWORK=1 RECLAIM_MIN_IDLE_MINUTES=0 \
    "$SCRIPT" </dev/null 2>&1)

  registered=$(git -C "$main" worktree list --porcelain)
  if [ -d "$wt" ] || grep -q "worktrees/w1" <<<"$registered"; then
    echo "✗ $desc: it survived, which is #898 exactly" >&2
    printf '%s\n' "$out" | sed 's/^/    /' >&2
    failures=$((failures + 1))
    return
  fi

  # --force is aimed at one worktree, but a submodule is shared-looking state:
  # prove the main checkout still has its own copy.
  if [ ! -f "$main/.shared-skills/skill.md" ]; then
    echo "✗ $desc: the main checkout's submodule went with it" >&2
    failures=$((failures + 1))
    return
  fi
  echo "✓ $desc"
}

# `--force` gives up git's own refusal on a dirty tree, so every clean check
# in this script now carries weight it did not carry before. What this case
# measures is the narrow half of that: whether uncommitted work INSIDE the
# submodule is visible to the superproject at all. It is not obvious that it
# is — the file is in another repository — and the answer is that git reports
# it as a modified gitlink, ` M .shared-skills`. Every gate downstream reads
# that line, so if it ever stopped appearing, nothing else here would notice.
#
# This case stops at the FIRST clean check, which returns before any removal
# is attempted; it therefore says nothing about the retry's own recheck, and
# it passes against the pre-`--force` script for that reason.
# `check_force_retry_gate` is the one that reaches the retry.
check_submodule_dirty_is_kept() {
  desc="a dirty submodule is visible to the first clean gate"
  work="$TMP/submodule-dirty"
  build_submodule_fixture "$work" || {
    echo "✗ $desc: could not build the fixture" >&2
    failures=$((failures + 1))
    return
  }

  # BEFORE the backdate, deliberately: at maxdepth 2 this file is also what the
  # idle guard sees, and a fixture that tripped that guard instead would be
  # kept for a reason that has nothing to do with being dirty.
  : > "$wt/.shared-skills/scratch.md"
  find "$wt" -depth -exec touch -t 200001010000 {} \; 2>/dev/null
  touch -t 200001010000 "$wt"

  dry=$(cd "$main" && CLAUDE_PROJECT_DIR="$main" RECLAIM_DRY_RUN=1 \
    RECLAIM_FORCE=1 RECLAIM_NO_NETWORK=1 RECLAIM_MIN_IDLE_MINUTES=0 \
    "$SCRIPT" </dev/null 2>&1)
  (cd "$main" && CLAUDE_PROJECT_DIR="$main" RECLAIM_FORCE=1 \
    RECLAIM_NO_NETWORK=1 RECLAIM_MIN_IDLE_MINUTES=0 \
    "$SCRIPT" </dev/null) >/dev/null 2>&1

  # Both halves, as in the orphan tier: that the clean check is the gate that
  # names the reason, and that the file is still there afterwards. Survival
  # alone would pass just as happily on a fixture that never reached the gate.
  #
  # The COUNT is pinned as well, because this fixture dirties exactly one
  # thing and the superproject reports it as exactly one porcelain line
  # (` M .shared-skills`) — which is the one width at which the off-by-one of
  # #900 was visible as a contradiction, "kept for 0 file(s)".
  if grep -q "no-remove w1: 1 uncommitted/untracked" <<<"$dry" &&
    [ -f "$wt/.shared-skills/scratch.md" ]; then
    echo "✓ $desc"
  else
    echo "✗ $desc: submodule dirt did not reach the superproject's porcelain" >&2
    printf '%s\n' "$dry" | sed 's/^/    dry: /' >&2
    [ -f "$wt/.shared-skills/scratch.md" ] || echo "    the scratch file is GONE" >&2
    failures=$((failures + 1))
  fi
}

# THE RETRY'S OWN GATE, which is where the change puts its weight: `--force`
# discards an unclean tree, so between the plain remove failing and the retry
# firing, the script re-asks whether the tree is still clean. That window is
# the reason the recheck exists — `du` of a multi-gigabyte worktree sits in it.
#
# No fixture reaches this gate on its own: the FIRST clean check returns long
# before, so a tree that starts dirty never gets here. The shim is what makes
# it reachable — it refuses the plain remove the way git's submodule refusal
# does, and changes the world on its way out:
#
#   dirty    the worktree acquires an untracked file as the remove is refused.
#   unknown  `git status` itself fails on the recheck. Its output is empty
#            either way, so an empty-output-is-clean test cannot tell "clean"
#            from "could not tell" — and would force on the second one.
#
# The assertion is that `--force` was never ATTEMPTED, not merely that the
# worktree survived: the shim refuses `--force` too, so survival alone would
# pass just as happily on a script that forced a dirty tree and was told no.
check_force_retry_gate() {
  mode=$1
  desc=$2
  work="$TMP/retry-$mode"
  build_submodule_fixture "$work" || {
    echo "✗ $desc: could not build the fixture" >&2
    failures=$((failures + 1))
    return
  }

  real_git=$(command -v git)
  forced="$work/forced"
  marker="$work/refused-once"

  # The --force arm is FIRST: `git -C <root> worktree remove --force <path>`
  # matches the plain pattern too, and a case takes the first match.
  cat > "$work/bin/git" <<SHIM
#!/usr/bin/env bash
case "\$*" in
  *"worktree remove --force"*)
    : > "$forced"
    echo 'fatal: the retry should not have run' >&2
    exit 128 ;;
  *"worktree remove"*)
    : > "$marker"
    [ "$mode" = dirty ] && : > "$wt/scratch.md"
    echo 'fatal: working trees containing submodules cannot be moved or removed' >&2
    exit 128 ;;
  *"status --porcelain"*)
    # Only the RECHECK, so the first clean check still answers honestly and
    # the run gets as far as attempting a removal.
    if [ "$mode" = unknown ] && [ -e "$marker" ]; then
      exit 1
    fi ;;
esac
exec "$real_git" "\$@"
SHIM
  chmod +x "$work/bin/git"

  find "$wt" -depth -exec touch -t 200001010000 {} \; 2>/dev/null
  touch -t 200001010000 "$wt"

  msg=$(cd "$main" && PATH="$work/bin:$PATH" CLAUDE_PROJECT_DIR="$main" \
    RECLAIM_FORCE=1 RECLAIM_NO_NETWORK=1 RECLAIM_MIN_IDLE_MINUTES=0 \
    RECLAIM_PRUNE_IDLE_MINUTES=0 "$SCRIPT" </dev/null 2>/dev/null)

  if [ ! -e "$marker" ]; then
    echo "✗ $desc: the run never attempted a removal, so the gate never ran" >&2
    printf '%s\n' "${msg:-(no output at all)}" | sed 's/^/    /' >&2
    failures=$((failures + 1))
    return
  fi
  if [ -e "$forced" ]; then
    echo "✗ $desc: --force ran anyway" >&2
    printf '%s\n' "${msg:-(no output at all)}" | sed 's/^/    /' >&2
    failures=$((failures + 1))
    return
  fi
  if [ ! -d "$wt" ] || ! grep -q 'REFUSED' <<<"$msg"; then
    echo "✗ $desc: the worktree went, or the refusal was never reported" >&2
    printf '%s\n' "${msg:-(no output at all)}" | sed 's/^/    /' >&2
    failures=$((failures + 1))
    return
  fi
  echo "✓ $desc"
}

# Whatever the next refusal turns out to be, it has to be audible. A dry-run
# note is no help — the dry run never attempts a removal — so the real run's
# systemMessage is the only channel, and it has to fire even on a run that
# freed nothing. The shim refuses every `worktree remove` and passes everything
# else to the real git.
check_refused_removal_is_audible() {
  desc="a removal git refuses is reported, not swallowed"
  work="$TMP/refused"
  build_submodule_fixture "$work" || {
    echo "✗ $desc: could not build the fixture" >&2
    failures=$((failures + 1))
    return
  }

  real_git=$(command -v git)
  cat > "$work/bin/git" <<SHIM
#!/usr/bin/env bash
case "\$*" in
  *"worktree remove"*) echo 'fatal: refused by the test shim' >&2; exit 128 ;;
esac
exec "$real_git" "\$@"
SHIM
  chmod +x "$work/bin/git"

  find "$wt" -depth -exec touch -t 200001010000 {} \; 2>/dev/null
  touch -t 200001010000 "$wt"

  msg=$(cd "$main" && PATH="$work/bin:$PATH" CLAUDE_PROJECT_DIR="$main" \
    RECLAIM_FORCE=1 RECLAIM_NO_NETWORK=1 RECLAIM_MIN_IDLE_MINUTES=0 \
    RECLAIM_PRUNE_IDLE_MINUTES=0 "$SCRIPT" </dev/null 2>/dev/null)

  if ! grep -q '"systemMessage"' <<<"$msg" ||
    ! grep -q 'REFUSED' <<<"$msg" ||
    ! grep -q 'w1' <<<"$msg"; then
    echo "✗ $desc: the run said nothing a session would see" >&2
    printf '%s\n' "${msg:-(no output at all)}" | sed 's/^/    /' >&2
    failures=$((failures + 1))
    return
  fi

  # And the refusal must not cost the disk win: a worktree tier 2 could not
  # remove still falls through to tier 1.
  if [ -d "$wt/target/debug" ]; then
    echo "✗ $desc: a refused worktree stopped getting its cache pruned" >&2
    failures=$((failures + 1))
    return
  fi
  echo "✓ $desc"
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
check_containment
check_orphan_report
check_submodule_removal
check_submodule_dirty_is_kept
check_force_retry_gate dirty \
  "a tree that goes dirty before the retry is not forced"
check_force_retry_gate unknown \
  "a recheck that cannot answer is not read as clean"
check_refused_removal_is_audible

echo
if [ "$failures" -gt 0 ]; then
  echo "✗ $failures lock case(s) failed" >&2
  exit 1
fi
echo "✅ reclaim-worktrees.sh keeps ownership and lock boundaries"
