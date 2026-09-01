---
name: pr-hygiene
description: How review, squashing, and agent definitions work in this repo. Use when opening or merging a PR, deciding squash vs merge commit, or considering adding an agent to .claude/agents/.
---

# Review happens at the merge boundary, not on the branch

Nothing mechanical blocks a merge here: GitHub Actions is disabled on the
repo and branch protection is not available on this plan, so `ci.sh` via
the `.githooks/pre-push` hook is the only automatic gate — one the hook
skips for a push that cannot change its answer — and it checks formatting,
clippy, the tests, the plugin package check, the two vendored crates' own
tests, the rustdoc doc links, the `harmonigraph-core` dependency guard,
worktree-reclaim ownership and lock cases, and the registered-worktree bundle
swap, and pre-push skip cases — not judgement. `ci.sh`'s own header is the list
to copy when this one looks stale.

**No session reviews its own branch, and it has no command to do it with.**
`/code-review` is a built-in whose frontmatter sets
`disable-model-invocation`, and the harness treats that as locked: no setting
re-enables it, and there is no Bash route to a slash command either — for the
`/code-review ultra` variant, sessions are told in so many words not to try.
It is billed, so it is Yan's to run, at the effort and on the target he picks.
The gate is per-skill and deliberate — `/simplify` sits beside it in the same
built-in family and is model-invocable — so this is a boundary to work within,
not an oversight to work around.

A project-local `/self-review` used to fill that gap with a `diff-reviewer`
subagent over `git diff main...HEAD`. It is retired, and the reasoning is
here so it is not rebuilt: it existed to be the one review a session could
start on its own, and once it became manual-trigger-only it cost a typed
command exactly like `/code-review` while returning findings no refuter had
touched, triaged by the session that wrote the code. It was 11% of all credit
spend across 189 runs. The two readings in it that were not generic — a cache
key asked in both directions, and whether a fixture is big enough to reach
the path it claims to test — moved to the root `CLAUDE.md`, where they shape
code as it is written rather than catching it afterwards.

Nothing agent-shaped replaces the per-branch half, because the class that
actually slips through here is a picture change and no diff reader sees one
at any effort: #453 moved the frame by a mean 3.3–3.7/255 with local swings
of −90 while the suite stayed 146/0 green. What replaces it is mechanical —
byte-exact frames, covering both the parts a feature PR is not supposed to
reach and the ones a given rework is. There are two sets, drawn by different
paths: the lattice's in `harmonigraph-render`'s `lattice_tests::golden`, and
the spectral pane's in `harmonigraph-offline`'s `golden`, which needs a whole
UI frame and so cannot live beside the first.

**A changed golden is a stated picture change**, and the PR body is where it
is stated. Re-baseline with `HARMONIGRAPH_BLESS=1 cargo test --workspace
golden` — `--workspace` rather than one `-p`, or the set that is not named is
silently left on its old frames — and read the contact sheet the failure names
before you do: a bless nobody looked at is the failure the gate exists to
catch, not a step on the way past it.

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

`merge-auditor` does the reading for `/audit-merges`. It hands back candidate
findings and the fix is written in the calling session. That split is the
point: it does not go from "this looks wrong" to a commit without the failing
test in between.

Be precise about how much of that is enforced, because it is easy to read as
more than it is. It is granted `Read, Grep, Glob, Bash`: `Write` and `Edit`
are withheld, but **`Bash` writes files, and can commit**. So the split is
instructed, not enforced — the prompt tells it to return findings, and nothing
stops it doing otherwise. `Bash` is granted deliberately: the #85 audit's
findings were proved with `cargo test`, `git merge-base` and `git blame`, and
a reviewer that cannot run the suite cannot tell a bug from a guess. What that
costs when it goes wrong is on record: the retired `diff-reviewer` ran
`load-plugin.sh` in three separate sessions, evicting the build Yan was
testing, and successively more explicit prompts did not stop it. Narrowing
`Bash` to read-only patterns would buy enforcement at the price of the audit's
own evidence; the trade is open, not settled.

One is the whole list, and the rule that keeps it short is worth stating:
**an agent encodes a job or a constraint, never a description of the code.**
`merge-auditor` describes a method, and a method does not go stale when a type
is renamed.

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

CLAUDE.md owns when a PR is required; this file owns what happens at the
merge boundary after one exists. An agent without `/audit-merges` runs the
same procedure out of `.claude/commands/audit-merges.md` rather than
substituting a per-branch review for it.
