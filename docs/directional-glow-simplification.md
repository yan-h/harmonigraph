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

## Deferred: rasterize directional halos into overlap statistics

This is option 3 and is not implemented in this trial.
Keep the original angular ink strip,
its color history and convolution,
radial falloff,
directional sampling,
overlap strength rule and final color repair.
Replace only how node contributions reach the combined glow target:

- Draw one halo quad per lit node,
  interpolating node-local coordinates through the vertex stage.
- Evaluate the original directional halo inside each quad.
- Write the overlap rule's sufficient statistics into three hardware-blended render targets,
  then resolve them into the combined light.
- Remove the CPU inverse-projection records,
  tile and global candidate lists,
  and the full-screen per-pixel candidate loop.

The prototype's `3e4ce223` implementation is a reference for the three-target accumulation technique,
not for its simplified per-node color model.
The later local color voting and hue-cap experiments should not be carried over.
Re-derive the target formats and blend equations directly from the current original overlap rule before using that code.

Start at full resolution,
with no extra blur or lower-resolution sampling,
so this is an implementation comparison against the preferred picture.
Pixel comparisons should cover isolated notes,
marks arriving and releasing,
bass and melody sharing a node,
multi-register C-major triads,
high Reach,
breathing,
overlap-strength extremes,
perspective and off-screen halos,
and viewport changes during release.
Validate the same behavior through offline rendering.

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
