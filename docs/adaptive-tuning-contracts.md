# Adaptive tuning engineering contracts

This is the stage 2 implementation contract for [#617](https://github.com/yan-h/harmonigraph/issues/617) and [#616](https://github.com/yan-h/harmonigraph/issues/616), under the product decisions in [adaptive-tuning.md](adaptive-tuning.md) and accepted [PR #633](https://github.com/yan-h/harmonigraph/pull/633).
It selects engineering mechanics before production plugin wiring.
None of the queues, scheduling states or production limits specified here is implemented by this documentation change.
The [stage handoff](adaptive-tuning-stage-2.md) distinguishes inspected code, arithmetic and verification still owed.

The retained probe establishes a constrained host premise, not production safety:
[Bitwig measurements](tuning-probe-bitwig.md) used D = 2,048 and an artificial policy.
The accepted production candidate remains **D = 512 samples at 44.1 kHz, with a 512-sample engine buffer**.
No smaller buffer, larger D, additional standalone spike or musical fallback is selected here.

## 1. Ownership and identity

One hub audio owner sequences policy, membership, configuration revisions, confirmed context and prospective history.
Each tuner audio owner retains its input performance stream, request lifetimes, player expression, accepted output and release obligations.
An owner is a serialized plugin callback, not a particular OS thread:
Bitwig moved callbacks between audio threads in the recorded trials.
Processing and the emitter completion callback for an instance must not overlap.
Display, take serialization, registry mutation, allocation and final destruction remain off that path.

Use these distinct identities:

| Name | Meaning and lifetime |
| --- | --- |
| `saved_session_uuid` | Persisted hub pairing identity; a tuner retains its selected pairing UUID. Two live hubs with the same saved UUID are ambiguous, even if both appear healthy. |
| `runtime_session` | Process-local monotonic `u64`, allocated by the off-thread registry for one hub lifetime; never restored from a file. |
| `source_slot` | `0` is reserved for hub direct input; `1..=16` are tuner slots. Enrollment fixes the source tie order until a membership boundary. |
| `source_incarnation` | Monotonic `u64` for each slot lease. Reusing a slot never reuses its incarnation or pending data. |
| `clock_epoch` | Mapping/activation generation; changes only at an acknowledged recovery boundary. Transport song position is not this clock. |
| `membership_revision` | Hub-owned immutable set of source leases and their continuous coverage starts. |
| `lifetime` | Source-monotonic `u64` assigned on input attack; same key retriggers get different lifetimes. |
| `input_sequence`, `output_sequence` | Separate source-monotonic `u64` sequences; an output sequence identifies an actual accepted event, not a send attempt. |
| `request`, `plan_generation`, `decision_serial` | A lifetime's assignment request, replaceable plan version, and total hub evaluation order. Revoking a plan need not cancel the request. |
| `configuration_revision` | Hub-owned monotonic revision, with the complete resolved musical configuration copied into each bound request/plan. |
| `recovery_id`, `stop_generation` | Monotonic control transactions independent of ordinary queue sequences. |

All internal addresses include runtime session, epoch and source incarnation, explicitly in a record or in an immutable endpoint binding plus checked record generation.
No bare source index, host note id, pointer address or saved UUID can validate a reply.
Counters use checked increment;
exhaustion requires off-thread reinitialization rather than wraparound/ABA.
Slot ordinals are deterministic within the accepted membership, including all callback permutations;
they do not promise the same arbitrary chord placement after a different enrollment order on project reload.

The musical voice key is `(source lease, channel, key)` and a lifetime disambiguates its replacements.
Pass the host note id through unchanged, including a wildcard release id;
resolve each incoming wildcard against the lifetimes present at that position in the source stream before buffering it.
Never resolve it against a later retrigger at output time.
One note port is supported initially.
The companion advertises no overlapping-voice support.
Unexpected same-key overlap becomes an explicit old-lifetime release followed by a new lifetime, never two indistinguishable held entries.

Hub direct input has its own coverage and sequence bookkeeping and at most 64 admitted voices, counted within the 256 session total.
Its received events describe observed direct input, with that provenance preserved;
they must not masquerade as tuner output or acquire D a second time.
Any notes the hub forwards through CLAP additionally obey the acceptance boundary below.

Source scope reaches core events, tracker and roll keys, held ends, take records and offline replay.
A recovery baseline is a distinct control record, not fabricated note-ons.
The implementing PR must state the take-format and saved-state changes and follow the [persistence contract](../.claude/skills/persistence-contract/SKILL.md):
container defaults on persisted structs, no compatibility aliases, and audible parse/version refusal.
No runtime queues, history, incarnations or clock epochs are persisted.

## 2. Named capacity and admission limits

These are selected storage ceilings, not throughput measurements.
Preallocate on registration/activation, including failure paths;
never let `Vec::capacity()` or a ring's internal capacity accidentally define admission.

| Constant | Value | Accounted resource |
| --- | ---: | --- |
| `TUNERS` / `SOURCE_ROWS` | 16 / 17 | Tuner leases / tuner plus reserved direct-input row |
| `HELD_PER_SOURCE` / `HELD_SESSION` | 64 / 256 | Accepted held voices plus admitted in-flight reservations; Off voices retain their source and session reservations |
| `PENDING_EVENTS` | 8,192 per tuner | All retained performance events, including unrelated MIDI and queued releases/expression |
| `LIFETIMES` | 8,192 per tuner | Separate request records, including unsounded attacks and terminal requests awaiting acknowledgements |
| `INTENT_RING` | 1,024 per tuner | Source to hub transfer window; unsent data stays in source-owned storage |
| `REPLY_RING` | 1,024 per tuner | Hub to source transfer window; unsent answers stay in the hub ledger |
| `OUTPUT_RING` | 2,048 per tuner | Accepted output deltas to the hub |
| `OUTCOME_JOURNAL` | 4,096 per tuner | Local ordinary accepted-output records, retained through the hub's explicit output-retention acknowledgement, never retired by a held baseline alone |
| `INGRESS_WINDOW` | 1,024 per source row | Hub-owned collected intents, including complete-cohort assembly |
| `OUTPUT_WINDOW` | 2,048 per source row | Hub-owned collected actual output awaiting a complete merge frontier |
| `PUBLICATION_RING` | 4,096 per session | Ordered canonical output/control publication to the non-RT display/take drainer; consumer loss is separate from musical state |
| `PLAN_LEDGER` | 131,072 session-wide | At most `16 * LIFETIMES` unretired tuner assignment plans; direct input needs no assignment plan |
| `COHORT_EVENTS` / `COHORT_ONSETS` | 1,024 / 256 | One same-sample dependency graph / onsets in that graph, across all source rows |
| `HISTORY_KEYS` | `17 * 16 * 128 = 34,816` | Direct-index source/channel/key history; confirmed and prospective copies |
| `BASELINE_SLOTS` | 2 per source | Independently owned complete 64-voice baseline slots |
| `PENDING_MANIFEST_WINDOW` | 64 per source | Chunked unsounded-request recovery records; never a substitute for the held baseline |
| `EMERGENCY_VOICES` | 64 per source | Dedicated release/completion cells, outside ordinary journals and queues |
| `EMERGENCY_CHANNELS` | 16 per source | Pedal/controller reset obligation and acceptance cells |
| `CONFIG_COMMANDS` / `CONFIG_SLOTS` | 128 / 2 | Bounded ordered configuration commands / owned restore snapshots |
| `CONFIG_TIMELINE` | 128 per session | Retained sample-timed configuration markers ahead of the finalized input frontier, separate from command/restore handoff storage |
| `CONTROL_SLOTS` | 2 per direction per source | Owned progress/recovery/control frames; repeated fault bits use atomic latches |
| `ATTACHMENT_SLOTS` | 2 per direction per instance | Registry offer/return and callback acknowledgement slots, available before any session is paired |

Each pending input event owns exactly one event slot until its output is accepted or its cancellation is authorized and acknowledged.
A request record is separate because its note-on event can leave the pending pool while the voice, reply rejection or revocation acknowledgement still needs its identity.
Do not retire a request until all its events, emitted lifetime, plan and acknowledgement references are gone.
The 8,192 lifetime ceiling can therefore be reached before the performance pool;
that is a named required resource, not an excuse to overwrite the oldest request.

Transfer-window saturation is backpressure while the sender retains the data.
Stop advancing the relevant publication cut and keep processing independent releases/control work.
An ordinary deadline miss, age, lack of a reply or full transfer ring alone does not authorize dropping an attack.
Actual exhaustion of a required event, lifetime, journal, cohort, voice or plan slot invokes the accepted emergency stop:
source-owned storage stops that source;
hub/global storage stops the session.
The 17th tuner is visibly refused without evicting one of the first 16.

Voice admission reserves a source cell and a session credit before any note-on is submitted.
Credits cover Off voices as well as adaptive voices and remain charged through an accepted release/choke or established host termination boundary.
Withdrawal from context does not return a credit for a voice still held downstream.
Use a single atomic session reservation count with bounded compare-exchange attempts:
one acquire load and one `compare_exchange` (`AcqRel` success, `Acquire` failure) per admission attempt, no retry loop.
A lost race leaves the attack buffered and retries on a later callback;
it is not proof of exhaustion.
An unattempted/rejected onset with no accepted prefix returns its reservation exactly once when its attempts settle.
For an accepted onset, termination returns exactly one credit with `fetch_sub(AcqRel)` from its owning source only after accepted release/choke and an acknowledged complete output frontier through that release sample, or an established host termination boundary.
Host acceptance of a future-offset release alone cannot free a cross-source credit for an earlier-timestamp onset.
Keep that retiring reservation charged while the complete frontier catches up;
an already accepted release awaiting that acknowledgement is backpressure, not proof of excessive concurrent held demand.
The hub mirrors reservation ownership through outcomes/baselines, never independently increments or decrements it.
Unknown downstream state retains the old lease's credits until a termination boundary.

An observed full count waits for already-planned preceding/same-sample releases to settle before attempting the replacement.
If the completed ordered cohort actually requires more than 256 concurrent reservations, latch session exhaustion;
an independent Off input performs the same admission check and reports a full reservation failure to the hub through the emergency lane.
If the hub is unavailable, retain the Off attack while possible and latch exhaustion only when required local retention is exhausted or the full live reservation set is established.
No assertion of healthy D applies to contended/full admission during failure.
All 16 sources may use up to 64 local cells, but cannot simultaneously reserve `16 * 64` session voices.

Pending events carry fixed-size CLAP note/expression and short MIDI values by value.
Do not retain a borrowed host pointer or clone heap-backed SysEx on the audio thread.
The existing `SysExMessage = ()` boundary remains explicit:
arbitrary SysEx payload forwarding is not added by this stage.
Ordinary supported MIDI CC, bend, pressure, program and per-note expression events must all be counted, delayed and ordered.

## 3. Publication, lifetime and bounded work

Use three `rtrb` 0.3.5 SPSC endpoint pairs per tuner for intents, replies and actual output.
The version is pinned in `Cargo.lock`.
Each producer/consumer belongs to one serialized owner for the whole lease, even as the host changes threads.
Messages are fixed `Copy` values with no destructor, owning `Arc`, `String`, `Vec` or external pointer.
The ring producer writes its exclusively owned empty slot before publishing its tail with Release;
the consumer acquires the tail before reading, and releases its head before producer reuse.
The reciprocal acquire prevents overwrite during a read.
Use the library's implementation, not a new speculative ring.

Progress, baseline and configuration frames use fixed slots with an `AtomicU8` ownership state:
`Empty -> Writing -> Ready -> Reading -> Empty`.
The designated producer acquires Empty once, writes the ordinary payload exclusively, then stores Ready with Release.
The designated consumer acquires Ready once, owns the immutable payload through its copy/application, then stores Empty with Release.
The next producer acquires Empty before rewriting.
Each side inspects at most two slots once per callback;
no spin, retry-until-stable or plain/volatile racing read is permitted.
The payload may be `UnsafeCell<T>` only behind this exclusive-ownership proof and a safe interface whose tests exercise slot retention and reuse.
A reader retains Reading across callbacks if bounded application is unfinished.
Do not reclaim a slot merely because a newer frame exists.

Separate atomic fault bitsets and monotonic revocation generations remain available when all slots are occupied.
Their Release publication / Acquire observation conveys a control request, not a coherent struct snapshot.
Publish the associated cut/acknowledgement in an owned frame;
until that frame arrives the reader knows only that work is inhibited.
Relaxed counters are permitted for diagnostic totals that publish no payload and decide no correctness.
[Rust's atomic ordering rules](https://doc.rust-lang.org/std/sync/atomic/enum.Ordering.html) do not make racing non-atomic payload copies safe.

The off-thread registry owns every session arena and all endpoint allocations through the entire callback lifetime.
Registration takes the registry mutex only off audio and creates the runtime lease and endpoint allocations.
Audio code never looks up a hub through a registry mutex or clones/drops the last arena reference.
Each instance constructs its attachment mailboxes and local performance/lifetime storage off audio, before pairing, and retains them while active and disconnected.
Hub registration, tuner registration and pairing edits run the same off-thread registry match:
exactly one compatible candidate produces a provisional offer;
missing/ambiguous matches remain visible and pending.
Adoption is not musical enrollment:
the audio owners still require that hub's healthy progress and validated mapping before opening membership.
An offer can therefore be prepared before the hub's first callback, without requiring a background worker to notice that callback later.
Thus a tuner activated before its hub can receive an offer later without reopening its editor or relying on another activation call.

An offer moves a preallocated endpoint bundle and immutable lease identity into a typed owned attachment slot using the ownership states above.
This ownership-transfer control is separate from the ordinary `Copy` musical messages:
endpoints may be moved, but never cloned, allocated, replaced by dropping, or destroyed on audio.
The registry pins the underlying arena before publishing Ready and cannot reclaim it on a timeout.
At most one outstanding offer per instance may be accepted;
the second slot does not authorize overlapping endpoint consumers.
Stale, canceled or now-ambiguous offers return through the same owned return path without being opened for musical work;
a full return slot keeps the offer pinned rather than dropping its endpoint bundle on audio.
At an enclosing callback boundary with no old lease, the instance acquires one Ready offer once, validates its offer generation/identity, moves the endpoints into its callback owner, and publishes `AdoptAck` through its dedicated return slot for registry ownership bookkeeping.
The source separately sends its matching adoption/coverage frame to the hub through the source-owned control lane;
the hub does not wait for the registry to consume `AdoptAck`.
It begins new continuous coverage and the baseline/pending-manifest protocol;
musical membership opens only after the hub acknowledges that boundary.
Pre-join buffered attacks retain their original input metadata and are admitted through this fresh boundary, never represented as complete historical coverage.
No background acknowledgement is needed on the subsequent musical decision path.

For an already-paired active tuner, a new pairing first establishes a local input cut:
buffer new attacks for the proposed lease while finishing old requests, held releases/expression, and outcome obligations on the old lease.
Do not send an old reply or completion to the proposed hub, cancel old requests merely to switch, or move live voice credits between sessions.
After the old lease reaches safe idle and its audio-owned output-retention and plan cuts settle, the hub fences/detaches it.
The source returns its endpoint bundle through an owned return slot at an enclosing completion boundary and acknowledges that it will never access it again.
Only then may the source adopt the new offer;
a full return/ack slot retains old ownership and postpones adoption.
If an old obligation cannot finish, the transition remains visibly pending until valid recovery or explicit Reset/termination, while required storage fits.
Local Stop/emergency inhibition survives adoption.
This path changes membership without changing the instance's serialized callback ownership or freeing any endpoint on audio.

Unregister publishes lease withdrawal and stops new use.
Endpoints return only after either this callback-boundary detach acknowledgement or host deactivation joining that instance's callbacks, and the hub's own detach acknowledgement.
The registry destroys the arena only after both sides quiesce, outside audio.
If a callback or acknowledgement never arrives, keep a bounded retired arena pinned and visibly refuse additional registration when registry storage is full;
do not guess quiescence from elapsed time.
Use `REGISTRY_SESSIONS = 4` and `REGISTRY_TUNER_LEASES = 64` per process, counting active and retired entries together.
This is an initial implementation ceiling, not cross-process discovery or an additional hosting mode.
Active, unpaired instances also occupy these registry entries;
their preallocated local storage is counted once in the same bounded storage budget, not duplicated on adoption.
Attachment slots remain instance-owned until host callback quiescence, even after the last session lease retires.

The following are maximum work slices per **enclosing CLAP callback**, shared by all Rust sub-blocks:

| Work counter | Ceiling | Exhausted slice |
| --- | ---: | --- |
| `INPUT_SCAN` | 2,048 host events per instance | Reject the over-limit callback visibly and enter emergency; never publish coverage for uninspected input |
| `SOURCE_TRANSFER` | 512 intent pushes, 512 reply pops, 512 outcome pushes | Retain cursors and remaining data |
| `HUB_INPUT_WORK` / `HUB_OUTPUT_WORK` | 4,096 each | Fair rotating source scan, at most 256 items per source row per pass |
| `HUB_EVENT_WORK` / `POLICY_CALLS` | 1,024 / 256 | Retain unfinished complete cohort and evaluation cursor; no later cohort passes it |
| `SOURCE_SCHEDULE_WORK` | 2,048 event/index visits | Continue from bounded ready structures; do not scan all 8,192 slots for each reply or release |
| `NORMAL_OUTPUT_ATTEMPTS` | 512 CLAP events | Note-on plus initial tuning consumes two; keep unattempted work buffered |
| `EMERGENCY_OUTPUT_ATTEMPTS` | 128 CLAP events | Reserved in addition to normal attempts; retry unfinished cells next callback |
| `RECOVERY_WORK` | 256 ledger/manifest/history entries | Continue cancellation/rebuild with a stored cursor |
| `CONTROL_WORK` | 2 frames per direction per source; 16 config commands | Leave frames/commands owned and pending |
| `ATTACHMENT_WORK` | 1 offer or detach attempt per instance | Keep ownership and retry at a later enclosing boundary; no polling loop or audio-thread registry lookup |

At most 17 queue heads participate in each chronological merge.
Collect into per-source windows, then a fixed cohort scratch array;
do not sort or copy the entire pending/plan capacity each callback.
The complete cohort must fit before its first policy call;
more than 1,024 same-sample events or 256 onsets is actual cohort-storage/admission exhaustion, even when they could have been played sequentially within one sample.
Persist an unfinished cohort and its graph cursor rather than allocate larger scratch.
Use per-lifetime event links and a bounded ready heap/index for established voices so a blocked attack does not hide their releases.
Heap operations are bounded by `log2(8192) = 13` levels;
initialization, clear and suffix recovery also run under counters, not unbounded `retain`, sort or map walks.

The selected work envelope is deliberately distinct from the retained-storage envelope.
It may produce lateness below storage capacity under a dense workload;
that is diagnosed and buffered, never silently interpreted as permission to cancel.
#621 still owes its candidate/domain constants and measured worst-case policy cost.
Reserve 4 MiB per session for that bounded policy scratch and refuse activation if its declared implementation does not fit;
this reservation does not select musical search bounds or establish that 256 worst-case calls fit a callback.

## 4. Clocks, coverage and membership

For source `s`, preserve all four times:

```text
raw_input       = enclosing_steady_time + sub_block_start + event_offset
mapped_input    = raw_input + calibrated_offset[s]
mapped_deadline = mapped_input + D
raw_deadline    = mapped_deadline - calibrated_offset[s] = raw_input + D
mapped_actual   = actual_enclosing_steady_time + actual_output_offset + calibrated_offset[s]
```

Use checked signed `i64` sample arithmetic and the fixed activation sample rate.
Negative/absent steady time, overflow, an in-activation gap/rewind, unsupported callback size or invalidated calibration suspends new sequencing and requests recovery.
Do not substitute song position, framework-extrapolated stopped transport, wall time, independent process-call counters or the hub's drain time.
Loop wraps and observed idle seeks can move song position while this clock stays continuous.

An input progress frame contains `(lease, epoch, membership_revision, coverage_start, through, input_cut)`.
It proves every input before exclusive `through` has been inspected and every required intent through `input_cut` is published.
The hub uses it only after collecting through that cut.
Contiguous frames extend `[coverage_start, through)` only when the next interval begins at the previous end.
The first frame after reactivation starts new coverage;
its large endpoint cannot fill the preceding absence.
Queue backpressure may delay a progress frame but cannot erase an unreported event.
An owned latest frame may cover several contiguous callbacks, provided its cut includes all their events.

One hub-owned membership revision is copied at the start of a merge pass and used for both collection and completeness.
For that set, finalize sample `t` only when `max(coverage_start) <= t < min(through)` and every relevant cut has been collected.
Off's context withdrawal and an instance's deactivation do not mutate this set halfway through the pass.
Enrollment/removal creates a future acknowledged boundary;
it cannot make an old unresolved cohort complete by deleting its missing contributor retrospectively.
Source disappearance or input before a returning member's coverage suspends that old sequencing history and enters recovery.
Keep surviving unsounded requests for explicit resubmission;
do not restamp them as fresh historical evidence.

Output progress has its own `(coverage_start, through, output_cut)` and membership revision.
It advances only after wrapper output attempts settle and all accepted deltas through the cut are retained/published.
`Plugin::process()` return and `ProcessTrace::SubBlockExit` are too early in the present wrapper.
An unresolved attack can coexist with advancing actual output progress:
it has not emitted an event, and its eventual legal actual time must be at or after the then-current output frontier.
Never backdate that late event.
An accepted report behind a previously committed output frontier is loss/protocol failure, not something to relabel.

Use `HEALTH_LAG = 4 * negotiated_max_frames` of mapped hub progress as a visible missing-progress threshold.
Every silent participating source still publishes progress.
The first unsupported gap or incarnation mismatch invalidates completeness immediately;
the threshold is only an activity diagnostic, never proof of silence, permission to remove membership or a note-drop timeout.
If all callbacks stop, no wall-clock worker is required to advance musical state.
The measured initial support requires continuous callbacks and fixed revalidated routing.

### The 512-sample calibrated boundary risk

This is an analytical counterexample, **not a newly executed host measurement**.
Let all raw callbacks be `[0,512)`, `[512,1024)`, with B offset +64 and A/C offset zero.
B's raw event 511 maps to 575 while A/C only prove exclusive progress 512. The hub must wait for the next A/C interval.
If B drains replies in its `[512,1024)` callback before those intervals and the hub's subsequent work, the earliest B callback that sees the reply starts at raw 1024. The physical deadline is raw `511 + 512 = 1023`, so the attack is at least one sample late.
Raw offsets `448..511` map beyond that first frontier and can be `64..1` samples late under the same legal callback order.
Adding the calibration offset to the physical delay would conceal the failure by changing the promised D.

[Trial 24](tuning-probe-bitwig.md#same-key-retrigger-and-isolated-vst3-compensation) measured this extra-callback shape at a 1,024-sample buffer and D = 2,048, including a minimum 1,021-sample margin;
it did not test D = 512. Production must exercise equal offsets at raw 0 and 511, then B +64 at raw 447, 448 and 511 with B's next drain before the hub, including all source callback permutations.
Assert exact physical deadlines and actual `try_push` times, not only eventual reply delivery or mapped cohort equality.
Run the same boundary topology in Bitwig with the editor closed, then the selected maximum-workload case in live and offline execution.
If D = 512 fails, report the observed order, workload and lateness before proposing a changed delay or supported routing restriction.
This stage neither declares the candidate reliable nor silently raises it.

## 5. Configuration and canonical input order

Extract the pure resolver from `harmonigraph-ui::begin_frame`, including `Comma::ALL` order, engage-only auto detection, explicit derived-axis unlock and the raw-axis `judged_axes` keys.
Its cache key includes only the raw axes each comma judges, not camera, tolerance, unrelated axes or a freshly minted frame revision.
Carry the judged state in the audio owner so unchanged host parameter values cannot undo a pending explicit unlock.
The renderer/UI consumes the resolved revision and no longer supplies the effective musical authority.

Main-thread restore and UI commands use one serialized off-thread producer of bounded `ConfigCommand` values.
Decode saved data off audio, publish complete fixed musical settings in an owned slot, and retain the slot until the audio owner acknowledges it.
Host automation enters through the hub's enclosing/sub-block boundary with sample and order metadata.
Do not assemble a coherent setting by loading unrelated parameter atomics during a policy call.
The audio owner consumes a complete command/automation batch and resolves it into one value containing origin/axes, tempered flags, policy version, domain/radii and weights.
The entire `ResolvedConfig` is limited to 128 bytes and copied into each bound request, so a bound request needs no pinned-revision list.
Unbound requests still require the intervening configuration timeline;
copying bound configurations does not bound that timeline.

The hub owns `CONFIG_TIMELINE = 128` fixed 256-byte marker slots, separate from `CONFIG_COMMANDS` and the restore slots.
Each marker retains its sample, command order, revision and complete resolved configuration until retired.
Reserve a timeline slot before acknowledging/moving a command or restore snapshot out of its handoff storage, or before accepting a sample-timed automation batch as represented.
One retained marker is charged for each distinct configuration command, including ordered changes at the same sample;
do not silently merge intermediate automation values.
If the pool is full, a producer that still owns its command can wait in its bounded command/restore slot.
If a required host automation marker cannot be retained, latch session configuration-storage exhaustion and enter the explicit emergency/reset path;
neither overwrite the oldest marker nor advance input completeness past the unrepresented change.

Insert a configuration marker into the input timeline at a sample not yet finalized:
`effective_at = max(observed_mapped_sample, finalized_input_exclusive)`.
At a given sample, apply the ordered configuration commands before that sample's cohort, then freeze the revision for the whole cohort.
Commands observed after cohort evaluation begins take effect at the next unstarted sample boundary;
never interrupt a partially evaluated cohort with mixed axes.
Record the actual effective boundary in the UI/take control stream.
Restoration uses the same protocol, including with the editor never opened.
Excess config commands retain/backpressure at the producer where possible;
if required sample-timed automation cannot be represented, latch the configuration/protocol fault instead of silently coalescing semantically distinct edits.

The audio owner keeps one fixed current resolved configuration plus the bounded ordered future-marker pool.
Retire a marker only after its entire sample/cohort is finalized, the current configuration incorporates that marker in command order, and every request evaluated under it has received its own configuration copy.
A blocked input frontier keeps later markers charged even after the incoming command queue was drained.
Requests arriving behind a finalized frontier use the explicit recovery/resubmission boundary;
they cannot demand an already-retired historical configuration as though their old interval were complete.
Timeline insertion, application and retirement share the 16-command `CONTROL_WORK` allowance;
exhaustion of that work slice retains slots/cursors and does not discard an intermediate revision.

Initial request binding occurs at canonical evaluation, not at tuner callback arrival.
Rescheduling that request retains its bound revision and configuration copy, including across Off or later edits.
Unbound requests use the configuration timeline at their sequencing boundary.
Revision changes clear transient policy history, never held offsets.
Context carries current accepted pitch and optional attack node/revision separately;
#621 projects current pitch under the request's configuration without overwriting the actual pitch.

Build one dependency graph for every complete same-sample cohort:

1. Resolve event addresses in original source sequence. Add edges for the same lifetime, old release before same-key replacement, and shared-channel MIDI/pedal dependencies. Wildcards carry the resolved target set. Input order remains authoritative for these edges.
2. For an attack followed by same-sample initial player tuning, capture that initial value for the injected onset expression. Keep the original tuning messages in source order; subsequent values at that same sample remain subsequent values. Do not borrow expression from a different lifetime or from a later sample.
3. Choose among ready non-onset events first, ordered by source ordinal and input sequence. Apply an established voice's release/choke or pitch change to prospective context before independent ready attacks. A dependent off after an on stays after that on, including zero-duration notes.
4. Choose ready independent onsets by `(key, channel, source_ordinal, input_sequence)`, call the policy once, and incorporate its prospective assignment/history before the next ready event. If a dependency makes an onset not ready, it cannot win merely through a smaller key.

This is a bounded topological traversal, not a global sort that moves every note-off before every note-on.
Healthy downstream output retains original per-source order at input + D, adding only the required initial tuning directly after each note-on.
The hub's canonical evaluation order for independent attacks need not be the source's host-output order for those independent attacks.
The artificial stage #616 policy must observe its preceding cross-source assignment;
the D/F/A musical fixture and scoring constants remain #621 work.

## 6. The production host-acceptance boundary

Inspected current code:
[`WrapperProcessContext::send_event`](../vendor/nice-plug/src/wrapper/clap/context.rs) pushes into a `VecDeque` initially sized 512, with no bounded admission result.
[`Wrapper::handle_out_events`](../vendor/nice-plug/src/wrapper/clap/wrapper.rs) later pops events, calls host `try_push`, and continues even after rejection.
`TracedOutput::push` reports the actual boolean through `ProcessTrace::Output` after that call.
The [probe tuner](../crates/harmonigraph-plugin/src/probe/tuner.rs) mutates held state and pops its pending event immediately after `send_event`, before acceptance.
Its trace catches rejection, but its state machine is not a production acceptance implementation.

Add a production opt-in CLAP emitter boundary to the vendored wrapper before #617 claims truthful aggregation.
It must control ordered submission and completion, not merely observe the old drainer.
Stage fixed output commands with an attempt token, addressed lifetime, plan generation, output sample and kind.
Admission returns a bounded result;
the plugin retains the underlying performance event until terminal completion.
Stage at most 512 normal plus 128 reserved emergency attempts per enclosing callback in fixed arrays.
Do not use the legacy growing output deque for the companion's path.
Account for raw input staging as well:
the current 512-capacity input deque is also not an enforced bound.
Inspect host event count and use preallocated bounded conversion storage before the old deque can grow.

The emitter serializes a source's attempts in nondecreasing enclosing-callback output offset.
Immediately before each independent event or onset group, check the source control generation and eligibility, with the in-flight protocol in section 7. An onset group's permit spans both note-on and initial tuning;
closing its gate after an accepted note-on must not itself suppress that group's tuning.
Record the result into a pre-reserved completion cell before moving to another event.
On rejection, stop the dependent normal chain, suppress unattempted tuning/attacks, and permit emergency work at the earliest offset that does not go backwards in the current output list.
If the wrapper has already sent a future-offset event, an emergency cannot claim sample zero in that same callback;
it uses that legal current cursor or the next callback.
At enclosing exit, deliver bounded completions to the owner and publish actual output progress.
Do not re-enter plugin code under a lock it already owns, or ask a trace hook to call the host.
The existing trace hook remains useful for validation of the new boundary.

Reserve journal space for every possible accepted normal event before staging any of its attempts.
A note-on bundle reserves two cells and a voice credit.
A release additionally has its independent emergency cell available before its original attack can be emitted.
One accepted event creates one immutable actual record with exact mapped time, host address and value;
one rejected attempt changes no accepted value.
Returned attempt tokens distinguish retries from duplicate delivery.
The local journal is retained through `OutputRetainedAck(lease, H)`, which proves the hub's audio owner has copied every actual record through H into its bounded output window or already completed its canonical publication attempt.
That acknowledgement transfers retention responsibility for full records;
a held-state baseline alone never authorizes it.
Track current-state application and canonical-history publication with separate source sequence cursors, so applying a baseline cannot skip the historical on/tuning/off stream.
Transfer-ring fullness leaves source ownership intact;
duplicate retransmission is suppressed by the hub's received/published sequence cuts.
No credits means backpressure while retention fits, then explicit exhaustion;
never submit first and hope the reporting path has space later.

The hub retains each record in its `OUTPUT_WINDOW` until its ordered canonical publication attempt into `PUBLICATION_RING`, even when a baseline has superseded its current-state effect.
A successful publication hands that exact event/time to the non-RT history/take drainer once, which then fans out to display and recording.
Neither receipt acknowledgement, baseline acceptance, musical recovery nor voice-credit release waits for GUI rendering, background consumption or disk serialization.
If the publication ring is full, latch a reporting-loss diagnostic with the affected sequence range, mark an active take incomplete/failed, and later publish the explicit gap/control boundary before claiming subsequent continuity.
That actual downstream publication failure permits bounded retirement after the failed attempt;
it does not cancel notes, invalidate correct audio-owned context or masquerade as a successful take.
Slow or closed displays can be restored by a current-state baseline independently.
Baseline recovery itself is never a reason to skip an available actual record or manufacture a history gap.

[CLAP's output list](https://github.com/free-audio/clap/blob/main/include/clap/events.h) accepts one event per `try_push` and has no capacity reservation or atomic note-on/tuning transaction.
The exact consequences are:

| Results | Required outcome |
| --- | --- |
| Note-on rejected | No held voice is confirmed. Suppress its unattempted tuning and dependent normal output, report failure and retain the exact unsounded attack for a legal retry after plan validation, unless an explicit cancellation applies. |
| Note-on accepted, tuning accepted | Confirm the onset and its accepted tuning, with frozen adaptive offset and initial player value. |
| Note-on accepted, tuning rejected | Record an actual onset with unestablished intended tuning, latch partial-output fault, inhibit new attacks and request immediate emergency release/choke. Never record the intended correction as accepted. |
| Later expression rejected | Keep the preceding accepted pitch as truth; inhibit dependent plans and enter output-fault containment. |
| Release/choke rejected | Voice remains an outstanding downstream obligation. Retain its identity/credit and retry through the emergency path. |

Partial note-on acceptance cannot be rolled back and may be audible at unintended pitch before termination.
It is a protocol fault outside supported normal operation, **not an allowed unretuned fallback**.
Do not retry tuning on that sounding voice as reconciliation, resend its note-on, or claim pair atomicity.
An accepted prefix followed by rejection invokes explicit visible output-fault containment even when storage remains available;
a rejected note-on with no accepted prefix remains an unsounded request.
Neither case is an ordinary assignment deadline miss.
The supported host premise must be validated:
at the declared workload, Bitwig accepts both onset events at their exact common sample and the configured destinations honor expression.
No finite buffer, ordering strategy or timeout can guarantee accepted emergency release against a host that rejects forever.
Remain latched with an honest release obligation until acceptance or an established external termination boundary.

Actual reports/takes retain the partial-output fault and discontinuity as well as each accepted event.
The downstream instrument's private default pitch and acoustic voice lifetime are not inferred from a failed tuning event.
Even two accepted events establish protocol delivery, not first-oscillator-sample pitch behavior.

## 7. Prospective state, revocation and in-flight truth

Maintain a confirmed model from accepted output and a separate prospective model from ordered input and issued plans.
History has separate confirmed and prospective entries with decision serials.
Normal release removes context at its scheduled point;
actual release removes it from confirmed held state only on acceptance.
GUI-ring loss cannot change either audio-owned model.
The hub does not confuse receiving an assignment, staging a command and acceptance by the host.

Every source request follows these phases:

```text
Unbound -> Waiting -> Planned(generation) -> Staged -> InFlight
                                                   -> Settled(accepted outcomes)
                                                   -> Settled(rejected/unattempted)
Planned/Staged -> RevokedUnemitted -> Waiting(new generation)
Waiting/Planned/Staged -> CanceledUnemitted         [authorized cancellation only]
```

`InFlight` is source-owned and begins before the emitter's final generation check/host call.
It ends only after recording all actually attempted results and suppressing the remainder of its command bundle.
The hub cannot CAS a request to "never sounded" from another thread.
A single Acquire read of a revocation flag before enqueue is insufficient:
revocation can arrive between that read and `try_push`.

Use one per-source `AtomicU64` emission gate packing a checked emission generation into 62 bits plus CLOSED and BUSY bits.
The hub is the sole writer of OPEN/CLOSED and the generation.
Immediately before a real onset group, the emitter makes one `compare_exchange` from `OPEN(g), idle` to `OPEN(g), BUSY` (`AcqRel` success, `Acquire` failure).
A failed claim leaves the original attack pending;
no retry loop runs in that callback.
The hub closes with `fetch_or(CLOSED, AcqRel)`, preserving BUSY.
If close wins, no old group can claim;
if claim wins, the group may finish note-on and its same-sample tuning even after close, with every accepted prefix recorded.
Explicit Stop/Reset/emergency then adds termination debt for any accepted onset in its scope.
The emitter stores the durable terminal result before `fetch_and(!BUSY, Release)`, preserving CLOSED.
Held releases/expression have an independent essential path and do not require an open attack gate.

Gate emission generations are distinct from each request's replaceable plan generation.
One fence per source is active at a time;
its immutable command slot cannot be overwritten before acknowledgement.
New invalidations are retained in hub-owned pending control state and prevent reopening until they too are applied.
The hub reopens only with one CAS against the exact CLOSED, idle generation whose cuts it consumed, incrementing the generation in that CAS.
The source independently inhibits new attacks immediately on local Stop/emergency, before requesting a hub fence;
it does not change the gate's control bits and a stale hub reopen cannot clear that local inhibition.
No second writer can issue an unversioned close which a racing reopen would erase.
If later implementation adds another control writer, it must version every newer close, including one already CLOSED, and re-prove the handshake.

Use this cancellation acknowledgement sequence:

1. The hub stops issuing affected plans, closes the emission gate, and publishes a command frame describing the fenced emission generation, affected decision-serial suffix, reason and desired disposition. All new staging observes the fence. Repeated invalidations widen the pending suffix monotonically while the first immutable transaction settles.
2. A source observes the fence with Acquire, inhibits staging in that suffix, and asks the emitter to suppress every unclaimed affected command. An already-claimed onset group finishes its permitted host calls; their results are irrevocable truth. A bundle already InFlight may have a partially accepted prefix if the host rejects its tuning.
3. At an enclosing output-completion boundary, the source has no affected InFlight commands. It publishes `RevokeAck(recovery_id, generation, input_cut, output_cut, settled_attempt_cut)` and a bounded request-disposition manifest. Accepted output through `output_cut` is retained in the ordinary journal or emergency ledger. The acknowledgement never means the host undid output.
4. The hub collects actual output through that cut, or a complete authoritative baseline for current state plus the complete request-disposition manifest for request outcomes. It classifies each request as accepted/partially accepted, retained unsounded or explicitly canceled. Only then may it reclaim the old plan, rebuild context or issue a replacement generation for an unsounded request. A baseline does not replace retained actual-history records: they still transfer to the hub and receive ordered canonical publication under the separate retention acknowledgement in section 6.

There is no cross-instance atomic instant that can recall a note already accepted by the host.
The cut and handshake bound what will no longer be emitted, while preserving racing acceptance as fact.
Missing callbacks leave the transaction incomplete;
they do not count as an acknowledgement.
Epoch/membership change alone cannot free old in-flight or held state.
Reject assignment replies from revoked generations, but still accept actual outcomes from closing generations until their acknowledged output cuts are collected.
One generic stale-epoch rejection rule cannot serve both message classes.
An old-generation accepted onset discovered after Reset creates a factual onset and release debt, never a canceled-plan fiction.
Accepted future-timestamp output is also irrevocable even if its acoustic onset has not occurred yet.
`RevokeAck` proves closed emission rights and settled attempts through its cuts;
it is distinct from `ReleaseComplete`, which requires accepted termination/neutralization or an established host termination boundary.
A fence can acknowledge a partial-onset release debt while that termination remains unresolved and recovery/admission stays inhibited.

On a missed deadline, changed actual onset/release/expression time, unexpected acceptance failure, source loss, Off withdrawal or configuration-context recovery, conservatively revoke the **entire unretired prospective suffix** from the earliest affected decision serial across all sources.
Do not attempt an unbounded per-note dependency DAG or silently leave later decisions based on lifetimes that no longer occurred as planned.
Freeze assignment publication while the suffix settles;
other sources' already-sounding lifetimes and unaffected ready output continue.
An accepted racing successor stays fixed even if its originally assumed predecessor was revoked.
Mark the divergence;
never retune it to make the original chord plan true.

Rebuild prospective context from the acknowledged actual baseline and completed output frontier, clear speculative history, and resubmit retained unsounded requests in original input order with an explicit new sequencing boundary.
Preserve original input time, duration, configuration binding and request identity as metadata;
give new plans a new generation and planned output times under section 8. The recovery boundary does not pretend the missing historical input interval was complete.
Accepted history may be cleared on this lifecycle-loss recovery as the product design permits.
Clear/rebuild the direct-index arrays with a 256-entry cursor or a generation-tagged validity field, never a racing whole-array write.
Already-issued configurations stored by value remain available even after history is cleared.

No decision can use an unacknowledged replacement baseline, an unresolved in-flight predecessor as though canceled, or an unplayed plan as actual output.
While those facts are unresolved, stop new policy work and retain attacks.
This may add latency on dependent sources;
it does not create a cross-track barrier delaying existing releases.

## 8. Late-stream scheduling and shared controls

Healthy operation schedules every supported performance event at raw input + D with original source order.
On a missed assignment, retain the unsounded attack, latch the timing failure once for its plan, and obtain a still-valid assignment or a newly acknowledged replacement plan.
Never discard a valid answer solely because its deadline passed.

At a failure boundary, separate already-sounding lifetimes from the retained unsounded stream.
Each accepted lifetime owns an immutable translation `shift = actual_onset - raw_input_onset - D`.
Its later addressed expression, note-off and choke use that same translation, even if an unrelated new attack is blocked.
They are reachable through per-lifetime ready indices, independently of the pending-stream head.
If a callback/host failure makes one of these events itself late, emit at its earliest legal opportunity, record that actual divergence and recover prospective state;
do not blame another pending attack or silently change the lifetime's nominal shift.

The unsounded stream uses ordered translation waves.
At a ready wave's head, choose `wave_shift >= previous_stream_shift` large enough for its first attack to reach the earliest legal output opportunity, including reply, revocation, channel and output-credit readiness.
Translate its still-unplayed events together by that amount.
Once any attack in the wave sounds, freeze that lifetime's shift.
If a later attack still misses, start a successor wave for only the still-unsounded portion;
established lifetimes keep their old shifts and releases remain reachable.
Later attacks do not reduce shift to catch up.
Retain at most one wave descriptor per pending lifetime, included in the lifetime pool.

For a note whose release was received while it waited, output release remains `accepted_onset + (raw_input_release - raw_input_onset)`.
The same relationship applies to its addressed expression points.
A positive played duration must not collapse to zero because several queued events are now overdue.
An originally zero-duration input retains ordered on/tuning/off at one sample.
Subsequent attacks retain their original spacing within a wave.
Only an explicit Stop/Reset, output-fault containment or storage emergency cancels this preservation obligation.

Shared-channel events cannot always be translated independently without changing an established voice:
pedals, channel bend/pressure, program changes and other channel MIDI affect both old and future voices.
Choose this conservative dependency rule:

- Deliver a channel event needed by the established wave at that wave's existing shift. In particular, a pedal-up or channel release/choke must not wait for an unrelated pending attack.
- Keep its chronological value/history for the unsounded wave as well, charging the retained event and any replay reference to `PENDING_EVENTS`. Do not send that history a second time while an established voice on that channel could be affected by the replay.
- Before EVERY differently shifted wave starts on a channel, drain older held lifetimes, in-flight output and channel obligations and reach accepted neutral sustain/sostenuto/hold state. This gate applies even when no controller event is currently queued: a future channel event could otherwise affect two sounding waves that require different times. Then emit the retained channel setup/history in translated order for the new wave. Other channels' established voices may continue.
- A same-key retrigger additionally waits for accepted termination of the previous addressed lifetime. Wildcard release/choke is resolved to the input-time target set and may use per-lifetime output releases to avoid killing a later wave.

At most one established translation is allowed per channel at a time.
Admission to a channel with that same translation remains possible under the ordinary lifecycle/credit rules.
A successor wave must not overlap its older wave merely because their keys differ or the current controller queue is empty.
Choose or increase the unsounded wave's shift only when its channel gate opens, and keep its retained note durations and controller relationships intact.
This makes the channel's drain/neutral boundary an explicit dependency gate.
A long held pedal or old voice can make a ready late attack wait longer;
that is preferable to changing its captured gestures or postponing old releases.
It is not a time-based cancellation rule.
Do not duplicate a shared CC into a still-sounding older wave, zero a user's pedal merely to accelerate a late attack, or burst an accumulated phrase at one output sample.
Directly addressed poly-expression can cross the pending head once its own onset is accepted.
Unrelated non-channel short MIDI retains source order in the unsounded stream.
These are the documented exceptions to original source order during failure;
healthy operation introduces none of them.

For example, shifts zero and +1,000 cannot coexist on one channel and share a subsequently received pedal-up without changing one wave's gesture.
Sending the pedal-up once at the older wave's due time would advance it relative to the younger wave's translated note-off;
sending it twice would affect the older wave again.
The admission gate prevents that state rather than inventing a shared-controller timing exception after both attacks have sounded.
If recovery discovers conflicting established translations, it cannot declare the channel recovered or retime a sounding voice;
require explicit termination/reset of the conflicting state.

Off changes only the adaptive requirement for newly received attacks;
it does not bypass this scheduling gate.
New Off notes retain zero correction but can inherit the fault's extra delay and wait behind older adaptive channel obligations, including pre-Off pending attacks that must still finish tuned.
Normal D while Off remains the healthy timing contract and resumes at the specified idle/recovery boundary, not at the moment Off is selected.

Return the source's stream shift to zero only with no retained performance events, no locally held or in-flight voices, neutral channel pedals, no unacknowledged output/revocation obligations, and an accepted fresh sequencing boundary.
Continuous playing may never reach that boundary.
Report the current extra delay and failure state without promising a catch-up time or hiding it in the fixed latency report.

## 9. Stop, Off and emergency completion

Transport Stop is a `playing=true -> false` transition, not every callback with `playing=false`.
Record a run identifier and the first local Stop edge once per source run.
At that source's observed edge, close its pre-Stop input sequence, immediately inhibit/cancel those unsounded lifetimes and schedule undelayed emergency releases for its old held set.
Read callback-start transport before input events, so new stopped live input at the boundary belongs to the new local run and remains queued for normal D.
Nonzero-offset transport must be ordered with raw input by the boundary hook before it can be supported.
A loop wrap with playing still true is not Stop;
host-delivered note-offs around the loop retain their actual input order.

The hub correlates local Stop markers by the current acknowledged run, **not identical raw sample observations**.
Source and hub transport edges can be independently compensated.
It publishes a session Stop generation and gathers each source's local cut/acknowledgement.
A source that already observed the edge acknowledges without canceling newly received stopped input again.
A source receiving the global marker before its own edge inhibits old-run output, retains subsequent input until the local edge identifies the cut, then classifies new stopped input once.
If that edge is not observed, remain visibly unresolved;
do not infer a cut from an elapsed timeout.
Hub direct input follows the same run/cut rule.
After all old obligations settle, establish the new sequencing boundary and run new stopped live input at D.
This delayed-observation case requires a production fixture;
trial 15 only measured Stop after a late attack sounded.

User Reset has explicit scope, creates its own cut on each source's observation, cancels older pending requests and uses the same termination/acknowledgement machinery.
Emergency stop and output fault additionally latch rejection of new attacks until explicit Reset and valid recovery.
New input after ordinary Stop is retained while recovery finishes;
being stopped does not turn adaptive participation Off.

Off records a source input-sequence boundary.
New attacks on or after it deliberately bind zero adaptive correction, while pre-Off requests retain `adaptive_required`, their configuration binding if already assigned, and their ordered obligations until tuned output or authorized cancellation.
The hub withdraws the source from future context/display at the corresponding sequencing marker and clears its history.
Keep a separate obligation membership for old requests and actual completion, including processing/progress while Off.
Do not remove the source from older cohorts to make them complete.
Already-held voices continue combining player expression with their frozen offsets.
Actual Off output remains in the local held/journal state for correct rejoin, including zero-offset onsets;
Off's public visibility withdrawal is a control record, not a fictional downstream release.

Each admitted held voice has a dedicated emergency cell containing lease/lifetime, exact output address, latest accepted value, required termination, attempt identity and completion status.
Each channel has dedicated pedal/reset cells.
Emergency signaling is an atomic latch and its cells are filled by the source owner;
it never needs an ordinary ring slot or pending-event allocation.
At cancellation, suppress queued pedal-down and other canceled performance, send CC64/66/69 neutralization for used channels and addressed note-off/choke obligations, respecting the output cursor.
The 128 reserved attempts cover `64 voices + 3 * 16 pedal resets = 112` in a callback when the host accepts them, with room for explicit channel-reset commands.
Remember accepted pedals/controllers separately from desired resets.
The mutable release debt lives in the source-owned local voice/channel tables.
An emergency completion cell is a separately owned publication slot:
the source writes a terminal accepted result once, publishes Ready with Release, and does not mutate it until the hub consumes and acknowledges it.
While that slot is Ready/Reading, retry state stays in the local table and wrapper attempt storage, not in a concurrently read payload.
Control frames carry cuts and bitmaps identifying those cells, not a racing copy of the entire mutable ledger.
An unaccepted debt is signaled by a latched bit and remains local;
its terminal publication slot was reserved before the original onset.
An accepted note-off/choke may end the defined held interval, but reset completion still waits for all required channel resets;
instrument tails are not evidence of failed protocol termination.

Rejection leaves the emergency cell pending and retries on subsequent callbacks at the earliest legal offset.
The source publishes a cumulative emergency frame with generation, outstanding bitmap, accepted termination records/times and output cut;
retain each completed cell until the hub acknowledges retaining its full actual result for canonical publication, separately from baseline current-state acceptance.
New emergencies cannot overwrite unacknowledged cells.
During an emergency the source remains latched, so it does not reuse their voice credits for new attacks.
Once a source is stopped by capacity or output failure, selecting Off cannot bypass the latch.

No model declares a held voice gone merely because its pending queue was cleared or its release was staged.
Host bypass/removal only substitutes for accepted release under the specific established host termination boundary, retained as a scoped control record.
The measured mute/solo behavior preserves held state;
missing callbacks alone proves no termination.
An unknown held set requires explicit voice reset/validated host termination before rejoin, even if this keeps old registry storage pinned.

## 10. Baseline, cut, acknowledgement and pending disposition

Use one recovery transaction per source at a time.
Loss, reconnect, Off-to-Participating, calibration change and uncertain membership all inhibit new adaptive plan publication until this transaction completes.
Known held offsets remain fixed while it runs.
Ordinary lateness alone uses revocation/rescheduling;
it does not erase actual held state.

1. **Quiesce plans.** Publish recovery generation, settle/suppress InFlight work using section 7, and choose an actual output cut `C` at an enclosing completion boundary. Continue responsive held releases/expression, recording later deltas as `> C`. Reserve their journal capacity before output.
2. **Publish the complete held baseline.** The source fills one owned slot with count, identity tuple, coverage boundary, `C`, current channel state and every held voice (up to 64): original input onset, actual accepted onset, current accepted pitch/player expression, frozen offset, attack node/configuration, host address and release status. A partial-tuning voice carries its fault status rather than an invented pitch. Never overwrite the slot before acknowledgement.
3. **Admit and replace current state.** The hub validates the full frame, counts every global/local reservation, and atomically replaces only this source's confirmed set in its own callback. The baseline supersedes current-state application of deltas `<= C`, never their retained history. Keep/transfer all available actual records through C and attempt their ordered canonical publication exactly once before publishing this source's baseline control and later deltas. Only already-unrecoverable event loss or an actual downstream publication failure warrants a diagnostic gap; an undrained journal does not. Do not count old and replacement copies as extra voices, publish a partial set or return credits for unconfirmed downstream releases. The baseline/recovery control updates display/take state without re-emitting attacks.
4. **Acknowledge distinct cuts.** Publish `BaselineAck(recovery_id, lease, C, membership_revision)` for current-state replacement and release of the owned baseline slot. It does not retire journal records. Only `OutputRetainedAck(lease, H)` retires source actual records through H after full-record transfer to the hub's audio-owned retention or canonical publication attempt. Neither acknowledgement waits on a GUI/background/disk consumer; retained records remain the responsible audio owner's obligation. The second baseline slot is spare for ownership overlap, not permission to supersede an unacknowledged transaction.
5. **Dispose of pending requests separately.** At quiescence the source fixes a pending-manifest input cut and enumerates every pre-cut request in 64-entry chunks, with total count and monotonic chunk sequence. Each record states retained-unsounded, accepted/partial, or authorized-canceled, plus request/plan/configuration identity. Source events and request slots remain owned while chunking; new post-cut input uses remaining storage and waits behind the recovery boundary. The hub acknowledges the complete manifest, never just the last chunk or the held baseline.
6. **Resume.** The hub discards old unplayed plans only after disposition and output cuts settle. Retained unsounded requests are resubmitted with original input/dependency metadata, preserved bound configurations and new plan generations into the new continuous-coverage boundary. Later ordered deltas `> C` are merged exactly once before new context is used. Publish `RecoveryComplete` only after baseline, manifest, output and membership boundaries agree.

An empty held baseline cannot acknowledge an unsounded attack.
It also cannot erase a completed accepted lifetime:
note-on, tuning and note-off still retained in a source journal must reach the hub and ordered canonical publication with their original actual times, even when all three precede the empty baseline's cut.
Replayed copies are deduplicated by source incarnation and output sequence, not by whether their voice is currently held.
A source disappearing during recovery leaves its old transaction unresolved until a validated termination/cancellation boundary or the same retained owner returns.
A new incarnation cannot answer for it.
If current state is trustworthy but old output deltas were lost, a baseline repairs current context with an explicit recorded gap;
it cannot reconstruct an exact take of events that were not retained.
Under the normal credit/journal contract that loss should be unreachable, and the fixture must prove it.

## 11. Conservative byte budget

This table is **layout arithmetic**, not `size_of` output or an allocation measurement.
Use fixed records aligned to at most 8 bytes with explicit fields/validity flags;
do not assume Rust enum/`Option` layout has these sizes.
Implementation must add compile-time upper-bound size/alignment assertions for each production type and report executed sizes, arena allocation totals and high-water marks before wiring is called complete.
No padding bytes are serialized or read as initialized data.

| Budgeted record | Bytes | Intended contents |
| --- | ---: | --- |
| `EventSlot` / `Intent` / `OutputDelta` | 128 | Up to 64-byte checked identity/sequence/time envelope, 40-byte value event, 24-byte scheduling/link/cut data; endpoint binding may carry common identity |
| `ResolvedConfig` | 128 | Exact axes, tempered/auto state, policy version, fixed search/scoring constants and revision |
| `Lifetime` / `Plan` / `Reply` | 256 | Identity/times, plan phase, indices, pitch/node/history metadata, and full configuration copy |
| `VoiceBaseline` / emergency voice cell | 256 | Actual/source onset and accepted pitch, assignment identity/configuration, output address and completion metadata |
| `HistoryEntry` | 64 | Valid generation, node, configuration/decision identity and history provenance |
| `ControlFrame` / wrapper command | 256 | Owned control fields or one command plus immutable attempt identity; large baselines occupy their separately budgeted storage |

The event envelope is a packing budget, not a requirement to squeeze every time and identity into one C struct.
If the real types exceed any row, revise the table/ceiling before implementation proceeds;
do not truncate identifiers, exact pitch or timestamps to make a guessed size pass.

| Pool | Calculation | Bytes |
| --- | --- | ---: |
| Source performance retention | `16 * 8192 * 128` | 16,777,216 |
| Source lifetime/request retention | `16 * 8192 * 256` | 33,554,432 |
| Intent/reply/output rings | `16 * (1024*128 + 1024*256 + 2048*128)` | 10,485,760 |
| Source outcome journals | `16 * 4096 * 128` | 8,388,608 |
| Hub plan ledger | `131072 * 256` | 33,554,432 |
| Hub input and output windows | `17 * (1024 + 2048) * 128` | 6,684,672 |
| Canonical publication ring | `4096 * 128` | 524,288 |
| Confirmed/prospective history | `2 * 34816 * 64` | 4,456,448 |
| Local voice, emergency and two baseline sets | `17 * 4 * 64 * 256` | 1,114,112 |
| Hub confirmed/prospective voices | `2 * 256 * 256` | 131,072 |
| Pending manifest windows | `17 * 64 * 256` | 278,528 |
| Wrapper input and output arrays | `17 * (2048*128 + 640*256)` | 7,241,728 |
| Cohort scratch | `1024 * 256` | 262,144 |
| Configuration commands and restore slots | `128*256 + 2*256` | 33,280 |
| Retained configuration timeline | `128 * 256` | 32,768 |
| Control/channel frames | `17 * (4*256 + 16*256)` | 87,040 |
| Policy scratch reservation | `4 * 1024 * 1024` | 4,194,304 |

The calculated subtotal is 127,800,832 bytes (about 121.88 MiB).
Reserve another **16 MiB** for ring headers/endpoint owners, alignment, arena allocator overhead, free lists, dependency/ready indices, cohort edges and diagnostic counters.
That allowance also covers the fixed registry attachment slots:
at most `(64 tuner + 4 hub instances) * 4 slots * 256 bytes = 69,632 bytes` process-wide, charged once rather than duplicated on each adoption.
The selected **session ceiling is 144 MiB** of additional adaptive-engine storage, excluding the full plugin's existing analyzer/UI/take/audio storage and non-RT disk history.
Four active/retired session arenas reserve at most 576 MiB under the registry ceiling.
The implementation must measure those excluded integration increments too, not call this the plugin's whole resident memory.
Pools may be split into off-thread allocations to avoid giant stack construction;
no callback stack may contain a session-sized temporary.

At a source input rate of `r` retained performance events per second, the pending pool alone represents `8192 / r` seconds **from empty**, before journal/lifetime/cohort limits.
At 1,000 events/s this is 8.192 s;
at 10,000 events/s it is 0.8192 s.
Healthy D = 512 at 44.1 kHz accounts for roughly 12 or 117 events respectively before callback/burst padding.
These illustrative calculations are not an accepted event-rate bound or a promise to retain a late phrase for that duration.
Dense same-sample events, emitted history awaiting acknowledgement, shared-controller replay and suspended callbacks consume different pools.

## 12. Required production verification

The following fixtures belong in the implementing stages, using their actual constants and types.
This documentation change does not claim they ran:

| Fixture | Path it must actually reach |
| --- | --- |
| Accepted-output boundary | Reject the first note-on, then separately accept note-on/reject initial tuning, reject a later expression and reject a release repeatedly. Verify suppression, exact actual records, partial fault, independent release retries and no premature credit/state removal. |
| Revocation interleavings | Fence before claim and claim before fence, inside `try_push`, between onset/tuning and after acceptance before reporting. Race stale/duplicate fences and a new Stop against reopen; ack cuts include accepted prefixes, including old-generation output after Reset. Test A revoked after B accepts but before C claims: B remains frozen, C survives rescheduling. |
| Saturation | Fill each selected owner pool to its real limit, separately from filling only a transfer window. Keep preexisting voices and pedals active, force ordinary output/journal saturation, then show the independent latch, release and completion route still functions. |
| Admission | Reach 16 tuners, 64 per source and 256 session reservations, including Off and direct input; contend reservations, release/retrigger at one sample, and reject the actual excess without evicting another source or inventing termination. |
| Active lease adoption | Activate a tuner with the editor closed before creating/activating its hub, then adopt at a callback boundary and establish coverage/baseline without another tuner activation. Also change pairing while active: buffer post-cut attacks, finish old obligations, fill return slots, and prove no old endpoint is read or freed after the two audio owners detach. |
| Completeness | Same local offsets in different sub-blocks; future reports; absent silent participant; returning coverage after an older foreign request; deactivation during collection; output progress before and after wrapper completion. |
| Canonical order | Permute cross-source callbacks and reply drains with same-sample independent attacks, off/on/tuning/off, wildcard release/retrigger, pedals and zero-duration notes. Artificial policy result must depend on its preceding assignment. |
| Late waves | Retain a positive-duration note's expression/release before its late reply; inject another blocked attack while an older voice sounds; verify old release timing, no burst, immutable shifts and safe idle return. With no controller queued at admission, request a +1,000-shift wave on a channel held at shift zero, then deliver pedal-up/channel bend only after the second attack would have sounded without the gate. Prove the second attack stayed pending, old events remain responsive and translated younger gestures are preserved. Include new zero-correction Off notes on that faulty channel. |
| Prospective divergence | A predecessor emits late or is canceled while successors are planned, including one accepted during revocation. Rebuild unplayed suffix from actual state; accepted offsets never change. |
| Stop and Off | Stop before reply and after late onset, independently delayed hub/source Stop observations, new stopped live input before global acknowledgement, pedals and loop wrap. Turn Off with held, bound-pending and unbound-pending adaptive requests; new Off attacks are the only intentional zero assignments. Also stop callbacks with release debt: no timeout may synthesize completion. |
| Baseline recovery | 64 held entries, including zero-offset Off voices, later `> C` expression/releases, more than 64 pending requests to force manifest chunking, occupied alternate slot, and disappearance before ack. Retain an entire accepted on/tuning/off lifetime in an undrained journal, then recover an empty held baseline: canonical replay receives all three exact events/times once, including under duplicate retransmission, and baseline ack alone frees no journal record. No partial replacement or unsounded acknowledgement. |
| Reporting independence | Stall the display/take drainer and actually fill `PUBLICATION_RING`; prove output-retention acknowledgement, baseline state recovery, voice-credit release and policy progress require no non-RT acknowledgement. Publication failure reports its sequence gap and incomplete take without changing correct audio-owned musical state. |
| Configuration without editor | Restore and automation with editor never opened; explicit unlock followed by stale raw parameter values; changes while a cohort is partly evaluated and old replies are pending; unrelated UI state never invalidates musical cache. |
| Retained configuration timeline | Block the input frontier while 128 distinct sample-timed markers occupy `CONFIG_TIMELINE`, even after the command queue drains. The 129th required automation marker takes the actual exhaustion path without overwriting an intermediate revision or advancing coverage. Separately release the frontier below capacity and verify each between-marker onset binds the correct revision before marker retirement. |
| Budget and real-time safety | Executed size/alignment/allocation totals, maximum depth/high-water counters, allocation/final-free guards through processing, publication, overflow and unregister, plus measured complete hub and source work at selected limits. |
| Delay and destinations | Section 4's exact D512 equal/calibrated boundary schedule, live/offline host validation and maximum central policy workload; exact accepted output and destination pitch behavior in the supported Bitwig configuration. |

Keep fixtures sized to their stated branch:
a four-note queue does not test an 8,192-slot emergency, a single pending request does not test chunked recovery, and reject-all output does not test partial onset acceptance.
Record executed measurements and any resulting capacity/host-support changes alongside these constants.
Use the full Actions gate after focused local checks.
The feature remains unfinished until #617, #616 and #621 implement and validate their respective contracts.
