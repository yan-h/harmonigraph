# Deep audit implementation decisions — 2026-10-03

This follows the [completed audit](https://github.com/yan-h/harmonigraph/pull/1434) and [tracking issue](https://github.com/yan-h/harmonigraph/issues/1439).
Recommendations were rechecked against main `77c453a4` and independently challenged before implementation.
The decision is based on combined maintainability, performance and correctness benefit for the actual plugin workflows.
No saved-state shape, audio output policy or normal-input picture is deliberately changed.

## Accepted changes

| Finding | Evidence and intervention | Lasting benefit and cost |
| --- | --- | --- |
| [#1435: retained CLAP status](https://github.com/yan-h/harmonigraph/issues/1435) | Only the exported tail extension consumes retained status. Publish its existing mapping through an `AtomicU32` after each processed subblock. | Removes unused retained information and the full enum's fallback lock without another owner. Keeps Release/Acquire ordering, immediate CLAP status and lifecycle behavior. No measured CPU speedup or claim that the whole wrapper is lock-free. |
| [#1438: obsolete documentation](https://github.com/yan-h/harmonigraph/issues/1438) | Current take/export and bend-publication source contradicts the guides. Correct the authoritative paragraphs and remove duplicate obsolete narratives. | Less misleading guidance and fewer contracts to maintain; no runtime cost or behavior change. |
| [#1436: discarded capture work](https://github.com/yan-h/harmonigraph/issues/1436) | The accepted-record cap does not count epoch rejections. Snapshot each paired consumer's available entries before draining. | A local finite work bound without persistent scheduling state. Concurrent arrivals can wait one callback; existing finite backlog, accepted cap and rotation remain. No measured dropout or end-to-end speedup is claimed. |
| Optional Namer fixture | The named fallback test called only the spelling helper. Replace its assertions with an actual held Just pitch named before and after a tuning change, explicitly proving no current node matches. | Better behavior coverage within the existing test; no production change or new test framework. This is not a shipping defect. |
| [#1437: nonfinite display recovery](https://github.com/yan-h/harmonigraph/issues/1437) | A fresh regression fails after clean audio replaces the FFT window while displayed power remains nonfinite. Treat invalid incoming power as silence in the existing smoothing loop. | Prevents persistent display contamination with one per-bucket finite check outside the callback. No new state or extra buffer pass. Actual incidence is unknown; that uncertainty warrants only this small guard. |

The Hub's bound applies to paired rows because other instances can replenish them concurrently.
The direct queue is owned by the current callback and cannot grow during its drain.
With sixteen paired rings of 1,024 entries, a collection visits at most 16,384 paired captures plus the direct queue's finite contents.
The 2,048 accepted-record cap remains independent of discarded work.
A global visited-record cap would be tighter but would defer more already-queued valid work behind stale backlog;
there is no measured need for that overload-policy change.

Tail publication still occurs only after actual plugin processing.
An early wrapper failure retains the last processed tail because no replacement result exists;
a plugin-returned Error publishes zero.
Reset and stop retain that last result, while start clears it for the new processing run.

The spectrogram constructor keeps its existing sanitization because it is an independent data boundary.
The new check protects carried curve smoothing without broadening raw analyzer semantics or validating every audio sample in the callback.

## Rejected and deferred alternatives

- Defer spectrogram target splitting: the audit's roughly 0.253 ms preparation delta includes necessary allocations and does not establish avoidable complete-frame cost. Extra resource ownership and rebinding rules do not earn their place without a representative hitch.
- Defer readback pipelining: load-sensitive export samples do not establish encoder-inclusive savings sufficient to justify more queues and GPU synchronization.
- Retain shared label/atlas ownership and distinct upload timing: lattice preparation and text painting use resources at different points in the frame.
- Retain Solo's full-layer work and Uniform's large-pane split: accepted color history and threshold-reaching parity coverage justify those paths.
- Retain recording's separate source, configuration and producer closure frontiers: they protect different late-data owners.
- Reject another analyzer/settings cache or generalized settings schema: recent ownership changes already removed the duplicate work, with no demonstrated need for more invalidation rules.
- Defer dense refold optimization pending the representative saved workload requested in [#886](https://github.com/yan-h/harmonigraph/issues/886); synthetic stress is not evidence of ordinary usage.
- Reject new build-target sharing or CI architecture for this task: the audit supplies no comparative measurement supporting their coordination cost.
- Reject a general malformed-audio recovery framework: the local display guard addresses the reproduced failure without a state machine.

Existing renderer investigations and platform/build issues remain with their existing trackers;
this task does not treat them as new implementation-ready findings.
Native Bitwig behavior remains unverified because the running DAW and installed plugin are not disturbed.
The audit's [host procedures](https://github.com/yan-h/harmonigraph/blob/codex/recursive-audit-2026-10-03/docs/audits/2026-10-03-recursive/HOST-VALIDATION.md) preserve that remaining work.

## Verification scope

The exported CLAP fixture checks all four status mappings, publication between distinct subblocks, early input failure and reset/stop/start behavior through the actual extension.
The Hub fixture injects a real queued arrival after the row snapshot and proves it waits while a valid peer progresses;
a second fixture reaches the accepted cap with three full rings and checks subsequent progress.
The display fixture supplies one NaN followed by two clean Fast analysis windows, checks both finite history and a lit finite curve, and failed on the original recurrence.
The existing Namer fallback test now invokes Namer with a proven nonmatching pitch.
Normal-input rendering remains covered by existing UI/offline tests and the repository CI gates; no golden baseline is changed.
