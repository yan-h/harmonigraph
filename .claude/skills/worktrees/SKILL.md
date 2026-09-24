---
name: worktrees
description: Worktree ownership for Claude and Codex sessions, the Codex-companion handoff, why a hand-written worktree lock is never released, and when parallel sessions are worth running. Use when creating, dispatching into, locking or cleaning up a worktree, when handing work to Codex, or when planning parallel sessions.
---

# Worktrees and parallel sessions

The always-loaded rules are in `CLAUDE.md`:
every write happens in an owner-managed worktree, nobody runs `git worktree lock`, and the merge audit is Yan's to start.
This file is the detail behind them.

## Who owns which worktree

- **Claude:** `.claude/worktrees/<branch>/`. `EnterWorktree` creates it and
takes the lock nothing here may take by hand;
`reclaim-worktrees.sh` prunes and removes it.
- **Codex app:** the app's managed worktree, under `$CODEX_HOME/worktrees` by
default or its configured Worktree root.
Start from the requested committed base, normally `main`, and create the requested branch (`codex/<slug>` by default) before the first edit because a managed worktree begins detached.
Stay there through commit, push and draft PR, using the app's approval flow for Git metadata and network access.
Codex owns its cleanup and snapshots, so the Claude reclaimer deliberately leaves it alone.

Keep those ownership domains separate:
do not point Codex's Worktree root at `.claude/worktrees`.
A hand-made worktree outside either owner has no automatic cleanup and is not a supported session workspace.
A write-capable session that finds itself in main leaves whatever is there alone:
Claude starts over through `EnterWorktree`, while a Codex coordinator sends the edit to a worktree task.

The Claude Companion handoff is not a Codex-managed task:
Codex inherits the dispatching Claude session's cwd.
Claude therefore enters its worktree before dispatching, never after;
`.claude/commands/implement-with-codex.md` makes that its first step.

## Before running sessions in parallel, check for file overlap

Parallelism buys wall-clock only when the work is disjoint;
when three sessions converge on `harmonigraph-ui/src/lib.rs` it buys merge-order bugs instead, and `/audit-merges` is what pays for them afterwards.
Overlapping work is better run in sequence, and variants of a single decision (three takes on one fade) are better as one session producing several builds to compare than as three branches to reconcile.

A Codex coordinator creates a separate top-level app task, and therefore a separate managed worktree, for each mutating stream.
Subagents inside one task share its worktree;
use them for read-only exploration or review, not parallel edits.

## The merge audit is Yan's to start

He types `/audit-merges` or `$audit-merges`;
when a batch looks worth auditing, the most a session does is say so in its reply.
That covers reading the procedure out of `SKILL.md` and running it by hand, and pointing `merge-auditor` subagents at a range directly —
both are the same act with the invocation skipped.

The skill carries `disable-model-invocation: true`, so in Claude it is not offered to the model at all.
Codex ignores that field and reaches a skill by reading its `SKILL.md`, so there the rule is the first paragraph of a file it reads in full, plus the line in `CLAUDE.md`.

## Why a hand-written lock is never released

The Claude harness locks its worktree when a session enters it and unlocks it when the session exits, while Codex owns the lifecycle of its managed worktree.
There is nothing for either session to do here.
Every Claude releaser —
both harness exit paths, the startup sweep, and `.claude/reclaim-worktrees.sh` —
recognizes a lock only by the shape of its reason string:

```
claude session <name> (pid <n> start <date>)
```

A reason that does not match that belongs to nobody, and all of them are right to leave it alone rather than guess at whose it is.
That makes a hand-written lock the one lock here that NOTHING can release:
it stands until a human runs `git worktree unlock`, and while it stands the worktree is invisible to the reclaim script's prune and remove tiers alike —
`target/debug` is never pruned out of it and the worktree itself is never removable.
The instance that produced this rule pinned 2.2G behind a lock that only a human could clear (#369).

If a reason ever does turn up, the string has to carry `(pid $$ start ...)` in exactly the format above, or it never comes back.
