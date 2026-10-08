---
name: pr-hygiene
description: How review, squashing, and agent definitions work in this repo. Use when opening or merging a PR, deciding squash vs merge commit, or considering adding an agent to .claude/agents/.
---

# Review happens at the merge boundary, not on the branch

GitHub Actions runs `ci.sh` as the automatic full gate for pull requests and pushes to `main`, one job per gate group (`./ci.sh <group>`), reported as a single `Full CI` check.
It runs every gate `ci.sh` lists, one `run` line each (`grep -nE '^run |^if in_group' ci.sh`), the committed Metal corpus among them since #965 —
not judgement.

**A session can run the ordinary review itself:** invoke `code-review` through the Skill tool, which is neither a typed slash command nor a Bash route to one.
It forks, runs in the background, and reports its findings back.
On PR #809 (2026-09-09) it caught a defect the issue author and the implementing session had both missed:
a same-key retrigger called `forget_voice` where the fix's own justification argued for `release_voice`, because `Sequencer::fill` re-appends released pitches into the scoring context and `forget` does not.

**Spend it at the merge boundary,** on a diff whose risk earns it, or when Yan asks.
An audio-path change, a persistence change, a cache key or a concurrency change earns it;
a docs edit, a rename or a backlog note does not.
Not mid-loop either, where it reads work that is about to be rewritten anyway.

**`--fix` needs an explicit range, or it can stage a revert.** On already-committed work with a clean tree it finds an empty diff, and on PR #592 (2026-09-03) it resolved that by checking `main`'s copy of a file into the index.
`origin/main...HEAD` runs cleanly and leaves its fixes uncommitted.
Write `origin/main` and not `main`:
a worktree's local `main` ref goes stale, so `main...HEAD` silently pulls in other sessions' merged PRs.

Do not rebuild the retired project-local `/self-review`, which ran a `diff-reviewer` subagent over `git diff main...HEAD`:
it was 11% of all credit spend across 189 runs, and returned findings no refuter had touched, triaged by the same session that wrote the code,
where `code-review` argues against each finding before anyone acts on it.

No diff review at any effort covers the class that actually slips through here, a picture change:
#453 moved the frame by a mean 3.3–3.7/255 with local swings of −90 while the suite stayed 146/0 green.
Byte-exact golden frames cover it, both the parts a feature PR is not supposed to reach and the ones a given rework is.
There are two sets:
the lattice's in `harmonigraph-render`'s `lattice_tests::golden`, and the spectral pane's in `harmonigraph-offline`'s `golden`, which needs a whole UI frame and so cannot live beside the first.

**A changed golden is a stated picture change**, and the PR body is where it is stated.
Re-baseline with `HARMONIGRAPH_BLESS=1 cargo test --workspace golden` —
`--workspace` rather than one `-p`, or the set that is not named is silently left on its old frames —
and read the contact sheet the failure names before you do:
a bless nobody looked at is the failure the gate exists to catch, not a step on the way past it.

The merge audit is Yan's to start (`CLAUDE.md` and the shared `audit-merges` skill hold the rule).
PR #85 is what it catches:
12 PRs merged in one night held two real bugs, both a cache whose missing input arrived in a *different* PR.

## Squash by default; merge-commit the exception

**When Yan says "merge it", he can say it while CI is still running:** the session waits and merges once `mergeStateStatus` is clean, so he is not coming back at green to press a button.
`/review-and-merge` in Claude, or `$review-and-merge` in Codex, is the same ask with a review in front of it, and this file is the project contract it reads:
its review tool here is `code-review` through the Skill tool against `origin/main...HEAD`, its merge method is this section's, and its checks are `mergeStateStatus` `CLEAN` with `Full CI` actually reported.
Codex has no `code-review`, so there the review is a fresh subagent briefed with the diff and `AGENTS.md`.
Not `gh pr merge --auto --squash`:
`main` carries no branch protection and no rulesets, so no check is *required*, and GitHub's auto-merge waits on required checks alone, so it would merge a mergeable PR at once rather than at green (#943).
`mergeStateStatus` is the better gate regardless, because it accounts for every check, including `Metal shader assets`, whose `paths:` filter keeps it off most PRs and therefore out of any required-checks scheme.

