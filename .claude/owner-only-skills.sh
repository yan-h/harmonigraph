#!/usr/bin/env bash
# PreToolUse(Skill): refuse the skills only Yan starts.
#
# `audit-merges` is expensive and it is a judgement call about when a batch is
# worth auditing, so it is Yan's to start and nobody else's. Prose alone does
# not hold that line: the skill's own description ends with a rule for when to
# use it, every session carries that description, and a session that has just
# watched several branches land reads it as an instruction. The audits that
# prompted this gate were all started that way.
#
# What makes a deterministic gate possible here is a seam in the harness,
# measured rather than assumed (2026-09-19, Claude Code hooks):
#
#   - a model-initiated `Skill(audit-merges)` is a tool call, so PreToolUse
#     fires and `permissionDecision: deny` stops it;
#   - Yan typing `/audit-merges` loads the skill body DIRECTLY, with no Skill
#     tool call, so this hook never sees it and never can block it.
#
# So denying here costs Yan nothing and removes the autonomous path entirely.
# The two halves are what .claude/tests/owner-only-skills.sh pins; the second
# one is the load-bearing half, and it is the half that would be expensive to
# discover by shipping a gate that locked Yan out of his own audit.
#
# The docs did not list `Skill` as a matchable PreToolUse tool name when this
# was written. It matches; the matcher filters on the tool name and `Skill` is
# a tool like any other. Read the test, not the docs page.
#
# This gate is instructed at its edges rather than enforced. A session can
# still open .claude/skills/audit-merges/SKILL.md and run the procedure by
# hand, exactly as it can ignore any other line in CLAUDE.md, so the prose in
# CLAUDE.md and in pr-hygiene closes that half and this closes the reflex.
#
#   echo '{"tool_input":{"skill":"audit-merges"}}' | .claude/owner-only-skills.sh
#
# Written for bash 3.2 (macOS system bash), like the scripts beside it.
set -uo pipefail

# Skills a session may not start for itself. One entry today; the list is the
# thing to add to, so the rule stays in one place rather than growing a second
# hook per skill.
OWNER_ONLY="audit-merges"

payload=$(cat)

# Absent, unparseable or unexpected input allows the call. A hook that fails
# closed on a payload it did not understand would break every skill in the
# tree, and it would break them on the harness changing its JSON rather than
# on anything here being wrong.
skill=$(printf '%s' "$payload" | python3 -c '
import json, sys
try:
    print(json.load(sys.stdin).get("tool_input", {}).get("skill", ""))
except Exception:
    print("")
' 2>/dev/null)

for owned in $OWNER_ONLY; do
  [ "$skill" = "$owned" ] || continue
  # `permissionDecisionReason` is what the model reads back, so it carries the
  # one thing that resolves the block: Yan types the slash command. A reason
  # that only said "denied" would send the session looking for a way around.
  cat <<JSON
{
  "hookSpecificOutput": {
    "hookEventName": "PreToolUse",
    "permissionDecision": "deny",
    "permissionDecisionReason": "/$skill is Yan's to start, not a session's. Do not run the procedure out of .claude/skills/$skill/SKILL.md by hand either, and do not spawn its subagents directly. If it looks worth running, say so in your reply and let Yan type /$skill himself."
  }
}
JSON
  exit 0
done

exit 0
