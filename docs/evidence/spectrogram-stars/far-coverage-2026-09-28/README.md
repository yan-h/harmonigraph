# Three versus five distant Stars layers

Bounded comparison for [#1256](https://github.com/yan-h/harmonigraph/issues/1256),
following [#1142](https://github.com/yan-h/harmonigraph/issues/1142).
The production base is `7f88775ed50aca914877276168408ded01ad41fc`.
This evidence-only change leaves the production renderer and defaults unchanged.

## Decision

Do not adopt five distant layers as a fix for the large dark pockets in this recording.
It fills the small remaining geometric holes,
but the three existing distant layers already average 99.45% composited opacity in lit areas.
Five raises that to 99.985% while retaining much of the visible dark pattern.
Star colors and near-layer overlap therefore deserve the next bounded look experiment.
A dim star can cover brighter material behind it;
adding stars behind an opaque foreground cannot correct that.
Reducing dark-star occlusion or changing their light sampling is a hypothesis,
not a measured optimization or an approved change of appearance.

Five half-resolution distant layers roughly tie today's renderer at 4K,
but cost 20–21% more at dense 1080p and about 21% more in the small pane fixture.
Keeping three and lowering their target to 50% saves 9–10% at 4K with slight distant softness;
it shows no dependable dense-1080p saving here.
These are synthetic offscreen GPU measurements on Apple M1 Pro / Metal,
not whole-DAW or actual-take frame-time claims.
No appearance has been selected or ported.

## Candidates and controls

| Candidate | Far layers | Far image width and height | Near layers |
| --- | ---: | ---: | --- |
| A | 3 | 75% | Existing two, unchanged |
| B | 3 | 50% | Existing two, unchanged |
| C | 5 | 50% | Existing two, unchanged |

Original physical indices 0–4 retain depths 0, 0.25, 0.5, 0.75 and 1,
including their salts, atlas bases, cell sizes, core sizes and memory identity.
C appends physical slices 5 and 6 at depths 0.125 and 0.375,
with independent salts,
and composites in physical order 0, 5, 1, 6, 2, 3, 4.
It does not redistribute the original depths or reduce the nearest layers' resolution.
The same four-cell short-support gather is used for every distant layer.
The atlas grows from 1,192,570 to 1,975,318 logical texels,
below its 4,194,304 cap and without changing cell sizes to fit.
That extra baking and color-memory work is part of the measured interval.
A smaller far image alone cannot make that cost disappear.

The layout identity and uniform binding checks passed;
see `identity-check.log`.
A and the untouched renderer match byte-for-byte across all 144 full-size captured frames;
see `baseline-parity.json`.
The original near layers' diagnostic opacity also matches exactly between A and C across the entire analyzed movie ROI.
This establishes the tested controls,
not correctness for every dial or profile.
All scratch candidates use seven uniform descriptors,
with the appended grids inactive in A and B.
Pixel parity with the untouched binary does not establish exact timing parity with its five-descriptor layout;
small performance differences should not be interpreted as production guarantees.

## GPU costs

Every case uses 120 warmup frames and 240 measured frames.
A second byte-identical control A2 is interleaved with A, B and C;
the case order rotates and periodically reverses.
Each case owns its resources and holds one mode for its lifetime.
The interval is source-BEGIN through dependent-composite-END,
including atlas, memory, near halos, far response and final composition.
There are no partial-pass savings substituted for full cost.
Runs were sequential without concurrent capture, compilation or encoding.
Raw sample vectors and fixture controls are in `timing/`.

Positive saving means faster than the paired mean of A and A2.
The percentage is the median of per-frame paired savings;
it need not equal a ratio of the independently reported medians.
All costs are milliseconds.

| Fixture | A / A2 median | B median | C median | B saving | C saving |
| --- | ---: | ---: | ---: | ---: | ---: |
| 926×720 | 4.789 / 4.873 | 4.679 | 5.712 | +2.76% | -20.71% |
| 1080p screen | 7.506 / 7.514 | 7.617 | 8.911 | -1.22% | -21.10% |
| 1080p repeat | 6.369 / 6.398 | 6.407 | 7.667 | -0.65% | -19.81% |
| 4K screen | 22.261 / 22.096 | 20.331 | 22.146 | +9.78% | +0.69% |
| 4K repeat | 17.898 / 18.038 | 16.313 | 18.012 | +9.45% | +0.45% |
| 1080p sparse (density 0.5) | 5.600 / 5.589 | 5.078 | 5.438 | +7.60% | +2.13% |
| 1080p no color memory | 5.997 / 6.008 | 6.032 | 7.012 | -0.82% | -16.84% |

A/A median differences range from −0.74% to +1.76%.
Absolute timings shifted between screen and repeat,
so compare within runs rather than across them.
Both dense 1080p and 4K repeats support the same decision;
all runs are retained and none was excluded.
The small sparse-C gain has only one run and is not a general speed claim.
Dense defaults include color memory;
the no-memory row disables both pickup and release.
The timing fixture is synthetic,
with full history, 39.149967 seconds of span and 90.02282 semitones.
The 1080p and 4K fixtures preserve logical size at 2 and 4 pixels per point.
No actual recording-derived GPU timing was collected in this bounded comparison.

## Coverage and the dark pockets

The diagnostic uses the actual star responses:
far opacity is `1 - product(1 - min(sum_of_star_coverages_in_layer, 1))`.
It is evaluated in the candidate's far target and sampled normally by the final pass.
This is mean opacity and transmittance,
not a binary count of sky area containing any star.
The final 8-bit channel values introduce quantization;
threshold counts below use code values explicitly.

A flat level of 0.6 removes lighting variation as an explanation for dark masks.
After 96 warmup frames,
24 frames at 24 fps give the following central-ROI results:

| Candidate | Mean far opacity | Pixels below 230/255 opacity | Mean uncovered weight |
| --- | ---: | ---: | ---: |
| A | 99.4496% | 1.0309% | 0.5504% |
| B | 99.4523% | 0.8275% | 0.5477% |
| C | 99.9848% | 0.00433% | 0.01520% |

The flat helper allocates a 1537×865 image for a 1536×864 pane;
analysis excludes the additional border.
Actual-take diagnostics agree:
using the same pixels where both A and C report source level at least 51/255,
mean far opacity is 99.4486% for A and 99.9848% for C.
Only 0.2750% of these pixels have both far and near opacity below 230/255 in A,
falling to 0.000998% in C.
Source level here is sampled at the pixel,
not each star's sampled or remembered color.
It serves as a common lit-area filter and does not fully characterize color memory.

A separate same-frame far-only color capture tests foreground overlap.
Among pixels with A's source at least 51/255,
far opacity at least 253/255,
and far-only weighted RGB brightness at least 20/255,
5.917% become more than 25% darker after adding the original near layers.
Of those darkened pixels,
86.414% have near opacity at least 230/255;
their mean near opacity is 96.437%.
The brightness proxy is weighted gamma-coded RGB,
not physical luminance.
This supports near-star darkening in this capture;
it does not imply all dark regions should be filled or that real spectral troughs are defects.
See `coverage-metrics.json` and `analyze-coverage.py` for precise filters and counts.

## Visual evidence

The private take is `take-2026-09-11_03-16-17.take`.
`appearance.ron` selects Stars with current defaults and full texture depth.
The original command and binary hash for each capture are in `capture-*.json`.
Each capture renders 46 seconds from 46.766945 to 92.766945 at 1536×864,
24 fps and scale 2,
discarding 40 seconds to warm visible history and color memory.
The final six seconds contain 144 frames.
A/B/C use the same input and clock.

The local artifact directory is recorded in `manifest.json`.
The stills use frame 72;
`overview.png` halves the full frame for presentation,
whereas `crops-2x.png` enlarges the same 512×320 source crop with nearest-neighbor pixels.
`motion-native-crops.mp4` presents a 640×432 native-pixel crop per candidate side by side,
with labels above the picture,
for all six seconds at 24 fps.
Its H.264 encoding is for viewing;
measurements use raw RGBA before encoding.
`foreground-diagnostic.png` compares A with its far-only image,
not a proposed new production look.

The private input, raw frames and large media are not committed.
Their local locations and final media hashes are recorded;
visual replay requires that input.
The synthetic timing and flat-coverage probes do not require it.

## Replay and limitations

Use a disposable owner-managed worktree at the pinned production base.
Copy this evidence directory outside that checkout before applying the patch.
Do not load its scratch plugin into the DAW.
The scratch mode is only defined for Optimized/P3;
Uniform and live profile/mode transitions are deliberately unsupported.
In particular the two appended slices have no Uniform halos.
Other existing tests assume five slices,
so this patch is a focused experiment rather than a production-ready change or a full-suite claim.
A future port needs normal profile, edge, dial, persistence and corpus verification.

Set `FAR_REPLAY_ROOT` to that worktree,
`FAR_OUTPUT` to an empty writable raw-output directory,
`FAR_TIMINGS` to a separate timing-output directory,
and `FAR_MEDIA` to a writable media directory.
The scripts require Python 3 with NumPy;
movie encoding uses `/opt/homebrew/bin/ffmpeg`.
The copied `png.py` is the repository look-prototype kit's bitmap helper.
It formats actual GPU pixels and does not simulate stars.

Before applying the patch,
build the untouched offline binary with `cargo build --release -p harmonigraph-offline`,
copy it outside `target/`,
and run `python3 <evidence>/render.py pristine <copied-binary> 0` if the private take is available.
Set `FAR_TAKE` if its location differs.
Then:

```sh
git apply --check /path/to/evidence/prototype.patch
git apply /path/to/evidence/prototype.patch
cargo build --release -p harmonigraph-offline
cargo test --release -p harmonigraph-render --no-run \
  --config 'profile.release.package.harmonigraph-render.opt-level=1' \
  --config 'profile.release.package.harmonigraph-render.codegen-units=64'
python3 /path/to/evidence/run-timing.py screen
python3 /path/to/evidence/run-timing.py repeat
```

Use the compiled `harmonigraph_render-*` test executable for the two exact tests `far_prototype_keeps_original_layers` and `spectral_uniforms_match_the_bound_shader_layouts`.
For the flat mask,
set `HARMONIGRAPH_SHADER_ASSETS=source`,
`HARMONIGRAPH_REQUIRE_GPU=1`,
`HG_FAR_DEBUG=2`,
`HG_FAR_FLAT=1`,
and `FAR_OUTPUT=<raw-output>/flat`,
then run `far_coverage_image_probe --ignored --nocapture --test-threads=1`.
Unset those diagnostic controls before timing or normal pictures.

With the private input and pristine capture available,
run `capture-all.py` for the A/B/C and two coverage movies.
Then run `HG_FAR_DEBUG=1 python3 <evidence>/render.py far-A <candidate-offline> 0` for the far-only movie.
Run `analyze-coverage.py`,
`make-sheets.py`,
`make-diagnostic.py`,
and `make-motion.py` after all GPU timings finish.
The source changes in the saved patch were mechanically formatted after the captures;
no semantic edit followed the measured binaries.
The production source was restored and patch applicability checked before committing this report.
