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
git diff <base>...HEAD --name-only | grep -E '\.(rs|sh|py)$'   # executable?
```

That third command decides how much of the rest of this file you run. Keep
its output.

If the diff is trivial — a comment fix, a one-line constant with an obvious
test — say so and stop. Four agents over a two-line change trains you to skim
the output, and skimmed review is worse than none.

## Who does the reading

Delegate to the **`diff-reviewer`** subagent, one per lens, **all spawned in
a single message so they run concurrently.** Each gets the same range and one
lens; each returns findings only. It is read-only on purpose — it hands back
candidates, and the fix and its failing test are written here by you.

Give every agent the range command verbatim and its lens in full.

### Run the suite once, here, before you spawn anything

If the executable-files grep came back non-empty, run `./ci.sh` and **paste
its output into every lens prompt.** Red means stop and fix first — there is
nothing to review on a branch that does not build, and four agents would
each rediscover the same failure.

The agents have `Bash` and will otherwise each reach for cargo themselves.
Across the runs measured so far, 36 of 37 spawned agents invoked cargo, 386
times between them: five lenses clippying and testing the same tree, for the
same branch, five times over. One run here costs a fraction of that, and it
gives every lens the same ground truth to argue against instead of one
private copy per lens.

### Run the mutants once too, when the diff changes a function body

Test reach is the highest-yield lens, and every one of its best findings is
the same defect: **a test that passes for the wrong reason.** #168's render
fixture had stopped exercising the index arithmetic its own PR was about,
while staying green; #177's `ubuf_ms: 0.0` made two closures the same
subtraction, so transposing them passed the whole suite; #106's overlay test
read labels only, so a flattened nesting passed too.

`cargo mutants` proves that class rather than arguing it, and it spends no
context doing it. Scope it to the functions this diff changed — never a whole
file, which is a 14-to-16-minute run:

```sh
cargo mutants -p <pkg> -F '<fn>|<fn>' --list     # no build; get a real count first
cargo mutants -p <pkg> -F '<fn>|<fn>' -j 4 --copy-target true --minimum-test-timeout 60
```

`-F` is a regex over the names `--list` prints, which is how you scope to the
functions this diff rewrote without paying for a whole file.

**Paste the surviving-mutant list into the test-reach lens prompt**, exactly
as `./ci.sh`'s output goes to every lens. A survivor is a reached-but-unpinned
path with a line number already on it, so the lens starts from proof and
spends its budget on the half mutation cannot reach.

Four things that cost time on the first run:

- **`--list` first.** Wall-clock is the per-mutant rebuild, not the tests, so
  the count is the cost: 131 mutants over one file took 14–16 minutes on 8
  cores. Past ~40 the filter is too wide — narrow it to the functions the
  diff actually rewrote.
- **`-F` does not filter every mutant.** `delete field … from struct …
  expression` mutants come out whatever the regex says: on
  `harmonigraph-record` a pattern matching nothing at all still lists three of
  them. So a scoped list carries a few lines from functions you did not name
  — drop those before pasting, or the lens spends its pass on a path this
  branch never touched.
- **`mutants.out/` is not in `.gitignore`.** Delete it before you commit.
- **Each `-j` job leaves a ~1.5 GB scratch copy** under
  `$TMPDIR/cargo-mutants-*.tmp`, not reliably cleaned up. `rm -rf` them when
  the run ends; parallel sessions have filled this disk before.

Skip this when the diff changes no function body — docs, comments, a constant
with its test. A survivor on a line this branch did not touch is not this
review's finding: note it and move on, the same as any other pre-existing
defect.

### Which lenses to run

**Key the lens set on that grep, not on what the diff feels like.** Empty —
no `.rs`, `.sh` or `.py` touched — and the conventions lens runs alone.
Non-empty, and the four standing lenses run: conventions, bugs, state and
invalidation, test reach.

The gate is mechanical because judgement reads a rename sweep or a backlog
drain as a docs change and scales down on it. #114 was exactly that shape,
and its worst defect was two lines of shell: both loader scripts built
`lib${PKG}.dylib`, which the old underscored package name had made correct
by accident, so the rename pointed them at a file cargo does not write.

Below that gate, dropping a lens is still yours to call, but name the ones
you dropped rather than dropping them silently.

### The history lens runs only when the diff takes something away

It is the one lens whose price has never been repaid by breadth. Across
twelve measured runs it found one thing — #114's sha-anchored NIGHT-NOTES
entry, which the rename sweep rewrote to name a symbol the commit that same
entry cites does not contain — and settled one "deliberate decision or quiet
orphaning?" question. The rest of the time it returns a clean list, at a
lens's full price. That is a real service at the price of the defects only it
can see, and a poor one at the price of running every time.

So it is conditional, on the same footing as the executable grep. Both greps
look for what the diff **removes**, because an added line has no history to
read:

```sh
G='assert|clamp|saturating|checked_|unwrap_or|is_finite|\.min\(|\.max\('
rm=$(git diff <base>...HEAD | grep '^-' | grep -v '^---' | grep -cE "$G")
ad=$(git diff <base>...HEAD | grep '^+' | grep -v '^+++' | grep -cE "$G")
[ "$rm" -gt 0 ] && [ "$rm" -ge "$ad" ] && echo 'history: a guard came out'

