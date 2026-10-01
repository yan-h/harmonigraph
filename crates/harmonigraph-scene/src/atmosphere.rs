//! Saved controls for the lattice atmosphere prototype and for the
//! spectrogram's cloud texture, which is no longer one.

use harmonigraph_core::LatticePos;

/// Which texture the atmosphere layer draws over the spectrogram.
///
/// Distinct constructions: [`CloudStyle::Mosaic`] is a pile
/// of soft domes joined by a soft union, and [`CloudStyle::Watercolor`] is a
/// field of overlapping globs. Both displace the same scalar picture without
/// altering its levels, then apply the shared Contours and palette controls.
/// [`CloudStyle::VelvetScales`] instead averages each overlapping scallop's
/// sampled light, with no raw source overlay at full material depth.
///
/// **These name what Yan sees on the page, and the code under each keeps the
/// name of its own CONSTRUCTION** — `scale_*` and `dome_*` for the mosaic's
/// dome geometry, `wash_*` for the watercolour's laid-over globs. That split is
/// not new and is not an oversight: the variant was `Water` over `scale_*`
/// fields before it was `Mosaic` over them. A menu entry names a look and may
/// be renamed whenever the look is better described; a field names the thing
/// the arithmetic builds.
///
/// Renaming either is allowed and neither is free, but do not read that as the
/// two costing the same. They are not symmetric, and the cheap-looking one is
/// the expensive one: renaming a FIELD drops a key, which the container-level
/// `serde(default)` absorbs, so that one field resets and nothing else moves.
/// Renaming a VARIANT fails the parse and takes the whole persist with it --
/// layout and camera included -- because `UI_PERSIST_VERSION` is a floor and a
/// floor cannot guard a variant. #913 renamed these two and said so in its PR
/// body with the measured refusal, which is the bar for doing it again.
///
/// [`CloudStyle::Stars`] is the odd one out: it does not displace the picture's
/// levels at all but REPLACES the picture — pinpoint stars in depth, each one
/// palette colour picked by the sound under it — so Contours do not reach it and
/// every `star_` setting belongs to it alone.
#[derive(Clone, Copy, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub enum CloudStyle {
    Mosaic,
    Watercolor,
    Stars,
    VelvetScales,
}

/// Stars rendering policy. P3 is the High preset; `Uniform`
/// keeps native cores and uses the adjustable halo resolution for every depth.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub enum StarHaloProfile {
    Uniform,
    /// Shorter-glow far three composited at 75%; near halos at 100% and 60%.
    P3,
    /// Back three at 50%, foreground at 75%, near halos at 75% and 45%.
    #[default]
    Medium,
    /// Back three at one third, foreground at half, near halos at 50% and 30%.
    Low,
}

/// The band the cloud size dials run over — [`MaterialSettings::scale_size`],
/// [`MaterialSettings::wash_size`] and [`MaterialSettings::velvet_size`], which
/// mean the same thing about different textures and so are worth one pair of
/// numbers rather than three.
///
/// **Exported because the dial and the load-door clamp have to be the SAME
/// range.** A bar that stops at one number over a clamp that stops at another
/// is the silent break the persistence rule is about: the blob keeps a size the
/// pane cannot show, or the pane shows one the blob will not keep.
///
/// It is not symmetric about the fresh 1x, and the asymmetry is the point.
/// Yan's report on the 1/4..4 range it replaces was both ends at once — *"0.25x
/// is still too big"* and *"4x for both settings is way too big for me to ever
/// use"* — so the useful band lies below the default, not around it. The top is
/// where one glob is a tenth of the pane's height, which is already a texture
/// with about ten things in it.
///
/// **The floor is measured rather than chosen.** A texture that is getting
/// finer raises the mean step between adjacent pixels until its own detail
/// nears one pixel, and then that number stops moving: past there the dial is
/// buying noise rather than a smaller texture. Over a 700-point pane — a
/// 1080p export's — the mosaic's mean adjacent-pixel step runs 0.034 at 1x,
/// 0.047 at 1/4, 0.058 at 1/8 and 0.062 at 1/16, and then FALLS to 0.061 at
/// 0.05. 1/16 is where it turns over, which is about 1.6 points to a cell, so
/// that is where the bar stops.
///
/// Scales' `Cell size` (`velvet_size`) runs over the same band, but the floor
/// was not measured on it, and its 1x is a coarser cell — about 6% of the
/// pane's height — so on that pane the floor stops it at about 2.6 points to a
/// cell rather than at the mosaic's turnover.
///
/// It is one floor over three textures and a pane whose height varies about
/// threefold, so it cannot be exactly right everywhere: the watercolour's
/// globs are averaging toward a flat film by 1/8 already, and on a short
/// editor pane the last of the travel aliases where on a tall portrait render
/// it still has room. A bar that goes slightly past useful on the smallest
/// pane is the right way round — the picture says so immediately, and the
/// alternative is a bar that cannot reach what the export needs.
pub const CLOUD_SIZE_MIN: f32 = 0.0625;
/// See [`CLOUD_SIZE_MIN`].
pub const CLOUD_SIZE_MAX: f32 = 2.0;

/// The top of [`SpectralAtmosphere::blur_time_step`], in slabs per texel.
///
/// One texel per two slabs is where the resample has already halved the data
/// the picture was drawn from, and the saving it is bought with has flattened:
/// on the 600 s pane this was measured for, the light field's time axis falls
/// from 1416 texels to 587 at a step of one and to 294 at two, so the second
/// half of the bar removes a fifth of the texels where the first removed three
/// fifths. Past it the dial would be spending picture for very little.
///
/// Snapped to halves, so the bar offers OFF and four resolutions to compare
/// rather than a continuum — what
/// was being judged was whether a resample along time reads at all, not where
/// between two of them it starts to. It does: step one is the default.
pub const BLUR_TIME_STEP_MAX: f32 = 2.0;

