# Long-term maintainability

Last updated: 2026-09-19.
Status: planning; investigation and implementation have not started.

## Start here

**Aim:** make Harmonigraph easier for future agents to change correctly,
while reducing the attention Yan must spend understanding and supervising those changes.

**Current focus:** establish a bounded first investigation before scheduling a broad cleanup.
The next proposed step is a read-only pilot that examines a small sample of past fixes and one area with recurring maintenance work.
It should produce at most three actionable recommendations,
including the option to leave the code alone.

**Decisions needed from Yan now:** none.
Agents should investigate enough to make any eventual product tradeoff concrete before asking.

**How to return:** ask an agent to continue from `docs/maintainability-plan.md`.
The agent should read the current document and linked work,
then summarize the current position and take the next unblocked step within the requested scope.
Creating this plan does not launch the investigation or authorize all the proposed changes.

## Confirmed preferences and the concern behind this work

These preferences come from the planning conversation with Yan on 2026-09-19:

- The project is entirely LLM coded; Yan relies on agents' explanations to understand the code.
- Many bugfixes are invisible in ordinary use: correctness, tests, comments and edge cases.
- Long-term maintainability matters; upfront cost is not a major concern.
- LLM quota is available; Yan's attention is the scarce resource.
- Paying an independent engineer is out of scope. Investigation, implementation and technical review will be performed by agents.
- A central concern is that an agent's assumption becomes a comment, then a requirement, then an architecture that preserves something Yan never valued.

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
The pilot can change this sequence.

| Stage | Agent work | Useful result | State |
| --- | --- | --- | --- |
| 1. Pilot | Examine six recent fixes and one area with recurring maintenance work; trace requirements and challenge the strongest findings. | At most three recommendations, with evidence, cost and user-visible tradeoffs. | Next proposed step |
| 2. Intent and reference cases | Resolve consequential requirement questions; identify representative existing projects, takes and outputs. | A small set of desired behaviors and reusable reference workflows. | Proposed |
| 3. Architecture through changes | Trace a few plausible future changes through state ownership and data flow. | Specific sources of duplicated knowledge, coordination or hidden assumptions. | Proposed |
| 4. Simplification | Implement the most valuable justified removals or structural repairs in a coherent order. | Reviewable PRs showing what complexity disappeared and what behavior was verified. | Proposed |
| 5. Verification and handoff | Check the resulting workflows and update the explanations future agents need. | Evidence of preserved or explicitly changed behavior, remaining issues and a clear stopping point. | Proposed |

### Pilot: assess whether the improvement process is working

Select six merged fixes from a stated recent range,
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

Use GitHub issues for concrete investigations or implementation work once justified.
Keep reproduction steps,
eliminated hypotheses,
acceptance criteria and detailed results with the relevant issue or PR.
Use the repository's existing evidence locations when an artifact belongs in the tree.
Link those sources here rather than duplicating their status or full contents.

No investigation issues or implementation PRs have been created for this plan yet.
Add links when that work starts.
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
> Use agents to gather and challenge evidence, and bring me only concrete product decisions with recommendations.
> Do not treat unconfirmed suggestions in the plan as requirements.
