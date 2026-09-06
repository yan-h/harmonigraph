# Companion aggregation for #617

This branch implements the production companion and session side of [#617](https://github.com/yan-h/harmonigraph/issues/617), on the canonical consumer foundation at `5ebe666b6eae6e29cd5e24d9468132047b2c217f`.
Its branch is `codex/617-companion-aggregation` and its draft PR will stack on `codex/617-canonical-recovery`.
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

Recording routes use the original runtime/epoch/pass and mapped session sample, with the adopted hub offset removed once for raw-span lookup.
A host reset freezes old route ownership until available old history and the actual terminal boundary settle.
Unplaceable post-reset terminal output uses an explicitly sealed stream acknowledgement with generation/final cut;
it does not fabricate a sample frontier.
The old take is marked incomplete after available old history is published.
Joined hub retirement retains the original publication owner and an independent recorder-writer lifetime until the delayed source closes.

## Executed verification so far

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

The counter-prefix fixtures seed internally consistent already-retained prefixes to reach exhaustion in finite time;
they do not claim to execute `2^64` historical outputs.
The stale-completion fixture is an explicit defensive duplicate injection;
CLAP host acceptance and completion remain synchronous.
The no-registry-drainer attachment fixture runs in a separate process because any other instance's main-thread setup legitimately services the process-wide registry.

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