/// Bounds shared by the [`SpectralAtmosphere::pitch_softness`] control and sanitizer.
pub const PITCH_SOFTNESS_MIN: f32 = 0.0;
/// See [`PITCH_SOFTNESS_MIN`].
pub const PITCH_SOFTNESS_MAX: f32 = 300.0;

/// Bounds shared by the [`SpectralAtmosphere::time_softness`] control and sanitizer.
pub const TIME_SOFTNESS_MIN: f32 = 0.0;
/// See [`TIME_SOFTNESS_MIN`].
pub const TIME_SOFTNESS_MAX: f32 = 2000.0;

/// Bounds shared by the [`SpectralAtmosphere::contours`] control and sanitizer.
pub const CONTOURS_MIN: f32 = 2.0;
/// See [`CONTOURS_MIN`].
pub const CONTOURS_MAX: f32 = 16.0;

/// Bounds shared by the [`SpectralAtmosphere::contour_softness`] control and sanitizer.
pub const CONTOUR_SOFTNESS_MIN: f32 = 0.01;
/// See [`CONTOUR_SOFTNESS_MIN`].
pub const CONTOUR_SOFTNESS_MAX: f32 = 0.5;

/// Bounds shared by the [`SpectralAtmosphere::cloud_speed`] control and sanitizer.
pub const CLOUD_SPEED_MIN: f32 = 0.0;
/// See [`CLOUD_SPEED_MIN`].
pub const CLOUD_SPEED_MAX: f32 = 20.0;
const MATERIAL_SPEED_DEFAULT: f32 = 4.242_738_7;
const MATERIAL_DIRECTION_DEFAULT: f32 = 174.0;

/// Bounds shared by the [`SpectralAtmosphere::cloud_direction`] control and sanitizer.
pub const CLOUD_DIRECTION_MIN: f32 = 0.0;
/// See [`CLOUD_DIRECTION_MIN`].
pub const CLOUD_DIRECTION_MAX: f32 = 360.0;

/// Bounds shared by the [`MaterialSettings::scale_refract`] control and sanitizer.
pub const SCALE_REFRACT_MIN: f32 = -1.0;
/// See [`SCALE_REFRACT_MIN`].
pub const SCALE_REFRACT_MAX: f32 = 1.0;

/// Bounds shared by the [`StarSettings::star_density`] control and
/// sanitizer, as a multiplier on stars per area.
pub const STAR_DENSITY_MIN: f32 = 0.5;
/// See [`STAR_DENSITY_MIN`].
pub const STAR_DENSITY_MAX: f32 = 10.0;
/// The top of [`StarSettings::star_fringe`]: past half, the fringes of a
/// dense slice add up to a flat wash of its average colour.
pub const STAR_FRINGE_MAX: f32 = 0.5;
/// Bounds for the per-axis resolution of the Stars halo images.
pub const STAR_HALO_RESOLUTION_MIN: f32 = 0.25;
pub const STAR_HALO_RESOLUTION_MAX: f32 = 1.0;

/// The top of [`StarSettings::star_defocus`].
pub const STAR_DEFOCUS_MAX: f32 = 1.5;
/// Bounds shared by the two ends of the `Star spacing` control
/// ([`StarSettings::star_spacing_min`], [`StarSettings::star_spacing_max`])
/// and their sanitizer, in star pixels at density 2.
pub const STAR_SPACING_MIN: f32 = 0.5;
/// See [`STAR_SPACING_MIN`].
pub const STAR_SPACING_MAX: f32 = 64.0;
/// Bounds shared by the two ends of the `Star size` control
/// ([`StarSettings::star_diameter_min`], [`StarSettings::star_diameter_max`])
/// and their sanitizer, in star pixels.
pub const STAR_DIAMETER_MIN: f32 = 0.25;
/// See [`STAR_DIAMETER_MIN`].
pub const STAR_DIAMETER_MAX: f32 = 32.0;
/// How many times its stored `Star size` and `Star spacing` the lattice draws
/// and shows them. The glow has no fine detail for stars to pick up, so its
/// stars run bigger. Applied where the lattice draws and in its bars, never
/// stored, so both panes share one [`StarSettings::default`] and a key a blob
/// lacks takes the right fresh value in either.
pub const LATTICE_STAR_SIZE_SCALE: f32 = 5.0;
/// Bounds shared by the two depth curves, [`StarSettings::star_spacing_curve`]
/// and [`StarSettings::star_diameter_curve`], and their sanitizer.
pub const STAR_DEPTH_CURVE_MIN: f32 = 0.5;
/// See [`STAR_DEPTH_CURVE_MIN`].
pub const STAR_DEPTH_CURVE_MAX: f32 = 4.0;
/// Bounds shared by the two ends of the `Star speed` control
/// ([`StarSettings::star_speed_min`], [`StarSettings::star_speed_max`])
/// and their sanitizer, as a share of the prototype's pace: at the top a depth
/// crosses the pane's height in about nine seconds. The top was once 5, and
/// everything past 1 was too fast to use while it crowded the useful range
/// into a fifth of the track.
pub const STAR_SPEED_MIN: f32 = 0.0;
/// See [`STAR_SPEED_MIN`].
pub const STAR_SPEED_MAX: f32 = 1.0;
/// Bounds shared by the [`StarSettings::star_speed_curve`] control and
/// sanitizer.
pub const STAR_SPEED_CURVE_MIN: f32 = 0.25;
/// See [`STAR_SPEED_CURVE_MIN`].
pub const STAR_SPEED_CURVE_MAX: f32 = 4.0;
/// Longest color-memory time constant in seconds.
pub const COLOR_MEMORY_MAX: f32 = 5.0;

/// Bounds shared by the [`StarSettings::star_lifetime`] control and
/// sanitizer, in seconds.
pub const STAR_LIFETIME_MIN: f32 = 0.5;
/// See [`STAR_LIFETIME_MIN`].
pub const STAR_LIFETIME_MAX: f32 = 20.0;

