# Project-wide adaptive tuning

## Status

This is the decided design for a planned feature;
none of it is implemented yet.
GitHub issue [#614](https://github.com/yan-h/harmonigraph/issues/614) is the design anchor, with separate children for the Bitwig timing spike, automatic aggregation, pitch output and the first policy.

This document fixes the product and real-time contracts, including the inputs the first musical policy needs.
The policy's scoring constants and musical iteration belong to #621;
the host time mapping remains conditional on #615's measurements.

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
| Attack timing | Immediate optimistic assignment, frozen through release | Synchronized attacks or later reconciliation | Avoids MIDI buffering, live latency, PDC, transition policy and a second failure state machine; Yan has analyzed and accepted cross-track blindness | Yan explicitly revisits the musical requirement; no blindness experiment gates implementation |
| Plugin boundary | Separate lightweight Harmonigraph Tune class exported from the same CLAP bundle as Harmonigraph | Full-plugin instances or one class with persisted Hub/Tuner roles | Keeps the pre-instrument device and its lifecycle small and gives the host a clear note-effect identity; a process-wide registry is shared only inside one dylib, so one bundle is what makes an in-process session possible at all | #615 shows separate classes cannot share a reliable supported process/session topology |
| Snapshot age | Latest healthy snapshot complete through the start of the current coordination region | Fixed sample look-behind horizon | Keeps the accepted optimistic behavior while rejecting incomplete, future or expired state | #615 cannot establish a usable causal time mapping |
| Snapshot sealing | The hub seals causally complete intervals using source progress; tuners read only eligible sealed snapshots | Tuners read one another's live state directly | Eligibility follows the measured region boundary, independent of callback arrival order | #615 cannot establish region boundaries and completeness in the supported graph |
| Snapshot storage | Bounded preallocated slots, copied into tuner-local storage with a Rust-safe publication protocol | Shared reference-counted immutable snapshots | Bounded copies and explicit slot ownership avoid allocation and final reclamation on the audio thread; the actual size and cost are measured in #617 | The measured capacity or copy cost requires a different bounded primitive |
| Report clock | Reports carry their mapped musical sample, epoch, source incarnation and sequence; sources publish processed-through watermarks | Stamp every queued report at the hub's current block plus its local offset | Framework sub-blocks and delayed drains make callback-relative offsets insufficient; #615 establishes the mapping before implementation | Host evidence supports a simpler representation with the same time and reset guarantees |
| Storage layout | In-process preallocated storage with explicit ownership and off-thread reclamation | Pointer-free memory-mappable arena | `rtrb` already uses pointers and shared ownership; real-time safety requires bounded access and lifetime management, not a cross-process ABI | A concrete cross-process transport is authorized |
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
Cross-track blindness is accepted on the basis of Yan's own analysis.
The host spike verifies timestamp correctness and snapshot eligibility, not whether this musical choice should be reopened.

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
Source recovery also needs an explicit state-baseline control record so a held set can be restored without pretending that MIDI attacks were emitted again.
That control reaches the display and take replay as well as the session model.

## Musical state ownership

The hub's audio callback owns a fixed-capacity active-voice model used to seal snapshots.
It consumes the same source-aware lifecycle and pitch reports forwarded to the live display and take recorder.
The existing `NoteTracker` and `NoteRoll` remain downstream display/history consumers:
their `BTreeMap`, `Vec` and bend-history allocations cannot run in the audio callback.
GUI-ring loss or a delayed background drainer must not alter adaptive context.

Effective tuning is also resolved independently of the editor.
Extract the pure comma detection and axis-derivation rules currently in `harmonigraph-ui::begin_frame` into shared musical code;
the audio-owned resolver receives restored settings, host parameters and explicit UI edits through a coherent bounded handoff.
It publishes one configuration revision containing the effective tuning, tempered commas, policy version and musical search bounds.
The UI mirrors that resolved configuration instead of running a competing authority.
A revision becomes eligible with its sealed snapshot, so a policy call cannot mix new axes with old policy settings.
Restoring state and automating tuning must work with the editor never opened.
A configuration change affects future decisions and context interpretation, never the frozen offsets of held voices.

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

1. identifies its coordination region using the time mapping proved by #615;
2. copies the latest eligible healthy snapshot into its own fixed storage;
3. replaces that snapshot's contribution from its own source with its authoritative local held set at the event time, including intervening releases;
4. calls the shared pure assignment policy with its explicit assignment history and current same-sample note-on batch;
5. emits its note and correction immediately;
6. reports the actual assignment and later lifecycle events, then advances its processed-through watermark.

Replacing the local contribution includes absence:
a released local voice must not survive by being copied back from an older snapshot.
Per-source acknowledged report sequences identify what the hub has already incorporated;
there is no unbounded journal of local overlays.

The hub merges reports by mapped sample, preserving each source's event sequence at equal samples, and feeds the resulting stream to its active-voice model, display and take.
A deterministic source order breaks cross-source timestamp ties.
It processes only a bounded amount of queued work and seals an interval only after every included source has reported complete progress through it.
Reports beyond that boundary stay pending;
the snapshot must not contain their future voices.
The hub does not make an attack-time decision for a tuner.

## Time and region contract

A coordination region is a shared musical interval whose boundaries and relation to the hub's clock are established by #615. It is not assumed to be one Rust `Plugin::process()` call.
The installed nice-plug wrapper splits a host CLAP callback on transport events, and can also split on automation when enabled.
Two such sub-blocks may both report offset zero before the hub drains either queue.
Neither a per-call counter nor the hub's current block start can recover the missing sub-block offset.

Reports therefore carry the session epoch, source incarnation, monotonic source sequence and mapped event sample.
The mapping accounts for the enclosing host callback, sub-block start, event offset and any measured track-latency relationship.
The concrete clock source and boundary hook are a required #615 result;
if the framework does not expose enough information, the spike must identify the necessary boundary change or reject that topology.
Do not substitute independent counters that merely happen to start together.

`processed_through_sample` and `committed_through_sample` are exclusive endpoints:
a seal at sample N includes events strictly before N.
A tuner in a region starting at N may use a snapshot sealed through N, but cannot use events at N or later from another source.
This remains true when that snapshot was published before the tuner happens to execute.
Within its region the tuner incorporates its own earlier events.

Every source publishes completed progress even when it has no notes.
A watermark advances only after all reports before it are available to the hub;
queue loss invalidates that progress rather than asserting a complete interval.
Activity counters help detect stopped callbacks but do not replace sample watermarks.
#615 measures live/stopped input, transport discontinuities, block splitting, differing track latency, sleeping tracks and faster-than-real-time export.
It records which reset starts a new epoch and how a returning source acquires a valid mapping before its reports are accepted.
Late reports for a sealed interval are a protocol failure requiring source recovery, not events to restamp at the drain time.

## Snapshot contract

A snapshot is equivalent to:

```text
GlobalSnapshot {
    session_incarnation,
    time_epoch,
    snapshot_sequence,
    committed_through_sample,
    valid_for_region,
    configuration_revision_and_effective_tuning,
    source_incarnations_and_acknowledged_report_sequences,
    active_voices_with_emitted_pitches_and_assignment_metadata,
}
```

A snapshot is a fixed-capacity value published through preallocated slots and copied into tuner-local storage.
#617 selects the concrete publication primitive and documents its ownership and memory-ordering argument before wiring it into the plugins.
Concurrent payload access must be atomic or excluded by slot ownership;
a plain or volatile struct copy racing a writer is not made safe by a seqlock retry afterwards.
Reader acquisition and writer publication are bounded, and slot exhaustion follows the unhealthy-session path.
Any shared allocation retains an off-thread owner until callback users have quiesced;
registration, unregister and final reclamation never free it in an audio callback.

The reader selects the newest complete snapshot eligible for its region, not simply the newest published slot.
The session binds one snapshot sequence to each coordination region before its assignments begin;
healthy tuners in that region use the same binding even if a newer eligible snapshot is published while their callbacks run.
#615 must establish where that binding can be made, and #617 tests a publication between two tuner reads.
Retaining a previous local copy after a failed read is allowed only while its session incarnation, epoch, source health and explicit validity interval still match.
A retained copy must also match the region's bound snapshot sequence.
A reset or expiry makes it unusable immediately.
Enough bounded history must remain available for the processing lead/lag measured by #615;
running out of eligible history passes new notes through unretuned.

Before implementing #617, record named limits for source slots, voices per source and per session, report queues, pending merge storage, snapshot history and retry/work budgets.
Record the resulting snapshot byte size and memory budget, not an assumed size of a few hundred bytes.
Snapshot validity and missing-source thresholds use the measured audio progress/region model, with explicit reset behavior and no wall-clock worker needed for correctness.
Fill in their values from #615 before calling the protocol complete.

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
A tuner auto-joins when exactly one compatible hub is healthy under the measured activity rule, retains explicit pairing when duplicated and reports missing or ambiguous hubs instead of guessing.
Two open project copies carrying the same saved UUID are ambiguous rather than one combined session.
The saved pairing UUID is distinct from the runtime session incarnation and time epoch;
neither a reload nor a transport reset may make an old snapshot valid again.

The first backend is an in-process registry with one bounded single-producer/single-consumer report queue per tuner and the snapshot slots above.
Bitwig's **By manufacturer** hosting mode is the expected initial requirement because it groups plugins from one developer for communication.
Issue [#623](https://github.com/yan-h/harmonigraph/issues/623) measures the exact process layout and the visible failure behavior of other hosting modes.

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
Master placement is not an attack-time barrier;
tuners consume the previous sealed state.
The session belongs to the plugin process rather than the editor, so closing the Harmonigraph window must not stop reports, snapshots or tuning.

## Real-time constraints

Registration, naming and allocation happen away from the audio callback.
The callback only touches bounded preallocated queues, snapshot slots and local fixed-capacity state.
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
- **Off:** remove the source from project context and visualization and leave new notes unretuned, while finishing the expression/lifecycle handling of already-held voices.

Specialized states such as a visible-but-untuned track or a fixed anchor are added only when the musical policy has a concrete use for them.
The single participation control does not remove the need for internal transition states:

| Transition or state | New notes | Already-held voices and session state |
|---|---|---|
| Healthy and participating | Assign from the eligible snapshot and local state | Continue reporting output and composing player expression with each frozen offset |
| Participating to Off | Pass through with zero adaptive offset | Withdraw this source from context/display; keep local voices and continue composing their existing offsets until release |
| Missing, ambiguous, expired or overloaded session | Pass through with zero adaptive offset; no local adaptive fallback | Discard remote context; preserve locally known offsets and lifecycle, and show the exact failure |
| Off to Participating, reconnect or report-loss recovery | Remain unretuned until recovery is acknowledged in an eligible healthy snapshot | Send a complete held-state baseline in a new source incarnation, including voices started unretuned; never retune them or re-emit their attacks |
| Source unregister or slot reuse | No source contribution until registration and recovery complete | Invalidate the old incarnation and release only its context/display voices; reject its queued reports |
| Explicit voice reset or host-guaranteed note termination | Start a fresh local lifetime | Clear offsets only after the corresponding downstream voices are terminated or the host guarantees that they are gone |

A later player-expression event is still emitted as `player value + frozen offset` while Off or disconnected.
Otherwise that event would erase the held voice's correction.
The local table also tracks notes started with zero correction during these states so rejoining restores the actual held set.
Transient policy history is cleared on participation withdrawal, configuration revision change, session/epoch change and lifecycle-loss recovery;
clearing that history never changes a held offset.

Recovery uses a bounded complete baseline with a sequence cut and an acknowledgement, followed by later ordered deltas.
The hub replaces only that source's state when the whole baseline is accepted;
partial baselines are never published as complete context.
It carries the voices' original onset information, current emitted pitches and assignment metadata, and an explicit recovery boundary to the display/take path.
Resetting an empty queue alone is insufficient because a held voice may never send another note-on.
Overflow signals and incarnation invalidation must remain deliverable when the ordinary report queue is full.

Host bypass and plugin removal differ from the Off control because callbacks may stop entirely.
#615 must measure the host's termination/resume behavior.
If lifecycle events were missed, do not republish an assumed local held set on resume:
require a fresh authoritative baseline or terminate/reset the affected downstream voices before clearing local state and rejoining.
The supported behavior and its audible reset consequence must be documented.
No code running in a stopped callback is assumed to repair downstream notes.

There is no hidden local adaptive mode during failure and no pitch correction used as reconciliation.

## Musical policy boundary

Every tuner carries the same versioned pure policy:

```text
assign_new_notes(
    tuning_configuration,
    sealed_global_snapshot,
    newer_local_source_overlay,
    assignment_history,
    same_sample_note_on_batch,
) -> (initial_voice_assignments, history_updates)
```

The infrastructure proof uses an obviously artificial deterministic policy whose result depends on prior state from another source.
It must not choose the eventual just-intonation behavior accidentally.

Assignment history is explicit transient state keyed by source/channel/key and retains the previous selected node after release.
It is capacity-bounded, is not a second held set and is not persisted.
History updates occur only for assignments actually emitted, with the reset rules above.
Empty eligible context uses the origin preference and ignores history from the preceding phrase.

The real policy still has to decide its scoring constants, anchors, root behavior and excluded pitches.
Its cost can be measured before considering a hub-precomputed candidate map;
duplicating a cheap pure calculation in each tuner is simpler and correctly incorporates local state newer than the snapshot.

The first real policy is issue [#621](https://github.com/yan-h/harmonigraph/issues/621), the nearest connected lattice node.
Its candidate search uses bounded musical coordinates around the fixed lattice origin, recorded in the effective policy configuration.
It never reads `ViewConfig` reach, camera centers, drawn windows or display tolerance.
The initial bounds and fixed scoring constants are named policy constants selected and documented in #621 before implementation;
they require no new user control or persisted field.
Candidates lie within 50 cents of the key's equal-tempered class, measured circularly with exact pitch arithmetic, and comma-equivalent nodes are deduplicated after respelling.
An empty candidate set emits zero adaptive offset, retains player expression and clears this key's assignment history.

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
Within a same-sample batch, evaluate distinct note-ons in ascending key then channel order;
earlier assignments in that canonical order contribute to later ones.
Preserve host lifecycle ordering for same-key replacements and preserve the original output event order and sample positions.
Canonical policy evaluation does not authorize moving an off, choke or expression across its addressed voice.

The emitted adaptive offset is the chosen node's pitch class minus the key's equal-tempered class, folded to the nearest octave.
The 50-cent candidate restriction bounds this correction relative to the input key, not the sum with player expression.
Hysteresis encourages continuity but is not an anti-drift guarantee;
#621 records the observed ii–V–I behavior and the bound the implementation actually enforces.
The policy stays pure in `harmonigraph-core` and remains the part to iterate by ear.

## Deliberately outside the design

The following are not launch modes or implied follow-up work:

- synchronized or delayed attack tuning;
- reconciliation or adaptive movement of held voices;
- a central MIDI rack/router;
- a separate headless conductor;
- cross-process session transport;
- MTS-ESP, MPE or compatibility-first pitch outputs;
- raw-intention visualization as a second canonical event stream.

They require a new request and a new issue before implementation.
There is no shadow-measurement or listening gate for cross-track blindness in this plan.

## Evidence and current seams

These sources establish available mechanisms and the constraints that motivated the design:

- [CLAP events](https://github.com/free-audio/clap/blob/main/include/clap/events.h) defines sample-accurate note expressions,
voice addressing and relative tuning in semitones;
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

0. [#623](https://github.com/yan-h/harmonigraph/issues/623) measures which sandbox mode carries two classes from one bundle and which reload gesture works, in the same session as the spike below, since its process-grouping table uses the spike's probe bundle.
1. [#615](https://github.com/yan-h/harmonigraph/issues/615) is a bounded host spike using a second class, fixed +50-cent output and preallocated traces flushed off-thread.
It distinguishes host callbacks from framework sub-blocks and proves the time mapping, region eligibility, progress/expiry rules and lifecycle constraints across the actual live/offline topology.
Unit tests then encode those observations rather than substituting for them.
2. Before wiring plugins in [#617](https://github.com/yan-h/harmonigraph/issues/617), record the concrete publication primitive, capacity/expiry table, configuration handoff and source recovery protocol against #615's verdict.
Implement the companion, audio-owned active state and effective tuning, source-aware display/roll/take/replay, and sealed snapshots with those contracts.
This alone replaces Note Receiver routing, a win before any note is retuned.
3. [#616](https://github.com/yan-h/harmonigraph/issues/616) adds local optimistic assignment and same-sample CLAP tuning expression with the artificial policy.
4. [#621](https://github.com/yan-h/harmonigraph/issues/621) replaces the artificial policy with the first real one.

If the spike rejects a premise, amend this document and #614 before continuing downstream.
