# Lattice material prototypes

Clouds, Contours and Interference are the selectable materials in **Lattice → Light → Background glow texture and breathing → Material**.
They texture the combined note light;
the note rings and labels retain their existing drawing.
There is no extra render pass, simulation history, texture upload or material cache.

| Material | Construction |
| --- | --- |
| Clouds | Original shared cloud texture; the default |
| Contours | Nested bands following the accumulated glow coverage |
| Interference | Two curved procedural wave fields forming fringes |

Interference is an artistic pattern illuminated by the notes, not a physical simulation of waves emitted by each note.
All materials multiply premultiplied RGBA together, preserving hue and the glow ceiling;
none creates light on silent ground.
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

The renderer property test `materials_texture_the_combined_light_without_creating_or_recoloring_it` exercises each material with one note, two separated notes, and 32 overlapping notes.
It checks spatial variation, distinct selector results, color/coverage bounds, motion, freeze, and disabled equivalence.
The fixture uses a broad texture so the half-resolution glow target actually resolves its fringes.
Clouds remains the default, and all original lattice and offline reference images remain unchanged.
The existing atmosphere persistence sweep uses a non-default material to check missing-key defaults and round trips.

Visual comparison uses the production offline renderer with `take-2026-09-22_05-10-12.take`, 8.4–12.4 seconds, full lattice pane at 1280×960.
This is a recorded sustained chord;
no synthetic background image is supplied to the shader.
Scratch frame and timing probes are removed after verification.

During the initial comparison, a 1536×1536 dense lattice probe on Apple M1 Pro with 355 lit halo instances and 30 names measured median GPU preparation plus composite times of 4.879 ms for Contours and 4.881 ms for Interference over 120 warm frames.
The p10–p90 ranges overlapped (roughly 4.5–5.3 ms), so these are comparable whole-frame costs rather than evidence of a precise ordering.
They exclude the DAW audio path and editor UI work.
Material cost does not multiply by note count.

Yan's selection was: “I like contours and interference.” Fibres and Liquid have been removed from the selector, saved enum and shader; Clouds remains the default.
