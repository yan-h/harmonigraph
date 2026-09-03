---
description: Audit everything merged into main since the last audit for bugs born at integration
argument-hint: [since-ref]
---

Audit the code that has landed on `main` since the last audit, looking for
bugs that no single branch could have contained.

## Why this exists rather than a per-PR review

Parallel sessions each open a branch that is correct against the `main` it
started from. A per-branch review therefore cannot see the failure mode
this project actually produces: two branches that are individually right
and wrong together. PR #85 is the worked example — 12 PRs merged in one
night, 11,393 insertions, and both bugs found were the same shape, a cache
whose invalidation missed one of its inputs because the input arrived in a
*different* PR.

So: read the combined diff, not the PRs one at a time.

## Range

Use `$1` if given. Otherwise the last audited point is the git tag
`last-merge-audit`; if that tag does not exist, audit the last 12 merge
commits into `main`.

The tag is local, so it is missing on a fresh clone — and the 12-merge
fallback is a guess that can reach back past an audit that already happened.
Before trusting it, look for a previous audit's own merge in the log and start
from there instead: PR #85 is one, and audited the night below it.

```sh
git rev-parse -q --verify last-merge-audit         # empty if never audited
git log --oneline --merges --first-parent -12      # the fallback window
git diff --stat <since>..HEAD                      # size up the job first
```

`--first-parent` matters: without it the list fills with "Merge origin/main
into worktree-X" commits, which are branches catching up to `main`, not
work arriving on it.

If the range is trivially small (one or two merges touching disjoint
files), say so and stop — the audit is not free and there is nothing for
it to find.

## Who does the reading

Delegate the survey to the **`merge-auditor`** subagent. The combined diff of
a night's merges runs to five figures of insertions, and reading it in the
main thread spends context on material that is never needed again; the agent
returns findings only. It is read-only on purpose — it hands back candidates,
and the fix and its failing test are written here, which is what keeps the
proof step from being skipped.

For a range spanning more than one subsystem, run one agent per subsystem
rather than one over everything: each gets a smaller diff and a sharper
prompt, and they run concurrently.

## What to look for, in priority order

1. **Two branches touching the same file.** `git log --format='%h %s'
   --name-only <since>..HEAD` and find the files that appear under more
   than one merge. That intersection is where integration bugs live.
2. **Caches and their invalidation keys.** Every cache added or touched in
   the range: list what it is keyed on, then ask what *else* now feeds the
   value that is not in the key. This is the bug class that has actually
   bitten this project twice.
3. **Conflict resolutions taken during the merges.** A merge that resolved
   a conflict by "taking both sides" is a place where two intentions were
   stitched together by hand. Re-read those hunks.
4. **Invariants asserted in one PR, relied on by another.** A comment
   saying "X is always true here" written before another branch made X
   conditional.
5. **Tests that pass for the wrong reason.** A test whose fixture is too
   small to reach the new code path — in #85 the aggregator test pushed 14
   columns into a tier holding 2048, so no test ever reached a tier merge.
6. **Prose that has gone stale.** `.claude/` and the shell scripts assert
   invariants about the code they describe, and nothing in the build notices
   when those stop being true. Check the range against all of it:

   ```sh
   git diff --name-only <since>..HEAD -- crates/
   ls .claude/agents/*.md .claude/skills/*/SKILL.md .claude/commands/*.md
   ```

   Cross-reference against the paths each file names. Prose carrying last
   month's invariants is the same defect class as a cache with a missing
   invalidation key — it is just one whose consumer is a future session
   rather than a frame. Report drift as a finding; the fix is an edit to the
   file in the audit PR.

   **The skills are where this actually bites, not the agents.** By
   `pr-hygiene`'s own rule an agent encodes a job or a constraint and never a
   description of the code, so by construction there is little in
   `.claude/agents/` that CAN rot — and a check scoped to it comes back clean
   while saying nothing. The skills are the opposite: they are full of paths,
   crate names, commands and pane names. #114 had to hand-fix five of them two
   merges after #107 created them, and the audit of #107–#126 found three more
   there plus one in `.claude/reclaim-worktrees.sh`, against zero in
   `.claude/agents/`.

   The scripts are the highest stakes of all, because a path that stops
   matching makes one a silent no-op rather than an error. `test -e` every
   path they glob, grep or copy.

## The bar for reporting something

**A finding is not a finding until a test fails on the old code.** Write
the test, watch it fail, then fix it. Anything you suspect but cannot make
fail, describe as a suspicion under its own heading and do not fix it —
speculative fixes to code that works are how this project loses an
evening.

## Output

Open a PR (`gh pr create --draft`) whose body has three parts:

- **Findings** — one section each: what breaks, the reproduction, the fix.
  Give the real-world trigger, not just the failing assertion ("dragging
  the plugin window between a Retina display and an external monitor").
- **Also checked, clean** — the things you looked at and cleared, named
  specifically. A report with no clean list is indistinguishable from a
  shallow one, and this section is what makes the findings trustworthy.
- **Range audited** — the `<since>..HEAD` range and the PR numbers in it.

One commit per finding. Wait for GitHub Actions' Full CI check before handing
off the draft PR.

If nothing is found, say so in chat with the clean list and open no PR.

## Afterwards

Move the marker so the next audit starts where this one ended:

```sh
git tag -f last-merge-audit HEAD
```

It is a local tag on purpose — it tracks what *you* have audited, and
never needs pushing.
