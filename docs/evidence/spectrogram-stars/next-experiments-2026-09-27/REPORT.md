# Stars follow-up experiments: complete response, native partitions, fused seed/filter

Research for #1142, in the requested order.
Baseline: `7711b98d2c28c737b5a9c44dd298eb3d96efe589` (merged P3 / Uniform implementation).
Hardware: Apple M1 Pro, Metal.
This branch preserves evidence and reproduction inputs only; no optimization is enabled in the plugin.

## Outcome

**3+2 is the strongest next P3 implementation candidate.** The simple partition change saved roughly 3–4% at default density against current production in the tested fixtures.
Complete response is more useful for Uniform Full than for P3.
The fused gather is a clear regression against its existing-blur control.
No candidate has been enabled or accepted as a new visual setting.

| Experiment | 1080p result | 4K result | Assessment |
| --- | --- | --- | --- |
| Complete response, P3 | No dependable gain | 2.6–2.8% saving on recorded input | Small conditional lead |
| Complete response, Uniform Full | 8–12% saving | About 10% saving | More promising for the override; still costs more than P3 at 4K |
| Native 3+2, P3 | 3.3–3.9% saving at default density | 2.8–3.8% saving at default density | Best simple follow-up |
| Native 1+4, P3 | No dependable gain | 3–5% slower | Reject for these settings |
| Fused seed/horizontal filter | 111–113% more cost than old blur | 20–22% more cost than old blur | Reject this implementation |

At 1080p, current production is **unsplit**.
The direct 3+2 versus forced 2+3 comparison saves 1.7–4.2% at 1080p and 3.0–3.4% at 4K across these fixtures.
The folder names ending in `repeat` denote **full-jitter confirmation runs**, changing jitter from 0.5 to 1.0 as well as order; they are not identical-setting repeats.

## Measurement contract

Timings span the actual source-pass BEGIN to dependent final-composite END.
They are not whole Bitwig frame times.
Each variant has separate callbacks, pipeline resources and color history.
All variants see the same scrolling recorded input or synthetic fixture per round.
The pane is 960×540 points at 2×/4× display scale (1080p/4K).
Default runs have 60 warmup rounds and 240 measured rounds, with six cases occupying every position equally in a forward/reverse rotating schedule.
Sparse screens use 120 measured rounds.
The two P3 controls and, where included, two Full or blur controls are independently allocated but behaviorally identical.
No build, image capture or video encoding runs concurrently with timing.
Ambient machine activity remains uncontrolled; keep A/A disagreements and repeat results beside small effects.
Bootstrap ranges are descriptive rather than calibrated confidence guarantees.
Savings are computed against the same-run mean of the appropriate control pair.
Do not add savings across these experiments or compare absolute times across runs.

## 1. Native complete response

The prototype keeps five depths, halo reach, targets and the production split policy.
At aligned native-resolution depths it stores the entire analytic response, eliminating compact-core subtraction in the halo pass and core evaluation/add-back in the composite.
P3 qualifies at depths 2 and 3; Uniform Full qualifies at all five.
Other depths retain the original residual-halo/native-core calculation.
Eligibility requires exact actual native dimensions, an integral physical pixel origin, and display scale exactly 1, 2 or 4.
All math remains f32, with the existing RGBA16Float target rounding moved after complete coverage.
This is unrelated to the previously rejected direct nine-neighbor final-composite experiment.

P3 measurements:

| Run | Size | Control mean ms | Candidate mean ms | Saving % | A/A disagreement % |
| --- | --- | ---: | ---: | ---: | ---: |
| complete-take | 1920x1080 | 8.676 | 8.416 | 3.00 | 6.20 |
| complete-take | 3840x2160 | 21.657 | 21.090 | 2.62 | 0.25 |
| complete-repeat | 1920x1080 | 9.465 | 9.519 | -0.57 | 3.95 |
| complete-repeat | 3840x2160 | 27.577 | 26.819 | 2.75 | 0.99 |
| complete-synthetic | 1920x1080 | 8.508 | 8.465 | 0.50 | 0.10 |
| complete-synthetic | 3840x2160 | 37.594 | 36.636 | 2.55 | 1.27 |

The first two recorded 1080p controls disagreed by 6.20% and 3.95%; apparent small gains there are not dependable.
The synthetic 4K P3 estimate was 2.55%, but its descriptive bootstrap range crosses zero.
Recorded default/full-jitter 4K ranges were 1.84–3.43% and 1.90–3.61%.
Uniform Full measurements:

| Run | Size | Control mean ms | Candidate mean ms | Saving % | A/A disagreement % |
| --- | --- | ---: | ---: | ---: | ---: |
| complete-take | 1920x1080 | 9.422 | 8.601 | 8.71 | 1.37 |
| complete-take | 3840x2160 | 29.390 | 26.280 | 10.58 | 0.30 |
| complete-repeat | 1920x1080 | 10.725 | 9.481 | 11.60 | 0.01 |
| complete-repeat | 3840x2160 | 37.135 | 33.264 | 10.42 | 0.19 |
| complete-synthetic | 1920x1080 | 9.587 | 8.821 | 7.99 | 0.22 |
| complete-synthetic | 3840x2160 | 50.439 | 45.302 | 10.18 | 0.91 |

## 2. Native partitions

Compare 1+4 and 3+2 against production 2+3 on large panes.
At 1080p production is unsplit; explicit 1+4, 2+3 and 3+2 cases force allocation and the far pass.
An explicit unsplit case is included at both sizes.
Both the far-pass ending depth and final-pass starting depth change together.
All five layers retain their composition order and original halos.
Changing the split moves the RGBA16Float intermediate rounding point.

