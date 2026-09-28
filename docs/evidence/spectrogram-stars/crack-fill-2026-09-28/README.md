# Distant Stars crack-fill comparison

Follow-up to [#1256](https://github.com/yan-h/harmonigraph/issues/1256) and [#1142](https://github.com/yan-h/harmonigraph/issues/1142).
Base: `7f88775ed50aca914877276168408ded01ad41fc`.
This evidence-only prototype restores production source and defaults.
No appearance has been selected or ported.

## Brief and result

Yan's target is the black cracks behind the backmost layers showing through and making the texture grainy.
Foreground stars obscuring other stars are not the requested defect.
High average opacity does not make narrow high-contrast cracks unimportant.
The previous far-coverage report's recommendation to redirect the task toward foreground occlusion was withdrawn in the issues and PR #1257 description.
Its numerical measurements remain useful.

All candidates retain the same three far layers at 75% width and height,
all original stars and memory identities,
and the two near layers.
A retains the current coverage;
B squares the remaining background leakage,
and C raises it to the fourth power.
The enlarged far-only crops show a localized reduction in dark notches,
stronger in C than B,
while substantial texture remains.
The full-image effect is subtle in these crops.
These measurements establish reduced partial-coverage gaps,
not that the perceived cracks are solved or that all dark grain is geometric leakage.
C is the stronger candidate to judge visually,
not a selected default.

Both variants effectively tie the baseline in dense 1080p and 4K screen/repeat measurements:
paired differences stay within 0.4%.
There is no demonstrated speedup and no meaningful added GPU cost in this bounded fixture.

## Construction

Let g be the product of one minus each capped far-layer coverage,
F the existing far composite,
and b its gamma-coded palette background.
B returns `b + (F-b)*(1+g)`;
C returns `b + (F-b)*(1+g)*(1+g*g)`.
Their effective coverage is respectively `1-g*g` and `1-g*g*g*g`.
An originally 50%-covered pixel becomes 75% or 93.75% covered.

The correction uses the same capped coverage and color gathers as existing composition,
before the RGBA16Float far target write and bilinear sampling.
It adds no star samples,
layers,
textures,
passes or atlas/memory entries.
The near pass returns its original color computation.
A's normal shader source is unchanged exactly;
the dedicated control test passed.
Unlike the previous extra-layer experiment,
all modes retain the original five-descriptor uniform layout.

Gain is bounded by two or four;
there is no division by near-zero coverage.
Zero star contribution remains the background,
and full coverage is unchanged mathematically.
Stars colored like the background also stay dark.
The adjustment strengthens partial tails and lifetime fades;
it does not expand support or guarantee continuous coverage.
Other palettes,
dial extremes,
silence/recovery and live transitions remain production-port checks.
Only Optimized/P3 is tested for the effect,
with each scratch mode fixed for its resource lifetime.

## Coverage and picture measurements

The flat diagnostic outputs the effective opacity before normal far-target filtering,
not by applying the formula to a previously filtered mask.
Level is fixed at 0.6;
96 warmup frames precede 24 measured frames at 24 fps.
The ROI is x=16..1415 and y=64..823.
The helper allocates 1537x865 for a 1536x864 pane;
the extra border is excluded.
Thresholds use final 8-bit codes.

| Candidate | Pixels below 230/255 opacity | Pixels below 128/255 opacity |
| --- | ---: | ---: |
| A: current | 1.030905% | 0.00389646% |
| B: gentle | 0.107824% | 0.00053650% |
| C: strong | 0.014074% | 0.00009398% |

These are about 9.6x and 73x fewer pixels below the first threshold,
not that many times fewer perceived cracks.
Residual low coverage remains.
No sampled code was zero in this flat ROI,
which does not prove absence of zero far-target texels or holes at other settings.

In actual far-only views,
fixed lit/low-coverage pixels selected from A brighten by mean weighted RGB values of 15.48/255 for B and 21.19/255 for C.
The filter is A far opacity below 230/255 and source level above 76/255.
The full-image subset additionally requires near opacity below 230/255;
its mean increases are 4.62/255 and 6.32/255.
These gamma-coded RGB proxies are descriptive,
not physical luminance or perceptual acceptance scores.
Full-image mean absolute channel changes are 0.034/255 and 0.041/255;
this describes localization and must not dismiss the cracks.
Precise filters and all counts are in `metrics.json` and `analyze.py`.

The prior A diagnostic movie supplies these fixed masks,
using the same base,
take,
appearance and clock.
Its capture metadata and raw hash are preserved here.
Fresh A matches the untouched renderer byte-for-byte across 144 frames,
so the diagnostic reuse is registered to the same picture.

## GPU cost

Seven sequential synthetic M1 Pro/Metal runs use 120 warmup and 240 measured frames per case.
A/A2 identical controls interleave with B/C;
order rotates and periodically reverses.
The source-BEGIN through dependent-composite-END interval includes bake,
color memory,
near halos,
far response and composition.
Capture,
encoding and compilation finished before timing.
All runs are retained without exclusions.

Times are milliseconds;
positive overhead is slower.
Overhead is the negative median of paired per-frame savings against the A/A2 mean,
not a ratio of separately computed medians.

| Fixture | A / A2 median | B median | C median | B overhead | C overhead |
| --- | ---: | ---: | ---: | ---: | ---: |
| Small 926x720 | 3.039 / 3.020 | 3.021 | 3.045 | -1.39% | -2.32% |
| 1080p screen | 5.308 / 5.273 | 5.319 | 5.317 | +0.30% | +0.29% |
| 1080p repeat | 5.399 / 5.390 | 5.421 | 5.410 | +0.08% | -0.37% |
| 4K screen | 15.716 / 15.736 | 15.746 | 15.835 | +0.17% | +0.31% |
| 4K repeat | 17.515 / 17.243 | 17.431 | 17.154 | +0.13% | -0.01% |
| 1080p sparse | 4.485 / 4.489 | 4.478 | 4.474 | -0.47% | -0.40% |
| 1080p memory off | 5.045 / 5.072 | 5.051 | 5.070 | -0.08% | -0.08% |

A/A median differences span -1.55% to +0.53%.
Tiny dense differences and the small-pane apparent savings are not optimization claims.
Absolute timings drifted between runs;
within-run comparisons are the evidence.
Dense density is 10,
sparse is 0.5,
and memory-off disables both pickup and release.
History is fully populated over 39.149967 seconds and 90.02282 semitones.
These are synthetic offscreen intervals,
not actual-take or whole-DAW timings.

## Visual artifacts and provenance

Captures use the private take-2026-09-11_03-16-17.take and saved `appearance.ron`.
They render 46 seconds from 46.766945 to 92.766945 at 1536x864,
scale 2,
and 24 fps;
the first 40 seconds warm visible history and color memory.
Each retained movie has 144 frames.
Capture JSON files record commands and binary hashes;
`baseline-parity.json` records exact control parity.
Private raw frames and media remain local;
locations and media hashes are in `manifest.json`.

The full and far-only 2x/4x sheets and 2x movies share the 256x160 crop in `crop.json`,
selected from A's low-coverage mask and weighted by source and near transmittance.
That is a targeted diagnostic region rather than a perceptual classifier;
the independent reviewer noted that it can be visually weak despite mathematical targeting.

The 96x64 detail in `far-notches-6x.png` and its matching movie uses `detail-crop.json`:
positive darkness against a 7x7 local mean of A far-only RGB,
weighted by far transmittance and gated by source level.
Selection uses A alone,
not B/C changes.
This provides a high-contrast dark-notch example alongside the broader views.
All magnified pixels use nearest-neighbor scaling.
Stills use frame 72;
movies retain all six seconds at the same fixed crop.
The observation is localized filling,
not disappearance of grain or a demonstrated motion/flicker improvement.

## Replay and limits

Copy this evidence outside a disposable owner-managed worktree at the pinned base,
then apply `prototype.patch` there.
Do not load the scratch plugin as production.
The two shader files are the actual normal B/C shader strings captured from the offline binary.
Build harmonigraph-offline in release,
then build the harmonigraph-render release tests with package opt-level=1 and codegen-units=64 as recorded in the prior comparison.
Run `run-timing.py screen` and `run-timing.py repeat` with FAR_REPLAY_ROOT set to the checkout and FAR_TIMINGS set to the output directory.
The runner retains those variable names from the prior one-off experiment.
Synthetic timing requires no private input.

Run `control_is_original_shader` on the render test executable without diagnostic environment variables.
For flat masks,
set HARMONIGRAPH_SHADER_ASSETS=source,
HARMONIGRAPH_REQUIRE_GPU=1,
HG_FILL_DEBUG=2,
HG_FILL_FLAT=1,
and FILL_OUTPUT to the flat output directory;
run `crack_coverage_image_probe --ignored --nocapture --test-threads=1`.
Unset diagnostics before normal captures or timing.

Capture,
analysis and presentation scripts preserve their original machine-local paths,
including the prior pristine and diagnostic captures;
adjust them for replay elsewhere.
They require NumPy,
the included PNG helper,
and ffmpeg at its recorded path.
Run `capture-all.py`,
`analyze.py`,
`present.py` and `detail.py` with the private inputs available.
The flat helper's unconditional-return spelling was repaired before building the test binary;
that branch is inactive in every normal offline capture,
and normal shader strings match the frozen generator.
Production files were restored and patch applicability checked before committing this evidence.
No production golden or Metal corpus change is included.
