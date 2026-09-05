# #615 Bitwig timing apparatus

Status:
[Bitwig measurements support a constrained configuration](tuning-probe-bitwig.md):
fixed 128/512/1,024-sample buffers at 44.1 kHz, calibrated fixed routing and continuous callbacks, with D = 2,048 samples.
Production late-stream and transport-stop behavior remains an explicit #616 contract question, recorded in [#632](https://github.com/yan-h/harmonigraph/issues/632).
The selected design lives in [adaptive-tuning.md](adaptive-tuning.md).
This document describes a disposable experiment, not an implementation of #616 or #617. The [frozen evidence archive](evidence/615/README.md) preserves the measured sources, captures and reproduction fixtures independently of this probe's future lifetime.

Keep the optional apparatus while it is needed for lower-delay measurements or #632's retiming and stop experiments.
When production tests cover its useful timing, lifecycle and host-boundary cases, remove the duplicate probe implementation, analysis command, feature wiring and unused trace hooks.
Retain the measured findings and the CLAP activation-ordering fix with a focused regression test that does not require the retired apparatus.
The archived source bundle preserves the original experiments after that removal.

## Build and configure

```
cargo build --release -p harmonigraph-plugin -p harmonigraph-offline --features harmonigraph-plugin/tuning-probe
python3 tools/tuning-probe.py prepare --delay 2048 --sources 3
```

Ordinary builds export the existing Harmonigraph class only.
The opt-in build additionally exports `Harmonigraph Tune — probe` from the same CLAP bundle, with the same vendor.
The tuner has no editor or analyzer and advertises no overlapping-voice support.
Its one host parameter, `Probe split`, exists solely to trigger sample-accurate automation splits.
The full Harmonigraph becomes the probe hub for CLAP;
its ordinary VST3 behavior is unchanged.

Configuration is read at activation from `~/.cache/harmonigraph/tuning-probe/config.json`.
The CLI and plugin use the same per-user directory;
`HARMONIGRAPH_PROBE_DIR` overrides it for an isolated run.
The plugin refuses to overwrite an existing trace filename.
Historical captures used the former `/tmp/harmonigraph-tuning-probe` directory;
their archived paths and configurations are preserved as recorded.
The script also creates `three-tracks.mid`, a type-1 MIDI file with simultaneous attacks, CC events, releases, short notes and silent intervals.
Import its tracks into a disposable Bitwig project.
Put a tuner before Polysynth on one track, before Vital VST3 on another, and before an Instrument Layer on the third;
put the full Harmonigraph on Master.
ValhallaShimmer can be added after an instrument for the branch-effect trial;
record its actual reported latency rather than assuming a reverb has nonzero latency.

The delay is a candidate in samples, fixed per activation.
No default value is a measured bound.
Use `--late N` to withhold source slot 0's first assignment until its planned deadline plus N mapped samples.
Use `--allow-sleep` to return the framework's ordinary sleep-eligible status instead of requesting continuous callbacks.
Every instance must read the same configuration;
reactivate all of them after a change.
Keep separate copies of the configuration and traces for each trial.

