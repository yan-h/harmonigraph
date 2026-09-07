# Adaptive tuning ordinary recovery

This stage starts from reviewed ordinary-progress commit `5c9d17f71a5b330152cdf094ef1c25c96b70edef`, draft [#694](https://github.com/yan-h/harmonigraph/pull/694).
It activates the existing recovery protocol from accepted output and qualifies a small ordinary D512 workflow with the artificial policy.
The [product priorities](adaptive-tuning.md#design-priorities) apply:
fixes requiring a large increase in complexity or addressing unlikely ordinary-use cases are deferred explicitly.
The [engineering contracts](adaptive-tuning-contracts.md) still govern physical output, ownership and channel safety.

## Accepted divergence

The Hub validates the retained Source lease, incarnation, epoch, request/Life index, lifetime and bound decision before comparing accepted output with its expected time.
The immutable decision identifies its configuration and emission generation.
Collection first deduplicates the contiguous accepted output sequence;
an accepted onset, release or pitch-expression record can therefore close the existing gates even while another Source delays the complete canonical publication frontier.
Factual output application runs before new input policy work.
No new dependent plan is issued after this addressed divergence is known.

A missing callback, delayed report, absent acceptance, sticky timing bit or broad factual revision alone does not activate ordinary recovery.
Terminal output/evidence/storage faults retain priority and their existing closed cancellation path.
Ordinary work and queue pressure retain notes and valid late assignments.
The normal delay remains 512 samples, including 512-frame callbacks.

An existing write-only Plan input field now holds its expected input-to-output translation.
It begins at 512 samples for normal decisions and at the explicit recovery boundary relative to original onset for replayed decisions.
The accepted onset then fixes that lifetime's translation for later expression and release comparisons.
A translated expression is not compared against the original input+512 schedule again.
The new generation keeps its original request and copied configuration binding.

Recovery reuses the finite fence, inventory, status, factual-context copy, replay and resume transaction.
Accepted facts in its already-fenced suffix before the factual copy are included in that copy;
an earlier affected decision or later divergence queues the existing continuation.
Racing accepted corrections remain frozen.
Pending durations and expression are preserved, and completed output sequences cannot restart recovery on quiet callbacks.
The existing factual-revision checks still protect an active replay's copied context;
they are not standalone evidence that a particular output missed its schedule.

An independent review found that a continuation could leave previously resumed Sources open while another participant still held its completion message.
A reaching three-Source fixture leaves C's real InventoryComplete in the repair cell, lets A/B reopen, then runs A→Hub→B while C holds canonical publication behind A's newly accepted late output.
Before correction B remained OPEN despite the queued continuation.
The correction reuses the existing resumed-generation closure before queuing that continuation, retaining a racing BUSY claim as factual output.
Afterward B stays closed, its successor completes once with its original duration, accepted releases settle, and all owners drain.

The review also questioned whether an accepted Stop release could be omitted from the pre-copy cut.
The runtime probe received sequence 9 with only 8 applied and a status cut of 8, but remained in Rebuild through 16 further A/B/Hub callbacks.
The existing pending-baseline guard was the reason:
Stop had published a baseline through that release, and it could not install ahead of its factual output.
Once C's delayed callback stream resumed, the release applied and the replayed successor used correction 0.02 with the released voice absent.
No additional cut tracking was added for this guarded case.
Both fixtures resume each Source at its own next raw interval;
skipping raw time would instead test a clock discontinuity.

## Reaching checks

The production automatic trace uses three Sources and four initial notes at 44.1 kHz / 512 frames.
A/C sound at sample 2048; B's raw 1984 onset maps to 2048 and actually sounds at 2560, 64 samples beyond its raw 2496 deadline.
C's fourth onset is already bound when B's accepted output arrives.
The Hub closes its old generation before C can claim it, without `test_request_recovery` or substituted mailbox data.
The successor gets a new decision/emission generation and the same configuration, exactly one On/tuning pair, expression at On+10 and Off at On+20.
An established release remains at its original input+512, sample 3591, while that successor is fenced.
The recovered successor sounds at sample 23552 versus original due time 2960:
20592 additional samples, about 467 ms in this trace.
The finite protocol completes, but this functional result does not establish acceptable musical latency.
Surviving held corrections stay unchanged and quiet callbacks stop advancing recovery.

The same trace keeps raw time continuous while playing song position wraps 16→0, then Stops with a late held voice and another bound pending onset.
Actual release occurs at Stop offset 0, the pending old attack never sounds, and a new phrase received on that Source at Stop completes once through repeated stopped callbacks.
After accepted neutral idle, a fresh gesture on the previously late channel returns to exact input+512.
This neutral-state precondition is explicit, not a claim about unknown initial pedals.

The native-GUI Off fixture now observes actual output with both a held voice and a pending adaptive request.
The held voice's initial correction 0.01 remains in its later expression 0.26; the pre-Off request finishes at 0.02; the newly received Off note alone uses 0.
Rejoin does not duplicate the five attacks, and final releases settle the held set.

The 15-note fixture retains all 60 original accepted events and exact gestures for all six offset/order cases.
Automatic recovery's finite inventory reader outlives the short notes, so a bounded quiet drain now waits for its acknowledgement before asserting Capture retirement.
It permits no extra output or terminal fault.
The initial affected group had 36 passes and this one ownership-wait failure;
the corrected fixture then passed all six cases independently and the full 37-test group passed before the two review fixtures were added.
These are functional allocation-guard checks, not musical-policy cost measurements or listening validation.

## Reset and CI fixture corrections

[Issue #693](https://github.com/yan-h/harmonigraph/issues/693)'s prefix fixture now asserts that a real rejected CC88 repair inhibits new nonrelease input before input-cut allocation.
Its complete fresh 37/On gesture occurs after Reset/admission.
Actual 55 before repair, actual 0 only after accepted repair, canceled 55 exclusion, unknown-prefix behavior, accepted 37 consumption and Hub receiver assertions remain intact.

The second Reset fixture originally stopped with setup generation 2 still pending, applied generation 1 and no offer, even though old output, recovery, lease and all event/Capture owners had just settled.
It now waits within a finite bound for the exact applied setup generation, cleared fault and real adopted/joined coverage before sending fresh input.
Its later accepted setup, controller/bend replay and capacity-pressure assertions remain unchanged in meaning.
Neither fixture adds a session Reset to its D0 aggregation scope.

[Issue #695](https://github.com/yan-h/harmonigraph/issues/695)'s withdrawn-Hub child now expects the measured REFERENCE_FAULT from Hub destruction.
No host output was rejected in that path.
Both children execute all 4096-journal, 35-emergency-output, 32-release and sequence 4163 assertions, followed by owner/credit drainage and the exact 64-record take checks.
The duplicate sealed-ACK fixture delivers its second reply after actual Source consumption frees the single ordinary cell.
The repair cell remains reserved, credits settle once, and the stale-epoch and recording assertions still execute.

## Memory and limitations

No allocation or owner population was added.
The Plan field is reused in place, and the full 256-byte future Plan/Life ceilings and configuration reservations remain intact.
Clippy exposed the inventory input-time copy as unused once the write-only Plan input field was repurposed.
That redundant `RequestInventory` field was removed;
original timing remains in Source Life/Capture and accepted `OutputDelta`, while the inventory retains exact original-On serial, lifetime, configuration and outcome.
The two inventory fixtures assert their original-On serials after this cleanup.
The guarded allocator rerun measures 106201496 ordinary bytes, 8192 fewer than the base, because each of sixteen inventory windows shrank by 512 bytes.
Record/window/slot sizes are now 96/6264/6272 bytes.
The window remains inside its existing prepaid manifest reservation, so the complete future projection stays 150730867, leaving 264077 below 144 MiB.
Both voice/history halves, indices and 4 MiB policy workspace remain reserved;
no prepaid capacity is reclaimed twice.

[Issue #692](https://github.com/yan-h/harmonigraph/issues/692)'s invalid-calibration Reset followed by valid Reset remains deferred.
It is UI-reachable, but the inadmissible-proposal sequence is low priority compared with normal use;
this stage adds no rejection/supersession subsystem and does not claim that sequence recovers.

[Issue #696](https://github.com/yan-h/harmonigraph/issues/696) records a demonstrated ordinary limitation.
Without any accepted CC64/66/69 values, a healthy first On/Off can fully settle while retaining a channel translation whose neutral state is unknown.
A second same-channel phrase needing greater translation after one delayed Hub callback remained unsounded through 64 continuous callbacks, with three pending events, one Life, unchanged output sequence/ACK 3 and no terminal fault.
Its bound assignment exists; neither missing service, physical voice debt nor exhausted storage explains the wait.
A real Stop cancels it legally.
No small automatic correction was identified that preserves the accepted-neutral contract without changing controller initialization policy.
It is deferred for that reason, not because the note-only case is rare.
Fresh Bitwig controller initialization is unmeasured and is a prioritized manual host check;
known-neutral fixtures do not prove universal progress.

The musical policy/history/rich-metadata integration, complete-policy timing and actual late-output display/take/offline integration remain subsequent #621 work.
Full Bitwig, listening and live/offline destination validation have not been performed here.
Extreme new capacity, stale-reply, peer-cancellation and destruction matrices are deferred under the user's complexity policy; existing useful safety coverage is retained.
The draft PR carries exact committed-head review, CI and both-package release receipts.
Nothing is merged or swapped into the shared DAW slot by this stage.
