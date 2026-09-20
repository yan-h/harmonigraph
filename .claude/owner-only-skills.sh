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
# The submodule path is matched too, and the refusal names no alternative.
# An earlier version exempted `.shared-skills/` so the skill stayed editable,
# and said so in the deny reason — whereupon Claude read the refusal, followed
# the path it named, and got the procedure anyway (measured 2026-09-19). An
# exemption for a second path to the SAME FILE is not an exemption, it is the
# gate's own bypass, and naming it in the refusal is handing it over. To edit
# this skill, ask Yan.
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

payload=$(cat)

# Fast path first. Codex's gate has to be USER-level — project-local hooks
# load only in a trusted project, and every Codex session runs in a fresh
# managed worktree whose path was never trusted — so this runs on every shell
# call in every repo on the machine. A bash string test costs nothing; the
# python start-up below would be ~30ms on each one.
# Derived from OWNER_ONLY, never spelled out: a fast path keyed on one name
# while the list holds two is the too-narrow key this repo keeps shipping,
# and it would fail open silently for whatever was added second.
hit=
for owned in $OWNER_ONLY; do
  case "$payload" in *"$owned"*) hit=1 ;; esac
done
[ -n "$hit" ] || exit 0

# Unreadable payloads allow. Failing closed on a harness JSON change would
# block ordinary shell calls, which is far worse than a missed audit.
target=$(printf '%s' "$payload" | python3 -c '
import json, sys
try:
    ti = json.load(sys.stdin).get("tool_input", {})
except Exception:
    sys.exit()
print(" ".join(str(ti.get(k, "")) for k in ("command", "file_path", "path")))
' 2>/dev/null)

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
    "permissionDecisionReason": "/$owned is Yan's to start, not a session's, so reading its SKILL.md is refused. Do not look for another path to the same file. If a range looks worth auditing, say so in your reply and let Yan invoke it; if you need to change the skill itself, ask him."
  }
}
JSON
  exit 0
done

exit 0
