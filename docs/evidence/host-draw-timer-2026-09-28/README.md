# Host draw timer overhead

The remaining host-timer question in [#1182](https://github.com/yan-h/harmonigraph/issues/1182) was measured through the native parented editor,
with the performance overlay hidden.
The normal scheduling runs suggest a small added renderer wall cost,
about **0.073 ms per frame** in this static fixture.
The separate GPU-sensitive experiment is too noisy to resolve the timer's GPU cost.
This does not establish zero overhead or a live DAW frame-time improvement.
Keep the production timer unchanged;
this small bounded measurement does not justify adding an overlay-dependent timer lifecycle.

## Results

Each cell reports the median of four run medians,
with their range in parentheses.
The on/off order was on, on, off, off, on, off, off, on,
including repeated identical settings to expose ordinary variation.

| Mode and measurement | Timer on, ms | Timer off, ms | Interpretation |
| --- | ---: | ---: | --- |
| Normal scheduling: renderer wall time excluding surface acquisition | 1.077 (1.030–1.086) | 1.005 (0.975–1.009) | Small separation, about 0.073 ms; includes present, so this is not pure CPU work. |
| Serialized: queue submission through GPU completion | 3.116 (2.508–4.729) | 3.032 (2.478–3.590) | Unresolved: identical-on runs differed by 0.988 ms, much more than the aggregate difference. |

The second mode waits for the submitted work before presenting,
so it changes normal pipelining and is only a completion-time probe.
Its large variation rules out attributing its aggregate difference to the timer.
The normal-mode `completion` CSV column is submission wall time only,
because that mode does not wait.
Do not combine medians of separate stages or interpret the two scheduling modes as equivalent workloads.

## Fixture and controls

- Apple M1 Pro, macOS 27.0, Metal, release build at baseline `3405f09746915a8710791d9fb40b2fe5fe974cba` plus [the scratch patch](probe.patch).
- The existing `editor-startup` example opens one real parented editor,
  using production graphics setup and the complete default static UI/callback path,
  then closes itself.
  No DAW, recorded take or shared plugin slot is involved.
- Every run warms 120 distinct painted frames and measures the next 240.
  All 16 runs completed normally.
  Each measured frame has the same 2000×1400 physical size at 2 pixels per point,
  23 egui primitives and 4,164 vertices.
- The device requests the same features in both arms.
  Only construction of the host `DrawGpuTimer` is disabled in the off arm;
  the lattice preparation timer remains the same in both.
- The enabled host timer was armed on 120 of each run's 240 measured frames;
  the disabled control was armed on zero.
  Results describe average product behavior,
  not cost per timestamped frame.
- Samples are buffered in preallocated vectors and printed during teardown,
  outside the measured loop.
  The UI's previous-frame readings are matched to successful draws by the paint counter.
  The runner rejects repeated UI samples, missing draw records or changed workload counts.
- The serialized mode uses the same `queue.submit` followed by `device.poll(PollType::wait_indefinitely())` in both arms,
  before presentation.
  It measures elapsed submission through completion,
  not an independent timestamp pass that could overlap or undercount the real work.
- All runs required the strict shader corpus and reported 84 libraries loaded,
  no source compilation, no rejected assets and no load failures.

The machine remained an ordinary desktop with background applications.
The static default scene contains no incoming notes or audio,
so these results do not bound every active Stars view or host-contention scenario.
There is no stable isolated GPU delta here to turn into a promised optimization.

## Evidence and replay

[The manifest](manifest.json) records the platform, baseline, executable hash, patch hash and run order.
[Samples](samples.csv) retain all 3,840 warmed frame measurements and their workload/counter fields.
[Run summaries](summary.json) include means and medians for each component.
The initial logging smoke test was discarded before measurement;
its per-frame stderr writes were removed from the final probe.

Copy this evidence directory outside the checkout to `/tmp/host-draw-timer-evidence` before selecting the baseline,
because that older commit does not contain these evidence files.
From the repository root of an isolated checkout of the baseline,
run:

```sh
git apply /tmp/host-draw-timer-evidence/probe.patch
cargo build --release -p harmonigraph-plugin --example editor-startup --features startup-probe
python3 /tmp/host-draw-timer-evidence/run.py /tmp/host-draw-timer-repeat
```

It opens and closes temporary native windows,
requires the fixed workload above,
and saves raw diagnostic logs alongside the extracted CSV and summaries.
Restore the two instrumented source files after replay.
Production source was restored after this experiment.

The same change removes `PROBE_LOCAL_SHADOW`,
whose claimed independent notation-shadow pass no longer exists after #1213.
Zeroing only resting-cross depth no longer measured the advertised workload.
