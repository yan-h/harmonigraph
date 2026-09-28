# Independent lattice texture and material

## Intent

The lattice separates Clouds, Contours and Interference patterns from the Watercolor and Mosaic displacement materials,
so a cloudy glow can feed either material.
The fixed order is note illumination and breathing → texture → optional segment pickup → material → lattice composition.
Texture remains lit by notes;
silence stays dark.
This is not a general layer stack or a new ambient background.

## Settings and controls

Keep `ViewConfig::atmosphere` as the owner of lattice glow effects.
Replace its combined enable/material controls with two independently selectable stages:

- Texture: None, Clouds, Contours, Interference; depth, size and speed.
- Material: None, Watercolor, Mosaic, Stars; amount and drift direction.
Watercolor and Mosaic use Drift speed;
Stars uses its depth-dependent Star speed range.
- Watercolor: glob size, edge feathering, shape warp, refraction, random brightness and fine layer mix.
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
Segment pickup adds one half-resolution node-quad draw into the existing source target when enabled;
it allocates no extra texture.
Compute texture and material drift independently from the existing decorative clock.
Keep texture uniforms distinct from material uniforms so neither stage borrows the other's depth, scale or clock.
All materials use the same resolved note illumination.
The former per-node source roughness and its dedicated splat pipeline are removed.
Materials drift at constant screen direction using the spectrogram’s motion calculation;
texture drift remains independent.
All texture attenuation and material sampling preserve premultiplied RGBA and the existing brightness ceiling.
No display modulation feeds back into ink history.
Watercolor’s Random brightness uses the shared signed glob field to vary linear RGB after refraction,
leaving alpha unchanged.
Equal brightening and dimming headroom preserves the expected average color and premultiplied ceiling;
saturated highlights vary less.
The default is 0%,
including when older saved settings omit the field.

While the material is active, its geometry is keyed by material kind, quantized tile resolution and the active style’s geometry controls:
Watercolor feathering/warp, or Mosaic variety.
Texture settings, amount, refraction, random brightness, fine layer mix, drift, illumination, camera and the inactive style’s controls do not invalidate geometry.
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

## Independent segment pickup

The material samples pigment behind each octave segment:
dark for unlit segments and the segment’s pitch color for lit segments.
Each slot’s actual activation interpolates the two contributions during attack and release.
This source is independent of the actual ring, mark and label shadows:
changing pickup never replaces or suppresses those shadows,
and changing an actual shadow’s width, darkness or kernel does not change pickup.

The material controls expose four persisted fields:

- Dark pickup (`material_shadow_pickup`): 0–100%, default 0% (off).
- Color pickup (`material_color_pickup`): 0–100%, default 0% (off).
- Pickup width (`material_shadow_width`): full band width, 0–800% of the node radius, default 150%.
- Pickup softness (`material_shadow_softness`): feather distance around each finite arc, 0–800% of the node radius, default 200%.

The band is centered on the configured ring rim and follows each note’s light envelope,
including release, independently of decorative breathing.
Width zero disables both contributions.
Material None, zero material amount and disabled glow also bypass them.
Reset material restores all four controls.
A nonzero dark pickup saved from the earlier circular-band draft now fades in lit sectors;
appearances without pickup retain zero for both strengths.

One half-resolution quad per active node blends pigment into the resolved light before the material pass.
The analytic band needs only node instances and uniforms,
with no shadow atlas or caster dependency and no new texture allocation.
Its own quad covers the full width, feather and antialiasing margin,
including when its center is outside the viewport.
The shader reuses the visible ring’s sector mapping, packed activation and pitch-color function,
including unequal extra octaves and the per-node seam.
Each pixel measures Euclidean distance to the finite segment arc,
including the arc’s endpoints.
Width expands on all sides and softness feathers that distance,
so the ends are rounded instead of extending as angular wedges;
at large softness values neighboring pigments intentionally mingle.
Normalized distance-field weights keep that mixture bounded even at the center.

