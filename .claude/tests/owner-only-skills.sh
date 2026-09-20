#!/usr/bin/env bash
# Does the owner-only read gate still refuse what it names, allow everything
# else, and is it still wired into both hosts?
#
# The gate sits on PreToolUse and its two failure directions are not
# symmetric. Too narrow and a session hand-runs the audit. Too wide and it
# denies ordinary shell calls, which is far more disruptive and is the reason
# unreadable payloads allow rather than block.
#
#   .claude/tests/owner-only-skills.sh          # run it
#
# Written for bash 3.2 (macOS system bash), like the scripts beside it.
set -uo pipefail

ROOT=$(cd "$(dirname "$0")/../.." && pwd)
GATE="$ROOT/.claude/owner-only-skills.sh"

[ -x "$GATE" ] || {
  echo "✗ $GATE is missing or not executable, so the hook never runs and never says so" >&2
  exit 1
}

fail=0

# Read the list from the gate rather than restating it, or this pins a second
# copy of the fact and the two drift.
owned=$(sed -n 's/^OWNER_ONLY="\(.*\)"$/\1/p' "$GATE")
[ -n "$owned" ] || {
  echo "✗ $GATE declares no OWNER_ONLY list" >&2
  exit 1
}

decision() {
  printf '{"tool_name":"Bash","tool_input":{"command":"%s"}}' "$1" | "$GATE" | python3 -c '
import json, sys
raw = sys.stdin.read().strip()
print(json.loads(raw)["hookSpecificOutput"]["permissionDecision"] if raw else "allow")
' 2>/dev/null
}

expect() { # expect <decision> <command> <why>
  got=$(decision "$2")
  [ "$got" = "$1" ] || {
    echo "✗ gate answered '$got', wanted '$1': $2" >&2
    echo "    $3" >&2
    fail=1
  }
}

for skill in $owned; do
  # The verb is not the subject — the path is — so a session cannot step
  # around the gate by reaching for sed or head instead of cat.
  expect deny "cat .claude/skills/$skill/SKILL.md" "a session could hand-run the audit"
  expect deny "sed -n 1,40p .agents/skills/$skill/SKILL.md" "the gate matched the verb, not the path"

  # Editing the skill has to stay possible: it lives in the agent-config
  # submodule, and that path is deliberately exempt.
  expect allow ".shared-skills/skills/$skill/SKILL.md" "the submodule copy is the maintenance path"
done

# Ordinary work must pass through untouched.
expect allow "cargo test --workspace" "the gate is on every shell call in the tree"
expect allow "cat .claude/skills/pr-hygiene/SKILL.md" "no other skill is restricted"

# Payloads the gate cannot read allow, for the same reason.
for payload in '' 'not json' '{}' '{"tool_input":{}}'; do
  got=$(printf '%s' "$payload" | "$GATE")
  [ -z "$got" ] || {
    echo "✗ gate answered '$got' to an unreadable payload instead of allowing" >&2
    fail=1
  }
done

# The wiring is the copy of the fact that lives elsewhere: the script can be
# perfect and simply never registered, in either host, silently.
python3 - "$ROOT" <<'PY' || fail=1
import json, os, sys

root = sys.argv[1]
missing = []

settings = json.load(open(os.path.join(root, ".claude", "settings.json")))
if not [
    h
    for e in settings.get("hooks", {}).get("PreToolUse", [])
    for h in e.get("hooks", [])
    if "owner-only-skills.sh" in h.get("command", "")
]:
    missing.append(".claude/settings.json (Claude)")

codex_path = os.path.join(root, ".codex", "hooks.json")
if not os.path.exists(codex_path):
    missing.append(".codex/hooks.json (Codex, absent)")
else:
    codex = json.load(open(codex_path))
    if not [
        h
        for e in codex.get("hooks", {}).get("PreToolUse", [])
        for h in e.get("hooks", [])
        if "owner-only-skills.sh" in h.get("command", "")
    ]:
        missing.append(".codex/hooks.json (Codex)")

if missing:
    print("✗ the gate is not registered in: " + ", ".join(missing), file=sys.stderr)
    print("    Codex is the host that needs it most — it ignores", file=sys.stderr)
    print("    disable-model-invocation, so this hook is its only gate.", file=sys.stderr)
    sys.exit(1)
PY

# A gitignored rule is inert in exactly the worktrees where sessions run.
LOCAL="$ROOT/.claude/settings.local.json"
if [ -f "$LOCAL" ] && grep -q 'owner-only-skills.sh' "$LOCAL"; then
  echo "✗ the gate is wired in the gitignored .claude/settings.local.json" >&2
  fail=1
fi

[ "$fail" -eq 0 ] || exit 1

echo "✓ owner-only read gate refuses ${owned}, passes ordinary work through, and is wired into both hosts"
