# Long-term maintainability

Last updated: 2026-09-19.
Status: the ten-package first batch is complete; the technical follow-up is tracked in #954.

## Start here

**Aim:** make Harmonigraph easier for future agents to change correctly,
while reducing the attention Yan must spend understanding and supervising those changes.

**Resume here:** read the [first-batch completion record](https://github.com/yan-h/harmonigraph/issues/954#issuecomment-5746459598) and the [next-session scope](https://github.com/yan-h/harmonigraph/issues/954#issuecomment-5746476887).
The [single-coordinator handoff](maintainability-handoff.md) is the completed first batch's historical implementation contract;
do not relaunch its package list or reuse its launch template as new authority.
The follow-up covers worktree skill preparation (#855),
allocation-probe reporting (#971),
current entry points and bounded GUI dependency feasibility (#968).
Start from current main,
check issue/PR status and active file ownership,
and follow the actual continuation request for implementation and merge authority.

**Current position:** the recommended historical change-cost analysis and developer-tool investigation are complete for this checkpoint.
The [historical report](historical-change-cost.md) inventories 100 merged changes and traces representative repairs,
ordinary successes and misleading large diffs.
It supports existing settings/test and replay-parity issues alongside the accepted removals,
not a broad architecture rewrite.
Notes and browser-only lab retirement remain optional product decisions;
Console and standalone are recommended keeps for now.
Naming/FOV comparisons remain separate investigations.
GUI-stack feasibility is now a bounded follow-up investigation,
not an approved migration.
Guidance corrections ([A, #977](https://github.com/yan-h/harmonigraph/pull/977)),
nested settings coverage ([B1, #980](https://github.com/yan-h/harmonigraph/pull/980)),
recording parity ([B2, #979](https://github.com/yan-h/harmonigraph/pull/979)) and tracing cleanup ([D2, #981](https://github.com/yan-h/harmonigraph/pull/981)) have landed.
Spacing retirement ([C5, #982](https://github.com/yan-h/harmonigraph/pull/982)) and separate-WAV/dormant export-field retirement ([C1, #983](https://github.com/yan-h/harmonigraph/pull/983)) have also landed.
Next-note outline retirement ([C4, #984](https://github.com/yan-h/harmonigraph/pull/984)) has landed.
Shared setting rules ([D1, #986](https://github.com/yan-h/harmonigraph/pull/986)) and Playhead retirement ([C2, #988](https://github.com/yan-h/harmonigraph/pull/988)) have landed.
Extra public export-layout retirement ([C3, #990](https://github.com/yan-h/harmonigraph/pull/990)) completed the named implementation batch.
Package PRs,
combined retained-workflow verification,
and the final combined plugin/offline build record belong in the existing [#954 index](https://github.com/yan-h/harmonigraph/issues/954).
The [completion record](https://github.com/yan-h/harmonigraph/issues/954#issuecomment-5746459598) records the combined verification,
its limits and the matched plugin/offline build at `6a4d80a5`.
It does not claim a live DAW acceptance check.
Yan explicitly deferred the merge audit for a later manual run;
ordinary PR review and final combined workflow checks completed with the first batch.
Existing issues hold completion evidence and follow-up status.

**Discovery history (2026-09-19, before launch):** the discovery phase systematically compared opportunities before selecting implementation work.
Spacing was a worked example of the method,
not the start of a sequence of immediate fixes.
The [broader discovery report](https://github.com/yan-h/harmonigraph/issues/954) now includes detailed checks of retained export/diagnostic machinery,
repeated settings knowledge and live-versus-recorded events.
The [reproduction evidence](evidence/maintainability-discovery/README.md) preserves the passing probes,
deliberately failing controls and their limits.
Yan subsequently asked for issues covering the leads and for further investigation.
The [follow-up index](https://github.com/yan-h/harmonigraph/issues/954#follow-up-issues) now links nine new issues,
with proposals and uncertain investigations distinguished.
Continued discovery found stale current-state documentation,
conflicting old dispatch instructions and a newly met dependency-upgrade investigation trigger.
Yan then authorized a requirements-value audit aimed directly at his stale-constraints concern,
and asked for broader coverage instead of a small representative sample.
The [requirements audit](requirements-value-audit.md) inventories the major requirement clusters,
traces consequential claims through history and current code,
and distinguishes worthwhile simplifications from constraints already retired or deliberately retained.
The GUI dependency investigation [#968](https://github.com/yan-h/harmonigraph/issues/968) remains a separate lead;
its newly met version trigger does not supersede this broader scope.
The audit found stale requirements that could revive a retired tuning protocol ([#969](https://github.com/yan-h/harmonigraph/issues/969)).
Yan directly confirmed that he does not use next-note outlines,
then approved removing that feature ([#970](https://github.com/yan-h/harmonigraph/issues/970)).
Implementation was deferred while he asked about other removable requirements.

**Historical first-batch instruction (2026-09-19 launch, now completed):** implement the handoff's named first batch,
including its technical prerequisites and documentation corrections,
and merge its in-scope PRs after the repository's checks and appropriate review.
Deferred features and investigations stay outside the batch.
The coordinator owns integration and the final built plugin/offline pair;
the shared DAW installation is not swapped.
This follows Yan's earlier request for a complete handoff and merging [PR #951](https://github.com/yan-h/harmonigraph/pull/951),
which superseded his request to keep the discovery PR open.
The discovery session changed documentation and evidence only.
Implementation and future code merges follow the actual launch request given to the coordinator;
accepting a removal or publishing this handoff alone does not start that work.

**Accepted decisions:** Yan is comfortable removing custom lattice spacing,
including its custom-appearance capability loss,
based on [#952](https://github.com/yan-h/harmonigraph/issues/952).
He also approved removing adaptive next-note outlines,
while retaining musical tuning and separate Lattice Map outlines.
He subsequently approved removing separate-WAV replacement for take exports ([#972](https://github.com/yan-h/harmonigraph/issues/972)).
The take's own captured audio remains supported.
He also approved removing the four dormant export fields ([#959](https://github.com/yan-h/harmonigraph/issues/959)),
saying they can be added back if needed.
He subsequently approved Playhead retirement ([#973](https://github.com/yan-h/harmonigraph/issues/973)) and the extra single-pane/custom public export layouts ([#974](https://github.com/yan-h/harmonigraph/issues/974)).
Preserve Scrolling,
Whole video and the combined Lattice/Analyzer arrangement,
proportion and orientation controls he uses.
These implementations landed in the completed first batch.
He chose to keep the cold-start shader optimization,
and explicitly requires an accurate current lattice plus spectrogram history from while the editor was closed,
within the existing history limits.
The [follow-up value decisions](requirements-value-audit.md#follow-up-value-decisions) record the answers to this batch;
none of its four questions remains unresolved.
In the subsequent whole-feature inventory,
Yan reported occasional Spiral use and no personal use of the Notes pane,
console,
standalone application or adaptive-tuning laboratory.
He also reported no use of Playhead,
single-pane exports or custom export arrangements,
while explicitly retaining the controls to arrange and orient Lattice and Analyzer together.
The four unused panes/developer interfaces remain [usage leads](requirements-value-audit.md#whole-feature-usage-inventory),
not approved removals.
The later explicit approval settles Playhead and extra public export layouts only.
The [feature-retirement trace](feature-retirement-discovery.md) now identifies concrete removal boundaries,
including shared tests, musical reference data and diagnostic sinks that need to remain available.

**How to return:** read the [completion record](https://github.com/yan-h/harmonigraph/issues/954#issuecomment-5746459598) and [next-session scope](https://github.com/yan-h/harmonigraph/issues/954#issuecomment-5746476887).
Inspect current main,
issue/PR status and active ownership before continuing within the actual request's authority.
The first batch is complete;
its old launch does not authorize new merges or optional product removals.
The following chronology records the earlier discovery-only decisions on 2026-09-19;
its deferred language does not supersede the later launch.
Yan authorized the first pilot and its expanded sample on 2026-09-19.
He then authorized the settings investigation and keeping its documentation in the open draft PR #951,
accepted future spacing removal,
and asked for systematic discovery instead of implementation.
He then requested individual issues for the leads and continued investigation on the same date.
He subsequently authorized the requirements-value audit and broadened its coverage beyond a sample.
Those issues preserve evidence and deferred scope;
opening one does not begin its implementation.
Yan later requested the first-batch handoff and documentation merge,
then launched that now-completed implementation.

## Discovery record (2026-09-19, before launch)

The remaining sections preserve the discovery's preferences,
proposed sequence and evidence at their recorded checkpoints.
Their implementation deferrals are historical;
the current position and return instructions above govern continuation.

## Confirmed preferences and the concern behind this work

These preferences come from the planning conversation with Yan on 2026-09-19:

- The project is entirely LLM coded; Yan relies on agents' explanations to understand the code.
- Many bugfixes are invisible in ordinary use: correctness, tests, comments and edge cases.
- Long-term maintainability matters; upfront cost is not a major concern.
- LLM quota is available; Yan's attention is the scarce resource.
- Paying an independent engineer is out of scope. Investigation, implementation and technical review will be performed by agents.
- A central concern is that an agent's assumption becomes a comment, then a requirement, then an architecture that preserves something Yan never valued.
- Accepted simplifications should accumulate during discovery; accepting a loss does not start its implementation in this session.

The [README](../README.md) describes the product's purpose:
composing microtonal music and making videos.
The current compatibility policy and development obligations live in [CLAUDE.md](../CLAUDE.md).
Read their current versions when work resumes.
Product assumptions in repository prose are still claims to investigate;
their presence does not prove Yan requested every behavior they describe.
Existing operational instructions govern how sessions work,
and proposed changes to those instructions should be explicit.

The methods below are proposals from this discussion,
not additional user-approved requirements or amendments to the project contract.
Update them when evidence suggests a cheaper or more useful approach.

## How to spend agent effort and conserve human attention

Use a coordinator to collect evidence,
assign bounded investigations and synthesize recommendations.
Yan should receive a short decision brief rather than several raw agent reports.
Technical choices that fit the authorized task belong to the agents;
questions for Yan should concern desired behavior,
acceptable loss or a tradeoff the available evidence cannot resolve.

Investigators should distinguish observed facts,
inferences and proposed preferences.
A reviewer should try to disprove a consequential finding using code,
history or a reproducer before it becomes implementation work.
Different agents agreeing is not independent proof when they inherit the same premise.
Another model can supply another perspective,
but the evidence still needs to support the conclusion.

Use parallel read-only work where it is useful.
Follow the existing worktree and overlap rules for edits;
this plan does not establish a new agent framework or review gate.
If review does not resolve a disagreement,
record the missing evidence and a bounded next experiment rather than repeating reviews indefinitely.

Batch product questions into small sets when possible.
For each question,
give a recommendation and an observable consequence in ordinary language.
If Yan has not answered a required product decision,
continue unrelated investigation and leave the dependent change pending.
Silence does not confirm a preference.

## Proposed sequence

Each stage should reuse existing evidence and infrastructure before adding anything.
The pilot and Yan's discovery-only direction have changed the sequence below.

| Stage | Agent work | Useful result | State |
| --- | --- | --- | --- |
| 1. Pilot | Examine recent fixes and one area with recurring maintenance work; trace requirements and challenge the strongest findings. | Twelve-case assessment and three recommendations in [#950](https://github.com/yan-h/harmonigraph/issues/950). | Complete |
| 2. Worked discovery case | Trace persisted spacing and settings validation; inspect saved examples and render a keep/remove comparison. | Evidence in [#952](https://github.com/yan-h/harmonigraph/issues/952); Yan accepted the custom-input loss. | Complete; implementation deferred |
| 3. Broader discovery and prioritization | Survey the areas below, trace realistic changes, challenge the best leads and compare their value. | [#954](https://github.com/yan-h/harmonigraph/issues/954), runtime probes, [requirements-value audit](requirements-value-audit.md), [100-change history](historical-change-cost.md) and developer-consumer trace. | Checkpoint reached; smaller optional leads remain independently returnable |
| 4. Selected simplification | After Yan requests implementation, sequence the most valuable accepted changes by dependencies and file overlap. | Focused PRs showing which maintenance obligations disappeared and what behavior was verified. | Deferred |
| 5. Verification and handoff | Check the resulting workflows and update the explanations future agents need. | Evidence of preserved or explicitly changed behavior, remaining issues and a clear stopping point. | Follows implementation |

### Pilot: assess whether the improvement process is working

The first pilot expanded from six to twelve cases at Yan's suggestion,
with a cheaper agent handling the additional sample.
Its selection,
findings and limits are recorded in [#950](https://github.com/yan-h/harmonigraph/issues/950).
The initial method was to select merged fixes from a stated recent range,
using a mix of production behavior,
test or documentation work,
and structural changes where the range contains them.
Record how the sample was chosen;
it is exploratory evidence and does not establish a defect rate for the whole project.

For each fix,
ask whether the original behavior violated a justified requirement,
whether the evidence reached the claimed failure,
whether the repair addressed the cause,
and what permanent complexity it added or removed.
A test should not be treated as proof of desirable behavior merely because it agrees with the implementation.

Use the history to select one area where changes repeatedly require coordination or repair.
For its consequential constraints,
identify whether each comes from Yan,
an external technical requirement,
the present implementation,
or an unverified historical assumption.
Trace claims to evidence where feasible and preserve uncertainty where it is not.

The pilot ends with a short recommendation brief and linked evidence.
Do not turn every suspicion into a fix or an issue.
Confirmed bugs that will remain unfixed follow the existing issue rule in CLAUDE.md.

### Broader discovery: find obligations worth retiring

Survey several kinds of maintenance cost,
not just more hidden settings.
These are search lenses,
not findings that the areas are defective:

| Area | What to investigate | What would make an opportunity credible |
| --- | --- | --- |
| Product capabilities and constraints | Hidden settings, alternate modes, retained fallbacks, historical support promises. | A capability with uncertain value creates recurring work, and its actual consumers and loss can be stated. |
| Duplicated knowledge | Defaults, ranges, units, field lists and rules repeated across UI, persistence, rendering and tests. | A normal change repeatedly needs coordinated edits or has already left the copies inconsistent. |
| State ownership and data flow | Live state, snapshots, recorded events, replay, caches and invalidation. | A realistic change exposes unnecessary synchronization or competing authorities; required realtime separation is accounted for. |
| Verification | Fixtures, golden coverage, hand-maintained test matrices and diagnostics used to justify fixes. | The evidence misses its claimed path, or maintaining a test duplicates a rule that could have one owner. |
| Tooling and dependencies | Retained probes, vendor patches, generated assets, build variants and CI checks. | A former consumer disappeared or an obligation can be removed with its cost and lost diagnostic value understood. |
| Written constraints | Comments, design records, agent guidance and old issue decisions. | A consequential rule lacks its claimed authority or outlived the circumstances that justified it. |

Start with the existing audits and decision records,
including [#895](https://github.com/yan-h/harmonigraph/issues/895),
[#922](https://github.com/yan-h/harmonigraph/issues/922) and [#940](https://github.com/yan-h/harmonigraph/issues/940).
Their findings are tied to earlier revisions;
check what landed and what still holds before carrying an item forward.
Their dated dispatch mechanics are superseded by current [CLAUDE.md](../CLAUDE.md).
The #940 completion record and #895 stopping decision remain historical evidence;
neither old wave plan is a current work order.
Their presence does not authorize this discovery session to execute an implementation wave.

Use a broad,
inexpensive inventory first,
then spend deeper investigation on the most promising few leads.
Record which areas and paths were inspected and which remain unknown;
do not call the whole project audited because each area received a quick scan.
Use bounded read-only agents for the inventory and evidence gathering,
with coordinator review of the resulting shortlist.

For each candidate,
record the source revision and evidence,
the maintenance obligation that could disappear,
actual or possible users,
what would change,
the strongest keep argument,
and the next check that could disprove the proposal.
Search indirect consumers such as generic serializers and command generators before treating an absent assignment as an absent use.
Low observed usage is evidence about a sample,
not proof of no value.

Compare removing the capability,
keeping it,
and simplifying its implementation while retaining it.
Prioritize repeated repair costs and obligations shared across many owners over line counts or unfamiliar abstractions.
Compare expected maintenance reduction with lost capability,
confidence in the evidence,
verification cost and interference with other work;
do not disguise uncertain judgments as precise numerical scores.

Keep the detailed inventory in one linked discovery report,
including rejected hypotheses and their evidence.
Use distinct states such as lead,
verified proposal,
accepted but deferred,
keep and needs more evidence.
Create individual implementation issues when an item merits that treatment,
not for every suspicious field or comment.
Bring Yan a small batch of concrete product choices after comparison;
do not request a decision for each lead as it appears.

A discovery checkpoint is useful when the shortlist explains the best opportunities found,
their dependencies and what remains unexamined.
It does not require finding a removal in every area or eliminating every uncertainty.

### Reference cases: anchor verification in desired behavior

Inventory the tests,
recordings and evidence already available.
Prefer a few representative workflows over a new comprehensive test platform:
play and edit in Bitwig,
save and reopen,
record a take,
and export a video.
Include an ordinary case and a demanding case that reflects actual use.

Agents should prepare comparisons and extract available settings themselves.
Ask Yan only for missing preferences or subjective judgments that affect a decision,
using a compact visual or audible comparison where possible.
An automatically captured current output is a comparison baseline;
it becomes evidence of desired behavior only when its expectation is justified.
Record the source revision and relevant settings so comparisons can be reproduced.

### Architecture: investigate the cost of realistic changes

Yan chose historical analysis as the next investigation on 2026-09-19.
The [first historical batch](historical-change-cost.md) is complete,
with its exact population and limits recorded.
Start with actual merged changes rather than implementing hypothetical ones.
Inventory a stated range broadly,
then inspect representative changes and their evidenced follow-ups,
including straightforward successes.
Examples of useful histories are:

- Add a visual behavior that must appear in both the editor and exported video.
- Change a setting while keeping its displayed, saved and rendered meanings consistent.
- Change how recorded performance data reaches the renderer.

File and line counts locate cases;
they do not measure effort or architectural quality.
Separate generated assets,
file moves,
tests and intentional product iteration from duplicated rules and avoidable coordination.
Treat a later change as a repair only when its diff or recorded evidence supports that connection.
Check whether the burden survives at the inspected revision before recommending another fix.
Use a new experiment only for a specific consequential question history cannot answer.

Identify where a maintainer must know the same fact in several places,
where state lacks a clear owner,
and where a local change has surprising consequences elsewhere.
Possible investigation targets include audio/UI/render/recording boundaries,
cache invalidation and vendored dependency obligations.
These are hypotheses about where to look,
not findings that those areas need rewriting.

Prefer proposals that remove an obligation,
eliminate synchronized copies of information,
or establish clear ownership.
Explain what becomes simpler and what capability or guarantee would be lost.
A smaller file or a new abstraction is not sufficient evidence of improved maintainability.

## What Yan should see at a decision point

Keep each recommendation short enough to judge without reading code:

- **Problem and evidence:** what is wrong or costly, how it was established, and what remains uncertain.
- **Recommendation:** the proposed action, including leaving the current behavior alone when appropriate.
- **Effect on use:** what Yan would gain, lose or need to do differently.
- **Maintenance effect:** which mechanisms disappear and which new obligations appear.
- **Verification:** how agents would establish that the result works, and what judgment still needs Yan.

Link the reproducer,
measurements or code analysis underneath the recommendation.
Do not require Yan to read that material to discover the decision.
Use examples such as losing a particular saved setting or accepting a slower export,
rather than asking whether an architectural pattern is preferable.

## Where the work lives

This file is the entry point for purpose,
current focus,
consequential product decisions and links to active work.
Keep the opening section current so returning does not require reading the whole history.

The investigation accumulated in [PR #951](https://github.com/yan-h/harmonigraph/pull/951).
Yan subsequently requested its merge together with a complete coordinator handoff.
Keep later implementation changes in focused PRs,
with current issue status and the handoff governing the selected batch.

Use GitHub issues for concrete investigations or implementation work once justified.
Keep reproduction steps,
eliminated hypotheses,
acceptance criteria and detailed results with the relevant issue or PR.
Use the repository's existing evidence locations when an artifact belongs in the tree.
Link those sources here rather than duplicating their status or full contents.

Completed investigation: [#950 — twelve-change maintainability pilot](https://github.com/yan-h/harmonigraph/issues/950).
It recommends the bounded settings investigation above,
repairing the already-filed misleading shader-corpus check in [#947](https://github.com/yan-h/harmonigraph/issues/947),
and retaining the useful cache ownership and diagnostic CI changes.
No implementation work was performed by the pilot.

Completed bounded investigation: [#952 — settings paths and lattice-spacing decision](https://github.com/yan-h/harmonigraph/issues/952).
It corrects the pilot's writer inventory:
the documented appearance generator can set spacing through its generic field handling.
That establishes a supported capability,
not a user requirement to preserve the field.
Seventeen current-format take files and two usable saved project views all used spacing 1;
the report states the sampling limits and separately records obsolete takes and an unusable project sample.
Four rendered frames establish that zoom alone differs from spacing,
while zoom plus label size exactly reproduces the chosen custom-spacing example.
This does not establish equivalence for every legal configuration.

The same report maps editor,
recording and offline validation paths.
Normal saved-state and offline appearance loading already normalize settings;
direct typed callers can bypass that boundary.
Do not turn the correction to a historical rationale into removal of unrelated numeric guards or a new validation framework.
The accepted future change retires spacing's configurable invariant;
camera checks,
node budgets and renderer units retain their separate purposes.

Active discovery report: [#954 — ranked leads, retained mechanisms and coverage](https://github.com/yan-h/harmonigraph/issues/954).
The first detailed batch ran against product revision `1786c5ba`,
after incorporating #946 and #953's note-history changes.
Its findings are:

- Four dormant export fields do not alter either render-request constructor across 16 tested combinations; changing active resolution does. Yan subsequently approved their removal in #959, superseding the intentional keep decision in #895. Implementation remains deferred.
- Retired CLAP tracing has no production opt-in or payload-checking consumer found. Turning it off exposed a fixture mutex initialization dependency, recorded in [#955](https://github.com/yan-h/harmonigraph/issues/955). Explicit initialization lets all 21 boundary tests pass with tracing off and allocation assertions intact. Retiring the hook is a supported proposal, with this prerequisite.
- Remaining range duplication favors reusing existing sanitizers and a few shared bounds. Some rendering defenses intentionally differ from UI ranges and should stay separate. A nested hostile-input omission passes existing settings tests; a small extension catches it while retaining all five real range tests. [#957](https://github.com/yan-h/harmonigraph/issues/957) records the measured coverage gap.
- Ordinary and canonical Recorder → display versus actual file → Replay comparisons both pass. They detect independently injected timestamp and pitch-conversion errors. Preserve the separate event representations and queue ownership; a focused parity fixture is better supported than an architectural rewrite.

All applied scratch source,
manifest and lockfile changes were restored.
The [unapplied evidence patches](evidence/maintainability-discovery/README.md) preserve the work without installing new tests or product behavior.
These are bounded observations,
not proof of complete live/export parity,
cross-platform tracing behavior or absence of other settings gaps.
At that earlier checkpoint,
spacing was the only newly accepted product removal.
Yan subsequently accepted adaptive next-note-outline removal too;
implementation stays deferred.

### Follow-ups opened at Yan's request

The detailed report remains the evidence and prioritization index.
These individual issues make the leads independently returnable without converting them into an implementation wave:

| Lead | Issue | Scope |
| --- | --- | --- |
| Dormant export fields | [#959](https://github.com/yan-h/harmonigraph/issues/959) | Removal accepted after revisiting the earlier keep decision; implementation deferred. |
| Retired tracing | [#960](https://github.com/yan-h/harmonigraph/issues/960) | Supported proposal; coordinate with fixture issue #955. |
| Repeated ranges | [#961](https://github.com/yan-h/harmonigraph/issues/961) | Bounded sanitizer/range reuse; coordinate with #957. |
| Recorder/display/replay parity | [#962](https://github.com/yan-h/harmonigraph/issues/962) | Promote focused coverage when implementation is requested. |
| Hidden naming reach | [#963](https://github.com/yan-h/harmonigraph/issues/963) | Investigate actual naming loss before a decision. |
| Custom camera FOV | [#964](https://github.com/yan-h/harmonigraph/issues/964) | Investigated and removed: every saved value measured at the default, so `fov_y` is now a constant rather than a persisted field. |
| Current architecture guide | [#966](https://github.com/yan-h/harmonigraph/issues/966) | Correct verified Fold-cache and closed-window completion descriptions. |
| Old dispatch plans | [#967](https://github.com/yan-h/harmonigraph/issues/967) | Reconcile dated mechanics with current worktree ownership. |
| GUI dependency cohort | [#968](https://github.com/yan-h/harmonigraph/issues/968) | Investigate the newly eligible published version before deciding on a rebase. |

Continued discovery ran at `fcea7dd1`,
with product sources still at `1786c5ba`.
Two existing regression tests passed and demonstrate that the visual-runtime guide's present-tense claims are stale:
Fold reuse already crosses frames,
and a closed editor already polls take completion.
The old benchmark numbers remain historical evidence;
they should not be rewritten as new measurements.
#940's hand-made worktree dispatch text conflicts with the current ownership policy,
while its linked #855 bootstrap defect remains open and should not be rediscovered as a new issue.

The support inventory found no broad Windows/Linux/host promise to retire.
README and the deferred-work decisions already identify macOS and Bitwig as the tested personal environment.
Portable dependency code and a generally named Video feature do not establish a support commitment.
No extra platform CI,
installer or deletion campaign follows from this inventory.

The [official published egui-baseview archive](https://static.crates.io/crates/egui-baseview/egui-baseview-0.7.2.crate),
verified against its registry checksum,
now meets #409's egui 0.36+ investigation trigger.
Source comparison suggests the old resize conversion patch may be structurally replaced;
texture-delta handling needs a runtime check,
and several other renderer fixes still appear to need adaptation.
#968 records the exact version,
checksum,
source sites and limits.
This is a reason to reassess the rebase's value,
not a conclusion that upgrading is worthwhile or safe.

That bounded feasibility work remains a separate investigation.
The later historical checkpoint supports selecting the smaller accepted removals and existing settings/parity proposals without waiting for a dependency rebase.
Hidden knobs remain secondary.
Product choices should still arrive as a small batch with concrete losses;
implementation remains deferred.

**Subsequent #968 checkpoint (2026-09-19):** the [published-cohort feasibility report](gui-cohort-feasibility.md) completes that bounded investigation.
It recommends retaining the current stack after the patch inventory and native prototypes;
the publication trigger is satisfied,
not an outstanding reason to wait.
Most obligations still need porting,
and the texture probe's lack of a successful presentation remains an explicit limit.

Do not create a parallel backlog or a second copy of the architectural documentation.

For a consequential product decision,
record the decision,
who supplied or confirmed it,
its date and the circumstances that would justify revisiting it.
Label agent suggestions as proposals until they have the appropriate authority.
Technical findings remain tied to their evidence and revision;
they do not become permanent product policy through repetition.
Carry that distinction into acceptance criteria,
tests and comments:
a characterization of current behavior does not by itself make that behavior a product requirement.

## Completion and resumption

Finish each bounded batch with the agreed behavior verified,
changes handed over under the existing project workflow,
and remaining uncertainty recorded.
During discovery,
the deliverable is verified evidence and a clear decision state;
an accepted recommendation can remain unimplemented without leaving the discovery batch unfinished.
No requirement exists to refactor every subsystem or eliminate every edge case.
Useful progress means fewer hidden assumptions,
clearer consequences for future changes,
and less human supervision needed to verify them.

On pause,
update the current focus,
linked work and next step.
If code or requirements have changed since an investigation,
recheck its relevant premises before implementing it.

For continuation,
use the [current return instructions](#start-here).
The pilot,
discovery and first implementation batch are complete;
do not restart them from an older resumption prompt.
