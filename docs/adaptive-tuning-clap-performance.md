# CLAP performance boundary for #616

This stage adds the production opt-in host boundary for the later shared-session and sequencer work.
It implements no companion, registry, central assignment policy, calibrated clock, musical gate, journal, voice credit, or retained retry policy.
Harmonigraph remains configuration-enabled and performance-opted-out.
The accepted D = 512 candidate is unchanged and still needs production sequencer and host evidence.

The exact committed base is `ee3303307ef87565d7a4efafe337b1e4220b0b33` on `codex/617-effective-configuration`.
Parent Full CI run `33972457858` passed at that exact head before any tracked edit or Cargo command.
The implementation runs in the Codex-managed worktree `/Users/yan/.codex/worktrees/7009/harmonigraph`, on `codex/616-clap-performance-adapter`.
Its stacked draft PR targets the parent branch and remains unmerged.
The final handoff records the exact reviewed head and release build tag.

## Input ownership and clocks

`ClapPlugin::CLAP_PERFORMANCE` defaults to false and is independent of `CLAP_CONFIGURATION`.
A performance-only class constructs no configuration mailbox.
Both opt-ins use the same single persistent 2,048-cell `InputStorage` allocation and existing exact `InputValue`/`OwnedInput` values.
Notes, expressions and three-byte MIDI retain signed wildcard addresses, note ID, flags and f64 values.
Transport retains every raw fixed-point timeline, tempo/increment, loop, bar and signature field.
The common envelope retains original sample, enclosing start/length, offset, event index, batch, command cut, command observation boundary and flush provenance.
No host pointer enters retained storage.

The callback-start transport, when present, occupies one cell before its host event list, with `event_index = u32::MAX` and offset zero.
That internal observation consumes real capacity:
2,048 host events plus a transport observation cannot fit the 2,048-cell retained pool.
The host list itself is limited to 2,048 events, including parameters, transport and ignored gesture headers, before its first event is inspected.
Each host event is copied at most once on the performance path.
Invalid, decreasing, oversized, out-of-interval or unsupported input rejects the callback without delivering a partial new prefix.
Previously retained input stays owned.
At a valid process boundary its consumers still get one bounded recovery pass when the new batch is rejected, so a full pool with a retained callback transport cannot prevent progress after performance backpressure clears.
Recovery preserves the old envelope and does not publish a complete configuration prefix for the rejected callback.
Emergency output and finalization still run.

Configuration applies same-sample parameters before observing notes and learning.
Separate acknowledgement cursors retain configuration and performance consumption, so either can stop without repeating the other's work.
A third short wrapper cursor reads those same cells to update ordinary parameters and split audio on exact transport/automation boundaries.
Reclamation requires all enabled consumers to finish.
Error finalization drains outstanding ordinary parameter work through that same wrapper cursor before acknowledging it, so raw delivery of an accepted flush event cannot stand in for updating the actual parameter value.
No growing legacy input deque or second host scan carries the performance stream.
A first event at a nonzero transport offset is itself a split boundary.

Raw callback and sub-block contexts carry the unmodified enclosing steady time, exact lengths/starts and owned raw transport observation.
They do not substitute framework-extrapolated transport or define a session epoch/calibration.
Untimed `params.flush` input stays untimed until a process boundary;
its original offset and flush provenance remain visible.
It is not a fabricated musical callback and emits only untimed parameter notifications.
Every rejected flush reports a configuration fault when that owner is enabled, including failures before the first host event is read.
The first unreported input failure stays latched until the next process callback reports it;
successful captures and state resets cannot erase it.

The supported performance payload is CLAP note on/off/choke/end, all CLAP note expressions and three-byte MIDI.
Parameter gestures are explicitly ignored after being counted.
Arbitrary SysEx, MIDI2, foreign event spaces and addressed parameter automation/modulation are unsupported and fault the performance callback.
They never silently acquire a claim of complete forwarding.
Default opt-out behavior retains the existing legacy event path.

## Host output and completion

One fixed scheduler shares 512 normal and 128 reserved emergency event/completion credits across the enclosing callback's sub-blocks and parameter notifications.
A fixed min-heap orders staged groups by enclosing offset and insertion order, with at most ten levels per insertion/removal.
Admission charges a single event once or an onset-plus-initial-tuning pair twice, atomically, before the plugin can claim its musical permit.
Credits are not recycled within a callback after an unattempted or rejected claim.
The caller retains the musical event when staging is refused.
Output offsets must be in the enclosing interval and at or beyond the current legal host cursor.

Immediately before a group, a short `clap_performance_prepare` call lets the caller validate identity/generation and claim its own gate/credits.
The wrapper drops every plugin, scheduler and event borrow before calling the host.
The tracing callback also runs after those borrows end.
A rejected onset suppresses its unattempted tuning and the remaining dependent normal chain.
An accepted onset attempts tuning under the same permit even if a fence closes between the host calls.
A partial onset is reported as its exact accepted prefix, with no invented tuning and no onset resend.

