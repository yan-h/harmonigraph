# Effective configuration foundation for #617

This stage implements effective tuning ownership, structured CLAP edits and restores, bounded confirmed-pitch learning, and resolved configuration recording/replay.
It does not implement the companion, aggregation, session admission, complete interval sequencing, accepted performance output, baseline publication, or adaptive assignment.
The exact committed base is `f6f1bdebdde073048f4bae636e97ed071a6648b9` on `codex/617-source-identity`.
Full CI run `33951261601` passed at that base before the first tracked edit.
The implementation branch is `codex/617-effective-configuration` in the Codex-managed worktree `/Users/yan/.codex/worktrees/ceee/harmonigraph`.
The stacked draft PR targets `codex/617-source-identity` and remains unmerged.
Its exact reviewed/pushed head belongs in the PR and final handoff, avoiding a self-referential hash here.

## Ownership and musical behavior

The serde-free core reducer owns raw axes, effective exact axes, comma/Auto/learning modes, dependency-specific comma judgements, and a musical revision.
Syntonic derivation precedes the septimal test.
Ordinary detection engages only;
explicit release judges its newly committed raw axes, and explicitly enabling Auto reopens that judgement.
Learning may release a comma only with its complete required evidence.
The existing 30 lattice regressions cover asynchronous stale parameter values, learning, manual release, and dependency-specific rechecks after extraction.
Command acknowledgement is separate from musical revision.
Display tolerance, camera state, and inert raw derived-axis changes do not create musical revisions.
Assignment policy version zero explicitly has no selected domain, radii, or weights;
#621 must supply those constants.

CLAP alone installs the new audio owner before editor construction.
Five existing host parameter IDs remain unchanged.
A preset, manual derived-axis unlock, comma change, Auto recheck, or learning switch is one semantic transaction.
The editor copies one coherent owner snapshot per frame.
It neither learns from its lossy display ring nor uses equal host values to acknowledge commands.
The standalone and VST display adapter use the same pure resolver synchronously;
they make no adaptive/session claim.
No delay or standalone latency setting changes here, including the planned D = 512 candidate.

The wrapper captures actual host event samples and original same-sample order before legacy parameter handling.
Each same-sample group applies its parameter operations before observing its notes, preserves note lifecycle order, and infers learning after initial tuning expressions.
A `params.flush` record remains untimed until the next explicit process boundary.
A deferred UI batch keeps its first observed boundary, and retained host events keep their original offsets across callbacks and wrapper sub-blocks.
There is one enclosing callback control budget, with one unit for each insertion, application, and retirement.

## Fixed storage and recovery seams

The off-thread producer serializes UI and restore ingress into 128 command cells.
Two separately owned restore slots stay unavailable until their command is applied;
queue transfer alone cannot acknowledge application or permit reuse.
Audio copies fixed values and never parses JSON, grows a collection, or releases the last owner of a restore allocation.
The coherent publication uses atomic payload words and an atomic sequence;
it does not race an ordinary struct behind a seqlock.
Main-thread snapshot reads do not take the audio runtime mutex.
Host callbacks run after audio runtime and plugin borrows are dropped.

The configuration timeline has 128 independently retained marker cells.
The exported factory fixture actually fills these with a drained command queue, then reaches the 129th required marker.
UI/restore backpressure leaves accepted commands owned;
required marker exhaustion reports an explicit configuration fault.
`CONTROL_WORK = 16` exhaustion alone retains cursors and does not report capacity failure.
The one persistent host-input allocation holds 2,048 exact supported input records;
the fixture fills it and rejects growth while a same-sample group is retained.
Unknown input types still use the existing immediate legacy performance path.
The later performance adapter must extend the owned representation before claiming retained forwarding of those types.

No session consumer yet supplies finalized/binding frontiers, so this stage deliberately does not guess a retirement clock.
Consequently a running instance retains at most 128 configuration operations until a host reset;
this is an integration-stage build, not finished adaptive tuning.
The core exposes cohort binding, pending-marker barriers, and budgeted retirement once the integrating owner proves all earlier input/commands have been represented and all bindings copied.
A cohort receives one copied configuration;
a later command cannot interrupt it.
Reset clears retained timed work and confirmed input while preserving the musical configuration, revision, and pending off-thread commands.
Untimed flush survives until a real boundary.
The later sequencer still owes whole-session emergency handling, frontend status integration, epoch mapping, and those frontier proofs.

