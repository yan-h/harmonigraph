# Stars shader performance

For the current architecture,
controls and the full discussion/decision record,
see [Stars optimization notes](stars-optimization-notes.md).
The dated measurements below preserve their original baselines.

The historical independent-pass timing columns below retain their original meaning.
Issue #1203 demonstrated that the old `end/full` bracket can reverse or undercount;
the corrected probe reports `source/full` from the first real source pass to the composite end.
Current probe cases explicitly select Mosaic and turn color memory off except in named memory cases (#1182).
These changes do not revise the historical values or the later dependency-ordered Stars split measurements.

Measured on an Apple M1 Pro with Metal on 2026-09-26,
against `b26fa162`.
The star atlas already bakes each star's position,
life,
size and palette color once per frame;
the remaining composite walks nine neighboring cells at each of five depths.

## Unroll the rows and bake inverse sigma

Writing out the three neighbor rows removes loop overhead while preserving the order of all nine additions.
Keep the five-depth loop:
unrolling both loops was slower.
Store inverse sigma in the atlas's existing half-float slot,
so each covered pixel multiplies by it instead of dividing by sigma for the core and fringe.
This adds no resource,
pipeline,
cache key or persisted setting.

These changes reduce measured GPU time by about 20% at 4K and 16% at 1080p.
The timings below are medians of 120 frames after ten warmup frames per case,
with cases interleaved in one run.
The second 4K run reverses their order.

| Pane and settings | Original | Rows unrolled | Rows and reciprocal | Combined reduction |
|---|---:|---:|---:|---:|
| 3840×2160, defaults | 19.581 ms | 17.244 ms | 15.686 ms | 19.9% |
| 3840×2160, reversed-order repeat | 19.874 ms | 17.595 ms | 16.007 ms | 19.5% |
| 1920×1080, defaults | 5.070 ms | 4.674 ms | 4.284 ms | 15.5% |
| 3840×2160, 2% history coverage | 13.883 ms | 12.381 ms | 11.663 ms | 16.0% |
| 3840×2160, Fringe 0 | 19.200 ms | 17.005 ms | 15.310 ms | 20.3% |

These are the existing `cloud_costs_by_style_and_dial` probe's `end/full` GPU intervals,
covering light preparation,
star baking and compositing.
They are not DAW frame times:
audio,
analysis,
egui,
presentation and CPU submission are outside them.
The fixture uses a fixed 1024-slab grid of 3828 buckets,
a ten-second visible span,
two pixels per point and an animated star clock.
The 2% case changes rasterized history coverage rather than slab count.
Every variant was compiled from WGSL source with `HARMONIGRAPH_SHADER_ASSETS=source`.
Absolute timings vary with background load;
only this GPU was measured.

### What the picture changes

Row unrolling was byte-identical in four 960×540 diagnostic frames.
Packing the reciprocal rather than sigma changes half-float rounding:
the combined change moved at most one RGB code value out of 255 in those frames,
with mean differences of 0.0036–0.0056/255 and 1.1–1.7% of pixels affected.
The frames cover defaults,
a later life clock,
Fringe 0 and maximum density/fringe.
These measurements describe that fixture rather than a bound across all settings.
Star placement,
count,
depth,
life and palette selection are unchanged.
The production offline Stars golden changed by mean 0.002/255 and maximum 1/255,
and was re-baselined after inspecting its amplified difference sheet.

### Alternatives that did not earn their complexity

Sequential screening runs placed the original at 16.93–16.95 ms at 4K.
They were followed by the interleaved validation above for the successful pair.

| Experiment | GPU time | Outcome |
|---|---:|---|
| Reject impossible cells before atlas load | 18.78 ms | Extra arithmetic and branching did not pay for the skipped reads |
| Reject on squared distance before taking its square root | 17.38 ms | No demonstrated gain |
| Simplify slice compositing | 17.07 ms | No demonstrated gain |
| Pack the atlas into RG32Uint | 19.17 ms | Reduced bandwidth did not pay for decoding |
| Truncate nearly invisible falloff tails | 18.25 ms | Slower despite changing the picture |

Removing early returns gave only a small screening improvement,
and combining it with row unrolling did not establish enough additional benefit to change the silent-star path.
Unrolling both neighbor rows and depth slices cost about 21.6 ms against a roughly 17.3 ms control.
Caching the star bake across frames is inappropriate:
its contents depend on drift,
life fade and the current light under each star.

## Picture-changing candidates, measured but not shipped

The [preserved prototype patches and evidence](evidence/spectrogram-stars/README.md) include runnable shader changes,
raw timings and labeled visual comparisons.

A second interleaved run compares more aggressive source-only prototypes with the optimized shader above.
All figures below include the existing light and star-bake passes;
the bake still processes five layers even when a prototype draws only three.
Percentages are additional savings against the optimized shader in this run,
not against the original shader in the earlier table.

| Prototype | Reads per pixel | 4K GPU | 4K reduction | 1080p GPU | Picture change |
|---|---:|---:|---:|---:|---|
| A: optimized current look | 45 | 17.167 ms | — | 4.521 ms | Reference |
| B: four neighbors, five depths | 20 | 9.202 ms | 46.4% | 2.795 ms | Tighter halos, more separation between pinpoints |
| C: three existing depths | 27 | 10.592 ms | 38.3% | 2.999 ms | Less layered dust and parallax |
| D: four neighbors, three depths | 12 | 6.312 ms | 63.2% | 2.010 ms | Both changes; thinner and darker field |
| E: four neighbors, half the jitter | 20 | 9.386 ms | 45.3% | 2.766 ms | Wider halos than B, more regular star placement |

B reads the nearest 2×2 cells,
starting at `floor(r - 0.5)`.
A center jitters at most 0.3 cells from its cell's middle,
so any excluded cell's nearest center is at least 0.7 cells away.
The radial shape must therefore fade completely to zero at 0.7;
this prototype fades between 0.6 and 0.7 cells,
instead of the current 0.84 to 1.2.
It preserves every star's position,
life,
color and layer,
but removes halo coverage.
E halves the jitter span from 0.6 to 0.3 cells,
which allows reach 0.85 and a fade between 0.7 and 0.85;
it buys back softness by changing star positions.
Neither is a drop-in exact 2×2 replacement for the existing 3×3 walk.

C keeps existing layers 0,
2 and 4,
with their current positions and speeds,
by stepping the composite loop by two.
D combines that omission with B's smaller support.
The prototypes deliberately leave atlas layout and baking unchanged to isolate the composite and picture tradeoff.

The comparison renders run the actual candidate WGSL over a light field derived from the prototype kit's real recording excerpt,
72–92 seconds of `take-2026-09-11_03-16-17.wav`,
with the kit's palette.
They include 960×540 stills,
2× crops,
a flat-field control and five-second motion comparisons.
They are visual prototypes rather than a replay of a saved Bitwig appearance.
B keeps all five depth layers and their irregular placement while taking about half the remaining GPU time,
so it is the first candidate to evaluate visually.
D offers more savings at the cost of a more substantial change in density and depth.
Reduced-resolution RGB compositing remains an unmeasured alternative;
it would need another target and would soften the finest pinpoints.

## Native far-layer split

For drawn Stars coverage of at least 2560 × 1440 device pixels in area,
render layers 0–1 over the palette floor into a native-resolution RGBA16Float target,
then render layers 2–4 over its exact texel in the final draw.
All five layers and their nine-neighbor halos remain.
The intermediate is gamma-coded RGB;
texture depth mixing and output color conversion happen once in the final draw.
No setting or persisted state changes.

The target reuses the cloud precomposite allocation and bind group.
Its size and the Stars atlas presence are already part of the allocation key,
including the format change when switching cloud styles.
The pass runs after color memory and star baking on every drawn frame.
Its scissor covers the union of the backdrop region and measured mesh,
so a combined spectrum pane does not shade a far layer over its unused area.
The cutoff uses that drawn coverage rather than the whole callback rectangle.
A split toggle carries compatible color history through the existing allocation path.
At 4K the additional RGBA16Float image uses about 63 MiB.

Use specialized final pipelines for the split and unsplit paths.
A runtime uniform branch made the unsplit path about 8% slower at 2304 × 1296 in an implementation comparison;
leaving the switch in the shader would penalize panes that never allocate the intermediate.
The override-false shader folds that branch away.

The original prototype's longer comparisons found 14.6–17.3% lower GPU time at 4K,
and no dependable benefit at 1080p,
on M1 Pro / Metal.
Intermediate-size screening found the crossover near 1440p;
the area cutoff keeps smaller Stars regions on the original walk.
See [issue #1178](https://github.com/yan-h/harmonigraph/issues/1178) for the original evidence and acceptance criteria.
These measurements do not establish a gain on every GPU or in full DAW frame time.

The regression fixtures compare split and native output within one 8-bit color step,
including a fractional display scale and pane origin,
clipped cloud coverage,
partial depth,
color history while toggling the split,
star-control extremes,
and an above-threshold sRGB target that encodes the extra pass.

The [final production comparisons](evidence/spectrogram-stars/split/README.md) use timestamps on the actual source and composite passes.
They confirm approximately 14–17% savings at dense defaults from 1440p through 4K,
with the smaller-pane fallback effectively unchanged.
The independent opening-pass end stamp used in earlier probes can undercount;
[issue #1203](https://github.com/yan-h/harmonigraph/issues/1203) records the measured failure.

## Earlier four-neighbor plugin trial

The four-neighbor trial at `4a35083afe12753a9e9264c4a37a2cd9565a867d` enabled configurable positional jitter for evaluation in the DAW.
It retains all five depth layers,
size and speed settings,
lifetimes and color memory.
The final walk selects the nearest two cells on each axis,
reducing 45 candidate evaluations per pixel to 20.
The new Jitter slider defaults to 50%,
halving positional variation from 0.6 to 0.3 cell widths and moving stars closer to their cell centers.
This permits halos to fade between 0.7 and 0.85 cells,
about 21% farther than the preceding full-jitter trial's 0.7-cell limit.
The original nine-neighbor renderer fades from 0.84 to 1.2 cells.
The full-resolution 2+3 split and its area cutoff remain active.

The Jitter slider runs from regular cell centers at 0% to the original positional variation at 100%.
The halo window follows it automatically:
0% fades from 0.8 to 1.0 cells,
50% from 0.7 to 0.85,
and 100% from 0.6 to 0.7.
All positions keep the same four-neighbor walk and five layers.
Bounds are computed once per frame and shared by the draw and color-memory paths.
Moving Jitter resets Stars' retained color because the sample positions have changed;
it does not invalidate other shades' history.

Jitter is saved with the appearance,
and older documents without the field load at 50%.
Existing projects therefore display the half-jitter placement and halo window while this build is loaded.
The Randomness dial still controls brightness and size variation;
it does not control positional jitter.
Loading the ordinary build restores the original look from the same saved settings.
Visual acceptance remains Yan's decision in #1142.

[Paired measurements of the preceding full-jitter trial](evidence/spectrogram-stars/four-neighbor-current/README.md) found 35.5–36.3% lower GPU time at 1080p and 39.6–39.8% at 4K on M1 Pro,
with color memory and the automatic split enabled.
These figures do not measure the half-jitter variant,
and synthetic offscreen comparisons do not establish live Bitwig frame-time savings.
The earlier prototype timings above are also historical,
not a claim of the same saving over the current split renderer.

[A paired half-jitter comparison](evidence/spectrogram-stars/half-jitter/README.md) measured 1.1–2.2% more GPU time at 4K than the preceding four-neighbor trial.
At 1080p the difference was smaller than the variation between identical controls.
These measurements retain the same five layers and current defaults,
but predate the configurable slider and its uniform-driven shader.

[A paired comparison of configurable 50% jitter against fixed half jitter](evidence/spectrogram-stars/configurable-jitter/README.md) finds no clear added GPU cost.
At 1080p the difference fits inside A/A variation;
the 4K candidate is slightly faster but the runs do not establish a reliable speedup.


## One-neighbor cores with separate halos

The current prototype on `codex/stars-four-neighbor` keeps one native-resolution core lookup per depth,
with the rest of each star's response in a reduced-resolution RGBA16Float image.
Halo resolution is adjustable from 25% to 100%,
with the original 50% sampling as its default.
Each of the five depths has its own image;
its halo coverage and weighted color join the native core before the usual color normalization and far-to-near composition.
The reduced pass walks the original nine cells and subtracts the compact core,
so it includes both Fringe and the Gaussian tails outside the core.
The outer response again fades from 0.84 to 1.2 cells at every Jitter setting.
The Jitter slider and its 50% default remain;
100% now combines the original position variation with the original halo reach.

The native core ends at `0.5 - 0.3 * jitter` cells,
which keeps it wholly inside its own cell.
Five native candidates plus 45 at quarter pixel count nominally gives 16.25 candidate evaluations per output pixel,
compared with 20 for the preceding four-neighbor trial.
That arithmetic excludes five filtered halo reads,
five additional render passes and their bandwidth.
At 4K the halo images require about 79.1 MiB beside the retained native two-layer split target's 63.3 MiB.
Allocation follows pane device pixels,
the requested halo dimensions and whether Stars is active;
Jitter changes the drawn values without reallocating these images.
The halo passes fill the whole pane so clipped regions can filter across their edges without seams.

The intended picture change is a wider,
smoother glow with softer fine dust.
Dense small panes change more than large stars;
the half-resolution response cannot preserve every subpixel Gaussian tail.
The compact centers remain native resolution.
Wrapped drift is split into integer and fractional parts before local distance evaluation,
preventing different passes from rounding a large wrapped coordinate differently.

A native-resolution test compares reconstructed coverage against an independent full nine-neighbor response at Jitter 0%,
50% and 100%,
with Fringe zero and maximum.
It also compares against core-only rendering to require visible halos even with Fringe zero.
The existing split tests cover fractional scale,
partial clipping,
retained color,
large-pane activation and extreme controls.
The five Stars-bearing offline goldens change intentionally;
Watercolor and the lattice retain their prior frames.
Yan chose to keep these analytic halos after the [per-depth blur comparison](evidence/spectrogram-stars/depth-blur/README.md).
The branch remains an unmerged visual trial for #1142.


[Paired M1 Pro timings](evidence/spectrogram-stars/separate-halos/README.md) measure 8.3–8.5% lower GPU time at 4K,
but 3.8–4.0% higher time at 1080p,
against the preceding configurable four-neighbor build at default 50% jitter.
The added passes pay off at the larger size in this fixture,
while the smaller pane pays more for the restored wide response.

The [Halo resolution comparison](evidence/spectrogram-stars/halo-resolution/README.md) measures the newer slider independently:
one third versus the 50% default saves 17.6–17.9% of total GPU time at 4K and 7.2–8.7% at 1080p in repeated paired runs.
