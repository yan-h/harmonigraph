# Historical cost of ordinary changes

Date: 2026-09-19.
Discovery checkpoint for the [maintainability plan](maintainability-plan.md),
in draft PR #951.
Yan requested historical analysis instead of making trial changes,
and approved future Playhead and extra public export-layout retirement.
No product changes,
new runtime experiments or builds were performed.

## Decision brief

The history supports targeted simplification and stronger checks at existing boundaries,
not a general architecture rewrite.
The clearest recurring obligations are keeping setting rules and hostile-input fixtures synchronized,
and preserving recording semantics across callback,
publication,
file and display boundaries.
Both already have concrete issues.
Some apparent hotspots are generated artifacts or moves;
some substantial cross-file edits are the necessary shape of a wanted feature.

Recommended implementation candidates remain:

- Retire the accepted capabilities in the [plan](maintainability-plan.md),
including Playhead and extra single-pane/custom public export layouts,
while retaining the normal combined controls.
History supplies an actual example of unused feature obligations in the tuning change below.
- Reuse [#957](https://github.com/yan-h/harmonigraph/issues/957)
and [#961](https://github.com/yan-h/harmonigraph/issues/961) for the demonstrated settings-maintenance burden.
Preserve useful conditional-UI checks;
do not replace them with a new settings framework.
- Reuse [#962](https://github.com/yan-h/harmonigraph/issues/962) for the focused Recorder-to-display/file-replay comparison already demonstrated by earlier probes.
Do not merge semantically different queues or representations to reduce file count.
- Keep Console and standalone at this checkpoint;
Notes and browser-only lab retirement remain bounded product proposals in [#975](https://github.com/yan-h/harmonigraph/issues/975).
The [completed consumer trace](feature-retirement-discovery.md) preserves their concrete losses and evidence limits.

No new bug or implementation issue was established by this historical pass.
Existing issues retain the reproduction work;
this report adds prioritization evidence rather than another copy of the backlog.
Implementation remains deferred until Yan requests it.

## Population and method

The [complete inventory](evidence/maintainability-discovery/history-inventory.md) covers the 100 most recent first-parent changes ending at `8b4edf4ea63669c6baf55f1878d917470b0ce4fa`,
also the remote main revision when checked.
The first included change is `723f9f2d6adb401a7b572eae16ea263ac461bad8`;
the dates span 2026-09-09 through 2026-09-19.
The product source in this documentation branch remains at `1786c5ba`,
so endpoint checks read the named main revision directly without merging or changing this branch's product code.

Every change received a file/line inventory.
Two read-only agents traced visual/settings and recording/state histories;
the coordinator checked consequential claims,
historical PR descriptions,
current issue status and selected endpoint callers.
A further read-only investigation finished the developer-tool consumer check.
Deep reads deliberately include large features,
local successes,
explicit repair chains and structural moves.
They are a selected causal sample,
not an estimate of the proportion of all changes that are defective.

Counts use a first-parent diff with explicit 50% rename detection.
Among the 100 changes,
87 touch crate Rust or WGSL,
including tests and comments.
Of those 87,
26 touch one to three such files,
40 touch four to nine,
and 21 touch ten or more;
the median is six files and 278 inserted-plus-deleted lines.
These numbers do not measure agent time,
human attention or architectural quality.
JavaScript,
vendor code,
fixtures and build tooling are outside that particular source count,
but remain meaningful in the semantic investigation.

Generated assets and source moves need separate interpretation:

- [#945](https://github.com/yan-h/harmonigraph/pull/945),
`e57fd575`,
changes one shader comparison and its explanatory comment:
11 Rust/WGSL churn lines in one file.
The normal rename-aware diff spans 15 files because the golden and Metal corpus also move.
A no-rename path inventory reports 27 files and 7,469 text lines for the same change;
that is a counting artifact,
not thousands of independently maintained decisions.
- [#849](https://github.com/yan-h/harmonigraph/pull/849),
`cb824074`,
has 10,028 crate-source churn lines even with rename detection,
but is a move-only recorder-module extraction.
[#958](https://github.com/yan-h/harmonigraph/pull/958),
`188a171f`,
likewise splits the old `view.rs` into existing shapes without changing its callers.
Their apparent size is not evidence that adding an ordinary feature requires writing that much code.

Squashed history does not expose every abandoned attempt,
and commit timestamps are not hours spent working.
Later same-file edits are not automatically repairs.
Newer changes have less subsequent history in which a repair could appear.
Current source was checked before carrying a historical burden forward.

## What the histories establish

### Settings: a repeated checklist is the strongest remaining example

`settings_ranges.rs` changes in 18 of the 100 entries,
more than any other crate Rust/WGSL path.
Frequency locates the workflow;
the causal evidence is more specific.
[#862](https://github.com/yan-h/harmonigraph/pull/862),
`79531920`,
introduced a deliberately manual scenario,
poisoned-field and visit inventory.
New controls must be represented there as well as in the model,
loading and UI.

The cloud features in #888/#909/#913/#918/#928/#933 changed or added nested settings while their hostile-input list remained at the older eight fields.
[#937](https://github.com/yan-h/harmonigraph/pull/937),
`a505cdcb`,
filled those concrete omissions.
The endpoint therefore does test the existing cloud settings;
the remaining problem is that a future nested omission is still not detected by the top-level completeness guard from #910.
Earlier discovery reproduced that failure and a bounded remedy in #957.
Do not mistake the historical omitted values for still-missing current values.

[#933](https://github.com/yan-h/harmonigraph/pull/933),
`24952475`,
is a useful ordinary range-change example:
it changed four source files and gave the two cloud-size controls and load clamps shared endpoints.
Those particular copies are already consolidated.
Other bounded same-purpose range pairs remain in #961;
wider renderer defenses and dependent limits serve different purposes.

The many-file visual features #884/#888/#909 are weaker ordinary-change baselines:
they added or redesigned GPU effects across model,
controls,
renderer,
shader,
tests and generated assets.
#888's history records requested visual iterations,
not a chain of bugs merely because the picture changed several times.
Likewise #944's exhaustive field fixtures intentionally make future additions demand a verification update.
That protective co-change is not duplication to delete.

### Recording: the difficult part was agreeing on what happened

The #817 → #828 and #838 → #848 histories expose concrete boundary costs:

| Change | Evidence and interpretation |
| --- | --- |
| [#817](https://github.com/yan-h/harmonigraph/pull/817), `2fb4423e` | At-bar completion crosses audio timing, saved settings, UI and closed-window completion. Same-PR review corrected host/UI bar numbering and whether finishing precedes an owed split. |
| [#828](https://github.com/yan-h/harmonigraph/pull/828), `c5be24b9` | Direct repair of the newly ordinary arm/jump-back/play workflow: a split created an empty pass the configuration side could never close, so Stop waited forever. |
| [#838](https://github.com/yan-h/harmonigraph/pull/838), `8c1a8e35` | Unifies captured-note truth across plain/configured input and two completion paths. Its PR explicitly accepts the loss of automatic completion for audio-only exports on one path. |
| [#848](https://github.com/yan-h/harmonigraph/pull/848), `cb2728ab` | Replaces the content gate with callback continuity and restores audio-only completion. Variable callback sizes, transport restore and delayed publication require distinct fixtures. |
| [#849](https://github.com/yan-h/harmonigraph/pull/849), `cb824074`; [#850](https://github.com/yan-h/harmonigraph/pull/850), `bb732a6c` | Separate existing modules, then extract a pure lifecycle transition. They localize policy without removing necessary clocks or callback/publication ownership. |
| [#901](https://github.com/yan-h/harmonigraph/pull/901), `58a641ab` | Production and test writer pumps had different failure, Stop and disconnect behavior. Sharing `Pump::pass` removes duplicated execution semantics, while keeping the distinct real-thread and test adapters. |

At the inspected endpoint,
callback continuity,
bar history,
configuration closure,
file completion and editor/background debounce still answer different questions.
The history justifies their separation and the existing centralized owners;
it does not justify another general recorder rewrite.

Note history gives a second causal example.
#911 (`f92c6133`) made gaps use normal release.
#946 (`c5aad48c`) then repaired a newly reachable duplicate release/early-history path when a baseline recovered a held note.
#953 (`1786c5ba`) deliberately removed the extra waiting state from that repair and documented a rare residual.
That last step is an accepted simplification,
not a regression to rediscover or an obligation to restore the deleted state.
#919 (`ff9bf009`),
which unified release-time visibility for drawing and pruning,
is a separate pre-existing mismatch rather than another repair caused by #911.

The remaining opportunity is the narrow boundary comparison in #962:
real Recorder input → live display versus actual writer pump → take file → `Take::read` → Replay.
Earlier discovery already demonstrated sensitivity to changed timestamps and lost exact pitch.
Promoting that focused evidence would protect meaningful trajectories without claiming complete DAW,
audio or visual equivalence.

### Preview: real coordination, but no new refactor yet

#820 (`5938548a`) reused the picture code for interactive export layout.
#821 (`7139bacf`) threaded a preview scale through the spectral pane so fixed-point ribbon hairlines scale with the frame.
#823 (`4cba72c7`) repaired the mistaken upper clamp:
an enlarged preview needed a scale greater than one.
Its regression deliberately reaches both small and enlarged previews.
#827 (`793ab14b`) adds requested arrangement/orientation interaction;
it is not a repair just because it touches the same owners.

The current preview still passes navigation and scale context and temporarily scales/restores shadow settings around the shared draw.
That is a coordination point to check when a future picture change reaches it,
but this cluster alone does not justify a new renderer abstraction or issue.
The accepted extra-layout retirement must preserve this wanted combined-preview path.

### Unused features: a concrete historical obligation

[#865](https://github.com/yan-h/harmonigraph/pull/865),
`829caaf8`,
changed adaptive pitch cost across 20 files.
Besides the musical selector,
configuration and tests,
it also changed `policy/reach.rs` for next-note outlines and the browser lab's controls/presentation.
Those are actual same-change obligations attached to features Yan does not use.
Removing the accepted outlines retires one of them;
browser-only retirement remains a separate proposal.

The JavaScript model and fixture exporter serve a different purpose.
[#956](https://github.com/yan-h/harmonigraph/pull/956),
`129b7503`,
updated examples,
the generator,
committed fixtures and Rust replay together so every declared setting is carried and the unsnapped branch is reached.
That active reference consumer should remain.
Its duplication can find disagreement only if its fixture reaches the intended policy;
#956 is the existing repair,
not a new issue from this audit.

## Counterexamples and already-completed work

Several histories argue against using file count as a refactoring target:

- #882 (`95278984`) fixes note-name lifetime and tests four orientations within one source file.
- #830 (`2153be1d`) adds Whole video through nine source files and 176 churn lines.
Persisting,
showing and applying the option are legitimate separate responsibilities;
no causal follow-up repair was found in this window.
- #833 (`73475921`) touches only two files,
but its verification required real encoder behavior,
color metadata and audio timing.
A small diff does not imply a cheap correctness question.
- #932 (`0c242167`) shares one identical pass-aging rule across six caches while retaining their distinct access patterns.
That is evidence for consolidating repeated policy,
not for collapsing all state that happens to have similar fields.
- #942 (`37157f9d`) reuses an existing document revision for a new cache rather than creating a second counter.
Its test distinguishes cached and rebuilt geometry.
The wider key's occasional rename/reorder rebuild is explicitly bounded;
the code does not require a perfectly minimal key at any cost.

The current endpoint already contains the view-file split (#958),
simulator-fixture repair (#956) and local strict Metal-corpus check (#965,
closing #947).
Do not repeat those repairs based on older audit text.
The cold-start Metal corpus itself is explicitly retained by Yan.

## Checkpoint and remaining work

This completes the recommended historical batch and developer-consumer investigation.
The approved removals are recorded;
Notes/browser-only retirement are the remaining small optional product choices.
Console and standalone can stay without another investigation round now.
There is sufficient evidence to select focused implementation when requested.

Lower-priority discovery remains independently returnable:
concrete naming/FOV comparisons in #963/#964,
and GUI-cohort feasibility in #968.
Neither blocks this checkpoint.
Before implementing selected changes,
map their retained workflows to existing tests/recordings and close any specific uncovered path;
do not make a new comprehensive reference platform a prerequisite.
Parked performance projects and unknown historical conversation provenance remain parked unless new evidence makes them relevant.
