# Lattice material prototypes

This document records the original prototype experiments and measurements.
The current controls and rendering contract are in [Independent lattice texture and material](design/lattice-texture-material.md).
Source roughness has since been removed,
and Watercolor now shares the spectrogram’s geometry controls.
Contours and Interference were later removed as patterns Yan did not like in use,
leaving Clouds the only pattern;
the notes on them below are history.

Clouds, Contours, Interference and Watercolor are the selectable materials in **Lattice → Light → Background glow texture and breathing → Material**.
They texture the combined note light;
the note rings and labels retain their existing drawing.
Clouds, Contours and Interference run in the existing glow resolve without an extra pass or material cache.
Watercolor adds a source target, a resampling pass and cached geometry.

| Material | Construction |
| --- | --- |
| Clouds | Original shared cloud texture; the default |
| Contours | Nested bands following the accumulated glow coverage |
| Interference | Two curved procedural wave fields forming fringes |
| Watercolor | Overlapping washes resampling the notes’ combined RGBA light |

Interference is an artistic pattern illuminated by the notes, not a physical simulation of waves emitted by each note.
Clouds, Contours and Interference multiply premultiplied RGBA together, preserving hue and the glow ceiling.
Watercolor resamples premultiplied RGBA, blending the notes’ colors while preserving the ceiling.
All four produce zero light for silent input.
Contours follows accumulated coverage independently of the color overlap control.

## Trying them

Select a material and adjust texture depth, size, speed and breathing with the sliders.
Glow reach and gain must both be above zero.
Changing the material preserves the current slider values.

Texture depth at zero restores smooth halos.
Texture speed at zero freezes the material, while changing notes still changes its illumination.
Larger texture size gives wider fringes and fewer contour bands.
The finest patterns fade toward their average at small render sizes to limit shimmer.
Reset effects restores the default Clouds settings.

Existing documents missing the new `atmosphere.material` field default to Clouds without resetting other fields.
Saved Clouds, Contours and Interference selections keep their values.
Saves that explicitly name the removed Fibres or Liquid variants are rejected as whole documents by the existing parser, including their saved camera and layout;
the editor reports this in its console and the offline renderer reports it before falling back to defaults.
There are no aliases or migration passes.

## Verification

The renderer property test `materials_texture_the_combined_light_without_creating_or_recoloring_it` exercises Clouds, Contours and Interference with one note, two separated notes, and 32 overlapping notes.
It checks spatial variation, distinct selector results, color/coverage bounds, motion, freeze, and disabled equivalence.
The fixture uses a broad texture so the quarter-resolution glow target actually resolves its fringes.
Clouds remains the default, and all original lattice and offline reference images remain unchanged.
The existing atmosphere persistence sweep uses a non-default material to check missing-key defaults and round trips.

Visual comparison uses the production offline renderer with `take-2026-09-22_05-10-12.take`, 8.4–12.4 seconds, full lattice pane at 1280×960.
This is a recorded sustained chord;
no synthetic background image is supplied to the shader.
Scratch frame and timing probes are removed after verification.

During the initial comparison, a 1536×1536 dense lattice probe on Apple M1 Pro with 355 lit halo instances and 30 names measured median GPU preparation plus composite times of 4.879 ms for Contours and 4.881 ms for Interference over 120 warm frames.
The p10–p90 ranges overlapped (roughly 4.5–5.3 ms), so these are comparable whole-frame costs rather than evidence of a precise ordering.
They exclude the DAW audio path and editor UI work.
Those three material masks do not multiply their cost by note count.
Watercolor’s optional source roughness runs per node splat, while its resampling pass runs once over the combined light.

Yan's selection was: “I like contours and interference.” Fibres and Liquid have been removed from the selector, saved enum and shader; Clouds remains the default.

## Watercolor and source roughness

Watercolor uses the spectrogram’s pure wash geometry with the lattice’s directional note colors and glow history.
Source roughness continuously moves from the selected A2 smooth source at 0% to the B2 ragged source at 100%.
The rough source changes each node before the overlap rule combines their light;
Texture depth scales both source roughness and wash displacement.
Depth zero or the effect switch off restores the existing smooth halo exactly.
Texture speed zero freezes both source and material motion.

For the prototype endpoints, select Watercolor, Texture depth 100%, Texture size 1× and Source roughness 0% or 100%.
Texture size 1× makes cells 23 pixels across in a 560-pixel-high pane.
The fixed wash geometry uses fuzz 25%, lobes 70%, layers 50% and refraction 1×.
Fresh settings still select Clouds and retain its existing appearance.
The new roughness field defaults to zero when absent from a saved document.

The production source preserves configured Reach, falloff and note colors;
it is not the prototype’s hand-placed Gaussian field or its guessed flat colors.
Its radial warp eases back to the configured boundary so no splat is clipped.
The 1.0794 endpoint gain is a fixed prototype tuning, not a guarantee of equal energy for every chord or falloff.

