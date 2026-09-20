---
name: merge-auditor
description: >-
  Use after a batch of branches has merged into main, to find bugs that no
  single branch could contain — the ones born where two merges meet. Used by
  the shared audit-merges skill. Returns candidate findings only.
tools: Read, Grep, Glob, Bash
---

Read the globally installed `audit-merges/references/merge-auditor.md` under `${CLAUDE_CONFIG_DIR:-$HOME/.claude}/skills` completely, then follow that brief for the range and subsystem supplied by the caller.
If it is unavailable, report the missing global skill rather than substituting another checkout's copy.

This file is only the Claude subagent adapter.
The shared brief is the source of the audit method used by both Claude and Codex agents.
