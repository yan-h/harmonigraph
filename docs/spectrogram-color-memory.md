# Spectrogram color memory

Color pickup and Color release are shared by Stars, Mosaic and Watercolor.
The defaults reproduce prototype B: pickup 0.08 seconds and release 0.6 seconds.
Both bars run from zero to five seconds and specify exponential time constants:
after one time constant, about 37% of the previous value remains.
Both zero restores the original immediate rendering path and releases the history textures.
Existing saved appearances missing these fields receive the new defaults.

Each material sample retains linear RGB and its source level.
A brighter incoming level chooses pickup; otherwise it chooses release.
The same `1 - exp(-elapsed / time_constant)` coefficient updates color and level.
Silence therefore draws recent color toward the palette floor without traversing the palette.
The first frame initializes directly from the current picture, including an export that starts partway through a take.
Exports warm history through frames they actually render; audio-analysis preroll does not invent earlier visual frames.

Stars retain one color per depth slice, absolute cell and lifetime.
The previous atlas origin and layout locate each surviving cell, including a drift-period wrap.
New lives and newly entering cells initialize from current light.
The geometry atlas still supplies sharp coverage, fringe, size and life fade;
color history never stores the stars' screen-space shapes.
A star over current silence remains while its retained source level is positive.
Never-lit stars still contribute no coverage.

Mosaic and Watercolor retain color on a fixed material lattice.
Drift splits into an integer lattice origin and a fractional display offset.
Feedback copies exact texels with integer loads; only the final displayed color is bilinearly sampled.
Repeated drift therefore adds no feedback blur.
The ordinary cloud tone pass is skipped while memory supplies color.

With memory enabled, Mosaic and Watercolor's Texture mix blends the plain color and remembered color in linear light.
This is bounded and preserves the zero and full-strength endpoints, but intermediate colors can leave the authored palette ramp.
With both times zero, the original scalar-before-palette Texture mix remains exact.
Zero refraction disables displacement but leaves temporal memory available.

## Lifetime and storage

Each pane owns two RGBA32F history textures.
Half-float feedback is insufficient: small response coefficients can round every update to zero and stall far from the target.
The final manual bilinear read needs no optional float-texture filtering feature.
A native 1920×1080 material field plus its two-texel margin takes about 63.5 MiB for both textures;
3840×2160 takes about 253.5 MiB.
Stars instead allocate one history texel per atlas cell, at most 128 MiB for both buffers at the existing atlas budget.
Memory disabled allocates none of these textures.

History survives changes to the scalar blur target's allocation when its own dimensions are unchanged.
The reset key includes active material coordinates, source interpretation, palette and active style's color controls.
Incoming audio, advancing time, response times, Texture mix and unrelated styles' controls are excluded.
Palette comparisons retain the existing shared-snapshot fast path.
A backward clock or source endpoint, disabled texture, invalid mapping, or a gap above 30 seconds resets history.
Thirty seconds is six times the longest allowed response time, leaving less than 0.25% residual even at that setting.
Repeated draws at the identical clock preserve history exactly, while interpretation changes still initialize it immediately.

## Verification

Production-GPU readbacks check exact material texel carry in both orientations,
all five moving star slices and life turnover,
pickup at 1 fps and 120 fps,
paused redraws, palette edits, seeks and the disabled allocation path.
The established renderer fixtures and offline goldens pin both response times to zero;
they continue to check the immediate rendering path.
The Stars reference was refreshed after the eleven star-specific defaults were captured from the DAW on 2026-09-26;
shared memory, drift and spectrogram defaults were not part of that capture.

## Measured cost

Apple M1 Pro, Metal, optimized test build, two pixels per point, full-pane coverage,
1024 slabs over ten seconds, paired and interleaved off/B cases through the existing GPU timing harness.
These are GPU full-frame medians rather than end-to-end DAW frame times, using the star defaults before the 2026-09-26 capture.
The 1080p run used 40 measured frames per case; the 4K run used 100.

| Style | 1080p off | 1080p B | 4K off | 4K B |
| --- | ---: | ---: | ---: | ---: |
| Stars | 9.16 ms | 8.96 ms | 31.32 ms | 31.45 ms |
| Mosaic | 1.22 ms | 1.67 ms | 2.55 ms | 4.42 ms |
| Watercolor | 1.33 ms | 1.71 ms | 3.20 ms | 4.79 ms |

Stars' difference is within run noise; it is not evidence of a speedup.
An earlier 40-frame 4K run overlapped other desktop work and had much wider tails;
the table uses the quieter repeat.
The real-take visual probe separately rendered all three styles off/B through the offline renderer at 640×360 and 24 fps,
retaining eight seconds after more than two seconds of continuous visual warmup.
