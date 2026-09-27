# Independent lattice texture and material

## Intent

The lattice currently chooses Clouds, Contours, Interference, Watercolor or Mosaic from one Material selector.
Separate the first three patterns from the two displacement materials so a cloudy glow can feed Watercolor or Mosaic.
The fixed order is note illumination and breathing → texture → material → existing lattice composition.
Texture remains lit by notes;
silence stays dark.
This is not a general layer stack or a new ambient background.

## Settings and controls

Keep `ViewConfig::atmosphere` as the owner of lattice glow effects.
Replace its combined enable/material controls with two independently selectable stages:

- Texture: None, Clouds, Contours, Interference; depth, size and speed.
- Material: None, Watercolor, Mosaic; amount, size and speed; Watercolor-only source roughness.
- Breathing: depth and speed in the Background glow controls, independent of either stage.

Use a flat `AtmosphereSettings` with `texture`, `texture_depth`, `texture_scale`, `texture_speed`,
`material_style`, `material_amount`, `material_scale`, `material_speed`, `source_roughness`,
`breath_amount` and `breath_speed`.
Two small enums express the valid choices;
None bypasses a stage without resetting its numeric settings.
The `material_style` key deliberately replaces the old `material` key:
serde ignores the retired key without any compatibility shim or dropped-variant parse failure.
The existing container-level serde defaults and normalization remain the persistence boundary.
Keep texture defaults numerically equal to today's Clouds settings and preserve breathing defaults.
Material defaults to None, with amount 1, size 1 and speed 1 ready when selected.
Reuse the current size/speed bounds for both stages.
Reset texture and material independently;
breathing has its own reset beside the glow controls.
Hide stage-specific controls when None is selected;
dim effect controls when glow reach or gain is zero.

## Rendering

Preserve the existing glow-statistics and resolve passes.
The resolve applies only the chosen texture to combined note light.
The material pass consumes that resolved texture using the existing optional source target and geometry tiles.
No extra render pass or target is introduced beyond those Watercolor/Mosaic already use.
Compute texture and material drift independently from the existing decorative clock.
Keep texture uniforms distinct from material uniforms so neither stage borrows the other's depth, scale or clock.
Watercolor source roughness stays a pre-splat operation tied to Watercolor amount and material motion;
it must not use texture depth or drift.
All texture attenuation and material sampling preserve premultiplied RGBA and the existing brightness ceiling.
No display modulation feeds back into ink history.

While the material is active, its geometry remains keyed only by material kind and quantized tile resolution.
Texture settings, amount, roughness, drift, illumination and camera do not invalidate geometry.
The active material source target has a lifetime bounded by the glow allocation;
resizing the glow replaces that source as before.
None or zero material amount releases both source and geometry resources and skips the pass, as before.
Reactivation recreates them; changes between positive amounts retain geometry.
None or zero texture depth bypasses modulation without disabling the material.

## Saved-state consequence

Old `enabled`, `material` and `nebula_*` keys are retired and ignored.
Old texture/material selections and tuning reset to the new defaults;
existing breathing and source roughness values survive.
An old disabled atmosphere can therefore load with the default Clouds and breathing active.
Camera, layout and unrelated appearance settings survive.
No migration, alias or compatibility struct is introduced.
New saved texture and material controls round-trip and normalize for both editor and offline rendering.

## Verification

Adapt existing per-effect render tests to the new ownership without weakening their fixtures.
Add a combined-stage render test covering all three textures with both materials:
combined output differs visibly from either stage alone, remains premultiplied and bounded, and clears on silence.
Use held notes and carried panes to verify independent clocks, toggling and resizing agree with fresh rendering.
Verify texture changes leave the material geometry allocation intact.
Keep default golden frames unchanged and inspect any unexpected difference before accepting it.
Extend persistence coverage to non-default values in both stages, partial old-key loading and normalization.
Update settings reachability coverage for None and each enabled stage.
Run fmt, focused scene/UI/render tests and relevant lint checks.
Regenerate Metal assets on the runner and import them into the implementation commit.
Open a draft PR, then build both plugin and offline release binaries for handover.

## Design review

An independent review approved the fixed two-stage design and separate GPU parameters.
Its lifecycle clarification is incorporated above:
retain geometry only while active, release on None/zero, and compare reactivation and resize with fresh panes.
No extra abstractions or configurable ordering were recommended.
