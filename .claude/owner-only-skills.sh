#!/usr/bin/env bash
# PreToolUse: refuse to read a skill only Yan starts.
#
# `audit-merges` is his to start. Claude enforces that from the skill's own
# `disable-model-invocation: true`, which hides it from the model entirely.
# Codex ignores that field (measured 2026-09-19: it listed the skill and
# loaded the body, from `.agents/skills` and `.codex/skills` alike), and its
# only route to a skill is reading SKILL.md with a shell command — so that
# read is what this refuses.
#
# It costs Yan nothing, because a typed invocation never reaches a hook. Both
# halves measured 2026-09-19 with an all-tools logging hook:
#
#           a session asks for it       Yan types /audit-merges or $audit-merges
#   Claude  Skill tool call             body injected, no tool call
#   Codex   Bash `cat .../SKILL.md`     NO tool call at all
#
# So a session's route is a tool call and Yan's is not, in both hosts.
#
# `.shared-skills` is deliberately not matched: the file lives in the
# agent-config submodule, and editing it there has to stay possible.
#
# One script for two hosts rather than a copy each, per CLAUDE.md's rule that
# a Claude path may hold procedure any agent reads.
#
#   echo '{"tool_input":{"command":"cat .claude/skills/audit-merges/SKILL.md"}}' \
#     | .claude/owner-only-skills.sh
#
# Written for bash 3.2 (macOS system bash), like the scripts beside it.
set -uo pipefail

OWNER_ONLY="audit-merges"

# Unreadable payloads allow. Failing closed on a harness JSON change would
# block ordinary shell calls, which is far worse than a missed audit.
target=$(python3 -c '
import json, sys
try:
    ti = json.load(sys.stdin).get("tool_input", {})
except Exception:
    sys.exit()
print(" ".join(str(ti.get(k, "")) for k in ("command", "file_path", "path")))
' 2>/dev/null)

case "$target" in *.shared-skills*) exit 0 ;; esac

for owned in $OWNER_ONLY; do
  case "$target" in
    *"skills/$owned/SKILL.md"*) ;;
    *) continue ;;
  esac
  cat <<JSON
{
  "hookSpecificOutput": {
    "hookEventName": "PreToolUse",
    "permissionDecision": "deny",
    "permissionDecisionReason": "/$owned is Yan's to start, not a session's, so reading its SKILL.md to run the procedure by hand is refused. If a range looks worth auditing, say so and let Yan invoke it. To EDIT the skill, read it at .shared-skills/skills/$owned/SKILL.md instead."
  }
}
JSON
  exit 0
done

exit 0
