# Spectral renderer measurements, 2026-09-27

These measurements address #1183, #1184 and #1190 on an Apple M1 Pro using Metal.
GPU intervals use the corrected first-source-to-final-composite probe from #1207;
they are synchronous headless measurements, not DAW frame rates.
Memory is off unless a case explicitly enables it.

## Shared modules

Each constructor now creates one spectrogram or roll shader module and shares it between its pipelines.
Three editor openings, with a fresh device on each opening, measured:

| Constructor | Before, ms | Shared module, ms |
| --- | --- | --- |
| Atmosphere | 65.61 / 60.63 / 87.62 | 17.83 / 16.15 / 13.74 |
| Roll | 12.66 / 17.54 / 21.39 | 6.96 / 6.50 / 5.29 |

The plain spectrogram still needs its own single module.
The first opening includes cold device initialization;
subsequent openings exercise the driver's retained caches.
Repeat with `cargo test -p harmonigraph-render timing_editor_pipeline_startup -- --ignored --nocapture`.

## Inactive Stars code does not justify more pipelines

The #1184 probe replaced each `if cloud.cloud_style == 2u` arm with `if false` in a second shader module.
Shipping and stubbed resources were interleaved every frame at 3840×2160, two pixels per point,
with ten warmup frames and sixty measured frames.
Four runs alternated case order.
The table reports stubbed / shipping median GPU time:

| Case | Run 1 | Run 2 | Run 3 | Run 4 |
| --- | --- | --- | --- | --- |
| Mosaic | .984 | .985 | .982 | .971 |
| Mosaic, memory | .964 | .951 | .966 | .966 |
| Watercolor | .969 | .985 | .968 | .967 |
| Watercolor, memory | .962 | .983 | .960 | .974 |
| Blur only | 1.000 | .962 | 1.016 | .941 |
| Blur only, memory dials on | .972 | .978 | .992 | .959 |
| Terraces only | 1.020 | .997 | 1.004 | .994 |
| Terraces only, memory dials on | 1.004 | .999 | .999 | .995 |

No case meets the issue's requirement of at most .95 in every run.
Keep the shared shader and avoid additional style pipelines.
Memory dials do not allocate memory when cloud depth is zero;
those controls retain that production behavior.

## Retain F32 color history

A format-only half-float trial halved history residency from about 253.5 to 127 MiB at 4K.
Paired 4K control rows were near 4.0 ms;
memory rows were 6.102 / 6.095 ms with F32 and 5.404 / 5.987 ms with half storage.
The half-storage p10 was 5.385 / 5.388 ms versus 6.072 / 6.061 ms with F32.
This suggests about .69 ms less bandwidth cost at the lower end,
but one half-storage run was noisy and the 1080p result was inconclusive.

The proposed minimum-one-quantum convergence rule changes more than the final release tail.
For a 243→255 grayscale palette, a one-second response with both time constants set to five seconds produced these displayed values:

| Half storage | 1 fps | 60 fps | 120 fps | 240 fps |
| --- | --- | --- | --- | --- |
| Release from white | 253 | 252 | 248 | 243 |
| Pickup from palette floor | 245 | 246 | 250 | 255 |

A normalized 16-bit trial was also rejected.
Stored RGB comes from a normalized palette followed by sRGB-to-linear conversion;
Stars brightness changes palette position before its clamp, not stored RGB gain.
Stored level comes from clamped spectral intensity, positive normalized filtering and bounded blends.
Stars fringe and final composition do not feed history.
An F32 GPU readback of all three styles, black/full/mid inputs and extreme sanitized effect settings confirmed every stored channel was finite and in [0,1].

The local adapter advertises filterable/renderable `Rgba16Unorm`.
Actual texture creation requires both `TEXTURE_FORMAT_16BIT_NORM` and `TEXTURE_ADAPTER_SPECIFIC_FORMAT_FEATURES`.
No production feature requirement or remote capability probe was added because the local accuracy check failed:

| UNORM release trial | Stored linear RGB | F32 exponential reference | Displayed sRGB8, trial / reference |
| --- | --- | --- | --- |
| 5 s, 240 fps, white→black after 25.35 s | .00013733 | .00628242 | 0 / 19 |
| 5 s, 60 fps, same transition/time | .00637827 | .00628242 | 19 / 19 |
| .6 s, 240 fps, near-black→black after 1.521 s | 0 | .00075128 | 0 / 2 |

The targeted GPU trial used the production memory pass with manual display sampling retained,
so storage rounding was the changed operation.
Its rule can be reproduced for each channel with:

```text
alpha = -expm1(-dt / tau)
prior = round(previous * 65535)
goal = round(current * 65535)
blended = round(mix(previous, current, alpha) * 65535)
if alpha > 0 and blended == prior and goal != prior:
    blended = prior + sign(goal - prior)
next = blended / 65535
```

Paused frames keep the previous value exactly.
Compare against `goal + (initial - goal) * exp(-elapsed / tau)` before the sRGB display conversion.
The high-frame-rate dark-tail error outweighs the measured bandwidth saving;
keep F32 rather than add residual accumulation or stochastic rounding machinery.

## Memory allocation and region size

A simulated one-pixel-per-frame width drag at 4K used ten warmup frames and one hundred measured frames,
with memory-on/off stationary and moving cases interleaved.
Two runs reversed the case order.
Exact-sized memory reallocated on all 110 frames:

| F32 case | GPU median, ms | Submit + wait median, ms | Prepare median, ms |
| --- | --- | --- | --- |
| Stationary, memory | 5.997 / 5.778 | 8.048 / 7.967 | .167 / .156 |
| Drag, memory | 10.636 / 9.861 | 26.004 / 21.713 | .489 / .450 |
| Stationary, no memory | 4.747 / 4.654 | 6.768 / 6.692 | .174 / .151 |
| Drag, no memory | 4.694 / 4.580 | 6.869 / 6.771 | .163 / .166 |

Physical allocations now round up to 64-pixel buckets, with at most 63 spare texels per axis.
The same drag fixture now reallocates three times in 110 frames.
A confirmation run measured prepare medians of .209 ms while dragging and .220 ms stationary;
the no-memory controls were .244 and .199 ms.
GPU and submit/wait intervals also showed no added drag cost within that run,
but all cases ran substantially slower than the earlier baseline,
so no absolute GPU speedup is claimed across runs.
The logical extent still sets every sampling coordinate, drift shift and memory-pass viewport.
Logical extent also participates in the history key:
a sampling-density change can reuse an allocation now, but must still reset history.
The GPU image test compares displayed pixels and stored texels exactly against unbucketed allocations,
including stationary response, within-bucket resize, bucket crossings, a nonzero pane origin and changed sampling density.
The sampling-change step asserts that pane geometry and physical allocation stay identical while logical extent changes.
Removing the logical-extent key makes that step fail on displayed pixels,
confirming that the fixture reaches the newly possible stale-history path.

At full material depth, the composite also skips the unused base-level lookup and palette work.
A separate original-shader comparison of 24 memory-on frames across Mosaic/Watercolor and blur on/off,
with active pickup and release, changed zero of 1,572,864 displayed channels.

A separate traffic bound reduced history texels by 22.3% while retaining the same composite work.
It saved .304 / .308 ms and about 56 MiB at 4K.
That probe changed sampling density and is not a correct cropping implementation.
Correct region-only history would need additional origin/extent handling in material coordinates and drift copies.
Defer that complexity for the measured saving;
#1183 remains open for region sizing and records the rejected format proposal.
