#!/usr/bin/env bash
# Shared implementation is installed from agent-config's session-lifecycle skill.
set -euo pipefail
TOOL="${AGENT_LIFECYCLE_TOOL:-$HOME/.agents/skills/session-lifecycle/scripts/lifecycle.py}"
[[ -f "$TOOL" ]] || {
  echo "session-lifecycle is not in ~/.agents/skills: run agent-config's scripts/install.sh --repo <this checkout> (or set AGENT_LIFECYCLE_TOOL)." >&2
  exit 1
}
exec python3 "$TOOL" --repo "$(cd "$(dirname "$0")" && pwd)" "$@"
