# Long-term maintainability

Last updated: 2026-09-19.
Status: discovery in progress; spacing removal accepted for future implementation; no product changes in this session.

## Start here

**Aim:** make Harmonigraph easier for future agents to change correctly,
while reducing the attention Yan must spend understanding and supervising those changes.

**Current focus:** systematically discover and compare maintainability opportunities across the project before selecting implementation work.
Spacing was a worked example of the method,
not the start of a sequence of immediate fixes.
The [first broader inventory](https://github.com/yan-h/harmonigraph/issues/954) records ranked leads,
potential behavior losses and the strongest reasons to keep each candidate.
Next,
challenge the retained-apparatus and settings-inventory leads with bounded checks,
and finish the fixture inventory for live-versus-recorded parity.

**Session scope:** discovery only,
as Yan requested on 2026-09-19.
Read-only code/history inspection,
scratch experiments and updates to this documentation are in scope.
Do not change product behavior or start implementation merely because a candidate is accepted.
Keep [draft PR #951](https://github.com/yan-h/harmonigraph/pull/951) open for the discovery record.

**Accepted decision:** Yan is comfortable removing custom lattice spacing,
including its custom-appearance capability loss,
based on [#952](https://github.com/yan-h/harmonigraph/issues/952).
Implementation is deferred.
No product decision is needed now;
gather and compare further candidates before bringing back a small batch of consequential choices.

**How to return:** ask an agent to continue from `docs/maintainability-plan.md`.
The agent should read the current document and linked work,
then summarize the current position and take the next unblocked step within the requested scope.
Yan authorized the first pilot and its expanded sample on 2026-09-19.
He then authorized the settings investigation and keeping its documentation in the open draft PR #951,
accepted future spacing removal,
and asked for systematic discovery instead of implementation.
Continue that discovery scope unless Yan changes it.

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
| 3. Broader discovery and prioritization | Survey the areas below, trace realistic changes, challenge the best leads and compare their value. | Initial inventory in [#954](https://github.com/yan-h/harmonigraph/issues/954); deepen the best leads into verified proposals. | Current phase; first breadth pass complete |
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

Choose a few plausible changes rather than auditing every abstraction against general style preferences.
Examples from this discussion are:

- Add a visual behavior that must appear in both the editor and exported video.
- Change a setting while keeping its displayed, saved and rendered meanings consistent.
- Change how recorded performance data reaches the renderer.

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

Yan chose on 2026-09-19 to keep [draft PR #951](https://github.com/yan-h/harmonigraph/pull/951) open for this investigation phase.
Update that PR as the findings develop;
do not open a documentation PR for each step.
Merge the phase at a meaningful checkpoint when Yan asks,
and keep implementation changes in their own focused PRs.

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
The first breadth pass groups opportunities around retained diagnostic/export machinery,
duplicated setting ranges and test inventories,
and live-versus-recorded event semantics.
Hidden naming reach and camera FOV are secondary leads.
The report records prior decisions and reasons to keep shared appearance ownership,
independent realtime queues,
core/wire type separation and useful tooling.
These source-inspected leads have not received spacing's level of runtime verification and are not accepted implementation tasks.

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

Suggested resumption request:

> Continue the maintainability work from `docs/maintainability-plan.md`.
> Check the current status and linked evidence first.
> If the pilot has not run, start with that read-only investigation.
> Keep this session discovery-only; record accepted changes for later implementation.
> Use agents to gather and challenge evidence, and bring me only concrete product decisions with recommendations.
> Do not treat unconfirmed suggestions in the plan as requirements.
