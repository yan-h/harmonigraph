#!/usr/bin/env bash
# Make the shared agent guidance in this checkout match the gitlink exactly.
#
# Linked worktrees give every submodule its own checkout. Git therefore leaves
# `.shared-skills` empty even when the primary checkout already has it, and the
# project-internal skill symlink dangles until this command runs.
set -uo pipefail

ROOT=${1:-}
if [ -z "$ROOT" ]; then
  ROOT=$(git rev-parse --show-toplevel 2>/dev/null) || {
    echo "✗ shared skills: not inside a Git worktree" >&2
    exit 1
  }
fi

EXPECTED=$(git -C "$ROOT" rev-parse HEAD:.shared-skills 2>/dev/null) || {
  echo "✗ shared skills: HEAD has no .shared-skills gitlink in $ROOT" >&2
  exit 1
}
SKILL_LINK="$ROOT/.claude/skills/audit-merges"
SKILL="$SKILL_LINK/SKILL.md"
BRIEF="$SKILL_LINK/references/merge-auditor.md"

ready() {
  [ "$(readlink "$SKILL_LINK" 2>/dev/null)" = "../../.shared-skills/skills/audit-merges" ] || return 1
  ACTUAL=$(git -C "$ROOT/.shared-skills" rev-parse HEAD 2>/dev/null) || return 1
  [ "$ACTUAL" = "$EXPECTED" ] || return 1
  git -C "$ROOT/.shared-skills" diff --quiet HEAD -- || return 1
  [ -r "$SKILL" ] && [ -r "$BRIEF" ]
}

# This is the ordinary-session path: two local object reads and no fetch.
ready && exit 0

echo "Preparing pinned shared skills for $ROOT" >&2
if ! git -C "$ROOT" submodule update --init --checkout -- .shared-skills >&2; then
  echo "✗ shared skills: could not initialise .shared-skills at $EXPECTED" >&2
  echo "  Network access is needed only when Git has no local copy of that pinned commit." >&2
  exit 1
fi

if ! ready; then
  ACTUAL=$(git -C "$ROOT/.shared-skills" rev-parse HEAD 2>/dev/null || true)
  echo "✗ shared skills: checkout did not establish the pinned audit guidance" >&2
  echo "  expected commit: $EXPECTED" >&2
  echo "  actual commit:   ${ACTUAL:-unreadable}" >&2
  echo "  required paths:  $SKILL" >&2
  echo "                   $BRIEF" >&2
  exit 1
fi
