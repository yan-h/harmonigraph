# Selected far-three 75% production port

Production draft: [#1248](https://github.com/yan-h/harmonigraph/pull/1248).
Implementation issue: [#1247](https://github.com/yan-h/harmonigraph/issues/1247).
[Full architecture exploration and decision](https://github.com/yan-h/harmonigraph/issues/1142#issuecomment-5862242676),
with replayable prototype evidence in [#1246](https://github.com/yan-h/harmonigraph/pull/1246).

## Fixed production policy

Optimized draws all five layers.
The complete farthest three responses use a four-neighbor gather and are composited together into an RGBA16Float image at 75% width and height,
then sampled bilinearly.
Their glow ends at `1 - 0.3 * Jitter` cell widths,
with a fade over the final 0.15 cells;
jitter itself is unchanged.
The nearest two retain native cores and their existing 100% and 60% halo images.
Uniform retains its previous five wide responses and native 3+2 split threshold.
The far-three halo images and passes are absent in Optimized.

Target allocation is keyed on actual rounded dimensions and active halo depths.
Color-history identity excludes sampling policy,
so changing profile preserves accumulated color.
The shader receives actual allocated far-target dimensions,
and cropped-pane scissors cover the bilinear taps beyond the region boundary.
No research switches or new persisted fields are shipped.
Existing saved Optimized appearances adopt the selected response.

## Production versus accepted prototype

`accepted-prototype-parity.json` compares every RGB pixel of the same 144 warmed frames at 768×432 and 24 fps.
99.9943298% of pixels are identical,
mean absolute channel difference is 0.0000189217 on the 0–255 scale,
and the maximum is one channel level.
`accepted-prototype-parity-4k.json` compares retained frame 72 at 3840×2160 after the same 960-frame warmup:
99.9943938% of pixels are identical,
mean absolute channel difference is 0.0000187275,
and the maximum is one channel level.
These measure port fidelity to the selected prototype,
not the selected look’s difference from the previous production shader.
The latter remains in the complete research findings.

The JSON records exact render commands and input/binary hashes.
The rendered binary was built from the production shader now in this PR,
using `HARMONIGRAPH_SHADER_ASSETS=source` before corpus regeneration.
The first low-resolution script completed all comparisons and wrote its JSON before an optional PNG export failed because Pillow was unavailable;
that did not affect the measurements.
The 4K export used the existing PNG writer and completed successfully.

## Production timing confirmation

The existing `cloud_costs_by_style_and_dial` probe measured the GPU interval from the first source pass through final compositing.
Each process used 240 measured frames after the probe’s ten-frame warmup,
a synthetic noisy grid,
full coverage,
and the unchanged ten-second history fixture.
The reported case is `stars, defaults`,
with color memory off.
A/B/B/A ordering brackets two candidate runs with two baseline runs;
this is a bounded production check,
not the research campaign’s interleaved per-frame statistical analysis.
No local builds or captures ran concurrently.

The baseline executable is the preserved research harness from #1246,
loading the unmodified combined WGSL at `c5c0cb77` (merged native 3+2 #1245).
Its runtime policy is the baseline five-halo path;
the intervening production Rust changes were comments only,
and `Cargo.lock` is identical.
The candidate is the production test executable.
Both use source shaders and the same standard probe;
binary hashes and all percentile summaries are retained.
CPU wall/prepare values from different test build profiles are not compared.

| Pane | Baseline run medians | Candidate run medians | Reduction from mean of run medians |
| --- | --- | --- | --- |
| 1920×1080, 2 px/pt | 7.156, 7.164 ms | 4.916, 4.912 ms | 31.4% |
| 3840×2160, 4 px/pt, repeat | 19.078, 19.052 ms | 14.771, 14.771 ms | 22.5% |

The first 4K sequence is retained in `production-timing.json` and its logs:
the baseline moved from 19.096 to 22.946 ms,
while candidate medians were 14.930 and 14.986 ms.
That substantial control drift disqualifies its percentage as the final estimate.
`production-timing-repeat.json` contains the stable repeat reported above.
These are GPU offscreen intervals,
not end-to-end DAW frame times,
and are not added to #1245’s already-shipped gain.

## Regression checks

`optimized_far_layers_cover_partial_panes_at_fractional_scale` reaches an interior 121×91 far target on an odd-sized pane at 125% scale with a nonzero origin.
It tests minimum,
default,
and maximum jitter,
with memory both off and on.
Full-region and cropped-region production pixels must match exactly inside the region,
including the first and last covered pixel centers.
The source mesh and lighting remain identical.
A separate nine-neighbor response comparison allows at most one channel level and mean below 0.01/255 for changed subtraction order.

Existing checks preserve Uniform’s native split parity,
wide-response reconstruction,
original array lookup,
and retained color across profile transitions.
Target-layout checks distinguish omitted far depths even at 1×1 sizes.
The reviewed golden changes are the short-pane,
tall-pane,
zoomed-in,
starfield,
and mixed spectral-shadow fixtures,
all of which use Stars.
Watercolor and the lattice frames remain unchanged.

All 73 spectrogram regressions pass,
as do changed-crate all-target Clippy checks and the workspace golden suite (including 15 lattice goldens).
The Metal corpus was regenerated and validated on [runner 36371196423](https://github.com/yan-h/harmonigraph/actions/runs/36371196423),
including strict catalog,
renderer/offline goldens,
and binding/fallback controls.
It is included in the same commit as the shader change.
