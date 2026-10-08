---
name: worktrees
description: Worktree ownership for Claude and Codex sessions, why a hand-written worktree lock is never released, and when parallel sessions are worth running. Use when creating, dispatching into, locking or cleaning up a worktree, or when planning parallel sessions.
---

# Worktrees and parallel sessions

## Who owns which worktree

- **Claude:** `.claude/worktrees/<branch>/`. `EnterWorktree` creates it and
takes the lock nothing here may take by hand;
`reclaim-worktrees.sh` prunes and removes it.
- **Codex app:** the app's managed worktree, under `$CODEX_HOME/worktrees` by
default or its configured Worktree root.
Start from the requested committed base, normally `main`, on the requested branch.
Stay there through commit, push and draft PR, using the app's approval flow for Git metadata and network access.

Keep those ownership domains separate:
Codex owns its worktrees' cleanup and snapshots, and the Claude reclaimer removes what sits under `.claude/worktrees`, so do not point Codex's Worktree root there.
A hand-made worktree outside either owner has no automatic cleanup and is not a supported session workspace.
A write-capable session that finds itself in main starts over elsewhere:
Claude through `EnterWorktree`, a Codex coordinator by sending the edit to a worktree task.

## Before running sessions in parallel, check for file overlap

Parallelism buys wall-clock only when the work is disjoint;
when three sessions converge on `harmonigraph-ui/src/lib.rs` it buys merge-order bugs instead, and `/audit-merges` is what pays for them afterwards.
Overlapping work is better run in sequence, and variants of a single decision (three takes on one fade) are better as one session producing several builds to compare than as three branches to reconcile.

A Codex coordinator creates a separate top-level app task, and therefore a separate managed worktree, for each mutating stream.
Subagents inside one task share its worktree;
use them for read-only exploration or review, not parallel edits.

## Why a hand-written lock is never released

The Claude harness locks its worktree when a session enters it and unlocks it when the session exits, and Codex owns the lifecycle of its managed worktree, so no session takes or releases a lock itself.
Every Claude releaser —
both harness exit paths, the startup sweep, and `.claude/reclaim-worktrees.sh` —
recognizes a lock only by the shape of its reason string, and the harness writes two:

```
claude session <name> (pid <n> start <date>)
claude agent <name> (pid <n> start <date>)
```

A reason that matches neither belongs to nobody, and every releaser leaves it alone rather than guess whose it is.
So a hand-written lock stands until a human runs `git worktree unlock`, and while it stands the reclaim script neither prunes `target/debug` out of the worktree nor removes it;
the instance that produced this rule pinned 2.2G (#369).

If a reason ever does have to be written, it must carry `(pid $$ start ...)` in exactly the session format above, or nothing releases it.
