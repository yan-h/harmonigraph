---
name: pr-hygiene
description: How review, squashing, and agent definitions work in this repo. Use when opening or merging a PR, deciding squash vs merge commit, or considering adding an agent to .claude/agents/.
---

# Review happens at the merge boundary, not on the branch

Nothing mechanical blocks a merge here: GitHub Actions is disabled on the
repo and branch protection is not available on this plan, so `ci.sh` via
the `.githooks/pre-push` hook is the only automatic gate, and it checks
clippy and tests — not judgement. Review is therefore a habit, in two
halves, and each half catches a class the other cannot.

**Sessions: review your own diff before you open the PR.** Not with
`/code-review` — that is a built-in whose frontmatter sets
`disable-model-invocation`, and the harness treats that as locked: no
setting re-enables it, and there is no Bash route to a slash command
either — for the `/code-review ultra` variant, sessions are told in so
many words not to try. It is billed, so it is Yan's to run. The gate is
per-skill and deliberate — `/simplify` sits beside it in the same built-in
family and is model-invocable — so this is a boundary to work within, not
an oversight to work around.

Run **`/self-review`** instead. It is the project-local command that goes
where `/code-review` cannot: it fans `diff-reviewer` subagents across
`git diff main...HEAD`, one per lens — conventions, bugs, invalidation,
test reach, history — then spawns a refuter per candidate and keeps only
what survives. A session has full context on what it just wrote, which
makes this cheap; it is also biased toward its own work, which is what
the subagents correct for, since they did not write it. This half catches
the bugs that live entirely inside one branch — a stale invalidation key,
an underflow, a test whose fixture never reaches the new path.

**Yan: run `/audit-merges` after a batch of merges lands.** Parallel
sessions produce branches that are each correct against the `main` they
started from, so the interesting bugs are the ones that do not exist until
two of them are combined — and a per-branch review is structurally blind to
those. PR #85 is the worked example: 12 PRs merged in one night, two real
bugs, both of them a cache whose missing input arrived in a *different* PR.
The command reads the combined diff and keeps a `last-merge-audit` tag so
consecutive audits do not re-read the same range.

## Squash by default; merge-commit the exception

**Squash a PR unless its commits are separable.** The question is not how
many there are — #97 had eight and was squashed, #95 had about seven and
took a merge commit. It is whether the commits *supersede one another*.

A session usually pursues one idea and arrives at it, so most of its
commits correct earlier ones on the same branch: #98's "four ways the drawn
marks were wrong", then "round a mark's SIZE", then "make a mark a coverage
bitmap" are three passes at one problem, two of them later overturned. On
`main` those read as decisions when they are revisions, and `blame` lands a
future reader on reasoning that was abandoned. Squashed, `main` carries the
conclusion and the passes stay in the PR.

The exception is a branch holding changes that merely share a branch. #95 —
remove peak hold, remove the Heat palette, the keyline change, the spectrum
always analyzed — is four decisions, each worth finding on its own, and a
merge commit keeps them findable.

**What squashing costs, and what to do about it.** This repo's commit
messages carry measurements and rejected alternatives, and condensing them
leaves that reasoning only in the PR. So treat *"would I lose this by
squashing?"* as the signal it was never safe there: a fact whose only home
is a commit message is already weakly anchored, because nothing puts it in
front of the next person to touch the code. The same argument the agents
section makes for doc comments applies here — put the load-bearing why in a
comment and squashing costs nothing. In #98 that is what happened: the
Iosevka stroke measurements and the atlas-quantization finding are in
comments and survived intact.

**Not a bisect argument, though it looks like one.** `.githooks/pre-push`
gates the push and not each commit, so intermediate commits on any branch
are unverified and a merge commit puts them on `main`. That sounds like it
breaks `git bisect`, and it does not: `git bisect start --first-parent`
tests only merge commits and squashes, each of which is a whole PR that
passed `ci.sh` on its way up. Reach for that flag rather than for a reason
to rewrite history.

**Which is also why the history behind this rule is left alone.**
Retroactively squashing what has already merged would mean force-pushing
`main`, invalidating every worktree built on it, and voiding every sha
already written down — the `last-merge-audit` tag, the `build … @<sha>`
stamped into binaries, every PR link. It would buy tidiness in a log that
`--first-parent` already reads cleanly. This rule is for the next PR, not
the last thirty.

## The agents in `.claude/agents/`

`merge-auditor` does the reading for `/audit-merges`; `diff-reviewer` does it
for `/self-review`, one instance per lens. Both hand back candidate findings
and the fix is written in the calling session. That split is the point:
neither goes from "this looks wrong" to a commit without the failing test in
between.

Be precise about how much of that is enforced, because it is easy to read as
more than it is. Both are granted `Read, Grep, Glob, Bash`: `Write` and
`Edit` are withheld, but **`Bash` writes files, and can commit**. So the split
is instructed, not enforced — the prompt tells them to return findings, and
nothing stops them doing otherwise. `Bash` is granted deliberately: the #85
audit's findings were proved with `cargo test`, `git merge-base` and `git
blame`, and a reviewer that cannot run the suite cannot tell a bug from a
guess. Narrowing `Bash` to read-only patterns would buy enforcement at that
price; the trade is open, not settled.

Two is the whole list, and the rule that keeps it short is worth stating:
**an agent encodes a job or a constraint, never a description of the code.**
Both describe a method, and a method does not go stale when a type is
renamed. `diff-reviewer` earns its file on the constraint half — it is the
read-only half of a review whose repair half happens in the calling session,
and the lens it works is passed in per call rather than baked into it.

A `spectral` agent carrying the spectrogram's retention and aggregation
invariants was written and then deleted before it ever ran, which is the
cheaper way to learn the rule. Every fact in it turned out to be already
documented, better, on `SpectrumHistory` and `SpectrogramAgg` — a doc comment
sits in the same diff as the code it describes, so a refactor that invalidates
it puts it in front of the author, and a prompt in `.claude/agents/` has no
such gravity. Duplicating docs into a prompt buys a second copy with worse
invalidation and equal authority.

So: if a fact has a natural home in a doc comment, that is where it goes, and
a session finds it by reading. Reach for an agent when it restricts tools in
a way that changes what can happen, encodes a repeatable job, or isolates
genuinely noisy searching — not when a subsystem merely feels important.
`/audit-merges` still checks the range for drift in whatever agents do exist,
because nothing in the build can catch a prompt that has gone stale — and it
checks the skills and the scripts harder, since this rule is what keeps the
rottable facts out of the agents and puts them there instead.
