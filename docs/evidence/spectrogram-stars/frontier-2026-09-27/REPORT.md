# Stars quality and performance frontier: research handoff

Baseline: `fd7c8f9f9fccb2a55f79f8ee87c25e2d5afd8d40` (PR #1234).
Hardware: Apple M1 Pro, Metal.
This evidence records the investigation for #1142 and the live comparison build selected from it.
The full-resolution analytic renderer remains the visual reference.

## What changed the recommendation

Different halo resolutions by depth were the strongest demonstrated way to approach the full-resolution appearance for less GPU time.
The original prediction that the far slices would be the first to need high resolution did not match the final composited default picture.
Nearer layers obscure much of their error.
Layer 3 was much more sensitive than layers 0 and 1 in the measured default scenes.

Yan accepted D `[50,50,50,100,100]` and F `[50,50,100,100,100]` in stills and motion.
After the finer search, Yan accepted P2 `[50,50,80,100,50]` and P3 `[50,50,100,100,60]` as useful intermediate stills, and rejected P1 `[50,50,50,90,50]`.
All vectors are percentages of native width and height, far to near.
Yan also accepted P2/P3 in the six-second motion comparison and requested a live plugin selector to make the final comparison in context.
The actual runtime grouped implementation has now been timed with duplicate controls; results follow.

## Measured live profiles

Actual production profiles use recorded scrolling input, a 960×540-point pane at 2×/4× display scales, 60 warmup rounds and 240 measured rounds per case.
The six-case forward/reverse schedule balances every case across every position.
The interval runs from source-pass BEGIN to final-composite END, not whole DAW frame time.

| Live profile | Depth resolutions, far → near | 1080p mean GPU | 1080p saving | 4K mean GPU | 4K saving | Status |
| --- | --- | ---: | ---: | ---: | ---: | --- |
| Half | 50/50/50/50/50% | 6.42 ms | 29.7%* | 16.19 ms | 45.8%* | Existing anchor |
| P2 | 50/50/80/100/50% | 7.51 ms | 17.86% [16.95, 18.74] | 20.40 ms | 31.78% [31.28, 32.21] | Accepted in stills and motion |
| P3 | 50/50/100/100/60% | 7.97 ms | 12.75% [11.71, 13.83] | 22.13 ms | 25.98% [25.74, 26.23] | Accepted in stills and motion |
| Full | 100/100/100/100/100% | 9.14 ms | Reference | 29.90 ms | Reference | Visual reference |

*Half savings are calculated from the two half case means against the two full case means in the same run; no half-vs-full bootstrap interval was generated. Full A/B differed by 0.31% at 1080p and 0.04% at 4K; half A/B differed by 0.92% and 0.22%. Source: `live-take-{1920x1080,3840x2160}.csv`, `-analysis.json`, and `.log` in `live-production/`. P2 and P3 match accepted reference pixels exactly at both resolutions on recorded and flat inputs. Half and Full differ by at most one 8-bit color level in fewer than 0.2% of RGB channels; no alpha differs. The strict comparator correctly reports this non-exact result.

The earlier **prototype** used five separate halo textures and a 240-round recorded-take batch at 2 pixels per point at both resolutions, with 120 warmup rounds. Its same-run savings were P2 16.18%/30.75%, P3 10.50%/25.00%, and F `[50,50,100,100,100]` 5.94%/18.41% at 1080p/4K. These remain historical prototype results, not the live grouped-path measurement above. In particular, the old 4K pane was 1920×1080 points, while the live 4K pane is 960×540 points. Source: `timings/separate-frontier/`.

## Why a smooth resolution curve is not enough

For output pixel index j, native width N, and halo width M, the bilinear source coordinate is `(j + 0.5) M/N - 0.5`.
At M=N every sample lands on its source texel center.
At M=N-1 the phase shifts by `-(j + 0.5)/N`, spanning almost a whole texel across the image.
Thus even a one-column reduction creates interpolation across much of the pane.
The sampled near-native branch does not smoothly approach the exact native result at this fixed output size.

The layer-3-only probe at 1080p confirmed this:
90%, 95%, 99%, and 99.9% all retained appreciable error, while 100% matched the reference.
The 99.99% case rounded up to the native dimensions and differed only by tiny arithmetic rounding.
The identity-coordinate control was byte-exact.
This is a discrete-grid and reconstruction effect, not a claim of a mathematical discontinuity in a continuous optical image.

Star width alone also misses the halo signal's shape.
The low-resolution image stores the full shape minus a compact native core.
Reconstructing it gives `I(full - core) + core`, so the compact subtraction window contributes detail that the Gaussian width does not describe.
Final-image sensitivity also depends on coverage by nearer layers.

The empirical model measured signed RGB perturbations from changing one layer at a time.
It formed a Gram matrix of their pairwise products and predicted the squared error of a combined choice, retaining cross-layer interactions.
The search enumerated 7,776 profiles from six measured resolution choices per layer over three drift phases and two input fields.
Combined GPU renders checked shortlisted predictions.
Predicted errors generally overestimated the tested combined errors by about 8–21%; the model was useful for screening, not a proof of perceptual optimality.
Yan's rejection of P1 is stronger evidence about the desired look than its favorable numeric error.

A smooth-image error bound can help choose an initial search range:
linear interpolation over spacing h has error at most `h² max|f''| / 8` on each interval.
Bilinear interpolation admits corresponding horizontal and vertical terms.
This suggests allocating more samples where curvature is high, rather than choosing resolution only from a nominal sigma.
For a simplified independent-layer squared-error model `E ≈ Σ a_k/r_k⁴` and pixel-work model `C ≈ Σ b_k r_k²`, a Lagrange multiplier gives `r_k ∝ (a_k/b_k)^(1/6)`, before bounds or grouping constraints.
That is a useful starting hypothesis, not the measured renderer's optimum:
subpixel phase, compact-core cancellation, nearer-layer transmittance, nonlinear composition, quantization, and the exact-native endpoint violate its simplifying assumptions.
The measured signed-image model incorporates more of those effects without pretending to predict subjective acceptance.

## Image evidence and memory

The frozen final 4K capture compares each candidate with the same-kind full reference at one clock phase, 50% jitter, on recorded-take and flat inputs. It is **one phase**, not the earlier three-phase 1080p model. RGB MAE/MSE (8-bit byte-channel units / byte-channel²) are numerical differences, not perceptual quality or timings:

| Profile | 4K take MAE / MSE | 4K flat MAE / MSE |
| --- | ---: | ---: |
| Half | 0.5172 / 1.2748 | 0.6539 / 1.4470 |
| Uniform 75% | 0.2578 / 0.4610 | 0.3262 / 0.5358 |
| Uniform 87.5% | 0.1978 / 0.3134 | 0.2503 / 0.3673 |
| P2 | 0.1219 / 0.1828 | 0.1567 / 0.2215 |
| P3 | 0.0600 / 0.0809 | 0.0754 / 0.0977 |
| F | 0.0142 / 0.0236 | 0.0196 / 0.0311 |

Source: `images-final-4k/final-image-metrics.json` and `manifest.json`. At 1080p, the P2/P3/F grouped captures share a byte-identical reference with curve phase p0; the uniform 75/87.5 captures use a different frozen reference. The three-phase signed-RGB Gram search evaluated 7,776 candidate vectors and overestimated actual combined-image MSE by roughly 8–21% on the tested mixed profiles. It was a screening tool; Yan's still and motion judgments determine acceptance.

## Allocation and grouping

The initial mixed-resolution probe rendered smaller viewports into a full-sized five-layer array.
It saved shading work but retained the large allocation.
Five separately sized textures reproduced the tested full, half, P2, and P3 images byte-for-byte.
They retain five draws and add sampled texture bindings.

At 4K, RGBA16F halo storage is about 316.4 MiB for five full-size layers, 151.2 MiB for P2, and 181.0 MiB for P3.
These figures exclude atlas/history storage and the existing native far-layer target.
Sharing a resolution lets depths share an array without combining their independent layer images or draw calls.
The grouped prototype uses up to three arrays and fewer bindings than the five-texture version.
Grouped full, half, P2, P3 and F stills matched their separate-texture controls byte-for-byte on both recorded and flat inputs.
An odd-sized output at fractional display scale also matched exactly for the native and P2 controls.

A production allocation key must include actual group extents and each depth's group/layer mapping.
Changing resolution must preserve color memory as #1234 already does.
The scratch harness's per-case source specialization is not a suitable production cache or runtime policy.
P2 and P3 each need three groups, but straight interpolation between their vectors usually needs four.
A smooth control needs a deliberately constrained path or a larger binding budget.

## Other leads

| Experiment | Finding |
| --- | --- |
| f16 bounded per-tap math / exp only | No repeatable useful gain in the tested Metal paths |
| Remove native 2+3 split | About 7% slower at 4K half; keep the split |
| Direct native nine-neighbor composite | Slower than the current full-resolution multipass renderer |
| Move fringe peak into compact core | Did not improve the tested speed/quality tradeoff |
| Wider native 2×2 core walk | Large slowdown; buys a different quality tradeoff |
| Shared decoded compute records | Slower |
| Shared packed records, 16×8 group | About 12% faster at dense 1080p half; no general higher-resolution or sparse-scene win |
| Fuse color memory and bake | Modest gain; approximately 4.7% at 1080p half and 1.4% at 4K in the recorded-input confirmation |
| Packed compute plus fusion | About 17% faster than current half at dense 1080p; does not generalize to 1440p/4K |
| RGBA8 halo storage | Small or negligible speed gain with added quantization error; poor tradeoff |
| Skip silent tiles | Only about 2–4% wholly dead tiles in the warmed recorded passage; insufficient justification for a mask path there |

Fusion is not generally byte-exact.
A 49-frame cold/warm/life/reset/silence probe found nine byte differences of one level across five frames; reset, recovery, and silence conditions passed.
The strict parity diagnostic's failure is preserved.
Color memory remains RGBA32F; lowering its precision is a separate feedback-stability problem.

The compute result requires a second rendering path and a selection rule depending on pane size and star density.
Its narrow benefit is less attractive than the simpler resolution policy for a first implementation.
The measured settings extremes also show that fixed default vectors are not a settings-independent optimum.

## Additional native-resolution screen

A complete-response prototype retains the five halo draws and native 2+3 split, but at aligned native resolution stores the entire per-layer response and skips the separate core lookup/add-back.
This also removes compact-core taper/subtraction work from its nine-neighbor draw.
It is distinct from the slower direct nine-neighbor final-composite experiment.
The bounded prototype only enables this at aligned native targets with 1x, 2x or 4x display scale; other configurations use the original residual/core path.
On the default 1080p and 4K recorded and flat stills, all observed channel differences were at most one 8-bit level. The separate 1.25× fallback matched byte-for-byte.
The rounding point changes because complete coverage now passes through RGBA16F.
Performance is unmeasured; this is not included in the live comparison build.

## Reproduction and limitations

The evidence bundle contains raw samples, manifests, shader variants, the scratch harness, and analysis scripts.
Recorded-input and visual fixtures remain local and are omitted from the public evidence.
Synthetic timing runs can be reproduced without private audio.
Motion advances the Stars clock over a static recorded light field; it does not establish equivalence for every audio transition.
GPU pass timestamps may overlap and must not be summed into an exclusive pass budget.
Bootstrap ranges are descriptive, not calibrated guarantees.
No alternative GPU or live Bitwig frame-time improvement has been verified.

The live comparison build adds quick Half/P2/P3/Full choices while retaining arbitrary uniform resolution.
It deliberately excludes compute, fusion, f16 and native-complete response experiments so the in-plugin comparison isolates sampling policy.
This report remains a research handoff, not a settings-independent optimality claim.

## Live implementation verification

The renderer suite passed 359 tests, including original-array-lookup parity, allocation layout and uniform ABI checks, fractional-scale split rendering, and color-history preservation through Half/P2/P3/Full transitions.
The persistence suite passed 53 tests, including old saves and offline appearance round trips.
Temporary acquisition modules are preserved in `live-production/` and removed from the shipped diff.
The actual recorded/flat image comparisons, including the non-exact Half/Full results, are in `live-production/images-1080/` and `images-4k/`.

The baseline offline golden suite passed. Four Stars golden frames changed by at most one 8-bit level under the new shader layout; their amplified sheets were inspected before rebaselining. Their names also cover zoom and shadows, but they all use the current Stars default. Watercolor and short-pane frames remained exact.