RGB blending multiplies the incoming pigment by destination alpha,
then attenuates the old RGB by pigment opacity.
The alpha write mask preserves coverage,
so colored pigment cannot create light where the source is empty or violate premultiplication.
Watercolor or Mosaic samples this pigment along with the source light.
Bloom brightens the colored source pigment using its strength and luminance soft knee,
with saturation at white to retain valid premultiplication.
This is the user-selected brightness coupling,
not a sample of the final blurred halo.
That halo is downstream of the material and sampling it here would introduce feedback.
Dark pickup remains independent of Bloom.
The normal scene shadow and bloom masks then apply unchanged.

Pickup controls do not belong in the geometry-tile cache key:
the live source draw reads current uniforms and instances each frame.
The first draft’s resting-cross shadow limitation (#1253) is removed by retaining the normal scene shadow pass in full.

GPU tests verify unchanged actual-shadow masks when pickup is adjusted,
unchanged pickup when actual shadows are adjusted or disabled,
per-slot pitch and activation at the source,
rounded pickup beyond both arc ends,
Bloom brightening only the colored source,
RGB changes without added alpha,
material/glow bypasses,
and wide/soft pickup reaching beyond the old node quad.

## Shared Stars material

Stars reads the same resolved note illumination and optional pigment pickup as the displacement materials.
Its source adapter samples one premultiplied color at each star center,
keeps that hue,
and spends brightness variation as coverage.
The shared renderer composites five depths over transparent black;
the final lattice material scales its bounded coverage to the existing glow strength/accumulation ceiling,
then blends with the original source by Material amount.
Scaling all premultiplied channels retains star contrast;
a hard cap flattened the bright center of a recorded note cluster.
Silence clears the output without running the star passes or discarding their allocations.

`StarSettings` owns defaults and normalization for both panes,
with independent values in each atmosphere's `stars` field and one shared controls widget.
`harmonigraph-render::stars` owns the layout,
clock reduction,
profile sizes,
halo allocation,
uniform transport and bake/halo/far/near pass sequence.
`shaders/stars.wgsl` owns the star geometry and premultiplied composition.
The spectrogram supplies its palette/color-memory adapter;
`lattice_stars` supplies the colored-light adapter and pane-specific resource bindings.
Color history remains spectrogram-owned.

The raw lattice light remains half resolution.
Stars has its own full-scene-resolution output so High and Uniform keep sharp foreground cores;
Medium keeps the same reduced foreground policy as the spectrogram.
`GlowTarget::binding` selects the finished material image for background composition and every node/label light reader,
so normal shadows and bloom see the same picture.
Other materials retain their existing half-resolution output.

The lattice star allocation key contains actual output,
atlas,
halo,
far and near image shapes.
Time,
color,
amount,
jitter and other appearance changes reuse images while their required shapes stay unchanged.
Star targets belong to their source texture owner,
so changing or dropping that source also drops its bindings;
material bypass releases the source and star targets together.

### Saved-state and reference-image consequences

The spectrogram's former flat `star_*` keys move under `stars`.
Old custom star tuning therefore resets to the current shared defaults;
other spectral controls,
material selection,
camera and lattice settings survive.
There is no migration or alias.
Lattice material defaults remain None.

The shared shader extraction changes one color channel by 1/255 at one pixel in each of two export references:
`spectrogram-short-pane` green 235 to 234 at (12,108),
and `spectrogram-spectral-shadows-mixed` red 179 to 178 at (173,71).
Their RGB mean differences are 0.00001017/255 and 0.00000521/255 respectively.
Restoring the old paint ordering did not remove the changes;
we retain the simpler shared implementation rather than a separate numerical path.
The dedicated High and Medium Stars references and all existing lattice references remain unchanged.

### Performance spot check

The existing `atmosphere_costs_by_polyphony` probe on an Apple M1 Pro,
with six MIDI notes (110 lit lattice nodes),
a 768×768 target and 60 measured frames,
reported total prepare/composite GPU medians of 11.13 ms for High,
8.80 ms for Medium and 12.90 ms for Uniform.
Watercolor and Mosaic measured 3.66 ms and 2.79 ms in the same probe;
these are total scene timings,
not isolated material-pass costs.
Stars is therefore a more expensive material,
with Medium providing the existing shared quality/performance tradeoff.
The callback CPU medians remained 0.05 ms across the materials.
