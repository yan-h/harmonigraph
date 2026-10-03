# Candidate decisions and tradeoffs

These are recommendations, not applied fixes.
Central execution results and final rank are recorded in README.md.

## Reconcile the current recording contract in documentation

Concrete cost: authoritative prose contradicts shipped behavior in several places, causing future work to start from obsolete contracts.
Scope: docs/offline-rendering.md lines 41–67, docs/adaptive-tuning-plugin.md lines 86–88, and docs/adaptive-tuning.md lines 460–463,590–591,768.
Source: take FORMAT_VERSION=6, RenderRequest::recorded uses captured dimensions, camera host parameters are recorded, appearance_for rejects a malformed selected document, Hub::apply publishes channel-bend deltas.
History: #1341 changed take schema, #1397 recorded camera/queued looks, #1402 selected arm-time dimensions, #1419 refused malformed appearance, #1223/#1272 implemented/repaired channel-bend publication.

Workflow/frequency: every recording/export investigation or user reading the guide, especially reproductions relying on saved appearance and bend behavior.
Evidence/confidence: source-verified contradiction; small mechanical documentation task.
Simplest intervention: one current behavior paragraph per topic, links instead of repeated control-flow narratives; keep old decisions explicitly historical.
Doing nothing preserves actively misleading guidance.
Maintenance gain: removes duplicate stale contracts; cost: one small prose edit and link/semantic checks.
Performance: no runtime effect; reduces investigation/setup mistakes, not measured CPU savings.
Correctness: no shipping behavior change; documents current refusal/capture semantics accurately.
New state/abstraction/synchronization/compatibility burden: none.
Verdict: pursue; high net benefit for low ongoing cost.

## Publish only the tail length that the host can read

Concrete structural cost: CLAP stores an entire ProcessStatus after each processed subblock, but its sole later reader maps it to a u32 tail length.
The error-string variant makes the stored enum larger than the lock-free atomic widths crossbeam-utils supports on this platform, so the ordinary store takes a global address-hashed SeqLock.
Scope: vendor/nice-plug/src/wrapper/clap/wrapper.rs:180,709,2194,2543,3633; nice-plug-core 0.1.5 ProcessStatus; locked crossbeam-utils 0.8.22 AtomicCell.
History: inherited in #630; prior boundary work noted that not every AtomicCell was lock-free but did not remove this unnecessary representation.

Workflow/frequency: every processed subblock, including ordinary Normal/KeepAlive output; actual lock contention, cost and audible impact are unmeasured.
Evidence/confidence: independently verified representation, dependency fallback and sole consumer; native layout query confirms size 24, alignment 8, lock_free=false versus lock-free u32.
Simplest intervention: AtomicU32 last_tail_samples, zero initially/on start, write the existing Tail(n)/KeepAlive/other mapping, and return its load in ext_tail_get.
Keep the original local ProcessStatus for CLAP result and error handling.
Doing nothing keeps a larger cross-thread representation and a fallback lock without a consumer for the additional data.

Maintenance gain: less retained information and synchronization machinery on this path; one scalar owner replaces one scalar owner.
Maintenance cost: move the existing mapping to publication and add one exported-tail fixture covering Normal, Error, Tail, KeepAlive, start reset and distinct statuses across two subblocks.
The fixture must query tail.get through the exported extension and also assert the immediate CLAP result.
Preserve publication only after actual plugin processing: early wrapper faults/invalid buffers retain the previous tail, while a plugin-returned Error publishes zero.
Retaining the previous tail after no new processing represents the last processed result; clearing it would discard that information.
Dedicated reset/stop currently retain it; start clears it.
Keep the existing Release/Acquire publication ordering: weakening it offers no measured benefit here and would widen the concurrency reasoning for an otherwise mechanical representation change.
Performance gain: source-established removal of a lock operation per subblock; no measured CPU or latency speedup claimed.
Performance loss: none expected from scalar publication; verify the chosen ordering against the scalar-only reader.
Correctness impact: tail values and immediate CLAP status/error must remain identical.
New state/abstraction/synchronization/compatibility burden: no new field count, cache, queue or public schema; a smaller existing publication.
Verdict: pursue; native representation confirmed; do not describe the whole wrapper as lock-free, since other locks and large AtomicCells remain.

## Bound Hub collection by work visited, including discarded epochs

Concrete mechanism: accepted batch length bounds retained captures; mismatched pops leave that length unchanged.
Scope: crates/harmonigraph-plugin/src/tuning/hub.rs collect, lines 600–654; Tune epoch adoption/publication and Session reset.
Workflow/frequency: reset/epoch transitions with queued MIDI from separate Tune instances; dense concurrent controller input is the stress case.
No normal-session dropout or sustained infinite drain has been measured.
Evidence: two independent source traces and the executed finite-capacity probe: matching rows pop/retain 2048, mismatched rows pop 3072 and retain zero.

Simplest intervention recommended: snapshot each paired row's available entries before draining, preserve accepted cap and row rotation.
This gives a finite work limit without persistent state and preserves current finite-backlog cleanup throughput.
A global visited-record budget is also defensible and tighter, but changes service latency for valid records behind discarded backlog more substantially.
Independent review favored the latter for a tighter bound; coordinator favors the snapshot for the smaller behavior change.
Either is a bounded local edit; this disagreement is about the desired overload policy, not the source mechanism.
The finite-backlog reproduction does not test the recommended snapshot fix: it intentionally would still consume all 3072 existing records.
At implementation, add a deterministic bounded-replenishment/queued-tail fixture and mixed stale/valid peer progress; the fix itself remains source-reasoned and untested in this audit.
Doing nothing leaves discard work dependent on other threads' progress.

