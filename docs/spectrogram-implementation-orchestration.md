# Selected spectrogram work: orchestration proposal

## Status and objective

Proposed on 2026-09-06 UTC, based on `origin/main` at `ce4ae624e3931357c2fc89521570b05bdd6f77b0`.
This document proposes execution settings;
no implementation tasks or paid implementation reviews have been launched by this scope update.
[Tracker #654](https://github.com/yan-h/harmonigraph/issues/654) and the [audited plan](spectrogram-rendering-plan.md) retain the source/evidence and acceptance details.

Implement only SG1, SG4A and SG2. The objective is a strong benefit relative to ongoing maintenance, with current performance already satisfactory.
Avoid new storage protocols, cache frameworks, staging pools, clock architectures and unrelated rendering improvements.
SG3, SG4B/C, SG5 and dedicated SG6 work are deferred and not planned, including automatic profiling campaigns.
Their deferral does not waive the targeted measurements and visual checks needed for the selected fixes.

## Suggested assignments

| Leaf | OpenAI implementation | Independent Claude review | Reason for this allocation |
|---|---|---|---|
| SG1 / #655: bounded gap recovery | `gpt-6-astra`, `high` | Opus, `high`, read-only | The edit is localized, but both rebuild paths, partial/held slabs, retained capacity and post-gap geometry need careful reasoning. |
| SG4A / #658: upload staging only | `gpt-5.6-sol`, `medium` | Opus, `medium`, read-only | The production change is narrow; alignment, ring offsets and reaching actual prepare staging dominate verification. |
| SG2 / #656: offline slice endpoints | `gpt-6-astra`, `high` | Opus, `high`, read-only | Sample-index endpoints, nonzero origin, trimming, lookahead and current replay integration are more subtle than the line count suggests. |
| Coordinator / combined candidate | `gpt-6-astra`, `high` | One final Opus review, `high` | Own the write-set boundaries, independent finding validation and verification of the combined changes against their actual base. |

These are workload-specific recommendations, not measured model rankings on these tasks.
The model IDs and efforts are available in the current app.
[Official OpenAI Astra guidance](https://developers.openai.com/api/docs/guides/latest-model?model=gpt-6-astra) describes its stronger long-task reasoning and instruction following;
[GPT-5.6 guidance](https://developers.openai.com/api/docs/guides/latest-model?model=gpt-5.6) recommends medium as a balanced starting point and additional effort when it improves quality.
Use `xhigh` only when an unresolved invariant or conflicting evidence warrants deeper analysis;
do not apply it to every implementation/review by default.
No second independent Astra review is proposed for every leaf:
the OpenAI writer validates Claude's findings against evidence, and the coordinator adjudicates the gate.

## Scheduling and ownership

Start SG1 and SG4A in two separate Codex-managed worktree tasks, each branching from the same recorded current-main commit before editing.
SG1 owns `crates/harmonigraph-ui/src/spectrogram.rs` and its focused fixtures.
SG4A owns `crates/harmonigraph-render/src/spectrogram.rs` and its focused staging/GPU fixtures.
Shared test infrastructure is not automatically part of either ownership grant;
coordinate any expansion before two writers reach it.
Keep their musical/history inputs, callback packet format, serial acknowledgement and shader math unchanged.

SG2 starts only once ownership of `crates/harmonigraph-offline/src/render.rs` and its integration fixtures is coordinated with the active replay stream, or that relevant work has integrated.
It can be developed alongside the two other spectrogram leaves when that specific boundary is clear.
The date-specific check inspected #669 `b134c39a`, #668 `2419ebef`, and #667 `353c4573`:
their cumulative tuning stack changes offline render/replay/frames/goldens and shared UI state, while dedicated spectrogram CPU/upload files remain untouched.
The companion worktree also had uncommitted changes in plugin performance files and record publication.
The selected lattice work has merged through `ce4ae624`;
its remaining optimization issues are parked, so there is no continuing lattice optimization stream to wait for.
Recheck current heads and dirty files before launch rather than relying permanently on this observation.

Parallel development must not contaminate measurements:
serialize CPU/GPU benchmarks and avoid concurrent heavy builds during timed probes.
Each task keeps its own target directory and uses the configured sccache.
Subagents within a task share a worktree and may explore/review read-only;
they do not implement separate concurrent edits.

## Scope-specific verification

| Leaf | Required evidence before review |
|---|---|
| SG1 | A fixture that reaches both warm append and cold/rung-change rebuild across long gaps; intermediate and retained allocation bounds proportional to admitted slots; unchanged short-gap/held/partial-edge behavior; sequential resume images including the first drawable callback and far-edge extent; existing CPU/GPU recovery tests and spectral goldens. |
| SG4A | A probe of actual warm `CallbackTrait::prepare` staging with zero and 1–3 dirty slabs, aligned production bins and odd generic bins; separate our allocations from driver requests; byte-equivalent GPU/full/delta output, wrap/recreation recovery and unchanged spectral goldens. No FPS acceptance threshold. |
| SG2 | Exercise the actual offline sliced-audio caller at 30/60/120 and fractional fps, nonzero audio origin, late/trimmed start and partial tail; date the true supplied final sample under the documented lookahead convention; preserve half-window lag, deterministic replay and whole-song behavior; inspect and explain intended scrolling-image changes before any golden update. |

The complete acceptance details remain in the selected component issues and audit.
Tests must reach the changed branch and check behavior rather than repeat its arithmetic.
Do not expand verification into a deferred profiling project or unrelated DSP audit.
Every plugin-affecting leaf leaves fresh release builds of both `harmonigraph-plugin` and `harmonigraph-offline` before pausing, as AGENTS.md requires.
The task builds its own artifacts and names its build tag;
Yan chooses whether to load one, and no task swaps the shared DAW slot.

## Review and integration workflow

1. Give each writer a bounded original brief and acceptance criteria. Record task/worktree, immutable base SHA, changed files and required checks.
2. Run deterministic checks and required builds first. Return ordinary failures to the writer before spending a Claude review. Commit a clean leaf head and open its draft PR.
3. Review that exact base/head pair independently with the assigned Claude model/effort. Supply the original brief, exact diff, source and verification results; ask for concrete correctness/contract defects, unnecessary state and lifetime machinery within the selected scope. Claude remains read-only and does not implement fixes or run the loader.
4. The original OpenAI writer validates every finding as fixed, rejected or deferred with source/reproduction evidence. Fix confirmed defects only, rerun affected checks/builds and commit a new head. A confirmed blocking defect cannot be silently deferred while advancing the leaf.
5. Re-review after a fix at the new immutable head, allowing at most two review/fix cycles. Escalate a persistent blocker or disagreement with its evidence; do not keep running reviewers indefinitely. Any head change invalidates the prior verdict.
6. Once leaves pass, verify a combined candidate in an owner-managed integration worktree against its recorded base and perform one final Claude Opus/high review over the full combined diff. Check the actual replay/tuning integration context used for SG2; passing isolated branches does not establish combined correctness.
7. Hand over scoped draft PRs and the combined validation/review record for Yan's merge decision. Merge order and any integration PR follow the actual branch bases; nothing merges merely because a review passed. If the final base or code changes, rerun affected validation and refresh the review target.

The cross-review-with-Claude contract supplies the immutable-head, read-only, disposition and bounded-cycle rules.
Its generic bundled helper currently has no model/effort flags;
use the existing `claude-review` runner's explicit model/effort support with the same contract, and have the coordinator record/adjudicate the verdict.
Do not claim a helper's default run used the requested settings or modify a shared skill simply to start this work.
This proposes three initial leaf reviews and one combined review, plus only the bounded re-reviews required by confirmed fixes.
No additional proposal-review round is needed for the unchanged audit ideas.
