//! Saved controls for the lattice atmosphere prototype and for the
//! spectrogram's cloud texture, which is no longer one.

use harmonigraph_core::LatticePos;

/// Which texture the atmosphere layer draws over the spectrogram.
///
/// Distinct constructions: [`CloudStyle::Watercolor`] is a field of
/// overlapping globs that displaces the scalar picture without altering its
/// levels, then applies the shared palette.
/// [`CloudStyle::VelvetScales`] instead averages each overlapping scallop's
/// sampled light, with no raw source overlay at full material depth.
///
/// **These name what Yan sees on the page, and the code under each keeps the
/// name of its own CONSTRUCTION** — `wash_*` for the watercolour's laid-over
/// globs, `velvet_*` for the scallops behind `Scales`. That split is not an
/// oversight. A menu entry names a look and may be renamed whenever the look
/// is better described; a field names the thing the arithmetic builds.
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
/// palette colour picked by the sound under it — and every `star_` setting
/// belongs to it alone.
#[derive(Clone, Copy, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub enum CloudStyle {
    Watercolor,
    Stars,
    VelvetScales,
}

/// The band the cloud size dials run over — [`MaterialSettings::wash_size`]
/// and [`MaterialSettings::velvet_size`], which mean the same thing about
/// different textures and so are worth one pair of numbers rather than two.
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
/// 1080p export's — the faceted dome texture this was measured on (since
/// retired) ran a mean adjacent-pixel step of 0.034 at 1x,
/// 0.047 at 1/4, 0.058 at 1/8 and 0.062 at 1/16, and then FALLS to 0.061 at
/// 0.05. 1/16 is where it turns over, which is about 1.6 points to a cell, so
/// that is where the bar stops.
///
/// Scales' `Cell size` (`velvet_size`) runs over the same band, but the floor
/// was not measured on it, and its 1x is a coarser cell — about 6% of the
/// pane's height — so on that pane the floor stops it at about 2.6 points to a
/// cell rather than at the measured turnover.
///
/// It is one floor over two textures and a pane whose height varies about
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

/// Bounds shared by the [`SpectralAtmosphere::cloud_speed`] control and sanitizer.
pub const CLOUD_SPEED_MIN: f32 = 0.0;
/// See [`CLOUD_SPEED_MIN`].
pub const CLOUD_SPEED_MAX: f32 = 20.0;

/// Top of the [`SpectralAtmosphere::wash_pool`] control and sanitizer: four times
/// the strength #909 shipped as its whole range, where 0.5 is its default. The
/// bottom is its mirror, where the edge lightens instead.
pub const WASH_POOL_MAX: f32 = 4.0;
/// See [`WASH_POOL_MAX`].
pub const WASH_POOL_MIN: f32 = -WASH_POOL_MAX;
/// Bounds of [`SpectralAtmosphere::wash_pool_width`], in radii of the glob whose
/// arc the tide line lies against. The top is the whole of what the tile's
/// distance channel holds.
pub const WASH_POOL_WIDTH_MIN: f32 = 0.05;
/// See [`WASH_POOL_WIDTH_MIN`].
pub const WASH_POOL_WIDTH_MAX: f32 = 1.0;

/// The top of every depth's [`StarSettings::star_solid`], as a share of the
/// star's radius: the
/// glow keeps a tenth of the radius at least, so no star has a hard edge to
/// alias as it drifts.
pub const STAR_SOLID_MAX: f32 = 0.9;
/// The fewest [`StarSettings::star_layers`]; the most is
/// [`crate::star_plan::STAR_DEPTHS`].
pub const STAR_LAYERS_MIN: u32 = 2;
/// Bounds for [`StarSettings::star_resolution`].
pub const STAR_RESOLUTION_MIN: f32 = 0.25;
pub const STAR_RESOLUTION_MAX: f32 = 1.0;
/// The grid [`StarSettings::star_resolution`] snaps to: 25, 50, 75 and 100%.
///
/// Only the star image's pixels scale with it, so 99% saves about 2% of them
/// while every ratio below 100% pays the bilinear resample's softening: on
/// a lavapipe render at 1280x800, 99% already sat 4.5/255 from 100% where
/// 75% sat 5.7. A ratio p/q also repeats its texel-to-pixel phase every q
/// pixels, and a near-1 ratio does so slowly enough to see: 99% left a
/// column beat of 107 px in the difference, at four times the amplitude of
/// any ratio with q of 4 or less. Eighths' 7/8, 5/8 and 3/8 repeat every 8
/// px, where a modelled far star's brightness swung 6-53% as it drifted
/// against none at quarters, and 7/8 bought nearly 75%'s softness for 2.5 ms
/// more of a 4K frame.
pub const STAR_RESOLUTION_STEP: f32 = 0.25;

