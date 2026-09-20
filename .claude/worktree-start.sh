#!/usr/bin/env bash
# Native EnterWorktree and isolated agents bypass Git's post-checkout hook.
# Both Claude hooks supply the new worktree's cwd before its first tool runs.
set -uo pipefail

INPUT=$(cat)
EVENT=$(printf '%s' "$INPUT" | python3 -c 'import json,sys; print(json.load(sys.stdin).get("hook_event_name", ""))' 2>/dev/null) || EVENT=

fail() {
  local message="✗ shared skills: $1. Do not use or delegate work that needs missing guidance, or substitute another checkout's copy; repair preparation first."
  echo "$message" >&2
  if [ "$EVENT" = SubagentStart ]; then
    # Claude records exit-2 stderr but does not deliver it to the child.
    # This hook is nonblocking: explicit context makes failure visible there.
    python3 - "$message" <<'PY'
import json
import sys

message = sys.argv[1]
print(json.dumps({"systemMessage": message, "hookSpecificOutput": {
    "hookEventName": "SubagentStart", "additionalContext": message}}))
PY
    exit 0
  fi
  exit 2
}

SOURCE_ROOT=${CLAUDE_PROJECT_DIR:-}
if [ -z "$SOURCE_ROOT" ]; then
  fail "CLAUDE_PROJECT_DIR is not set"
fi

CWD=$(printf '%s' "$INPUT" | python3 -c '
import json
import sys

value = json.load(sys.stdin).get("cwd")
if not isinstance(value, str) or not value:
    raise SystemExit("worktree hook input has no cwd")
print(value)
' 2>/dev/null) || {
  fail "could not read cwd from worktree hook input"
}

ROOT=$(git -C "$CWD" rev-parse --show-toplevel 2>/dev/null) || {
  fail "hook cwd is not a Git worktree: $CWD"
}

if ! "$SOURCE_ROOT/.claude/ensure-shared-skills.sh" "$ROOT"; then
  fail "pinned worktree guidance is unavailable in $ROOT"
fi

# These events cannot undo worktree creation. Fail visibly above and leave
# lifecycle/cleanup with Claude. reloadSkills belongs to SessionStart only.
