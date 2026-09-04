---
name: delegate-with-subagents
description: Orchestrate a selected problem with subagents when independent, bounded workstreams can improve speed, quality, or context isolation. Invoke explicitly when discretionary delegation is wanted.
---

# Delegate with subagents

Assess the task before delegating.
Use subagents when the work divides into independent, bounded workstreams and the likely gain in elapsed time, review quality, or context isolation exceeds the coordination and token cost.
Continue locally when the work is small, sequential, tightly coupled, or likely to converge on the same files.

Use the smallest useful set of agents.
Give each agent a concrete scope, the context it needs, its allowed side effects, and the result it must return.
Avoid duplicating work unless independent comparison or review is itself useful.
Use in-session subagents rather than creating standalone user-owned tasks unless the user explicitly asks for separate tasks.

Choose each agent's available model and reasoning effort in proportion to its work:

- Prefer a fast, economical model with low effort for narrow lookup, inventory, mechanical checking, or summarization.
- Prefer a balanced model with medium effort for ordinary exploration, test analysis, and well-scoped implementation.
- Prefer the strongest suitable model with high or greater effort for ambiguous design, complex debugging, security or correctness review, and difficult synthesis.
- Inherit the parent configuration when changing it has no material benefit.
- Use only models and effort levels available in the current environment.

Prefer parallel read-heavy work such as exploration, triage, testing, log analysis, research, and independent review.
Respect the host's isolation model and all repository instructions for edits.
When agents share a worktree, do not assign overlapping writes; use read-only agents or sequence the editing work unless file ownership is clearly disjoint and local guidance permits parallel edits.

Delegation does not expand the user's authorization, permissions, or task scope.
A subagent may delegate further under these same rules when that creates useful independent parallelism and concurrency remains available.

Keep useful local work moving while agents run.
Collect the relevant results, reconcile disagreements, verify consequential claims or edits, and synthesize one coherent answer.
The parent agent remains responsible for completeness and correctness.
If subagents are unavailable or fail, continue locally when possible rather than blocking solely on delegation.