/// Bounds shared by the lattice pattern size control and sanitizer.
pub const NEBULA_SCALE_MIN: f32 = 0.031_25;
/// See [`NEBULA_SCALE_MIN`].
pub const NEBULA_SCALE_MAX: f32 = 4.0;

/// Bounds shared by the lattice texture/material speed controls and sanitizer.
pub const NEBULA_SPEED_MIN: f32 = 0.0;
/// See [`NEBULA_SPEED_MIN`].
pub const NEBULA_SPEED_MAX: f32 = 20.0;

/// Bounds shared by the [`AtmosphereSettings::breath_speed`] control and sanitizer.
pub const BREATH_SPEED_MIN: f32 = 0.0;
/// See [`BREATH_SPEED_MIN`].
pub const BREATH_SPEED_MAX: f32 = 4.0;

/// Geometry and sampling controls shared by lattice and spectrogram materials.
/// Each view owns its own copy; changing one view never changes the other.
#[derive(Clone, Copy, Debug, PartialEq, serde::Serialize, serde::Deserialize)]
#[serde(default)]
pub struct MaterialSettings {
    /// Velvet cell size; 1× is 24/405 of the pane height (the S1 prototype).
    pub velvet_size: f32,
    /// Radius variation; 50% reproduces S1's 0.65..1.10 cell radii.
    pub velvet_variety: f32,
    /// Width of the smooth transition around each scale's edge.
    pub velvet_edge: f32,
    /// Cell-center jitter and smooth coordinate warp; S1 uses 80%.
    pub velvet_irregularity: f32,
    /// Round bodies at zero, tapered overlapping scallops at one.
    pub velvet_shape: f32,
    /// Size of one scale, as a multiplier on that size: how many of them cross
    /// a cloud moves the other way, because the count is divided by this.
    /// Runs over [`CLOUD_SIZE_MIN`]..=[`CLOUD_SIZE_MAX`].
    pub scale_size: f32,
    /// How much the scales differ in size from each other. 0 is one radius for
    /// every glob in the field, which is the most regular texture there is; 1
    /// draws each from the whole band the layer's coverage proof allows.
    pub scale_variety: f32,
    /// How far a scale carries the light behind it, and which way — the
    /// refraction, and the whole reason the layer reads as a lens rather than
    /// as something painted over the picture. 0 leaves the light where it is.
    ///
    /// ABOVE 0 the light is read where the scale's own face POINTS, in scale
    /// widths, so the picture bends smoothly through the cloud. BELOW 0 it is
    /// pulled toward the scale's CENTRE, and at -1 it is read there — one value
    /// across the whole scale, so the picture comes apart into flat quantized
    /// patches. That second half used to be its own `scale_facet` dial, a blend
    /// between the two readings; the pair spanned a plane and the looks worth
    /// having lie on this line through it, which is also exactly what the
    /// watercolour's `wash_refract` has always meant by the word.
    pub scale_refract: f32,
    /// How big one glob is, as a multiplier on that size: how many of them
    /// cross the cloud frame moves the other way, because the count is divided
    /// by this. Larger is bigger, like `scale_size`, and over the same
    /// [`CLOUD_SIZE_MIN`]..=[`CLOUD_SIZE_MAX`] band.
    pub wash_size: f32,
    /// One dial over everything that dissolves a glob's rim: how far it feathers
    /// into what lies beneath and how far it bleeds into what is about to cover it.
    pub wash_fuzz: f32,
    /// How far the shared domain warp carries glob space off the grid: 0 is
    /// bubbles, the top of the dial is shearing lobes.
    pub wash_lobe: f32,
    /// How far each glob's tone is pulled to the light at its own centre. 0
    /// leaves the picture exactly where it is.
    pub wash_refract: f32,
    /// How opaque the finer octave's wash is over the coarse one. 0 draws the
    /// coarse octave alone and skips the finer one's work.
    pub wash_layers: f32,
    /// Balanced per-glob brightness variation after coloring, from 0 to 1.
    /// Highlight headroom limits both signs equally, preserving expected RGB.
    pub wash_randomness: f32,
}
impl Default for MaterialSettings {
    fn default() -> Self {
        Self {
            velvet_size: 1.0,
            velvet_variety: 0.5,
            velvet_edge: 0.34,
            velvet_irregularity: 0.8,
            velvet_shape: 1.0,
            scale_size: 0.153_937_07,
            scale_variety: 0.5,
            scale_refract: -1.0,
            wash_size: 0.181_260_21,
            wash_fuzz: 1.0,
            wash_lobe: 1.0,
            wash_refract: 0.950_153_47,
            wash_layers: 0.5,
            wash_randomness: 0.0,
        }
    }
}
impl MaterialSettings {
    /// Sampling offset in cloud units, opposite the visible screen direction.
    /// Keep f64 until upload; repeating materials reduce their period first.
    /// Velvet keeps the translation unwrapped because its warp is nonperiodic.
    pub fn drift(speed: f32, direction: f32, now: f64) -> [f64; 2] {
        let distance = now * f64::from(speed) * 0.047_169_905_660_283_02;
        let (sin, cos) = f64::from(direction).to_radians().sin_cos();
        [-distance * cos, 0.6 - distance * sin]
    }

