#!/usr/bin/env bash
# Do session, native Claude and ordinary Git worktree hooks materialise the exact
# shared-skills gitlink deeply enough for audit-merges and its brief to load?
#
# The source repository advances after the superproject pins it. Reading only
# the directory, or even the source's latest commit, cannot pass this test: the
# fresh worktree must contain both files from the older pinned commit.
set -uo pipefail

for v in $(env | sed -n 's/^\(GIT_[A-Z_]*\)=.*/\1/p'); do
  unset "$v"
done

ROOT=$(cd "$(dirname "$0")/../.." && pwd)
ENSURE="$ROOT/.claude/ensure-shared-skills.sh"
SESSION_START="$ROOT/.claude/session-start.sh"
WORKTREE_START="$ROOT/.claude/worktree-start.sh"
POST_CHECKOUT="$ROOT/.githooks/post-checkout"
for file in "$ENSURE" "$SESSION_START" "$WORKTREE_START" "$POST_CHECKOUT"; do
  [ -x "$file" ] || { echo "✗ not executable: $file" >&2; exit 1; }
done

TMP=$(mktemp -d "${TMPDIR:-/tmp}/shared-skills worktrees.XXXXXX") || exit 1
trap 'rm -rf "$TMP"' EXIT
SOURCE="$TMP/shared source"
MAIN="$TMP/main checkout"
FRESH="$TMP/fresh worktree"
FAILED="$TMP/failed worktree"
UNPREPARED="$TMP/unprepared worktree"
mkdir -p "$SOURCE/skills/audit-merges/references" "$MAIN"

(
  cd "$SOURCE" || exit 1
  git init -q . 2>/dev/null || exit 1
  git config user.email t@t
  git config user.name t
  printf '%s\n' 'skill from pinned commit' > skills/audit-merges/SKILL.md
  printf '%s\n' 'brief from pinned commit' > skills/audit-merges/references/merge-auditor.md
  git add skills
  git commit -q -m pinned || exit 1
) || { echo "✗ could not build the shared-skills source fixture" >&2; exit 1; }

