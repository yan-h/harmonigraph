# Preserve the nine-neighbor look: second optimization round

Measured on Apple M1 Pro / Metal on 2026-09-26 against `3a3846fd`.
Yan rejected the shorter-halo four-neighbor appearance: “I do prefer A”.
This round retains nine neighbors and five depths,
and tests cheaper falloff evaluation and skipping immediate-color work when color memory supplies the result.
Neither candidate is enabled in production.

## Outcome

The falloff lookup preserves the appearance closely but costs substantially more GPU time.
The color-work cleanup matches the two captured frames exactly,
but its measured reduction ranges from 0.2% to 3.3% and nearly disappears in the longer 4K run.
Neither earns a production change on this evidence.
The cleanup remains a small candidate if future profiling establishes a useful gain;
these results do not prove that its true benefit is zero.

| Paired run | Current A | Color cleanup | Cleanup reduction | Falloff lookup | Lookup increase |
|---|---:|---:|---:|---:|---:|
| 1080p, 120 frames | 11.634 ms | 11.496 ms | 1.2% | 16.358 ms | 40.6% |
| 1080p, 240 frames | 14.340 ms | 13.862 ms | 3.3% | 19.534 ms | 36.2% |
| 4K, 120 frames | 30.632 ms | 30.328 ms | 1.0% | 36.504 ms | 19.2% |
| 4K, 240 frames | 36.055 ms | 35.969 ms | 0.2% | 42.398 ms | 17.6% |

The first lookup uses an analytic fringe fallback past its table domain;
the other three use a zero tail without that exponential fallback.
Rows are separate runs under varying shared-machine load:
compare candidates within each row,
not absolute durations between rows.
No formal significance claim is made for the small cleanup differences.

## What was measured

Every case uses the current `SpectralAtmosphere::default()` with Stars selected,
including density 10,
size 1.0045346–11.529175,
lifetime 0.8390586 seconds,
and color pickup/release 0.08/0.6 seconds.
The 16:9 atlas holds approximately 4.19 million cells.
The probe uses full history coverage,
a ten-second span,
1024 slabs of 3828 buckets,
and two pixels per point.

Cases are interleaved once per frame after ten warmup rounds.
Confirmation runs reverse the candidate order relative to their screening runs.
Numbers are the median `end/full` samples printed in the detailed min/p10/med/p90/max rows,
covering light preparation,
color memory,
star baking and final drawing.
These are synchronous offscreen GPU intervals,
not full DAW frame times.
All variants use WGSL source mode and the same extra lookup binding and resource,
including A,
so differences do not include allocating a new resource per frame.
The table is uploaded once per target allocation,
outside measured steady state.

- [1080p screening](1080-screen.log)
- [1080p confirmation](1080-confirm.log)
- [4K screening](4k-screen.log)
- [4K confirmation](4k-confirm.log)

## Cost probes

Replacing the entire nine-neighbor walk with one dependent atlas read leaves 3.028 ms at 1080p and 4.383 ms at 4K,
against 11.634 and 30.632 ms for A in those same screening runs.
The light,
memory and star-bake passes remain encoded,
and the final shader still reads the completed atlas.
This implicates the final walk as the dominant remaining cost;
it is an ablation,
not a precise additive timing of individual passes.

Replacing just the two falloff exponentials with constants leaves 12.869 ms at 1080p and 29.352 ms at 4K.
The result is slower in the first run and only about 4% faster in the second.
Changing shader expressions can change compiler scheduling and register use,
so this does not measure an isolated exponential instruction cost.
It does show that removing those expressions did not yield the large saving the lookup idea needed.

## Candidates and picture checks

`cleanup` skips the bake's light sample and immediate palette calculation when remembered color supplies its output.
The color-memory pass still samples the current light and computes its target color.
The disabled-memory path's arithmetic is moved into a branch;
the existing shader documents rounding sensitivity there,
and this round does not validate that path for shipping.

`lookup` stores `exp(-0.5 u²)` and `exp(-0.4 u)` in a 4096-entry RG16Float texture over `u = distance / sigma` from 0 to 32.
One linearly filtered sample supplies both components.
The original ring window,
life fade,
coverage clamp,
star count,
jitter and compositing remain intact.
`lookup_zero` selects zero beyond 32 instead of evaluating the analytic fringe tail.
At maximum Fringe 0.5,
the omitted contribution beyond 32 is at most about 0.00000138 per star before the remaining fades.
Neither version improves performance.
`combined` adds the cleanup to the first lookup and is also slower in the screening run.

Two native 1920×1080 frame pairs use the real-recording light input from the look-prototype kit and a flat level of 128/255,
with the production default gradient,
two seconds of color-memory warmup at 60 Hz,
and the actual production shader/layout path.
The recording frame begins around a 72–92 second window of `take-2026-09-11_03-16-17.wav`.
This is not a full saved-Bitwig-project or analyzer replay.
The matched crops were visually inspected.

| Candidate | Recording: mean / max RGB difference out of 255 | Flat: mean / max |
|---|---:|---:|
| Color cleanup | 0 / 0 | 0 / 0 |
| Lookup with zero tail | 0.00162 / 1 | 0.00142 / 1 |

[Raw image metrics](image-metrics.json) and the [baseline](base-images.log),
[cleanup](cleanup-images.log),
and [lookup](lookup_zero-images.log) render logs record those checks.
These two frames establish neither an error bound across every control nor motion parity.
No golden was re-baselined.

## Reproduce the timings

Use a clean owner-managed worktree at the measured source plus these evidence files.
The preparation script verifies the shader hash and writes variants only to scratch.
The harness patch deliberately uses `/private/tmp/stars-round2` for its shader and lookup inputs.
It adds scratch resource bindings and test hooks;
do not ship it or build a plugin from it.

```sh
python3 docs/evidence/spectrogram-stars/round2/prepare.py
git apply --check docs/evidence/spectrogram-stars/round2/harness.patch
git apply docs/evidence/spectrogram-stars/round2/harness.patch
HARMONIGRAPH_SHADER_ASSETS=source PROBE_COMPARE=base,cleanup,lookup_zero \
  PROBE_CASE=stars PROBE_FILLS=1 PROBE_FRAMES=240 PROBE_SIZE=3840x2160 \
  cargo test --release -p harmonigraph-render cloud_costs_by_style_and_dial \
  -- --ignored --nocapture --test-threads=1
git apply -R docs/evidence/spectrogram-stars/round2/harness.patch
```

Set `PROBE_SIZE=1920x1080` for 1080p.
For the full screening set,
use `PROBE_COMPARE=base,one_read,no_falloff,cleanup,lookup,combined` and 120 frames.
The retained raw logs list the exact case order for each run.
The frame harness is retained in the patch,
but its recording-derived byte inputs are not repository assets;
the synthetic timing probe above does not need them.
All temporary production-source changes were removed after the measurements.