    pub fn sanitized(mut self) -> Self {
        let fresh = Self::default();
        let clamp = |value: f32, fallback: f32, low, high| {
            if value.is_finite() {
                value.clamp(low, high)
            } else {
                fallback
            }
        };
        self.velvet_size =
            clamp(self.velvet_size, fresh.velvet_size, CLOUD_SIZE_MIN, CLOUD_SIZE_MAX);
        self.velvet_variety = clamp(self.velvet_variety, fresh.velvet_variety, 0.0, 1.0);
        self.velvet_edge = clamp(self.velvet_edge, fresh.velvet_edge, 0.01, 1.0);
        self.velvet_irregularity =
            clamp(self.velvet_irregularity, fresh.velvet_irregularity, 0.0, 1.0);
        self.velvet_shape = clamp(self.velvet_shape, fresh.velvet_shape, 0.0, 1.0);
        self.scale_size = clamp(self.scale_size, fresh.scale_size, CLOUD_SIZE_MIN, CLOUD_SIZE_MAX);
        self.scale_variety = clamp(self.scale_variety, fresh.scale_variety, 0.0, 1.0);
        self.scale_refract =
            clamp(self.scale_refract, fresh.scale_refract, SCALE_REFRACT_MIN, SCALE_REFRACT_MAX);
        self.wash_size = clamp(self.wash_size, fresh.wash_size, CLOUD_SIZE_MIN, CLOUD_SIZE_MAX);
        self.wash_fuzz = clamp(self.wash_fuzz, fresh.wash_fuzz, 0.0, 1.0);
        self.wash_lobe = clamp(self.wash_lobe, fresh.wash_lobe, 0.0, 1.0);
        self.wash_refract = clamp(self.wash_refract, fresh.wash_refract, 0.0, 1.0);
        self.wash_layers = clamp(self.wash_layers, fresh.wash_layers, 0.0, 1.0);
        self.wash_randomness = clamp(self.wash_randomness, fresh.wash_randomness, 0.0, 1.0);
        self
    }
}

/// Independent spectrogram diffusion, analyzer shading and note light.
///
/// There is no style here. Plain, Blur and Lava were three presets over three
/// effects that never depended on each other — the blur, the terraces and the
/// cloud — so each is a dial whose zero is OFF, and [`Self::effects`] is what
/// the renderer reads to pay for exactly the ones that are on. The measured
/// picture is all of them at zero.
#[derive(Clone, Copy, Debug, PartialEq, serde::Serialize, serde::Deserialize)]
#[serde(default)]
pub struct SpectralAtmosphere {
    pub pitch_softness: f32,
    pub time_softness: f32,
    pub spread: f32,
    /// How coarse the blurred light field's TIME axis may be, in slabs per
    /// source texel, 0 for off. A PERFORMANCE dial, and the one that spends
    /// time resolution.
    ///
    /// At a long Span the blur's time radius is a fraction of a pixel, so the
    /// renderer leaves that axis at the pane's FULL resolution — while the data
    /// under it is a few hundred slabs two or three pixels wide, and the field
    /// between slab centres is a straight line the shader redraws pixel by
    /// pixel. The field's whole cost is linear in its texels, so bounding that
    /// axis at one texel per this many slabs draws the same line out of fewer
    /// of them.
    ///
    /// **SLABS and not points or pixels.** The editor and an offline export lay
    /// out their own slabs, so "never finer than one slab" means the same thing
    /// to both, where a number of points would mean a different resolution on
    /// each.
    ///
    /// What it spends is a resample along time, about one slab of extra
    /// softening at a step of one. The pitch axis is never touched: at full
    /// zoom-out it already carries a couple of buckets to the pixel and is
    /// data-limited rather than pane-limited. Runs over
    /// 0..=[`BLUR_TIME_STEP_MAX`], snapped to halves.
    ///
    /// **Computed by hand, then looked for in motion and not seen.** The
    /// source's texels are fixed to the PANE while the slabs scroll under them,
    /// so the box each one integrates slides across the slab grid and the
    /// effective kernel breathes: `[1, 6, 1] / 8` where a texel is centred on a
    /// slab, `[1, 1] / 2` where it straddles two, once per slab scrolled (about
    /// 1 Hz at a 600 s Span). A maximal isolated transient one slab wide reads
    /// 0.75 of its encoded level and then 0.5, about a fifth of its DISPLAYED
    /// level once `density_decode` has run. A step of a half gives 0.875 to
    /// 0.75, and with the dial off the pane's own pixels already breathe about
    /// 0.90 to 0.79 — this deepens a pulse it did not create. The time Gaussian
    /// damps none of it: at full zoom-out it is sub-pixel (32 ms of
    /// `time_softness` is 0.08 px at a 600 s Span), so this box is all the
    /// smoothing the time axis gets.
    ///
    /// Yan judged a step of one live in the DAW at a 600 s Span on 2026-09-20,
    /// made it the default, and looked for the shimmer in the moving picture
    /// without finding one. That agrees with the arithmetic rather than
    /// contradicting it: the swing falls as `0.25 * (1 - h)` with the
    /// neighbouring slabs' level, so the worst case wants an isolated event
    /// about a second long with near-silence either side, which continuous
    /// playing does not contain.
    ///
    /// **If it ever does show, the fix is not the slab-space relayout** that
    /// stood here: the pulse needs the texel centres to LAND on slab centres,
    /// not the texture to be laid out in slab space. Give this axis a spacing
    /// of exactly one slab — covering a hair more than the pane, so the spacing
    /// is the slab width rather than the pane's over a `ceil` — and shift the
    /// origin each frame by the fractional scroll phase. The kernel is then
    /// always `[1, 6, 1] / 8` and the softening it costs stays. Downstream the
    /// open-coded `pt / cloud.size` light lookups become one helper over a new
    /// origin/extent uniform, a multiply-add per fragment, so this dial's
    /// saving survives. It wants an integer number of texels per slab, and the
    /// lock breaks wherever `retained_size` holds a stale size through a Span
    /// drag.
    pub blur_time_step: f32,
    /// How far the levels are gathered into terraces, 0 for none. What the
    /// `Lava` style used to switch on whole.
    pub contour_strength: f32,
    pub contours: f32,
    pub contour_softness: f32,
    /// Texture strength. Without memory, displaced levels mix before Contours
    /// and the palette. With memory, held RGB mixes in linear light.
    /// Zero disables the texture; zero refraction disables displacement alone.
    pub cloud_depth: f32,
    /// Linear-light color response times in seconds. Both zero bypass history.
    pub color_pickup: f32,
    pub color_release: f32,
    /// Drift speed, as a multiplier on a slow crossing like the lattice
    /// nebula's. The cloud FRAME it drifts in is fixed in the shader
    /// (`CLOUD_UNITS`): it used to be a dial, and it was a second copy of
    /// `scale_size`/`wash_size` for the texture's size and of this one for its
    /// travel.
    pub cloud_speed: f32,
    /// Constant visible texture drift direction in screen degrees: 0 points
    /// right, 90 down, 180 left and 270 up.
    pub cloud_direction: f32,
    pub material_settings: MaterialSettings,
    /// Which texture the layer draws; [`CloudStyle`] says what each is. Each
    /// reads its own settings and no other's: `scale_*` for `Mosaic`, `wash_*`
    /// for `Watercolor` and `velvet_*` for `VelvetScales`, all in
    /// [`Self::material_settings`], and [`Self::stars`] for `Stars`.
    pub cloud_style: CloudStyle,
    pub stars: StarSettings,
}

