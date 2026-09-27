# Per-depth blur versus analytic Stars halos

Measured on M1 Pro / Metal on 2026-09-27,
against the one-neighbor-core plus separate-halo prototype at `f5d90b663f15d3eec1ff23c5282aeaa0c46e4339` in draft PR #1234.
Yan's decision: **“Let's keep the halos.”** The blur implementation was reverted;
only this evidence and the reproducible scratch patches remain.
This experiment informs #1142.

## Result

| Run | Current halos mean | Blur per depth mean | Blur GPU cost change | A/A difference |
| --- | ---: | ---: | ---: | ---: |
| 1080-a | 7.012 ms | 13.489 ms | +92.39% | +0.76% |
| 4k-a | 17.322 ms | 23.291 ms | +34.46% | +0.72% |
| 4k-b | 17.319 ms | 23.230 ms | +34.13% | -0.31% |
| 1080-b | 7.042 ms | 13.467 ms | +91.25% | +0.47% |

This implementation is substantially slower at both sizes.
It does not establish that every possible per-depth blur design loses.
Absolute timings must not be compared across earlier investigations;
paired ratios and the A/A controls are the relevant comparison.

All cases retain five layers,
50% jitter,
color memory and the automatic native 2+3 split at 4K.
Each run interleaves two identical Halo controls and one Blur candidate with independent resources,
60 warmup rounds and 240 measured rounds per case.
All six orders repeat in balanced blocks;
the second run offsets that schedule by three.
The baseline column averages the two controls.
Positive A/A means control B ran faster than A.
The reused runner calls the candidate `paired four` in the raw CSV;
here it means the blur prototype.

The input is the existing synthetic noisy grid,
1024 slabs,
3828 buckets,
a ten-second history,
full pane coverage and two pixels per point.
Timestamps span the actual first source pass through final composite completion,
including all new passes.
These are synchronous offscreen GPU intervals,
not live Bitwig frame times.

## What was compared

The baseline gathers the original outer response minus each compact core through nine atlas neighbors at half width and height.
It keeps weighted palette color and coverage separately for each depth,
then joins each halo to its native core before depth composition.

The candidate replaces that halo gathering with:

1. One instanced quad per atlas star,
writing a 2×2 bilinear seed with additive weighted RGB and coverage.
The four tent weights preserve each seed's total energy under subpixel motion.
2. A horizontal Gaussian blur with seven bilinear texture samples.
3. A vertical Gaussian blur with seven bilinear samples,
writing the same half-resolution halo array that the existing composite consumes.

Two RGBA16Float scratch images are reused across the five depths.
Each is the half-resolution pane plus eight pixels of padding on every side.
Their combined extra allocation is approximately 32.4 MiB at 4K.
Near layers can use a smaller active seed and horizontal-filter viewport as their blur radius grows;
the vertical pass still fills the full half-resolution output layer.
There are 15 coverage passes instead of five.

CPU radial quadrature matches the representative star's residual halo mass and estimates its Gaussian width,
with width capped at 0.35 cells.
Actual per-star seed energy is approximated by the squared ratio of its sigma to the representative sigma.
This retains random core sizes while sharing one halo width per depth.
The derived kernel key contains jitter and each depth's cell,
sigma,
cap,
defocus and fringe;
drift,
life and color update the seeds without recomputing kernels.

## Picture

These production-GPU renders use the same light field derived from the real take `take-2026-09-11_03-16-17.wav`,
starting at 72 seconds and spanning 20 seconds,
plus a constant-level control.
They are 960×540 at native sampling;
the crops are enlarged 2× without smoothing.
The musical light field is static during the motion probe while the actual Stars clock advances.
They are renderer comparisons,
not recordings of live Bitwig playback.

- [Real-take comparison](stars-take.png)
- [Real-take 2× crop](stars-take-2x.png)
- [Flat-field comparison](stars-flat.png)
- [Flat-field 2× crop](stars-flat-2x.png)

A is current halos;
B is blur per depth.
At these dense defaults the difference is subtle:
blur makes the glow more uniform and softens some grain.
Mean RGB over the take is `(117.601, 46.835, 80.033)` for halos and `(117.621, 46.830, 80.059)` for blur on an 8-bit scale.
Matching average brightness does not establish matching star profiles.
A Gaussian residual also fills the center more than the original annular residual,
and per-star halo-width variation is lost.
A three-second side-by-side motion render at full jitter was produced for this comparison;
the scratch frame harness in the patch reproduces it.

The prototype is not a finished alternative:
vertical filtering pairs taps while sampling fractional positions for upsampling,
which adds a phase-dependent approximation to the intended Gaussian.
Its odd-sized-pane coordinate agreement and extreme-control edge support have not been validated.
The original atlas margin was reused;
no claim is made that it covers every possible wide seed footprint at minimum star size and small output resolution.
No golden baseline was changed.

## Why it loses and what remains possible

At the dense 16:9 defaults,
the existing atlas layout contains approximately 1.19 million stars.
Six vertices per seed means approximately 7.16 million vertices per frame before filtering.
That cost remains when the pane shrinks,
helping explain the larger relative regression at 1080p.
Silent atlas entries also emit zero-valued quads in this first prototype.