(
  cd "$MAIN" || exit 1
  git init -q . 2>/dev/null || exit 1
  real=$(pwd -P)
  here=$(git rev-parse --absolute-git-dir 2>/dev/null)
  case "$here" in
    "$real"/*) ;;
    *) echo "refusing: Git resolves to ${here:-nothing}, not $real" >&2; exit 1 ;;
  esac
  git config user.email t@t
  git config user.name t
  git config protocol.file.allow always
  mkdir -p .claude/skills .githooks
  cp "$ENSURE" .claude/ensure-shared-skills.sh
  cp "$SESSION_START" .claude/session-start.sh
  cp "$WORKTREE_START" .claude/worktree-start.sh
  cp "$ROOT/.claude/settings.json" .claude/settings.json
  cp "$POST_CHECKOUT" .githooks/post-checkout
  ln -s ../../.shared-skills/skills/audit-merges .claude/skills/audit-merges
  git -c protocol.file.allow=always submodule add -q "$SOURCE" .shared-skills || exit 1
  git add .claude .githooks .gitmodules .shared-skills
  git commit -q -m superproject || exit 1
) || { echo "✗ could not build the superproject fixture" >&2; exit 1; }

PIN=$(git -C "$MAIN" rev-parse HEAD:.shared-skills) || exit 1

# Advance the source after the superproject commit. A checkout of the source's
# tip now has readable files too, but it is the wrong guidance.
printf '%s\n' 'skill from newer unpinned commit' > "$SOURCE/skills/audit-merges/SKILL.md"
printf '%s\n' 'brief from newer unpinned commit' > "$SOURCE/skills/audit-merges/references/merge-auditor.md"
git -C "$SOURCE" add skills
git -C "$SOURCE" commit -q -m newer || exit 1

# Positive control for the original defect. Before SessionStart enables the
# tracked hooks, the same creation operation leaves the gitlink uninitialised
# and both paths dangling even though the primary checkout has the submodule.
if ! git -C "$MAIN" worktree add -q --detach "$UNPREPARED" HEAD; then
  echo "✗ could not build the unprepared-worktree control" >&2
  exit 1
fi
if ! git -C "$UNPREPARED" submodule status -- .shared-skills | grep -q '^-' ||
  [ -e "$UNPREPARED/.claude/skills/audit-merges/SKILL.md" ] ||
  [ -e "$UNPREPARED/.claude/skills/audit-merges/references/merge-auditor.md" ]; then
  echo "✗ the control did not reproduce the dangling fresh-worktree paths" >&2
  git -C "$MAIN" worktree remove --force "$UNPREPARED" 2>/dev/null || true
  exit 1
fi

# `claude --worktree` can create this shape before any SessionStart has enabled
# the Git hook. CLAUDE_PROJECT_DIR still names MAIN, while the input cwd names
# UNPREPARED; initialise the latter and request the same-session skill rescan.
WORKTREE_SESSION_OUT=$(printf '{"cwd":"%s","hook_event_name":"SessionStart","source":"startup"}\n' "$UNPREPARED" |
  GIT_CONFIG_COUNT=1 GIT_CONFIG_KEY_0=protocol.file.allow GIT_CONFIG_VALUE_0=always \
  CLAUDE_PROJECT_DIR="$MAIN" "$MAIN/.claude/session-start.sh")
WORKTREE_SESSION_STATUS=$?
if [ "$WORKTREE_SESSION_STATUS" -ne 0 ] ||
  ! printf '%s\n' "$WORKTREE_SESSION_OUT" | grep -q '"reloadSkills":true' ||
  [ "$(git -C "$UNPREPARED/.shared-skills" rev-parse HEAD 2>/dev/null)" != "$PIN" ] ||
  [ "$(sed -n '1p' "$UNPREPARED/.claude/skills/audit-merges/references/merge-auditor.md" 2>/dev/null)" != 'brief from pinned commit' ]; then
  echo "✗ SessionStart prepared CLAUDE_PROJECT_DIR instead of its worktree cwd" >&2
  git -C "$MAIN" worktree remove --force "$UNPREPARED" 2>/dev/null || true
  exit 1
fi

# A direct resume inside that linked worktree reports it as both project dir
# and cwd, while the shared config may hold an absolute path to MAIN's hooks.
# Both directories belong to the same repository, so this is still the tracked
# hook and must not suppress reloadSkills.
git -C "$MAIN" config --local core.hooksPath "$MAIN/.githooks"
DIRECT_SESSION_OUT=$(printf '{"cwd":"%s","hook_event_name":"SessionStart","source":"resume"}\n' "$UNPREPARED" |
  CLAUDE_PROJECT_DIR="$UNPREPARED" "$UNPREPARED/.claude/session-start.sh")
DIRECT_SESSION_STATUS=$?
if [ "$DIRECT_SESSION_STATUS" -ne 0 ] ||
  ! printf '%s\n' "$DIRECT_SESSION_OUT" | grep -q '"reloadSkills":true'; then
  echo "✗ a direct worktree resume rejected the primary checkout's absolute hooksPath" >&2
  git -C "$MAIN" worktree remove --force "$UNPREPARED" 2>/dev/null || true
  exit 1
fi
git -C "$MAIN" worktree remove --force "$UNPREPARED" || exit 1
git -C "$MAIN" config --local --unset core.hooksPath || exit 1
echo "✓ the control reproduces Git's uninitialised worktree and SessionStart repairs that cwd"

# Model the first clone: the gitlink is present but neither its checkout nor a
# cached module repository exists. SessionStart must initialise it, enable the
# creation-time hook, and request a skill rescan for this same first prompt.
git -C "$MAIN" submodule deinit -q -f -- .shared-skills || exit 1
MODULE_GITDIR="$MAIN/.git/modules/.shared-skills"
[ -d "$MODULE_GITDIR" ] || { echo "✗ fixture module cache is missing: $MODULE_GITDIR" >&2; exit 1; }
rm -rf "$MODULE_GITDIR" "$MAIN/.shared-skills"
[ ! -e "$MODULE_GITDIR" ] || { echo "✗ fixture module cache survived removal" >&2; exit 1; }
SESSION_OUT=$(printf '{"cwd":"%s","hook_event_name":"SessionStart","source":"startup"}\n' "$MAIN" |
  GIT_CONFIG_COUNT=1 GIT_CONFIG_KEY_0=protocol.file.allow GIT_CONFIG_VALUE_0=always \
  CLAUDE_PROJECT_DIR="$MAIN" "$MAIN/.claude/session-start.sh")
SESSION_STATUS=$?

if [ "$SESSION_STATUS" -ne 0 ] ||
  ! printf '%s\n' "$SESSION_OUT" | grep -q '"reloadSkills":true' ||
  [ "$(git -C "$MAIN" config --local --get core.hooksPath)" != .githooks ] ||
  [ "$(git -C "$MAIN/.shared-skills" rev-parse HEAD 2>/dev/null)" != "$PIN" ] ||
  [ "$(sed -n '1p' "$MAIN/.claude/skills/audit-merges/SKILL.md" 2>/dev/null)" != 'skill from pinned commit' ] ||
  [ "$(sed -n '1p' "$MAIN/.claude/skills/audit-merges/references/merge-auditor.md" 2>/dev/null)" != 'brief from pinned commit' ]; then
  echo "✗ SessionStart did not establish and reload the pinned shared guidance" >&2
  exit 1
fi
echo "✓ a first SessionStart initialises the gitlink and reloads its skills"

# post-checkout also runs for ordinary branch switches, but with a non-zero
# old HEAD. Replace the initializer with a failing probe: the switch must still
# return success, proving the creation-only guard skipped it.
printf '%s\n' '#!/usr/bin/env bash' 'exit 23' > "$MAIN/.claude/ensure-shared-skills.sh"
if ! git -C "$MAIN" switch -q -c ordinary-switch; then
  echo "✗ the worktree initializer interfered with an ordinary branch switch" >&2
  exit 1
fi
git -C "$MAIN" checkout -q -- .claude/ensure-shared-skills.sh || exit 1
echo "✓ ordinary branch switches do not run worktree initialisation"

# This covers ordinary Git creation, not Claude's native --no-checkout path. Its
# post-checkout hook runs in FRESH with old=zero and flag=1; a successful add
# therefore proves the hook returned only after the worktree-local submodule
# reached the pin.
if ! git -C "$MAIN" \
  -c protocol.file.allow=always \
  -c core.hooksPath="$MAIN/.githooks" \
  worktree add -q --detach "$FRESH" HEAD; then
  echo "✗ git worktree add failed while preparing shared skills" >&2
  exit 1
fi

if [ "$(git -C "$FRESH/.shared-skills" rev-parse HEAD 2>/dev/null)" != "$PIN" ] ||
  [ "$(sed -n '1p' "$FRESH/.claude/skills/audit-merges/SKILL.md" 2>/dev/null)" != 'skill from pinned commit' ] ||
  [ "$(sed -n '1p' "$FRESH/.claude/skills/audit-merges/references/merge-auditor.md" 2>/dev/null)" != 'brief from pinned commit' ] ||
  git -C "$FRESH" submodule status -- .shared-skills | grep -q '^-'; then
  echo "✗ a fresh worktree did not expose both files from the pinned commit" >&2
  git -C "$MAIN" worktree remove --force "$FRESH" 2>/dev/null || true
  exit 1
fi
echo "✓ git worktree add exposes the pinned audit skill and auditor brief"

# Claude's foreground and isolated-agent paths bypass post-checkout. Model
# that shape and dispatch the actual configured hook command, with a launch
# root that already has skills and a different payload cwd that does not.
# Real Claude 2.1.278 probes in #855 establish both payload timings; this
# regression fixture checks our scripts/settings, not the external harness.
run_worktree_hook() {
  python3 - "$MAIN" "$1" "$2" <<'PY'
import json
import os
import subprocess
import sys

root, event, cwd = sys.argv[1:]
with open(root + "/.claude/settings.json") as f:
    groups = json.load(f)["hooks"][event]
payload = {"cwd": cwd, "hook_event_name": event}
if event == "PostToolUse":
    payload["tool_name"] = "EnterWorktree"
    groups = [group for group in groups if group.get("matcher") == "EnterWorktree"]
commands = [hook["command"] for group in groups for hook in group["hooks"]]
assert len(commands) == 1, commands
env = dict(os.environ, CLAUDE_PROJECT_DIR=root, GIT_CONFIG_COUNT="1",
           GIT_CONFIG_KEY_0="protocol.file.allow", GIT_CONFIG_VALUE_0="always")
result = subprocess.run(commands[0], shell=True, input=json.dumps(payload),
                        text=True, env=env, cwd=root)
raise SystemExit(result.returncode)
PY
}

for EVENT in PostToolUse SubagentStart; do
  NATIVE="$TMP/native $EVENT"
  git -C "$MAIN" worktree add -q --no-checkout --detach "$NATIVE" HEAD || exit 1
  git -C "$NATIVE" reset -q --hard HEAD || exit 1
  if [ -e "$NATIVE/.claude/skills/audit-merges/references/merge-auditor.md" ]; then
    echo "✗ native creation control did not bypass post-checkout" >&2
    exit 1
  fi
  if ! run_worktree_hook "$EVENT" "$NATIVE" ||
    [ "$(git -C "$NATIVE/.shared-skills" rev-parse HEAD 2>/dev/null)" != "$PIN" ] ||
    [ "$(sed -n '1p' "$NATIVE/.claude/skills/audit-merges/references/merge-auditor.md")" != 'brief from pinned commit' ]; then
    echo "✗ $EVENT did not prepare its worktree cwd at the exact pin" >&2
    exit 1
  fi
  # Repeated events must also work on the already-prepared fast path.
  run_worktree_hook "$EVENT" "$NATIVE" || exit 1
  git -C "$MAIN" worktree remove --force "$NATIVE" || exit 1
  echo "✓ $EVENT prepares the native creation shape before dependent reads"
done

# An ordinary start on an already-correct checkout must stay local. Make the
# configured source unavailable; the exact-pin/readability fast path still
# succeeds because it does not run submodule update or contact the source.
mv "$SOURCE" "$SOURCE.unavailable"
if ! "$FRESH/.claude/ensure-shared-skills.sh" "$FRESH"; then
  echo "✗ an already-prepared worktree still depended on its remote" >&2
  git -C "$MAIN" worktree remove --force "$FRESH" 2>/dev/null || true
  exit 1
fi
echo "✓ an already-prepared worktree needs no network access"

# Missing pins must reach the foreground model and the isolated child.
# SubagentStart is nonblocking: Claude ignores exit-2 stderr in the child,
# so it needs successful JSON delivery of the failure as additionalContext.
for EVENT in PostToolUse SubagentStart; do
  NATIVE="$TMP/unavailable $EVENT"
  git -C "$MAIN" worktree add -q --no-checkout --detach "$NATIVE" HEAD || exit 1
  git -C "$NATIVE" reset -q --hard HEAD || exit 1
  HOOK_OUT=$(run_worktree_hook "$EVENT" "$NATIVE" 2>"$TMP/hook-error")
  HOOK_STATUS=$?
  if [ "$EVENT" = SubagentStart ]; then
    if [ "$HOOK_STATUS" -ne 0 ] || ! printf '%s' "$HOOK_OUT" | python3 -c '
import json,sys
value = json.load(sys.stdin)
context = value["hookSpecificOutput"]
assert context["hookEventName"] == "SubagentStart"
assert "pinned worktree guidance is unavailable" in context["additionalContext"]
assert "Do not use or delegate" in context["additionalContext"]
assert value["systemMessage"] == context["additionalContext"]
'; then
      echo "✗ SubagentStart did not deliver missing-pin context to its child" >&2
      exit 1
    fi
  elif [ "$HOOK_STATUS" -ne 2 ]; then
    echo "✗ EnterWorktree failure did not signal its foreground session" >&2
    exit 1
  fi
  if ! grep -q 'pinned worktree guidance is unavailable' "$TMP/hook-error" ||
    [ -r "$NATIVE/.claude/skills/audit-merges/references/merge-auditor.md" ]; then
    echo "✗ $EVENT hid missing guidance or used another checkout" >&2
    exit 1
  fi
  git -C "$MAIN" worktree remove --force "$NATIVE" || exit 1
  echo "✓ $EVENT reports an unavailable pin without replacing owner cleanup"
done

# A new linked worktree still needs its own module checkout. With the source
# unavailable, post-checkout must fail loudly. Git has already registered the
# worktree at that point; the hook deliberately leaves owner cleanup to Git's
# lifecycle rather than deleting an owner-managed directory itself.
FAILED_ADD_OUT=$(git -C "$MAIN" \
  -c protocol.file.allow=always \
  -c core.hooksPath="$MAIN/.githooks" \
  worktree add -q --detach "$FAILED" HEAD 2>&1)
FAILED_ADD_STATUS=$?
FAILED_PHYSICAL=$(cd "$FAILED" 2>/dev/null && pwd -P) || true
if [ "$FAILED_ADD_STATUS" -eq 0 ] || [ -z "$FAILED_PHYSICAL" ] ||
  ! printf '%s\n' "$FAILED_ADD_OUT" | grep -q 'Git already created this checkout' ||
  ! git -C "$MAIN" worktree list --porcelain | grep -qF "worktree $FAILED_PHYSICAL"; then
  echo "✗ an unavailable pin did not leave a visible owner-managed failure" >&2
  git -C "$MAIN" worktree remove --force "$FAILED" 2>/dev/null || true
  exit 1
fi
git -C "$MAIN" worktree remove --force "$FAILED" || exit 1
if [ -e "$FAILED" ] ||
  git -C "$MAIN" worktree list --porcelain | grep -qF "worktree $FAILED_PHYSICAL"; then
  echo "✗ explicit owner cleanup left the failed worktree registered" >&2
  exit 1
fi
echo "✓ an unavailable pin fails audibly and explicit owner cleanup is complete"

# A checkout that cannot expose the agreed symlink is not treated as ready
# merely because the submodule directory and commit exist. The diagnostic is
# part of the contract: an agent must not silently improvise or read elsewhere.
rm "$FRESH/.claude/skills/audit-merges"
FAILURE_OUT=$("$FRESH/.claude/ensure-shared-skills.sh" "$FRESH" 2>&1)
FAILURE_STATUS=$?
if [ "$FAILURE_STATUS" -eq 0 ] ||
  ! printf '%s\n' "$FAILURE_OUT" | grep -q 'did not establish the pinned audit guidance'; then
  echo "✗ an unreadable pinned brief did not fail with a visible diagnostic" >&2
  git -C "$MAIN" worktree remove --force "$FRESH" 2>/dev/null || true
  exit 1
fi
echo "✓ an unreadable pinned brief fails audibly"

git -C "$MAIN" worktree remove --force "$FRESH" || exit 1