/// Shared star geometry and rendering controls. Each pane owns its own values.
#[derive(Clone, Copy, Debug, PartialEq, serde::Serialize, serde::Deserialize)]
#[serde(default)]
pub struct StarSettings {
    /// Stars per area at every depth, as a multiplier: the cells each depth's
    /// stars are hashed into shrink by its square root. Runs over
    /// [`STAR_DENSITY_MIN`]..=[`STAR_DENSITY_MAX`].
    ///
    /// Shared by [`CloudStyle::Stars`] and [`LatticeMaterial::Stars`].
    pub star_density: f32,
    /// How far the stars differ from each other in brightness, spent as a
    /// palette position in the spectrogram or a level of the sampled lattice
    /// hue: the steepness of the brightness rank and how far it spreads each
    /// star above and below the colour behind it, keeping their average. At 0
    /// every star is the colour behind it, lifted a little.
    pub star_randomness: f32,
    /// How far the stars' cores shrink below their depth's size, each by its
    /// own draw, independent of its brightness: every star at its depth's
    /// size at 0, down to 1/11 of it at 1. Only shrinks, so no star grows into
    /// the cap its spacing sets. Runs over 0..=1.
    pub star_size_variation: f32,
    /// Positional variation within each cell, from regular centers at 0 to
    /// the original 0.6-cell jitter width at 1. The optimized far-three response
    /// ends at `1.0 - 0.3 * star_jitter` cells; Uniform and the nearest two
    /// layers retain the wide 1.2-cell response.
    pub star_jitter: f32,
    /// The farthest depth's star spacing in star pixels at density 2. A depth
    /// `d` from 0 (far) to 1 (near) spaces its stars at `min · (max /
    /// min)^(d^curve)`, divided by the square root of half the density. Every
    /// cell holds a star, so how many a depth has follows its spacing alone.
    /// Runs over [`STAR_SPACING_MIN`]..=[`STAR_SPACING_MAX`] (the lattice
    /// draws it [`LATTICE_STAR_SIZE_SCALE`] times over), never above
    /// [`Self::star_spacing_max`].
    pub star_spacing_min: f32,
    /// The nearest depth's star spacing. See [`Self::star_spacing_min`].
    pub star_spacing_max: f32,
    /// The exponent on depth in the spacing: 1 spreads it evenly over the
    /// depths, higher puts most depths in the fine dust. Runs over
    /// [`STAR_DEPTH_CURVE_MIN`]..=[`STAR_DEPTH_CURVE_MAX`].
    pub star_spacing_curve: f32,
    /// The farthest depth's star size: its core's diameter in star pixels,
    /// four of its sigmas. A depth `d` spreads it like the spacing, `min ·
    /// (max / min)^(d^curve)`, and the size is a function of that value
    /// alone, so one value on the control is one star size at every depth.
    /// A core never exceeds a third of its depth's spacing (a sigma of a
    /// third of a cell), which keeps the dust pinpoint and every star inside
    /// the cells a pixel reads. Runs over
    /// [`STAR_DIAMETER_MIN`]..=[`STAR_DIAMETER_MAX`] (the lattice draws it
    /// [`LATTICE_STAR_SIZE_SCALE`] times over), never above
    /// [`Self::star_diameter_max`].
    pub star_diameter_min: f32,
    /// The nearest depth's star size. See [`Self::star_diameter_min`].
    pub star_diameter_max: f32,
    /// The exponent on depth in the size, as [`Self::star_spacing_curve`].
    pub star_diameter_curve: f32,
    /// The farthest depth's drift speed: the slowest stars. A depth `d` from 0
    /// (far) to 1 (near) drifts at `min + (max - min) d^curve`, along the
    /// shared `Drift direction`; the stars never read `cloud_speed`, which is
    /// the other textures' pace. Runs over
    /// [`STAR_SPEED_MIN`]..=[`STAR_SPEED_MAX`], never above
    /// [`Self::star_speed_max`].
    pub star_speed_min: f32,
    /// The nearest depth's drift speed: the fastest stars. See
    /// [`Self::star_speed_min`].
    pub star_speed_max: f32,
    /// The exponent on depth in the parallax, between the two ends of
    /// [`Self::star_speed_min`]. 1 steps the speeds evenly. Runs over
    /// [`STAR_SPEED_CURVE_MIN`]..=[`STAR_SPEED_CURVE_MAX`].
    pub star_speed_curve: f32,
    /// How long one star lives, in seconds, before its cell draws a new one,
    /// alike at every depth. Each fades in and out over its life. Runs over
    /// [`STAR_LIFETIME_MIN`]..=[`STAR_LIFETIME_MAX`].
    pub star_lifetime: f32,
    /// A wider, fainter fringe of each star's own colour round its core, at
    /// every depth: its coverage at the centre, falling off over 2.5 sigmas.
    /// Runs to [`STAR_FRINGE_MAX`].
    pub star_fringe: f32,
    /// Coverage of the farthest three layers: 0 preserves their response,
    /// 0.5 squares remaining background leakage, and 1 raises it to the fourth
    /// power. Interpolates between those responses without adding stars.
    pub star_far_fill: f32,
    /// Halo image width and height relative to the pane's device pixels.
    /// Lower values soften the halo sampling without moving stars or changing
    /// their reach. Runs over [`STAR_HALO_RESOLUTION_MIN`]..=[`STAR_HALO_RESOLUTION_MAX`].
    pub star_halo_resolution: f32,
    /// Uniform uses `star_halo_resolution`; the quality presets fix their own
    /// resolutions, for the far three depths' one shared image and the
    /// nearest two's halos. Saves without a profile use Medium.
    pub star_halo_profile: StarHaloProfile,
    /// How much every star is widened, equally at every depth.
    /// Runs to [`STAR_DEFOCUS_MAX`].
    pub star_defocus: f32,
}
impl Default for StarSettings {
    fn default() -> Self {
        Self {
            // Yan's Stars controls captured from the DAW on 2026-09-26.
            star_density: 10.0,
            star_randomness: 0.080912866,
            // The old shared dial's size spread at its fresh value, anchored
            // at the top: the same smallest-to-largest ratio, now 0.47..1
            // where it was 0.69..1.45.
            star_size_variation: 0.310_684_4,
            star_jitter: 0.5,
            star_spacing_min: 2.315533,
            star_spacing_max: 14.752405,
            star_spacing_curve: 2.1178954,
            // Fitted to the cores the 2026-09-26 capture drew when size and
            // spacing were one control: the far four depths unchanged (the
            // far three at their spacing's cap), the nearest no longer
            // smaller than the one behind it (1.03 star px sigma, was 0.60).
            star_diameter_min: 1.37,
            star_diameter_max: 4.1,
            star_diameter_curve: 1.04,
            star_speed_min: 0.08931082,
            star_speed_max: 0.16860056,
            star_speed_curve: 3.179647,
            star_lifetime: 2.9719827,
            star_fringe: 0.5,
            star_far_fill: 0.0,
            star_halo_resolution: 0.5,
            star_halo_profile: StarHaloProfile::default(),
            star_defocus: 0.35391274,
        }
    }
}
impl StarSettings {
    /// These settings with every size and spacing `scale` times over: what
    /// the lattice draws, at [`LATTICE_STAR_SIZE_SCALE`].
    pub fn scaled(self, scale: f32) -> Self {
        Self {
            star_spacing_min: self.star_spacing_min * scale,
            star_spacing_max: self.star_spacing_max * scale,
            star_diameter_min: self.star_diameter_min * scale,
            star_diameter_max: self.star_diameter_max * scale,
            ..self
        }
    }
    pub fn sanitized(mut self) -> Self {
        let fresh = Self::default();
        let clamp = |value: f32, fallback: f32, low, high| {
            if value.is_finite() {
                value.clamp(low, high)
            } else {
                fallback
            }
        };
        self.star_density =
            clamp(self.star_density, fresh.star_density, STAR_DENSITY_MIN, STAR_DENSITY_MAX);
        self.star_randomness = clamp(self.star_randomness, fresh.star_randomness, 0.0, 1.0);
        self.star_size_variation =
            clamp(self.star_size_variation, fresh.star_size_variation, 0.0, 1.0);
        self.star_jitter = clamp(self.star_jitter, fresh.star_jitter, 0.0, 1.0);
        // Each is one control with two handles, so its ends cannot cross on
        // screen; a blob that holds them crossed is drawn, and kept, as the
        // one pair.
        let pair = |min: &mut f32,
                    max: &mut f32,
                    fresh: [f32; 2],
                    range: std::ops::RangeInclusive<f32>| {
            let (low, high) = range.into_inner();
            *min = clamp(*min, fresh[0], low, high);
            *max = clamp(*max, fresh[1], low, high);
            if *min > *max {
                std::mem::swap(min, max);
            }
        };
        pair(
            &mut self.star_spacing_min,
            &mut self.star_spacing_max,
            [fresh.star_spacing_min, fresh.star_spacing_max],
            STAR_SPACING_MIN..=STAR_SPACING_MAX,
        );
        pair(
            &mut self.star_diameter_min,
            &mut self.star_diameter_max,
            [fresh.star_diameter_min, fresh.star_diameter_max],
            STAR_DIAMETER_MIN..=STAR_DIAMETER_MAX,
        );
        self.star_spacing_curve = clamp(
            self.star_spacing_curve,
            fresh.star_spacing_curve,
            STAR_DEPTH_CURVE_MIN,
            STAR_DEPTH_CURVE_MAX,
        );
        self.star_diameter_curve = clamp(
            self.star_diameter_curve,
            fresh.star_diameter_curve,
            STAR_DEPTH_CURVE_MIN,
            STAR_DEPTH_CURVE_MAX,
        );
        self.star_speed_min =
            clamp(self.star_speed_min, fresh.star_speed_min, STAR_SPEED_MIN, STAR_SPEED_MAX);
        self.star_speed_max =
            clamp(self.star_speed_max, fresh.star_speed_max, STAR_SPEED_MIN, STAR_SPEED_MAX);
        // The same one control with two handles as `Star size`.
        if self.star_speed_min > self.star_speed_max {
            std::mem::swap(&mut self.star_speed_min, &mut self.star_speed_max);
        }
        self.star_speed_curve = clamp(
            self.star_speed_curve,
            fresh.star_speed_curve,
            STAR_SPEED_CURVE_MIN,
            STAR_SPEED_CURVE_MAX,
        );
        self.star_lifetime =
            clamp(self.star_lifetime, fresh.star_lifetime, STAR_LIFETIME_MIN, STAR_LIFETIME_MAX);
        self.star_fringe = clamp(self.star_fringe, fresh.star_fringe, 0.0, STAR_FRINGE_MAX);
        self.star_far_fill = clamp(self.star_far_fill, fresh.star_far_fill, 0.0, 1.0);
        self.star_halo_resolution = clamp(
            self.star_halo_resolution,
            fresh.star_halo_resolution,
            STAR_HALO_RESOLUTION_MIN,
            STAR_HALO_RESOLUTION_MAX,
        );
        self.star_defocus = clamp(self.star_defocus, fresh.star_defocus, 0.0, STAR_DEFOCUS_MAX);
        self
    }
}

