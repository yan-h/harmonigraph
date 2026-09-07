# Adaptive tuning diagnostics display

This #616 stage adds a shared UI formatter to the native Tune editor and the Hub's Session menu.
It displays the existing `Shared.status` and `Shared.extra_delay` publications;
it does not implement the audio producer or complete adaptive tuning.
The Session menu remains outside the picture shared with exported video.

`setup::TIMING_FAILURE` is bit 5 and means a noncanceling timing failure.
Its text remains valid after a delayed attack sounds because the producer may retain the historical latch.
Timing alone does not display retention exhaustion or require Reset.
Every published emergency reason is displayed alongside timing when both are present:
retention capacity, output, clock, input and reference/session capacity.
Unknown status is identified without guessing a capacity failure.

Current extra delay is displayed in samples above fixed tuner latency.
The display performs no conversion using draft clock values and makes no catch-up promise.
Pairing, adopted clock validity and pending setup remain separate context.
Reset and Apply still submit the prepared setup request;
the UI neither clears diagnostics nor treats a submitted request as completed recovery.

## Producer dependency

The central #616 integration must publish the noncanceling timing bit with coherent status composition, latch it once per plan, and publish additional samples above D rather than the total input-to-output shift.
Those audio changes belong to the central implementation stage.
On this branch's capture-arena base, the timing bit is not yet produced and the existing delay producer still reports the total shift.
The extra-delay label becomes semantically correct only after that integration.

## Storage

No instance field, persisted shape or callback path changes here.
`Shared` retains its existing atomics and representation:
this stage adds zero bytes to the audio storage ledger.
The baseline future plan remains 150,890,091 of 150,994,944 bytes, with 104,853 bytes of slack before the central stage's own accounting.
The formatter allocates temporary text only on UI refresh and is called from the native cosmetic timer and Session menu.
