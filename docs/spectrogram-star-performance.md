# Stars shader performance

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