/// Bounds of every depth's `Star spacing` ([`StarSettings::star_spacing_ratio`])
/// and its sanitizer, in multiples of the depth's star size.
///
/// The low end is the closest a star of any size can sit and still be read
/// whole by the widest gather wherever `Position variation` puts it: a radius
/// of half the star's size in a cell of 5/12 of it is 1.2 cells, exactly
/// [`crate::star_plan::StarGather::Three`]'s half-width of 1.5 less the 0.3 a
/// centre strays at full variation. So a depth read 3x3 draws every star
/// uncapped at every setting, and no spacing asks for a wider read. A
/// cheaper read caps a star strayed toward its cell's edge rather than cut
/// it, so no star is ever cut off at a cell edge.
pub const STAR_SPACING_MIN: f32 = 5.0 / 12.0;
/// See [`STAR_SPACING_MIN`].
pub const STAR_SPACING_MAX: f32 = 32.0;
/// Bounds of every depth's `Star size` ([`StarSettings::star_size`]) and its
/// sanitizer: a star's whole diameter, glow included, in star pixels.
pub const STAR_SIZE_MIN: f32 = 0.5;
/// See [`STAR_SIZE_MIN`].
pub const STAR_SIZE_MAX: f32 = 64.0;
/// How many times its stored `Star size` the lattice draws and shows it, and
/// so its spacing, which is a multiple of the size. The glow has no fine detail for stars to pick up, so its
/// stars run bigger. Applied where the lattice draws and in its bars, never
/// stored, so both panes share one [`StarSettings::default`] and a key a blob
/// lacks takes the right fresh value in either.
pub const LATTICE_STAR_SIZE_SCALE: f32 = 5.0;
/// Bounds of every depth's `Star speed` ([`StarSettings::star_speed`]) and
/// its sanitizer, as a share of the prototype's pace: at the top a depth
/// crosses the pane's height in about nine seconds. The top was once 5, and
/// everything past 1 was too fast to use while it crowded the useful range
/// into a fifth of the track.
pub const STAR_SPEED_MIN: f32 = 0.0;
/// See [`STAR_SPEED_MIN`].
pub const STAR_SPEED_MAX: f32 = 1.0;
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
    /// Round bodies at zero, squares with barely rounded corners at one. With
    /// `velvet_tilt`, `velvet_irregularity` and `velvet_shape` at zero the
    /// squares stand on the grid: the tiled look the deleted Mosaic drew.
    pub velvet_square: f32,
    /// How far each body turns off the pane's axes, as a share of S1's
    /// random 0.2 ± 0.43 radian turn. Invisible on round bodies.
    pub velvet_tilt: f32,
    /// How big one glob is, as a multiplier on that size: how many of them
    /// cross the cloud frame moves the other way, because the count is divided
    /// by this. Larger is bigger, like `velvet_size`, and over the same
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
            velvet_square: 0.0,
            velvet_tilt: 1.0,
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
        self.velvet_square = clamp(self.velvet_square, fresh.velvet_square, 0.0, 1.0);
        self.velvet_tilt = clamp(self.velvet_tilt, fresh.velvet_tilt, 0.0, 1.0);
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
/// There is no style here. The retired Plain/Blur/Lava styles were presets over
/// effects that never depended on each other, so each surviving one — the blur
/// and the cloud — is a dial whose zero is OFF, and [`Self::effects`] is what
/// the renderer reads to pay for exactly the ones that are on. The measured
/// picture is both at zero.
#[derive(Clone, Copy, Debug, PartialEq, serde::Serialize, serde::Deserialize)]
#[serde(default)]
pub struct SpectralAtmosphere {
    pub pitch_softness: f32,
    pub time_softness: f32,
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
    /// Texture strength. Without memory, displaced levels mix before the
    /// palette. With memory, held RGB mixes in linear light.
    /// Zero disables the texture; zero refraction disables displacement alone.
    pub cloud_depth: f32,
    /// Linear-light color response times in seconds. Both zero bypass history.
    pub color_pickup: f32,
    pub color_release: f32,
    /// Drift speed, as a multiplier on a slow crossing like the lattice
    /// nebula's. The cloud FRAME it drifts in is fixed in the shader
    /// (`CLOUD_UNITS`): it used to be a dial, and it was a second copy of
    /// `wash_size`/`velvet_size` for the texture's size and of this one for its
    /// travel.
    pub cloud_speed: f32,
    /// Constant visible texture drift direction in screen degrees: 0 points
    /// right, 90 down, 180 left and 270 up.
    pub cloud_direction: f32,
    /// Watercolor's tide line: a glob shaded along the arc of the glob painted
    /// over it, 0 for none. Above 0 it darkens the level before the palette;
    /// below 0 it lightens it in proportion to `level * (1 - level)`. Silence stays on the palette's floor either way. Here rather
    /// than in [`MaterialSettings`] because the lattice's watercolor glow,
    /// which shares that struct, draws no pigment.
    pub wash_pool: f32,
    /// How far out from the arc the tide line reaches, in that glob's radii.
    pub wash_pool_width: f32,
    /// The tide line's fade: 0 a flat hard-edged band, 1 a long soft tail.
    /// See [`Self::pool_exponent`].
    pub wash_pool_softness: f32,
    pub material_settings: MaterialSettings,
    /// Which texture the layer draws; [`CloudStyle`] says what each is. Each
    /// reads its own settings and no other's: `wash_*` for `Watercolor` and
    /// `velvet_*` for `VelvetScales`, both in
    /// [`Self::material_settings`], and [`Self::stars`] for `Stars`.
    pub cloud_style: CloudStyle,
    pub stars: StarSettings,
}

