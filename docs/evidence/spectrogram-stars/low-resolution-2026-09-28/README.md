# Stars resolution below Medium

This experiment changes only intermediate rendering resolutions,
using the production Medium path at `2409d55445defb4b0e63ca11f917fd36c1b1e8df`.
It adds no preset and changes no production behavior.
All five depths,
star geometry,
analytic glow support,
life and motion,
color behavior,
light-source dimensions,
and native output remain unchanged.
The same production WGSL renders every variant.

## Candidates

Percentages are width and height,
not pixel count.
Actual rounded dimensions are printed in every run log.

| ID | Background three | Foreground composite | Foreground halo targets |
| --- | ---: | ---: | --- |
| H: High | 75% | Native | 100%, 60% |
| M: Medium | 50% | 75% | 75%, 45% |
| L1 | 33.333% | 50% | 50%, 30% |
| L2 | 25% | 50% | 50%, 30% |
| V | 16.667% | 33.333% | 33.333%, 20% |
| X | 12.5% | 25% | 25%, 15% |

L1 and L2 isolate the additional background reduction.
M2 is an independent resource set with byte-identical Medium settings.
Each candidate owns its resources;
the scratch override is set before its callback preparation.

## GPU cost

Apple M1 Pro / Metal,
960×540 logical points at 2 or 4 device pixels per point,
full coverage,
default Stars settings with color memory disabled,
and synthetic noisy spectrum data.
The interval starts at the first real source pass and ends at the dependent final composite.
It includes the upsample and is not whole-DAW frame time.

The initial screen interleaved H/M/L1/L2/V/X/M2,
then reversed that order,
with ten warmup and 240 measured frames per case.
The machine was busy:
4K Medium medians ranged from 14.670 to 20.641 ms between those runs.
Those absolute readings are retained in `timing.json` and the timing logs,
but are not used as the headline numbers.
No experiment build or capture ran during timing.
Unrelated applications remained running.

The confirmation below uses only M/candidate/M2 resident at once,
interleaved each frame after ten warmup frames,
with 600 measured frames per case.
Each row is its own run;
compare its candidate to its own two controls.
Savings use the mean of the two control medians.
The controls measure local repeatability,
not a confidence interval or a universal hardware guarantee.

| Size | Candidate | Medium control medians, ms | Candidate median, ms | Reduction |
| --- | --- | ---: | ---: | ---: |
| 1920x1080 | L1 | 5.146 / 5.141 | 4.212 | 18.1% |
| 1920x1080 | L2 | 5.171 / 5.166 | 4.120 | 20.3% |
| 1920x1080 | V | 5.148 / 5.224 | 3.376 | 34.9% |
| 1920x1080 | X | 4.310 / 4.192 | 2.755 | 35.2% |
| 3840x2160 | L1 | 9.619 / 9.611 | 6.832 | 28.9% |
| 3840x2160 | L2 | 12.044 / 12.314 | 8.943 | 26.6% |
| 3840x2160 | V | 12.273 / 12.734 | 7.713 | 38.3% |
| 3840x2160 | X | 9.621 / 9.616 | 5.498 | 42.8% |

At 4K the L1 and X control pairs differ by less than 0.1%.
The 4K L2 and V control pairs differ by about 2.2% and 3.7%,
so their 26.6% and 38.3% reductions are approximate.
The 1080p X controls differ by about 2.8%,
so describe its gain as about 35%,
not a precise fractional percentage.
`confirmation.json` and the `confirm-*` logs retain min/p10/median/p90/max for every case.
The later confirmation runs use the identical executable as the initial screen.

## Visual difference

Captures use the same recording-derived field and palette as the previous Stars research,
held static while the stars drift and turn over.
This is not advancing audio replay.
Each variant has a six-second 60 fps movie after ten warmup frames.
The default lifetime is about 2.97 seconds.
Twelve uncompressed RGBA frames form six adjacent pairs,
one pair per second;
the same simulation times are compared in every variant.
All metrics come from those exact RGBA frames,
never from compressed video.

The table compares each candidate directly to Medium.
MAE is mean absolute RGB channel error on the 0–255 scale.
The changed-pixel percentage counts pixels with an absolute error greater than 5 in any channel.
Neither is a perceptual acceptability threshold.