A separate diagnostic retained all passes and clears but omitted the candidate's seed draw calls.
In that run,
means were 28.788 ms for analytic halos,
36.517 ms for full blur and 24.197 ms for blur without seeds.
The larger absolute times reflect a noisier load/clock period and are not comparable to the table above.
Omitting seeds removed 33.7% of the blur frame interval,
so seed generation is a substantial cost.
This diagnostic draws a different image and changes texture contents;
it is not an exclusive seed-pass timing or a usable optimization.
Per-pass timestamps are not summed because Metal pass intervals can overlap (#1203).

Filtering vertically at each depth's reduced working resolution,
then upsampling once during final composition,
could reduce the current vertical bandwidth and fix its pairing approximation.
It requires different target sizing or per-layer coordinate mapping.
Collapsing silent quads would remove their raster work while retaining their vertex work.
At the time of this first experiment,
neither proposal was implemented or measured.
The later [gathered-blur experiment](../gathered-blur/README.md) tested per-depth vertical-filter sizing together with directly gathered seeds,
but did not demonstrate a win over one-third-resolution analytic halos.
The measured regression does not justify shipping the current added rendering machinery.

A single nominal-cell seed lookup was not used:
at reduced resolution and full jitter it can miss tiny stars or change their energy during drift.
The splats avoid that structural sampling problem,
but their geometry cost is precisely what this experiment exposes.

## Why one neighbor did not save 75% versus four neighbors

“One neighbor” names the current native core lookup only.
Per depth,
the prior tighter-halo version evaluated four neighbors at native resolution.
The current version evaluates one native core plus nine halo neighbors at a quarter of the pixel count:
`1 + 9/4 = 3.25` nominal evaluations instead of four,
a reduction of about 19% before pass and texture costs.
The candidate work per evaluation and early exits also differ,
so that count is not a timing prediction.

The new arrangement adds five halo images and filtered reads during composition,
and restores halos extending to 1.2 cells instead of the four-neighbor version's 0.85 cells at 50% jitter.
The earlier [paired four-neighbor comparison](../separate-halos/README.md) measured 8.3–8.5% lower GPU time at 4K but 3.8–4.0% higher time at 1080p.
It changed both the algorithm and the appearance.

## Lower-cost follow-up within the current halo architecture

These were follow-up proposals at the time of this experiment.
[One-third analytic resolution was subsequently measured](../halo-resolution/README.md) and exposed through the Halo resolution slider;
the other analytic-halo proposals below remain unmeasured.
Reducing the nine-neighbor halo image from half width and height to one third retains full jitter and the 1.2-cell halo reach while evaluating 56% fewer halo pixels.
Nominal core-plus-halo work falls from `1 + 9/4 = 3.25` to `1 + 9/9 = 2` evaluations per depth.
Quarter width and height would evaluate 75% fewer halo pixels and cost `1 + 9/16 = 1.5625` nominal evaluations.
Pass overhead and native composition remain;
these are work counts rather than predicted frame-time savings.
Softer grain and motion aliasing in the fine far layers need visual evaluation.

A four-neighbor reduced halo pass can also retain full jitter,
but a safely supported halo then reaches only 0.7 cells rather than 1.2.
It likewise costs `1 + 4/4 = 2` nominal evaluations per depth,
with a narrower glow.
Reducing resolution only for the broader near layers is another option,
requiring layer-specific working sizes or sampling coordinates.

## Reproduce

Use an owner-managed worktree at the baseline revision above.
These patches are experiments,
not changes to include in a plugin build or to run through strict Metal assets.
`prototype.patch` restores the candidate and the paired timing/real-take frame harness;
`skip-seeds.patch` is an optional diagnostic applied afterward.
Neither patch changes the Metal corpus.

```sh
git apply docs/evidence/spectrogram-stars/depth-blur/prototype.patch
HARMONIGRAPH_SHADER_ASSETS=source cargo test --release -p harmonigraph-render --lib --no-run
python3 docs/evidence/spectrogram-stars/four-neighbor-current/run.py --binary /path/to/printed/test-executable --output /private/tmp/stars-depth-blur-repeat
git apply -R docs/evidence/spectrogram-stars/depth-blur/prototype.patch
```

For the diagnostic,
apply `skip-seeds.patch` after the prototype,
recompile,
and select `PROBE_CASE=paired`,
`PROBE_FILLS=1`,
`PROBE_SIZE=3840x2160`,
`PROBE_FRAMES=180` and a `PROBE_RAW` output path.
`paired baseline A` is analytic halos,
`paired four` is full blur,
and `paired baseline B` is blur without seed draws.
Reverse the diagnostic before reversing the prototype.

The frame harness reads `take-levels.u8`,
`flat-levels.u8` and `palette.rgba` from `/private/tmp/stars-depth-blur`.
[prepare-input.py](prepare-input.py) builds them with the look-prototype kit.
Run the ignored `scratch_take_stars_frames` test with `HARMONIGRAPH_SHADER_ASSETS=source`.
`PROBE_JITTER` selects jitter;
`PROBE_VIDEO_FRAMES=90` produces three seconds at 30 fps.
[sheet.py](sheet.py) assembles the first-frame comparisons.
Raw paired GPU samples and the seed diagnostic are stored as compressed CSVs;
[analysis.json](analysis.json) contains the paired means and ratios.
