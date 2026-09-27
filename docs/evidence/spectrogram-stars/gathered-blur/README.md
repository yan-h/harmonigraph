# Directly gathered Stars blur seeds

Measured on M1 Pro / Metal on 2026-09-27,
against `2d7e08173e104903cb4e94b5bac4f9f7fd4fa6ba` in draft PR #1234.
This follows the [quad-seeded blur experiment](../depth-blur/README.md) and informs #1142.
**No useful speedup was demonstrated;
the experiment was reverted and the existing analytic halos remain active.** The saved patches are experiments,
not a supported plugin option.

## Result

The initial version cost 13–14% more GPU time at 1080p and was roughly tied at 4K,
with means about 1% slower.
Two attempts to remove nearly zero-weight filter taps also failed to beat the analytic baseline.
The final specialized version cost 16–19% more at 1080p and 3–5% more at 4K.
All cases use **100% jitter**;
the analytic baseline uses exactly **one-third halo resolution**.

| Filter variant | Run | Analytic halos mean | Gathered blur mean | GPU cost change | A/A difference |
| --- | --- | ---: | ---: | ---: | ---: |
| Full filter | 1080-a | 5.869 ms | 6.643 ms | +13.18% | +0.79% |
| Full filter | 4k-a | 12.696 ms | 12.817 ms | +0.95% | -1.00% |
| Full filter | 4k-b | 12.683 ms | 12.823 ms | +1.10% | +0.07% |
| Full filter | 1080-b | 5.852 ms | 6.652 ms | +13.67% | +0.02% |
| Dynamic trimmed filter | 1080-a | 12.971 ms | 15.265 ms | +17.68% | -0.92% |
| Dynamic trimmed filter | 4k-a | 25.223 ms | 26.258 ms | +4.10% | +0.99% |
| Dynamic trimmed filter | 4k-b | 25.730 ms | 26.775 ms | +4.06% | -0.43% |
| Dynamic trimmed filter | 1080-b | 9.989 ms | 12.268 ms | +22.81% | -2.60% |
| Specialized trimmed filter | 1080-a | 11.890 ms | 14.095 ms | +18.54% | +0.98% |
| Specialized trimmed filter | 4k-a | 22.317 ms | 23.396 ms | +4.83% | +0.41% |
| Specialized trimmed filter | 4k-b | 22.879 ms | 23.552 ms | +2.94% | +0.43% |
| Specialized trimmed filter | 1080-b | 8.342 ms | 9.665 ms | +15.86% | +1.81% |

Each run interleaves two identical analytic controls and the candidate,
using independent resources,
60 warmup rounds and 240 measured rounds per case.
All six orders repeat in balanced blocks;
the repeat offsets that schedule by three.
The baseline column averages the controls.
Positive A/A means control B ran faster than A.
Raw CSVs retain the shared runner's `paired four` name for the gathered-blur candidate.
Negative `reduction_percent` in the analysis files means increased cost.

The fixture is the synthetic noisy full-pane field with 1024 slabs,
3828 buckets,
a ten-second history and two device pixels per point.
All five depths,
color memory and the automatic native 2+3 split remain enabled.
Timestamps span the actual first source pass through final composition,
including seed production and both blur directions.
These are synchronous offscreen GPU intervals,
not live Bitwig frame times.
Background load or clocks varied substantially between series;
compare each candidate with its interleaved controls,
not absolute milliseconds across series.
The results do not isolate the effect of kernel specialization from those changes.
No exclusive pass budget is inferred from overlapping Metal timestamps.

## What changed

The earlier prototype drew a quad per atlas star and always wrote its vertical filter at half pane resolution.
This experiment removes both costs:

1. A fullscreen seed pass directly gathers the actual jittered centers contributing to each seed pixel.
Each star deposits a tent into four pixel centers,
preserving its energy under subpixel motion.
The candidate range expands with the footprint;
it is not a fixed one- or nine-neighbor lookup that can miss tiny stars.
Both grid axes are clamped before flattening the atlas address.
2. Each depth chooses its own logical image size from its representative halo width,
capped at one third of the pane's width and height.
3. Both horizontal and vertical filters run at that logical size.
Native composition upsamples once,
using separate logical-to-allocated image coordinates for each depth.

Two shared RGBA16Float scratch images and the five-layer halo array allocate the maximum size plus eight pixels of padding on each side.
Viewports and scissors restrict each pass to its depth's logical rectangle plus padding.
The atlas gains a border intended to cover the padded seed/filter footprint.
Integer-centered filtering avoids combining paired Gaussian taps with fractional upsampling in the same pass.
There are still 15 coverage passes rather than five,
with intermediate image writes and reads.
Direct gathering also has variable candidate counts when many stars fit within a seed pixel.
The measurements do not establish which of these remaining costs dominates.

The first version takes seven bilinear samples per filter direction.
The two trimmed versions choose a radius of approximately three standard deviations,
rounded up to an even radius from two through six pixels,
and renormalize the weights.
The dynamic version loops over a uniform pair count;
the final version uses separately compiled one-, two- and three-pair pipelines.
Neither trimming strategy demonstrated a useful improvement over analytic halos.

As in the first experiment,
CPU radial quadrature calibrates representative residual-halo mass and width,
with width capped at 0.35 cells.
Each star's seed mass scales with its squared sigma relative to that representative.
The filter variance subtracts the tent deposit's average variance of one sixth of a seed pixel squared.
This is an approximation:
a common kernel loses individual halo-width variation,
fills the center of the residual differently,
and cannot exactly compensate each subpixel phase.

## Picture and seed verification

