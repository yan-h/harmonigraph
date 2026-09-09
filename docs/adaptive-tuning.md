# Adaptive tuning

Harmonigraph retunes a performance as it is played.
A lightweight **Harmonigraph Tune** note effect sits before each instrument, holds its track's MIDI for a fixed delay, and asks one full Harmonigraph —
the **Hub**, normally on Master —
what pitch each new note should have.
The Hub sees every track's notes in one chronological order, so a chord spread across three tracks is tuned as one chord rather than three independent guesses.
The chosen adaptive correction is composed into a CLAP per-note tuning expression and never moves again for the life of that note;
the player's later per-note expression and MIDI channel bend remain live.

That is the whole feature.
Everything below is what it cost to make that sentence true in a real DAW, and which of the harder-sounding things it deliberately does not do.

## Status

The moving, register-aware policy developed from [Adaptive tuning: a moving harmonic neighbourhood](adaptive-tuning-design.md) shipped in [#782](https://github.com/yan-h/harmonigraph/pull/782).
That design document remains the requirements and discussion record;
the [policy-v2 implementation record](adaptive-tuning-plugin.md) is authoritative for the shipped scorer, controls, expression handling, resources and persistence.
This document owns the surrounding transport, lifecycle and decision history rather than a second copy of those implementation details.

Built and merged in [#709](https://github.com/yan-h/harmonigraph/issues/709), then substantially simplified by the seven-stage [#712](https://github.com/yan-h/harmonigraph/issues/712) stack, which removed the recovery transaction, the shared capture arena, the controller-history replay, the resumable cohort sort, the configuration timeline and the publication repair protocol.
That stack is on `main`.
[#775](https://github.com/yan-h/harmonigraph/pull/775) subsequently integrated follow-up fixes for recorder boundaries and failures, tuning-owner destruction and replacement scheduling;
[#780](https://github.com/yan-h/harmonigraph/pull/780), [#781](https://github.com/yan-h/harmonigraph/pull/781) and [#784](https://github.com/yan-h/harmonigraph/pull/784) then completed the release-test, Reset-supersession and atomic-replacement follow-ups before policy v2 landed.
This document describes that current `main` tree.

It replaces twenty-two staged handoff documents.
Those were written one per implementation slice, each describing the tree at the moment it landed, and by the end most of them described machinery that no longer existed.
The measurements they carried are archived under [`evidence/adaptive-tuning/`](evidence/adaptive-tuning/README.md);
the handoffs themselves are not carried forward.

Yan has since used adaptive tuning in an ordinary Bitwig setup and reports that it works as expected.
That is positive end-to-end evidence for ordinary use, but it is not the exact scripted host acceptance requested by [#632](https://github.com/yan-h/harmonigraph/issues/632) and [#696](https://github.com/yan-h/harmonigraph/issues/696).
There is still no recorded pass of the pending- and sounded-note Stop cases with pedal state and new stopped-live input, or of a fresh unseeded late second phrase on one channel;
those issues therefore remain open.
[The list of what is open](#open-deferred-and-not-built) is at the end.

## Design priorities

This feature is optimized first for one person's maintained macOS/Bitwig workflow, not for format or host coverage.
Correct musical state, an inspectable failure mode and one small execution path outrank compatibility that has no current use.
Add another format, transport, mode or control only when it serves a concrete project and is cheap enough to carry in every later change.
Implementation prioritizes ordinary use:
about 15 notes across three tracks, with a generous growth bound of 16 tracks and 100+ simultaneous notes, and normal Stop, Off, Reset and reopening.

Do not fix a case merely because a probe reaches it.
An uncommon failure may be handled with simple logic —
drop, fault, reset —
rather than recovery machinery, and the limitation gets written down instead.
Preserve existing physical-output truth and working safety behaviour while making that judgment.

Host behaviour is evidence rather than architecture.
Where the design depends on callback order, voice identity or CLAP event delivery, it was measured before it was relied on.

## Decision record

This table preserves the strongest alternative and the condition that can reopen each choice.
A later implementation should not broaden a row merely because its rejected branch is technically possible.

### Product decisions

| Decision | Chosen design | Strongest alternative | Why this won | Reopen only when |
|---|---|---|---|---|
| Attack timing | Central sequencing at a fixed chosen delay, with the adaptive correction and harmonic onset reference frozen from onset through release | Immediate assignment from a prior snapshot | Simultaneous cross-track attacks must share the preceding assignments, and one policy owner avoids distributed assignment and contention; player expression can remain live without rerunning the adaptive choice | A musical requirement needs a correction chosen before its predecessors are known |
| Plugin boundary | A separate lightweight Harmonigraph Tune class, exported from the same CLAP bundle as Harmonigraph | Full-plugin instances, or one class with persisted Hub/Tune roles | Keeps the pre-instrument device and its lifecycle small and gives the host a clear note-effect identity; a process-wide registry is shared only inside one dylib, so one bundle is what makes an in-process session possible at all | A supported hosting mode stops sharing one process between two classes of one bundle |
| Input completeness | The Hub orders complete source intervals, using each source's published coverage rather than callback arrival | Assign in callback arrival order | A later source callback may carry an earlier musical event, so only complete intervals establish chronological order | Coverage stops being establishable in a supported graph |
| Assignment order | Chronological, then deterministic sequential assignment of simultaneous attacks | Joint chord optimization | Each new assignment sees the earlier ones without selecting a globally optimal chord or moving held voices | A concrete musical requirement demands a different policy |
| Report clock | Reports carry a mapped sample, the enclosing callback, the sub-block, the session epoch, the source incarnation and a sequence | Stamp every queued report at the Hub's current block plus its local offset | The wrapper splits a host callback on transport and automation, so two sub-blocks can both report offset zero before the Hub drains either queue; neither a per-call counter nor the Hub's block start recovers the missing offset | Host evidence supports a simpler representation with the same time and reset guarantees |
| Storage layout | In-process preallocated storage with explicit ownership and off-thread reclamation | A pointer-free memory-mappable arena | `rtrb` already uses pointers and shared ownership; real-time safety wants bounded access and lifetime management, not a cross-process ABI | A concrete cross-process transport is authorized |
| Voice identity | Source, channel and key, with the host note id passed through untouched | The host voice id as identity, with a multiplicity fallback for its absence | Tune advertises no overlapping-note support, so a host must not overlap one key and channel; a retrigger replaces, which is the rule the tracker already applies | Bitwig is observed delivering overlapping same-key notes to a note effect that does not advertise them |
| State authority | Actual emitted output, with pending assignments kept separately for scheduling | Treat every planned attack as already emitted | Later decisions need scheduled predecessors, while display, take and reset must distinguish a plan from actual output | A real downstream pitch-feedback mechanism exists and is worth integrating |
| Pitch output | CLAP per-note tuning expression for the adaptive correction, composed with live player expression; MIDI channel bend remains MIDI | MTS-ESP, MPE or VST3 note output | Matches sample-timed per-voice assignments and the actual host while adding no external tuning service; Bitwig converts a note effect's per-note pitch to MPE or VST3 note expression downstream, so the instrument's own format is not restricted | A required instrument cannot consume it |
| Session transport | An in-process registry under a documented Bitwig hosting mode | Cross-process shared memory | Avoids process discovery, crash recovery and stale shared state for compatibility nothing currently needs | A concrete workflow requires another hosting mode |
| Hub ownership | Full Harmonigraph, normally on Master | A headless conductor, or an elected Tune peer | Reuses the existing configuration, display, take and combined-audio location without another authority or plugin role | The Hub cannot stay active in the supported graph, or a project needs tuning without a full Harmonigraph |
| Missed assignment deadline | Keep that one attack pending, take its valid late assignment, and show the miss | Drop it, or emit it unretuned at the deadline | Extra latency on one note beats a missing or wrongly tuned note, and the visible counter is what makes raising the delay a deliberate act rather than a guess | Lateness alone never reopens this; actual storage exhaustion is the separate latched fault |
| Participation UI | One Participating/Off control per Tune | Independent visibility, context and retune switches | Minimizes persisted states, combinations and tests until anchors or monitor-only tracks have a concrete musical contract | A real project requires a specific excluded combination |
| Player pitch | Preserve the player's MIDI bend and per-note pitch expression in both modes; Participating adds one frozen adaptive correction | Participating centres bend and replaces expression, while Off alone passes them through | Player gestures remain expressive, and policy v2 can score their absolute onset pitch without changing its frozen adaptive choice later | A required gesture cannot be represented by this composition |
| Policy location | One pure sequential policy in the Hub | Distributed evaluation of shared state by each Tune | One owner has the complete ordered input and every preceding assignment; Tunes buffer and apply answers | The measured Hub work budget cannot support the chosen policy |

### Mechanism decisions, and what each replaced

The "strongest alternative" column below is not hypothetical.
Every one of these was built, shipped, and then removed —
which is why these rows exist at all:
a design whose rejected branch was implemented first knows exactly what it cost.

| Decision | Chosen design | Built, then removed | Why this won | Reopen only when |
|---|---|---|---|---|
| The delay | A per-Tune **Tuning delay** parameter: 1 to 16 multiples of the host's advertised maximum callback size, resolved at activation, fixed for that activation and reported as latency | One fixed session-wide delay of 512 samples | A sample count is either short at one buffer size or needless latency at another; a multiple of the host's own callback is the quantity the round trip actually has to fit inside, and 16× reaches the old 512 even from a 32-frame callback | Automatic latency adaptation is requested — it explicitly is not, today |
| Tune→Hub transport | Self-contained copied records through bounded preallocated queues; the Hub owns what it receives and Tune owns its pending state | A shared `CaptureArena` with `unsafe impl Sync`, `Token`/`Key`/`Permissions`, a split between original and local storage, and a storage-retirement handshake | Each record is fixed-size and its storage preallocated, so copying costs no allocation and no extra delay — and it deletes an entire ownership protocol whose only purpose was avoiding the copy | Copying is measured to cost callback budget the assignment cannot afford |
| Same-sample ordering | Merge the per-source streams by sample; within one sample apply every release and controller from every source, then assign onsets in key, channel, source order | A 15-phase incremental topological sort over a 1,280-vertex adjacency bitset, built by an O(n²) pairwise scan and budgeted across callbacks | Across sources the dependency graph is empty by construction, and within one source the host already delivers events in dependency order, which copying preserves. There was nothing for a graph to discover | A dependency exists that this order does not already respect |
| Configuration granularity | One resolved configuration adopted per enclosing Hub callback, captured when a group's assignment starts and held until that group completes | A 128-marker `ConfigTimeline` with a `ControlBudget`, and a pass-routing `Recording` of 2,178 segments (~198 KB live on the audio thread) answering which tuning was in force at an exact sample | An edit waits at most one host block — about 10.7 ms at 512/48 kHz — and nobody has to ask the sample-exact question at all | Sample-exact configuration automation becomes a musical requirement |
| Divergence and faults | No retrospective reassignment. A resource or host-output fault is a latched shared reset: cancel pending attacks, release what was forwarded, reject obsolete replies, then refuse new attacks until an explicit Reset | An 8-phase recovery transaction (Prepare→Inventory→Preserve→Status→Rebuild→Replay→Resume) with its own protocol messages, ~20 `recovering()` branches and three duplicated ordinary/replay code pairs | Its documented outcome when it fired was a note sounding 467 ms late, and its other responsibility — terminal faults — is exactly what the latch does | A measured ordinary case needs a recovered phrase rather than a reset one |
| Controller context for a late note | A late note meets the receiver's current controller state; shared channel controls keep their own input-plus-delay schedule | A channel-wave mechanism replaying 130 folded controller registers plus raw prelude history into the receiver ahead of a younger onset | Different articulation during a late note is accepted; reconstructing the controller context a note "should have" met is a replay, and replay is the thing this design excludes | A measured ordinary case is musically wrong because of it |
| Publication loss | The lane reports its own loss into a reserved cell; the display clears stale state and refreshes from a current snapshot; an incomplete take exports with a warning | Per-source baselines with generation handles, an `UnsafeCell` bank behind a busy state machine, cross-thread resync requests and per-source repair cursors | A snapshot restores what is sounding now, which is the whole of what a display needs after a gap; the repair protocol was reconstructing history the display had already drawn | Reconstructing lost history becomes worth its own protocol |
| Membership and allocation | A Tune's row, queues and plan slots are allocated at pairing, on the main thread; adding, removing or re-pairing a Tune is a reset boundary | Sixteen rows preallocated in every Harmonigraph's constructor, with live hot-plug reconciliation through `attach`/`detach`/`seal`/`Adoption` | An unpaired Harmonigraph fell from 45,348,144 to 7,313,600 retained bytes, and seamless live reconciliation was paying for a case that a reset covers | Seamless re-pairing during a phrase becomes a requirement |

## The shape

One full Harmonigraph is the project's state hub, combined note display and take recorder.
A **Harmonigraph Tune** sits before each independently routed instrument path that participates:

```text
Track A: notes -> Harmonigraph Tune -> instrument A -> Master
Track B: notes -> Harmonigraph Tune -> instrument B -> Master
Track C: notes -> Harmonigraph Tune -> instrument C -> Master
Master:  Harmonigraph hub and analyzer
```

One Tune can sit before a Bitwig Instrument Layer and cover the instruments inside it.
Independent tracks need independent Tune instances, because each note stream has to change before its own instrument consumes it.

Tunes talk to the Hub internally.
They create no host MIDI routes, and each still sends ordinary note output only down its own track;
their reports replace the manual Note Receiver routing that displaying several tracks together used to need.

Both classes come out of the one existing bundle, `nice_export_clap!(Harmonigraph, HarmonigraphTune)`, because a process-wide registry is shared only inside one dylib.
The loader scripts see the same bundle they always saw.

## The path of one note

```text
notes -> Tune buffer -> assigned, delayed notes -> instrument
           |    ^                  |
  captures |    | assignments      | actual output
           v    |                  v
         Harmonigraph Hub: ordered sequencer + confirmed state
                                   |
                                   v
                      combined display and take
```

For each processing interval a Tune:

1. maps incoming events onto the shared clock, through the enclosing callback, the sub-block start and the event offset;
2. retains their performance data in fixed-capacity pending storage;
3. **copies** each one into a self-contained `Capture` record and sends it, so the Hub never reads Tune's storage;
4. advances its coverage watermark only once every preceding record is available to the Hub;
5. consumes replies addressed to its exact pending requests, checking incarnation, epoch and reset generation;
6. emits ready events at input time plus the delay, or holds an unresolved attack and counts a missed deadline;
7. reports actual output times, pitches and lifecycle events, and advances its output progress separately.

The Hub merges complete input intervals by mapped sample rather than callback arrival.
It applies releases and controllers at their proper times, assigns each onset once, and folds that assignment into the prospective context before assigning the next.
It replies to the originating Tune, consumes actual emission reports for its confirmed state, and publishes those to the display and the take.
Tunes run no assignment policy and hold no copy of the Hub's state.

Work is bounded per callback.
An incomplete interval simply stays pending;
no callback ever waits for another instance, takes a contended lock, allocates, frees, or does I/O.
Offline rendering runs the same sample-timed state machine and cannot depend on a worker keeping up.

## The delay

The **Tuning delay** parameter is a stepped integer, 1 to 16, defaulting to 1. One step is the host's advertised **maximum** callback size at activation —
not its minimum, not the current callback length, not an observed typical size —
so

```text
D_samples = multiplier × advertised_max_frames
```

and that number is fixed for the whole activation.
Each Tune persists its own multiplier as an ordinary plugin parameter and reports the resulting latency from it at activation, before any Hub pairing exists.
The Hub's Session menu offers "apply to all paired Tunes" as a convenience, and each Tune that changes then requests its own reactivation.

The editor shows the multiplier, the sample count and the milliseconds together —
`2x buffer - 128 samples / 2.67 ms` —
and, while a change is pending, shows the active value beside the requested one, so the number on screen is never one the engine is not using.
A slider adjustment applies when the gesture finishes rather than restarting on every intermediate value.
A session reset and an audible interruption during a slider adjustment or a host format change are accepted.

The delay applies to the complete performance stream —
note-offs, choke, pedals, expression and unrelated MIDI —
so healthy operation preserves durations and gestures, and it is the same in both participation modes.
This is not host bypass.
More delay adds live-playing latency;
the host's own compensation can align scheduled playback but cannot anticipate a live key press.
No automatic latency adaptation is offered.

### Why a fixed sample count was the wrong knob

The delay used to be a fixed 512 samples, and the argument against that is arithmetic rather than taste.

Take raw callbacks `[0,512)` and `[512,1024)`, with source B carrying a +64 routing offset and A and C carrying none.
B's raw event 511 maps to 575, while A and C have only proved exclusive progress to 512, so the Hub must wait for their next interval.
If B then drains replies inside its own `[512,1024)` callback, before those intervals and before the Hub's work, the earliest B callback that can see the reply starts at raw 1024 —
and the physical deadline was `511 + 512 = 1023`.
The attack is at least one sample late, and raw offsets 448 to 511 are late by 64 down to 1 under the same legal callback order.
Adding the calibration offset to the physical delay would only conceal it by changing the promised D.

There is no fixed sample count that is right at every buffer size:
512 samples is four callbacks at 128 frames and half a callback at 1,024. What the round trip actually has to fit inside is *some number of host callbacks*, which is what the multiplier expresses.
The measured Bitwig spike ran at D = 2,048 and did not search for a minimum, so the right shape was a knob with a visible miss counter beside it rather than a constant somebody had to be right about in advance.

### Deadline feedback

A missed deadline shows as a count and a worst lateness —
`3 notes missed their deadline · worst lateness 0.8 ms` —
cleared when a new setting is applied, so raising the multiplier is measured rather than guessed.

Three states are *not* a missed deadline, and the status names each rather than counting it as evidence that the delay is too small:
a session still starting up, a missing or ambiguous Hub pairing, and a latched output or resource fault.
A larger delay is no remedy for a missing Hub.
A default of 1× is a starting point, not a reliability guarantee;
validate the multiplier in the actual project.

## Same-sample ordering

Within one source, the host delivers same-sample events already in dependency order, and copying preserves that order.
Across sources, nothing depends on anything except onsets on onsets.

So the pass is a merge of the per-source streams by sample, and within one sample, two passes:
apply every release and controller from every source first, then assign onsets in the key, channel, source tie-break the musical decision has always used.
It runs to completion within one Hub callback, on the copied records.
There is no adjacency matrix, phase machine, resumable cursor or graph of any kind.

The sort key is a tuple —
`(is_onset, key, channel, source, serial)` —
and that is the entire ordering algorithm.

## Late playback: two rules

An event addressed to a note that has not yet sounded waits with that note.
Everything else emits on its own schedule.
A replacement onset on the same channel and key force-releases its predecessor at the moment of emission.

Those two rules are the whole per-note shift contract;
there are no further ordering exceptions.

What follows from them is worth stating, because the previous design promised the opposite.
A blocked attack does not hold up unrelated ready notes and releases, including on its own Tune.
There is no track-wide accumulated delay and no catching up in a burst.
A late note keeps its own duration, because its onset lateness is carried onto its own subsequent release and expression and onto nothing else.
A same-key retrigger cannot be terminated by its predecessor's obsolete release.
And a late note meets the controller state the receiver is actually in, rather than one reconstructed for the deadline it missed —
different articulation and temporary timing mismatches during late playback are accepted.

Ordinary lateness is never authorization to cancel.
Explicit Stop and Reset, and actual resource or output failures, are separate cases.

## Configuration

Effective tuning is resolved independently of the editor.
The `ConfigReducer` owns the semantics and ordering of combined edits, presets, explicit unlocks and mode changes —
a preset must not become a mixture of old locks and new axes —
and it publishes one resolved configuration containing the effective tuning, tempered commas and policy-v2 controls.
Those controls cover neighbourhood radius and axes, harmonic, pitch and register weighting, released memory and recency, repetition tolerance, silence timeout and transport resets;
their exact ranges and defaults live in the [implementation record](adaptive-tuning-plugin.md#controls-and-live-neighbourhood).
The UI mirrors that resolved configuration rather than running a competing authority, and restoring state or automating tuning works with the editor never opened.

Adoption is **block-level**.
At the start of each enclosing Hub audio callback, pending edits are adopted coherently, and one resolved configuration serves every assignment group started in that block.
An edit encountered during a block becomes effective at the next block boundary;
parameter-driven sub-blocks do not create extra boundaries.

A simultaneous-note group keeps the configuration captured when its assignment started until that group completes, even across a callback boundary, and every track's notes in that group share it.
An unassigned note uses the configuration current when the Hub *starts assigning its group* —
never one reconstructed from the note's original input sample.
Once assigned, the adaptive correction and harmonic onset reference stay fixed through later edits and delayed emission.
Already sounding notes keep that choice while later player expression and MIDI channel bend can still move their emitted pitch.
Note timestamps retain sample precision;
only configuration application is block-granular.

Auto and Learn results become effective at block boundaries too, without reducing musical observation to the notes held at a block end.
Display and recording use the configuration actually adopted, and take replay reuses the recorded pitches, times and configurations instead of re-running inference.

Ordinary tuning edits do not reset the plugin or change its reported latency.
An edit can wait about one host block, notes near an automation transition may get the old configuration, and changing the DAW's block size can change assignment results near configuration automation.
Replay of a recorded take still follows the recorded results.

## Participation: Participating and Off

Each Tune has one control.

- **Participating** — new notes get adaptive assignments, the track contributes to every other track's tuning context, its notes appear in Harmonigraph, and the frozen adaptive correction is composed with the player's per-note expression and MIDI channel bend.
- **Off** — the track is excluded from adaptive context and visualization, and its notes are forwarded without adaptive correction, at the same delay, with the player's own MIDI bend and per-note pitch expression passed through.

**Off is a local forwarding path.** An Off onset asks the Hub for no assignment, so it waits for no reply, and the Hub mints no plan for it:
it emits at its own input plus D whatever the Hub is doing, spends no decision, holds no plan-ledger slot and takes no cell in the policy's context.
Those two halves are one decision rather than two.
An onset that emits without an assignment reports decision zero, which matches no plan, so a Hub that had minted one would never retire that slot —
one leaked lifetime per Off note, ending in a storage fault as soon as the Tune hands the same request slot back.
For the same reason a cancelled Off attack reports no cancellation.
What Off keeps is what the reset needs:
bounded local storage, note and release tracking, truthful output-acceptance handling, and the control coordination that withdraws the source and rejects obsolete work.

**Off leaves the display, the take and learning too**, and those are three separate consumers of one exclusion, each gated where that consumer reads.
The Hub refuses an Off record before it builds a request identity, so nothing enters the policy's context.
`Hub::confirm` declines to contribute an Off row's pitches, so nothing enters what Auto and Learn infer from.
The accepted-output lane skips the note delta, so nothing reaches the display ring or the take.
That last gate reads the **row's** mode rather than the note's, because the row's mode is already what a published baseline carries downstream, and that flag is what hides the source in `NoteTracker` —
a per-note gate would publish deltas for a source the display has already hidden.
It is also where the deferred "report Off notes for display only" option would be one flag rather than a change of shape.
Baselines keep flowing in both modes, and they have to:
a baseline is how the display learns that a track went Off, and it carries that track's truthful voices while hiding them.

**Either toggle direction resets that Tune.** The toggle stands as a boundary marker in the Tune's own input queue, at the sample it arrived on, and the reset runs where output reaches it:
cancel the attacks still standing before the marker and reject their obsolete replies, then send the releases and pedal cleanup needed to terminate what has already been forwarded *before* that ownership is forgotten.
Player pitch survives the boundary;
returning to Participating does not recenter MIDI channel bend, and subsequent attacks include its current displacement in policy-v2 selection.
The copied participation record tells the Hub, which drops that source's obsolete context and sequences later input against the new membership.

Audible interruption of that Tune's phrase is accepted.
Notes started in the old mode are not carried into the new one, instrument release tails may remain, and smooth sustain or bend continuity across the toggle is not required.
Other tracks' issued assignments are not recomputed because this Tune withdrew.

**A missed deadline is not a participation toggle** and is not authorization to cancel.

## Resets and faults

There is one reset primitive, and every path calls it:
cancel unsounded attacks at the cut, arm the release debt for what was actually forwarded, and leave downstream ownership held until the release is accepted or a host termination boundary settles it.
Resetting internal tables alone must never strand voices in the receiving instrument.

| Trigger | Scope | Restart |
|---|---|---|
| Transport Stop or host Reset | Cancel pending attacks and release forwarded voices without the retained stream delay. Host Reset clears musical context; Stop clears released memory and moving displacement only when **Reset context on stop** is enabled | Resumes by itself; being stopped is not a repeated reset, and a loop/seek has its own optional policy reset |
| Participation toggle, either direction | The above at the toggle's own sample, preserving player pitch while changing adaptive membership | The next note is already in the new mode |
| Host reactivation, or a latency or format change | The above, then recompute D from the saved multiplier and the new activation format | Immediate; lease, epoch and generation deliberately survive |
| Membership change — adding, removing or re-pairing a Tune | The above, resolved at a controlled boundary on the main thread; old replies cannot revive notes in the new session | At the next adoption |
| Explicit setup Reset | The above plus a fresh clock | Arms the un-latch |
| **Terminal fault** — storage, output, clock, input or reference | The same body, plus a latch: every affected emission gate closes and new attacks are refused | **Only** an explicit Reset with a valid fresh boundary |

The musical-context controls are independent of physical note cleanup.
A zero silence timeout never clears context;
a positive timeout clears released memory and displacement only after no held context remains and the completed input frontier reaches it.
The optional loop/seek reset detects a greater-than-two-millisecond discontinuity between host seconds and elapsed sample time, then clears context once at the first subsequent attack while held notes keep their frozen corrections.

A terminal fault is not a recovery pass.
It is the ordinary reset plus a latch, which is the whole of what the deleted recovery modules were still needed for.
Accepted output remains factual across cancellation, while policy-v2 memory and displacement follow the selected reset controls.
Release and pedal debt stays independently owned until acceptance.

Ordinary callback-order misses, delayed assignments, ring backpressure, routine missing callbacks, healthy configuration or Off changes, and display-only loss with intact audio-owned facts do **not** establish a fault.
Neither elapsed time nor workload size alone authorizes cancellation.

Host bypass and plugin removal differ from the Off control, because callbacks may stop entirely.
If lifecycle events were missed, do not republish an assumed local held set on resume:
require a fresh authoritative baseline, or terminate the affected downstream voices before clearing local state and rejoining.
No code running in a stopped callback is assumed to repair downstream notes.

## Voice identity

Channel and MIDI key are not enough once several tracks contribute:
two tracks can hold channel 0 / key 60 at once, and releasing one must not release the other.

```text
SessionVoiceId { source_instance_id, channel, key }
```

That is complete by contract rather than by luck.
Tune leaves its poly-modulation config unset, so it never advertises the CLAP voice-info extension, and a host must then not send overlapping note-ons for one key and channel.
A retrigger that arrives anyway replaces the held voice, which is the rule the tracker already applies.
The host's note id is passed through on every emitted event and the tuning expression is addressed with it, but it is not part of the identity.

The source id reaches every identity-bearing path:
core events, the active-voice model, tracker keys and held-end keys, `NoteRoll` live keys and bend/release lookup, take records and offline replay.
The Hub's own direct input has a reserved identity so it cannot collide with a Tune.
A source reset releases only that source;
a session reset has explicit all-source scope.
Each reusable source slot carries an incarnation, so queued reports from an earlier occupant cannot affect its replacement.

The initial active lifetime is note-on through note-off or choke.
Sustain-aware harmonic context is deferred until a real sustain-heavy project demonstrates the need;
Tune still forwards pedals and unrelated MIDI at the same delay, and pedal sustain or instrument release tails do not turn a released policy contribution back into held context.

## Pitch output

The only output backend is the CLAP per-note tuning expression, and Tune is exported only as CLAP.
At note-on, Tune emits a `CLAP_NOTE_EXPRESSION_TUNING` value for the same voice and sample position as the note.
CLAP note expressions state the current value rather than adding a delta.

While Participating, Tune freezes only the adaptive correction and the tuned onset used as harmonic context.
The policy scores the note's absolute attack pitch, including register, attack expression and current MIDI channel displacement.
Later per-note expression is added to that frozen correction, and raw MIDI channel bend is forwarded at its original sample position without being counted into the correction twice.
RPN 0 pitch-bend sensitivity is followed, with a two-semitone default.
Velocity, pressure and other non-pitch controls keep their normal behaviour.
Off uses the same player-pitch forwarding without adding an adaptive correction.

Actual emitted pitch and the tuned onset used by the policy are separate facts.
Channel-bend changes update the factual voices and request display/take baselines without pretending that the original MIDI message was a separately accepted per-note event.
The live output and final factual pitch are correct, but [#783](https://github.com/yan-h/harmonigraph/issues/783) records that multiple bends within one callback do not yet produce a separate sample-timed per-voice trajectory for every intermediate value in the display or take.

CLAP note expression fits because it is sample-accurate and addresses a distinct voice by note id, port, channel and key.
Bitwig converts a note effect's per-note pitch into MPE or VST3 note expression for whatever instrument sits downstream, so CLAP-only output does not restrict the instrument's format.

MTS-ESP is not a second implementation of the same semantics:
it publishes a global note/channel tuning table that clients query, cannot naturally distinguish simultaneous same-key voices on one channel, does not aggregate note lifecycle, and may move held notes when a client polls an updated table.
MPE adds channel allocation, bend-range and reset state;
VST3 note output adds another host and interoperability matrix.
None belongs here.

The musical core returns an adaptive correction in Harmonigraph's exact microcent representation;
the plugin boundary composes it with player expression and converts the result to a CLAP semitone value.
That is enough separation to add a concrete backend later without maintaining an unused abstraction today.

## Session, pairing and process boundary

The full Harmonigraph owns the authoritative tuning configuration and one persisted session UUID.
A Tune auto-joins when exactly one compatible Hub is healthy, retains explicit pairing when duplicated, and reports missing or ambiguous Hubs instead of guessing.
Two open project copies carrying the same saved UUID are ambiguous, not one combined session.
The saved pairing UUID is distinct from the runtime session incarnation and time epoch;
neither a reload nor a transport reset makes an old capture or reply valid again.

Bitwig's **by Vendor** hosting mode is the measured requirement, with individual hosting overrides off for both Harmonigraph classes.
The backend is an in-process registry with bounded per-source queues.

Each instance takes sample rate and maximum buffer size from host activation;
they are read-only runtime values, not saved routing configuration, and a reloaded instance uses the new host format automatically.
Setup starts automatically at signed offset zero, using the host's actual format and continuous steady time —
there is no manual validation checkbox.
If a branch has a known timing offset, set that advanced offset explicitly and apply it after routing-latency changes.
Automatic measurement of graph or delay-compensation latency is not claimed.
Continuous callbacks are a supported-configuration requirement:
missing progress is not silent input, and unobserved sleep/wake behaviour is not accepted as safe recovery.

The hosting mode also decides what a build swap costs.
A sandbox process re-reads the plugin binary only when it starts, and a grouped process lives as long as any instance in its group is loaded, so a project with a Tune on every track holds the old image until every one of them is unloaded.
Bitwig's plug-in settings also carry a per-plug-in list that runs a named plug-in Individually under any global mode, which is how another plug-in of the same vendor is kept out of the session's process without renaming anything.

The Hub normally sits on Master because it is downstream of the participating audio and can analyze their combined signal.
The session belongs to the plugin process rather than the editor, so closing the Harmonigraph window must not stop progress, sequencing or tuning.

Cross-process shared memory is not part of the design.
It adds process discovery, stale participants, crash recovery and system-level synchronization without improving the intended workflow.

## The musical policy

The Hub runs the pure, bounded policy-v2 scorer once for each ordered onset and folds each selected predecessor into the next decision, including across sources at the same sample.
The [policy-v2 implementation record](adaptive-tuning-plugin.md) owns the exact score, controls, reset rules, limits and persistence shape;
the summary here records only the system boundary.

Input is absolute pitch, including register, per-note attack expression and the current MIDI channel displacement.
The previous onset's full output-minus-input correction is the moving reference and is not octave-wrapped, so repeated material can keep travelling through the lattice instead of being pulled back to one fixed origin domain.

Candidates are the union of bounded local Manhattan neighbourhoods around the contributing context, restricted by the selected axes.
The scorer chooses the nearest octave realization by pitch error plus register-weighted harmonic distance, with coordinate order as the final tie-break.
Held references have full weight;
recently released onset pitches contribute through configurable weight, recency and absolute-pitch repetition tolerance.
New activity replaces the bounded released memory;
elapsed wall time alone does not decay it.

The adaptive correction and tuned-onset harmonic reference remain frozen for the voice lifetime.
Later player expression and channel bend change emitted and factual pitch without rerunning the adaptive choice.
Departing or excluded sources leave no stale released context.
Silence, Stop and loop/seek resets follow the controls described in the implementation record, while an explicit Reset clears musical context through the existing reset boundary.

Selection uses preallocated scratch and refuses resource exhaustion instead of scoring a truncated candidate set.
The policy never reads camera reach or display tolerance.
The live lattice's C2–C7 winner outlines are an off-audio-thread view of the Hub's authoritative next-attack context, not an input to the score and not part of preview or export pictures.

## Capacities and memory

These are selected ceilings sized for 16 tracks and 100+ simultaneous notes, not measured throughput.
Audio transport and policy-selection storage is preallocated;
the more expensive live-neighbourhood calculation runs off the audio thread.
Exhaustion is an explicit bounded failure, and shrinking a buffer does not make its overflow path unreachable.

| Constant | Value | Resource |
|---|---:|---|
| `TUNERS` | 16 | Tuner leases per session; the Hub's own direct input has a reserved row |
| `HELD_PER_SOURCE` / `HELD_SESSION` | 64 / 256 | Held voices per source / across the session, including Off voices |
| `PENDING_EVENTS` | 8,192 | Retained performance events per Tune, including unrelated MIDI |
| `LIFETIMES` | 8,192 | Request records per Tune. A slot is held per in-flight **input**, not per sounding note |
| `INTENT_RING` / `REPLY_RING` / `OUTPUT_RING` | 1,024 / 1,024 / 2,048 | The three rtrb rings per paired Tune |
| `OUTCOME_JOURNAL` | 4,096 | Local accepted-output records per Tune |
| `CAPTURES_PER_SOURCE` / `ROW_OUTPUT` | 1,024 / 2,048 | Hub-side staging per paired row |
| `BATCH_EVENTS` | 2,048 | One ordering pass's copied records |
| `MAX_COHORT_ONSETS` | 256 | Onsets assigned within one sample |
| `MAX_CONTEXT` / `MAX_MEMORY` / `MAX_CANDIDATES` | 256 / 24 / 4,096 | Held policy references, released contributions and the complete local candidate union |
| Plan ledger | `LIFETIMES` per **paired** row | Allocated at pairing, not in the constructor |
| `PUBLICATION_RING` / `SNAPSHOT_SLOTS` / `GAP_RESERVE` | 4,096 / 20 / 1 | Publication items, snapshot frames, and the one cell an outage may always spend |
| `DELAY_MULTIPLIER_MAX` | 16 | Steps on the Tuning delay parameter |

A 17th Tune is refused visibly rather than evicting one of the first 16. All 16 sources may use up to 64 local voice cells each, but cannot simultaneously reserve 16 × 64 session voices.
Policy coordinates use signed 32-bit lattice coordinates and corrections use signed 64-bit microcents, so an individual decision is bounded without imposing the old small-coordinate or approximately 2,147-cent limit on a musical journey.

Memory is dominated by what is allocated **when**.
A Harmonigraph with no paired Tune still owns its policy scratch, but allocates no per-Tune plan ledger, row queue backing or ring triple;
those are built in the registry's re-match, on the main thread, at pairing.
That took an unpaired instance from 45,348,144 to 7,313,600 retained bytes.
Audio-thread allocation is excluded, and the vendored framework's allocation guard asserts it around the real CLAP process callback.

The historical byte-by-byte ledger, including its own drift, is in [the evidence archive](evidence/adaptive-tuning/README.md).

## Display, the take and export

The Hub publishes actual emitted assignments and player expression —
not a recomputed ideal chord —
to the live display and the take recorder, with current factual voice baselines after channel-pitch changes.
This is not a measurement of resulting acoustic pitch;
a receiving instrument may ignore, smooth or modulate the expression.
Within Harmonigraph's observable protocol, accepted output and its resulting factual pitch are nevertheless the only honest authorities.
[#783](https://github.com/yan-h/harmonigraph/issues/783) is the narrower exception:
a baseline preserves the final channel-bent pitch but not every intermediate per-voice trajectory within one callback.

Publication is two independent lanes, one for the display and one for the take, and the rule that keeps them independent is stated as a type:
**no capacity, outcome or cursor on the publication path is a value derived from both lanes.** A full display ring must not hold back a snapshot the take needs, and a take-lane gap must not blank the display.
The display does not hop through the writer thread.

There is no acknowledgement, generation or resync protocol between the ends —
one FIFO of items and one FIFO of snapshot frames.
Loss is reported, never repaired:
the lane that lost a report pushes a gap into a cell reserved for exactly that, so an outage stays readable even if audio never calls again.
The consumer then clears that source's stale held state and refreshes from a current snapshot.
A snapshot restores **what is sounding now**;
it does not reconstruct missing attacks, releases or trajectories, and the test that pins it asserts no note-on appears among the refresh records.
A gap clears every source, so one lost report owes every source a snapshot.

Takes are format v4;
v1–v3 are refused whole with a version error rather than half-read.
Every record carries a container-level `#[serde(default)]` and none denies unknown fields, which is what makes dropping a field safe without touching the format version —
and is why a take written today is refused by a pre-prune build rather than misread by one.
Policy-v2 settings are saved in musical settings and take configuration metadata, and tuned onset pitch is a saved voice fact.
Old policy-descriptor fields were removed without aliases or migration shims;
missing v2 fields use sanitized baseline defaults.
Replay uses recorded output pitches and never reruns adaptive selection.

**A take that parsed is rendered;
a take that did not parse is refused.** Missing note history is a warning, not a refusal:
the renderer prints it on stderr before anything the command line can complain about, and the Video pane's status line carries it onto the render it succeeded on.
Surviving data is drawn with its gaps;
nothing invents missing notes or durations.
The warning's lifetime is the recording rather than the file, so a hole that outlived the pass it happened in still marks the pass that exports.
Conservatively marking an unaffected take is an accepted residual;
failing to mark an affected one is not.

## The framework boundary

The vendored `nice-plug` wrapper supplies what the plugin cannot get for itself:

- **Provenance.** Every input carries absolute sample, enclosing callback start and length, sub-block, flush and batch flags — because the wrapper splits a host callback on transport and automation, and two sub-blocks can both report offset zero.
- **Steady time**, which nice-plug-core's `Transport` did not expose.
- **Lossless CLAP identity**, since `NoteEvent::PolyTuning` carries `f32` and drops the note id, flags and `f64`.
- **Per-event host-acceptance feedback**, because `send_event` throws the result away and a plugin that owns downstream note ownership cannot infer acceptance.

Removed from it during the simplification:
the retain-and-acknowledge input contract (`Consumption::Pending`), the legacy input walker, and a second complete notification emitter reachable only from fixtures.
Kept, against an earlier plan to remove them:
the three input cursors, which turned out to be one progress mark per consumer over a single shared pool rather than the retain machinery —
giving each consumer its own copy would add cost and a mechanism instead of removing one.

**The broader group/lane/heap output-scheduler simplification remains deferred.** `Group`, `Output`, `Lane`, `Token` and `Completion` were listed for removal, but measurement showed that some carry load-bearing ownership and acceptance semantics.
[#784](https://github.com/yan-h/harmonigraph/pull/784) strengthened the existing scheduler instead:
a same-key predecessor release, replacement note-on and optional initial tuning now form one compact sequence with one preparation and completion, and every required resource is preflighted before the first host call.
The emergency release slot remains its own owner through acknowledgement or destruction.
That closes the correctness questions in [#752](https://github.com/yan-h/harmonigraph/issues/752) and [#756](https://github.com/yan-h/harmonigraph/issues/756), but does not claim that the scheduler or its heap was removed.

## Setting it up in Bitwig

1. Load the matching plugin and renderer pair and confirm its build tag.
Use **by Vendor** process hosting, with individual hosting overrides off for both Harmonigraph classes.
2. Place one full Harmonigraph on Master and one Harmonigraph Tune in the note-effect path before each instrument.
Keep **Participating** on in each Tune editor.
Pairing is automatic when exactly one compatible Hub is present;
choose the intended Hub explicitly when there is more than one, and resolve a missing or ambiguous status before playing.
3. Hub and Tune start automatically at signed offset zero and read sample rate and maximum buffer size from host activation.
No validation checkbox or first-use Apply is required.
The Hub's **Session** menu and each Tune editor expose an advanced signed sample offset for a known routing delay;
use **Apply / Reinitialize** after changing it.
4. Leave **Tuning delay** at 1× and watch the missed-deadline line.
If notes are missing their deadline —
and the status does not say the session is starting, the Hub is missing, or a fault is latched —
raise the multiplier and play again.
The Hub's Session menu can apply one multiplier to every paired Tune.
5. In the Hub's **Tuning** pane, choose the tuning axes and policy-v2 controls for the session.
The moving-neighbourhood defaults match the simulator's baseline profile;
the [implementation record](adaptive-tuning-plugin.md#controls-and-live-neighbourhood) describes every control and the precision profile used by its paired intentional-E examples.
6. Play a phrase across several participating tracks and check the instrument's actual pitch as well as the displayed notes and live neighbourhood.
For reproducible policy checks use the versioned fixtures linked from the [implementation record](adaptive-tuning-plugin.md#validation), rather than the removed fixed-domain D/F/A expectations.
Destination conversion still matters:
the [historical pitch-conversion measurements](tuning-probe-bitwig.md#pitch-conversion) include Vital's required matching MPE settings and bend ranges.
7. Check Stop, the Participating toggle, project reopening, and live and offline output before treating a build as host-qualified.

## Diagnostics

The plugin emits `HG-TUNING` summaries through the Info-level plugin logger, including when editors are closed.
Its default sink is stderr, which Bitwig includes in `~/Library/Logs/Bitwig/engine.log`;
the `NICE_LOG` override still applies.
Summaries carry PID, registration, build tag, selected Hub and runtime source/session identities, so three Tune instances stay distinguishable.

Audio callbacks update fixed counters and publish atomic snapshots at roughly one-second audio intervals;
all formatting and logging happens on the main thread, at most once per wall-clock second and only when a meaningful value changes.
Absolute clocks do not cause idle logging.
No editor, worker, transport message or persisted field is involved, and diagnostics never mark setup dirty.

Each Tune reports consumed note on/off counts, accepted output counts and last actual pitch, received assignments and correction, adoption, retained work and the actual setup wait.
Each Hub reports the last published source, key and pitch together, the resolved tuning axes, locks, Auto and Learn, transition/input/publication wait reasons, and each row's membership, coverage and retention.
`*_mc` values are microcents;
sample frontiers use the shared mapped clock;
the minimum signed integer denotes an absent frontier.
Counters are cumulative per instance, so a last note stays historical after its release.

## Deliberately outside the design

These are not launch modes or implied follow-up work:

- immediate optimistic assignment, or a local adaptive fallback during failure;
- joint chord optimization;
- reconciliation or adaptive movement of already-sounding voices;
- retrospective reassignment of pending successors after a late predecessor;
- controller-history replay, or reconstructing the context a late note "should have" met;
- a central MIDI rack or router;
- a separate headless conductor;
- cross-process session transport;
- MTS-ESP, MPE or compatibility-first pitch outputs;
- raw-intention visualization as a second canonical event stream;
- specialized CC88 high-resolution velocity prefix association — ordinary note velocity is supported, correct prefix association under reordering is not.

Reconciliation is excluded absolutely, not deferred as a possible failure-recovery mechanism.
The others require a new request and a new issue before implementation.

## Open, deferred and not built

**Deferred framework simplification.** The broader output-scheduler removal described above is still unbuilt.
[#784](https://github.com/yan-h/harmonigraph/pull/784) completed the atomic replacement and emergency-owner correctness work without turning that optional simplification into a prerequisite.

**Deferred by decision.** Reporting Off notes to the Hub for display only.
It became a one-flag option the moment Off turned into local forwarding, and it is not built.

**Deferred policy choices.** Pedal-aware harmonic holding, a successor to the deliberately temporary bounded recent-note memory, and explicit anchors or additional root and excluded-pitch controls remain unbuilt.
The shipped policy-v2 controls do not imply those choices were made.

**Filed and open.**

| Issue | What it is |
|---|---|
| [#632](https://github.com/yan-h/harmonigraph/issues/632) | Final Bitwig acceptance for Stop, covering pending and sounded notes, pedal state and new stopped-live input |
| [#696](https://github.com/yan-h/harmonigraph/issues/696) | Final Bitwig acceptance for a fresh late second phrase on one channel without seeded neutral CC64/66/69. The historical blocking mechanism was structurally removed, but this exact host scenario remains unverified |
| [#738](https://github.com/yan-h/harmonigraph/issues/738) | **Reset context on stop** is now user-selectable and defaults off, so ended notes remain recent context and the moving displacement survives. Yan's choice of the desired behaviour and default remains open |
| [#748](https://github.com/yan-h/harmonigraph/issues/748) | Reply latency under roughly 510 same-sample captures is not reproducible run to run |
| [#783](https://github.com/yan-h/harmonigraph/issues/783) | Live MIDI channel bend is forwarded correctly and factual state reaches the final pitch, but display/take baselines do not preserve a separate sample-timed per-voice trajectory for every intermediate bend inside one callback |

The [#775](https://github.com/yan-h/harmonigraph/pull/775) follow-up batch closed [#728](https://github.com/yan-h/harmonigraph/issues/728), [#757](https://github.com/yan-h/harmonigraph/issues/757), [#759](https://github.com/yan-h/harmonigraph/issues/759) and [#761](https://github.com/yan-h/harmonigraph/issues/761) with their fixes integrated on `main`.
[#780](https://github.com/yan-h/harmonigraph/pull/780) closed [#774](https://github.com/yan-h/harmonigraph/issues/774) by gating the obsolete-completion injection test with its debug-only helpers;
[#781](https://github.com/yan-h/harmonigraph/pull/781) closed [#692](https://github.com/yan-h/harmonigraph/issues/692) by letting a later valid Reset supersede an invalid prospective Hub clock while inheriting the original transition boundary;
and [#784](https://github.com/yan-h/harmonigraph/pull/784) closed [#752](https://github.com/yan-h/harmonigraph/issues/752) and [#756](https://github.com/yan-h/harmonigraph/issues/756) with atomic replacement preparation and structurally verified emergency release-slot ownership.

**Closed, recorded here because the record was wrong about them more than once.** [#672](https://github.com/yan-h/harmonigraph/issues/672) was fixed through #669 → #709 and closed on 2026-09-07;
it was twice described as waiting on later work.
[#703](https://github.com/yan-h/harmonigraph/issues/703), reactivation staying pending after a host format change, was fixed while building the delay parameter —
it had to be, because the session sat in a dead latch and no delay change could be adopted at all.
It was reported as structurally resolved one stage earlier than it actually was.

## Evidence

The measurements this design rests on are archived rather than maintained:

- [`evidence/adaptive-tuning/`](evidence/adaptive-tuning/README.md) — the memory ledger, the replay callback-cost measurements, and the record of which numbers have since moved and by how much;
- [`evidence/615/`](evidence/615/README.md) with its [measurement report](tuning-probe-bitwig.md) — the Bitwig host-timing spike: callback order, event delivery, transport and offline cases.
Its verdict is *viable with narrower constraints*, at D = 2,048 samples, and it did not search for a minimum;
- [`tuning-probe.md`](tuning-probe.md) — the disposable apparatus that produced them, since deleted.

External references that establish the mechanisms, none of which prove Bitwig's behaviour in this chain:

- [CLAP events](https://github.com/free-audio/clap/blob/main/include/clap/events.h) — sample-accurate note expressions, voice addressing, relative tuning in semitones;
- [CLAP latency](https://github.com/free-audio/clap/blob/main/include/clap/ext/latency.h) — latency in samples, changes limited to activation, restart requested when already active;
- [Bitwig plugin hosting modes](https://www.bitwig.com/userguide/latest/vst_plug-in_handling_and_options/) — **By manufacturer** for plugins from one developer that communicate, and the per-plug-in Individually list;
- [Bitwig Note FX](https://www.bitwig.com/userguide/latest/note_fx/) — the pre-instrument note-effect placement;
- [MTS-ESP](https://github.com/ODDSound/MTS-ESP/blob/main/README.md) — its single-master note/channel lookup and client-query model;
- [Rust volatile-read semantics](https://doc.rust-lang.org/std/ptr/fn.read_volatile.html) — volatile access supplies no inter-thread synchronization.

The code seams:

- [`harmonigraph-plugin/src/performance`](../crates/harmonigraph-plugin/src/performance/mod.rs) — Tune, the Hub, retained input, the ordering pass, accepted output and central sequencing;
- [`harmonigraph-core/src/policy.rs`](../crates/harmonigraph-core/src/policy.rs) — the musical decision, pure and allocation-free;
- [`harmonigraph-core/src/notes.rs`](../crates/harmonigraph-core/src/notes.rs) and [`roll.rs`](../crates/harmonigraph-core/src/roll.rs) — source-aware tracking and live history, off the audio thread;
- [`harmonigraph-plugin/src/configuration.rs`](../crates/harmonigraph-plugin/src/configuration.rs) — effective CLAP tuning on audio, including editor-independent learning;
- [`harmonigraph-record/src/publication.rs`](../crates/harmonigraph-record/src/publication.rs) — the two independent lanes;
- [`harmonigraph-take/src/lib.rs`](../crates/harmonigraph-take/src/lib.rs) — format v4, source and reset scope, actual sample and pitch provenance.