The shared shader contains only geometry, with fuzz and lobes passed explicitly.
The spectrogram keeps its own scalar light and palette mapping.
Two half-float tiles bake the expensive cell searches only when quantized texel density changes;
light, camera, time and source roughness never invalidate them.
The lattice holds the tile independently of resize-dependent targets and applies the washes with two displaced samples plus the original sample for depth blending.
This isolates the reusable geometry from the light representation for Watercolor.

### Production comparison and measured cost

The Watercolor comparison renders a recorded chord through the production offline pipeline after warming glow history from 8.0 to 8.8 seconds.
It uses a 1010×720 frame with the lattice on 95% of its width, then compares the same 640×560 crop as the prototype.
The comparison sets Glow reach 2, gain 0.75, curve 0, wash 0, Texture depth 100%, Texture size 0.77778× and Texture speed 0;
the size compensates for the 720-pixel pane so cells remain 23 pixels across.
Roughness 0%, 50% and 100% show a continuous transition from broad blended washes to separated ragged lobes.

This is a structural port, not a pixel match.
The original prototype used Gaussian sources, guessed flat colors and black node cutouts.
Production keeps the configured falloff, directional note colors, overlap rule and native node illumination, so light remains visible inside node outlines.
The tiled geometry is filtered at a quarter of the scene resolution, whereas the prototype evaluated the geometry directly per pixel.
The local comparison images remain outside the repository.

GPU times below bracket production preparation through the final composite on Apple M1 Pro at 1536×1536.
They include glow, scene, shadows and bloom;
they exclude UI construction, the audio path and initial pipeline creation.
Each comparison derives the same scene for Clouds, Watercolor at 0%, Watercolor at 100%, then Clouds again.
Synthetic names and held MIDI notes isolate the material cost while retaining production note light.

| Held MIDI notes / lit halo instances | Clouds before / after | Watercolor 0% | Watercolor 100% | Warm frames |
| --- | --- | --- | --- | --- |
| 6 / 110 | 6.558 / 6.509 ms | 6.491 ms | 8.376 ms | 60 |
| 24 / 225 | 11.662 / 11.473 ms | 11.764 ms | 15.183 ms | 120 |

The stable six-note run’s p10–p90 ranges were 6.447–6.585 ms for the final Clouds pass, 6.427–6.567 ms for smooth Watercolor and 8.299–8.455 ms for rough Watercolor.
Smooth Watercolor is comparable to Clouds in these measurements;
there is no reliable speedup claim.
Full roughness costs approximately 28–32% more on these heavily overlapping halo workloads because its noise is evaluated per source splat.
A separate run under changing host GPU load took roughly twice as long in absolute terms while retaining a similar dense-case ratio;
these are workload measurements rather than frame-rate guarantees.

Cold first-frame GPU brackets measured 24.069–26.415 ms for Watercolor, including its initial tile bake, versus 7.503–15.652 ms for Clouds.
Those brackets include the rest of the first frame and do not isolate bake time.
Selecting Watercolor or crossing a quantized texture-density boundary can therefore cause a brief one-frame cost increase even though the warm path never walks the glob grid.
At this size the two geometry tiles use 1280² half-float RGBA texels each, about 25 MiB together;
the cap is 64 MiB per live pane, plus the quarter-resolution source image.

The new GPU behavior test reaches the material pass and all three roughness values, verifies premultiplied and peak bounds, and checks motion, freeze, depth-zero equivalence, disabled equivalence and silence after a lit frame.
Two focused regression tests hold the lone-node source ceiling and prevent source identity from being interpreted as a Gaussian shadow kind.
Temporarily removing each fix makes its test fail;
the uncapped source reaches alpha 165 where the configured peak rounds to 153.
The renderer suite and existing offline golden frames pass without blessing, and the persistence sweep covers the new selector and roughness field.

### Mosaic in the production lattice

Removed on 2026-10-01 from both the lattice and the spectrogram: Scales covers its faceted look.
The rest of this section records what it was.

Mosaic uses the spectrogram’s soft-union dome geometry with variety 0.5 and full centre gathering,
matching its default flat-facet reading.
Both materials share `atmosphere_geometry.wgsl` and the lattice’s cached-tile and premultiplied-light resampling machinery.
The spectrogram keeps its scalar light and palette path;
the lattice samples all four channels of the existing combined note light together.
Texture depth, size, speed and breathing retain their lattice meanings.
Source roughness remains specific to Watercolor.
Existing saved appearances keep their selected material;
Mosaic adds a variant without changing defaults or renaming saved keys.

The tile cache is keyed on material and quantized texel density.
Switching styles rebakes even at equal size,
while note light, time, depth and camera changes do not invalidate geometry.
Mosaic allocates one RGBA16F tile instead of Watercolor’s two,
with a 32 MiB cap per pane at 2048² texels.
Its warm pass reads the tile once and samples the source light twice;
it never walks the dome grid per frame.