| Run | Size | Control mean ms | Candidate mean ms | Saving % | A/A disagreement % |
| --- | --- | ---: | ---: | ---: | ---: |
| partitions-take | 1920x1080 | 8.726 | 8.388 | 3.87 | 0.73 |
| partitions-take | 3840x2160 | 26.153 | 25.161 | 3.79 | 0.25 |
| partitions-repeat | 1920x1080 | 8.283 | 8.009 | 3.30 | 0.59 |
| partitions-repeat | 3840x2160 | 20.473 | 19.910 | 2.75 | 0.05 |
| partitions-synthetic | 1920x1080 | 8.883 | 8.563 | 3.60 | 1.17 |
| partitions-synthetic | 3840x2160 | 23.812 | 23.034 | 3.27 | 0.22 |
| partitions-sparse | 1920x1080 | 5.836 | 5.474 | 6.20 | 1.45 |
| partitions-sparse | 3840x2160 | 18.963 | 18.363 | 3.17 | 0.59 |

These results do not establish an optimal split cutoff, a settings-independent policy or benefits for Uniform profiles.
Those are implementation follow-ups before changing production.

## 3. Fused seed generation and horizontal filtering

Restore the prior gathered blur construction from the #1234 evidence snapshot, including `full-filter.patch` (all seven bilinear taps per axis).
The original seed pass plus horizontal pass is the control; the candidate folds the same discrete radius-six horizontal kernel into an expanded star gather.
The vertical pass, calibrated mass/width, per-depth logical size and native core remain unchanged.
For horizontal displacement d, n=floor(d), f=fract(d), the star's weight is `(1-f) K(n) + f K(n+1)` times its vertical tent weight.
The candidate gather covers seven pixels horizontally and one vertically, clamps both atlas grid axes, and preserves real-valued clamping at scratch boundaries.
This removes a pass and intermediate seed-image traffic, but expands per-pixel atlas work and changes accumulation and half-rounding points.
It does not recover individual analytic halo shapes.
Blur sampling retains the original per-depth cap of one third; this is the previously evaluated blur construction, not an accepted analytic sampling setting.

The bounded scratch port uses test-only resources and an existing uniform row; it adds no persisted enum, UI choice or production pipeline.
Independent resources are mandatory: the scratch mode is selected when each case allocates, and is not a supported runtime switch.
Atlas expansion asserts the production cap instead of silently exceeding it at extreme settings.
No general blur product path is proposed.

| Run | Size | Control mean ms | Candidate mean ms | Saving % | A/A disagreement % |
| --- | --- | ---: | ---: | ---: | ---: |
| fusion-take | 1920x1080 | 6.335 | 13.470 | -112.61 | 1.69 |
| fusion-take | 3840x2160 | 13.493 | 16.221 | -20.22 | 2.85 |
| fusion-synthetic | 1920x1080 | 6.505 | 13.732 | -111.11 | 0.40 |
| fusion-synthetic | 3840x2160 | 12.535 | 15.239 | -21.57 | 0.16 |

Negative savings mean a slowdown.
The larger repeated star gather outweighs the removed seed pass in this implementation.
A future tiled/shared-memory implementation is not ruled out, but no such complexity is justified by these measurements alone.
The independent CPU scatter-plus-discrete-convolution probe passed all 17 dense full-jitter phases, including an atlas-row crossing, adjacent sentinels and an odd-sized output.
The original seed probe also passed.
These validate the intended unquantized construction, while the rendered comparisons below measure differences from the original quantized GPU blur.

## Image verification

| Capture | Compared frames (case/input/frame combinations) | Maximum channel difference |
| --- | ---: | ---: |
| complete-motion | 120 | 1 |
| complete-fractional | 12 | 0 |
| complete-origin | 12 | 1 |
| complete-4k | 12 | 1 |
| partitions-motion | 72 | 1 |
| partitions-fractional | 18 | 1 |
| fusion-motion | 60 | 1 |
| fusion-4k | 6 | 1 |

No alpha channel changed in these comparisons.
Complete/split/blur-fusion differences stayed within one 8-bit channel level for the captured fixtures; this is not a universal bound.
`complete-fractional` at PPP=1.25 is the verified byte-identical fallback.
`complete-origin` does **not** establish fractional-origin fallback: production snaps the origin to physical pixels before constructing the shader uniform, so this fixture still exercises the native path.
The complete motion and fused motion captures cover 30 frames at 30 fps after 60 warmup frames, at full jitter.
Partition motion covers 12 frames at 30 fps.
Other captures cover three frames.
The fused and old blur remain visibly different from Full; P3 is closer in these captures.
The image-error metrics are screening data, not a perceptual acceptance rule.
Contact sheets include native-pixel crops and are in the corresponding image directories.
No new visual tradeoff has been selected.

## Reproduction and limitations

The evidence includes exact source substitutions, scratch test modules, hooks, scripts, binary hashes, raw timing CSVs, analysis, capture manifests and image-error metrics.
Raw private spectrogram fixtures and multi-gigabyte RGBA captures are omitted.
Recorded reproduction needs the same local fixtures; synthetic timings and the independent convolution probe do not.
Image motion advances the Stars clock over a static recorded or flat light field; timing uses scrolling recorded input.
This is not validation on every live audio transition or another GPU.
Research source changes are reverted after capture; evidence is retained on this draft research PR and linked from #1142, not proposed as shipped documentation.
