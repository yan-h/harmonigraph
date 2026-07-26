---
description: Review this branch's own diff with parallel subagents before opening the PR
argument-hint: [base-ref]
---

Review the diff you are about to open a PR for, before you open it.

## Why this exists

The built-in `/code-review` is user-invocable only — its frontmatter sets
`disable-model-invocation`, which the harness treats as locked, so a session
cannot reach it and should not try. That left the session half of the review
habit in the root CLAUDE.md with nothing behind it. This is what goes there.

It is not a smaller `/code-review`. It is the half a session is placed to
do: you have full context on what you just wrote, and you are biased toward
it standing. The subagents are the correction for the second half of that
sentence — they did not write this and have no stake in it.

## Range

Use `$1` as the base if given, otherwise `main`. The diff is
`git diff <base>...HEAD` — **three dots**, so it is this branch's own work
against its merge base rather than everything that has landed on `<base>`
since you branched. Two dots here would hand the reviewers a pile of other
sessions' merges and bury yours.

```sh
git diff --stat <base>...HEAD          # size the job first
git diff <base>...HEAD --name-only     # what the lenses will be pointed at
```

If the diff is trivial — a doc edit, a comment fix, a one-line constant with
an obvious test — say so and stop. Five agents over a two-line change trains
you to skim the output, and skimmed review is worse than none.

## Who does the reading

Delegate to the **`diff-reviewer`** subagent, one per lens, **all spawned in
a single message so they run concurrently.** Each gets the same range and one
lens; each returns findings only. It is read-only on purpose — it hands back
candidates, and the fix and its failing test are written here by you.

Give every agent the range command verbatim and its lens in full. Scale down
for a small diff: a self-contained change to one file does not need the
history or test-reach lenses, and dropping them is a judgement call you
should state rather than make silently.

### The lenses

1. **Conventions.** Read the root `CLAUDE.md` and any `CLAUDE.md` in the
   directories this diff touches, plus the doc comments on the types and
   functions it modifies — then check the diff against what you read, rather
   than against conventions you already believe this repo holds. Both are
   written guidance this diff has to obey, and here the doc comment is
   deliberately the primary home for an invariant, so a change that
   contradicts one is a real defect and not a style note. Quote the line you
   are holding the diff to; a convention finding that cannot cite its source
   is an opinion.

2. **Bugs.** Read the changed hunks and scan for defects in what changed.
   Stay close to the diff; do not go spelunking for context you do not need.
   Favour large over small, and drop anything that smells like a false
   positive rather than reporting it hedged.

3. **State and invalidation.** For every cache, memo, dirty flag, or
   derived value this diff adds or touches: write down what it is keyed on,
   then ask what *else* feeds the value and is missing from the key. This is
   the standing prior for this project — it is the bug that has actually
   shipped here, twice. In the same pass, check the numeric edges: unsigned
   subtraction that can underflow, an index built from a length, a
   saturating cast that silently clamps.

4. **Test reach.** For each new or changed code path, find the test that
   executes it and confirm the fixture is actually big enough to get there.
   A test whose input is too small to reach the new branch passes for the
   wrong reason and reads as coverage. Name any path this diff adds that no
   test reaches.

5. **History.** `git log` and `git blame` the lines this diff changes. Ask
   why the code was the way it was: a hunk that reverts a deliberate fix,
   re-opens a bug someone closed, or drops a guard added in response to a
   real failure is invisible to every other lens.

## Verifying, before you believe any of it

Findings from a first reading are mostly wrong, and a review that forwards
them all is noise wearing a review's format.

For each finding, spawn **one `diff-reviewer` in parallel prompted to refute
it** — give it the finding, the file, and the claimed breaking input, and ask
it to establish that the finding is *false*, defaulting to refuted when it
cannot decide. Keep what survives. Drop what is refuted, and drop what the
refuter can only call plausible.

If the lenses return more than eight findings, refute the eight most severe
and **say in your output how many you did not verify** — a truncated
verification pass that reports as a complete one is the failure mode this
whole section exists to prevent.

Deduplicate before refuting: five lenses reading one diff will land on the
same line more than once, and refuting a finding twice costs twice.

## The bar

**Nothing is a finding until you can name the input that breaks it** — the
same bar `/audit-merges` holds to. Concrete state, wrong output. Anything you
suspect but cannot make fail is a suspicion: keep it, mark it, do not fix it.
Speculative fixes to code that works are how this project loses an evening.

## Output

Report through the **`ReportFindings`** tool — one call, verified findings
first, most severe first — and do not also print them as prose. If a finding
did not survive refutation it does not go in the call.

Then fix them, here, in this session:

- One commit per finding.
- Where a test can express the defect, write it, watch it fail, then fix.
  Where it genuinely cannot — a visual change, a camera angle — say that
  outright rather than writing a test that asserts the new behaviour and
  proves nothing.
- Re-run `./ci.sh` after fixing.
- If you touched plugin-affecting code, rebuild before you pause, per the
  root CLAUDE.md, and name the build tag when you hand it over.

If nothing survives verification, call `ReportFindings` with an empty list
and say what you checked and cleared. A clean review that names its lenses is
worth something; a clean review that just says "looks good" is not.