The final specialized version was rendered at 960×540 with the light field derived from the private take `take-2026-09-11_03-16-17.wav`,
starting at 72 seconds and spanning 20 seconds,
and with a constant-level control.
A is analytic halos at exactly one-third resolution;
B is gathered blur.
Both use full jitter.

- [Real-take comparison](stars-gather-take.png)
- [Real-take 2× crop](stars-gather-take-2x.png)
- [Flat-field comparison](stars-gather-flat.png)
- [Flat-field 2× crop](stars-gather-flat-2x.png)

The images are close at these dense defaults.
Gathered blur makes the grain somewhat smoother and the glow more uniform.
Mean RGB over the take is `(117.547, 46.887, 79.952)` for halos and `(117.613, 46.891, 79.970)` for blur on an 8-bit scale.
Similar average brightness does not establish matching individual halo profiles.
A three-second side-by-side motion render advances the actual Stars clock through 90 frames at 30 fps while keeping the musical light field static.
Sampled frames showed no obvious gross clipping or banding;
this is not an exhaustive temporal or control-range validation.
The frame harness reproduces the motion render.
No golden baseline was changed.

The scratch GPU test `gathered_seeds_match_independent_tent_deposits_through_full_jitter_motion` compares the seed image against an independent CPU scatter over 17 motion phases.
Its odd 9×11 logical pane uses cells only 0.41×0.67 seed pixels wide,
so more than nine stars can contribute to a pixel.
Jitter reaches both 0.2 and 0.8 cell-center limits.
An 18×14 atlas slice starts at index 2040,
crossing a 2048-texel row;
bright sentinels outside the slice expose incorrect row or depth reads.
Each star contributes mass 0.05.
Every GPU pixel agrees with the CPU tent deposition within 0.0003,
and total mass stays within 0.01 of 12.6 through all phases.
[The final test log](images-static.log) records the seed test and frame probe passing.
This verifies seed conservation and addressing for that fixture,
not equivalence to the analytic halo or arbitrary parameter extremes.

## Limitations and decision

This implementation did not earn the complexity of another production rendering path.
The renderer,
UI selector and persisted experimental enum were restored to the baseline;
only this evidence remains.
The supported controls and defaults are unchanged.
Lowering analytic Halo resolution remains the demonstrated saving from the [previous paired test](../halo-resolution/README.md).

The scratch code also has two known costs to address before any future adoption:

- Extra atlas padding is appended after the existing atlas-cap fitting.
Small panes or extreme star sizes can exceed that cap or overflow size arithmetic.
The tested 1080p and 4K fixtures do not exercise this limitation.
- Border planning repeats the five 128-step radial quadratures during CPU preparation despite the target kernel cache.
For example,
the first 1080p run reports median preparation of 0.179 ms for blur versus 0.137–0.139 ms for its controls.
This CPU work is outside the measured GPU interval;
the saved run logs retain those medians.

Kernel cache inputs include pane aspect and response parameters;
drift,
lifetime and color update image contents rather than the kernel.
The gathered seed test deliberately exercises dense sampling and atlas boundaries,
but the full extreme-control and fractional-scale matrix has not been verified.
Fixing these limitations without a demonstrated GPU win would add more work to an already rejected approach.

The result rules out this separate gathered-seed plus two-filter prototype as an improvement over the one-third analytic baseline on the measured device and fixtures.
It does not rule out fused seed/filter generation,
shared-data analytic compute,
or every possible convolution implementation.
Those remain unmeasured proposals.

## Reproduce

Use an owner-managed worktree at the baseline revision above,
with the evidence files available.
The final `prototype.patch` includes the implementation,
paired timing harness,
frame probe and GPU seed test.
Use source shaders;
the experiment does not update the production Metal corpus and is not intended for a plugin handoff.

```sh
git apply docs/evidence/spectrogram-stars/gathered-blur/prototype.patch
HARMONIGRAPH_SHADER_ASSETS=source cargo test --release -p harmonigraph-render --lib --no-run
python3 docs/evidence/spectrogram-stars/four-neighbor-current/run.py --binary /path/to/printed/test-executable --output /private/tmp/stars-gather-repeat
HARMONIGRAPH_REQUIRE_GPU=1 HARMONIGRAPH_SHADER_ASSETS=source cargo test --release -p harmonigraph-render gathered_seeds_match_independent_tent_deposits_through_full_jitter_motion -- --nocapture
```

`prototype.patch` alone reproduces `timings-static`.
To reproduce the initial `timings-v1`,
apply `full-filter.patch` after it and rebuild.
To reproduce `timings-tuned`,
apply `dynamic-filter.patch` after the prototype and rebuild.
These two variant patches are alternatives,
not cumulative.
Reverse the selected variant before reversing the prototype.
Both variant patches and the full prototype were checked for clean application and reversal against the baseline files.

[prepare-input.py](prepare-input.py) creates the frame inputs in `/private/tmp/stars-gather-blur` using the look-prototype kit and the private take.
Then run the ignored `scratch_take_stars_frames` test with `HARMONIGRAPH_SHADER_ASSETS=source`.
`PROBE_JITTER=1` selects full jitter;
`PROBE_VIDEO_FRAMES=90` produces three seconds at 30 fps and requires `ffmpeg`.
Run once without the video variable to include the flat field.
[sheet.py](sheet.py) assembles first-frame comparisons in the scratch `sheets` directory.

Raw GPU samples are stored as compressed CSVs in each series directory,
with CPU-preparation summaries in the corresponding logs.
The analyses preserve means and paired ratios:
[full filter](timings-v1/analysis.json),
[dynamic trimmed filter](timings-tuned/analysis.json),
and [specialized trimmed filter](timings-static/analysis.json).
Remove all experimental source changes after the probes.
