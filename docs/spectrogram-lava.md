# Spectrogram power averaging and lava

The spectrogram now averages linear power in time.
Plain shows the mapped field;
Blur smooths it;
Lava adds smoothly edged intensity terraces before the palette.
All spectral buckets follow the same rule,
including broadband sound and quiet content.
There is no partial detection,
peak selection,
level normalization,
procedural motion or simulation.

## Measurement and retention

Each FFT-center point sample enters history as an unquantized `f32` power sum and a represented sample count.
Coarsening adds both;
it never averages dB or re-quantizes a previous mean.
Mixed history tiers therefore contribute in proportion to the number of original measurements they contain.
At the fixed live cadence this is time weighting by represented analysis hops.
Gaps do not acquire extra weight from the distance between timestamps.
The existing one-slab jitter hold remains a display copy,
with no fabricated measurements in the accumulator.

A slab uses an `f64` sum and count while it is mutable.
It divides once when read and quantizes to the existing half-dB byte grid.
Further arrivals continue the unquantized sum;
completed slabs and GPU uploads stay byte-sized.
Live incremental aggregation,
batch/offline aggregation and partial first-slab repair use the same accumulator.
No additional FFTs are required.

Coarsening necessarily loses timing within a group.
A full refold can assign that group to a different slab than the original measurements occupied.
The live grid preserves completed raw-arrival slabs rather than rebuilding on every history cascade.
Count weighting preserves totals,
not arbitrary reconstruction of discarded timing or gaps inside a coarsened group.
The existing retention and gap policies remain unchanged.
Likewise,
an offline whole-song analysis using a larger hop is not a bit-identical replay of live analysis.
The shared fold agrees when given the same measurements.

## Smoothing and style

The renderer maps measured power to scalar display intensity before smoothing.
Pitch and time softness are Gaussian widths in cents and milliseconds,
converted using the full unclipped axes.
Rotation transposes those widths;
zoom and resize preserve their musical meaning.
The wide field applies a Gaussian five times wider to the close field.
Spread blends the two normalized fields.
Blur and Lava use that softened field directly;
there is no raw-detail blend.

The existing four separable Gaussian passes are reused.
Each pass has seventeen taps.
Zero softness preserves full resolution on that axis;
both zero widths bypass smoothing.
The scratch images store one `R16Float` scalar per pixel.
When a width grows,
the source resolution decreases along that axis to keep the taps dense.
The source integrates its pitch footprint and exactly integrates the piecewise-linear time read between slab centers,
so reduction cannot skip periodic broadband energy.
Filtering normalizes over the available source rectangle,
preserving constant fields at its edges.

A localized shader style dispatcher transforms the smoothed scalar before the existing palette lookup.
Lava uses several nested terraces,
softens their boundaries,
and fades the terrace contrast when a pixel spans multiple levels.
A small residual slope preserves quiet values below the first terrace.
Zero stays zero.
Additional styles belong in this transfer stage and its selector,
without copying history,
cache or upload machinery.

The measurement cache is keyed by column identity/range and slab width.
None of the style controls belong in it.
The scalar target allocation is keyed by its derived size;
its contents and uniforms refresh each draw.
The palette keeps its existing independent key.

## Saved appearance

Lava is the new default,
with 35-cent pitch softness,
120-ms time softness,
25% spread,
seven contours and 15% edge softness.
The old `diffusion` field is removed,
so its saved value is ignored and the new controls take their defaults.
Analyzer softness and note glow remain independent.
The persisted struct retains container-level defaults and sanitizes every new numeric control.

## Verification

The fixtures cover mixed coarse/fine counts,
raw incremental history across actual tier merges,
partial-window holds,
large gaps,
quiet and silent buckets,
style-independent cache keys,
periodic broadband source reduction,
flat-field preservation and nested lava levels.
Three new renderer goldens compare Plain,
Blur and Lava on the same broad field,
quiet band,
granular patch and black margins.
Existing offline goldens exercise the shared live and whole-song drawing paths.

## Measured cost

Release probes on the local Apple Silicon machine,
2026-09-14:

| Quantity | Result |
| --- | ---: |
| Maximum history bin storage | 119.625 MiB |
| Active slab accumulator | 29.9 KiB |
| Mixed-tier refold after 8,192 arrivals | 5.80 ms |
| Fresh column including allocation and readout | 0.0089 ms |
| Complete frame fixtures at 1024 × 768 | 2.4–4.3 ms |
| Complete frame fixtures at 3840 × 2160 | 5.8–7.4 ms |

History storage is four times the former byte store;
completed display slabs and GPU uploads retain their old byte representation.
Scalar filtering uses four two-byte-per-pixel targets,
whose dimensions follow the two musical widths.
An axis with zero softness retains display resolution;
when both widths are zero the renderer uses the detailed field directly.

The CPU probe has two active bins and measures a median of eleven refolds.
The frame figures are medians of three runs of the existing two-second audio fixture,
including replay,
analysis,
UI and readback after sixteen warmup frames.
The 180-second-span case still contains that two-second clip;
it is not a worst-case full-history timing.
Sequential runs have warmup and scheduling scatter,
so Plain being slower than a style in some rows is not evidence of a shader speedup.
[The retained measurements](evidence/spectrogram-lava/performance.txt) give every case and its method.
