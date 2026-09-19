#!/usr/bin/env bash
# Does the owner-only skill gate still deny what it names, allow everything
# else, and is it still wired to the event that reaches it?
#
# The gate exists so no session starts `/audit-merges` for itself. It sits on
# PreToolUse(Skill), which is a hook every skill invocation in the tree passes
# through, so its two failure directions are not symmetric:
#
#   - too narrow, and an audit runs unasked, which is the thing it was built
#     to stop and which costs a run nobody wanted;
#   - too wide, and EVERY skill stops loading. That one is silent in the worst
#     way, because a denied skill reads to the session as "no such procedure"
#     and it simply proceeds without one.
#
# Neither shows up in any Rust test, and both are one careless edit away: the
# gate is a shell script feeding JSON to a hook, and nothing else in the tree
# parses either end.
#
#   .claude/tests/owner-only-skills.sh          # run it
#
# What this test CANNOT cover, and the measurement it stands on: Yan typing
# `/audit-merges` loads the skill body directly, with no Skill tool call, so
# PreToolUse never fires for it. That was measured against the real harness on
# 2026-09-19 (a project-local hook that denied everything; the typed slash
# command ran the body anyway and the hook's payload file was never written).
# Confirming it here would mean spending a model call per CI run, so it stays
# measured-once and written down. If a harness change ever routed the typed
# command through the Skill tool, this gate would lock Yan out of his own
# audit, and the symptom would be `/audit-merges` answering with the deny
# reason below instead of running.
#
# Written for bash 3.2 (macOS system bash), like the scripts beside it.
set -uo pipefail

ROOT=$(cd "$(dirname "$0")/../.." && pwd)
GATE="$ROOT/.claude/owner-only-skills.sh"
SETTINGS="$ROOT/.claude/settings.json"

for f in "$GATE" "$SETTINGS"; do
  [ -f "$f" ] || {
    echo "✗ missing: $f" >&2
    exit 1
  }
done

# A hook command the harness cannot execute fails open, quietly, on every call.
[ -x "$GATE" ] || {
  echo "✗ $GATE is not executable, so the hook never runs and never says so" >&2
  exit 1
}

fail=0

# The gate reads its list from one place; read it from the same place rather
# than restating it here, or this test pins a second copy of the fact and the
# two drift.
owned=$(sed -n 's/^OWNER_ONLY="\(.*\)"$/\1/p' "$GATE")
if [ -z "$owned" ]; then
  echo "✗ $GATE declares no OWNER_ONLY list" >&2
  exit 1
fi

# Deny, once per named skill. `--argjson`-free on purpose: python3 is already
# this tree's JSON reader in ci.sh and semantic-breaks.py, and jq is not a
# dependency here.
decision() {
  printf '{"tool_name":"Skill","tool_input":{"skill":"%s"}}' "$1" | "$GATE" | python3 -c '
import json, sys
raw = sys.stdin.read().strip()
if not raw:
    print("allow")
    sys.exit()
out = json.loads(raw)["hookSpecificOutput"]
print(out["permissionDecision"] + "\t" + out.get("permissionDecisionReason", ""))
' 2>/dev/null
}

for skill in $owned; do
  got=$(decision "$skill")
  case "$got" in
    deny*) ;;
    *)
      echo "✗ $skill is listed OWNER_ONLY but the gate answers '${got:-<nothing>}'" >&2
      echo "    a session can start it unasked, which is the whole point of the gate" >&2
      fail=1
      continue
      ;;
  esac

  # The reason is the only thing the blocked session reads, and it is the only
  # place the way forward is written. A deny with nothing in it sends the
  # session looking for a route around instead of back to Yan.
  reason=${got#*$'\t'}
  case "$reason" in
    *"/$skill"*) ;;
    *)
      echo "✗ $skill is denied without naming /$skill as the way to run it" >&2
      echo "    reason was: ${reason:-<empty>}" >&2
      fail=1
      ;;
  esac

  # A gate naming a skill this tree does not have is dead text that reads as
  # protection. The skills are symlinks as often as directories, so test the
  # manifest rather than the directory.
  [ -f "$ROOT/.claude/skills/$skill/SKILL.md" ] || {
    echo "✗ OWNER_ONLY names '$skill', which has no .claude/skills/$skill/SKILL.md" >&2
    fail=1
  }
done

# And everything else passes through. `pr-hygiene` is a real skill in this
# tree on purpose: a made-up name would pass this check while the loop above
# was broken in a way that only ever matched real ones.
for skill in pr-hygiene build-handover capture-daw-state; do
  got=$(decision "$skill")
  [ "$got" = allow ] || {
    echo "✗ the gate answers '$got' for $skill, which nobody restricted" >&2
    echo "    a skill denied here reads to the session as 'no such procedure'" >&2
    fail=1
  }
done

# Payloads the gate does not understand allow, for the same reason: failing
# closed on a harness JSON change would take every skill down with it.
for payload in '' 'not json at all' '{}' '{"tool_input":{}}'; do
  got=$(printf '%s' "$payload" | "$GATE")
  [ -z "$got" ] || {
    echo "✗ the gate answered '$got' to an unreadable payload instead of allowing" >&2
    fail=1
  }
done

# Finally the wiring, which is the copy of the fact that lives somewhere else.
# The script can be perfect and still never run: the hook has to be registered
# under PreToolUse with a matcher that catches the Skill tool. Both halves are
# silent when wrong — an unregistered hook simply never fires.
python3 - "$SETTINGS" <<'PY' || fail=1
import json, sys

settings = json.load(open(sys.argv[1]))
hooks = settings.get("hooks", {}).get("PreToolUse", [])
wired = [
    h
    for entry in hooks
    if entry.get("matcher") in ("Skill", "Skill|.*", ".*")
    for h in entry.get("hooks", [])
    if "owner-only-skills.sh" in h.get("command", "")
]
if not wired:
    print(
        "✗ .claude/settings.json does not run owner-only-skills.sh on"
        " PreToolUse with a matcher that catches Skill",
        file=sys.stderr,
    )
    print(
        "    the script then passes its own test and never sees a single call",
        file=sys.stderr,
    )
    sys.exit(1)
PY

# .claude/settings.local.json is gitignored, so a rule left there is inert in
# every worktree — which is where sessions run, and where the audit would be
# started from. CLAUDE.md says this about permissions; the gate has the same
# shape and the same failure.
LOCAL="$ROOT/.claude/settings.local.json"
if [ -f "$LOCAL" ] && grep -q 'owner-only-skills.sh' "$LOCAL"; then
  echo "✗ the gate is wired in .claude/settings.local.json, which is gitignored" >&2
  echo "    a fresh worktree never gets a copy, so the gate is absent where sessions run" >&2
  fail=1
fi

[ "$fail" -eq 0 ] || exit 1

echo "✓ owner-only skill gate denies ${owned}, passes every other skill through, and is wired to PreToolUse(Skill)"
