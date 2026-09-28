# Bounded near-layer quad comparison

Follow-up [#1254](https://github.com/yan-h/harmonigraph/issues/1254) to [#1142](https://github.com/yan-h/harmonigraph/issues/1142).
Baseline: `7f88775ed50aca914877276168408ded01ad41fc`.
**Decision: do not port either candidate.** Neither meets the predeclared roughly 5% saving threshold;
residual quads regress across all workloads,
and complete-response quads are slower or approximately tied.
Production renderer changes were reverted.
This directory is a frozen experiment bundle,
not another supported rendering mode.

## What was compared

- **A / A2:** two identical current Optimized controls.
- **Residual near two:** instanced analytic quads replace the nearest two halo gathers at their existing 100% / 60% resolutions.
  Native cores remain.
- **Complete layer 3:** quads draw the second-nearest layer's complete analytic response at its existing native resolution;
  its separate core calculation is omitted.
  The nearest layer stays unchanged.

Both candidates preserve the far-three 75% short-glow group,
wide near support,
atlas bake,
per-star color and memory,
per-layer weighted color/coverage normalization,
and far-to-near composition.
The quads use four vertices,
a conservative 1/16 halo-texel raster margin,
and One/One blending into the existing RGBA16Float layer targets.
Zero coverage is discarded before blending.
At dense 16:9 defaults,
the nearest two padded grids contain 81,341 cells;
at density 0.5 they contain 4,993.
The star population does not increase when export resolution increases.

[#1242's K trial](https://github.com/yan-h/harmonigraph/issues/1242) already tested residual near-only quads without establishing a saving.
This retests that method under today's 100% / 60% near sampling and far-three architecture,
instead of K's older uniform 50% halos.
The complete layer-3 candidate additionally removes core subtraction/add-back.
It is a scatter experiment;
it does not supersede the earlier aligned complete-response **gather** lead for Uniform Full.

## GPU results

Each percentage is the median per-frame change relative to the mean of the two controls in that same round.
Positive means slower.
Absolute columns are individual run medians in milliseconds;
the paired percentage is not calculated from those summary medians.

| Workload | A / A2 ms | Residual ms | Complete ms | Residual change | Complete change |
| --- | ---: | ---: | ---: | ---: | ---: |
| small | 3.154 / 3.190 | 3.629 | 3.454 | +12.29% | +5.39% |
| 1080 | 9.029 / 8.982 | 9.823 | 9.592 | +9.45% | +6.56% |
| 4k | 23.404 / 23.365 | 25.447 | 23.475 | +8.73% | +0.11% |
| sparse1080 | 8.174 / 8.124 | 8.690 | 8.380 | +5.29% | +1.75% |
| 4k-repeat | 21.998 / 21.942 | 24.094 | 22.308 | +8.47% | +0.74% |
| 1080-repeat | 5.806 / 5.799 | 6.484 | 6.083 | +11.88% | +5.51% |
| recorded1080 | 9.692 / 9.815 | 10.440 | 10.211 | +9.24% | +4.00% |
| no-memory1080 | 9.165 / 8.945 | 10.318 | 9.953 | +12.52% | +7.58% |

Small is 926×720 at 1 pixel/point.
1080p is 1920×1080 at 2 pixels/point;
4K is 3840×2160 at 4 pixels/point,
so those two have the same logical pane.
Default density is 10;
the sparse row uses 0.5.
Memory uses the production pickup/release defaults except in the explicitly named no-memory row.
Recorded1080 uses a preprocessed recording-derived field resampled to the probe's grid;
it is a stress input,
not a faithful replay of the original take or an appearance reference.

The first sequence was small → 1080p → 4K → sparse1080p;
the confirmation reversed the dense resolution order,
then tested recording-derived input and memory off.
Two identical controls and both candidates run interleaved in balanced rotating/reversing order.
Each process discards 120 warmup rounds and retains 240 measured rounds.
The star clock advances at 60 Hz through life transitions.
Input history is prefilled and static during the timing runs.
The source grid has 1024 slabs × 3828 buckets,
39.149967 seconds of visible history,
and 90.02282 semitones of pitch span.

All timings use source-BEGIN through dependent composite-END on Apple M1 Pro / Metal.
No local build,
image capture,
or encoding ran alongside the timing suite.
These are offscreen GPU intervals,
not whole-DAW frame times.
Absolute 1080p control medians moved from about 9.0 ms to 5.8 ms between runs;
use within-run comparisons rather than cross-run absolute times.
A/A median spread was at most 1.28% with memory on and 2.41% in the noisier memory-off run.
All eight measured runs are retained;
none were excluded.
A separate 12-frame smoke run validated pipelines and was not used for performance conclusions.
No confidence interval is claimed.

## Image checks

The final timing binary passed `near_quad_image_probe` after the final discard edit.
Both candidates differed from their matched controls by at most **1/255 per channel** in every fixture.
Mean RGB absolute error ranged from about 0.003 to 0.025 on the 0–255 scale.
The expected numerical difference is half-float rounding after each blended contribution,
instead of one final gather store.

Checks cover dense and sparse recording-derived scrolling fields,
96 captured frames after 96 warmup frames at 24 fps for each main sequence,
jitter 0 and 1,
memory on/off,
silence/recovery,
nonzero fractional origin at 125% scale,
and four captured 4K frames at a long-running clock.
The fixtures assert visible motion and actual target layouts.
Representative dense baseline and candidate frames were also inspected visually.
Raw parity measurements are in `image-parity.log`.
Private input hashes and the exact binary hash are in `manifest.json`.

This is bounded parity evidence,
not proof of complete-response eligibility at every origin,
scale,
profile,
or future halo resolution.
A production complete-response path would still need an alignment rule or demonstrated broader coverage.
No such machinery is justified by these timings.

## Reproduction and scope

Use an owner-managed experiment worktree at the pinned baseline,
then apply `experiment.patch`.
Do not load the scratch renderer into the shared DAW slot.
The patch deliberately adds test-only mode switches and one-off probes;
it is evidence rather than a proposed production diff.
The common bind-group layout exposes the star atlas and cloud uniform to vertex shaders for all cases;
controls retain the original WGSL source.
No final port against untouched production layouts is claimed.

```sh
git apply /path/to/experiment.patch
cargo test --release -p harmonigraph-render --no-run \
  --config 'profile.release.package.harmonigraph-render.opt-level=1' \
  --config 'profile.release.package.harmonigraph-render.codegen-units=64'
python3 /path/to/run-comparison.py screen
python3 /path/to/run-comparison.py repeat
```

The driver uses the current directory as the experiment root;
`QUAD_REPLAY_ROOT` overrides it and `QUAD_RESULTS` changes the output directory.
Synthetic `screen` replay is self-contained.
The `repeat` recording-derived row and image probe require the private files at the paths recorded in the manifest;
those files and raw images are not committed.
The shader and wgpu/Naga dependencies retain their release optimization;
the host renderer test crate uses opt-level 1 / 64 codegen units to limit build cost.
No CPU-performance conclusion is drawn.
Run the resulting test executable with `near_quad_image_probe --ignored --nocapture --test-threads=1`,
setting `HARMONIGRAPH_SHADER_ASSETS=source HARMONIGRAPH_REQUIRE_GPU=1`,
to repeat image checks.

Raw per-frame samples,
summary JSON,
and replay code accompany this report.
`git apply --check` passed against the restored baseline after the experiment.
The implementation does not isolate vertex cost,
overdraw,
blending,
or shader arithmetic as the cause of the regression.
It rules out these two bounded constructions as worthwhile optimizations on this workload and GPU,
not every possible sprite renderer.
