# Stars: lower back and foreground resolution

Measured against the current Optimized profile at `7f88775ed50aca914877276168408ded01ad41fc`.
This is an experiment,
not a production behavior change.
Percentages refer to width and height,
not pixel count.

## Variants

| ID | Back three | Foreground composite | Foreground halo targets |
| --- | --- | --- | --- |
| A | 75% | 100% | Existing 100% and 60% |
| B | 50% | 100% | Existing 100% and 60% |
| C | 50% | 75% | Existing 100% and 60% |
| D | 50% | 75% | 75% and 45%: both existing targets multiplied by 0.75 |

C and D rasterize the final Stars composite into a gamma-encoded RGBA16Float target at 75% dimensions,
then bilinearly upsample into the original output.
This also resamples the already-composited back layers.
It is a concrete implementation of reduced foreground rendering,
not a claim that every possible implementation costs the same.
Atlas,
light-source,
color-history,
and logical scene dimensions stay unchanged.
The final upsample is included in the GPU interval.
All five layers remain,
and star geometry and glow support are unchanged.

The back pass shades 55.6% fewer pixels than the existing 75% target.
The foreground composite shades 43.75% fewer pixels at 75% dimensions.
Those are pass workload reductions,
not whole-frame savings.

## Measured cost

Apple M1 Pro / Metal;
full 960×540-point pane at 2 or 4 device pixels per point;
current default Stars dials;
color memory off;
synthetic noisy spectrogram input.
These are source-pass BEGIN through dependent final-pass END GPU intervals,
not whole-DAW frame times.
Each run has ten warmup frames followed by 240 measured frames.
The order is A/B/C/D/D/C/B/A,
and the table uses the mean of each variant's two run medians.
No build,
capture,
or encoding ran concurrently with the retained timing sequence.

| Variant | 1080p GPU ms | Change from A | 4K GPU ms | Change from A |
| --- | ---: | ---: | ---: | ---: |
| A: current | 4.878 | — | 14.766 | — |
| B: back 50% | 4.827 | 1.0% lower; no dependable gain | 13.217 | 10.5% lower |
| C: plus foreground composite 75% | 4.556 | 6.6% lower | 11.482 | 22.2% lower |
| D: plus foreground halos scaled too | 3.891 | 20.2% lower | 9.624 | 34.8% lower |

At 1080p the A controls were 4.941 and 4.815 ms,
so B's nominal 1% gain is smaller than control drift.
C measured 4.596 and 4.515 ms;
D measured 3.889 and 3.892 ms.
At 4K the A controls were 14.770 and 14.762 ms,
and both repeats of each candidate agree within 0.02 ms.
Treat the small 1080p differences more cautiously than the larger gains.
A prior corrected pilot had disturbed 1080p controls but independently reproduced the 4K B/C direction;
the retained balanced repeat supplies all percentages here.

## Measured visual difference

Separate capture runs use the retained recording-derived spectrogram field and palette from the archived architecture trials.
The field is held static while stars move through time;
this is not an advancing audio replay.
The comparison includes twelve matching frames,
spaced twenty simulation frames apart over approximately 1.53 seconds at a 144 Hz clock.
Timing and capture workloads therefore differ intentionally;
capture-run timings are not used above.
Every metric below compares directly with A,
not with an older shader.

| Variant | Mean absolute RGB error /255, 1080p | Mean error /255, 4K | Pixels with any channel error >5, 1080p | Same, 4K |
| --- | ---: | ---: | ---: | ---: |
| B | 0.242 | 0.104 | 0.974% | 0.103% |
| C | 0.885 | 0.319 | 3.745% | 0.233% |
| D | 0.750 | 0.269 | 2.640% | 0.200% |

B leaves foreground cores at native resolution and slightly softens the distant texture.
C and D soften foreground grain as well,
more visibly at 1080p.
The sum of absolute neighboring-pixel RGB differences falls by 1.6%/1.0% for B,
8.3%/3.8% for C,
and 7.3%/3.4% for D at 1080p/4K.
This spatial-gradient statistic is a proxy for retained fine detail,
not a perceptual sharpness score.
D's smaller aggregate error than C in this fixture does not imply that reducing halo resolution always improves fidelity.

At 4K,
99% of channel errors are at most 2/255 for B and D,
and 3/255 for C.
Localized maxima are much larger:
44/255 for B and 54/255 for C/D.
Whole-frame means do not certify visual equivalence.
The static field and sparse temporal samples also do not establish the absence of shimmer during scrolling,
life transitions,
or other settings.

The full-resolution extra-pass control uses the same float intermediate and upsample at 100%.
Its maximum channel error is 1/255,
mean 0.0261/255 at both sizes,
and gradient change is approximately -0.026%.
This rules out a large color-space or coordinate mismatch in C/D's intermediate path.

[1080p crops at 2×](1920x1080-crops-2x.png) and [4K crops at 2×](3840x2160-crops-2x.png) show the same region and moment for all four variants.
Crop selection uses the brightest baseline 320×180 block;
all metrics use the complete frames.
The uncompressed captures and full-frame PNGs remain at the local path in `manifest.json`.

## Assessment

Back-only 50% is a small visual concession with a useful roughly 10% 4K saving,
but the measurements give no performance reason to select it at 1080p.
Reducing the entire foreground rendering workload buys materially more:
roughly 35% at 4K and 20% at 1080p in this probe,
at the cost of softer foreground detail and another intermediate target/pass.
C is a smaller cost reduction with no visual advantage established by these captures.
A production decision should be based on the intended display size and a motion comparison;
this evidence PR selects no new default.

## Reproduce and provenance

Use an owner-managed worktree at the pinned base above.
Apply `research.patch` only in an experiment checkout,
then compile the release renderer tests with source shaders.
The patch is temporary test instrumentation;
no changed production shader is shipped by this report,
and no Metal corpus or plugin build is required for this evidence-only diff.
A second agent reviewed the coordinate mapping,
color space,
source sizing,
and dependency-ordered timestamp bracket.
The test executable validates all pipelines during the measured runs.

```sh
git apply /absolute/path/to/research.patch
cargo test --release -p harmonigraph-render --lib --no-run
export RESEARCH_WORKTREE="$PWD"
export RES_OUTPUT=/private/tmp/stars-resolution-replay
python3 /absolute/path/to/run.py timing
python3 /absolute/path/to/run.py capture
python3 /absolute/path/to/analyze.py
```

NumPy is needed for analysis.
Timing replay is self-contained.
Capture replay needs the private `take-levels.u8` and `palette.rgba` at `/private/tmp/stars-full-halo-compare`,
identified by SHA-256 in `manifest.json`.
The manifest also records the binary hash and hashes of all 120 RGBA captures.
The run script uses a single release renderer test executable in the experiment checkout;
use a clean target directory if multiple matching executables exist.

`timing.json` and the timing logs retain run-level minimum,
p10,
median,
p90,
and maximum values.
`visual-metrics.json` retains errors,
spatial-gradient changes,
and differences of consecutive captured-frame deltas.
The latter measures temporal error between sparse samples and is not a flicker score.
The prototype assumes full panes at the tested dimensions;
its scalar coordinate correction is not suitable for arbitrary fractional rounded targets without further work.
