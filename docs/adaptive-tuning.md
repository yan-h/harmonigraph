# Project-wide adaptive tuning

## Status

This is the decided design for a planned feature;
none of it is implemented yet.
GitHub issue [#614](https://github.com/yan-h/harmonigraph/issues/614) is the design anchor, with separate children for the Bitwig timing spike, automatic aggregation and pitch output.

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
| Plugin boundary | Separate lightweight Harmonigraph Tune CLAP companion | Full-plugin instances or one class with persisted Hub/Tuner roles | Keeps the pre-instrument device and its lifecycle small and gives the host a clear note-effect identity | #615 shows separate classes cannot share a reliable supported process/session topology |
| Snapshot age | Latest snapshot sealed before the current processing region | Fixed sample look-behind horizon | Uses the freshest causally complete context without a latency or staleness parameter | #615 shows materially different live/offline results across buffer configurations |
| State authority | Assignment emitted downstream for each voice | Ideal assignment recomputed later by the hub | Future notes and recorded takes must use the pitch Harmonigraph actually requested rather than a hypothetical result | A real downstream pitch-feedback mechanism exists and is worth integrating |
| Pitch output | CLAP per-note tuning expression only | MTS-ESP, MPE or VST3 note output | Matches sample-timed per-voice frozen assignments and the actual personal host while adding no external tuning service | A required instrument cannot consume it or #615 disproves reliable delivery |
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

1. pins the latest eligible immutable global snapshot;
2. overlays lifecycle state from its own source that is newer than that snapshot;
3. calls the shared pure assignment policy for its current same-sample note-on batch;
4. emits its note and correction immediately;
5. reports the actual assignment and later lifecycle events to the hub.

The hub merges reports by musical timestamp rather than callback arrival order and publishes the next snapshot.
It does not make an attack-time decision for a tuner.

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

Snapshots are immutable after publication.
A tuner retains one reference for its full processing region and never reads the hub's mutable tracker directly.

The default eligibility rule is the latest snapshot committed before the current host processing region.
This supplies the freshest causally complete state without delaying notes.
Issue [#615](https://github.com/yan-h/harmonigraph/issues/615) must measure playing, stopped input, loops, transport resets, variable callbacks and offline rendering.
If callback size makes the musical result unacceptably unstable, the spike may recommend a fixed sample look-behind horizon;
that is a measured amendment to this design rather than a second user-facing mode.

Every source also reports how far it has processed even when it had no events.
Those watermarks let the hub distinguish a silent source from a missing one and state what time a snapshot is complete through.

## Voice identity

Channel and MIDI key are not enough once several tracks contribute.
Two tracks can hold channel 0 / key 60 at the same time, and releasing one must not release the other.

The session address is at least:

```text
SessionVoiceId {
    source_instance_id,
    host_voice_id,
    channel,
    key,
    generation,
}
```

The host voice ID is preserved whenever present.
The Bitwig spike measures whether it survives the intended CLAP chain reliably.
If it is absent, overlapping voices from the same source/channel/key share one adaptive correction and are tracked with multiplicity rather than being assigned speculative identities that later note-offs cannot address.
The picture may fold equal pitches together, but the tracker keeps the count until the last matching voice ends.

The initial active lifetime is note-on through note-off or choke.
Sustain-aware harmonic context is deferred until a real sustain-heavy project demonstrates the need;
the tuner still forwards pedal and unrelated MIDI without changing their timing.

## Pitch output

The first and only output backend is CLAP per-note tuning expression, and the tuner companion is exported only as CLAP initially.
At note-on the tuner emits a `CLAP_NOTE_EXPRESSION_TUNING` value for the same voice and sample position as the note.
CLAP note expressions state the current value rather than adding a delta.
The tuner therefore retains the player's current tuning expression per voice and emits `player expression + frozen adaptive offset` at note-on and after every later player-expression change.
It never forwards a later player value unmodified because that would erase the adaptive offset.

CLAP note expression is a good fit because it is sample-accurate and can address a distinct voice by note ID, port, channel and key.
The Bitwig spike verifies event ordering and the exact instruments used in practice before the feature is considered viable.

MTS-ESP is not a second implementation of the same semantics.
It publishes a global note/channel tuning table that clients query, cannot naturally distinguish simultaneous same-key voices on one channel, does not aggregate note lifecycle, and may move held notes when a client continuously polls an updated table.
MPE adds channel allocation, bend-range and reset state.
VST3 note output adds another host/interoperability matrix.
None belongs in the first implementation.

Keep the musical core expressed in Harmonigraph's exact pitch representation and convert to a CLAP semitone offset only at the plugin boundary.
That is enough separation to add a concrete compatibility backend later without maintaining an unused abstraction today.

## Session, pairing and process boundary

The full Harmonigraph owns the authoritative tuning configuration and one persisted session UUID.
A tuner auto-joins when exactly one compatible hub is active, retains explicit pairing when duplicated and reports missing or ambiguous hubs instead of guessing.
Two open project copies carrying the same saved UUID are ambiguous rather than one combined session.

The first backend is an in-process registry with one bounded single-producer/single-consumer report queue per tuner and immutable shared snapshots.
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
The callback only touches bounded preallocated queues, immutable snapshots and local fixed-capacity state.
It never:

- waits for another plugin instance;
- takes a contended lock;
- allocates;
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

- [`harmonigraph-plugin/src/lib.rs`](../crates/harmonigraph-plugin/src/lib.rs) declares basic MIDI input/output,
forwards host events and currently drops host voice identity while mapping notes;
- [`harmonigraph-core/src/notes.rs`](../crates/harmonigraph-core/src/notes.rs) currently identifies tracked notes by channel and key,
which is insufficient across sources;
- [`harmonigraph-core/src/tuning.rs`](../crates/harmonigraph-core/src/tuning.rs) already supplies the exact pitch representation the policy and emitted-assignment state should retain.

The external documents do not prove actual Bitwig behavior in this plugin chain.
That empirical evidence belongs on #615 as bounded traces and measured verdicts, so an auditor can distinguish specification, design assumption and observed result.

## Implementation order

1. [#615](https://github.com/yan-h/harmonigraph/issues/615) proves Bitwig process grouping, clocks, watermarks, snapshot visibility and CLAP output ordering.
2. [#617](https://github.com/yan-h/harmonigraph/issues/617) implements the lightweight companion, project identity, aggregation, voice identity, actual-output display/take stream and immutable snapshots.
3. [#616](https://github.com/yan-h/harmonigraph/issues/616) adds local optimistic assignment and same-sample CLAP tuning expression.

If the spike rejects a premise, amend this document and #614 before continuing downstream.
