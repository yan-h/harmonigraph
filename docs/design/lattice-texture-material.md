# Independent lattice texture and material

## Intent

The lattice separates Clouds, Contours and Interference patterns from the Watercolor and Mosaic displacement materials,
so a cloudy glow can feed either material.
The fixed order is note illumination and breathing → texture → material → existing lattice composition.
Texture remains lit by notes;
silence stays dark.
This is not a general layer stack or a new ambient background.

## Settings and controls

Keep `ViewConfig::atmosphere` as the owner of lattice glow effects.
Replace its combined enable/material controls with two independently selectable stages:

- Texture: None, Clouds, Contours, Interference; depth, size and speed.
- Material: None, Watercolor, Mosaic; amount, drift speed and direction.
- Watercolor: glob size, edge feathering, shape warp, refraction and fine layer mix.
- Mosaic: cell size, size variation and signed refraction.
- Breathing: depth and speed in the Background glow controls, independent of either stage.

`AtmosphereSettings` owns texture, material selection/amount/motion and breathing.
Its nested `material_settings` and the spectrogram’s `material_settings` each hold an independent `MaterialSettings`.
This shared type owns geometry defaults and normalization;
both views use the same geometry and motion widgets, ranges and meanings.
Size is relative to pane height in both views.
No color memory is added to the lattice.
Two small enums express the valid choices;
None bypasses a stage without resetting its numeric settings.
The `material_style` key deliberately replaces the old `material` key:
serde ignores the retired key without any compatibility shim or dropped-variant parse failure.
The existing container-level serde defaults and normalization remain the persistence boundary.
Keep texture defaults numerically equal to today's Clouds settings and preserve breathing defaults.
Material defaults to None, with amount 1 ready when selected.
Geometry and motion defaults match the spectrogram’s captured defaults.
The material size range is 1/16× to 2×;
the texture size range remains 1/4× to 4×.
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
All materials use the same smooth note illumination.
The former per-node source roughness and its dedicated splat pipeline are removed.
Materials drift at constant screen direction using the spectrogram’s motion calculation;
texture drift remains independent.
All texture attenuation and material sampling preserve premultiplied RGBA and the existing brightness ceiling.
No display modulation feeds back into ink history.

While the material is active, its geometry is keyed by material kind, quantized tile resolution and the active style’s geometry controls:
Watercolor feathering/warp, or Mosaic variety.
Texture settings, amount, refraction, fine layer mix, drift, illumination, camera and the inactive style’s controls do not invalidate geometry.
The active material source target has a lifetime bounded by the glow allocation;
resizing the glow replaces that source as before.
None or zero material amount releases both source and geometry resources and skips the pass, as before.
Reactivation recreates them; changes between positive amounts retain geometry.
None or zero texture depth bypasses modulation without disabling the material.

## Saved-state consequence

Old `enabled`, `material` and `nebula_*` keys are retired and ignored.
Old texture/material selections and tuning reset to the new defaults;
existing breathing values survive.
The subsequent shared-controls change also retires `source_roughness` and `material_scale`.
The spectrogram’s former flat `scale_*` and `wash_*` fields move under `material_settings`,
so saved custom geometry values reset to shared defaults;
no migration is provided.
Existing lattice texture settings, material selection/amount/speed, and spectrogram style/motion/color memory survive.
An old disabled atmosphere can therefore load with the default Clouds and breathing active.
Camera, layout and unrelated appearance settings survive.
No migration, alias or compatibility struct is introduced.
New saved texture and material controls round-trip and normalize for both editor and offline rendering.

## Verification

Adapt existing per-effect render tests to the new ownership without weakening their fixtures.
Add a combined-stage render test covering all three textures with both materials:
combined output differs visibly from either stage alone, remains premultiplied and bounded, and clears on silence.
Use held notes and carried panes to verify independent clocks, toggling and resizing agree with fresh rendering.
Verify texture and live sampling changes leave the material geometry allocation intact,
and each active geometry control rebakes it.
Compare carried panes with fresh panes after each edit.
Verify independent lattice/spectrogram material values survive a save/load.
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
