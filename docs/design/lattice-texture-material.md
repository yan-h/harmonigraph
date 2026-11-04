# Independent lattice texture and material

## Intent

The lattice separates the Clouds pattern from the Watercolor and Scales materials,
so a cloudy glow can feed either material.
The fixed order is note illumination and breathing → texture → material → lattice composition.
Texture remains lit by notes;
silence stays dark.
This is not a general layer stack or a new ambient background.

## Settings and controls

Keep `ViewConfig::atmosphere` as the owner of lattice glow effects.
Replace its combined enable/material controls with two independently selectable stages:

- Texture: Clouds; depth (Pattern contrast, where zero turns it off), size and speed.
- Material: None, Watercolor, Stars, Scales; amount and drift direction.
Watercolor and Scales use Drift speed;
Stars uses its per-layer Star speed.
- Watercolor: patch size, edge feathering, shape warp, refraction, random brightness and fine layer mix.
- Scales: cell size, size variation, edge softness, irregularity and scale shape.
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
the texture size range is 1/32× to 4×, on an eased (logarithmic) bar.
Reset texture and material independently;
breathing has its own reset beside the glow controls.
Hide stage-specific controls when None is selected;
dim effect controls when glow reach or gain is zero.

## Rendering

Preserve the existing glow-statistics and resolve passes.
The resolve applies only the chosen texture to combined note light.
The material pass consumes that resolved texture using the existing optional source target and geometry tiles.
Accumulate texture and material drift on the pane's shared travel clock,
with a separate phase for each stage.
Keep texture uniforms distinct from material uniforms so neither stage borrows the other's depth, scale or phase.
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

While Watercolor is active, its geometry is keyed by quantized tile resolution and its geometry controls:
feathering and warp.
Scales and Stars bake no tile.
Texture settings, amount, refraction, random brightness, fine layer mix, drift, illumination, camera and the inactive style’s controls do not invalidate geometry.
The active material source target has a lifetime bounded by the glow allocation;
resizing the glow replaces that source as before.
None or zero material amount releases both source and geometry resources and skips the pass, as before.
Reactivation recreates them; changes between positive amounts retain geometry.
Zero texture depth bypasses modulation without disabling the material.

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

## Shared Stars material

Stars reads the same resolved note illumination as the displacement materials.
Its source adapter samples one premultiplied color at each star center,
keeps that hue,
and puts the star's level in its color, as a palette position does:
a star is whole however dim, and Brightness variation spreads the levels (#1287).
The shared renderer composites the drawn layers, up to five by Star layers, over transparent black;
the final lattice material blends that with the original source by Material amount.
Silence clears the output without running the star passes or discarding their allocations.

`StarSettings` owns defaults and normalization for both panes,
with independent values in each atmosphere's `stars` field and one shared controls widget.
`harmonigraph-render::stars` owns the layout,
clock reduction,
star image size,
uniform transport and the bake and star image passes.
`shaders/stars.wgsl` owns the star geometry and premultiplied composition.
The spectrogram supplies its palette/color-memory adapter;
`lattice_stars` supplies the colored-light adapter and pane-specific resource bindings.
Color history remains spectrogram-owned.

The raw lattice light remains quarter resolution.
Stars draws every depth into one image at **Stars resolution** of the full scene,
the same policy as the spectrogram.
`GlowTarget::binding` selects the finished material image for background composition and every node/label light reader,
so normal shadows and bloom see the same picture.
Other materials retain the quarter-resolution output.

The lattice star allocation key contains actual output,
atlas and star image shapes.
Time,
color,
amount,
jitter and other appearance changes reuse images while their required shapes stay unchanged.
Star targets belong to their source texture owner,
so changing or dropping that source also drops its bindings;
material bypass releases the source and star targets together.
The motion is not theirs:
the stars' travel and life clock, the displacement materials' drift and the glow's pattern and breathing are built up per pane (`PaneBuffers::material_travel`),
so a resize, a bypass or a change of material goes on from where they were.

### Saved-state and reference-image consequences

The spectrogram's former flat `star_*` keys move under `stars`.
Old custom star tuning therefore resets to the current shared defaults;
other spectral controls,
material selection,
camera and lattice settings survive.
There is no migration or alias.
Lattice material defaults were None then; they are Watercolor since #1475.

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
with Medium providing the shared quality/performance tradeoff at the time;
#1442 replaced the profiles with Stars resolution.
The callback CPU medians remained 0.05 ms across the materials.
