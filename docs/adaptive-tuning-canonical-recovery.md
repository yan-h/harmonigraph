# Canonical recovery and recording closure for #617

This completes the consumer and publication foundation of [#617](https://github.com/yan-h/harmonigraph/issues/617).
The companion registry, Tune class, native controls, ordinary credited scheduling and automatic source resynchronization are still separate work.
The exact base is `ddeeb507199ef499639780c3d83a1c44dc4da9f6`, [the Stage 5 draft PR #639](https://github.com/yan-h/harmonigraph/pull/639), on `codex/616-clap-performance-adapter`.
Full CI run `33978379810` and Security run `33978379797` passed at that exact base before editing.
This branch is `codex/617-canonical-recovery` in the Codex-managed worktree `527d`;
its stacked PR targets the Stage 5 branch and stays draft and unmerged.
The exact head, PR and review results belong in the final handoff.

## One canonical consumer stream

`harmonigraph-core::canonical` supplies dependency-free `NoteDelta`, `SourceBaseline` and `PublicationGap` values.
A delta distinguishes observed direct input from accepted output and retains exact input/actual samples, optional planned samples, clock identity and lifetime identity beside presentation seconds.
The ordinary plugin direct path remains observed input;
forwarding through nice-plug does not establish host output acceptance.

A baseline contains all 64 possible voices and 16 channel states.
Construction and consumption validate the whole frame before changing any source.
Duplicate addresses/lifetimes, a 65th voice, invalid final entries and malformed channel state refuse the complete frame.
Stored voice facts include original input/actual onset, host address, current exact pitch/player expression, frozen correction, optional assignment/attack node and release/partial-output status.
An aggregation producer leaves assignment absent.
The tracker retains the full baseline metadata independently of its floating-point drawing adapter.

Per-source output and baseline cursors prevent duplicate retransmission from replaying attacks.
Available on/tuning/off history must precede the baseline cut;
a baseline never advances the historical output cursor to conceal retained history.
A new nonduplicate historical delta at or behind an already published baseline cut is invalid.
The producer must first publish all available history through the cut, then the baseline, then later deltas.

Matching lifetimes retain onset, bend history, settled pitch and held-end identity.
A replacement neither releases missing notes as though downstream accepted an Off nor invents earlier expression paths for newly recovered notes.
The roll records observation stopping separately from actual termination.
When a baseline resumes a known lifetime after a gap, it reuses the original roll row and marks the missing interval;
it does not create another source/onset cache identity.
Off visibility retains factual hidden lifetimes so later expressions and rejoin keep their original history.
Pitch history already accumulated from earlier visible observations remains historical evidence.
Eligibility for adding a released voice to pitch history is captured at its factual release, so later visibility changes and frame/prune cadence cannot erase it.
An accepted Off arriving after a gap retains its actual release time on the matching historical lifetime while its drawn trajectory still ends at the last observation.
At the 64-bend limit, folding preserves both boundaries of a missing interval;
when every retained point is a boundary, it conservatively forgets the oldest prefix instead of drawing across loss.

A gap makes affected source current state uncertain and remains in historical diagnostics after recovery.
A later On establishes only that individual lifetime.
Only an authoritative complete baseline or scoped reset restores that source's current certainty.
The shared UI displays a persistent missing-history warning.
One source's baseline cannot restore another source's certainty.

## Audio publication and non-RT ownership

The actual primary `PUBLICATION_RING` has 4,096 small cells.
Complete baseline payloads occupy a separate preallocated bank of 34 slots, two for each of 17 source rows.
A queue item carries a generation-tagged slot handle.
These are display/take publication slots, not the producer's musical recovery slots or acknowledgements.

The one publisher acquires Empty to Writing, copies the complete frame, and releases Ready before enqueueing its handle.
The one consumer acquires Ready to Reading and copies or serializes the whole payload before releasing Empty.
A deferred consumer returns false and retains both the queue item and its whole payload.
An unsuccessful enqueue reclaims only the publisher's untransferred slot.
A full pair returns `BaselineBusy` without overwriting either reader-owned frame or asserting an invented history loss.

The existing recording worker is the sole fanout consumer.
It serializes into the original addressed take while owning the primary payload and copies into a separate complete display payload bank.
No second producer writes the existing 65,536-cell audio recording ring.
The editor/background consumer receives the display queue;
shutdown performs a final background catch-up after its stop signal.
All these consumers are independent of audio-owned confirmation and learning state.
No GUI, background or disk acknowledgement decides musical retention, baseline acknowledgement, credit release or policy progress.

A failed ordinary enqueue records a serial range in a separate atomic loss descriptor.
The reader uses one bounded version check;
all descriptor payload words are atomic, so an interrupted read is retried on its next poll without racing ordinary memory.
The writer places a Release fence after publishing its odd version and before relaxed payload stores, paired with the reader's Acquire fence after payload loads.
If a reader sees any new payload word, those fences order the odd-version update before its final version load;
atomic coherence then forbids accepting the previous even version with mixed payload.
An initial Acquire load of the new final even version instead observes its preceding complete payload.
This uses Rust's [fence-to-fence synchronization rule](https://doc.rust-lang.org/std/sync/atomic/fn.fence.html).
A Loom model of the production version/payload ordering accepts a mixed range without the writer fence and passes with it.
The descriptor cannot overtake successful earlier queue items.
It delivers a gap even if the producer never calls again.
A resumed publisher queues the gap before claiming subsequent continuity.
Loss affecting recording writes a durable incomplete marker before dropping the open writers, preserves the readable prefix and suppresses automatic rendering.
A failed downstream display copy gets its own explicit display gap.

## Recording routes and independent closure

`publication::Route` contains the immutable original `RecordAddress` and presentation-to-pass time translation.
An absent address explicitly means the event happened while disarmed;
background serialization never applies the current arm state retroactively.

`configuration::Recording` retains raw HUB sample segments in their `ClockId` until both configuration and canonical source publication frontiers cross them.
Routing an actual mapped session sample first verifies clock identity, then subtracts the adopted HUB offset exactly once.
A source publication frontier means every earlier actual record acquired an immutable route or an explicit publication-failure disposition.
Source receipt, `OutputRetainedAck`, baseline application and configuration progress alone do not establish that frontier.

The ordinary direct path captures its callback recording segment before publishing direct notes, then advances its direct publication frontier only after those attempts.
The next session producer must replace `direct_publication_complete` with its complete merged output-publication frontier.
It must adopt the runtime clock and HUB offset, route historical output before calling `source_frontier`, and call `finish_recording_publication` afterward.
Resetting clock provenance refuses unresolved recorded segments rather than matching equal raw samples from different epochs.
No automatic `CONFIG_TIMELINE` retirement is introduced;
that remains the future input sequencer's independent proof obligation.

The writer retains old rewind passes until both ConfigurationPassComplete and SourcePassComplete are consumed.
Epoch completion additionally requires the original producer fence, configuration closure and source closure.
Delayed notes can therefore arrive after Stop or rewind and still reach their original file, including deciding which older voiced pass is rendered.
WAV tails remain with their own pass.
The real 128-pass limit refuses the 129th unresolved source pass with durable incompleteness;
source closure is drained before a new pass is charged, so released capacity is reusable even when controls arrive on different lanes.
A producer failure requests finalization but does not prove its retained prefix has reached disk.
The worker waits for pending Start/NewPass ownership and both lanes, including a stable consumed loss snapshot, before accounting failure for that exact epoch.
Capacity or file-creation refusal keeps the existing writers until they have been marked and flushed, or an actual I/O refusal has been reported.
After failure is accounted, only that epoch's recording copies are discarded;
display fanout and baseline reclamation continue.
A later Stop of that already-accounted epoch is terminal, clears the finishing state and drops its render request on the worker while preserving the failure status.
Disconnect enters the same cross-lane pump and first honors a pending Stop whose final source closure has now arrived.

## Take format and replay

Take format is now **4**.
Versions 0 through 3 are refused explicitly;
there is no compatibility shim.
Every persisted record struct has container-level serde defaults, while a complete baseline still must satisfy its full semantic contract.
Malformed complete final records, including typed integer/enum failures and invalid cut ordering, are errors, not successful truncated recordings.
A genuinely interrupted last line retains the format's existing readable-prefix behavior, including a cut inside a nonempty parameter ID string.
RON leaves unterminated string contents unconsumed, so string EOF is recognized independently of the empty-remainder check used for missing delimiters.

`Take.events` is one ordered canonical note/control stream.
The `notes()` iterator is an inspection adapter, not a second replay list.
Parsing validates the ordered stream before offline replay can consume it.
Standalone direct recording retains its ordinary Note adapter;
standalone verification and incremental/full offline replay use the canonical consumer.
An incomplete take is readable for inspection, but the offline render entrypoint refuses to render it.

Live timing uses continuous presentation seconds across raw sample-clock resets/rate changes and a fresh audio heartbeat independent of historical delivery times.
Queued pre-reset history therefore keeps its presentation domain while exact clock epochs remain available for original recording routes.
An idle GUI poll cannot re-anchor an old heartbeat.
The consumer maps drawing times while retaining the unshifted onset for direct lifetime matching and exact sample provenance for accepted output.
A known lifetime keeps its already-mapped onset even when a later heartbeat changes the offset or a baseline resumes it after a gap.
A fixture with audio now 11 and GUI now 21 maps delayed audio time 2 to GUI time 12;
a later heartbeat changes the offset by 0.01 while both direct and accepted lifetimes retain GUI onset 12.

## Verification and measurements

Maintained fixtures exercise complete 64-voice replacement and invalid duplicate/65th/final entries, other-source isolation, retained complete lifetimes before an empty baseline, duplicate retransmission, matching held recovery and Off/rejoin, actual publication saturation and no-future-callback loss, owned baseline backpressure/copy/reuse and gap-to-baseline resumption.
The recorder fixtures use its production rings and writer to serialize delayed history, original rewind routes, disarmed provenance, WAV tails and actual 128/129 pass capacity.
Serialized v4 replay fixtures run at 1 ms, 24 fps and 60 fps, compare exact roll history with full-roll reconstruction, and actually prune at each cadence for visibility/history checks.
Two deterministic fixtures pause the spawned recording worker after its actual Empty poll or after Stop is consumed:
the former retains two complete empty baseline payloads plus 4,094 notes and exact loss 4,097 without another producer call;
the latter receives final source history/closure before Disconnected and finishes a valid take.
The same spawned-worker fixture also sends Stop after failure has been accounted and checks terminal cleanup at the next actual Empty poll, before another callback or disconnect.
The 128/129 fixture continues publishing after refusal and reuses the same two-slot baseline bank three times through disk/display fanout.

The plugin target runs the real exported CLAP factory with `nice-plug/assert_process_allocs`.
Its allocation guard checks deallocation too.
The complete payload/full-ring/reuse fixture uses that same guard on actual publication operations.
The existing configuration factory and recording restore tests remain enabled.
Actual test counts, allocation measurements, final Actions and independent review results are recorded in the handoff.

Measured on this macOS target, `NoteDelta` is 128 bytes, the actual publication cell 176, `VoiceBaseline` 200, `SourceBaseline` 15,288 and a slot 15,304. One ring's cell payload is 720,896 bytes and its complete baseline bank 520,336 bytes.
The primary and display channels own distinct copies of these pools.
A temporary allocator probe measured four allocations requesting 1,241,848 bytes for one actual channel, with no construction-time frees.
Ordinary plugin default construction requested 13,211,948 bytes in 94 caller-thread allocations and freed 992 bytes during construction;
this includes existing UI/analyzer/take storage, excludes later CLAP owner activation and worker-thread allocations, and is not a resident-memory measurement.
Ordinary cells remain below the 256-byte ceiling.
The engineering budget now explicitly reserves both rings and both separate banks, raising its calculated subtotal to 132,716,032 bytes;
with the existing 16-MiB allowance it remains below the 144-MiB session ceiling.
A full 64-voice validation examines at most 2,016 earlier-voice pairs and 2,048 controller values;
publication attempts inspect at most two owned slots and never grow storage on audio.
A guarded test fill of two full baselines, 4,094 notes and Busy/Lost paths measured 328.042 microseconds in this optimized test build.
This is one local measurement, not callback throughput, Bitwig qualification or end-to-end timing evidence.

## Required next producer integration

Automatic direct current-state repair after reporting loss is explicitly deferred to the next companion/session producer slice.
Today's direct confirmed-pitch table lacks velocity, complete lifetime and channel metadata;
turning it into a rich authoritative frame would invent facts.
The present path keeps a visible gap and uncertainty instead.
The next owner must maintain truthful audio-owned baseline metadata, retain available actual history independently of state recovery, and publish the complete baseline through the exercised ownership API.

This slice does not implement source registry, lease/membership validation, voice credits, Tune/native controls, D-delay scheduling, automatic direct baseline production or adaptive policy.
It does not complete #617 or provide live multi-track/host acceptance evidence.
The next serial writer must start from this slice's final independently reviewed and CI-passing commit.