git diff <base>...HEAD | grep '^-' | grep -v '^---' \
  | grep -E '\b[0-9a-f]{7,40}\b|#[0-9]+|20[0-9]{2}-'   # prose citing a sha, PR or date
```

Either one firing runs the lens; both quiet and it stays off, and you say so.

**The guard test is net, and that is the whole of why it works.** Counting
removed guard lines alone fires on every code diff in this repo, which clamps
and asserts everywhere: measured over nine merged PRs it said *run* nine
times out of nine and saved nothing. What separates them is the ratio — real
feature work *adds* guards (#164 removed 2 and added 30, #119 removed 4 and
added 56), while a diff that takes one away does not. `rm >= ad` with
`rm > 0` fires on #114 and stays off for seven of those nine; the one other
it fires on, #191, is a "stops doing X" removal, which is the shape the lens
exists for. #192 is that shape too and the guard test misses it (`rm=0`,
`ad=7`) — what runs the lens there is the prose grep, on the `**Issue #135**
is open` line it deleted, which is the case for keeping both.

The prose grep is deliberately loose — `defaced` is a hex word and a take id
is a long number, so it catches things that are not shas. That bias is the
right way round: a false positive costs one lens on one branch, a false
negative costs the only reading that sees a hunk quietly undoing a fix. Do
not tighten it into something that misses #114.

Building the gate out of the cases the lens has earned is deliberate. The
alternative is a guess at what it might find next, and the measurement says
the guess would come back empty ten times in twelve.

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
   test reaches. Where the prompt carries a surviving-mutant list, start
   from it — each survivor is that defect already proven, with a line number
   on it — and spend the rest of the pass on what mutation cannot model: a
   fixture that reaches the line but asserts against the wrong branch, and a
   path no mutant is generated for at all.

**History**, when the gate above opens. `git blame` the lines this diff
*modifies or deletes* — not the ones it adds, which have no history to read —
and `git log -n 5` the commits behind them. Ask why the code was the way it
was: a hunk that reverts a deliberate fix, re-opens a bug someone closed, or
drops a guard added in response to a real failure is invisible to every other
lens. Hold to that bound, which is also what its gate is keyed on.

## Verifying, before you believe any of it

Findings from a first reading are mostly wrong, and a review that forwards
them all is noise wearing a review's format.

For each finding **that would change code**, spawn **one `diff-reviewer` in
parallel prompted to refute it** — give it the finding, the file, and the
claimed breaking input, and ask it to establish that the finding is *false*,
defaulting to refuted when it cannot decide. Keep what survives. Drop what is
refuted, and drop what the refuter can only call plausible.

**A finding about prose does not get an agent.** A claim that a comment, a
doc line or a name says something the code does not is settled by reading the
code it describes: one `Read` of the function, here, and the question is
closed. An adversary buys nothing against a definite answer, and it is priced
like one that argues about behaviour. In the run this rule comes from, three
of eight refuters were on doc claims and spent 76k output tokens between them
— 54% of that whole verification pass — to conclude what the function body
says outright.

That split is the one the fix already follows: prose findings are checked
here and fixed here, code findings get an adversary first.

If the code findings run past eight, refute the eight most severe and **say
in your output how many you did not verify** — a truncated verification pass
that reports as a complete one is the failure mode this whole section exists
to prevent.

Deduplicate before refuting: four lenses reading one diff will land on the
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
