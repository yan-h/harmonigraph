#!/usr/bin/env bash
# Removes resolved worktrees under .claude/worktrees and prunes idle build
# caches. The implementation and its tests live in agent-config's
# session-lifecycle skill, shared with every project that uses it, because
# per-project copies drifted apart and a fix landed in only one of them.
#
# Without the skill installed nothing reclaims, since there is no tested local
# fallback to run instead, so every session start says so: a silent no-op left
# a machine's worktrees and build caches piling up with nothing on screen.
TOOL="${AGENT_LIFECYCLE_TOOL:-$HOME/.agents/skills/session-lifecycle/scripts/lifecycle.py}"
SCRIPT="$(dirname "$TOOL")/reclaim-worktrees.sh"
if [ ! -f "$SCRIPT" ]; then
  printf '{"systemMessage":"%s"}\n' "Worktree cleanup cannot run: agent-config's session-lifecycle skill is not in ~/.agents/skills. Run agent-config's scripts/install.sh --repo <this checkout>."
  exit 0
fi
exec bash "$SCRIPT" "$@"