| Candidate | 1080p MAE /255 | 1080p pixels >5 | 4K MAE /255 | 4K pixels >5 |
| --- | ---: | ---: | ---: | ---: |
| L1 | 0.857 | 3.31% | 0.324 | 0.43% |
| L2 | 0.946 | 4.63% | 0.381 | 1.01% |
| V | 1.888 | 17.19% | 0.782 | 3.75% |
| X | 3.031 | 32.68% | 1.266 | 8.63% |

The full JSON also compares every candidate to High,
reports RMSE,
99th-percentile and maximum channel error,
spatial-gradient change,
and error within the brightest quartile of the matching High frame.
The same High-derived bright mask is used for all variants.
For L1 at 1080p,
bright-region MAE is 1.361/255 versus whole-frame 0.857/255;
for X it is 4.770/255 versus 3.031/255.
Dark regions therefore do not account for the entire apparent similarity.

Against Medium,
L1 reduces summed absolute neighboring-pixel RGB differences by 6.7% at 1080p and 3.2% at 4K;
X reduces them by 20.8% and 10.8%.
This is a fine-detail proxy,
not a perceptual sharpness score.
The adjacent-pair metric measures the difference between each variant's frame-to-frame changes and its reference's changes.
Six isolated adjacent pairs cannot establish the absence of shimmer across all motion and settings.

[1080p matched crops at 2×](1920x1080-crops-2x.png),
[4K matched crops at 2×](3840x2160-crops-2x.png),
and [flat-input control crops](flat-crops-2x.png) retain inspectable visual evidence.
Each recording crop is the brightest High 320×180 block at frame 190;
the coordinates are shared by every variant at that output size.
Metrics cover complete frames.
The six-second movies use those same crops,
and full-frame movies and PNGs remain in the output directory recorded in `manifest.json`.
MP4s use lossy encoding for viewing only.

The independent M2 capture matches Medium exactly in both adjacent control frames at both sizes.
A flat-input control at 1080p confirms that L1 and X mainly coarsen the field rather than shifting its average brightness:
mean RGB is 106.963 for Medium,
106.963 for L1,
and 106.960 for X.
The flat control covers H/M/L1/X at one moment,
not all variants or their complete animation.

## Engineering assessment

L1 is the strongest first candidate for a Low preset:
about 29% less rendering time than Medium at 4K,
with modest additional softness in the matched crops.
It reuses the existing resolution controls internally and requires no different star construction.
L2 worsens visual error relative to L1 without establishing a worthwhile additional performance benefit.
Its small 1080p gain and reversed 4K ordering do not support preferring it.
V and X explore larger concessions:
small points become less distinct and the field becomes progressively coarser,
especially at 1080p.
The stars keep their motion and palette behavior.

This experiment establishes the tested range,
not the ultimate minimum cost possible at still smaller resolutions.
Resolution reduction leaves source processing,
per-star preparation,
and native output work in place,
so pixel-count savings cannot be converted directly into total speedups.
A production selection should use the intended display size and the accompanying motion comparison.
No new default or preset is selected here.

## Reproduce

Use an owner-managed worktree at the pinned base.
`research.patch` contains temporary test instrumentation only.
No production shader or binding layout changes,
so no Metal corpus regeneration or plugin handover build is required for this evidence-only change.
The test executable still validates production pipelines.
The maintained GPU timer and readback routines are reused.

```sh
git apply /absolute/path/to/research.patch
cargo fmt --all
cargo test --release -p harmonigraph-render --lib --no-run --message-format=json > /tmp/low-build.json
export RESEARCH_WORKTREE="$PWD"
export RES_EXE=/absolute/path/to/the/executable/from/low-build.json
export RES_OUTPUT=/private/tmp/stars-low-resolution
python3 /absolute/path/to/run.py timing
python3 /absolute/path/to/run.py capture
python3 /absolute/path/to/supplement.py confirm
python3 /absolute/path/to/supplement.py controls
python3 /absolute/path/to/analyze.py
python3 /absolute/path/to/movies.py
```

NumPy and ffmpeg are needed for analysis and captures.
The private input files at `/private/tmp/stars-full-halo-compare` are identified by SHA-256 in the manifest.
Capture reproduction requires those files;
synthetic timing reproduction does not.
The manifest identifies the base,
test executable,
patch,
inputs,
raw samples,
and movies by hash.
Capture-run GPU figures are retained for diagnosis but excluded from timing claims because readback and encoding disturb their workload.
Restore the two instrumented Rust files after archiving the patch.