/// Which of the three spectrogram effects a setting actually draws — what the
/// retired style enum used to say in one word, read off the dials instead.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct SpectralEffects {
    /// Either softness is above zero, so the picture is the blurred field.
    pub soft: bool,
    /// The levels are gathered into terraces.
    pub contours: bool,
    /// A cloud texture is drawn over the picture.
    pub cloud: bool,
}

impl SpectralEffects {
    /// Nothing is on: the measured heatmap, on the renderer's plain path.
    pub fn none(self) -> bool {
        !(self.soft || self.contours || self.cloud)
    }

    /// Whether the scalar light field has to be built. The blur IS that field,
    /// and the cloud reads it — at zero softness it is the measured picture
    /// carried through unblurred, which is what lets a cloud be drawn over a
    /// sharp spectrogram. The terraces alone read the level under the pixel and
    /// need none of it.
    pub fn light(self) -> bool {
        self.soft || self.cloud
    }
}

impl Default for SpectralAtmosphere {
    fn default() -> Self {
        Self {
            // The Stars look captured from the DAW on 2026-09-26: a sharp
            // field with no time blur or spread, and the Mosaic and Wash
            // controls below riding inert at their captured values.
            pitch_softness: 6.726_529_6,
            time_softness: 0.0,
            spread: 0.0,
            // One texel a slab: Yan judged it live at a 600 s Span (2026-09-20),
            // where it takes the pane from about 100 fps back to 144 and reads
            // the same. It binds only where the pane is finer than the data.
            blur_time_step: 1.0,
            // Full strength is what the `Lava` style drew, and that style was
            // the fresh one.
            contour_strength: 1.0,
            contours: 16.0,
            contour_softness: 0.492_202_6,
            cloud_depth: 1.0,
            color_pickup: 0.043_984_346,
            color_release: 0.711_714_74,
            cloud_speed: MATERIAL_SPEED_DEFAULT,
            cloud_direction: MATERIAL_DIRECTION_DEFAULT,
            cloud_style: CloudStyle::Stars,
            material_settings: MaterialSettings::default(),
            stars: StarSettings::default(),
        }
    }
}

