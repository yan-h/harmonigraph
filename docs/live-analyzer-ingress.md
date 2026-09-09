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
