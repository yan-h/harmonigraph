# Project-wide adaptive tuning

## Status

This is the decided design for a planned feature;
none of it is implemented yet.
GitHub issue [#614](https://github.com/yan-h/harmonigraph/issues/614) is the design anchor, with separate children for the Bitwig timing spike, automatic aggregation, pitch output and the first policy.

The musical assignment algorithm is deliberately outside this design.
This document fixes the product and real-time contracts that algorithm will run inside.

## Design priorities

This feature is optimized first for one person's maintained macOS/Bitwig workflow, not for format or host coverage.
Correct musical state, an inspectable failure mode and one small execution path outrank compatibility that has no current use.
Add another format, transport, mode or control only when it serves a concrete project and is cheap enough to carry in every later change.

Host behavior is evidence rather than architecture.
Where the design depends on Bitwig process grouping, callback order, voice identity or CLAP event delivery, issue [#615](https://github.com/yan-h/harmonigraph/issues/615) measures the premise and either records the supported constraint or stops the downstream work.

## Decision record

This table preserves the strongest alternative and the condition that can reopen each choice.
A later implementation should not broaden a row merely because its rejected branch is technically possible.

| Decision | Chosen design | Strongest alternative | Why this won | Reopen only when |
|---|---|---|---|---|
| Attack timing | Immediate optimistic assignment, frozen through release | Synchronized attacks or later reconciliation | Avoids MIDI buffering, live latency, PDC, transition policy and a second failure state machine; simultaneous cross-track blindness is expected to be negligible in practice | Listening or shadow measurement demonstrates a musically material discrepancy |
| Plugin boundary | Separate lightweight Harmonigraph Tune class exported from the same CLAP bundle as Harmonigraph | Full-plugin instances or one class with persisted Hub/Tuner roles | Keeps the pre-instrument device and its lifecycle small and gives the host a clear note-effect identity; a process-wide registry is shared only inside one dylib, so one bundle is what makes an in-process session possible at all | #615 shows separate classes cannot share a reliable supported process/session topology |
| Snapshot age | Latest snapshot sealed before the current processing region | Fixed sample look-behind horizon | Uses the freshest causally complete context without a latency or staleness parameter | Measurement shows materially different live/offline results across buffer configurations |
| Snapshot sealing | The hub seals one snapshot per processing region and tuners read only sealed snapshots | Tuners read one another's live state directly | Whether track B sees track A's same-region note is then fixed by the region boundary rather than by which callback thread ran first; the region is the unit of simultaneity and scheduling is invisible | The hub cannot run after every tuner within a region in the supported graph |
| Snapshot storage | Fixed-capacity slots under a seqlock, copied into tuner-local storage at region start | Shared reference-counted immutable snapshots | The last holder of a shared reference frees it on the audio thread; copying a few hundred bytes with bounded retries keeps the callback allocation-free | A snapshot outgrows the fixed capacity |
| Report clock | The hub stamps each report on its own clock at the report's in-region offset as it drains the queues | Per-source absolute sample clocks and watermarks inside the protocol | The framework exposes no host-shared steady clock, the hub runs after the tuners, and same region plus same offset is already the ordering; a per-source region counter is the only liveness the protocol needs | #615 shows the hub does not run after every tuner within a region |
| Voice identity | Source, channel and key, with the host note id passed through untouched | Host voice id as identity with a multiplicity fallback for its absence | The tuner advertises no overlapping-note support, so the host must not overlap one key and channel; a retrigger replaces, which is the rule the tracker already applies | #615 observes Bitwig delivering overlapping same-key notes to a note effect that does not advertise them |
| State authority | Assignment emitted downstream for each voice | Ideal assignment recomputed later by the hub | Future notes and recorded takes must use the pitch Harmonigraph actually requested rather than a hypothetical result | A real downstream pitch-feedback mechanism exists and is worth integrating |
| Pitch output | CLAP per-note tuning expression only | MTS-ESP, MPE or VST3 note output | Matches sample-timed per-voice frozen assignments and the actual personal host while adding no external tuning service; Bitwig converts a note effect's per-note pitch to MPE or VST3 note expression for the instrument downstream, so the instrument's format is not restricted | A required instrument cannot consume it or #615 disproves reliable delivery |
| Session transport | In-process registry under a documented Bitwig hosting mode | Cross-process shared memory | Avoids process discovery, crash recovery and stale shared state for compatibility that is not currently needed | #615 shows no usable in-process topology or a concrete workflow requires another hosting mode |
| Hub ownership | Full Harmonigraph, normally on Master | Headless conductor or elected tuner peer | Reuses the existing configuration, display, take and combined-audio location without another authority or plugin role | The hub cannot remain active in the supported graph or a project demonstrably needs tuning without a full Harmonigraph |
| Unhealthy session | Discard remote context and pass new notes through unretuned | Continue with stale context or a local-only adaptive mode | Failure is obvious and deterministic and cannot silently substitute a different tuning system | Measured transient session gaps make the fail-open result more disruptive than an explicitly designed alternative |
| Participation UI | One Participating/Off control | Independent visibility, context and retune switches | Minimizes persisted states, combinations and tests before anchors or monitor-only tracks have a concrete musical contract | A real project requires a specific excluded combination |
| Policy location | Same pure policy runs in each tuner | Hub-precomputed next-note map | Naturally includes the tuner's newer local overlay and same-sample batch without another response path | Profiling shows the actual policy is too expensive on the audio thread |

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
The Bitwig spike must prove that the companion and hub share the required in-process session under the documented hosting mode;
only a failed premise can reopen the packaging decision.

## Locked tuning behavior

The tuning model is **optimistic with no reconciliation**:

- a tuner chooses an adaptive correction when a note starts;
- it emits the note and correction immediately at the original sample position;
- the adaptive correction remains fixed until that voice ends;
- new notes use the last eligible sealed project state plus newer state from their own tuner;
- later project changes never retune an already-sounding voice.

There is no intentional coordination latency, performance-stream buffer, decision deadline, plugin delay compensation or attack-time response from the hub.
There is also no runtime choice between optimistic, reconciled and synchronized modes.

Notes starting at the same sample within one tuner are one batch and may take one another into account.
Notes starting simultaneously on independent tracks consume the same prior global state and do not see one another's new assignments.
Later notes see the combined result once the hub has sealed it.
A note ending changes future context but does not move the survivors.

This behavior is chronological and can be path-dependent.
That is the product semantics rather than an approximation awaiting a globally synchronized answer.

## Emitted assignments are authoritative

The project state contains the pitch assignments the tuners emitted downstream, not a recomputed ideal chord.
This is not a measurement of the resulting acoustic pitch:
a receiving instrument may ignore or smooth the expression or add its own modulation.
Within Harmonigraph's observable event protocol, the emitted assignment is nevertheless the only honest authority.
Otherwise a later note could adapt to an ideal pitch that was never requested downstream.

The same emitted-assignment stream drives:

- future adaptive context;
- the lattice, note roll and other live views;
- the take recorder;
- offline replay of that take.

Recording assignments rather than reconstructing them also preserves the requested result if callback or render-buffer chronology proves observable.

This stream is the vocabulary the hub already consumes.
A report is a note-on followed by a per-note Tuning event, which the tracker, the note roll, the take format and offline replay all handle today.
Aggregation adds a source id to that event and a queue to carry it;
it adds no second event stream.

## Report and snapshot flow

The real-time protocol is one-way source reporting plus immutable snapshot publication:

```text
                 sealed snapshot N
               +--------------------+
               |                    v
notes -> Tune A +-> corrected notes -> instrument A
               |
               +-> actual assignment reports --+
                                                   |
notes -> Tune B +-> corrected notes -> instrument B |
               |                                   |
               +-> actual assignment reports ------+-> Harmonigraph hub
                                                        |
                                                        +-> snapshot N+1
                                                        +-> display and take
```

For each processing region a tuner:

1. copies the latest sealed snapshot into its own fixed storage;
2. overlays lifecycle state from its own source that is newer than that snapshot;
3. calls the shared pure assignment policy for its current same-sample note-on batch;
4. emits its note and correction immediately;
5. reports the actual assignment and later lifecycle events to the hub.

The hub drains every source queue once per processing region, stamps each report on its own clock at the report's in-region offset, feeds it to the same tracker and take recorder its own input reaches, and seals the next snapshot.
Same region and same offset is the ordering;
nothing across sources is finer than a region by design.
It does not make an attack-time decision for a tuner.

The hub seals rather than letting tuners read one another's live state, and the reason is thread order.
With a sealed snapshot, whether track B sees track A's same-region note is fixed by the region boundary;
with live peer reads it would depend on which callback ran first, and the chronology would change from run to run.

## Snapshot contract

A snapshot is equivalent to:

```text
GlobalSnapshot {
    session_generation,
    snapshot_sequence,
    committed_through_sample,
    tuning_configuration,
    active_voices_with_actual_assigned_pitches,
}
```

A snapshot is a fixed-capacity value in one of a small ring of slots the hub owns, each guarded by a seqlock.
At region start a tuner copies the newest slot into its own storage with bounded retries and keeps its previous copy if every retry fails.
Nothing is reference-counted across threads, so no tuner frees anything on the audio thread, and a tuner never reads the hub's mutable tracker directly.

The default eligibility rule is the latest snapshot committed before the current host processing region.
This supplies the freshest causally complete state without delaying notes.
The implementation's own tests cover playing, stopped input, loops, transport resets, variable callbacks and offline rendering.
If buffer size proves to make the musical result unacceptably unstable, a fixed sample look-behind horizon is the amendment;
that is a measured amendment to this design rather than a second user-facing mode.

Every source also bumps a region counter on every process call, including calls with no events.
That counter is how the hub tells a silent source from one the host has stopped calling, and it is the only per-source clock in the protocol;
absolute sample positions belong to the spike's trace as a diagnostic, not to the session.

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

The tracker's channel field is documented as half of a note's identity and nothing else, so a source byte beside it is the whole change.
It reaches the core note event, the take's note record and the tracker's map key, and it changes the take format;
the implementing PR says so in its body.
The picture may still fold equal pitches together.

The initial active lifetime is note-on through note-off or choke.
Sustain-aware harmonic context is deferred until a real sustain-heavy project demonstrates the need;
the tuner still forwards pedal and unrelated MIDI without changing their timing.

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
A tuner auto-joins when exactly one compatible hub is active, where active means its region counter has moved recently, retains explicit pairing when duplicated and reports missing or ambiguous hubs instead of guessing.
Two open project copies carrying the same saved UUID are ambiguous rather than one combined session.

The first backend is an in-process registry with one bounded single-producer/single-consumer report queue per tuner and the seqlocked snapshot slots above.
Bitwig's **By manufacturer** hosting mode is the expected initial requirement because it groups plugins from one developer for communication.
Issue #615 verifies the exact process layout and the visible failure behavior of other hosting modes.

Cross-process shared memory is not part of the initial design.
It adds process discovery, stale participants, crash recovery and system-level synchronization without improving the intended personal Bitwig workflow.

The hub normally sits on Master because it is downstream of the participating audio and can analyze their combined signal.
Master placement is not an attack-time barrier;
tuners consume the previous sealed state.
The session belongs to the plugin process rather than the editor, so closing the Harmonigraph window must not stop reports, snapshots or tuning.

## Real-time constraints

Registration, naming and allocation happen away from the audio callback.
The callback only touches bounded preallocated queues, seqlocked snapshot slots and local fixed-capacity state.
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
- **Off:** remove the source from project context and visualization and pass MIDI through unchanged.

Specialized states such as a visible-but-untuned track or a fixed anchor are added only when the musical policy has a concrete use for them.
Host bypass and plugin removal still need explicit all-off/generation handling because a host may stop calling a bypassed instance.

Missing, ambiguous, stale or overloaded session state fails visibly and audibly safe:

- new notes pass through unretuned;
- stale remote voices are not used as context;
- already-sounding voices keep their frozen offsets until release or an explicit reset;
- lifecycle loss invalidates the affected source generation and resynchronizes rather than leaving a permanent voice;
- the tuner and hub show the exact disconnected, stale or overflow condition.

There is no hidden local adaptive mode during failure and no pitch correction used as reconciliation.

## Musical policy boundary

Every tuner carries the same versioned pure policy:

```text
assign_new_notes(
    tuning_configuration,
    sealed_global_snapshot,
    newer_local_source_overlay,
    same_sample_note_on_batch,
) -> initial_voice_assignments
```

The infrastructure proof uses an obviously artificial deterministic policy whose result depends on prior state from another source.
It must not choose the eventual just-intonation behavior accidentally.

The real policy still has to decide spelling, anchors, root behavior, excluded pitches, repeated keys and deterministic tie-breaking.
Its cost can be measured before considering a hub-precomputed candidate map;
duplicating a cheap pure calculation in each tuner is simpler and correctly incorporates local state newer than the snapshot.

The first real policy is issue [#621](https://github.com/yan-h/harmonigraph/issues/621), the nearest connected lattice node.
Candidates for a key are the window's positions whose pitch class under the hub's tuning lies within half a semitone of the key's equal-tempered class;
each is scored by summed lattice distance to the sounding voices' nodes, a small hysteresis term toward the node this key last took, and the origin when nothing sounds, with a deterministic tie-break.
The emitted offset is the node's pitch class minus the equal-tempered class, which is the Tuning event the picture already lights by pitch match.
It is pure, in `harmonigraph-core`, and the part to iterate by ear.

## Deliberately outside the design

The following are not launch modes or implied follow-up work:

- synchronized or delayed attack tuning;
- reconciliation or adaptive movement of held voices;
- a central MIDI rack/router;
- a separate headless conductor;
- cross-process session transport;
- MTS-ESP, MPE or compatibility-first pitch outputs;
- raw-intention visualization as a second canonical event stream.

They require new evidence and a new issue before implementation.
An optional development-only shadow measurement may later compare frozen output with a hypothetical complete-state result without changing MIDI.
That measurement would test whether synchronization or reconciliation has musical value rather than presuming it.

## Evidence and current seams

These sources establish available mechanisms and the constraints that motivated the design:

- [CLAP events](https://github.com/free-audio/clap/blob/main/include/clap/events.h) defines sample-accurate note expressions,
voice addressing and relative tuning in semitones;
- [Bitwig plugin hosting modes](https://www.bitwig.com/userguide/latest/vst_plug-in_handling_and_options/) describes **By manufacturer** as useful for plugins from one developer that communicate;
- [Bitwig Note FX](https://www.bitwig.com/userguide/latest/note_fx/) establishes the pre-instrument note-effect placement;
- [MTS-ESP](https://github.com/ODDSound/MTS-ESP/blob/main/README.md) documents its single-master note/channel lookup and client-query model.

The implementation starts from these repository seams:

- [`harmonigraph-plugin/src/lib.rs`](../crates/harmonigraph-plugin/src/lib.rs) declares basic MIDI input/output and forwards host events;
nice-plug passes the host note id through in both directions and emits PolyTuning as the CLAP tuning expression, so the boundary needs no framework work;
- [`harmonigraph-core/src/notes.rs`](../crates/harmonigraph-core/src/notes.rs) identifies tracked notes by channel and key and already consumes the note-on plus Tuning stream a tuner reports;
it gains a source byte in that key;
- [`harmonigraph-take/src/lib.rs`](../crates/harmonigraph-take/src/lib.rs) already records per-note Tuning, so a take carries emitted assignments once its note record carries the source;
- [`harmonigraph-core/src/tuning.rs`](../crates/harmonigraph-core/src/tuning.rs) already supplies the exact pitch representation the policy and emitted-assignment state should retain.

The external documents do not prove actual Bitwig behavior in this plugin chain.
That empirical evidence belongs on #615 as bounded traces and measured verdicts, so an auditor can distinguish specification, design assumption and observed result.

## Implementation order

1. [#615](https://github.com/yan-h/harmonigraph/issues/615) is one afternoon rather than a project:
a second class in the same bundle, a process-wide counter the hub bumps each region, a fixed +50 cent tuning expression after every note-on, and one log line per region from a background thread.
It answers process grouping, hub-after-tuners ordering, whether the tuning expression takes on the instruments in use, and whether the note id arrives.
Everything else on the spike's original list falls out of the implementation's tests.
2. [#617](https://github.com/yan-h/harmonigraph/issues/617) implements the companion, the session module, aggregation, the source byte and the sealed snapshots.
This alone replaces Note Receiver routing, a win before any note is retuned.
3. [#616](https://github.com/yan-h/harmonigraph/issues/616) adds local optimistic assignment and same-sample CLAP tuning expression with the artificial policy.
4. [#621](https://github.com/yan-h/harmonigraph/issues/621) replaces the artificial policy with the first real one.

If the spike rejects a premise, amend this document and #614 before continuing downstream.