impl SpectralAtmosphere {
    pub fn sanitized(mut self) -> Self {
        let fresh = Self::default();
        let clamp = |value: f32, fallback: f32, low, high| {
            if value.is_finite() {
                value.clamp(low, high)
            } else {
                fallback
            }
        };
        self.pitch_softness = clamp(
            self.pitch_softness,
            fresh.pitch_softness,
            PITCH_SOFTNESS_MIN,
            PITCH_SOFTNESS_MAX,
        );
        self.time_softness =
            clamp(self.time_softness, fresh.time_softness, TIME_SOFTNESS_MIN, TIME_SOFTNESS_MAX);
        self.spread = clamp(self.spread, fresh.spread, 0.0, 1.0);
        // Snapped to halves for the reason [`BLUR_TIME_STEP_MAX`] gives: five
        // resolutions to compare, not a continuum to hunt through.
        self.blur_time_step =
            (clamp(self.blur_time_step, fresh.blur_time_step, 0.0, BLUR_TIME_STEP_MAX) * 2.0)
                .round()
                / 2.0;
        self.contour_strength = clamp(self.contour_strength, fresh.contour_strength, 0.0, 1.0);
        self.contours = clamp(self.contours, fresh.contours, CONTOURS_MIN, CONTOURS_MAX).round();
        self.contour_softness = clamp(
            self.contour_softness,
            fresh.contour_softness,
            CONTOUR_SOFTNESS_MIN,
            CONTOUR_SOFTNESS_MAX,
        );
        self.cloud_depth = clamp(self.cloud_depth, fresh.cloud_depth, 0.0, 1.0);
        self.color_pickup = clamp(self.color_pickup, fresh.color_pickup, 0.0, COLOR_MEMORY_MAX);
        self.color_release = clamp(self.color_release, fresh.color_release, 0.0, COLOR_MEMORY_MAX);
        self.cloud_speed =
            clamp(self.cloud_speed, fresh.cloud_speed, CLOUD_SPEED_MIN, CLOUD_SPEED_MAX);
        self.cloud_direction = if self.cloud_direction.is_finite() {
            self.cloud_direction.rem_euclid(CLOUD_DIRECTION_MAX)
        } else {
            fresh.cloud_direction
        };
        self.material_settings = self.material_settings.sanitized();
        self.stars = self.stars.sanitized();
        self
    }

    /// Which effects these settings draw. Read off SANITIZED values — a NaN
    /// compares false to everything and would switch an effect off that the
    /// sanitizer is about to give its fresh value back to.
    pub fn effects(self) -> SpectralEffects {
        SpectralEffects {
            soft: self.pitch_softness > 0.0 || self.time_softness > 0.0,
            contours: self.contour_strength > 0.0,
            // A stationary sample still draws brightness variation or a temporal
            // color response; only a texture with none of these is bypassed.
            cloud: self.cloud_depth > 0.0
                && match self.cloud_style {
                    CloudStyle::Mosaic => {
                        self.material_settings.scale_refract != 0.0
                            || self.color_pickup > 0.0
                            || self.color_release > 0.0
                    }
                    CloudStyle::Watercolor => {
                        self.material_settings.wash_refract != 0.0
                            || self.material_settings.wash_randomness > 0.0
                            || self.color_pickup > 0.0
                            || self.color_release > 0.0
                    }
                    // Light rather than a displacement, so it has no dial at
                    // which it draws the ordinary picture: `Texture mix` alone
                    // switches it off.
                    CloudStyle::Stars | CloudStyle::VelvetScales => true,
                },
        }
    }
}

