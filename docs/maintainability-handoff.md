# Coordinator handoff: maintainability implementation

Prepared on 2026-09-19 for one coordinator taking over the entire bounded batch.
The aim is to reduce recurring maintenance work and Yan's supervision burden.
The discovery is complete enough to act;
do not restart the audit or make new trial features to measure change cost.

## Start and authority

Read this handoff,
the current [project instructions](../CLAUDE.md),
and the [plan's accepted decisions](maintainability-plan.md#start-here).
Then inspect current main and the linked issues before dispatching work.
The last historical audit ended at `8b4edf4e`;
it is evidence tied to that revision,
not a reason to start new implementation from an old checkout.

**Launch update, 2026-09-19:** Yan has now authorized the bounded first-batch implementation and in-scope merges after checks and appropriate review.
The coordinator owns review, merge order and integration;
workers still deliver draft PRs for that coordinator.
See the [plan's current position](maintainability-plan.md#start-here);
existing issues hold live progress.

Before that launch,
Yan requested this handoff and merging documentation PR [#951](https://github.com/yan-h/harmonigraph/pull/951),
superseding the earlier request to keep that PR open.
That documentation merge did not itself launch implementation or authorize future code merges.
The launch request below is retained as the template for activating the bounded scope in one message.
If a different launch request authorizes implementation only,
deliver draft PRs and use its actual merge authority;
do not infer broader permission from the template's presence in the repository.
An explicit batch merge authorization need not be requested again for every included PR.

**Suggested launch request for Yan to send:**

> Act as the single coordinator for `docs/maintainability-handoff.md`.
> Implement its named first batch from current main,
> including the technical prerequisites and documentation corrections.
> Create separate implementation tasks with owner-managed worktrees as needed,
> and use read-only agents for investigation and review.
> You may merge the resulting in-scope PRs after the repository's checks and appropriate review pass,
> in the documented dependency order.
> Complete the combined-result verification and leave the final plugin and offline renderer built and loadable,
> without swapping my shared DAW installation.
> Do not expand into the explicitly deferred features or investigations.
> Bring me only new product tradeoffs or genuine blockers;
> keep routine implementation and integration decisions with the coordinator.

## Fixed product boundary

The following removals were explicitly accepted by Yan:

| Remove | Keep and disclose |
| --- | --- |
| Custom lattice spacing, [#952](https://github.com/yan-h/harmonigraph/issues/952) | Retain ordinary camera/zoom/label controls and numeric safety. Custom-spacing appearance loss is accepted; the measured zoom/label equivalence was for one example, not every configuration. |
| Adaptive next-note outlines, [#970](https://github.com/yan-h/harmonigraph/issues/970) | Remove the whole production feature, without a replacement guide. Preserve musical candidate selection, current configuration adoption before silence expiry, musical tests, and separate Lattice Map outlines. |
| Separate-WAV replacement, [#972](https://github.com/yan-h/harmonigraph/issues/972) | Preserve the take's own captured audio, spectrum, soundtrack, missing-file diagnostics and timing. Existing alignment of recorded audio is a separate consumer; do not delete it by name association. |
| Four dormant export fields, [#959](https://github.com/yan-h/harmonigraph/issues/959) | Remove `record_audio`, `auto_render`, `audio_path`, `audio_offset` from the dormant saved config. Active capture/render behavior and internal audio-placement math remain. |
| Playhead export, [#973](https://github.com/yan-h/harmonigraph/issues/973) | Preserve Scrolling, Whole video, ordinary live history, shared live time-window bounds and musical replay assertions. This retires a picture mode, not transport playhead handling. |
| Extra public export layouts, [#974](https://github.com/yan-h/harmonigraph/issues/974) | Retire single-pane lattice/analyzer/spiral choices and custom-file arrangements. Preserve combined arrangement, proportions, orientation, aspect/resolution, preview/export agreement and internal single-pane test compositions. Ordinary combined CLI choices were not established as unwanted. |

Also explicitly retain cold-start Metal optimization,
accurate current lattice state when reopening,
spectrogram history from while the editor was closed within current limits,
and the occasionally used Spiral pane.
Backwards compatibility is not a new constraint:
follow current project policy and the [persistence skill](../.claude/skills/persistence-contract/SKILL.md),
disclose lost saved values or whole-blob refusal,
and preserve observable errors.
Do not add compatibility shims to soften an accepted retirement.

Notes and browser-only lab retirement remain unapproved proposals in [#975](https://github.com/yan-h/harmonigraph/issues/975).
Keep both outside this batch,
along with Console and standalone.
The headless musical model and fixture generator remain active verification tools.
Also leave naming/FOV (#963/#964),
GUI-stack feasibility (#968),
parked performance campaigns and broad architecture rewrites outside the batch.
#956,
#958 and #965 already fixed their identified problems at the audit endpoint;
recheck status rather than reimplementing them.

## Work packages and sequence

These are bounded implementation packages,
not a new permanent development process.
Use the issue as the detailed acceptance/evidence record;
track live status and PR links there rather than maintaining a second task board.

| Package | Deliverable and dependency | Expected overlap to check |
| --- | --- | --- |
| A. Guidance, #966/#967/#969 | One focused documentation correction PR. Correct current claims and mark dated dispatch mechanics as superseded; preserve historical measurements. Land before workers rely on those guides. | Architecture/tuning docs and old audit plans; may overlap comments/docs later changed by outline retirement. |
| B1. Nested settings coverage, #957 | Finish the demonstrated bounded completeness check; retain conditional control scenarios. Land before spacing/range edits depend on it. | UI settings-range fixtures. |
| B2. Recording parity, #962 | Promote the focused real Recorder → display versus writer pump → file → Replay comparison. Preserve its deliberate timestamp/pitch sensitivity and stated limits. Land before the export/replay cleanup. | Recorder test support, offline replay tests and possible dev dependencies. |
| C1. Replacement audio + dormant fields, #972/#959 | One PR for the related export/config cleanup. Start after relevant B2 coverage; verify own-take audio and cropping/alignment. | Offline CLI/audio selection, take config, record requests and their tests/docs. |
| C2. Playhead, #973 | Its own PR after C1. Preserve shared live bounds and replay assertions while retiring fixed-timeline machinery. | Offline rendering/replay tests, UI runtime/spectrum/spectral panes and export config. |
| C3. Extra layouts, #974 | Its own PR after C2. Keep ordinary combined framing and minimal internal test compositions. | Offline CLI/render fixtures, UI layout/preview, prototype instructions and active scripts. |
| C4. Next-note outlines, #970 | Its own PR. Preserve config adoption and move musical test observation off the retired mailbox. Do not write concurrently with C2. | Core policy/reach, plugin Hub/editor, UI runtime and tuning/lattice panes. |
| C5. Spacing, #952 | Its own PR after B1. Remove the configurable invariant and follow it through scene derivation, loading and tests. | Current scene view modules/derive, UI settings/persist fixtures and picture consumers. |
| D1. Shared setting rules, #961 | Reuse existing sanitizers and bounded constants after relevant C removals settle. Avoid consolidating rules belonging to fields being deleted. | Same settings/scene files as B1 and C5; possibly render bounds from C1–C3. |
| D2. Tracing, #955/#960 | One PR: explicit fixture initialization plus complete unused tracing retirement. Preserve allocation guards, host boundary assertions and unrelated ownership hooks. | Vendored CLAP wrapper/boundary tests and patch documentation. |

B1 and B2 are candidates for parallel implementation after a fresh overlap check.
C1 and C5 may likewise be independent;
do not assume that because the issue titles differ.
C4 and C2 are explicitly sequential because they share visual-runtime ownership.
Other scheduling choices belong to the coordinator:
move a disjoint package earlier if its prerequisites hold,
but never prepare a stale branch against files another active task is changing.
Two simultaneous mutating tasks is the default ceiling for this batch;
one is preferable when the changes share owners.

## Coordinator and worker responsibilities

The coordinator owns decomposition,
fresh file-overlap checks,
review triage,
merge order,
combined-result verification and the final handoff.
Before dispatch,
refresh issue/PR status and identify any already-running task on the same files.
If an item was solved elsewhere,
validate its acceptance criteria and record the evidence instead of doing it twice.

Each mutating task has one writer and its own owner-managed worktree/branch.
Follow the current project instructions for the owner actually running it.
In Codex,
use separate app tasks for separate implementation worktrees;
subagents inside one task share its directory and are read-only helpers.
The coordinator can own a package itself in its own managed worktree,
but must not share that checkout with another writer.
Start dependent work from the committed result of its prerequisite,
normally updated main after the preceding PR merges.
With draft-only authority,
record exact prerequisite commits and do not claim unmerged dependent work has landed.

Every worker receives the issue links,
approved loss,
retained behavior,
expected files,
base revision,
required evidence and the instruction not to widen scope.
Use faster agents for bounded inventory and mechanical checking,
and stronger agents for audio/concurrency,
persistence,
shared runtime or ambiguous review.
The coordinator verifies consequential claims;
agreement between agents is not the evidence.

Unexpected product loss or a proposed new abstraction comes back to the coordinator before expansion.
Routine fixes within scope do not need another user decision.
Unrelated measured bugs follow the repository's issue rule and stay out of the current PR unless the requested behavior cannot work without them.
Keep working on independent packages if one is blocked.

## Verification and merge discipline

Read the current project instructions and relevant skills at execution time;
do not freeze their commands or check inventories into this handoff.
The [PR-hygiene skill](../.claude/skills/pr-hygiene/SKILL.md) owns review and merge mechanics,
and [build handover](../.claude/skills/build-handover/SKILL.md) owns loading/tag details.
Use focused draft PRs and satisfy the repository's build,
formatting,
golden and Metal-corpus obligations as applicable.

Risky removals receive independent evidence-based review of the final diff.
Give reviewers the product decisions so they do not restore intentionally removed behavior.
Preserve behavior checks while deleting obsolete feature-only coverage;
existing evidence patches are dated scratch experiments,
not finished tests to apply blindly.
Check that fixtures reach the branch they claim to protect.
Picture changes need picture evidence through the retained editor/preview/export paths,
not just a successful compilation or an uninspected golden rebaseline.
No perpetual review loop or new validation platform is part of this batch.

Merge only within the launch request's actual authority,
after the current PR head's checks and appropriate review are complete.
Follow the repository's merge-state gate,
not just one workflow's friendly name.
When main moves through overlapping work,
bring the dependent branch forward and recheck the affected behavior before merging it.
After a related group lands,
inspect the combined diff and verify the affected retained workflows;
use the repository's merge-audit procedure when invoked under its current rules.

Do not install a branch into Yan's shared DAW slot.
For product-affecting work,
leave both plugin and offline renderer built in the owning worktree as required before pausing.
The final integration result needs a matching built pair and the actual load command/build tag,
not merely several independently built intermediate PRs.

## Progress, stopping point and handoff

Use existing issues for live package status,
PR links,
verified results and blockers.
Use the [plan](maintainability-plan.md) as the short entry point and update its current position at a meaningful checkpoint.
Keep historical reports as evidence tied to their source revisions;
update current instructions affected by a removal in the same implementation PR.
Do not promote an agent suggestion into a product requirement by copying it into a task brief.

Give Yan concise batch updates:
what is complete,
what is next,
and any concrete decision that cannot be resolved from the accepted scope.
Ask only for new product tradeoffs or genuine missing authority,
with a recommendation and observable consequence.
Do not repeatedly ask about the six accepted removals or already-authorized merges.

The batch is complete when the listed packages have landed with their retained behavior verified,
including any package established as already satisfied by another change.
Report an unresolved blocker and the unfinished work as incomplete,
not as a completed implementation objective.
If only draft delivery was authorized,
report that boundary explicitly.
Completion is not a line-deletion target and does not require exhausting deferred investigations.

The final coordinator report should fit in a short message:
PRs and issue status,
which maintenance obligations disappeared,
preserved-behavior evidence and its limits,
saved-state/CLI effects,
the final loadable build and any remaining decision.
Offer one combined build for Yan's ordinary play/reopen/record/export check;
do not claim that an unperformed DAW check passed.
