# Lattice material prototypes

Four selectable materials sit beside the existing Clouds in **Lattice → Light → Background glow texture and breathing → Material**.
They texture the combined note light;
the note rings and labels retain their existing drawing.
There is no extra render pass, simulation history, texture upload or material cache.

| Material | Construction |
| --- | --- |
| Clouds | Existing shared noise field; unchanged default |
| Fibres | Two families of bent, crossing ridges |
| Liquid | Bright seams between moving, warped cells |
| Contours | Nested bands following the accumulated glow coverage |
| Interference | Two curved procedural wave fields forming fringes |

Interference is an artistic pattern illuminated by the notes, not a physical simulation of waves emitted by each note.
Liquid is a network of caustic-like seams rather than a fluid simulation.
All materials multiply premultiplied RGBA together, preserving hue and the glow ceiling;
none creates light on silent ground.
Contours follows accumulated coverage independently of the color overlap control.

## Trying them

Select a material and click **Try material preset**.
This sets texture depth to 85%, texture size to 1×, texture speed to 1×, and breathing depth to zero.
It preserves glow reach and gain, which must both be above zero.
The same settings make comparisons between materials easy;
the existing sliders remain available for tuning.

Texture depth at zero restores smooth halos.
Texture speed at zero freezes the material, while changing notes still changes its illumination.
Larger texture size gives wider threads/cells/fringes and fewer contour bands.
The finest patterns fade toward their average at small render sizes to limit shimmer.
Reset effects restores the original Clouds settings.

Existing documents missing the new `atmosphere.material` field default to Clouds without resetting other fields.
There are no renamed fields, dropped variants or saved-state migrations.
A document saved with a new material will have that field ignored by older builds.

## Verification

The renderer property test `materials_texture_the_combined_light_without_creating_or_recoloring_it` exercises each material with one note, two separated notes, and 32 overlapping notes.
It checks spatial variation, distinct selector results, color/coverage bounds, motion, freeze, and disabled equivalence.
The fixture uses a broad texture so the half-resolution glow target actually resolves its threads.
Existing lattice and offline reference images are checked without blessing.
The existing atmosphere persistence sweep uses a non-default material to check missing-key defaults and round trips.

Visual comparison uses the production offline renderer with `take-2026-09-22_05-10-12.take`, 8.4–12.4 seconds, full lattice pane at 1280×960.
This is a recorded sustained chord;
no synthetic background image is supplied to the shader.
Scratch frame and timing probes are removed after verification.

A 1536×1536 dense lattice probe on Apple M1 Pro, 355 lit halo instances and 30 names, measured the following median GPU preparation plus composite times over 120 warm frames:

| Clouds | Fibres | Liquid | Contours | Interference |
| --- | --- | --- | --- | --- |
| 4.806 ms | 4.815 ms | 4.870 ms | 4.879 ms | 4.881 ms |

The p10–p90 ranges overlap (roughly 4.4–5.3 ms), so these are comparable whole-frame costs, not evidence of a precise ordering among materials.
They exclude the DAW audio path and editor UI work.
Liquid does the most procedural work per textured pixel;
there is no per-note multiplication of material cost.

Keeping Clouds in its original shader function matters:
a shared-function refactor changed three reference frames by at most 1/255 under Metal despite algebraically identical expressions.
Restoring that function and calling it from the material selector restored byte-exact references without changing their PNGs.
