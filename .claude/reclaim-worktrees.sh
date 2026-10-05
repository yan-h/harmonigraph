#!/usr/bin/env bash
# Removes resolved worktrees under .claude/worktrees and prunes idle build
# caches. The implementation and its tests live in agent-config's
# session-lifecycle skill, shared with every project that uses it, because
# per-project copies drifted apart and a fix landed in only one of them.
#
# Without the skill installed this does nothing: SessionStart runs it in every
# session, and there is no tested local fallback to run instead.
TOOL="${AGENT_LIFECYCLE_TOOL:-$HOME/.agents/skills/session-lifecycle/scripts/lifecycle.py}"
SCRIPT="$(dirname "$TOOL")/reclaim-worktrees.sh"
if [ ! -f "$SCRIPT" ]; then
  [ "${RECLAIM_DRY_RUN:-0}" = 1 ] &&
    echo "agent-config's session-lifecycle skill is not installed; nothing reclaims here" >&2
  exit 0
fi
exec bash "$SCRIPT" "$@"