/// A pattern applied to combined note light before material displacement.
/// `None` reaches the shader as zero pattern depth.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub enum LatticeTexture {
    #[default]
    Clouds,
    None,
}

/// A material sampling the textured glow; None bypasses the material pass.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[repr(u32)]
pub enum LatticeMaterial {
    #[default]
    None = 0,
    Watercolor = 1,
    Mosaic = 2,
    Stars = 3,
    VelvetScales = 4,
}

/// Pickup width and edge softness, in node radii. Independent of ordinary shadows.
pub const SHADOW_PICKUP_SIZE_MAX: f32 = 8.0;

#[derive(Clone, Copy, Debug, PartialEq, serde::Serialize, serde::Deserialize)]
#[serde(default)]
pub struct AtmosphereSettings {
    pub texture: LatticeTexture,
    /// Replaces the retired combined `material` key, which serde ignores.
    pub material_style: LatticeMaterial,
    pub material_amount: f32,
    /// Dark pigment from unlit segments, sampled by the material.
    pub material_shadow_pickup: f32,
    /// Pitch-colored pigment from lit segments, sampled by the material.
    pub material_color_pickup: f32,
    /// Full width of the pigment band, in node radii.
    pub material_shadow_width: f32,
    /// Feather distance beyond each band edge, in node radii.
    pub material_shadow_softness: f32,
    pub material_settings: MaterialSettings,
    pub stars: StarSettings,
    pub material_speed: f32,
    pub material_direction: f32,
    pub texture_depth: f32,
    pub texture_scale: f32,
    pub texture_speed: f32,
    pub breath_amount: f32,
    pub breath_speed: f32,
}

impl Default for AtmosphereSettings {
    fn default() -> Self {
        Self {
            texture: LatticeTexture::Clouds,
            material_style: LatticeMaterial::None,
            material_amount: 1.0,
            material_shadow_pickup: 0.0,
            material_color_pickup: 0.0,
            material_shadow_width: 1.5,
            material_shadow_softness: 2.0,
            material_settings: MaterialSettings::default(),
            stars: StarSettings::default(),
            material_speed: MATERIAL_SPEED_DEFAULT,
            material_direction: MATERIAL_DIRECTION_DEFAULT,
            texture_depth: 0.134_627_85,
            texture_scale: 0.840_435_3,
            texture_speed: 6.077_757_4,
            breath_amount: 0.429_406_55,
            breath_speed: 1.813_457_6,
        }
    }
}

impl AtmosphereSettings {
    pub fn sanitized(mut self) -> Self {
        let fresh = Self::default();
        let clamp = |value: f32, fallback: f32, low, high| {
            if value.is_finite() {
                value.clamp(low, high)
            } else {
                fallback
            }
        };
        self.material_amount = clamp(self.material_amount, fresh.material_amount, 0.0, 1.0);
        self.material_color_pickup =
            clamp(self.material_color_pickup, fresh.material_color_pickup, 0.0, 1.0);
        self.material_shadow_pickup =
            clamp(self.material_shadow_pickup, fresh.material_shadow_pickup, 0.0, 1.0);
        self.material_shadow_width = clamp(
            self.material_shadow_width,
            fresh.material_shadow_width,
            0.0,
            SHADOW_PICKUP_SIZE_MAX,
        );
        self.material_shadow_softness = clamp(
            self.material_shadow_softness,
            fresh.material_shadow_softness,
            0.0,
            SHADOW_PICKUP_SIZE_MAX,
        );
        self.material_settings = self.material_settings.sanitized();
        self.stars = self.stars.sanitized();
        self.material_speed =
            clamp(self.material_speed, fresh.material_speed, CLOUD_SPEED_MIN, CLOUD_SPEED_MAX);
        self.material_direction = if self.material_direction.is_finite() {
            self.material_direction.rem_euclid(CLOUD_DIRECTION_MAX)
        } else {
            fresh.material_direction
        };
        self.texture_depth = clamp(self.texture_depth, fresh.texture_depth, 0.0, 1.0);
        self.texture_scale =
            clamp(self.texture_scale, fresh.texture_scale, NEBULA_SCALE_MIN, NEBULA_SCALE_MAX);
        self.texture_speed =
            clamp(self.texture_speed, fresh.texture_speed, NEBULA_SPEED_MIN, NEBULA_SPEED_MAX);
        self.breath_amount = clamp(self.breath_amount, fresh.breath_amount, 0.0, 1.0);
        self.breath_speed =
            clamp(self.breath_speed, fresh.breath_speed, BREATH_SPEED_MIN, BREATH_SPEED_MAX);
        self
    }

    /// Identity comes from the lattice coordinates, which survive a camera
    /// rebase. The modulation never feeds back into the carried glow history.
    pub fn breath(self, position: LatticePos, now: f64) -> f32 {
        if self.breath_amount == 0.0 || self.breath_speed == 0.0 {
            return 1.0;
        }
        let phase = f64::from(position.fives) * 2.173
            + f64::from(position.threes) * 3.719
            + f64::from(position.sevens) * 5.137;
        let time = now * f64::from(self.breath_speed);
        let wave =
            0.5 + (time * 0.73 + phase).sin() / 3.0 + (time * 1.13 + phase * 1.7).sin() / 6.0;
        1.0 - self.breath_amount * (1.0 - wave as f32)
    }
}
