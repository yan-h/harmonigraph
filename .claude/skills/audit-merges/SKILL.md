---
name: audit-merges
description: Audit a batch of merges into main for defects born where otherwise-correct branches interact. Use after several branches have landed together, not as a per-PR review.
---

# Audit the combined merge range

Audit the code that has landed on `main` since the last audit, looking for bugs that no single branch could have contained.

Parallel sessions each open a branch that is correct against the `main` it started from.
A per-branch review therefore cannot see the failure mode this project actually produces:
two branches that are individually right and wrong together.
PR #85 is the worked example —
12 PRs merged in one night, 11,393 insertions, and both bugs found were the same shape:
a cache whose invalidation missed an input because the input arrived in a different PR.

Read the combined diff, not the PRs one at a time.

## Choose the range

Use the first ref supplied after the invocation when one is present.
In Claude that argument is `$0`;
in Codex it is the text following `$audit-merges`.
Otherwise use the local git tag `last-merge-audit`.
If that tag does not exist, inspect the last 12 first-parent merge commits into `main`.

The tag is local, so it is missing on a fresh clone —
and the 12-merge fallback can reach back past an audit that already happened.
Before trusting the fallback, look for a previous audit's own merge in the log and start from there instead:
PR #85 is one, and audited the night below it.

```sh
git rev-parse -q --verify last-merge-audit         # empty if never audited
git log --oneline --merges --first-parent -12      # the fallback window
git diff --stat <since>..HEAD                      # size up the job first
```

`--first-parent` matters:
without it the list fills with "Merge origin/main into worktree-X" commits, which are branches catching up to `main`, not work arriving on it.

If the range is trivially small —
one or two merges touching disjoint files —
say so and stop.

## Use this host's own subagents

Read [the merge-auditor brief](references/merge-auditor.md) completely before delegating.
Delegate the survey through the invoking host's native subagent mechanism:

- In Claude, use the `merge-auditor` subagent.
- In Codex, spawn Codex subagents directly and include the auditor brief,
the selected range, and the assigned subsystem in each task.

Never invoke another agent product, model host, or CLI to perform the audit.
A Codex invocation is performed entirely by Codex agents;
a Claude invocation is performed entirely by Claude agents.

For a range spanning more than one subsystem, run one subagent per disjoint subsystem concurrently rather than one over everything.
Subagents share the task's worktree and are read-only for this survey:
they return candidate findings and suspicions but do not edit, commit, tag, or open a PR.

## Prove and repair in the calling agent

The calling agent validates every candidate.
A finding is not a finding until a test fails on the old code.
Write the test, observe the failure, then fix it and observe it pass.

Anything that cannot be made to fail stays under a separate **Suspicions** heading and is not fixed.
This separation prevents speculative repairs and keeps the proof step from being skipped.

## Report the result

If there are findings, open a draft PR whose body has three parts:

- **Findings** — one section each covering what breaks, the reproduction, and the fix.
Give the real-world trigger, not only the failing assertion.
- **Also checked, clean** — name the areas and hypotheses that were examined and cleared.
- **Range audited** — record the `<since>..HEAD` range and the PR numbers in it.

Use one commit per finding and wait for GitHub Actions' Full CI check before handing off the draft PR.

If nothing is found, report the range and the specific clean list in chat and open no PR.

After the audit, move the local marker so the next audit starts where this one ended:

```sh
git tag -f last-merge-audit HEAD
```

The tag is local on purpose and must not be pushed.
