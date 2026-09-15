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
Blur and Lava weight each mapped bucket with the fixed display transform `x * (0.1 + 0.9 * x)` before pitch or time resampling,
so narrow bright content contributes before the reduced source footprint averages it.
The 10% linear toe preserves quiet values in `R16Float`.
This is an artistic brightness weighting,
not RGB gamma correction or another audio-power average.
Both Gaussian scales and their Spread combination remain in that encoded domain.
The reduced material bake applies the stable inverse `2*y / (0.1 + sqrt(0.01 + 3.6*y))` once per reduced pixel.
The final composite linearly upsamples decoded intensity,
accepting a small interpolation approximation on the smooth field to avoid a square root per full-resolution pixel.
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
Contour strength fades in across the first band,
so quiet values approach their input brightness rather than collapsing to the bottom terrace.
Higher bands keep their existing contour treatment.
Zero stays zero.
Additional styles belong in this transfer stage and its selector,
without copying history,
cache or upload machinery.

The measurement cache is keyed by column identity/range and slab width.
None of the style controls belong in it.
The scalar targets retain their dimensions while each stays within 10% of the requested size,
so small zoom and Span changes do not continually allocate textures.
Full-resolution axes must match the visible pixel dimensions exactly,
and retained dimensions never exceed those dimensions after a resize.
The filters and source footprint use the actual retained size;
contents and uniforms refresh each draw.
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
2026-09-14.
The complete-frame figures below precede the brightness-weighting revision;
they are a baseline,
not measurements of the current shader at 4K:

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

The brightness revision raised a matched 3.2-pixel Lava ridge from 56 to 109/255 at its peak.
A focused 1024 × 768 zoom comparison used the old and new shaders in one release executable,
four alternating-order repeats,
48 warmup frames and 240 measured frames per run,
in both orientations and zoom directions.
One isolated repeat followed the initial run.
Median times stayed close:
3.25–3.38 ms initially and 3.22–4.64 ms on the repeat.
Tail timings were noisy,
including substantially more new-shader stalls in one repeat case;
these measurements do not establish unchanged frame-drop behavior.
They include GPU completion and full readback,
exclude analysis and UI layout,
and do not measure Bitwig presentation.
The evidence file retains both runs rather than selecting the cleaner result.

A targeted diagnosis then interleaved old and new renders at the same pitch zoom,
reversing their order each pair and retaining both warmed resource sets.
Across 600 measured pairs,
total medians were 3.250/3.243 ms and 95th percentiles were 4.106/4.116 ms (old/new),
with neither version exceeding 16.7 ms.
The new 99th percentile remained higher (7.734 versus 4.622 ms),
primarily in the readback/wait stage.
CPU preparation and render submission medians were essentially unchanged.
The earlier long stalls did not reproduce in the paired comparison,
but this remains a headless completion/readback result rather than a live frame-drop guarantee.
GPU timestamp instrumentation stalled Metal's wait and was removed from the successful diagnosis,
so the wait stage is not an isolated GPU execution measurement.
