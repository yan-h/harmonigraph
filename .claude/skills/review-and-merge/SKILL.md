---
name: review-and-merge
description: Review a PR if its diff earns a review, fix what the review confirms, wait for green, then merge it. Started only by Yan invoking it explicitly; that invocation is his "merge it".
disable-model-invocation: true
---

# Review if it earns one, then merge

Yan typing this is the ask that `AGENTS.md`'s "nothing merges unless Yan asks" waits for, for the PR named in the arguments or, with none, the current branch's PR.
It covers that one PR, not the rest of the session.
A session never starts it for itself:
Claude keeps it off the model's list through `disable-model-invocation`, which Codex ignores, so there this line is the rule.
The `pr-hygiene` skill owns the reasoning behind each step;
read it rather than guessing at squash versus merge commit.

1. **Decide whether the diff earns `code-review`,** reading `git diff origin/main...HEAD`.
   An audio-path change, a persistence change, a cache key, a concurrency change, a shader or pipeline change, or any logic beyond a rename or a one-line fix:
   yes.
   Docs, backlog notes, config and pure-test edits:
   no.
   When unsure, review.
   Say which you decided and why in one line.
2. **Review** by invoking `code-review` through the Skill tool against `origin/main...HEAD`, never bare `main`.
   Fix every CONFIRMED finding, and fix or answer each PLAUSIBLE one in the PR body.
   Commit and push the fixes;
   a change that touches the picture owes the two-package build again.
   A finding that is a real design question, not a defect, stops here and goes to Yan instead of being merged past.
3. **Wait for `mergeStateStatus` to be `CLEAN`** on the head you just pushed, polling in this session, not a subagent.
   `CLEAN` before `Full CI` has reported is not green;
   check that `gh pr checks` lists it as passed.
   `DIRTY` means a conflict with `main` and no checks will ever run:
   merge `origin/main` in, re-run the checks you would owe, push, and wait again.
   A red check is fixed, not merged past.
4. **Merge:**
   `gh pr ready <N>` if it is a draft, then `gh pr merge <N> --squash` (or `--merge` where `pr-hygiene` says the commits are separable).
   Skip `--delete-branch`, which fails from a worktree after the merge has landed;
   delete the remote branch with `git push origin --delete <branch>`.
5. **Confirm it landed:**
   `gh pr view <N> --json state,mergeCommit` says `MERGED`, and `git fetch` then `git log --oneline -1 origin/main` shows it.
   Report the merge commit, whether it was reviewed, and what the review changed.

Codex has no `code-review`.
Invoked as `$review-and-merge` there, a diff that earns a review stops at step 1 and says so rather than merging unreviewed.
