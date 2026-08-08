---
description: Review this branch's own diff in one fresh subagent before opening the PR
argument-hint: [base-ref]
---

Review the diff you are about to open a PR for, before you open it.

## Why this exists

`/code-review` is user-invocable only — its frontmatter sets
`disable-model-invocation`, which the harness treats as locked, so neither a
session nor a subagent can reach it. It is billed, and it is Yan's to run.
This is what a session runs instead.

It is not a smaller `/code-review`. It is the half a session is placed to
do: you have full context on what you just wrote, and you are biased toward
it standing. The subagent is the correction for the second half of that
sentence — it did not write this and has no stake in it.

## Range

Use `$1` as the base if given, otherwise `main`. The diff is
`git diff <base>...HEAD` — **three dots**, so it is this branch's own work
against its merge base rather than everything that has landed on `<base>`
since you branched. Two dots here would hand the reviewer a pile of other
sessions' merges and bury yours.

```sh
git diff --stat <base>...HEAD          # size the job first
git diff <base>...HEAD --name-only     # what the review is pointed at
git diff <base>...HEAD --name-only | grep -E '\.(rs|sh|py)$'   # executable?
```

Keep that third command's output. It decides whether the suite runs below.

If the diff is trivial — a comment fix, a one-line constant with an obvious
test — say so and stop. An agent over a two-line change trains you to skim
the output, and skimmed review is worse than none.

## Run the suite once, here, before you spawn anything

If the executable-files grep came back non-empty, run `./ci.sh` and **paste
its output into the agent's prompt.** Red means stop and fix first — there is
nothing to review on a branch that does not build.

The agent has `Bash` and will otherwise reach for cargo itself, which is the
expensive way to learn what one run here already knows. It also gives the
review a ground truth to argue against rather than a private copy.

## Who does the reading

Delegate to **one** `diff-reviewer` subagent, spawned fresh so it carries
none of your context. Give it the range command verbatim, `./ci.sh`'s output
when the diff touches code, and all four readings below in a single prompt.
It returns findings only — it is read-only on purpose, and the fix and its
failing test are written here by you.

Ask it to work the four in one pass and to keep them separate in what it
hands back, so you can tell which reading produced which finding.

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

3. **Cache keys and numeric edges.** For every cache, memo, dirty flag, or
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

## Checking what comes back

There is no refuter pass here, so the agent's findings arrive unverified and
you are the only thing between them and a commit. Findings from a first
reading are mostly wrong, and forwarding them all is noise wearing a
review's format.

**A finding about prose is settled here, by reading.** A claim that a
comment, a doc line or a name says something the code does not is answered by
one `Read` of the function it describes, and the question is closed.

**A finding about behaviour needs the input named before you touch code.**
Construct it, run it, watch it fail. If you cannot make it fail, it is a
suspicion and it stays one.

## The bar

**Nothing is a finding until you can name the input that breaks it** — the
same bar `/audit-merges` holds to. Concrete state, wrong output. Anything you
suspect but cannot make fail is a suspicion: keep it, mark it, do not fix it.
Speculative fixes to code that works are how this project loses an evening.

## Output

Report through the **`ReportFindings`** tool — one call, most severe first —
and do not also print them as prose. Leave `verdict` unset: no verify pass
ran, and a finding that reports as CONFIRMED without one is a lie about how
much reading is behind it.

Then fix them, here, in this session:

- One commit per finding.
- Where a test can express the defect, write it, watch it fail, then fix.
  Where it genuinely cannot — a visual change, a camera angle — say that
  outright rather than writing a test that asserts the new behaviour and
  proves nothing.
- Re-run `./ci.sh` after fixing.
- If you touched plugin-affecting code, rebuild before you pause, per the
  root CLAUDE.md, and name the build tag when you hand it over.

If nothing survives, call `ReportFindings` with an empty list and say what
you checked and cleared. A clean review that names its four readings is
worth something; a clean review that just says "looks good" is not.