/// Shared star geometry and rendering controls. Each pane owns its own values.
#[derive(Clone, Copy, Debug, PartialEq, serde::Serialize, serde::Deserialize)]
#[serde(default)]
pub struct StarSettings {
    /// How far the stars differ from each other in brightness, spent as a
    /// palette position in the spectrogram or a level of the sampled lattice
    /// hue: the steepness of the brightness rank and how far it spreads each
    /// star above and below the colour behind it, keeping their average. At 0
    /// every star is the colour behind it, lifted a little.
    pub star_randomness: f32,
    /// How far the stars shrink below their depth's size, each by its own
    /// draw, independent of its brightness: every star at its depth's size at
    /// 0, down to 1/11 of it at 1. Only shrinks, so no star grows past what
    /// its depth's read holds. Runs over 0..=1.
    pub star_size_variation: f32,
    /// Positional variation within each cell, from regular centers at 0 to
    /// the original 0.6-cell jitter width at 1. A star it strays toward its
    /// cell's edge is drawn smaller where it would overrun its depth's read
    /// ([`crate::star_plan::StarGather::half_width`]); it widens a read only
    /// where the renderer's texel floor widens the depth's stars.
    pub star_jitter: f32,
    /// How many depths the starfield draws, from
    /// [`STAR_LAYERS_MIN`]..=[`crate::star_plan::STAR_DEPTHS`]: always the
    /// back and the front, with the rest spaced evenly between, each in
    /// the depth nearest its place ([`crate::star_plan::star_layer_depths`]).
    pub star_layers: u32,
    /// Each depth's star spacing, back (0) to front, as a multiple of its own
    /// [`Self::star_size`]. Every cell holds a star, so at one value a depth's
    /// stars cover the same share of the sky whatever their size. Runs over
    /// [`STAR_SPACING_MIN`]..=[`STAR_SPACING_MAX`], whose low end is what
    /// holds every star whole, in any order across the depths.
    ///
    /// This and the other per-depth arrays hold a value for every depth, drawn
    /// or not: a depth `Star layers` leaves out keeps its own for when it is
    /// drawn again.
    pub star_spacing_ratio: [f32; crate::star_plan::STAR_DEPTHS],
    /// Each depth's star size, back (0) to front: the whole star's diameter,
    /// glow included, in star pixels, always drawn as set, since the spacing
    /// is a multiple of it ([`Self::star_spacing_ratio`]). Runs over
    /// [`STAR_SIZE_MIN`]..=[`STAR_SIZE_MAX`] (the lattice draws it
    /// [`LATTICE_STAR_SIZE_SCALE`] times over), in any order across the
    /// depths.
    pub star_size: [f32; crate::star_plan::STAR_DEPTHS],
    /// Each depth's drift speed, back (0) to front, along the shared `Drift
    /// direction`; the stars never read `cloud_speed`, which is the other
    /// textures' pace. Runs over [`STAR_SPEED_MIN`]..=[`STAR_SPEED_MAX`], in
    /// any order across the depths, so the front stars can drift slower than
    /// the back ones.
    pub star_speed: [f32; crate::star_plan::STAR_DEPTHS],
    /// How long one star lives, in seconds, before its cell draws a new one,
    /// alike at every depth. Each fades in and out over its life, as far as
    /// [`Self::star_twinkle`] says. Runs over
    /// [`STAR_LIFETIME_MIN`]..=[`STAR_LIFETIME_MAX`].
    pub star_lifetime: f32,
    /// How far each depth's stars fade out as one life gives way to the next,
    /// back (0) to front, over 0..=1. At 1 each star fades to nothing and the
    /// next is drawn somewhere new in its cell. Below 1 a cell's star keeps
    /// its place across lives, dips only this far, and blends into the next
    /// life's brightness and size, so at 0 a layer dense enough to cover the
    /// sky never opens a hole.
    pub star_twinkle: [f32; crate::star_plan::STAR_DEPTHS],
    /// How much of each depth's stars is solid, back (0) to front: the share of
    /// the star's radius at full coverage, the rest being glow that falls to
    /// nothing at the star's edge ([`crate::star_plan::star_profile`]), so a
    /// dense bed of solid far stars can sit behind near stars that are all
    /// glow. Runs over `0..=`[`STAR_SOLID_MAX`], in any order across the
    /// depths.
    pub star_solid: [f32; crate::star_plan::STAR_DEPTHS],
    /// How the glow falls from the solid edge to the star's edge, over 0..=1:
    /// 0 stays bright almost to the edge, 0.5 falls evenly, 1 drops at once
    /// and leaves a long faint tail ([`crate::star_plan::star_falloff_bend`]).
    pub star_glow_falloff: f32,
    /// The width and height of the one image every depth is drawn into,
    /// relative to the pane's device pixels, filtered up into the pane. Lower
    /// values soften every star alike and cost less, without moving stars or
    /// changing their reach. Runs over
    /// [`STAR_RESOLUTION_MIN`]..=[`STAR_RESOLUTION_MAX`], snapped to
    /// [`STAR_RESOLUTION_STEP`].
    pub star_resolution: f32,
    /// Which depths are soloed: while any drawn depth is, only the soloed
    /// ones are composited. Hidden layers keep baking and updating colour
    /// history, so toggles preserve the look and cost the full field. A flag on a
    /// depth `Star layers` leaves out counts for nothing, and the panel clears
    /// them all when the layer count changes. A look aid,
    /// never saved, so every load draws every layer and no export is soloed.
    #[serde(skip)]
    pub star_solo: [bool; crate::star_plan::STAR_DEPTHS],
}
impl Default for StarSettings {
    fn default() -> Self {
        Self {
            // Yan's Stars controls captured from the DAW on 2026-09-26.
            star_randomness: 0.080912866,
            // The old shared dial's size spread at its fresh value, anchored
            // at the top: the same smallest-to-largest ratio, now 0.47..1
            // where it was 0.69..1.45.
            star_size_variation: 0.310_684_4,
            star_jitter: 0.5,
            star_layers: crate::star_plan::STAR_DEPTHS as u32,
            // The captured spacings (1.036..6.597 star pixels, curve 2.118)
            // over the sizes below, fitted within 1% at all five depths, then
            // taken off the fitted curve (far 0.624, near the floor, exponent
            // 3.5) when each depth got its own. The nearest sits at the floor,
            // as close as its stars ever fit: an ulp above STAR_SPACING_MIN,
            // where the curve left it.
            //
            // These per-depth values are the old curves' own f32 results on
            // Apple's `powf`, printed exactly: star cells key placement, so an
            // ulp moves stars, and values rounded or worked out on another
            // platform's `powf` moved the starfield goldens by 1/255. At four
            // layers the curves were read at thirds, so the second and third
            // drawn layers differ from before.
            star_spacing_ratio: [0.624, 0.62203425, 0.602118, 0.53839743, 0.4166667],
            // A rough fit of the core-and-fringe stars this replaced: the
            // nearest at the 1.2-cell reach it was drawn to, the second
            // nearest at 0.93 of a cell and the far three at 0.80-0.82, all
            // inside the 2x2 read so they keep its cost. The far cores fill their stars, as the old capped
            // cores did, which is what made the far bed dense; the nearest
            // core is the old one's share. The gentle falloff holds the far
            // stars near full coverage out to half a cell, as the old fringe
            // on a wide core did; at 0.7 the fresh frame of the time sat
            // 5.6/255 from the old one on average. Taken, like the spacing,
            // off the curve it was set as: 1.66 to 15.8 in octaves, exponent
            // 2.3.
            star_size: [1.66, 1.8216218, 2.6231027, 5.309201, 15.799999],
            // The captured 0.0893 to 0.1686, exponent 3.18, per depth.
            star_speed: [0.08931082, 0.0902766, 0.09806162, 0.12107633, 0.16860056],
            star_lifetime: 2.9719827,
            // Every star fades to nothing and is drawn anew, as before the dial.
            star_twinkle: [1.0; crate::star_plan::STAR_DEPTHS],
            // Fitted to the Gaussian-core-and-glow stars this replaced, at their
            // fresh dials, over the star's area at every depth: 48% far to 0%
            // near, on the size's curve.
            star_solid: [0.48, 0.46020737, 0.3825297, 0.23232502, 0.0],
            star_glow_falloff: 0.5,
            // Between the old Medium preset's 50% far and 75% near images:
            // sharper than Medium, cheaper than High.
            star_resolution: 0.75,
            star_solo: [false; crate::star_plan::STAR_DEPTHS],
        }
    }
}
impl StarSettings {
    /// These settings with every size `scale` times over, and so every
    /// spacing: what the lattice draws, at [`LATTICE_STAR_SIZE_SCALE`].
    pub fn scaled(self, scale: f32) -> Self {
        Self { star_size: self.star_size.map(|size| size * scale), ..self }
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
        self.star_randomness = clamp(self.star_randomness, fresh.star_randomness, 0.0, 1.0);
        self.star_size_variation =
            clamp(self.star_size_variation, fresh.star_size_variation, 0.0, 1.0);
        self.star_jitter = clamp(self.star_jitter, fresh.star_jitter, 0.0, 1.0);
        self.star_layers =
            self.star_layers.clamp(STAR_LAYERS_MIN, crate::star_plan::STAR_DEPTHS as u32);
        // Each per-depth array runs back to front and may run either way, so its
        // values are clamped one by one and never reordered.
        let depths = |values: &mut [f32; crate::star_plan::STAR_DEPTHS],
                      fresh: [f32; crate::star_plan::STAR_DEPTHS],
                      (low, high): (f32, f32)| {
            for (value, fresh) in values.iter_mut().zip(fresh) {
                *value = clamp(*value, fresh, low, high);
            }
        };
        depths(
            &mut self.star_spacing_ratio,
            fresh.star_spacing_ratio,
            (STAR_SPACING_MIN, STAR_SPACING_MAX),
        );
        depths(&mut self.star_size, fresh.star_size, (STAR_SIZE_MIN, STAR_SIZE_MAX));
        depths(&mut self.star_speed, fresh.star_speed, (STAR_SPEED_MIN, STAR_SPEED_MAX));
        self.star_lifetime =
            clamp(self.star_lifetime, fresh.star_lifetime, STAR_LIFETIME_MIN, STAR_LIFETIME_MAX);
        depths(&mut self.star_twinkle, fresh.star_twinkle, (0.0, 1.0));
        depths(&mut self.star_solid, fresh.star_solid, (0.0, STAR_SOLID_MAX));
        self.star_glow_falloff = clamp(self.star_glow_falloff, fresh.star_glow_falloff, 0.0, 1.0);
        self.star_resolution = (clamp(
            self.star_resolution,
            fresh.star_resolution,
            STAR_RESOLUTION_MIN,
            STAR_RESOLUTION_MAX,
        ) / STAR_RESOLUTION_STEP)
            .round()
            * STAR_RESOLUTION_STEP;
        self
    }
}