Maintenance gain: loop bound is explicit and locally reviewable.
Maintenance cost: a local count and a focused queued-tail/progress regression; no new owner or coordination.
Performance gain: bounded worst-case work; no end-to-end improvement claimed.
Performance loss: arrivals during drain may wait until a later callback; global budget also defers some existing backlog.
Correctness: preserves dropped-epoch semantics, requires proving accepted work remains queued and eventually services peers.
New state/abstraction/synchronization/compatibility burden: local counter only; no schema or compatibility work.
Verdict: pursue a small fix; finite probe confirmed the mechanism; do not build a scheduler or handshake.

## Make the existing naming fallback test reach Namer

Concrete cost: a test named for the fallback only exercises equal_tempered_name directly.
Scope: crates/harmonigraph-ui/src/panes/spectral/names.rs existing fallback fixture around line 2581.
Workflow: an actual held pitch stops matching nodes after current tuning/equivalence edits.
Evidence: source inspection; executed scratch probe asserts failed lattice lookup then Namer fallback for fixed MIDI 63.863136: Just E- becomes current equal-tempered E.
Simplest intervention: strengthen the existing fixture with this path, replacing redundant helper-only assertions rather than multiplying tests.
Doing nothing leaves this integration path unprotected by the named test, while source still appears correct.
Maintenance gain: behavior-focused fixture proves its title; cost: a small setup, no framework.
Performance: no production effect; negligible test cost.
Correctness: no known shipping bug; protects a clean behavior that independent reasoning supports.
New state/abstraction/synchronization/compatibility burden: none.
Verdict: optional small improvement, behind the first two; do not call it a runtime defect.

## Spectrogram transient target splitting

Concrete structure: any of seven allocation-shape changes recreates all transient Targets while tile/history are independently carried.
Scope: crates/harmonigraph-render/src/spectrogram.rs lines 846–875; spectrogram/atmosphere.rs Targets::new around 1149–1171.
Workflow: halo-quality edits or resize, not ordinary fixed-size frames.
Evidence: native Metal probe confirmed unchanged allocation shapes with source-view replacement.
Across 30 alternating stable/change pairs at 1080p Uniform, CPU prepare median was 119.354 versus 372.292 microseconds (delta 252.938); p95 was 153.166 versus 462.458 microseconds.
This is one stage run under recorded desktop load; the delta includes necessary halo allocation.
Simplest possible intervention: retain only the demonstrated expensive unchanged resource, if a real drag hitch is attributable to it.
Doing nothing keeps one readily audited aggregate owner and rebind path.
Maintenance effect if split: negative unless a narrower owner also removes existing rules; selective retention adds dependent attachment/read bindings and invalidation conditions.
Avoiding unnecessary allocations is a potential performance benefit, not a maintenance gain.
Performance: the stage delta includes necessary allocation and does not isolate avoidable work or predict savings.
Correctness risk: stale views/bindings or history/carry failures during resize.
New burden: resource identity/rebinding rules; no reason for a generic resource graph.
Verdict: defer. A roughly 0.25 ms stage delta does not justify extra rebinding ownership for this infrequent setting transition.
Investigate further only if a saved real drag shows a material complete-frame hitch; the probe is not a prototype speedup.

## Recover the visual display after a nonfinite input sample

Concrete behavior: one NaN audio sample poisons smoothed display buckets; after two clean windows the newest history is finite and lit but 2564 display buckets remain nonfinite.
Scope: crates/harmonigraph-ui/src/spectrum.rs:422 display recurrence, analysis input/output, and crates/harmonigraph-core/src/spectrogram.rs:61 history sanitization.
Workflow/frequency: malformed upstream audio while audio continues flowing; occurrence in real projects is unknown.
Evidence/confidence: deterministic failing scratch assertion at shipped Fast 4096/48kHz, attack/release zero; clean newest history rules out an incompletely replaced FFT window.
The recurrence keeps a nonfinite carried value nonfinite even when subsequent input is finite; this is a visual recovery defect, not an observed audio dropout.

Simplest intervention: a small finite-power guard at visual publication or the shared raw-power boundary, with the reproduced recovery behavior as its test.
Prefer that to per-sample audio-callback validation or a reset/recovery state machine.
Doing nothing accepts loss of the affected display until its state is reset.
Maintenance gain: one explicit boundary contract; cost: a small guard and one behavior fixture.
Performance gain: none claimed; a guard adds per-bucket work, so keep it off the audio callback and avoid duplicating whole-buffer passes.
Correctness: prevents malformed input from permanently contaminating otherwise recovered visual state.
New state/abstraction/synchronization/compatibility burden: none is needed.
Verdict: pursue only the small local guard, low priority because actual incidence is unknown; reject a generic malformed-audio recovery framework.

## Rejected or retained on purpose

- Shared label/atlas ownership already exists; lattice prepare-time encoding and text paint-time carry must remain different.
- Solo's full-layer work preserves accepted color history; skipping hidden layers changes the look.
- Uniform's large-pane split has threshold-reaching parity coverage and an owner decision to retain it.
- Recorder source/configuration/producer closures protect different owners; collapsing them breaks late-data finalization.
- Recent analyzer reconfiguration and scene-normalization work already removed duplicate work; another cache or generalized settings schema adds debt without measured benefit.
- Readback pipelining adds queues, frame ownership and GPU synchronization; require end-to-end encoder-inclusive benefit before revisiting the recorded deferral.
- #886 dense refolds need the representative saved workload the owner requested; synthetic full-cap stress is not new evidence of normal usage.
- Build-target sharing or more CI jobs trade compile/queue behavior across sessions; this audit has no comparative measurement that justifies a change.