Every staged group receives `clap_performance_complete`, including ineligible, inhibited, missing-output and process-error outcomes.
The masks distinguish accepted, rejected-attempted and unattempted events.
The caller must durably record that result before clearing BUSY or returning its permit.
The completion can stage emergency work at the current legal cursor or later.
Its completion cell remains reserved through that call.
Musical retry retention, accepted-output sequences, containment, release debt and credit return belong to the later caller.
The wrapper cannot establish downstream termination by clearing a queue.

`clap_performance_finalize` runs once on every process exit before the final emergency drain.
`clap_performance_end` then receives the fully settled summary before generic state restore/reset.
A missing host output list produces unattempted completions;
it never invents acceptance.
Using legacy `ProcessContext::send_event` while opted in sets an explicit summary fault and cannot grow the legacy output deque.

Configuration notification begin/value/end attempts share normal credits and the same chronological cursor.
Unattempted phases stay in their original notification cell across callbacks.
A monotonic insertion sequence preserves order when cells are reused, and an already-open parameter gesture finishes before a later begin.
Rejected end gestures retain closing debt, which must settle before a later begin on that parameter.
Accepted restore generations can suppress obsolete values without losing an already-open gesture's close.
Completion preserves the independent dirty/rescan latch, including a restore racing an old host write.
The final drain requests any newly required rescan on every exit, including process errors and rejected input.
Ordinary parameter output retains a refused or budget-blocked queue head and cannot spend emergency credits.
The configuration-only Harmonigraph path preserves its existing notification implementation and recording contracts.

## Lifecycle and storage budget

New CLAP main-thread init, activate, deactivate and destroy seams are distinct from `Plugin::initialize`, which can run during audio-thread state restore.
They expose future registry preparation/reclamation boundaries without implementing registry policy.
Serialized performance start/stop/reset hooks run inside allocation/deallocation guards.
Activation still publishes deferred latency after dropping the plugin/init-context borrows and before marking the wrapper active, with no redundant restart.
GUI work queued during a performance callback publishes a deferred wakeup and rechecks the audio phase.
An atomic read-modify-write handshake assigns the wakeup to the finishing audio thread or the delayed producer, so a producer that finishes after the last process callback cannot strand its task.
Audio callers still defer host callbacks until their plugin/runtime borrows have ended.

Full retained transport and provenance require revising the guessed input-cell ceiling from 128 to **192 bytes**, rather than truncating values or adding another pool.
The engineering contract permits such measured layout revisions.
The 17-wrapper input budget increases by `17 * 2048 * 64 = 2,228,224` bytes, or 2.125 MiB.
The revised contract subtotal is 130,029,056 bytes;
with its existing 16 MiB allowance it remains below the 144 MiB session ceiling.
Compile-time assertions constrain the actual allocated `Option<OwnedInput>` cell and alignment, and the group/completion types.
Executed on this macOS build, `Option<OwnedInput>` is 176 bytes with alignment eight, so the one input payload allocation is 360,448 bytes.
`Option<performance::Group>` is 232 bytes and `Completion` is 240 bytes.
The output group allocation is 148,480 bytes plus a 1,280-byte fixed heap index in the owner;
a completion is a bounded stack value whose reserved group cell remains owned during delivery.
The two boundary payload allocations therefore total 508,928 bytes per performance-enabled wrapper, excluding small owners and the plugin's configuration/mailbox state.
The focused boundary target contains 23 fixtures and the existing Harmonigraph configuration factory target contains 21 fixtures.
No persisted configuration field, take format, latency setting or visual default changes here.

## Verification and remaining integration

The vendored `clap_boundary` integration target exports combined, configuration-only, performance-only and legacy fixture classes through the real factory.
It supplies the declared stereo main and sidechain ports and a preallocated selective host sink.
`assert_process_allocs` guards both allocation and deallocation through production hooks, saturation, output and finalization.
The required `clap-boundary-tests` feature adds an instance-local scheduling observation for the deterministic deferred-wakeup race;
production builds do not enable it.
`ci.sh` runs it independently of the optional tuning probe.
Existing Harmonigraph configuration factory fixtures protect captured recording intent, prefix/segment completion, restore generations and gesture debt.

Fixtures reach the actual input pool, normal/emergency attempt limits, one-cell-left onset admission, repeated local offsets and short callbacks, exact retained transport, shared configuration/performance backpressure, notification timing and partial gestures, onset rejection/partial acceptance, fencing before claim/between host calls, expression/release retry values and unconditional finalization.
They validate the real wrapper boundary, without claiming a production session gate, journal or adaptive sequencer exists.
Recovery fixtures fill the real pool with callback-start transport plus 2,047 notes and exercise true `params.flush` calls at and beyond capacity.
An accepted ordinary flush parameter followed by a rejected batch must retain its exact raw provenance and update actual host parameter readback before recovery acknowledges its cell.
Deterministic race fixtures pause the real GUI executor after observing the audio phase, and pause an old host value write while main-thread restore and rescan complete.
They require a new host callback without relying on a later process call to rescue pending work.

The later serial stages still own source/session identity validation, main-thread registration and offers, musical enrollment, lifecycle gates, durable journals, held credits, retry retention and emergency debt, complete sequencing/clock frontiers, the actual D512 schedule and musical policy.
No timing measurement from a synthetic boundary fixture establishes supported production delay.