/// Which of the two spectrogram effects a setting actually draws — what the
/// retired style enum used to say in one word, read off the dials instead.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct SpectralEffects {
    /// Either softness is above zero, so the picture is the blurred field.
    pub soft: bool,
    /// A cloud texture is drawn over the picture.
    pub cloud: bool,
}

impl SpectralEffects {
    /// Whether the scalar light field has to be built, which is whether
    /// anything is drawn but the measured heatmap on the renderer's plain path.
    /// The blur IS that field, and the cloud reads it — at zero softness it is
    /// the measured picture carried through unblurred, which is what lets a
    /// cloud be drawn over a sharp spectrogram.
    pub fn light(self) -> bool {
        self.soft || self.cloud
    }
}

impl Default for SpectralAtmosphere {
    fn default() -> Self {
        Self {
            // The Watercolor look captured from the DAW on 2026-10-07: a sharp
            // field with no pitch or time blur under a wash pooled at full
            // strength, with the Scales and Stars controls below riding inert
            // at their captured values.
            pitch_softness: 0.0,
            time_softness: 0.0,
            // One texel a slab: Yan judged it live at a 600 s Span (2026-09-20),
            // where it takes the pane from about 100 fps back to 144 and reads
            // the same. It binds only where the pane is finer than the data.
            blur_time_step: 1.0,
            cloud_depth: 1.0,
            color_pickup: 0.197_773_75,
            color_release: 0.207_913_31,
            cloud_speed: 2.711_340_4,
            cloud_direction: 187.846_47,
            wash_pool: 4.0,
            wash_pool_width: 0.55,
            wash_pool_softness: 0.535_810_05,
            cloud_style: CloudStyle::Watercolor,
            material_settings: MaterialSettings {
                velvet_size: 0.0625,
                velvet_variety: 1.0,
                velvet_edge: 0.646_271_65,
                velvet_irregularity: 1.0,
                velvet_square: 0.222_504_93,
                velvet_tilt: 0.0,
                wash_size: 0.163_324_39,
                wash_refract: 1.0,
                wash_layers: 0.484_902_4,
                ..MaterialSettings::default()
            },
            // Large far stars thinning to small near ones, the far three
            // layers packed at the spacing floor and steady, twinkling more
            // toward the near layer.
            stars: StarSettings {
                star_randomness: 0.136_416_58,
                star_size_variation: 0.101_771_27,
                star_jitter: 1.0,
                star_spacing_ratio: [
                    0.416_666_66,
                    0.416_666_66,
                    0.416_666_66,
                    0.673_546_14,
                    2.391_195_3,
                ],
                star_size: [7.680_750_4, 6.114_797, 4.190_219, 3.112_796_5, 2.220_784_4],
                star_speed: [0.162_222_3, 0.162_964_21, 0.168_944_7, 0.186_624_7, 0.223_133_03],
                star_lifetime: 0.984_566_4,
                star_twinkle: [0.0, 0.325_125_55, 0.585_016_3, 0.792_508_2, 1.0],
                star_glow_falloff: 0.680_821_24,
                ..StarSettings::default()
            },
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
        // Snapped to halves for the reason [`BLUR_TIME_STEP_MAX`] gives: five
        // resolutions to compare, not a continuum to hunt through.
        self.blur_time_step =
            (clamp(self.blur_time_step, fresh.blur_time_step, 0.0, BLUR_TIME_STEP_MAX) * 2.0)
                .round()
                / 2.0;
        self.cloud_depth = clamp(self.cloud_depth, fresh.cloud_depth, 0.0, 1.0);
        self.color_pickup = clamp(self.color_pickup, fresh.color_pickup, 0.0, COLOR_MEMORY_MAX);
        self.color_release = clamp(self.color_release, fresh.color_release, 0.0, COLOR_MEMORY_MAX);
        self.cloud_speed =
            clamp(self.cloud_speed, fresh.cloud_speed, CLOUD_SPEED_MIN, CLOUD_SPEED_MAX);
        self.cloud_direction = if self.cloud_direction.is_finite() {
            crate::wrap_degrees(self.cloud_direction)
        } else {
            fresh.cloud_direction
        };
        self.wash_pool = clamp(self.wash_pool, fresh.wash_pool, WASH_POOL_MIN, WASH_POOL_MAX);
        self.wash_pool_width = clamp(
            self.wash_pool_width,
            fresh.wash_pool_width,
            WASH_POOL_WIDTH_MIN,
            WASH_POOL_WIDTH_MAX,
        );
        self.wash_pool_softness =
            clamp(self.wash_pool_softness, fresh.wash_pool_softness, 0.0, 1.0);
        self.material_settings = self.material_settings.sanitized();
        self.stars = self.stars.sanitized();
        self
    }

    /// The exponent `Pooling softness` raises the tide line's linear fade to:
    /// 1/4 at 0, a nearly flat band with a hard outer edge, through 2 at the
    /// fresh 0.75, which is #909's squared crescent, to 4 at 1.
    pub fn pool_exponent(softness: f32) -> f32 {
        (4.0 * softness - 2.0).exp2()
    }

    /// Which effects these settings draw. Read off SANITIZED values — a NaN
    /// compares false to everything and would switch an effect off that the
    /// sanitizer is about to give its fresh value back to.
    pub fn effects(self) -> SpectralEffects {
        SpectralEffects {
            soft: self.pitch_softness > 0.0 || self.time_softness > 0.0,
            // A stationary sample still draws brightness variation or a temporal
            // color response; only a texture with none of these is bypassed.
            cloud: self.cloud_depth > 0.0
                && match self.cloud_style {
                    CloudStyle::Watercolor => {
                        self.material_settings.wash_refract != 0.0
                            || self.material_settings.wash_randomness > 0.0
                            || self.wash_pool != 0.0
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

/// A material sampling the textured glow; None bypasses the material pass.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[repr(u32)]
pub enum LatticeMaterial {
    #[default]
    None = 0,
    Watercolor = 1,
    Stars = 3,
    VelvetScales = 4,
}

/// Pigment's outer reach from a ring segment, in node radii.
/// Covers the former maximum half-width plus feather distance.
pub const PIGMENT_REACH_MAX: f32 = 12.0;

#[derive(Clone, Copy, Debug, PartialEq, serde::Serialize, serde::Deserialize)]
#[serde(default)]
pub struct AtmosphereSettings {
    /// Replaces the retired combined `material` key, which serde ignores.
    pub material_style: LatticeMaterial,
    pub material_amount: f32,
    /// Dark pigment from unlit segments, sampled by the material.
    pub material_shadow_pickup: f32,
    /// Pitch-colored pigment from lit segments, sampled by the material.
    pub material_color_pickup: f32,
    /// Pigment reach from a ring segment, in node radii; zero disables pickup.
    /// The fixed band/feather ratio preserves the captured 1.5-wide, 2.0-soft profile.
    pub pigment_reach: f32,
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
            // A Watercolor glow at full material, picking up a little under half
            // its colour, as captured from the DAW on 2026-10-07.
            material_style: LatticeMaterial::Watercolor,
            material_amount: 1.0,
            material_shadow_pickup: 0.0,
            material_color_pickup: 0.447_641_97,
            pigment_reach: 2.75,
            material_settings: MaterialSettings {
                wash_size: 1.459_047_9,
                wash_refract: 0.640_725_85,
                wash_layers: 0.581_831_34,
                wash_randomness: 0.384_269_62,
                ..MaterialSettings::default()
            },
            stars: StarSettings {
                star_randomness: 0.086_385_85,
                star_jitter: 1.0,
                star_layers: 4,
                star_glow_falloff: 0.624_109_57,
                ..StarSettings::default()
            },
            material_speed: 3.136_019_5,
            material_direction: 203.0,
            texture_depth: 0.104_140_51,
            texture_scale: 0.239_558_98,
            texture_speed: 5.970_565,
            breath_amount: 0.429_406_55,
            breath_speed: 1.813_457_6,
        }
    }
}

impl AtmosphereSettings {
    /// Fixed profile at the captured proportions. Divide before multiplying so
    /// the default reach reproduces width 1.5 and softness 2.0 exactly.
    pub fn pigment_width(self) -> f32 {
        (self.pigment_reach / 2.75) * 1.5
    }

    pub fn pigment_softness(self) -> f32 {
        (self.pigment_reach / 2.75) * 2.0
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
        self.material_amount = clamp(self.material_amount, fresh.material_amount, 0.0, 1.0);
        self.material_color_pickup =
            clamp(self.material_color_pickup, fresh.material_color_pickup, 0.0, 1.0);
        self.material_shadow_pickup =
            clamp(self.material_shadow_pickup, fresh.material_shadow_pickup, 0.0, 1.0);
        self.pigment_reach = clamp(self.pigment_reach, fresh.pigment_reach, 0.0, PIGMENT_REACH_MAX);
        self.material_settings = self.material_settings.sanitized();
        self.stars = self.stars.sanitized();
        self.material_speed =
            clamp(self.material_speed, fresh.material_speed, CLOUD_SPEED_MIN, CLOUD_SPEED_MAX);
        self.material_direction = if self.material_direction.is_finite() {
            crate::wrap_degrees(self.material_direction)
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
