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

Built and merged in [#709](https://github.com/yan-h/harmonigraph/issues/709), simplified by the seven-stage [#712](https://github.com/yan-h/harmonigraph/issues/712) stack, and then **replaced from the transport down** by stage 8, [#786](https://github.com/yan-h/harmonigraph/issues/786).
Every adaptive-tuning bug in the two days after the #712 stack landed was a lifecycle bug — nine closed, plus [#787](https://github.com/yan-h/harmonigraph/pull/787)'s Apply stall — and none was a musical-policy bug.
The cause was structural: a graceful-settlement protocol between Tunes and the Hub, built to make every partial failure exactly-once.
Stage 8 deleted it.
What replaced it is **the fault cut**: a note emits on time with the correction that came back or without one, a fault is a status bit that never silences a track, and the only lifecycle event is a cut that nothing waits for.

The transport is now 3,253 production lines where it was 12,981, and its tests 1,425 where they were 12,040.
The module is `crates/harmonigraph-plugin/src/tuning/` rather than `performance/`.
Policy v2 is untouched: the same scorer, the same controls, the same fixtures.

Yan has used adaptive tuning in an ordinary Bitwig setup and reports that it works as expected.
That was the pre-stage-8 tree, so it is positive end-to-end evidence for the musical behaviour and not for the transport that now carries it.
[The list of what is open](#open-deferred-and-not-built) is at the end.
## Design priorities

This feature is optimized first for one person's maintained macOS/Bitwig workflow, not for format or host coverage.
Correct musical state, an inspectable failure mode and one small execution path outrank compatibility that has no current use.
Add another format, transport, mode or control only when it serves a concrete project and is cheap enough to carry in every later change.
Implementation prioritizes ordinary use:
about 15 notes across three tracks, with a generous growth bound of 16 tracks and 100+ simultaneous notes, and normal Stop, Reset and reopening.

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
| Input completeness | **Assign whatever arrived, sorted by sample.** A record whose sample was already sequenced is assigned when it arrives | Order complete source intervals against each source's published coverage watermark | Trial 20 of the Bitwig spike had complete intervals on all 3,422 Hub callbacks with zero offsets; only a deliberate +64 branch offset ever made the Hub wait, and waiting is what a Tune cannot afford now that it never holds a note back | A supported graph is measured delivering a source's earlier musical event after a later one |
| Assignment order | Chronological, then deterministic sequential assignment of simultaneous attacks | Joint chord optimization | Each new assignment sees the earlier ones without selecting a globally optimal chord or moving held voices | A concrete musical requirement demands a different policy |
| Clock | One absolute sample per copied record, taken from the host's steady time. No mapping, no offset, no per-report provenance | A mapped sample carrying the enclosing callback, the sub-block, the session epoch, the source incarnation and a sequence | The wrapper already resolves a sub-block's absolute start before the plugin sees an event, so the sub-block provenance was recovering something nothing had lost. Steady time is what makes two tracks orderable against each other, and a host that does not supply it is a fault status rather than a clock to reconstruct | A supported host is measured without a usable steady sample timeline |
| Storage layout | In-process preallocated storage with explicit ownership and off-thread reclamation | A pointer-free memory-mappable arena | `rtrb` already uses pointers and shared ownership; real-time safety wants bounded access and lifetime management, not a cross-process ABI | A concrete cross-process transport is authorized |
| Voice identity | Source, channel and key, with the host note id passed through untouched | The host voice id as identity, with a multiplicity fallback for its absence | Tune advertises no overlapping-note support, so a host must not overlap one key and channel; a retrigger replaces, which is the rule the tracker already applies | Bitwig is observed delivering overlapping same-key notes to a note effect that does not advertise them |
| State authority | **What the Hub scheduled** — input plus that source's D, derived from its own sequenced input | Actual emitted output, reported back by each Tune and reconciled against the plan | One authority cannot disagree with itself. The reconciliation was two provenances, an output ring, a journal and a dual-provenance reducer, to establish a fact the Hub had already computed | A real downstream pitch-feedback mechanism exists and is worth integrating |
| Pitch output | CLAP per-note tuning expression for the adaptive correction, composed with live player expression; MIDI channel bend remains MIDI | MTS-ESP, MPE or VST3 note output | Matches sample-timed per-voice assignments and the actual host while adding no external tuning service; Bitwig converts a note effect's per-note pitch to MPE or VST3 note expression downstream, so the instrument's own format is not restricted | A required instrument cannot consume it |
| Session transport | An in-process session under a documented Bitwig hosting mode, with **exactly one Hub per process** and no pairing choice | A saved pairing UUID with auto-join, ambiguity handling and a popup to resolve it | One Hub is the only case that has ever existed here. Pairing became one atomic load of one slot, which deleted the offer/return/lease handoff and every way it could stall | Two Harmonigraphs in one process become a configuration worth supporting rather than a fault to report |
| Hub ownership | Full Harmonigraph, normally on Master. **It is a Tune plus the policy**: it delays its own notes by D and is sequenced as a seventeenth source | A headless conductor, or an elected Tune peer; and separately, a direct observation path for the Hub's own MIDI that skipped the delay | Reuses the existing configuration, display, take and combined-audio location without another authority. Making its own track ordinary deletes `direct.rs` and the DIRECT half of every predicate that used to have one | The Hub cannot stay active in the supported graph, or a project needs tuning without a full Harmonigraph |
| Missed assignment deadline | **Emit on time, uncorrected, and count the miss.** A late reply is discarded | Keep that one attack pending and take its valid late assignment | Waiting for one note is what every stall in the #712 tree grew out of: per-note pending states, linked work cells, two late-playback rules and atomic same-key replacement all exist to make waiting safe. A raw note and a counter cost one note's tuning; waiting cost the whole track | A musical requirement makes a late correct pitch better than an on-time raw one — which is the judgement this reverses, so it needs new evidence rather than a preference |
| Participation UI | **None.** A Tune always participates; bypass or remove it in Bitwig | One Participating/Off control per Tune | Off was a whole second forwarding path with three separate consumer gates and a reset boundary of its own, for a state the host already provides | A project needs a track excluded from tuning while still passing through this plugin |
| Player pitch | Preserve the player's MIDI bend and per-note pitch expression, and add one frozen adaptive correction on top | Centre the bend and replace the expression | Player gestures remain expressive, and policy v2 can score their absolute onset pitch without changing its frozen adaptive choice later | A required gesture cannot be represented by this composition |
| Policy location | One pure sequential policy in the Hub | Distributed evaluation of shared state by each Tune | One owner has the complete ordered input and every preceding assignment; Tunes buffer and apply answers | The measured Hub work budget cannot support the chosen policy |
| A fault | **Keep passing notes through uncorrected, and show it.** No fault ever silences a track | Five fault classes, a latched shared reset, and an emission gate that refuses new attacks until an explicit Reset | A missing Hub, a full queue and a clock that stopped are all the same thing musically: this note has no correction. The gate turned every one of them into silence, and every stall Yan reported was audible as silence rather than as a wrong pitch | Silence becomes a better answer than a raw pitch for some fault this cannot distinguish |

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
| Divergence and faults | No retrospective reassignment, and no reassignment at all: a fault is a status bit and the note sounds raw | An 8-phase recovery transaction (Prepare→Inventory→Preserve→Status→Rebuild→Replay→Resume) with its own protocol messages, ~20 `recovering()` branches and three duplicated ordinary/replay code pairs | Its documented outcome when it fired was a note sounding 467 ms late. The latch that replaced it then produced silence instead, which is the failure Yan actually heard, so #786 removed that half too | A measured ordinary case needs a recovered phrase rather than a raw one |
| Controller context for a late note | A late note meets the receiver's current controller state; shared channel controls keep their own input-plus-delay schedule | A channel-wave mechanism replaying 130 folded controller registers plus raw prelude history into the receiver ahead of a younger onset | Different articulation during a late note is accepted; reconstructing the controller context a note "should have" met is a replay, and replay is the thing this design excludes | A measured ordinary case is musically wrong because of it |
| Publication loss | The lane reports its own loss into a reserved cell; the display clears stale state and refreshes from a current snapshot; an incomplete take exports with a warning | Per-source baselines with generation handles, an `UnsafeCell` bank behind a busy state machine, cross-thread resync requests and per-source repair cursors | A snapshot restores what is sounding now, which is the whole of what a display needs after a gap; the repair protocol was reconstructing history the display had already drawn | Reconstructing lost history becomes worth its own protocol |
| Membership and allocation | Sixteen rows and their ring pairs are built once **with the process-wide session**, and a side claims its own half at activation. Attach and detach bump the epoch, which is the cut for every row | Per-pairing allocation through an offer/return handshake, with `Slots`/`Reservation` and a lease each side had to agree on | The rings are ~1.5 MB for the whole process rather than per instance, and pairing became one atomic load — which is the only version of pairing that cannot stall, because there is nothing in it to wait for | Per-instance ring memory becomes a measured problem |
| The missed deadline | A Tune emits at input plus D whatever came back, and counts the onsets that came back with nothing | Per-note `Pending`/`Staging`/`Wait` states, linked work cells in `work.rs`, two late-playback rules, and the atomic same-key replacement that made retriggering a late note safe | Nothing waits, so none of it has anything to guard. #787 is the worked example: one barrier behind another, where fixing the settle predicate still did not restore audio | A late correct pitch is shown to beat an on-time raw one |
| What the display and take show | The Hub's own schedule, from its own sequenced input | An output-report ring per Tune, `Outcome`/`OutputDelta`, a local journal, a plan manifest, `debt.rs`, a dual-provenance `State` reducer, and nice-plug's per-event acceptance feedback | All of it established a fact the Hub had already computed. What it bought was detecting a host refusal, which is now an undetected residual — see case 2 below | A host refusal is observed happening, rather than reasoned about |
| Routing offset and branch latency | None. The Hub sequences whatever arrived in each callback, sorted by sample | A signed per-instance calibration offset, clock-mapping validation, Apply/Reinitialize, a coverage watermark and complete-interval waits | The offset was never once set to anything but zero, and the machinery that existed to make a nonzero one correct is what #787 stalled inside | A supported routing needs a branch offset, measured rather than assumed |
| Pairing | One atomic load of the session's Hub slot, at activation and on every callback while unpaired | A saved pairing UUID, auto-join, ambiguity handling, a popup, and offers/returns/leases through two slot cells | There is one Hub per process, so there was never a choice to make. #787 showed the re-pairing after a commit stalls independently of the settle stall; with pairing reduced to a load there is no re-pairing protocol left to stall | A second Hub becomes a supported configuration |
| Participation | None; a Tune always participates | An Off forwarding path, a toggle that stood as a reset boundary in the input queue, and three separate per-consumer Off gates | The host's own bypass is the same control, already built and already understood | A project needs a track that passes through this plugin untuned |
| Every lifecycle transition | **The cut**: Note-Off every held voice, neutralise the pedals, clear the delay line, adopt the new epoch. Nothing waits on anyone | `commit_transition` with ten `setup_wait` reasons, six settle predicates across Source and Row, and incarnation, generation, reset-generation and cancel-cut counters | One epoch remains of the four counters. A cut cannot stall because there is no second party to it: the Tune that takes it does not tell anyone, and the Hub learns from the same atomic it reads anyway | A transition needs to preserve sounding notes across it, which is the one thing a cut cannot do |
| A fault | One status bit, shown and never acted on | The `BUSY`/`CLOSED` emission gate, "refuse new attacks until Reset", and the latch as behaviour rather than as a status line | The latch survives as a status line, which is the half that was ever useful | A fault class is found where continuing is worse than stopping |

## The shape

One full Harmonigraph is the project's state hub, combined note display and take recorder.
A **Harmonigraph Tune** sits before each independently routed instrument path that should be tuned:

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
their copies replace the manual Note Receiver routing that displaying several tracks together used to need.

The Hub's own track is one of these.
It embeds the same Tune every other row runs, so its notes are delayed by D and tuned by the same policy;
what it does not have is a ring, because its copies are a direct call.

Both classes come out of the one existing bundle, `nice_export_clap!(Harmonigraph, HarmonigraphTune)`, because a process-wide session is shared only inside one dylib.
The loader scripts see the same bundle they always saw.

## The path of one note

```text
notes -> Tune delay line -> corrected, delayed notes -> instrument
           |    ^
   copies  |    | corrections
           v    |
         Harmonigraph Hub: one ordered pass, one decision per onset
                                   |
                                   v
                      combined display and take
```

For each processing interval a Tune:

1. copies every input event to the Hub's ring for its row —
`(epoch, serial, absolute sample, event)` —
and keeps the event in a local delay line keyed by `sample + D`.
A full ring drops the copy;
that note will emit uncorrected and count as a miss;
2. drains replies.
A reply `(epoch, serial, correction)` attaches to the pending note-on with that serial.
Wrong epoch, or no such serial: dropped;
3. emits everything whose time has come.
A note-on with a correction gets its tuning expression;
one without emits uncorrected and increments the miss counter.
Bend and per-note expression pass through, with the frozen correction composed into any expression addressed to a corrected voice;
4. keeps the held set `(channel, key)` and the pedal state per channel.
That is the whole of what the cut needs.

Each callback the Hub drains every row's ring, sorts by `(sample, is_onset, key, channel, source, serial)`, applies releases and controllers, assigns each onset through policy v2, and pushes the reply.
It folds the scheduled result —
input plus that source's D —
into the display and the take through the existing two publication lanes.
Tunes run no assignment policy and hold no copy of the Hub's state.

The Hub's own MIDI takes exactly that path.
It embeds a Tune whose ring is a direct call, so its track is delayed by D, assigned by the same policy and published through the same row as any other.
There is no second observation path to keep in step with the first.

Work is bounded per callback and runs to completion inside it.
No callback waits for another instance, takes a contended lock, allocates, frees, or does I/O.
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

An onset that reached its own emission time with no correction sounds at its raw pitch, and the Tune counts it:
`3 notes sounded uncorrected - raise the delay and play again`.
The count clears at every cut.

Two states are *not* a missed deadline, and the status names them rather than counting them as evidence that the delay is too small:
no Harmonigraph in the process, and no free row.
A larger delay is no remedy for a missing Hub.
A default of 1x is a starting point, not a reliability guarantee;
validate the multiplier in the actual project.

One case reaches the counter for a reason a bigger multiplier does fix, and one does not.
If the Hub's callback runs before a Tune's within a block, every 1x note misses —
the counter shows it and 2x fixes it.
If the copy ring was full, that note was never in the Hub's context at all;
a larger delay does not make the ring bigger.
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

## Nothing is late

An onset that has no correction when its time comes emits without one.
There is no per-note wait, so there is no late playback, no track-wide accumulated delay, no catching up in a burst, and no ordering exception for a note whose answer arrived after it sounded.
A reply that arrives after its note has emitted is dropped where it is found.

What this costs is one note's tuning, once, with a counter beside it.
What it removes is every mechanism that existed to make waiting safe:
the pending and staging states, the linked work cells, the rule that an event addressed to an unsounded note waits with it, the rule that a replacement onset force-releases its predecessor at emission, and the atomic same-key replacement that made the second rule correct.

A late note therefore meets the receiver's current controller state, as it did before, and shared channel controls keep their own input-plus-D schedule.
Ordinary lateness is not a reason to cancel anything.
Explicit Reset, transport Stop, and a membership change are the separate cases, and all three are the same cut.
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

## Membership

A Tune claims one of sixteen rows when the host activates it, and gives it back when the host destroys it.
Both are one main-thread operation on the process-wide session, and both bump the session epoch.

There is no participation control.
A track that should not be tuned is bypassed or removed in Bitwig, which is the same control the host already provides for every note effect.
An unbypass in the middle of a note is Bitwig's bypass behaviour, as for any note effect.

A seventeenth Tune finds no free row.
It is refused visibly —
its status says so and its notes pass through uncorrected at the same delay —
rather than evicting one of the first sixteen.
## The cut, and faults

There is one lifecycle operation and every path performs it.
**The cut**: Note-Off every held voice, neutralise the pedals that are down, clear the delay line, adopt the new epoch.
Nothing waits on anyone, and no second party has to agree that it happened.

| Trigger | How it reaches everyone |
|---|---|
| Explicit Reset, from either editor | Bumps the session epoch and marks it a Reset. Every Tune cuts on its next callback; the Hub clears its context and its released memory |
| Transport Stop | The falling edge of the host's playing flag, seen by whichever Tune the host gives the transport to. Bumps the epoch and marks it a Stop; released memory then follows the **Reset context on stop** control |
| The audio engine stopping, or a host reset | The same as Stop, from `clap_plugin.reset` and `stop_processing` |
| A Tune attaching or detaching | Bumps the epoch, so every paired track cuts. This is what buys the absence of a hot-plug reconciliation protocol |
| Host reactivation, or a latency or format change | That Tune cuts locally and adopts D from the saved multiplier and the new format |
| Destruction | The row goes back to the session on the main thread. Records the departing Tune left in its ring carry an epoch the Hub has already moved past, so they are refused where they are found |

A **fault is not a cut**.
It is a bit in a status word, shown in the editor and in `HG-TUNING`, and it never stops a note:
no Harmonigraph in the process, a second one, no free row, a full copy ring, no steady sample timeline, a policy evaluation the scorer refused, a publication lane that lost a report, and a retained event that did not fit.
Every one of them means the same thing musically —
*this note has no correction* —
and the answer to all of them is the raw pitch and the counter.

The epoch is one `AtomicU64`.
A Tune reads it once per callback and cuts when it differs from the one it is running under;
the Hub reads the same value and clears its own context.
That is the entire protocol, and it is why there is no `commit_transition` here to park on a row that can never settle.

The musical-context controls remain independent of physical note cleanup.
A zero silence timeout never clears context;
a positive timeout clears released memory and displacement once no held context remains and the callback frontier reaches it.
The optional loop/seek reset detects a greater-than-two-millisecond discontinuity between the host's seconds timeline and elapsed sample time, then clears released memory once at the first subsequent attack while held notes keep their frozen corrections.
## Voice identity

Channel and MIDI key are not enough once several tracks contribute:
two tracks can hold channel 0 / key 60 at once, and releasing one must not release the other.

```text
(source row, channel, key)
```

That is complete by contract rather than by luck.
Tune leaves its poly-modulation config unset, so it never advertises the CLAP voice-info extension, and a host must then not send overlapping note-ons for one key and channel.
A retrigger that arrives anyway replaces the held voice, which is the rule the tracker already applies.
The host's note id is passed through on every emitted event and the tuning expression is addressed with it, but it is not part of the identity.

The source id reaches every identity-bearing path:
core events, the active-voice model, tracker keys and held-end keys, `NoteRoll` live keys and bend/release lookup, take records and offline replay.
The Hub's own track keeps the reserved `SourceId::DIRECT` every consumer downstream already knows it by, and a Tune row is one past its own index, so no paired track can collide with it.
A cut releases every source at once, because the epoch it adopts is the session's.
A row that changes hands keeps nothing from its last occupant:
records left in its ring carry an epoch the Hub has already moved past, and it refuses them where it finds them.

The initial active lifetime is note-on through note-off or choke.
Sustain-aware harmonic context is deferred until a real sustain-heavy project demonstrates the need;
Tune still forwards pedals and unrelated MIDI at the same delay, and pedal sustain or instrument release tails do not turn a released policy contribution back into held context.

## Pitch output

The only output backend is the CLAP per-note tuning expression, and Tune is exported only as CLAP.
At note-on, Tune emits a `CLAP_NOTE_EXPRESSION_TUNING` value for the same voice and sample position as the note.
CLAP note expressions state the current value rather than adding a delta.

Tune freezes only the adaptive correction and the tuned onset used as harmonic context.
The policy scores the note's absolute attack pitch, including register, attack expression and current MIDI channel displacement.
Later per-note expression is added to that frozen correction, and raw MIDI channel bend is forwarded at its original sample position without being counted into the correction twice.
RPN 0 pitch-bend sensitivity is followed, with a two-semitone default.
Velocity, pressure and other non-pitch controls keep their normal behaviour.
An onset that got no correction emits with no tuning expression at all;
stating a zero over whatever bend the player is holding would be centring it by another name.

Emitted pitch and the tuned onset used by the policy are separate facts.
Channel-bend changes update the scheduled voices and request display/take baselines without pretending that the original MIDI message was a separately accepted per-note event.
The live output and final scheduled pitch are correct, but [#783](https://github.com/yan-h/harmonigraph/issues/783) records that multiple bends within one callback do not yet produce a separate sample-timed per-voice trajectory for every intermediate value in the display or take.

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

There is **one session per process**, allocated with its rings the first time any instance registers and never freed.
It holds one Hub slot, sixteen Tune rows and one epoch.

Pairing is a load of the Hub slot.
The first Harmonigraph to register owns the session;
a Tune claims a free row at activation and can retry on any callback while it holds none, because the claim is a compare-exchange and the endpoints come out under a `try_lock` that never blocks.
A second Harmonigraph gets no rings and sets `SECOND_HUB` on itself and on every Tune —
a status on both rather than a choice to resolve.
Nothing is offered, returned, leased or acknowledged, so there is nothing here that can be half-done.

No pairing state is saved.
There is no session UUID, no per-Tune pairing UUID and no routing calibration, so a reopened project pairs the same way a fresh one does.

Bitwig's **by Vendor** hosting mode is the measured requirement, with individual hosting overrides off for both Harmonigraph classes.
Each instance takes sample rate and maximum buffer size from host activation;
they are read-only runtime values and a reloaded instance uses the new host format automatically.
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
Later player expression and channel bend change the emitted and scheduled pitch without rerunning the adaptive choice.
Departing or excluded sources leave no stale released context.
Silence, Stop and loop/seek resets follow the controls described in the implementation record, while an explicit Reset clears musical context through the existing reset boundary.

Selection uses preallocated scratch and refuses resource exhaustion instead of scoring a truncated candidate set.
The policy never reads camera reach or display tolerance.
The live lattice's C2–C7 winner outlines are an off-audio-thread view of the Hub's authoritative next-attack context, not an input to the score and not part of preview or export pictures.

## Capacities and memory

These are selected ceilings sized for 16 tracks and 100+ simultaneous notes, not measured throughput.
Audio transport and policy-selection storage is preallocated;
the more expensive live-neighbourhood calculation runs off the audio thread.
Exhaustion is an explicit bounded failure —
a status bit and a raw pitch —
and shrinking a buffer does not make its overflow path unreachable.

| Constant | Value | Resource |
|---|---:|---|
| `TUNERS` | 16 | Tune rows in the session; the Hub's own track is a seventeenth source with a reserved index |
| `HELD_PER_SOURCE` / `HELD_SESSION` | 64 / 256 | Held voices per source / across the session |
| `PENDING_EVENTS` | 8,192 | Retained events in one Tune's delay line, including unrelated MIDI |
| `CAPTURE_RING` / `REPLY_RING` | 1,024 / 1,024 | The two rings per row. A full capture ring costs that note its correction |
| `BATCH_EVENTS` | 2,048 | Records one ordering pass holds across every source, and deltas one callback may publish |
| `CUT_EVENTS` | 128 | Note-offs and pedal neutralisations one cut may owe |
| `MAX_CONTEXT` / `MAX_MEMORY` / `MAX_CANDIDATES` | 256 / 24 / 4,096 | Held policy references, released contributions and the complete local candidate union |
| `PUBLICATION_RING` / `SNAPSHOT_SLOTS` / `GAP_RESERVE` | 4,096 / 20 / 1 | Publication items, snapshot frames, and the one cell an outage may always spend |
| `DELAY_MULTIPLIER_MAX` | 16 | Steps on the Tuning delay parameter |

The rings now belong to the **process** rather than to an instance.
Sixteen capture rings and sixteen reply rings are about 1.5 MB, allocated once when the first instance registers, and that is what pays for pairing being an atomic load instead of a handoff.
A Tune's own delay line is the per-instance cost:
8,192 retained events, allocated at construction.

Policy coordinates use signed 32-bit lattice coordinates and corrections use signed 64-bit microcents, so an individual decision is bounded without imposing the old small-coordinate or approximately 2,147-cent limit on a musical journey.
Audio-thread allocation is excluded, and the vendored framework's allocation guard asserts it around the real CLAP process callback.

The historical byte-by-byte ledger, including its own drift, is in [the evidence archive](evidence/adaptive-tuning/README.md).
## Display, the take and export

The Hub publishes **what it scheduled** —
input plus that source's D, with the correction it chose and the player's own expression composed in —
to the live display and the take recorder.
That is what the Tune will emit, so it is a schedule rather than a confirmation.
Nothing observes whether the host accepted it:
if Bitwig refuses an emitted event, the display and the take show a note the instrument never got, and nothing in the protocol can tell.
This was never a measurement of resulting acoustic pitch either;
a receiving instrument may ignore, smooth or modulate the expression.

One provenance reaches the take now rather than two.
`PitchProvenance` keeps both variants so an old take still parses, but every record this build writes carries `AcceptedOutput`.
Learning is the one consumer that reads something else:
it is given the **player's** pitch, with the adaptive correction taken back off.
Every row is tuned now, the Hub's own included, so feeding Learn what was emitted would make it a fixed point that infers the axes it has already chosen.
The old design avoided that only because the Hub's own track was the one thing it did not retune.
Learn also reads one callback later than it used to, because the Hub's own input is delayed by D like everyone else's;
the configuration walk that runs it happens before the performance input for that callback is delivered.
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

Removed from it during the #712 simplification:
the retain-and-acknowledge input contract (`Consumption::Pending`), the legacy input walker, and a second complete notification emitter reachable only from fixtures.
Kept, against an earlier plan to remove them:
the three input cursors, which turned out to be one progress mark per consumer over a single shared pool rather than the retain machinery —
giving each consumer its own copy would add cost and a mechanism instead of removing one.

**The broader group/lane/heap output-scheduler simplification remains deferred, and is now unused rather than load-bearing.** [#786](https://github.com/yan-h/harmonigraph/issues/786) listed `Group`, `Output`, `Lane`, `Token` and `Completion` for deletion along with per-event acceptance feedback.
The plugin side of that is done:
`clap_performance_prepare` returns `true` unconditionally and `clap_performance_complete` ignores its argument, because there is no eligibility left to check and no acceptance to record.
The vendored code is still there.
Deleting it means giving `Output` the host's event pointer so `push(value, time)` can call `try_push` directly, and rewriting the 1,644-line boundary suite that tests the scheduler;
that is a second, self-contained change in a vendored crate and it was left out of stage 8 deliberately rather than forgotten.
The emergency lane and the two-event onset group are the only shapes the plugin still stages.

## Setting it up in Bitwig

1. Load the matching plugin and renderer pair and confirm its build tag.
Use **by Vendor** process hosting, with individual hosting overrides off for both Harmonigraph classes.
2. Place one full Harmonigraph on Master and one Harmonigraph Tune in the note-effect path before each instrument.
There is nothing to pair:
each Tune finds the one Harmonigraph in the process at activation.
A track that should not be tuned is bypassed or has no Tune.
3. Leave **Tuning delay** at 1x and watch the uncorrected-note line.
If notes are sounding uncorrected —
and the status does not say there is no Harmonigraph in the process or no free row —
raise the multiplier and play again.
The Hub's **Session** menu can apply one multiplier to every Tune at once;
each one then requests its own reactivation.
4. In the Hub's **Tuning** pane, choose the tuning axes and policy-v2 controls for the session.
The moving-neighbourhood defaults match the simulator's baseline profile;
the [implementation record](adaptive-tuning-plugin.md#controls-and-live-neighbourhood) describes every control and the precision profile used by its paired intentional-E examples.
5. Play a phrase across several tracks and check the instrument's actual pitch as well as the displayed notes and live neighbourhood.
For reproducible policy checks use the versioned fixtures linked from the [implementation record](adaptive-tuning-plugin.md#validation).
Destination conversion still matters:
the [historical pitch-conversion measurements](tuning-probe-bitwig.md#pitch-conversion) include Vital's required matching MPE settings and bend ranges.
6. Check Stop, **Reset voices**, adding and deleting a Tune mid-phrase, deleting the Hub mid-phrase, a host buffer-size change, project reopening, and live and offline output before treating a build as host-qualified.
Every one of those is a cut, and what it should sound like is the phrase stopping cleanly and the next one being tuned again.
## Diagnostics

The plugin emits `HG-TUNING` summaries through the Info-level plugin logger, including when editors are closed.
Its default sink is stderr, which Bitwig includes in `~/Library/Logs/Bitwig/engine.log`;
the `NICE_LOG` override still applies.

Audio callbacks store fixed counters into a seqlocked snapshot at roughly one-second audio intervals;
all formatting happens on the main thread, at most once per wall-clock second and only when a value changed.
An idle plugin never logs, because the interval is counted in frames rather than in wall-clock time.

One line carries the process id, the role, the build tag, how many Harmonigraphs are loaded, the session epoch and the fault text.
A Tune adds its row, its delay, notes in and out, copies made, uncorrected notes, dropped events, voices held and events pending.
A Hub adds its live row count, its policy context size, decisions taken, deltas published and the moving reference.
Counters are cumulative per instance, so a last value stays historical after its note ends.

The `HG-TUNING-SETUP` line is gone with the setup generation it reported.
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

## Residual cases, written down rather than handled

These are the consequences [#786](https://github.com/yan-h/harmonigraph/issues/786) accepted in exchange for a transport with nothing in it to stall.
None of them is a bug to be fixed without reopening the decision above it.

1. A note that misses its deadline sounds at raw pitch;
the counter says so.
2. If Bitwig refuses an emitted event, the display and the take show a note the instrument never got, or a note-off it never got.
Undetected, because per-event acceptance feedback went with the output reports.
3. If the Hub's callback runs before a Tune's within a block, every 1x note on that Tune misses;
the counter shows it and 2x fixes it.
4. Cross-track chord order is arrival order within a Hub callback, sorted by sample;
branch latency is not corrected.
5. Explicit Reset, transport Stop, a Tune attaching or detaching, and a host-format change cut sounding notes on **every** track, not only the one that caused it.
6. A second Harmonigraph in the process is a fault status on both;
there is no pairing choice, and the first one loaded keeps the session.
7. A ring overflow drops that note from the Hub's context and it sounds uncorrected.
8. An unpaired Tune passes notes through with the delay and no correction, and its status says there is no Hub.
9. Off is gone;
an unbypass mid-note is Bitwig's bypass behaviour, as for any note effect.
10. A destroyed Tune cannot release what it is holding, because no callback follows its destruction.
The instrument's own voices end with the device, which is what deleting a note effect mid-note does anyway.
11. Learn reads one callback later than the input it learns from, and reads the player's pitch rather than the emitted one.
Both follow from the Hub tuning its own notes;
the second is what keeps Learn from becoming a fixed point.
12. [#783](https://github.com/yan-h/harmonigraph/issues/783), intermediate bend trajectories, is unchanged by this pass.

## Open, deferred and not built

**Deferred framework simplification.** The nice-plug output scheduler is now unused by the plugin but still vendored;
[#789](https://github.com/yan-h/harmonigraph/issues/789) holds the exact remaining edit and the one non-mechanical question in it.

**Deferred policy choices.** Pedal-aware harmonic holding, a successor to the deliberately temporary bounded recent-note memory, and explicit anchors or additional root and excluded-pitch controls remain unbuilt.
The shipped policy-v2 controls do not imply those choices were made.

**Filed and open.**

| Issue | What it is |
|---|---|
| [#632](https://github.com/yan-h/harmonigraph/issues/632) | Final Bitwig acceptance for Stop, covering pending and sounded notes, pedal state and new stopped-live input. The mechanism it tested is gone; the host scenario is not |
| [#696](https://github.com/yan-h/harmonigraph/issues/696) | Final Bitwig acceptance for a fresh late second phrase on one channel without seeded neutral CC64/66/69 |
| [#738](https://github.com/yan-h/harmonigraph/issues/738) | **Reset context on stop** is user-selectable and defaults off. Yan's choice of the desired behaviour and default remains open |
| [#748](https://github.com/yan-h/harmonigraph/issues/748) | Reply latency under roughly 510 same-sample captures was not reproducible run to run. The mechanism it was measured against no longer exists |
| [#783](https://github.com/yan-h/harmonigraph/issues/783) | Live MIDI channel bend is forwarded correctly and the schedule reaches the final pitch, but display/take baselines do not preserve a separate sample-timed per-voice trajectory for every intermediate bend inside one callback |

**Closed, recorded here because the record was wrong about them more than once.** [#672](https://github.com/yan-h/harmonigraph/issues/672) was fixed through #669 → #709 and closed on 2026-09-07;
it was twice described as waiting on later work.
[#703](https://github.com/yan-h/harmonigraph/issues/703), reactivation staying pending after a host format change, was fixed while building the delay parameter.
[#787](https://github.com/yan-h/harmonigraph/pull/787) reproduced the Apply stall and diagnosed it without fixing it, on purpose:
its test is retargeted here as "Reset cuts and audio resumes within two callbacks".
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

- [`harmonigraph-plugin/src/tuning`](../crates/harmonigraph-plugin/src/tuning/mod.rs) — the Tune's delay line, the Hub's ordering pass, the process-wide session and the cut;
- [`harmonigraph-core/src/policy.rs`](../crates/harmonigraph-core/src/policy.rs) — the musical decision, pure and allocation-free;
- [`harmonigraph-core/src/notes.rs`](../crates/harmonigraph-core/src/notes.rs) and [`roll.rs`](../crates/harmonigraph-core/src/roll.rs) — source-aware tracking and live history, off the audio thread;
- [`harmonigraph-plugin/src/configuration.rs`](../crates/harmonigraph-plugin/src/configuration.rs) — effective CLAP tuning on audio, including editor-independent learning;
- [`harmonigraph-record/src/publication.rs`](../crates/harmonigraph-record/src/publication.rs) — the two independent lanes;
- [`harmonigraph-take/src/lib.rs`](../crates/harmonigraph-take/src/lib.rs) — format v4, source and reset scope, actual sample and pitch provenance.
