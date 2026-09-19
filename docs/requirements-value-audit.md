# Requirements-value audit

Date: 2026-09-19.
Inspected revision: `2a5595fd770f02108206592f3dee2bab328a8053`;
product sources remain at `1786c5ba1a4e2994c20dc2672eb7d200fd605d9b`.
Part of the [maintainability discovery](maintainability-plan.md),
in open draft [PR #951](https://github.com/yan-h/harmonigraph/pull/951).
Discovery only; no implementation.
Yan directly confirmed during this audit that he does not use the next-note outlines,
then explicitly approved removing them while continuing to discuss other candidates.
He subsequently approved separate-WAV replacement removal,
chose to keep cold-start optimization,
and confirmed that reopening must show accurate current lattice state and spectrogram history from while closed.

## Findings for Yan

The strongest evidence of the feared mechanism is old present-tense prose that still requires a more complicated adaptive-tuning protocol after its deliberate removal.
That can tell a future agent to rebuild work Yan already chose to delete.
The audit also found live requirements worth questioning,
but did not establish that they were invented by an agent or unwanted.

- **Retiring unused next-note outlines is the strongest new candidate.** Yan answered the concrete comparison with “I don't use these outlines.”
That is direct current evidence of low value;
he subsequently approved whole-feature retirement rather than replacing the exact winner display with another guide.
The existing design explicitly asks for winners,
so this revisits a recorded choice using a current preference;
it does not prove an agent invented the original feature.
[#970](https://github.com/yan-h/harmonigraph/issues/970) preserves the scope and remaining checks.
- **Keep closed-window analysis.** Yan explicitly values accurate current lattice state and spectrogram history from while the editor was shut.
Starting fresh on reopening would lose wanted behavior.
Preserve these outcomes within the existing history limits;
this does not require drawing invisible frames or freeze the current worker implementation.
- **Do not revive an extreme-load tuning project.** The 100+ note target is not a mandate to build a scheduler for every maximum setting.
The existing performance issue was closed as not planned because no crackle had been heard.
Its explicit trigger is a crackling dense downbeat, with the radius setting checked first.
- **Keep simple automatic recording and rendering.** Adding opt-outs creates more branches while preserving the same machinery.
The previous simplification deliberately removed options.
Yan approved the small, separate removal of four dormant saved fields in [#959](https://github.com/yan-h/harmonigraph/issues/959).

Spacing, adaptive next-note outlines, separate-WAV replacement ([#972](https://github.com/yan-h/harmonigraph/issues/972)) and the four dormant export fields ([#959](https://github.com/yan-h/harmonigraph/issues/959)) are accepted for future removal.
The take's own recorded audio remains supported.
Cold-start optimization is explicitly retained.
Yan subsequently approved Playhead and extra single-pane/custom public export layouts in [#973](https://github.com/yan-h/harmonigraph/issues/973) and [#974](https://github.com/yan-h/harmonigraph/issues/974),
while retaining Scrolling,
Whole video and normal combined arrangement/orientation controls.
No removal of Notes,
Console,
standalone or the browser laboratory has been approved.

## Method and limits

Yan asked for coverage across the project rather than a small representative sample.
This pass inventories major requirement clusters in tuning,
recording/replay,
visualization/settings,
and dependencies/development support.
It follows expensive or doubtful claims into their current owners,
named tests and decision history.
The coverage tables distinguish a detailed trace from an inventory using earlier verified discovery.
This is not a line-by-line audit of every setting, shader or historical conversation.

For each consequential claim,
ask what user-visible outcome it serves,
where its authority comes from,
what maintenance obligations it creates,
and what would actually disappear if it were relaxed.
Check the strongest reason to keep it and the smallest alternative.
A shorter implementation, fewer settings and a weaker promise are different things;
none automatically improves maintainability.

Evidence labels in this report:

- **Direct:** a statement from Yan in the supplied conversation or the current instructions he supplied.
- **Attributed:** a repository document, issue or PR says Yan chose something, or records a host observation.
The original conversation was not recovered.
An issue authored by `yan-h` is not automatically direct evidence: agents publish through that identity.
- **Technical:** a property required by the selected boundary, such as keeping the audio callback nonblocking or preserving an event's original recording time.
The boundary itself may still be a product choice.
- **Implementation:** one way of delivering an outcome, including exact capacities, formulas and data structures.
- **Unknown:** no adequate origin found in the material inspected.

Missing provenance is uncertainty, not proof that a feature should be deleted.
Keep recommendations below are audit judgments, not newly confirmed user requirements.
Current operational instructions remain in force;
this report creates no new review gates, compatibility promises or scheduled audits.

## Tuning coverage

Source trace: `harmonigraph-core/src/policy.rs`, `policy/reach.rs` and policy tests;
plugin `tuning/hub.rs`, `tune.rs` and tuning tests;
UI `adaptive.rs`;
[design](adaptive-tuning-design.md),
[current behavior](adaptive-tuning-plugin.md) and [implementation history](adaptive-tuning.md).

| Requirement cluster | Evidence and obligation | Disposition |
| --- | --- | --- |
| One central, sequential cross-track policy owner | Attributed decisions in #625 and #786; current Hub sorts what arrived in each callback. | Keep. Per-track snapshots change musical context and require a replacement ownership model. |
| Common activation delay and delayed expression stream | Attributed choice plus recorded Bitwig scheduling/latency observations; Tune retains a local delay line. | Keep within the current architecture. Exact multiplier range is a choice, not a host law. |
| Complete global chronological context | Former requirement reversed by #786; later arrivals are assigned when received. | Already retired. Do not reconstruct watermark/frontier barriers from older prose. |
| Retain an attack until its correction arrives | Former requirement reversed by #786; current deadline attempts raw output if correction is absent. | Already retired. Host refusal can still defer output for a different reason. |
| Fault latch that prevents notes, exact actual-output reconciliation | Former requirements reversed by #786 and removed in #788/#813. | Already retired. Local host acceptance tracking still prevents duplicate onsets and incorrect held state. |
| Frozen correction with audible later player bends | Repeated repository attribution to Yan in the design; tuned-onset context is separate from live bend. | Keep. Automatically retuning sounding notes is expressly excluded. |
| Moving reference and register-aware context | Attributed motivating musical examples and explicit reversal of earlier register deferral. | Keep the musical behavior; exact reference/scoring formula remains revisable. |
| Held/released hierarchy and recent memory | #861 records two weighted alternatives, observed drawbacks and accepted musical loss. Fixed 24-entry memory is explicitly temporary. | Keep the smaller hard rule. Do not fossilize a storage capacity as musical truth. |
| Hard local harmonic eligibility | Attributed requirement; complete candidate construction refuses exhaustion rather than silently truncating. | Keep unless the musical vocabulary changes. Exact union construction is an implementation choice. |
| Exact reachable-next-input outlines | Recorded desired winners, with register range/computation initially open; analytic solver and separate worker now implement it. Yan directly says he does not use the outlines and approves removal. | Whole-feature retirement accepted in #970. Implementation deferred. |
| Large cohorts, all-context scoring, generous growth capacity | Recorded growth target; historical v2 timing in closed #790; current code still evaluates each collected onset to completion. | Keep parked. No current v3 timing or heard crackle was established by this audit. |
| Simulator parity and musical fixtures | Named parameter profiles; design explicitly permits outputs to change with settings/policy. | Keep agreement with the claimed model. Intentional musical changes may update fixtures. |
| One Hub, in-process CLAP session, global lifecycle cuts | Attributed simplifications in #786, with known audible consequences. | Keep. More hubs/backends and graceful settlement are additional capabilities, not robustness fixes. |

### Exact outlines: a real choice, not an automatic deletion

After the comparison below,
Yan said on 2026-09-19: “I don't use these outlines.” This supersedes the need to ask him whether the cheaper approximation is good enough.
Yan subsequently approved removing the feature.
Whole-feature retirement is accepted;
starting implementation remains separate from that product decision under this session's discovery scope.
The exact-versus-eligible analysis remains evidence of what the existing feature promises,
not a recommendation to build a replacement he does not need.

The [design's reachability section](adaptive-tuning-design.md#lattice-reachability-indicator) explicitly distinguishes eligible candidates from actual winners.
The current [UI contract](adaptive-tuning-plugin.md#controls-and-live-neighborhood) outlines the latter over C2–C7,
including off-screen winners in the count.
It intentionally excludes these annotations from preview/export pictures.

The obligations are concrete:
`policy/reach.rs` contains 189 lines of analytic interval computation,
including competing-score crossings and selector boundary checks;
UI `adaptive.rs` contains 95 lines for requests, results, generation cancellation and pending/error state.
The Hub publishes authoritative next-attack context.
Scoring changes must remain consistent with both ordinary selection and the winner solver.
These line counts describe ownership, not a measured deletion size or runtime problem.

Replacing the promise with a clearly labeled eligible neighborhood can reuse candidate preparation and leave musical output unchanged.
It gives up the guarantee that every outlined node can actually be played next.
Exact interval solving and its parity obligations disappear.
Current context publication and correct invalidation remain necessary;
candidate generation still has a cost,
so deleting the worker has not been established.
An optional exact mode retains most maintenance obligations and is not the same simplification.

Strongest keep argument: the requested feature answers what the player can choose next,
and candidate-only outlines can suggest unavailable choices.
The exact solver is modest in size and already off the audio thread.
The appropriate decision is whether that distinction matters in use,
not whether 189 lines sounds large.

A [standalone probe](evidence/maintainability-discovery/README.md#eligible-neighborhood-versus-exact-winners) compiled the current production core and compared both sets at two existing musical-fixture snapshots:

| Snapshot | Eligible nodes | Exact winners | Additional outlines under the cheaper promise |
| --- | --- | --- | --- |
| C–E-flat–G before B-flat | 36 | 23 | 13 |
| C–G–D–A fifth chain before E | 46 | 24 | 22 |

These are actual production preparation/solver results with fixture-provided context and default settings,
including production keyboard-selection logic.
They demonstrate that the semantic loss is material in these examples.
They do not count visible on-screen nodes,
execute a live Hub or establish all-context equivalence,
and they are not performance measurements.

Whole-feature removal needs a narrower cut than deleting everything named neighborhood:
the ordinary musical candidate preparation and Neighborhood steps control still determine tuning.
Lattice Map also uses `draw_assignment_outline`,
so that shared drawing helper and map outlines remain.
A particularly important coupling is `Sequencer::publish_neighbourhood`:
it adopts the current musical configuration before publishing,
and `Hub::begin` calls it before expiring silent memory.
Simply removing that call would remove the configuration update needed even without incoming notes.
Preserve equivalent adoption before expiry;
do not treat the feature's current function boundary as the boundary of its musical responsibility.
This is a prospective removal hazard, not a demonstrated existing bug.

The subsequent whole-feature trace found three dedicated production modules totaling 337 lines,
including comments and blanks:

| Retirement surface | What disappears | What stays |
| --- | --- | --- |
| Core `policy/reach.rs` (189 lines), its module export and reach-only `benefit_radius` helper | Exact winner interval solver and its extra scoring coupling. | Ordinary candidate preparation, selection, keyboard rules and musical policy. |
| UI `adaptive.rs` (95 lines), `VisualRuntime` field/default and editor handoff | Worker, request/result queues, cancellation, pending/error state and reach-only repaint requests. | Other runtime state, ordinary tuning status and picture rendering. |
| Plugin `tuning/neighbourhood.rs` (53 lines), setup mailbox/accessor and Hub publication cache | Atomic context-transfer channel and feature-specific cached publication. | Musical context/configuration, held voices, working scratch and reference ownership. |
| Tuning checkbox and adaptive lattice outline/count branches | The unused live annotation and its control. | Neighborhood steps musical setting, Lattice Map outlines/labels and map-edit highlighting. |

No persisted visibility field, plugin parameter or take/configuration schema for this annotation was found.
The runtime default also creates the worker for offline pictures,
although those pictures do not draw its annotations.
Removal would eliminate that construction without implying changed export pixels.
No timing or process-memory saving was measured.

Tests need selective treatment:
`an_unprofitable_keyboard_match_keeps_the_note_unsnapped` mixes selector and reach assertions;
the selector assertions remain.
`Hub::test_next_context` currently observes the feature's mailbox,
but its consumers verify musical resets, moving reference, Retune exclusion and repeats.
Keep those behaviors covered through an appropriate test observation point,
accounting for the old mailbox snapshot being published before silence expiry.
Deleting their observation helper is not a reason to discard musical tests.

The JavaScript tuning laboratory has a separate reachability/interval-inspection tool.
Yan's answer about these product outlines does not establish that separate tool is unused;
leave it outside the proposed production retirement unless its value is separately resolved.
The discovery probe here is historical evidence tied to the stated source revision,
not a future requirement to keep the Rust reach API alive after removal.

### Old requirements that must not return

[#786](https://github.com/yan-h/harmonigraph/issues/786) records eleven decisions attributed to Yan on 2026-09-09,
including raw output at a missed correction deadline,
display/take of Hub-scheduled output,
callback-local ordering and continued uncorrected notes under faults.
Current `Tune::schedule` and the test `a_record_arriving_after_its_sample_was_sequenced_is_still_assigned` agree with those changes.

However,
the opening adaptive section of [deferred-work.md](deferred-work.md) still says a missed assignment delays the attack,
storage exhaustion latches a terminal fault,
and central sequencing provides complete chronological context.
Those are present-tense descriptions of removed obligations.
They are a credible route for old complexity to grow back.
Correct the current-state summary while preserving dated decisions as history;
do not revive the deleted protocol to satisfy it.
[#969](https://github.com/yan-h/harmonigraph/issues/969) records this defect and the stale background-analysis rationale below.

[#790](https://github.com/yan-h/harmonigraph/issues/790) is another important counterexample.
Its old v2 optimized-core measurements were 0.106 ms for 15 default attacks,
6.205 ms for 100 default attacks and 63.311 ms for 100 attacks at three axes/radius five.
These are neither current v3 measurements nor whole-DAW timings.
The issue was closed as not planned because no crackle had been heard;
its comment requires a concrete dense-downbeat problem before reopening.
A broad growth target does not overrule that disposition.

## Recording and replay coverage

Detailed trace: recorder lifecycle, publication, writer/fanout and render-job code;
plugin recording-route configuration;
offline CLI and Replay;
[offline guide](offline-rendering.md),
history #523, #608, #712, #817, #903 and existing discovery #959/#962.

| Requirement cluster | Evidence and obligation | Disposition |
| --- | --- | --- |
| Explicit recording intent across host process modes | Attributed Bitwig reproduction: an offline-labelled pass had no played notes. Current arm/progress logic avoids that detector. | Keep explicit intent. The exact toggle is not the technical requirement. |
| Shared transport coordinates for notes/audio | Recorded host evidence and alignment necessity; take and replacement audio have explicit starts. | Keep unambiguous time mapping. A local clock still needs an equivalent mapping. |
| Original pass/time despite delayed publication | Immutable route is resolved by the audio owner; later arming must not refile old events. | Keep causality. Segment/frontier data structures can change if the same bounded ownership is preserved. |
| Fresh transport history even when a jump is rejected | Lifecycle distinguishes history-only discontinuities from accepted capture progress. | Keep the distinction. Jitter tolerances and continuity thresholds are revisable policies. |
| Seal the intended pass before automatic export | #523 records a restored playhead causing a tiny scrap pass to replace the real export. Current triggers distinguish rewind/disarm behavior. | Keep the demonstrated behavior; do not generalize it into “every rewind always ends everything.” |
| Observable gaps and incomplete records across rollover | Bounded queues cannot promise losslessness; replay must clear false held state and exports warn about known loss. | Keep visible loss and durable markers. Conservative attribution to the next take is an implementation tradeoff. |
| Selected audio source and no callback blocking | Main/Sidechain identity, original time, whole-frame drops and gap marking. | Keep stream identity and nonblocking capture. Format/precision is a separate quality choice. |
| Always capture adjacent audio and auto-render a finished take | Recorded arm/play workflow; prior simplification deleted capture/bounce toggles and related branches. | Keep. An opt-out adds policy and tests without retiring capture, finalization or rendering. |
| Manual replacement audio and alignment | Reachable CLI capability deliberately retained when #903 removed auto-correlation; Yan now explicitly approves separate-WAV replacement removal. | Remove replacement when implementation starts (#972). Preserve recorded-audio placement; manual alignment can also affect the take's own audio and needs a separate consumer trace. |
| Snapshot appearance, explicit rerender overrides | One shared appearance document; settings/camera gestures during a take are deliberately not all recorded. | Keep the represented-input contract. Do not promise recording of every live edit. |
| Deterministic replay/preview correspondence | Replay orders represented events/configs and uses the shared picture path. Existing #962 has bounded positive and negative-control evidence. | Keep semantic agreement. Full window, cadence, host chrome and universal pixel parity were not established. |
| Format refusal and normalization | Current instructions supplied by Yan allow breaking old saves with disclosure and forbid compatibility shims; container defaults and sanitizers keep accepted values coherent. | Direct current authority. No migration framework or general backward-compatibility project. |

The always-capture suggestion was challenged during this audit:
default-on plus opt-out would increase maintained behavior,
so the initial investigator's suggestion was rejected.
The initial audit did not recommend deleting the manual CLI merely because its original user transcript was unavailable.
Yan's subsequent explicit removal decision now supersedes that provisional keep judgment for separate-WAV replacement.
The existing [dormant-fields probe](https://github.com/yan-h/harmonigraph/issues/959) remains the concrete small opportunity.

## Visualization, settings and persistence coverage

Detailed trace: plugin `background.rs`, current analyzer zero-window branch,
shared appearance parsing/normalization,
history ownership and live-ingress documents.
Earlier verified discovery supplies settings/FOV/naming and live/file probes.
This does not assess the artistic value of every shader effect or every control combination.

| Requirement cluster | Evidence and obligation | Disposition |
| --- | --- | --- |
| Full live visual history while the editor is closed | #305 records the note/spectrum mismatch; Yan now directly requires accurate current lattice state and the spectrogram from while closed. | Keep these outcomes within current history limits. Do not expand this into unlimited history or invisible rendering. |
| Adopt saved analysis settings before first open | Same closed-window feature otherwise creates columns with the wrong analysis window. | Required if closed-window history is retained. Not a separate demand for another synchronization layer. |
| Preserve active unsaved UI edits | Saved host blob is not continuously authoritative over active appearance. Restore guard protects live edits. | Keep current behavior unless redesigning the save/restore authority intentionally. |
| Bounded queues, explicit analyzer discontinuities and source timestamps | Current ingress already permits drops and resets on gaps/epochs; notes and columns must share coordinates. | Keep honest timing/loss. No lossless-history guarantee to retire. |
| Ten-minute live history and lower-detail older spectra | Existing bounded/tiered storage; whole-song export has a separate path and known tradeoffs. | Keep current scope. No requirement for unlimited perfect history was found. |
| FFT resolution/tapers, channel power and silence progression | Current analysis shares live/offline kernels; exact-zero windows emit columns without FFT. Window length trades time against frequency resolution. | Keep represented signal semantics. Current numerical method/ranges remain implementation choices. |
| Lattice glow/history continuity through viewport changes | History owner separates persistent ink from viewport targets; capacity growth explicitly reseeds and may lose old release color. | Keep targeted behavior. This is not a universal “every resource transition preserves all pixels” rule. |
| Persisted hidden controls and repeated ranges | Earlier writer/normalization traces and saved examples in #952; further naming/FOV investigations #963/#964. | Spacing removal accepted. Other controls need their own visible-loss evidence; reuse existing validation rather than add a framework. |
| Shared appearance across live/save/take/export | One owner with snapshots at boundaries; nested hostile-input gap measured in #957. | Keep shared semantics. Repair specific coverage/duplication if selected; no generic schema engine follows. |
| GPU/CPU caches and history ownership | Earlier cache evidence distinguishes stale values from unnecessary invalidation; existing ownership reductions already landed. | Keep useful reuse. Test newly reachable carry-forward behavior when narrowing keys. |

### Closed-window history: distinguish feature cost from removable architecture

**Current decision:** keep accurate current lattice state and spectrogram history from while closed,
as Yan directly requested after reading this comparison.
The fresh-history counterfactual below was considered and is not selected.

[PR #305](https://github.com/yan-h/harmonigraph/pull/305) added background analysis so reopening would not show filled note history over a missing spectrogram.
Its report explains the mechanism and cost,
but does not establish a direct request from Yan to preserve all unseen history.
The current worker also adopts saved appearance,
advances current note state,
polls take completion and drains final owned payloads.
The test `a_take_ends_itself_while_the_editor_window_is_shut` covers an independent recording requirement.

If unseen visual history is unimportant,
a simpler product promise could restart that history on opening while continuing recording and necessary current-state work.
That permits avoiding some spectrum construction and before-first-open history obligations.
It does not permit deleting the whole worker or leaving stale notes/audio queued for reopen.
No removal prototype or current CPU measurement was run,
so this ranks below the exact-outline question.
Adding an on/off preference would retain both paths and weaken the maintenance argument.

The module comment also overstates today's cost:
it says a continuous FFT is paid even for silence and that the hop is the only way to reduce it.
Current `harmonigraph-analysis` skips transforms for exactly zero retained windows,
with `silent_windows_keep_readiness_retained_audio_and_resume` covering continuation semantics.
The old PR/module timings must not be presented as current full-plugin measurements.
This is stale rationale to correct, not evidence to delete silent-history progression.

## Dependencies and development support coverage

Inventory reuses the preceding [dependency/support discovery](maintainability-plan.md#follow-ups-opened-at-yans-request),
with direct reads of current CI, core README, golden gate, appearance parser and vendor surface-format boundary.
No new dependency build, platform certification, security audit or legal conclusion was performed.

| Requirement cluster | Evidence and obligation | Disposition |
| --- | --- | --- |
| macOS/Bitwig and in-process CLAP scope | README/deferred records identify the personal tested environment. | Keep narrow scope. Portable upstream code does not imply a Windows/Linux/other-host support promise. |
| GUI vendor patches and version coupling | #968 verified published egui-baseview 0.7.2 meets the old investigation trigger; some patch topologies changed and others remain. | Continue that bounded feasibility issue separately. No automatic upgrade or broad fork retirement. |
| Precompiled Metal startup | Dated native-probe evidence with the cache deliberately blocked compares about 4.77 s source readiness with 0.77 s strict assets; warm readiness was about 96 ms either way. Corpus follows generated MSL/options. | Yan explicitly chose to keep this optimization. The cold probe is not a prediction of every Bitwig opening. |
| Pure dependency-free core | CI guards accidental dependency changes but explicitly permits deliberate intended permissive additions with a guard/rationale update. Analysis already lives separately with RealFFT. | Do not interpret the guard as an immutable ban or a reason to reimplement libraries. No useful deletion established. |
| Exact golden pixels | Shared gate produces differences and permits deliberate blessing with an explanation. | Keep detection of unintended picture changes. Byte-exact comparison is not a promise to freeze every pixel forever. |
| Standalone harness and font fallback | Prior #895 traces actual standalone consumers and unavailable same-frame font-sharing insertion point. | Keep; production plugin font reuse alone does not retire the fallback. |
| Optional tracing, diagnostic fixtures, security suppressions | #955/#960 show retired trace payloads plus fixture initialization dependency; prior support inventory checked target-specific advisory premises mechanically. | Retire only the supported trace lead if selected. No broad diagnostics/CI deletion. |
| Owner worktrees, draft PRs, paired plugin/offline builds | Current instructions supplied by Yan explicitly require these; old #940 mechanics conflict and #967 tracks that. | Direct current authority. Audit current costs without silently overriding the workflow. |
| Assumed surface format and “upstreaming off the table” | Constant remains live; vendor renderer knows its actual format but exposes no plugin accessor. No current direct prohibition on proposing upstream work recovered. | Current accessor limitation is real; old policy wording is not a permanent ban. No failure or benefit justifies new work here now. |

## Follow-up value decisions

Yan asked whether other potentially removable requirements had been withheld from the question list.
The initial shortlist prioritized demonstrated savings,
but an early answer about actual use can cheaply rule a capability in or out before a removal prototype.
The table now records his answers to all four questions.
After the dormant-field explanation,
Yan explicitly approved removing them,
saying they can be added back if needed.

| Requirement to reconsider | Concrete loss to discuss | Maintenance evidence and recommendation |
| --- | --- | --- |
| Retain four dormant settings for a possible future export UI | Discard currently inactive saved audio/render options. Existing capture and automatic rendering continue; active separate-WAV replacement is a distinct removal. | Removal accepted in #959 after explanation; implementation deferred. Its 16-combination probe found no request changes. Savings are small. |
| Reconstruct live visual history while the editor is closed | Reopening starts a fresh visual history instead of showing what happened while closed; recording still completes normally. | Keep. Yan explicitly wants accurate current lattice state and closed-window spectrogram history. Whole-worker deletion was never established. |
| Rerender a take using a separate WAV | Lose the ability to replace captured audio with a clean bounce or another soundtrack. | Removal accepted in #972; implementation deferred. Retain the take's own WAV and recorded-audio alignment. |
| Fast readiness when the shader cache is cold | Potentially wait longer for the first fully drawn editor instead of maintaining precompiled Metal assets. | Keep by explicit user decision. No corpus/provider deletion follows from this audit. |

The dormant fields are `record_audio`, `auto_render`, `audio_path` and `audio_offset` in `RenderConfig`.
They persist the nominal choices to capture audio,
automatically render,
select replacement audio and offset it,
but current capture and render-request construction ignore them.
Recording captures audio when a take is armed,
and a completed take renders according to its active trigger,
regardless of the two saved booleans.
The comment explicitly retained all four so a future bounced-audio interface could return without re-deciding its shape.
This is a recorded future-design commitment,
not a present feature consumer.
Removing the fields would discard dormant saved values and their defaults/descriptions;
it would not remove active recording/rendering behavior or internal audio placement math.
Yan's decision to remove the actual replacement workflow strengthened the recommendation;
his subsequent explicit four-field approval settles this separate question too.
Future needs can define settings when they arise rather than retaining this inactive design in advance.

Custom camera perspective (#964) and note-naming search reach (#963) remain less ready for a product question:
show which pictures or names change before asking Yan to value an unfamiliar hidden setting.
Non-use of the standalone application also would not by itself establish that its agent/development consumers can be removed.

## Whole-feature usage inventory

Yan subsequently reported using Spiral occasionally,
and never personally using the Notes pane,
console,
standalone application or adaptive-tuning laboratory.
This is direct current usage evidence.
No removal of those four interfaces/tools has been approved.

| Feature | Current evidence | Next audit boundary |
| --- | --- | --- |
| Spiral | Occasionally used by Yan. | Retain; do not infer disuse of its render path. |
| Notes pane | Not used by Yan; consumer trace complete. | Small UI-retirement proposal; lose the exact per-voice debug table, retain shared tracking/naming and Analyzer warning. |
| Console | Not used by Yan; two important plugin failures lack an existing alternative diagnostic. | Recommend keeping the small current pane; no new logging system. |
| Standalone application | Not used by Yan; maintained developer capability, current launch frequency unknown. | Recommend keeping it at this checkpoint; no equivalent existing native interactive replacement established. |
| Adaptive-tuning laboratory | Browser not used by Yan; headless fixture pipeline actively maintained. | Browser-only retirement is a proposal; preserve model, tests and fixture generation. |
| Playhead export | Non-use followed by explicit removal approval. | Accepted, implementation deferred; preserve Scrolling and Whole video. |
| Single-pane and custom export layouts | Non-use followed by explicit removal approval. | Accepted, implementation deferred; retain normal combined controls and internal compositions needed by visual tests. |
| Combined Lattice/Analyzer arrangement and orientation | Explicitly used by Yan. | Keep normal preview/export layout, proportions, side and orientation controls. |

Video layout means the composition of the exported frame:
the ordinary Video preview arranges Lattice and Analyzer beside or above one another with a chosen split and aspect.
Offline presets additionally include each picture pane alone,
and a custom layout file can place or overlap panes.
Those extra interfaces are separate from retaining the normal preview controls.

The Video page's spectrogram modes change how time is presented:
Scrolling uses the configured recent-history duration;
Whole video still scrolls but expands that duration to the render length within existing limits;
Playhead lays out the recorded spectrogram in advance and moves a cursor through it.
These export choices do not change Yan's confirmed requirement to retain live spectrogram history while the editor is closed.

The [subsequent feature-retirement trace](feature-retirement-discovery.md) follows these usage leads into production paths,
tests and development consumers.
It recommends investigating Playhead removal,
distinguishes an unused browser interface from a live musical reference model,
and preserves diagnostic access despite personal non-use of the Console pane.
Those usage answers alone did not approve removal.
Yan's subsequent explicit approval settles Playhead and extra public export layouts;
the four panes/developer interfaces remain proposals.

## Next use of this audit

The [historical change-cost audit](historical-change-cost.md) and the developer-consumer continuation are now complete for this checkpoint.
They reinforce the existing settings and replay-parity proposals,
with no new general refactor or duplicate issue.
Notes and browser-only lab retirement remain optional choices;
keeping Console and standalone is the current audit recommendation.

Keep the choice list small:
Yan's direct non-use answer and removal approval make adaptive next-note outlines an accepted simplification;
separate-WAV replacement is accepted too,
while closed-window analysis and cold-start optimization are explicitly retained.
The dormant-field removal is accepted too.
Playhead and extra single-pane/custom public export-layout removal are also explicitly accepted,
with normal combined arrangement/orientation controls retained.
This batch of value questions is resolved;
carry forward the accepted removals and existing concrete issues when Yan requests implementation.
Do not reopen #790 or explicitly parked spectrogram campaigns merely because audit capacity is available.

The main remaining coverage gaps are original user conversations behind attributed choices,
current extreme-workload timing if its trigger occurs,
and aesthetic judgments about effects/settings that require comparison pictures.
The major subsystem inventory does not eliminate those gaps.

For future costly changes,
preserve the reasoning with the existing issue or PR:
the user-visible need, evidence source/date,
the smaller alternative rejected and the condition that would make the requirement worth revisiting.
Record unknowns as unknowns;
tests and repeated comments must not promote them to user decisions.
This is a lightweight documentation practice proposed by the audit,
not a new mandatory template or approval process.
