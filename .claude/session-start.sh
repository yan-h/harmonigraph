#!/usr/bin/env bash
# Prepare shared skills before Claude's first turn, then ask it to rescan them.
set -uo pipefail

SOURCE_ROOT=${CLAUDE_PROJECT_DIR:-}
if [ -z "$SOURCE_ROOT" ]; then
  echo "✗ shared skills: CLAUDE_PROJECT_DIR is not set" >&2
  exit 2
fi

# CLAUDE_PROJECT_DIR stays at the checkout where Claude started when it enters
# a worktree. The hook input's cwd is the checkout the session will actually
# use, so do not initialise the former by accident.
CWD=$(python3 -c '
import json
import sys

value = json.load(sys.stdin).get("cwd")
if not isinstance(value, str) or not value:
    raise SystemExit("SessionStart input has no cwd")
print(value)
' 2>/dev/null) || {
  echo "✗ shared skills: could not read cwd from SessionStart input" >&2
  exit 2
}

ROOT=$(git -C "$CWD" rev-parse --show-toplevel 2>/dev/null) || {
  echo "✗ shared skills: SessionStart cwd is not a Git worktree: $CWD" >&2
  exit 2
}

if ! "$SOURCE_ROOT/.claude/ensure-shared-skills.sh" "$ROOT"; then
  echo "✗ shared skills: session continues without the pinned worktree guidance" >&2
  exit 2
fi

# A fresh clone has no repository-local hook configuration. Install the
# tracked hook path once so later `git worktree add` calls initialise before
# the owner hands the worktree to an agent. Preserve a deliberate custom hook
# path rather than silently replacing it, but make the missing guarantee loud.
HOOKS_PATH=$(git -C "$ROOT" config --local --get core.hooksPath 2>/dev/null || true)
if [ -z "$HOOKS_PATH" ]; then
  if ! git -C "$ROOT" config --local core.hooksPath .githooks; then
    echo "✗ shared skills: could not enable the tracked Git hooks" >&2
    exit 2
  fi
  HOOKS_PATH=.githooks
fi

case "$HOOKS_PATH" in
  /*) HOOKS_DIR=$HOOKS_PATH ;;
  *) HOOKS_DIR="$ROOT/$HOOKS_PATH" ;;
esac

HOOKS_DIR_PHYSICAL=$(cd "$HOOKS_DIR" 2>/dev/null && pwd -P) || true
ROOT_COMMON=$(git -C "$ROOT" rev-parse --path-format=absolute --git-common-dir 2>/dev/null) || true
HOOK_COMMON=$(git -C "$HOOKS_DIR_PHYSICAL/.." rev-parse --path-format=absolute --git-common-dir 2>/dev/null) || true
if [ -z "$HOOKS_DIR_PHYSICAL" ] || [ ! -x "$HOOKS_DIR_PHYSICAL/post-checkout" ] || {
  [ "${HOOKS_DIR_PHYSICAL##*/}" != .githooks ] ||
    [ -z "$ROOT_COMMON" ] || [ "$HOOK_COMMON" != "$ROOT_COMMON" ];
}; then
  echo "✗ shared skills: core.hooksPath does not select this repository's .githooks" >&2
  echo "  configured path: $HOOKS_PATH" >&2
  echo "  run: git config core.hooksPath .githooks" >&2
  exit 2
fi

# Skill discovery normally precedes SessionStart. Re-scan now that the
# submodule target exists so audit-merges is available for the first prompt.
printf '%s\n' '{"hookSpecificOutput":{"hookEventName":"SessionStart","reloadSkills":true}}'
