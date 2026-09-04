---
name: merge-auditor
description: >-
  Use after a batch of branches has merged into main, to find bugs that no
  single branch could contain — the ones born where two merges meet. Used by
  the shared audit-merges skill. Returns candidate findings only.
tools: Read, Grep, Glob, Bash
---

Read `.claude/skills/audit-merges/references/merge-auditor.md` completely, then follow that brief for the range and subsystem supplied by the caller.

This file is only the Claude subagent adapter.
The shared brief is the source of the audit method used by both Claude and Codex agents.
