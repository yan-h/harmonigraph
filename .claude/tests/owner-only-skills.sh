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

  # The submodule path is the SAME FILE through the gitlink. Exempting it so
  # the skill stayed editable is what defeated the first version of this gate:
  # the deny reason named that path, and Claude read the refusal and followed
  # it. A second route to the file is not an exemption, it is the bypass.
  expect deny ".shared-skills/skills/$skill/SKILL.md" "the submodule path reaches the same file"

  # And the refusal must not hand over a route. This is the finding above
  # turned into a check: the gate held, and the message defeated it.
  reason=$(printf '{"tool_input":{"command":"cat .claude/skills/%s/SKILL.md"}}' "$skill" | "$GATE" | python3 -c '
import json, sys
print(json.load(sys.stdin)["hookSpecificOutput"].get("permissionDecisionReason", ""))
')
  # A path, not the bare filename: naming the file is how the refusal makes
  # sense, but naming a route to it is what handed the bypass over.
  case "$reason" in
    */SKILL.md*|*.shared-skills*|*skills/*)
      echo "✗ the deny reason names a path to the file it just refused:" >&2
      echo "    $reason" >&2
      fail=1
      ;;
  esac
done

# The gate's fast path skips the JSON parse for payloads that mention no
# owned skill. With one name in the list, a fast path hardcoding that name
# and one derived from the list behave identically, so the loop above cannot
# tell them apart — run a copy with a DIFFERENT name to reach the difference.
FAKE=$(mktemp -t owner-only-skills)
sed 's/^OWNER_ONLY=.*/OWNER_ONLY="probe-skill"/' "$GATE" > "$FAKE"
chmod +x "$FAKE"
got=$(printf '{"tool_input":{"command":"cat .claude/skills/probe-skill/SKILL.md"}}' | "$FAKE" | head -c 4)
rm -f "$FAKE"
[ -n "$got" ] || {
  echo "✗ a renamed OWNER_ONLY entry is not gated — the fast path is keyed on a literal" >&2
  echo "    so anything added to the list after the first name fails open, silently" >&2
  fail=1
}

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

user_path = os.path.join(root, ".codex", "user-hooks.json")
if not os.path.exists(user_path):
    missing.append(".codex/user-hooks.json (the copy Codex worktrees load)")

if missing:
    print("✗ the gate is not registered in: " + ", ".join(missing), file=sys.stderr)
    print("    Codex is the host that needs it most — it ignores", file=sys.stderr)
    print("    disable-model-invocation, so this hook is its only gate, and its", file=sys.stderr)
    print("    project layer does not load in the worktrees sessions run in.", file=sys.stderr)
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