**Squash a PR unless its commits are separable.** The question is not how many there are —
#97 had eight and was squashed, #95 had about seven and took a merge commit.
It is whether the commits *supersede one another*.

Most of a session's commits correct earlier ones on the same branch:
#98's "four ways the drawn marks were wrong", then "round a mark's SIZE", then "make a mark a coverage bitmap" are three passes at one problem, two of them later overturned.
On `main` those read as decisions when they are revisions, and `blame` lands a future reader on reasoning that was abandoned.
Squashed, `main` carries the conclusion and the passes stay in the PR.

The exception is a branch holding changes that merely share a branch.
#95 —
remove peak hold, remove the Heat palette, the keyline change, the spectrum always analyzed —
is four decisions, each worth finding on its own, and a merge commit keeps them findable.

**What squashing costs, and what to do about it.** This repo's commit messages carry measurements and rejected alternatives, and squashing leaves that reasoning only in the PR.
So treat *"would I lose this by squashing?"* as the signal it was never safe there, and put the load-bearing why in a code comment, where squashing costs nothing.
In #98 the Iosevka stroke measurements and the atlas-quantization finding were in comments and survived intact.

**Merge commits do not break `git bisect`.** Full CI verifies the PR head and not each intermediate commit, so a merge commit puts unverified commits on `main`, but `git bisect start --first-parent` tests only merge commits and squashes, each a whole PR that passed Full CI.
Reach for that flag rather than for a reason to rewrite history.

**Leave already-merged history alone.** Retroactively squashing it would mean force-pushing `main`, invalidating every worktree built on it, and voiding every sha already written down —
the `last-merge-audit` tag, the `build … @<sha>` stamped into binaries, every PR link —
for tidiness in a log that `--first-parent` already reads cleanly.

## The agents in `.claude/agents/`

`merge-auditor` does the reading for the shared `audit-merges` skill.
It hands back candidate findings and the fix is written in the calling session, so nothing goes from "this looks wrong" to a commit without the failing test in between.
Claude's `/audit-merges` uses this Claude adapter, Codex's `$audit-merges` spawns Codex subagents directly, and both read the globally installed skill's `references/merge-auditor.md` as the one audit brief.

That split is instructed, not enforced.
The Claude adapter is granted `Read, Grep, Glob, Bash`, so `Write` and `Edit` are withheld, but **`Bash` writes files, and can commit**;
Codex subagents inherit their task's tools and rely on the brief's read-only instruction.
Shell access is retained deliberately:
the #85 audit's findings were proved with `cargo test`, `git merge-base` and `git blame`, and a reviewer that cannot run the suite cannot tell a bug from a guess.
What that costs when it goes wrong is on record:
the retired `diff-reviewer` ran `load-plugin.sh` in three separate sessions, evicting the build Yan was testing, and successively more explicit prompts did not stop it.
Narrowing `Bash` to read-only patterns would buy enforcement at the price of the audit's own evidence;
the trade is open, not settled.

One is the whole list, and the rule that keeps it short is:
**an agent encodes a job or a constraint, never a description of the code.** `merge-auditor` describes a method, and a method does not go stale when a type is renamed.

A `spectral` agent carrying the spectrogram's retention and aggregation invariants was written and then deleted before it ever ran:
every fact in it was already documented, better, on `SpectrumHistory` and `SpectrogramAgg`.
A doc comment sits in the same diff as the code it describes, so a refactor that invalidates it puts it in front of the author;
a prompt in `.claude/agents/` has no such gravity, so copying docs into one buys a second copy with worse invalidation and equal authority.
So a fact with a natural home in a doc comment goes there.
Reach for an agent when it restricts tools in a way that changes what can happen, encodes a repeatable job, or isolates genuinely noisy searching —
not when a subsystem merely feels important.
