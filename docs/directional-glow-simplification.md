# Directional glow simplification trial

The original directional renderer is the visual baseline,
with wide glow already removed in #873.
The flat splat experiment remains on `codex/glow-splat-prototype` for reference;
its color patches and dense-chord blending were not preferred.

## Implemented: fixed halo rim

Every halo uses the view's maximum configured ring/mark rim plus Reach,
scaled with node size.
Marks still contribute their colors through the original angular ink strip,
but appearing or disappearing no longer changes a node's halo footprint.
This removes the per-node mark-size envelope,
its attack/release bookkeeping,
and its scene and GPU inputs.
Unmarked nodes can have a slightly wider halo than before;
a marked node already used this maximum rim.
Breathing still modulates brightness rather than footprint.

Color transitions use the ordinary ring rim independently of that larger footprint.
The glow eases from its mean at the center to fully directional color at the ring edge,
so widening the configured mark rim does not wash out nearby color patches.
This restores the original transition for unmarked nodes;
marked nodes now reach fully directional color closer to their centers than before.
Both radii are fixed view geometry,
with no per-node size envelope.

## Implemented: one renderer-owned ink clock

`GlowStep` carries only brightness,
row identity and row index.
The redundant scene `mix` input is removed.
Timed live and offline scenes keep the existing renderer-owned ink history,
which computes interpolation from the last frame actually encoded for that pane.
Untimed scenes are stateless snapshots seeded from current ink.
The two consumers still need their own timestamps:
UI layout can advance without a corresponding GPU frame.
Brightness is stepped on the CPU,
while directional color history lives on the GPU.

The previously tested attack fix is retained:
a newly lit node starts dark when attack is nonzero,
and keeps its row while approaching its target.
No persisted controls or settings change.

## Prototype: rasterize directional halos into overlap statistics

Option 3 is implemented on `codex/directional-glow-mrt-prototype`,
starting from the merged simplifications in #876.
The prototype keeps the original angular ink strip,
its color history and convolution,
radial falloff,
directional sampling,
overlap strength rule and final color repair.
Only the route from node contributions to the combined glow target changes:

- Draw one halo quad per lit node,
  interpolating node-local coordinates through the vertex stage.
- Evaluate the original directional halo inside each quad.
- Write the overlap rule's sufficient statistics into three hardware-blended render targets,
  then resolve them into the combined light.
- Remove the CPU inverse-projection records,
  tile and global candidate lists,
  and the full-screen per-pixel candidate loop.

The earlier splat experiment supplied the three-target accumulation technique,
but none of its replacement color rules.
This version derives its target formats and blend equations from the original directional overlap rule.
It renders at full resolution,
with no extra blur or lower-resolution sampling.
The existing renderer tests cover isolated notes,
marks arriving and releasing,
mixed-register colors,
high Reach,
breathing,
overlap-strength extremes,
perspective and off-screen halos,
and viewport changes during release.
The offline golden suite checks the shared rendering path as well.

This change is compatible with both simplifications above:
it replaces spatial gathering,
not halo sizing or temporal color history.
It may reduce CPU bookkeeping and candidate scanning,
but adds render targets,
blend bandwidth and a resolve pass.
Dense high-Reach chords are the important benchmark;
large overlapping quads still shade many fragments.
Half-float accumulation order can also change rounding.
Measure CPU and GPU time before claiming an improvement,
and compare the rendered result before reducing resolution.

The prototype uses RGBA16F for summed linear RGB plus contributor count,
RG16F for normalized luminance and coverage screens,
and RGBA16F for the gamma RGBA screen.
These add 20 bytes per target pixel (11.25 MiB at 768×768),
in addition to the existing resolved glow target.
The existing angular strip and its history still provide the directional colors.
Breathing is a separate instance component and never changes the history level or coefficient.

Twelve lattice reference images differ by at most 1/255 per channel,
consistent with half-float blending rounding after each contribution instead of one final store.
The row-reordering regression allows this one-byte difference while still detecting swapped colors.
The contact sheets were visually reviewed before updating those references;
the five offline golden images are unchanged.
All 244 non-golden renderer checks pass (ten diagnostic tests are ignored).

### Local timing comparison

Apple M1 Pro,
release builds using source shader compilation,
24 held MIDI notes (1,025 lit lattice instances),
Reach 4.795,
with breathing,
nebula and bloom enabled.
Three fresh processes per implementation and resolution ran in alternating order,
each with ten warmup frames and 120 measured frames.
Values below are medians of the three per-process medians.

| Target | Original completion | Prototype completion | Original CPU prepare | Prototype CPU prepare |
| --- | ---: | ---: | ---: | ---: |
| 768×768 | 8.407 ms | 7.087 ms | 0.371 ms | 0.301 ms |
| 1536×1536 | 30.295 ms | 13.845 ms | 0.342 ms | 0.289 ms |

Completion measures submission through final paint and readback completion,
including host encoding and waiting;
it is not isolated GPU time or a DAW frame-rate prediction.
The measured reduction is about 16% at 768×768 and 54% at 1536×1536.
Callback construction stays approximately 0.07 ms.
The probe's separate GPU timestamp pairs remained noisy and did not track the resolution cost,
so they do not substantiate a GPU-only speedup claim.
The harness excludes scene derivation,
UI work,
note turnover and audio processing.

This trades roughly 600 net lines of renderer and test machinery for three textures and one resolve pass.
The extra targets consume 11.25 MiB at 768×768 or 45 MiB at 1536×1536,
per lattice pane.
It is a promising testable prototype,
with the visual comparison in the DAW still left to the user.

### Known prototype precision limit

The one-byte reference-image difference is a measured bound for those scenes,
not a guarantee for arbitrary overlap.
The ignored `dense_faint_overlap_order_precision` GPU probe deliberately stacks faint halos at one position.
At 1,024 contributors on Apple M1 Pro,
reversing the color blocks changes the output by up to 2/255 at accumulation zero,
4/255 at 0.5 and 7/255 at one.
The 128-contributor fixture stays within 1/255;
this particular 4,096-contributor fixture reaches the same rounded result in both orders.
This does not yet establish a visible problem in an ordinary lattice scene.
[Issue #878](https://github.com/yan-h/harmonigraph/issues/878) retains the reproduction and next measurements before promoting the prototype.
