---
name: pr-hygiene
description: How review, squashing, and agent definitions work in this repo. Use when opening or merging a PR, deciding squash vs merge commit, or considering adding an agent to .claude/agents/.
---

# Review happens at the merge boundary, not on the branch

GitHub Actions runs `ci.sh` as the automatic full gate for pull requests and pushes to `main`, one job per gate group (`./ci.sh <group>`), reported as a single `Full CI` check.
It runs every gate `ci.sh` lists, one `run` line each (`grep -nE '^(quick )?run ' ci.sh`), the committed Metal corpus among them since #965,
and reports every gate that failed rather than stopping at the first —
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
Byte-exact golden frames, drawn from the base and the branch, cover it: both the parts a feature PR is not supposed to reach and the ones a given rework is.
There are three sets:
the lattice's in `harmonigraph-render`'s `lattice_tests::golden`,
the spectrogram styles' (`spectrogram-style-*`) in `harmonigraph-render`'s `spectrogram/tests.rs`,
and the spectral pane's in `harmonigraph-offline`'s `golden`, which needs a whole UI frame and so cannot live beside the first two.

**A moved golden is a stated picture change**, and the PR body is where it is stated.
Nothing is committed and nothing is blessed:
`tools/golden-diff.py` draws every golden frame from the base and from the branch on one machine, byte-exactly, and names each frame that moved with its mean, max and a contact sheet.
Run it locally under `./session-lifecycle.sh run --` (about 40 s);
`Full CI` runs it in `isolated` against `HEAD^1` and uploads the `golden-diff` artifact.
When a frame moved, read its sheet, then paste the report's table into the PR body with what moved and why.
`Full CI`'s Linux summary job fails while any row of the current report is missing from the body, so a frame a later push moves, or moves again by a different amount, needs its new row;
after adding it, re-run that job alone (`gh run rerun <run> --job <job id>`), not the macOS groups.
Reading the sheet is the session's job, not Yan's:
he judges a look in the DAW, and the PR body's line is the record of what moved.
A frame reported unstable was drawn differently twice by the same build, which is nondeterminism to report, not a picture change.

The gate earns its keep on the PR that must not move the picture —
a refactor, a frame saving, a cache —
where `tools/golden-diff.py --strict` (exit 1 if anything moved) is the proof of "behaviour-preserving" this repo has, and can now be had before pushing.
On a PR that means to move the picture it is a formality, so keep those rare:

- The lattice and spectral-pane frames are drawn from the pinned look in `crates/harmonigraph-golden/look.ron`, never from the fresh defaults, so capturing a DAW look as the new defaults moves none of them.
  The `spectrogram-style-*` pair is the exception: `cloud_fixture` still fills its atmosphere, materials and stars from `harmonigraph_scene`'s defaults, so a capture that moves those moves the pair.
  A golden that moves under a capture is a fixture still reading a default: pin it rather than state it as a picture change.
  `harmonigraph_golden::LOOK` documents how the pinned look ages under a new, renamed or dropped field.
- Do not add a golden for a look still being dialled in.
  It moves on every round and proves nothing, as the starfield's did through #1136–#1457;
  claim tests carry a look while it moves, and a golden joins once it settles.

The merge audit is Yan's to start (`CLAUDE.md` and the shared `audit-merges` skill hold the rule).
PR #85 is what it catches:
12 PRs merged in one night held two real bugs, both a cache whose missing input arrived in a *different* PR.

## Squash by default; merge-commit the exception

**When Yan says "merge it", he can say it while CI is still running:** the session waits and merges once `mergeStateStatus` is clean, so he is not coming back at green to press a button.
`/review-and-merge` in Claude, or `$review-and-merge` in Codex, is the same ask with a review in front of it, and this file is the project contract it reads:
its review tool here is `code-review` through the Skill tool against `origin/main...HEAD`, its merge method is the global squash default, judged as below, and its checks are `mergeStateStatus` `CLEAN` with `Full CI` actually reported.
Codex has no `code-review`, so there the review is a fresh subagent briefed with the diff and `AGENTS.md`.

A ruleset on `main` ("main: Full CI required") requires `Full CI` on the PR's head, and no longer that the branch be up to date with `main`.
That requirement made every PR `main` had moved past merge `main` in and run `Full CI` again before it could go,
which cost #1507 two full rounds behind #1505 and #1510.
A merge queue keeps its guarantee without the loop, but GitHub offers one only on organization-owned repositories, and this one is Yan's own.
What it guaranteed is now `main`'s own run, on every push:
two PRs that each passed alone are first tested together there, and a red `main` is fixed before anything else merges.
So a PR that touches code another PR has just landed merges `main` in and lets `Full CI` run anyway, because passing alone is all its check said;
one that does not can merge as it stands.
`Metal shader assets` is NOT required: its `paths:` filter keeps it off most PRs and therefore out of the required-checks scheme.
GitHub's auto-merge waits on required checks alone, so `gh pr merge --auto --squash` would land a shader-path PR at `Full CI` green with the corpus job still running or red (#943);
`mergeStateStatus` accounts for every check, which is why it is the gate.

**The global default is to squash unless the commits are separable; what makes them separable here:** not how many there are —
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
