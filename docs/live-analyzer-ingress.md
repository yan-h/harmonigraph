# Live analyzer ingress

The callback publishes one descriptor for each retained whole-frame prefix into a bounded SPSC queue,
after committing its interleaved samples to a second bounded SPSC queue.
Only the descriptor exposes that payload to `LiveInput`.
Both queues are allocated when the plugin is constructed;
publication reserves once per callback and neither allocates nor waits for the consumer.
The sample queue holds 131072 floats and the descriptor queue holds 4096 callback records.
Very small callbacks can exhaust descriptors first.
Neither queue promises lossless delivery.

Each descriptor carries its frame count,
channel count,
sample rate,
selected input identity,
reset epoch,
first source frame and presentation origin.
Source positions count frames,
not interleaved samples,
including the entire duration of callbacks that were partly or wholly dropped.
A plugin reset or format/input change starts a new epoch at frame zero.
Its presentation origin uses the producer's existing continuous seconds clock,
which advances by processed frames and does not rewind with project transport.
Queued records keep their old metadata across these changes.

`LiveInput` drains only the descriptor count visible at entry,
joining wrapped sample slices in reusable bounded scratch before analysis.
A missing source interval or a new epoch resets the streaming analyzer and its hop phase,
requiring a complete new FFT window.
Previously measured history remains available.
The editor and closed-window scheduler share this same consumer and analyzer;
opening or closing the editor introduces no source epoch.
Offline seeking remains a separate access mechanism.

The first retained frame establishes the source origin of each continuous run.
Every FFT boundary then follows integer frame counts at the existing hop interval,
and every column retains the existing half-window center offset and channel power combination.
The current note `ClockMapper` offset converts the source origin to GUI seconds once per drain.
The analyzer does not estimate another offset from arrival time or smooth at callback boundaries.
Consequently complete columns are identical across callback/drain partitions and consumer delays when the shared heartbeat mapping is held equal.
Different heartbeat observations can still correct GUI timestamps:
the existing mapper includes initial delivery latency and follows callback pauses and later observations.
Audio does not pin a separate permanent offset that could drift away from notes.
A backward clock correction can temporarily place new columns at or before the retained history tail.
Those overlapping columns are deliberately omitted from live history until mapped time passes the tail,
while every FFT hop and the current spectrum curve continue advancing.
Existing history stays sorted for incremental heatmap aggregation;
timestamps are neither clamped nor rewritten.
This is history suppression until mapped dates pass the retained tail after a clock correction,
not lossless recovery.
Offline arrival-dated analysis is unchanged.
Factual note timestamps and take/export clocks are unchanged.

This is a timing and ownership contract improvement,
not an FPS claim.
No extra callback batching or worker was added.
Live display latency still includes callback delivery,
the existing scheduler cadence,
FFT window coverage and presentation scheduling.

## Verification

The plugin regressions compare every column time and quantized bucket across callback sizes 127,
511,
4093 and 50017 frames,
with grouped and delayed drains under the same clock mapping.
Separate coverage checks queued formats,
wrapped stereo frames,
sample/descriptor exhaustion,
source advancement through loss,
reset window refilling,
actual process transport loops/stops/unavailable positions,
and the existing open/closed scheduler handover.

The shared-clock test starts with 250 ms of calibration bias and applies 60 timely observations.
The real incremental spectrogram aggregation regression failed at the first corrected hop before the overlap policy was added.
It now follows the correction through a full newly silent FFT window and recovery,
checks strict timestamp order,
compares every incremental aggregate with a fresh complete-history fold,
and covers exact equality with the old tail.

## Cost measurements — 2026-09-09

A quiet-window native CPU probe ran on an 8-core Apple M1 Pro with Rust 1.92.0.
Other batch builds and probes were paused;
Bitwig and WindowServer remained active,
so this was not an otherwise idle machine.
The scratch executable used `rustc -O` and the repository's compiled dependencies (dev profile opt-level 2,
dependencies opt-level 3).
It compared the old ring publication plus per-sample pop into reusable scratch with the new publication plus bulk copy,
using mono/stereo callbacks of 32,
128,
512 and 2048 frames and one/eight callbacks per drain.
Queues and scratch were preallocated on both sides to measure steady state.
Each case ran 10000 iterations;
a counting allocator observed zero publication/drain allocations in every case.
These are synthetic in-process CPU costs,
not host callback scheduling or display latency measurements.
The original callback's latest-channel atomic store was outside the baseline probe.

Representative stereo results,
with eight callbacks per drain:

| Callback frames | Publication p50 old → new | Publication p99 old → new | Raw drain p50 old → new | Raw drain p99 old → new |
| --- | --- | --- | --- | --- |
| 128 | 0.333 → 0.343 µs | 0.447 → 0.416 µs | 7.208 → 0.208 µs | 15.041 → 0.292 µs |
| 512 | 1.291 → 1.333 µs | 3.151 → 2.864 µs | 28.834 → 0.667 µs | 57.083 → 1.416 µs |

The reduced raw drain cost comes from bulk copying instead of per-sample ring pops.
Copy volume is unchanged:
one float written to the ring and one copied to scratch per retained channel sample (384000 bytes/s per copy at 48 kHz stereo).
`size_of::<Block>()` measured 48 bytes,
so 4096 descriptors add 196608 bytes (192 KiB).
The sample ring remains 512 KiB;
scratch now allocates its bounded 512 KiB eagerly instead of growing as before.
This is additional metadata memory,
not a memory saving.

A separate probe included the full public analyzer feed after draining,
so the new per-descriptor analyzer setup was inside the measurement.
Both sides used the default 8192-frame window,
stereo power combination and the same continuous source grid at 48 kHz.
Each case warmed 100 drains and timed 400 further drains.
Publication,
note mapping,
window scheduling and rendering were excluded from this second measurement.
The small single-callback median often contains no FFT;
the tails and grouped drains are more informative.

| Frames × callbacks per drain | Full drain p50 old → new | Full drain p95 old → new | Full drain p99 old → new |
| --- | --- | --- | --- |
| 128 × 1 | 0.0021 → 0.0015 ms | 0.172 → 0.233 ms | 0.213 → 0.267 ms |
| 128 × 8 | 0.471 → 0.462 ms | 0.576 → 0.531 ms | 0.644 → 0.608 ms |
| 512 × 1 | 0.171 → 0.159 ms | 0.352 → 0.346 ms | 0.403 → 0.399 ms |
| 512 × 8 | 1.751 → 1.729 ms | 1.923 → 1.958 ms | 2.068 → 2.163 ms |

Full-path tails are mixed,
so these single paired CPU runs do not establish a general analyzer speedup or regression.
No end-to-end DAW FPS,
frame-time tail or input-to-display latency claim follows from them.
No native DAW comparison was run and the shared plugin slot was not changed.
The implementation adds no waiting/batching stage;
its timestamp behavior improves retained delayed audio while actual delivery latency still depends on the existing host and display schedules.
