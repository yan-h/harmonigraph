# Companion aggregation for #617

This branch implements the production companion and session side of [#617](https://github.com/yan-h/harmonigraph/issues/617), on the canonical consumer foundation at `5ebe666b6eae6e29cd5e24d9468132047b2c217f`.
Its branch is `codex/617-companion-aggregation` and [draft PR #669](https://github.com/yan-h/harmonigraph/pull/669) stacks on `codex/617-canonical-recovery`.
The implementation remains in progress:
ordinary delayed shared-channel waves and setup replay, final measurements, independent committed-head review and the final release handoff are outstanding.
No adaptive correction or artificial delay is applied in this aggregation intermediate.
It is not the completed auto-tuning feature.

## Actual plugin and ownership path

The CLAP bundle exports full Harmonigraph and Harmonigraph Tune;
VST3 still exports full Harmonigraph only.
Tune has note input/output, one stable Participating parameter and a small native AppKit editor.
Its constructor creates no analyzer, GPU editor, audio recording ring or configuration mailbox.
Pairing, adopted clock status, explicit setup/reinitialization and the actual build label are available without making editor refresh responsible for musical progress.

Saved hub UUID and selected pairing UUID are separate from runtime session, source incarnation, clock epoch and offer generation.
Restored duplicate UUIDs remain visibly ambiguous.
The process registry counts four active/retired hubs and 64 active/unpaired/retired Tune owners;
each hub prepares all 16 source endpoint triples off audio.
The objects crossing attachment slots own the actual rtrb endpoints.
An audio owner reserves its return slot before taking an offer and retains the entire value on stale or full paths.
There is one attachment or detach attempt per enclosing source callback.
Off-thread collection only recycles an endpoint after both actual callback users have detached.
Retired unresolved owners retain their counted entries and credits.

The wrapper has one owned input pool shared by the ordinary walker, configuration and performance consumers.
It never relies on a retained host pointer or rescans host input for another consumer.
Output prepare reserves actual journal space, credit and the source emission gate before the host call.
Completion records the accepted prefix before releasing the permit.
The shared limits remain 512 normal and 128 emergency attempts per enclosing callback.

Unpaired attacks retain their original input and wait for actual session admission.
Already admitted Off sources forward and report actual output, with the same per-source 64 and per-session 256 reservation limits as participating sources and DIRECT forwarding.
An accepted termination keeps its credit until the hub acknowledges retention of the corresponding complete output stream.
Baseline acknowledgement only releases its current-state slot.
It cannot acknowledge an unsounded request, erase actual history or return a note credit.

## Original events, captured targets and release debt

Each original retained input owns one of 8,192 pending envelopes.
Wildcard targets, retrigger cleanup and channel-terminal targets use a separate 32,768-cell slab of 16-byte references, including an embedded free chain.
Capture preflights the complete original envelope, lifetime and reference group before changing input bindings or the finalized input cut.
One original input increments that cut once regardless of its target count.
Parent input serials protect a reused parent/child index from an old prepare or completion token.

Each target pins its input-time lifetime until its own completion or acknowledged disposition.
The original envelope is never overwritten with a resolved child event.
A mixed-generation wildcard can settle new-generation children locally while retaining the old child's disposition obligation and the complete parent/reference owner until the real old hub acknowledges it.
Cleanup and child settlement are bounded work, with explicit progress for joined retirement.

One accepted CC120 or CC123 produces one physical wire fact and explicitly linked logical terminal facts for the captured sounded lifetimes.
It does not fabricate several host output attempts.
CC120 retains owed physical Note-Off bindings;
CC123 clears the input-active bindings.
Neither controller silently resets sustain, sostenuto or hold.
Journal and output-sequence reservation count the terminal facts that this particular wire acceptance can still create.

Emergency voice/channel debt is separate from ordinary input and output storage.
Accepted neutral pedal resets are not resent merely because a later fault is stronger.
Unknown pedal values on an actually used channel still require accepted neutralization.
At the current selected voice/channel limits, one cycle can require 64 terminal outputs plus 48 distinct pedal resets:
112 actual facts within the 128-attempt reserve.
Ordinary output reserves enough sequence headroom for that whole emergency lane before making another host call.
Sequence exhaustion cannot wrap into a new identity namespace.

Stop/Reset cancels unsounded ordinary obligations through bounded per-child manifests;
held state alone is not their disposition proof.
A transport Stop is the actual playing-to-stopped edge in retained original input order.
It arms retryable old voice and pedal-release debt without permanently inhibiting new stopped-live input;
repeated stopped callbacks do not cancel the newer phrase.
Pairing-generation changes preserve old unsounded ownership until a real cancellation authority arrives.
Ordinary offer adoption preserves local fault inhibition;
an explicitly accepted Reset can clear it after the old lease obligations settle.
The all-retired service loop owns the actual stopped Source and Hub objects and drains their available history and dispositions without inventing a callback or time coverage.
Its conservative aggregate phase bound includes all 16 full pending/reference pools and is below its 2,048-round limit.
Unknown held output remains pinned until actual termination and the required acknowledgement.

## Clocks, canonical history and recording

Each instance has persisted nonautomatable signed sample offset, expected rate, maximum buffer and explicit validation status.
Apply/Reinitialize prepares off audio;
adopted calibration remains immutable for its clock boundary.
The controls show the audio owner's adopted values and validity separately from a newer accepted setup waiting for old output to settle.
An overlap between setup preparations refuses before mutating parameters, UUID, calibration or generation.
Zero offset is a configured value, not evidence of valid routing.
Graph or delay-compensation changes that the host does not expose still require manual revalidation.

Raw enclosing time plus sub-block/event offset maps with checked arithmetic into the adopted session clock.
The hub takes immutable membership for a collection pass and merges only continuous complete intervals.
Silence requires an actual processed callback;
an absent source is not silent.
The earliest pending progress cut is retained across collection windows.
Contiguous, monotonically mapped actual output can prove only the exclusive prefix before the latest received sample;
an equal-sample suffix remains blocked until complete.
Unmapped, backward or discontinuous input cannot create historical coverage.

Observed DIRECT input has its own rich lifetime/channel state and `ObservedDirect` provenance.
Actual DIRECT CLAP forwarding separately holds accepted-output credit and history.
Automatic publication repair snapshots the authoritative rich state for every affected source, including Off and DIRECT.
An unrelated successful baseline cannot consume another source's repair request.

Available actual output through a baseline cut is published once before that complete baseline and its later deltas.
Original input/actual onset, player expression, source incarnation, epoch and lifetime survive late collection.
The audio owner confirms musical state and acknowledges output independently of display and disk consumers.
A full publication queue produces a sequence gap and incomplete recording without blocking actual releases or credit retirement.
The same independence holds when both complete baseline payloads remain owned by a stopped drainer.
An incoming baseline that cannot be kept for reporting consumes a real loss serial at its original recording route, while musical state and baseline acknowledgement advance.
Later authoritative reconstruction repairs the display without removing the recording's historical gap.

Recording routes use the original runtime/epoch/pass and mapped session sample, with the adopted hub offset removed once for raw-span lookup.
A host reset freezes old route ownership until available old history and the actual terminal boundary settle.
Unplaceable post-reset terminal output uses an explicitly sealed stream acknowledgement with generation/final cut;
it does not fabricate a sample frontier.
The old take is marked incomplete after available old history is published.
Joined hub retirement retains the original publication owner and an independent recorder-writer lifetime until the delayed source closes.

## Executed verification so far

The follow-on [ordinary channel-wave slice](adaptive-tuning-channel-waves.md) adds known-state setup/history replay, indexed established-voice readiness and CC88 consumer reconciliation to this forwarding path.
Its functional and allocation evidence remains separate from outstanding full assignment, sequencing and timing integration.

The [physical allocation ledger](adaptive-tuning-aggregation-memory.md) reconciles the actual pools and measured factory increments with the constrained future 144 MiB session plan.
It preserves the absent assignment/history/policy reservations and identifies the remaining integration allowance.

These are production exported-factory fixtures using the real wrapper and host acceptance sink under the allocation/deallocation guard.
They are intermediate evidence, not final-head or Bitwig certification.
Tests are in `crates/harmonigraph-plugin/src/performance/tests.rs` and its focused child modules.

| Behavior | Actual reach |
| --- | --- |
| Admission | 16 simultaneous offers, refused 17th tuner, 64 notes/source, 256 session reservations, counted unresolved 64/65 registry boundary |
| Event/reference storage | 8,192 original pending events independent of captured fanout; all 32,768 references; next whole capture refuses without advancing its input cut |
| Joined retirement | All 16 owners have full original-event/reference/intent storage; 256 real terminal reservations remain unacknowledged before source-then-hub destruction; no rescue callback is supplied |
| Output pressure | Last of 512 ordinary attempts rejects; 64 voice terminations and 48 distinct pedal resets remain available; credit waits for actual acknowledgement |
| Sequence boundary | Consistently seeded Source/Hub prefix reaches the actual final `u64::MAX` terminal; duplicate transfer publishes once; mapped complete file and unmapped sealed incomplete file both settle credit |
| Input identity | Partial wildcard acceptance, same-key retriggers, mixed-generation child disposition and defensive stale completion after real index reuse |
| Attachment ownership | Actual Adopted plus whole Returned endpoint fill both return slots without registry service; real callback paused after offer take races a pairing change and returns the whole stale generation |
| Clock coverage | Nonzero calibrated offsets and transport sub-blocks; delayed future reports spanning retained output windows; missing silent callback and genuinely absent held source |
| Recovery | 64 channel-terminal targets, owed CC120 physical Offs, rejected/reset release debt, full manifest windows, stale seal/epoch acknowledgements and old-route closure |
| Publication | Real display overflow and mid-publication repair race across two Tune sources, including Off, plus DIRECT; complete baselines restore current state without synthetic attacks |
| Primary recording pressure | Two Tune sources, including Off, plus DIRECT hold 192 credits and overfill the actual primary queue; all 192 real releases and credits settle before any file/display drain; the file's incomplete range matches the primary loss descriptor |
| Full held rejoin | 64 Off-held voices rejoin with a complete baseline between retained tuning and later releases; an actual complete take contains exactly 192 original on/tuning/off records with unchanged lifetimes and samples |
| Transport Stop | An old held voice plus a new pre-Stop attack, expression and controller retain offsets 4/8/10 before an offset-16 Stop; terminations and three pedal neutralizations use offset 16 or a legal later retry; the new stopped phrase retains its timing and canonical identity |
| Stop cancellation pressure | 641 MIDI clocks fill the 512 normal attempts before a pedal and Stop; the actual 64-entry disposition window fills while additional old work remains, all old non-release events remain ineligible across acknowledgements, and exactly one newer stopped-live controller survives |
| Unpaired Reset | A malformed-input fault and 1,024 queued pedal messages keep Reset unapplied across its actual local cut; a genuinely later controller survives that pending cut and appears exactly once after recovery, with no old pedal replay |
| Pairing and inhibition | 512 actual expressions exhaust ordinary output before a retained onset; duplicate UUIDs preserve its old obligation; malformed unpaired input stays inhibited through ordinary adoption and recovers after an explicit settled Reset |
| Occupied baseline payloads | A third snapshot encounters two undrained payloads; actual release credit and journal retire before any consumer drain, an actual take records the exact one-serial loss, and later reconstruction is empty and participating |
| Open settings editors | The real egui Session menu stays open across prepared restore, drag and Apply; the exported native CLAP/AppKit editor refreshes existing-candidate selection and calibration across restore, applies calibration only, and submits Reset independently of invalid or unapplied drafts |

The counter-prefix fixtures seed internally consistent already-retained prefixes to reach exhaustion in finite time;
they do not claim to execute `2^64` historical outputs.
The stale-completion fixture is an explicit defensive duplicate injection;
CLAP host acceptance and completion remain synchronous.
The no-registry-drainer attachment fixture runs in a separate process because any other instance's main-thread setup legitimately services the process-wide registry.
Exported-factory fixtures share a whole-fixture guard through recorder injection, construction and teardown so independent tests cannot consume the four-Hub production limit.
Concurrency deliberately created inside a fixture remains active.
The two configuration-capacity fixtures invoke actual host Reset after their assertions, with accepted commands drained through bounded callbacks;
scratch occupancy measurements confirmed every ordinary fixture returns registry counts to zero.
Fixtures that intentionally retain unresolved owners remain isolated in child processes.
[Issue #672](https://github.com/yan-h/harmonigraph/issues/672) now has a separate destruction repair:
the joined wrapper explicitly disposes unfinished configuration input, commands and learning work while preserving applied configuration and original recording routes.
It does not advance successful configuration/source prefixes or fabricate a seed.
A Recorder-owned retirement hold prevents a failed take from closing while actual accepted Source output is still in transit.
Joined sources publish an incarnation/epoch/final-sequence fact through the existing fixed control slots;
that fact permits publication closure through actual output, independently of musical release and credit settlement.
The worker observes hold release before checking its queues and independent loss snapshot again.
Refused Hubs drain their joined observed DIRECT history and release this ownership synchronously because no registry entry can retain them.
The original two configuration-capacity states now destroy without Reset or another audio callback and return registry capacity.
Actual recording fixtures preserve On/Off at 67/87, retain all 16 applied 690..705 configuration values at sample 128 and omit unapplied 706. Additional joined On-only, owed CC120 physical-Off and pedal-only cases close Incomplete files while retaining unresolved musical owners.
The pedal-only fixture also exposed a prerequisite defect:
a source could previously seal a held pedal when no voices remained, so seal and settlement now require factual pedal neutralization.
A real reused source slot has the same old/new accepted cut 2 but a new incarnation and no inherited joined proof.
An actual refused fifth Hub closes its incomplete recording while the four registered Hubs remain alive.
Its strengthened fixture enables Learning, exhausts eight real command insert/apply pairs and observes a C/E/G triad at sample 67;
the required learning commit cannot fit, leaving actual DIRECT sequence 3 and pending sample 67 at configuration prefix 67 before destruction.
All three notes then reach the incomplete file through retired DIRECT publication.
Temporarily omitting that drain makes the same fixture record no notes;
the negative control is not shipped.
Moving 17 commands before the notes was separately measured to prevent observation entirely and was rejected as fixture reach.
A file-backed recorder fixture holds a failed take open through empty lanes, then fills all 4,096 publication slots and retains one further explicit loss.
Releasing retirement ownership while the ordinary lane is still full permits closure only after all 4,096 deltas and the exact 4097 loss reach the file.
The full ordinary/emergency retirement fixture reaches 4,096 ordinary facts plus 35 actual emergency outputs, including 32 owed physical Offs, at accepted cut 4,163. With configuration frozen at sample 128, requiring the complete report cut to transfer first deadlocks against the two 2,048-record receiver windows.
Destroying the Hub before the final Source callback also reproduces the defect without a pending baseline;
that callback has already reclaimed 512 factual receipts, leaving 3,584 ordinary and 35 emergency records at the Source while the receiver retains the earlier portion.
Both orders retain 32 credits, the registry owners and the file writer before correction.
Completed reports now advertise their original cumulative Coverage and full accepted cut before whole-cut transfer, giving the pending baseline's original pair priority over any later report.
The unchanged receiver grants full coverage only after receiving the cut;
otherwise it grants only the exclusive last mapped timestamp prefix under continuous, ordered, same-start coverage.
Contiguous callbacks cannot overlap a sample, and one callback creates at most 512 normal plus 128 emergency wire facts and 64 derived channel terminals before credits can be recycled.
The resulting 704-fact same-sample group fits the 2,048-record row, allowing that exclusive prefix to advance without splitting a timestamp group.
No final joined-producer, publication-hold or musical-credit condition is weakened.
Both actual destruction orders now release all owners and credits and close an Incomplete file containing exactly the 32 original Ons at sample 64 and 32 logical CC120 terminals at sample 128;
later source-only history has no original Hub recording route and is not assigned one.
The Stop/Reset correction passed 121 guarded plugin tests with two test workers and the separately coordinated callback timing fixture excluded.
The optional probe build follows its existing export selection:
production Tune/native-editor code and helpers used only by production Tune fixtures are excluded when `tuning-probe` supplies the companion class.
Stop markers share the existing 8,192 pending cells and use constant-time unlinking during the existing retirement cleanup phase;
they add no retirement handshake or pool.
Joined-producer publication adds 16 conservatively serialized control-consumption rounds to the former 1,422-round bound.
The retained baseline's saved progress report adds another 16, giving 1,454 below the existing 2,048-round ceiling.
A joined Source owns at most one baseline and creates no new one during retirement.
Retries wait for already-accounted report/output drainage and publish/consume each advance the service revision;
there is no new acknowledgement lane.
Advertising these same baseline/final reports before complete transfer changes their eligibility, not the number of retirement phases or their storage.
The full 16-owner event/reference fixture still reaches 8,192 inputs and 32,768 references per owner plus 256 actual terminal reservations.
Temporary diagnostics measured 659 final service rounds, 168 joined publication attempts, 16 successes and 16 consumptions;
those observed counts are evidence of retry reach, not the worst-case proof.
The diagnostics are not shipped.
The actual four-session factory retirement also returns registry counts to zero.
Native interaction checks used an isolated local CLAP host and actual AppKit controls, with saved state and host-callback receipts confirming the action;
they did not load or modify Bitwig.
The Stop sample-boundary, unpaired Reset-cut and optional-probe export corrections cleared both scoped independent reviews and CI at 3c529a1a.
The first issue #672 review found two remaining final-disposition stalls:
more than the 2,048-record receiver window beyond a frozen configuration prefix, and a retained baseline later than that prefix with no queued output to extend drainage.
Retirement now derives its candidate drain extent from actual retained output and baseline timestamps before requiring complete final cuts;
existing member coverage and FIFO/baseline ordering still clamp publication, and only strict final-cut disposition releases the recording hold.
The combined fixture additionally retains baseline cut 3,601 at sample 703 followed by output through cut 4,002;
its old Progress reports cannot authorize the baseline acknowledgement.
Source now retains the baseline's genuine original Coverage and can publish that bounded report once the baseline cut transfers, independently of later blocked output.
It never pairs a historical cut with a newer callback's coverage.
All three actual destruction fixtures fail before correction and pass afterward without rescue callbacks, preserving original routes and explicit failure for source-only spans with no Hub route.
A live absent Source still retains publication ownership because its actual history may be outside the receiver lane.
The take Writer flushes each appendable record, so its already-written prefix remains readable while final Incomplete marking and worker exit wait for factual closure;
queue emptiness or a timeout does not grant that authority.
The coherent follow-up awaits committed-head re-review;
it does not complete the remaining integration below.

## Remaining integration boundary

Shared-channel delayed waves and chronological setup replay are still required in this aggregation task.
The current all-target channel-header wait is not a complete solution for an established voice sharing a channel with an unsounded younger wave.
The accepted contract requires responsive older controllers/releases, preserved younger gestures and an accepted drain/neutral boundary before another translation starts.
Opaque downstream controller state also presents a concrete contract conflict:
an unobserved earlier CC7 value cannot be recovered after a required old-wave overwrite.
A user decision about that unreconstructable case is pending;
this branch does not silently assume defaults or add a new cancellation exception.

#616 still owns assignment-dependent sequencing, D, accepted plan/revocation state, prospective suffix recovery and the finalized adaptive-input frontier.
That frontier must retire the 128-marker effective-configuration timeline;
GUI or recording-publication progress is not a substitute.
The real policy and cohort components developed on separate drafts are not wired into this branch.
Final integration must verify their declared memory/scratch limits and the simultaneous cross-track musical cases.
Manual Bitwig routing/calibration, D512 timing, live/offline destination behavior and musical audition remain unperformed.

The final handoff must supply the committed head, open draft PR, independent review results, exact-head CI, final allocation/timing evidence and fresh release builds of both `harmonigraph-plugin` and `harmonigraph-offline`.
Only then can it name the actual `load-plugin.sh --tag` result and load command.
Nothing in this intermediate document claims that a build is already installed in the shared Bitwig slot.
