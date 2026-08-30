---
name: audit-merges
description: >-
  Audit a batch of merges on main for integration bugs that no branch
  contained alone. Use when Yan asks to audit recent merges or invokes the
  audit-merges workflow; do not use for ordinary per-branch review.
---

# Audit the combined merge range

Read `.claude/commands/audit-merges.md` completely and follow it as the
canonical procedure. It owns range selection, evidence standards, output,
and the `last-merge-audit` marker.

That procedure names Claude's `merge-auditor` subagent. In a session that
cannot invoke the named agent directly:

1. Read `.claude/agents/merge-auditor.md` completely.
2. Delegate the survey to an available subagent with that file as its job
   definition, plus the exact range and subsystem boundary. The delegation is
   explicitly authorized by this skill when the procedure's size check says
   an audit is warranted.
3. Keep the delegate read-only. It returns candidates, suspicions, and the
   clean list; the calling session writes each failing test and fix.

Do not translate the workflow into a normal branch review. Its subject is the
combined result of multiple merges, and the proof bar remains a test that
fails on the unaudited code.