Confirmed state has 256 rows and a 64-row per-source limit.
Direct input is explicitly observed input;
future validated accepted-output rows carry their own provenance and lifetime.
Incomplete input disables inference until complete replacement/reset, rather than silently learning a truncated chord.
Replacement validates the whole source set before touching live rows.
Learning uses fixed scratch and exact pitch-class-set memoization, with a maximal fixture containing 256 distinct classes and all 32,640 unordered pairs.
An unchanged class set does not revisit those pairs;
rearming reopens the same chord, and an empty chord changes no axis.
This is bounded ownership and apparatus evidence, not a host timing guarantee.

## Restore, host notification, and persistence

Both host state load and GUI apply prepare the complete musical snapshot off audio under the serialized producer boundary.
They exclude owned parameter and musical fields from generic visual restoration, bypass its active-state rendezvous, and do not call generic initialize/reset for an ordinary edit.
An active plugin with suspended callbacks can accept a restore and immediately return coherent host `get_value` and both save paths without opening an editor.
The latest accepted restore shadows musical readback until a published applied command ID covers it.
An older restore acknowledgement or UI notification cannot retire a newer shadow.
An accepted restore preview includes the actual held modulation offset;
owner adoption preserves host modulation even when the host sends no new modulation event.
The offset is stored explicitly because clamped normalized output cannot reconstruct it.

Applied UI/learning edits produce begin/value/end host gestures.
Host notification rejection never rolls back the local commit and has a distinct visible status.
A separate dirty latch requests main-thread parameter rescan and host state dirty notification, independent of the generic task queue.
Restore acceptance also preserves that latch so an older in-flight notification cannot be the last host cache update.
A refused full UI queue reports refusal rather than pretending that command was accepted.

The new authoritative CLAP `musical-settings` field persists comma, Auto, and learning modes independently of whether an editor has ever existed.
Its struct and the new take structs carry container-level serde defaults.
Old UI blobs still contain their legacy display modes for VST/standalone, but CLAP no longer treats those duplicates as authority.
Old CLAP states missing `musical-settings` start with current mode defaults;
learning is now saved for CLAP.
Raw host parameter values retain their five keys and ordinary units.
Runtime command IDs, judgement caches, and live revision identities are not saved into plugin state.
There are no aliases or migration shims.

Take format is now version 3 and rejects older headers before an old final record can look like an interrupted write.
CLAP records initial and effective resolved configuration boundaries into the existing recorder stream;
each new take pass rewrites its initial configuration.
Standalone records synchronous resolved configurations.
Replay consumes the exact recorded axes/modes/policy and does not rerun comma detection or learning per video frame.
The serialization/cadence fixture uses 1 ms, 24 fps, and 60 fps advances, an armed learning mode, and contradictory raw automation to reach that bypass.
Legacy VST recordings do not claim audio-owned configuration capture.

## Verification and rejected approaches

All Cargo work is serialized with `CARGO_BUILD_JOBS=2` and `RUST_TEST_THREADS=2`, repository sccache, and this worktree's own target.
The real exported Harmonigraph factory fixtures run with `nice-plug/assert_process_allocs` and without the optional tuning probe.
They cover suspended active host/GUI restores, ordered distinct UI IDs and external same-value automation, exact offsets and flush, same-sample initial tuning, rejected output notification, bounded continuation, actual marker/input exhaustion, restore-slot reuse, preserved modulation, and default opt-out construction.
`ci.sh` now includes this feature combination independently of the existing optional probe fixture.
The final handoff records focused replay/record/standalone/take checks, exact-head independent review, Actions, and the loadable release pair.

Rejected approaches were UI/raw-value polling as command authority, value-equality acknowledgement, parsing musical settings from `Plugin::initialize`, waiting for a suspended audio callback during restore, clearing held modulation to make preview agree, guessing retirement from callback time, and rerunning learning at video frame cadence.
A transient implementation error cleared newly captured input while reconciling reset;
reset reconciliation now precedes capture.
Another restamped deferred UI batches at their eventual drain;
the continuation fixture now pins the first boundary.
No session, adaptive output, or live Bitwig timing evidence is inferred from these apparatus checks.
Before pausing, commit first, build both release packages, and read `./load-plugin.sh --tag`;
the shared Bitwig plugin and renderer slots remain untouched.
