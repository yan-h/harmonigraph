# Project-wide adaptive tuning

## Status

This is the decided design for a planned feature;
none of it is implemented yet.
GitHub issue [#614](https://github.com/yan-h/harmonigraph/issues/614) is the design anchor, with separate children for the Bitwig timing spike, automatic aggregation, pitch output and the first policy.

This document fixes the product and real-time contracts, including the inputs the first musical policy needs.
The policy's scoring constants and musical iteration belong to #621;
[#630](https://github.com/yan-h/harmonigraph/pull/630) established a constrained working host configuration, not the minimum reliable delay.
Yan has accepted the latency target, late-track behavior, stop/Off rules, emergency stop at capacity and restricted routing recovery below.
The implementation owns the remaining event-ordering, storage and recovery mechanics in #617/#616;
those mechanics must satisfy these product decisions before adaptive output is complete.
An ordinary late assignment retains its attack and reports the failure;
explicit cancellation and the visible emergency stop are the documented exceptions.

## Design priorities

This feature is optimized first for one person's maintained macOS/Bitwig workflow, not for format or host coverage.
Correct musical state, an inspectable failure mode and one small execution path outrank compatibility that has no current use.
Add another format, transport, mode or control only when it serves a concrete project and is cheap enough to carry in every later change.

Host behavior is evidence rather than architecture.
Where the design depends on callback order, voice identity or CLAP event delivery, issue [#615](https://github.com/yan-h/harmonigraph/issues/615) measures the premise and either records the supported constraint or stops the downstream work;
where it depends on Bitwig process grouping and what a build swap then costs, issue [#623](https://github.com/yan-h/harmonigraph/issues/623) does.

## Decision record

This table preserves the strongest alternative and the condition that can reopen each choice.
A later implementation should not broaden a row merely because its rejected branch is technically possible.

| Decision | Chosen design | Strongest alternative | Why this won | Reopen only when |
|---|---|---|---|---|
| Attack timing | Fixed-delay central sequencing with sequential assignment, frozen through release | Immediate assignment from a prior snapshot | Simultaneous cross-track attacks must share the preceding assignments; one policy owner avoids distributed assignment and contention | #615 rejects the supported timing topology or measured delay |
| Plugin boundary | Separate lightweight Harmonigraph Tune class exported from the same CLAP bundle as Harmonigraph | Full-plugin instances or one class with persisted Hub/Tuner roles | Keeps the pre-instrument device and its lifecycle small and gives the host a clear note-effect identity; a process-wide registry is shared only inside one dylib, so one bundle is what makes an in-process session possible at all | #615 shows separate classes cannot share a reliable supported process/session topology |
| Input completeness | The hub orders complete source intervals using timestamped intents and source progress | Assign in callback arrival order | A later source callback may contain an earlier musical event; only complete intervals establish chronological order | #615 cannot establish usable boundaries and progress in the supported graph |
| Assignment order | Chronological, then deterministic sequential assignment of simultaneous attacks | Joint chord optimization | Each new assignment sees earlier ones without selecting a globally optimal chord or moving held voices | A concrete musical requirement demands a different policy |
| Session storage | Bounded preallocated queues and owned state with Rust-safe publication | Shared reference-counted immutable snapshots on the decision path | One central state owner removes tuner snapshot copies and overlays; allocation and final reclamation stay off the audio thread | Measured capacities require a different bounded primitive |
| Report clock | Reports carry their mapped musical sample, epoch, source incarnation and sequence; sources publish processed-through watermarks | Stamp every queued report at the hub's current block plus its local offset | Framework sub-blocks and delayed drains make callback-relative offsets insufficient; #615 establishes the mapping before implementation | Host evidence supports a simpler representation with the same time and reset guarantees |
| Storage layout | In-process preallocated storage with explicit ownership and off-thread reclamation | Pointer-free memory-mappable arena | `rtrb` already uses pointers and shared ownership; real-time safety requires bounded access and lifetime management, not a cross-process ABI | A concrete cross-process transport is authorized |
| Voice identity | Source, channel and key, with the host note id passed through untouched | Host voice id as identity with a multiplicity fallback for its absence | The tuner advertises no overlapping-note support, so the host must not overlap one key and channel; a retrigger replaces, which is the rule the tracker already applies | #615 observes Bitwig delivering overlapping same-key notes to a note effect that does not advertise them |
| State authority | Actual emitted output, with separate pending assignments for sequential scheduling | Treat every planned attack as already emitted | Later decisions need scheduled predecessors, while display, take and recovery must distinguish a plan from actual output | A real downstream pitch-feedback mechanism exists and is worth integrating |
| Pitch output | CLAP per-note tuning expression only | MTS-ESP, MPE or VST3 note output | Matches sample-timed per-voice frozen assignments and the actual personal host while adding no external tuning service; Bitwig converts a note effect's per-note pitch to MPE or VST3 note expression for the instrument downstream, so the instrument's format is not restricted | A required instrument cannot consume it or #615 disproves reliable delivery |
| Session transport | In-process registry under a documented Bitwig hosting mode | Cross-process shared memory | Avoids process discovery, crash recovery and stale shared state for compatibility that is not currently needed | #615 shows no usable in-process topology or a concrete workflow requires another hosting mode |
| Hub ownership | Full Harmonigraph, normally on Master | Headless conductor or elected tuner peer | Reuses the existing configuration, display, take and combined-audio location without another authority or plugin role | The hub cannot remain active in the supported graph or a project demonstrably needs tuning without a full Harmonigraph |
| Missed assignment deadline | Keep the note pending, accept its valid late assignment and report a failure | Drop it or emit it unretuned at the deadline | Yan prefers extra latency to either missing or incorrectly tuned notes | Lateness alone never changes this rule; actual capacity exhaustion uses the separate visible emergency stop |
| Participation UI | One Participating/Off control | Independent visibility, context and retune switches | Minimizes persisted states, combinations and tests before anchors or monitor-only tracks have a concrete musical contract | A real project requires a specific excluded combination |
| Policy location | One pure sequential policy in the hub | Distributed tuner evaluation of shared state | One owner has the complete ordered input and all preceding assignments; tuners buffer and emit answers | The measured hub work budget cannot support the chosen policy |

## User workflow

One full Harmonigraph is the project's state hub, combined note display and take recorder.
A lightweight **Harmonigraph Tune** companion plugin sits before each independently routed instrument path that participates:

```text
Track A: notes -> Harmonigraph Tune -> instrument A -> Master
Track B: notes -> Harmonigraph Tune -> instrument B -> Master
Track C: notes -> Harmonigraph Tune -> instrument C -> Master
Master:  Harmonigraph hub and analyzer
```

One tuner can sit before a Bitwig Instrument Layer and cover the instruments inside it.
Independent tracks need independent tuner instances because each note stream must be changed before its instrument consumes it.

The tuners communicate with Harmonigraph internally.
They do not create host MIDI routes to the hub, and each still sends ordinary note output only down its own track.
Their reports replace the manual Note Receiver routing currently needed to display several tracks together.

The first implementation ships a separate lightweight tuner plugin class rather than making the full plugin switch roles.
That keeps the pre-instrument instance free of the analyzer, audio rings, recorder, renderer and editor.
Both classes are exported from the one existing bundle, `nice_export_clap!(Harmonigraph, HarmonigraphTune)`, because a process-wide registry is shared only inside one dylib;
the loader scripts see the same bundle they see today.
The [stage 1 discovery handoff](adaptive-tuning-stage-1.md) records the plist fingerprint refresh needed for Bitwig to discover a changed class list after installation.
The Bitwig spike proved the companion and hub share the required in-process session under `by Vendor` hosting in its measured graph;
this does not establish every hosting mode in #623's wider matrix.

## Locked tuning behavior

The tuning model is **fixed-delay central sequencing with sequential assignment and no reconciliation**:

- each tuner buffers its performance stream and submits timestamped intents;
- the hub waits for complete input intervals in buffered state, orders their events and assigns new notes sequentially;
- each new assignment takes the preceding assignments into account, including simultaneous notes from other tracks;
- tuners normally emit every performance event at its mapped input time plus one common fixed delay D;
- the adaptive correction remains fixed from actual onset through release and composes with later player expression;
- if an assignment is unavailable at its deadline, its note remains pending and the session reports a timing failure;
- a valid late assignment permits a late attack, never a correction to an already-sounding note.

The delay is a scheduling budget, not time spent blocking an audio callback.
There is one execution path, with no optimistic fallback, joint chord solver or runtime synchronization mode switch.
Reconciliation and adaptive movement of held voices are excluded absolutely.

Simultaneous cross-track D, F and A must be evaluated as one ordered sequence, not three independent choices from an empty prior state.
Which note establishes the comma placement is not a product preference;
the initial order is ascending MIDI key with deterministic ties.
This solves the missing cross-track context, not every possible incompatibility with already-held pitches or policy constraints.
Sequential assignment is still chronological and path-dependent.
A note ending changes future context but does not move the survivors.

## Delay and late assignments

The product target is approximately **10 ms of added tuner delay**, on top of the existing monitoring path.
An initial candidate of **512 samples at 44.1 kHz (11.610 ms)** with a 512-sample engine buffer is acceptable;
a 128-sample engine buffer is not a product requirement.
The [#615 measurements](tuning-probe-bitwig.md) retained D = 2,048 samples (46.440 ms) at every tested engine buffer and did not search for a minimum.
Normal replies at a 512-sample buffer arrived in the next source callback, supporting the smaller candidate;
calibrated boundary cases at 1,024 samples sometimes needed another callback.
Neither one buffer nor 11.610 ms is a proven bound for the production policy or arbitrary routing.
Validate the smaller candidate during normal implementation validation, including event boundaries, branch offsets and the chosen workload limits;
do not require another standalone latency spike before beginning the feature.
If the candidate fails, report the measured reason before increasing the accepted latency or requiring a smaller engine buffer.
Select and report D in samples for an activation, accounting for the supported maximum callback size and measured lead/lag.
Changing D follows CLAP's activation/restart contract.
Bitwig compensated scheduled playback and export in the measured configuration;
that compensation cannot remove the tuner's added delay from live playing.

Delay the complete performance stream, including note-offs, choke, pedals, expression and unrelated MIDI, so healthy operation preserves durations and gestures.
Keep D while the tuner's participation control is Off or its session is disconnected.
Host bypass and removal are separate lifecycle cases measured by #615.

A missed assignment deadline publishes an explicit, latched diagnostic through a bounded status path while the callback continues.
Do not discard an answer merely because its deadline passed:
accept it only while its request, voice lifetime, source incarnation, session epoch and configuration binding still match the pending attack.
Emit that attack at the earliest legal output opportunity under the late-event scheduling contract.
Do not drop the note, emit it with zero correction, or emit first and repair its pitch later.
Extra lateness is a fault outside the reported fixed D, so normal plugin delay compensation cannot remove it.

During lateness, the affected track may temporarily fall behind while other tracks continue.
Preserve the queued notes' durations and expression relationships;
do not burst accumulated notes out to catch up.
A release or expression must not overtake its delayed attack;
dumping accumulated on/off pairs into one sample must not silently collapse a played note to zero duration.
An unrelated pending attack must not add delay to releases or expression of voices already sounding before the fault.
Return to D at a safe idle boundary with no queued performance events or locally held voices, neutral pedals and an accepted sequencing/recovery boundary.
Until then, later attacks on the affected stream can inherit its extra delay.
This permits temporary cross-track timing differences and does not promise a catch-up time during an uninterrupted phrase.
#616 defines the exact ordering of retriggers, pedals and shared-channel events across that boundary, including the dependency-safe exceptions to original stream order during failure.
Keeping musical order does not by itself guarantee simultaneous acoustic attacks during a deadline failure;
cross-track release barriers are not implicitly part of this design.

Persistent failure is not an indefinitely supportable delay with finite storage.
At actual required-storage exhaustion, enter a visible, latched **emergency stop**:
cancel affected queued attacks, invalidate their replies, send releases for affected sounding voices and reject new attacks until explicit Reset and valid recovery.
This deliberately permits note loss at catastrophic exhaustion;
a missed deadline, elapsed timeout or unavailable hub alone does not authorize it while the required state still fits.
Source-local exhaustion stops that source;
hub/global exhaustion stops the session.
Dependent unplayed decisions on other sources must be invalidated or rescheduled without treating their planned state as confirmed output.
Reserve a failure-signaling path that still works when ordinary queues are full.
Release/cancellation delivery and its completion tracking must also survive full ordinary queues;
local state cannot declare a voice terminated before accepted downstream output or a measured host termination boundary.
Do not silently evict notes, emit unretuned substitutes or allocate an unbounded queue.
Explicit Stop/Reset cancellation is a separate lifecycle event described below.

## Emitted assignments are authoritative

The confirmed project state contains the pitch assignments the tuners emitted downstream, not a recomputed ideal chord.
This is not a measurement of the resulting acoustic pitch:
a receiving instrument may ignore or smooth the expression or add its own modulation.
Within Harmonigraph's observable event protocol, the emitted assignment is nevertheless the only honest authority.
The sequencer also needs pending assignments so each new note can use its scheduled predecessors.
Those assignments are prospective state, not evidence that the note already sounded.
Their request identities, planned times and eventual emission outcomes must remain distinct from the confirmed voice set.

The same emitted-assignment stream drives:

- confirmed adaptive context and recovery;
- the lattice, note roll and other live views;
- the take recorder;
- offline replay of that take.

Recording actual emitted assignments and actual mapped output times preserves late attacks as played, rather than reconstructing an ideal on-time result.
Input intent time and planned output time remain separate protocol fields.
#616 must define how a missed deadline affects pending decisions that assumed an on-time predecessor and when confirmed state permits normal sequencing to resume.
Do not silently assume that planned note lifetimes still match audible ones after a timing failure, and never repair this discrepancy by retuning a sounding voice.

This stream is the vocabulary the hub already consumes.
A report is a note-on followed by a per-note Tuning event, which the tracker, the note roll, the take format and offline replay all handle today.
Aggregation adds a source id to that emitted event and a queue to carry it.
Input intents and assignment replies are additional internal scheduling messages, not competing canonical display or take streams.
Source recovery also needs an explicit state-baseline control record so a held set can be restored without pretending that MIDI attacks were emitted again.
That control reaches the display and take replay as well as the session model.

## Musical state ownership

The hub's audio callback owns the fixed-capacity sequencer, pending assignments and confirmed active-voice model.
It consumes the same source-aware lifecycle and pitch reports forwarded to the live display and take recorder.
The existing `NoteTracker` and `NoteRoll` remain downstream display/history consumers:
their `BTreeMap`, `Vec` and bend-history allocations cannot run in the audio callback.
GUI-ring loss or a delayed background drainer must not alter adaptive context.

Effective tuning is also resolved independently of the editor.
Extract the pure comma detection and axis-derivation rules currently in `harmonigraph-ui::begin_frame` into shared musical code;
the audio-owned resolver receives restored settings, host parameters and explicit UI edits through a coherent bounded handoff.
It publishes one configuration revision containing the effective tuning, tempered commas, policy version and musical search bounds.
The UI mirrors that resolved configuration instead of running a competing authority.
A revision becomes eligible at an explicit input-time sequencing boundary, so a policy call cannot mix new axes with old policy settings.
Bind each issued assignment to its configuration revision;
a later configuration edit does not reinterpret its reply.
Restoring state and automating tuning must work with the editor never opened.
A configuration change affects future decisions and context interpretation, never the frozen offsets of held voices.

## Intent, assignment and output flow

The real-time protocol has input intents, central assignment replies and actual output reports:

```text
notes -> Tune A buffer -> assigned, delayed notes -> instrument A
           |    ^                  |
   intents |    | assignments      | actual output
           v    |                  v
         Harmonigraph hub: ordered sequencer + confirmed state
           ^    |                  ^
   intents |    | assignments      | actual output
           |    v                  |
notes -> Tune B buffer -> assigned, delayed notes -> instrument B

Actual output -> combined display and take
```

For each processing interval a tuner:

1. maps incoming events using the host clock contract proved by #615;
2. retains their performance data in fixed-capacity pending storage and submits ordered intents;
3. advances its input processed-through watermark only after all preceding intents are available to the hub;
4. consumes replies addressed to the exact pending requests, validating their incarnation, epoch and configuration binding;
5. emits ready events at input time + D, or retains an unresolved attack and signals a deadline failure;
6. reports actual output times, pitches and lifecycle events, and advances separate output progress.

The hub merges complete input intervals by mapped sample rather than callback arrival order.
It preserves per-source lifecycle dependencies and uses deterministic ordering for independent same-sample attacks.
It applies releases and expression at their proper event times, assigns each onset once and immediately incorporates that assignment into prospective context before assigning the next.
It sends bounded replies to the originating tuners and consumes actual emission reports for confirmed state, display and take.
Tuners run no musical assignment policy and maintain no remote snapshot overlay or assignment-history replica.

Work is bounded per callback.
Incomplete intervals stay pending without any callback waiting for another instance.
The normal D must leave enough measured callback opportunities and work budget for input completion, assignment and reply delivery.
Neither callback arrival nor publication of a later interval authorizes skipping an unresolved earlier intent.

## Time and region contract

A coordination region is a shared musical interval whose boundaries and relation to the hub's clock are established by #615. It is not assumed to be one Rust `Plugin::process()` call.
The installed nice-plug wrapper splits a host CLAP callback on transport events, and can also split on automation when enabled.
Two such sub-blocks may both report offset zero before the hub drains either queue.
Neither a per-call counter nor the hub's current block start can recover the missing sub-block offset.

Intents and output reports therefore carry the session epoch, source incarnation, monotonic source sequence and mapped event sample.
Replies additionally identify the exact pending attack and its bound configuration revision.
Keep input intent time, planned output time and actual output time distinct.
The mapping accounts for the enclosing host callback, sub-block start, event offset and any measured track-latency relationship.
The concrete clock source and boundary hook are a required #615 result;
the measured mapping is raw `steady_time` plus a fixed instance offset, the Rust sub-block start and the local event offset.
One branch with 64 samples of limiter latency required a +64 source offset to recover simultaneous cohorts.
Do not substitute independent counters that merely happen to start together.

Progress endpoints are exclusive:
an input watermark at N means all source intents strictly before N are available.
The sequencer can finalize events at sample t only when every included source has proved progress beyond t.
Track the beginning of each continuous coverage interval as well as its exclusive endpoint;
a returning source cannot establish completeness for time it did not process.
Use one membership snapshot for collection and the completeness calculation so concurrent deactivation cannot remove a limiting source midway through a pass.
These are the two completeness corrections retained by #630's exported-CLAP regressions.
Output progress independently identifies complete emitted reports.
It cannot be inferred from input progress or assignment publication, especially when an attack is late.

Every participating source publishes completed input and output progress even when it has no notes.
A watermark advances only after all messages before it are available to the hub;
queue loss invalidates that progress rather than asserting a complete interval.
Activity counters help detect stopped callbacks but do not replace sample watermarks.
#615 records selected live/stopped input, transport, sub-block, branch-latency and offline cases with their exclusions;
actual sleep/wake, variable enclosing callbacks and manual seeks while held remain unproven.
Its activation/reset observations constrain how a returning source acquires a valid mapping before its reports are accepted.
An intent arriving behind a finalized input frontier or a report arriving behind confirmed output progress is a protocol failure requiring source recovery, not an event to restamp at drain time.
A valid assignment arriving after its output deadline is a different case:
it remains usable by its still-pending attack.

## Bounded storage and ownership

The hub is the only writer of sequential policy state and history.
Tuners own their pending performance events, actual local held voices, incoming player expression and frozen offsets.
The protocol distinguishes intents, assignments, emission outcomes, source progress and recovery controls.
Tuner reads of region-bound global snapshots are no longer on the assignment path.
Any snapshots used for status or downstream views describe their confirmed frontier and do not replace the request/reply protocol.

#617 selects the concrete bounded storage and publication primitives and documents their ownership and memory-ordering argument before wiring them into plugins.
Use per-source single-producer/single-consumer channels where their ownership fits;
keep intents and actual output distinguishable and provide a return path for assignments in #616. Concurrent payload access must be atomic or excluded by slot ownership;
a plain or volatile struct copy racing a writer is not made safe by a seqlock retry afterwards.
Reader acquisition and writer publication are bounded.
Any shared allocation retains an off-thread owner until callback users have quiesced;
registration, unregister and final reclamation never free it in an audio callback.

Record named limits for source slots, voices per source and per session, input and output queues, assignment replies, pending performance events, merge storage, history and callback work budgets.
Record actual byte sizes and the total memory budget, including the buffering needed for D and the supported late-event margin.
The current workload is about 15 simultaneous notes across three tracks;
the initial capacity targets deliberately leave substantial growth room:

| Resource | Initial capacity target | Scope |
|---|---:|---|
| Registered tuner sources | 16 | Per session; reserve the hub's direct-input identity separately |
| Simultaneously held voices | 64 | Per source, including locally held Off voices |
| Simultaneously held voices | 256 | Across the session, including any admitted hub direct-input context |
| Pending performance events | 8,192 | Per tuner; includes attacks, releases, pedals, expression and unrelated MIDI |

Per-source voice limits share the global limit;
16 sources cannot each occupy 64 global context slots simultaneously.
Pending lifetimes and historical assignments are separate from the held-voice counts and need their own bounded accounting.
Event capacity is not a duration guarantee:
dense expression can consume it much faster than sparse notes.
Choose the related intent, reply, output, merge, baseline and history capacities in #617 with an explicit admission/ownership argument and reserve recovery space independently.
Do not silently overwrite still-required history or state when any limit is reached.
Registration beyond 16 refuses the extra source visibly without evicting an existing source;
exceeding an admitted voice or required event/state limit follows the emergency-stop contract.
These are engineering sizing targets, not measured throughput claims;
#617/#621 must check the complete central workload and actual byte budget at the selected limits before claiming normal-delay support.
Any necessary capacity revision is documented against that measurement, rather than silently shrinking support to the three-track fixture.
Source health uses the measured audio progress/region model, with explicit reset behavior and no wall-clock worker needed for correctness.
Queue saturation and invalidation must remain observable even when the ordinary channel is full.
Required-storage exhaustion follows the visible emergency-stop contract, never an unretuned assignment fallback.

## Voice identity

Channel and MIDI key are not enough once several tracks contribute.
Two tracks can hold channel 0 / key 60 at the same time, and releasing one must not release the other.

The session address is:

```text
SessionVoiceId {
    source_instance_id,
    channel,
    key,
}
```

That is complete by contract rather than by luck.
The tuner leaves its poly-modulation config unset, so it never advertises the CLAP voice-info extension, and a host must then not send overlapping note-ons for one key and channel.
A retrigger that arrives anyway replaces the held voice, which is the rule the tracker already applies.
The host's note id is passed through on every emitted event, which the framework does today, and the tuning expression is addressed with it;
it is not part of the identity.

The source id reaches every identity-bearing path:
core events, the active-voice model, tracker keys and held-end keys, `NoteRoll` live keys and bend/release lookup, take records and offline replay.
Reserve an identity for the hub's direct input so it cannot collide with a tuner.
A source reset releases only that source;
a session reset has explicit all-source scope.
The session also carries an incarnation for each reusable source slot, so queued reports from an earlier occupant cannot affect the replacement.
This changes the take format, including recovery/reset scope;
the implementing PR states the break and follows the persistence contract.
The picture may still fold equal pitches together.

The initial active lifetime is note-on through note-off or choke.
Sustain-aware harmonic context is deferred until a real sustain-heavy project demonstrates the need;
the tuner still forwards pedals and unrelated MIDI with the same normal D as the note stream.
Sustain-aware context remains separate from the late-event scheduling rules.

## Pitch output

The first and only output backend is CLAP per-note tuning expression, and the tuner companion is exported only as CLAP initially.
At note-on the tuner emits a `CLAP_NOTE_EXPRESSION_TUNING` value for the same voice and sample position as the note.
CLAP note expressions state the current value rather than adding a delta.
The tuner therefore retains the player's current tuning expression per voice and emits `player expression + frozen adaptive offset` at note-on and after every later player-expression change.
It never forwards a later player value unmodified because that would erase the adaptive offset.
The framework already carries this boundary:
nice-plug emits a PolyTuning output event as `CLAP_NOTE_EXPRESSION_TUNING` with the note id preserved, so the tuner emits the same event kind the hub already receives.

CLAP note expression is a good fit because it is sample-accurate and can address a distinct voice by note ID, port, channel and key.
The Bitwig spike verifies event ordering and the exact instruments used in practice before the feature is considered viable.
Bitwig converts a note effect's per-note pitch into MPE or VST3 note expression for whatever instrument sits downstream, so CLAP-only output does not restrict the instrument's format;
what #615 checks is that the conversion reaches the instruments in use.

MTS-ESP is not a second implementation of the same semantics.
It publishes a global note/channel tuning table that clients query, cannot naturally distinguish simultaneous same-key voices on one channel, does not aggregate note lifecycle, and may move held notes when a client continuously polls an updated table.
MPE adds channel allocation, bend-range and reset state.
VST3 note output adds another host/interoperability matrix.
None belongs in the first implementation.

Keep the musical core expressed in Harmonigraph's exact pitch representation and convert to a CLAP semitone offset only at the plugin boundary.
That is enough separation to add a concrete compatibility backend later without maintaining an unused abstraction today.

## Session, pairing and process boundary

The full Harmonigraph owns the authoritative tuning configuration and one persisted session UUID.
A tuner auto-joins when exactly one compatible hub is healthy under the measured activity rule, retains explicit pairing when duplicated and reports missing or ambiguous hubs instead of guessing.
Two open project copies carrying the same saved UUID are ambiguous rather than one combined session.
The saved pairing UUID is distinct from the runtime session incarnation and time epoch;
neither a reload nor a transport reset may make an old intent or assignment reply valid again.

The first backend is an in-process registry with bounded per-source intent/output channels and an assignment return path, with the ownership rules above.
Bitwig's **by Vendor** hosting mode is the measured initial requirement, with individual hosting overrides off for both Harmonigraph classes.
Issue [#623](https://github.com/yan-h/harmonigraph/issues/623) records the wider process/reload investigation;
untested cells in its matrix are not additional launch requirements for the already measured mode.

Initial support permits fixed calibrated routing and reinitialization after changes that invalidate track-clock alignment.
Changes to branch latency require offset revalidation before adaptive sequencing resumes;
automatic calibration through arbitrary graph edits is not claimed.
Known held voices retain their offsets while the session reestablishes a valid baseline.
If downstream held state is unknown, require an explicit voice reset and confirmed termination before rejoining.
Continuous callbacks remain a supported-configuration requirement;
missing progress is not silent input, and unobserved sleep/wake behavior is not accepted as safe recovery.

The hosting mode also decides what a build swap costs.
A sandbox process re-reads the plugin binary only when it starts, and a grouped process lives as long as any instance in its group is loaded, so a project with a tuner on every track holds the old image until every one of them is unloaded.
Which gesture short of a Bitwig restart does that is measured on #623, and the loader's own tripwire for the case is issue [#624](https://github.com/yan-h/harmonigraph/issues/624).
Bitwig's plug-in settings also carry a per-plug-in list that runs a named plug-in Individually under any global mode, which is how another plug-in of the same vendor is kept out of the session's process without renaming anything.

Cross-process shared memory is not part of the initial design.
It adds process discovery, stale participants, crash recovery and system-level synchronization without improving the intended personal Bitwig workflow.
There is no pointer-free arena or memory-mapped ABI requirement in the in-process implementation.
`rtrb` stores pointers and `Arc` ownership, which is compatible with preallocation and off-thread reclamation but not a relocatable mapping.
A future cross-process implementation must justify its own layout, ownership, synchronization and recovery costs.

The hub normally sits on Master because it is downstream of the participating audio and can analyze their combined signal.
Master placement is a candidate source-to-hub ordering advantage, not proof of a complete barrier or a one-buffer round trip.
#630 established complete intervals and timely replies at its measured D;
the smaller production candidate still needs normal implementation validation.
The session belongs to the plugin process rather than the editor, so closing the Harmonigraph window must not stop progress, sequencing or tuning.

## Real-time constraints

Registration, naming and allocation happen away from the audio callback.
The callback only touches bounded preallocated queues, pending-event storage and fixed-capacity state.
It never:

- waits for another plugin instance;
- takes a contended lock;
- allocates or frees, including through a shared reference it happens to hold last;
- performs socket or filesystem I/O;
- depends on an editor or wall-clock background worker.

Offline rendering runs the same sample-timed state machine and cannot depend on a worker keeping up with faster-than-real-time processing.

## Controls and failure behavior

The initial tuner has one participation control rather than three independent visibility/context/retune switches:

- **Participating:** report actual output, contribute to context, appear in Harmonigraph and retune new notes;
- **Off:** remove the source from project context and visualization and intentionally leave newly received notes unretuned at the same D, while finishing the expression/lifecycle handling of already-held voices.

Off is an explicit user selection, not an automatic deadline fallback.
It affects newly received notes;
preexisting pending adaptive requests remain adaptive and finish with their valid assignments under the late-stream rules unless explicitly canceled by Stop/Reset or emergency stop.
Withdrawing a source from future context does not erase its pre-transition sequencing obligations or permit it to stop required callbacks.
Already-issued replies retain their original configuration binding across a later configuration edit.

An explicit transport **Stop** transition or user **Reset** cancels affected pending attacks, invalidates their replies and dependent planned state, and sends releases for affected sounding voices at the earliest legal output opportunity without the retained stream delay.
Handle sustain/hold state so a queued pedal or release cannot keep those voices alive after the cancellation.
Instrument release tails can remain;
the tuner must not postpone the release command by its accumulated delay.
Reset the extra delay and resume normal D only after cancellation and the new sequencing boundary are established.
The measured stop defect is [#632](https://github.com/yan-h/harmonigraph/issues/632).
Being stopped is not itself a repeated reset:
new live input while stopped still follows normal tuning, and a loop wrap is not automatically a Stop transition.
Emergency-stop recovery remains latched until explicit Reset;
selecting Off cannot bypass that latch.

Specialized states such as a visible-but-untuned track or a fixed anchor are added only when the musical policy has a concrete use for them.
The single participation control does not remove the need for internal transition states:

| Transition or state | New or pending notes | Already-held voices and session state |
|---|---|---|
| Healthy and participating | Central sequential assignment, normally emitted at input time + D | Continue reporting actual output and composing player expression with each frozen offset |
| Participating to Off | Newly received notes intentionally use zero correction; pending adaptive requests finish tuned under the normal/late timing contract | Withdraw this source from future context/display; preserve existing offsets until release |
| Assignment deadline missed | Retain the attack for its valid assignment and report a timing failure; no unretuned or dropped-note fallback | Preserve frozen offsets; handle related queued events under the specified late-event schedule |
| Missing, ambiguous, expired or overloaded session | Report the fault and hold unresolved participating attacks while required storage fits | Preserve locally known offsets and lifecycle; do not use stale context or invent new assignments |
| Required-storage exhaustion | Cancel affected pending attacks visibly and reject new attacks until explicit Reset and valid recovery | Send releases through the independent emergency path; latch source-local or session-wide failure as appropriate |
| Transport Stop or explicit Reset | Cancel affected pending attacks and reject their obsolete replies | Send releases without accumulated delay, invalidate dependent plans and establish a fresh sequencing boundary |
| Off to Participating, reconnect or report-loss recovery | Resume adaptive sequencing only after the complete held baseline and pending-request state are accepted | Restore actual state without re-emitting attacks or retuning survivors |
| Source unregister or slot reuse | Old requests and replies cannot address the replacement | Invalidate the old incarnation and release only its context/display voices |
| Explicit voice reset or host-guaranteed note termination | Cancel obsolete pending lifetimes under the reset contract, reject their replies and start fresh | Clear offsets only after downstream voices are terminated or the host guarantees they are gone |

A later player-expression event is still emitted as `player value + frozen offset` while Off or disconnected.
Otherwise that event would erase the held voice's correction.
The local table also tracks notes deliberately started with zero correction while Off so rejoining restores the actual held set.
No note is assigned zero correction merely because a participating request missed its deadline.
Transient policy history is cleared on participation withdrawal, configuration revision change, session/epoch change and lifecycle-loss recovery;
clearing that history never changes a held offset.

Recovery uses a bounded complete baseline with a sequence cut and an acknowledgement, followed by later ordered deltas.
The hub replaces only that source's confirmed state when the whole baseline is accepted;
partial baselines are never published as complete context.
Pending unsounded requests require a separate disposition, so a held-state baseline cannot acknowledge an attack that was never emitted.
It carries the voices' original onset information, current emitted pitches and assignment metadata, and an explicit recovery boundary to the display/take path.
Resetting an empty queue alone is insufficient because a held voice may never send another note-on.
Overflow signals and incarnation invalidation must remain deliverable when the ordinary report queue is full.

Host bypass and plugin removal differ from the Off control because callbacks may stop entirely.
#615 measured downstream signal termination for the specific Bitwig bypass/removal actions in its fixture, while mute/solo preserved held lifetimes and callbacks.
Those observations do not establish termination for every missing callback or host action.
If lifecycle events were missed, do not republish an assumed local held set on resume:
require a fresh authoritative baseline or terminate/reset the affected downstream voices before clearing local state and rejoining.
The supported behavior and its audible reset consequence must be documented.
No code running in a stopped callback is assumed to repair downstream notes.

There is no hidden local adaptive mode during failure and no pitch correction used as reconciliation.

## Musical policy boundary

The hub runs one versioned pure policy in sequence:

```text
assign_new_note(
    tuning_configuration,
    context_at_this_intent,
    assignment_history,
    next_ordered_note_on,
) -> (initial_voice_assignment, history_update)
```

The sequencer calls this for each ordered onset and incorporates its result before the next call, including across sources at the same sample.
The context includes the appropriate scheduled predecessors and confirmed current pitches under the healthy timing contract.
Planned state and confirmed output stay distinct during failure and recovery.
The infrastructure proof uses an obviously artificial deterministic policy whose result depends on an earlier assignment from another source at the same sample.
It must not choose the eventual just-intonation behavior accidentally.

Assignment history is explicit transient state keyed by source/channel/key and retains the previous selected node after release.
It is capacity-bounded, is not a second held set and is not persisted.
Pending history advances with sequential decisions, but must retain its relationship to the requests and confirmed emissions that justify it.
#616 specifies invalidation/reset behavior if planned and actual output diverge.
Empty usable context uses the origin preference and ignores history from the preceding phrase.

The real policy still has to select its initial search bounds and scoring constants.
Anchors, additional root controls and excluded-pitch controls remain deferred.
Measure the total central work against the callback budget;
no distributed policy or precomputed next-note map is part of this design.

The first real policy is issue [#621](https://github.com/yan-h/harmonigraph/issues/621), the nearest connected lattice node.
Its candidate search uses bounded musical coordinates around the fixed lattice origin, recorded in the effective policy configuration.
It never reads `ViewConfig` reach, camera centers, drawn windows or display tolerance.
The initial bounds and fixed scoring constants are named policy constants selected and documented in #621 before implementation;
they require no new user control or persisted field.
Candidates lie within 50 cents of the key's equal-tempered class, measured circularly with exact pitch arithmetic, and comma-equivalent nodes are deduplicated after respelling.
An empty candidate set returns an explicit no-candidate result that the initial musical policy defines as zero adaptive offset, retaining player expression and clearing this key's assignment history.
This is a completed policy result, not a missing assignment or deadline fallback.

Every voice carries its current emitted pitch separately from optional attack-time node metadata and the configuration revision that selected it.
The emitted pitch is always updated when player expression changes;
the attack node is never silently treated as the current pitch.
For scoring under the current configuration, reuse a node only if it remains in the musical domain and still exactly represents the emitted pitch.
Otherwise project that pitch to the nearest node in the musical domain within the policy's fixed 50-cent context radius, with the policy tie-break.
A voice with no such node remains in display/take/state but contributes no lattice-distance term.
This projection is an explicit policy approximation and never replaces the authoritative stored pitch.
It handles zero-offset voices, expression bends and voices held across configuration changes by the same rule.

The score combines summed L1 lattice distance after respelling, an explicitly weighted hysteresis distance and the origin preference when no usable context remains.
Use an integer/rational score with named weights so identical inputs produce identical results across platforms.
Tie-break by smallest absolute threes, fives and sevens, then signed coordinates in that order.
Within a complete same-sample cohort, evaluate independent note-ons across all sources in ascending key then channel order, with a deterministic source tie-break;
earlier assignments in that canonical order contribute to later ones.
Define the source tie-break and lifecycle precedence in #616 so callback permutations cannot change the result.
Preserve host lifecycle ordering for same-key replacements and the original per-source output order at samples shifted by D during healthy operation.
Canonical policy evaluation does not authorize moving an off, choke or expression across its addressed voice.

The emitted adaptive offset is the chosen node's pitch class minus the key's equal-tempered class, folded to the nearest octave.
The 50-cent candidate restriction bounds this correction relative to the input key, not the sum with player expression.
Hysteresis encourages continuity but is not an anti-drift guarantee;
#621 records the observed ii–V–I behavior and the bound the implementation actually enforces.
The policy stays pure in `harmonigraph-core` and remains the part to iterate by ear.

## Deliberately outside the design

The following are not launch modes or implied follow-up work:

- immediate optimistic assignment or a local adaptive fallback;
- joint chord optimization;
- reconciliation or adaptive movement of held voices;
- a central MIDI rack/router;
- a separate headless conductor;
- cross-process session transport;
- MTS-ESP, MPE or compatibility-first pitch outputs;
- raw-intention visualization as a second canonical event stream.

Reconciliation is excluded absolutely by Yan's decision, not deferred as a possible failure-recovery mechanism.
The other alternatives require a new request and a new issue before implementation.
The simultaneous cross-track context requirement is part of acceptance, including a D–F–A fixture.

## Evidence and current seams

These sources establish available mechanisms and the constraints that motivated the design:

- [CLAP events](https://github.com/free-audio/clap/blob/main/include/clap/events.h) defines sample-accurate note expressions,
voice addressing and relative tuning in semitones;
- [CLAP latency](https://github.com/free-audio/clap/blob/main/include/clap/ext/latency.h) defines latency in samples and limits changes to activation, with a restart request when already active;
- [Bitwig plugin hosting modes](https://www.bitwig.com/userguide/latest/vst_plug-in_handling_and_options/) describes **By manufacturer** as useful for plugins from one developer that communicate, and a per-plug-in list that runs a named plug-in Individually under any global mode;
- [Bitwig Note FX](https://www.bitwig.com/userguide/latest/note_fx/) establishes the pre-instrument note-effect placement;
- [MTS-ESP](https://github.com/ODDSound/MTS-ESP/blob/main/README.md) documents its single-master note/channel lookup and client-query model;
- [Rust volatile-read semantics](https://doc.rust-lang.org/std/ptr/fn.read_volatile.html) states that volatile access supplies no inter-thread synchronization.

The implementation starts from these repository seams:

- [`harmonigraph-plugin/src/lib.rs`](../crates/harmonigraph-plugin/src/lib.rs) declares basic MIDI input/output and forwards host events;
nice-plug passes the host note id through in both directions and emits PolyTuning as the CLAP tuning expression, but its transport-event sub-block splitting must be accounted for in #615's time mapping;
- [`harmonigraph-core/src/notes.rs`](../crates/harmonigraph-core/src/notes.rs) identifies tracked notes by channel and key and already consumes the note-on plus Tuning stream a tuner reports;
its allocating tracker stays off the audio thread, and every identity-bearing key gains source scope;
- [`harmonigraph-core/src/roll.rs`](../crates/harmonigraph-core/src/roll.rs) has an independent live-note map and bend history that also need source-aware identity and resets;
- [`harmonigraph-ui/src/lib.rs`](../crates/harmonigraph-ui/src/lib.rs) currently resolves effective tuning in `begin_frame`, so that authority must move into the shared audio-owned configuration path;
- [`harmonigraph-take/src/lib.rs`](../crates/harmonigraph-take/src/lib.rs) already records per-note Tuning and gains source/reset/recovery scope, with corresponding offline replay changes;
- [`harmonigraph-core/src/tuning.rs`](../crates/harmonigraph-core/src/tuning.rs) already supplies the exact pitch representation the policy and emitted-assignment state should retain.

The external documents do not prove actual Bitwig behavior in this plugin chain.
That empirical evidence belongs on #615 as bounded traces and measured verdicts, so an auditor can distinguish specification, design assumption and observed result.

## Implementation order

0. Use [#623](https://github.com/yan-h/harmonigraph/issues/623)'s measured engine-restart reload and [#630](https://github.com/yan-h/harmonigraph/pull/630)'s same-process `by Vendor` configuration.
Other hosting modes remain outside initial support until separately established.
1. Carry forward [#615](https://github.com/yan-h/harmonigraph/issues/615)'s constrained verdict and retained opt-in probe from #630.
Its measured D is 2,048 samples, not the accepted production target or a minimum.
Encode the observations and completeness regressions in production tests;
validate the smaller candidate as part of implementation without another prerequisite spike.
2. Before wiring plugins in [#617](https://github.com/yan-h/harmonigraph/issues/617), record the concrete storage primitives, capacity/health table, configuration handoff and source recovery protocol against #615's verdict.
Implement the companion, audio-owned confirmed active state and effective tuning, and source-aware display/roll/take/replay.
This aggregation-only milestone preserves input timing and supplies no adaptive output;
it does not create a second launch mode.
3. Before implementing [#616](https://github.com/yan-h/harmonigraph/issues/616), specify the event-ordering and prospective/confirmed-state mechanics that implement the accepted late-stream, Stop/Off, recovery and emergency-stop decisions above.
Add the central sequential assigner, bounded full-stream delay and replies with the artificial policy, plus deadline diagnostics and valid late-answer handling.
4. [#621](https://github.com/yan-h/harmonigraph/issues/621) replaces the artificial policy with the first real one, including the simultaneous cross-track D–F–A case.

If the spike rejects a premise, amend this document and #614 before continuing downstream.