The repository's shared-slot contract still applies:
build here, then load the chosen branch with `./load-plugin.sh codex/615-tuning-probe` when the DAW is available.
Do not copy over a running plugin.
Under the measured `by Vendor` setup, right-click the chip icon near the top middle, deactivate the audio engine, load the build, then click Activate Audio Engine.
Verify the actual build tag with `./load-plugin.sh --tag` and the hub's performance overlay.
The existing [#623 Test A evidence](https://github.com/yan-h/harmonigraph/issues/623#issuecomment-5547518334) covers reload gestures;
it does not establish two-class grouping or timing.

## What the apparatus observes

The vendored nice-plug hook is opt-in per class and compiled out of ordinary processing.
It records the original CLAP callback address, instance-local callback number, raw `steady_time`, frame count, audio-buffer latencies, transport and raw note events.
It observes exact Rust sub-block entry/exit and the final host `try_push` result for each output event.
It also counts actual CLAP latency queries;
reporting a latency internally is not treated as evidence that the host queried it.
The hub records stereo peak and squared energy for each sub-block, without storing audio samples.
This permits silent downstream voice checks while physical monitoring remains disabled;
mixed-track energy does not identify individual voices, so isolate the relevant track for lifecycle trials.
The pointer address is supporting evidence only;
hosts may reuse it on every callback.

CLAP explicitly permits unrelated per-instance `steady_time` origins and permits an unavailable value of -1. The probe's candidate mapping is raw steady time plus an explicitly configured per-source/hub offset, with the exact sub-block start added.
No automatic offset calibration is performed.
Equal initially coactivated counters are insufficient:
add a tuner later, reactivate an individual tuner, change branch latency and reset the engine before accepting that mapping.
Raw transport values remain separate from nice-plug's derived transport, including at in-buffer transport changes.

Each source submits onsets through its own preallocated SPSC queue and publishes exclusive progress through that same ordered queue, including silence.
The hub only assigns samples strictly below every expected source's progress.
It orders eligible attacks by input sample, key, channel, source and request, and returns an artificial +0.5-semitone reply.
The source never computes that correction.
Membership is fixed by the expected source count for a trial;
a missing participant stalls completeness.
Progress records include the start of each covered interval as well as its exclusive endpoint.
Noncontiguous progress invalidates a run;
after a source rejoins, pending attacks before its new coverage start also invalidate the run.
The hub uses one source-membership snapshot throughout each callback's completeness calculation.
This is deliberately not session discovery, automatic recovery or a musical policy.

The tuner delays the framework's complete supported performance stream, emits note-on followed by `PolyTuning` at the same sample, and adds the frozen correction to later per-note tuning.
Initial note-on, correction and same-sample player expression remain separately visible in the raw output trace.
Host acceptance still does not prove that the instrument used the expression from the first audio sample;
record and inspect downstream audio for that result.

Input, deadline, reply production/visibility and requested output time are distinct trace records.
Only accepted `raw_output` events establish that the host received an event at its recorded callback offset.
The source's local output bookkeeping precedes the framework's host push, so an output rejection invalidates the experiment instead of becoming confirmed musical state.

## Late events and lifecycle limits

A missed deadline retains the pending attack and latches a trace/host-log diagnostic.
A valid late reply remains usable.
The probe translates the remaining track stream together by the extra lateness and retains that extra delay until reset.
This preserves queued note duration in the isolated late-note fixture, but can extend already-sounding notes and separate attacks across tracks.
It is an experimental scheduling rule, not a selected #616 return-to-D policy.
Use the measured results to specify that policy before implementing the feature.

Source reset increments its generation;
hub reset increments the process-local session epoch.
Obsolete replies cannot address a new pending lifetime.
The trace reports the held and pending counts at reset/deactivate/stop.
A changed hub epoch with existing source lifetimes invalidates the run until the host explicitly resets them.
The probe does not claim that local reset terminates an instrument's downstream voices.
Measure bypass, mute, sleep, removal and held-note behavior independently, including the sound after callbacks stop.

Same-key overlap, wildcard channel/key events the framework cannot preserve, tuning without an addressed lifetime, unavailable steady time, queue exhaustion and rejected output abort the experiment with a visible diagnostic.
These are unmeasured or unsupported apparatus cases, not authorized production drop/fallback policies.
Restart the fixture after an invalid run.
SysEx outside the framework's `()` message support and MIDI 2.0 are outside this apparatus.

## Bounded storage and analysis

There are at most eight tuner slots, 4096 entries per SPSC queue, 2048 pending performance events per source, 2048 hub pending requests and 2048 withheld replies.
Per-callback loops have those explicit bounds;
the hub's sort is in-place and allocation-free.
The process-local registry keeps the bounded shared allocation alive;
endpoints are acquired and returned only during construction/destruction outside processing.
No callback takes the registry lock, allocates, formats records or writes files.
Only the hub's audio callback produces assignments, including during offline execution.

Each instance has a 65,536-record SPSC trace ring and an independent lost-record counter.
An off-thread drainer writes JSONL and fault diagnostics;
its speed never decides assignments.
Overflow or I/O failure invalidates evidence.
A completed trace has a footer with zero loss and successful I/O;
a missing footer means the instance is still running or the file is incomplete.
Full callback tracing perturbs timing, so record that instrumentation configuration when deriving a conservative supported bound.

```
python3 tools/tuning-probe.py analyze --output /tmp/probe-summary.json
cargo test -p harmonigraph-plugin --features tuning-probe,nice-plug/assert_process_allocs probe::tests:: -- --nocapture
```

The analyzer lists process/class/instance identity, callback sizes, sub-block starts, latency queries, note IDs, faults and request/reply stages.
It deliberately does not turn observed maxima or equal counters into a viability verdict.
The exported-CLAP fixture checks the apparatus against split callbacks, silence completeness, expression composition, withheld and obsolete replies, unavailable clocks and rejected host output.
Those are hostless correctness checks;
they are not Bitwig observations.

## Repeating or extending the host evidence

Record Bitwig version, sample rate, buffer setting, negotiated maximum interval, hosting mode and per-plugin overrides for every trial.
The [completed measurement matrix](tuning-probe-bitwig.md) records shared-clock mapping and resets, complete source intervals, callback-order permutations, round-trip opportunities, candidate D and margin, compensation, processing modes, lifecycle and pitch behavior, with explicit exclusions.
Extending that supported configuration requires fresh host evidence for the changed assumption.
Include editor-never-opened playback and export.
Match actual accepted output to downstream recorded audio and link #623's distinct-class process and reload evidence.
Hostless correctness checks do not expand the measured supported configuration.
