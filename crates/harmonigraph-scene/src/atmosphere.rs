//! Saved controls for the lattice atmosphere prototype and for the
//! spectrogram's cloud texture, which is no longer one.

use harmonigraph_core::LatticePos;

/// Which texture the atmosphere layer draws over the spectrogram.
///
/// Two constructions, not two presets of one: [`CloudStyle::Mosaic`] is a pile
/// of soft domes joined by a soft union, and [`CloudStyle::Watercolor`] is a
/// field of overlapping globs. Both displace the same scalar picture without
/// altering its levels, then apply the shared Contours and palette controls.
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
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub enum CloudStyle {
    #[default]
    Mosaic,
    Watercolor,
}

/// The band both cloud size dials run over — [`SpectralAtmosphere::scale_size`]
/// and [`SpectralAtmosphere::wash_size`], which mean the same thing about two
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
/// 1080p export's — the mosaic's mean adjacent-pixel step runs 0.034 at 1x,
/// 0.047 at 1/4, 0.058 at 1/8 and 0.062 at 1/16, and then FALLS to 0.061 at
/// 0.05. 1/16 is where it turns over, which is about 1.6 points to a cell, so
/// that is where the bar stops.
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

/// Bounds shared by the [`SpectralAtmosphere::contours`] control and sanitizer.
pub const CONTOURS_MIN: f32 = 2.0;
/// See [`CONTOURS_MIN`].
pub const CONTOURS_MAX: f32 = 20.0;

/// Bounds shared by the [`SpectralAtmosphere::contour_softness`] control and sanitizer.
pub const CONTOUR_SOFTNESS_MIN: f32 = 0.01;
/// See [`CONTOUR_SOFTNESS_MIN`].
pub const CONTOUR_SOFTNESS_MAX: f32 = 0.5;

/// Bounds shared by the [`SpectralAtmosphere::cloud_speed`] control and sanitizer.
pub const CLOUD_SPEED_MIN: f32 = 0.0;
/// See [`CLOUD_SPEED_MIN`].
pub const CLOUD_SPEED_MAX: f32 = 20.0;

/// Bounds shared by the [`SpectralAtmosphere::cloud_direction`] control and sanitizer.
pub const CLOUD_DIRECTION_MIN: f32 = 0.0;
/// See [`CLOUD_DIRECTION_MIN`].
pub const CLOUD_DIRECTION_MAX: f32 = 360.0;

/// Bounds shared by the [`SpectralAtmosphere::scale_refract`] control and sanitizer.
pub const SCALE_REFRACT_MIN: f32 = -1.0;
/// See [`SCALE_REFRACT_MIN`].
pub const SCALE_REFRACT_MAX: f32 = 1.0;

/// Bounds shared by the [`AtmosphereSettings::nebula_scale`] control and sanitizer.
pub const NEBULA_SCALE_MIN: f32 = 0.25;
/// See [`NEBULA_SCALE_MIN`].
pub const NEBULA_SCALE_MAX: f32 = 4.0;

/// Bounds shared by the [`AtmosphereSettings::nebula_speed`] control and sanitizer.
pub const NEBULA_SPEED_MIN: f32 = 0.0;
/// See [`NEBULA_SPEED_MIN`].
pub const NEBULA_SPEED_MAX: f32 = 20.0;

/// Bounds shared by the [`AtmosphereSettings::breath_speed`] control and sanitizer.
pub const BREATH_SPEED_MIN: f32 = 0.0;
/// See [`BREATH_SPEED_MIN`].
pub const BREATH_SPEED_MAX: f32 = 4.0;

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
    pub analyzer_softness: f32,
    pub note_glow: f32,
    /// Blend from original to displaced levels before Contours and the palette.
    /// Zero disables the texture; zero refraction is also exactly neutral.
    pub cloud_depth: f32,
    /// Drift speed, as a multiplier on a slow crossing like the lattice
    /// nebula's. The cloud FRAME it drifts in is fixed in the shader
    /// (`CLOUD_UNITS`): it used to be a dial, and it was a second copy of
    /// `scale_size`/`wash_size` for the texture's size and of this one for its
    /// travel.
    pub cloud_speed: f32,
    /// Constant visible texture drift direction in screen degrees: 0 points
    /// right, 90 down, 180 left and 270 up.
    pub cloud_direction: f32,
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
    /// Which texture the layer draws. `Mosaic` is the refracting scale clouds
    /// above; `Watercolor` is the glob field below, and every `wash_` setting
    /// belongs to it alone.
    pub cloud_style: CloudStyle,
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
            pitch_softness: 35.0,
            time_softness: 120.0,
            spread: 0.25,
            // One texel a slab: Yan judged it live at a 600 s Span (2026-09-20),
            // where it takes the pane from about 100 fps back to 144 and reads
            // the same. It binds only where the pane is finer than the data.
            blur_time_step: 1.0,
            // Full strength is what the `Lava` style drew, and that style was
            // the fresh one.
            contour_strength: 1.0,
            contours: 7.0,
            contour_softness: 0.15,
            analyzer_softness: 0.5,
            note_glow: 0.5,
            cloud_depth: 1.0,
            cloud_speed: 1.0,
            // The visible direction of the former drift's steady component.
            cloud_direction: 147.994_61,
            // 1.0x now draws what `cloud_scale` 0.5 against `scale_size` 2.2
            // drew, because `SCALE_CELLS` carries the retired dial's default.
            scale_size: 1.0,
            scale_variety: 0.5,
            scale_refract: 0.30,
            cloud_style: CloudStyle::Mosaic,
            // J2 "dissolved" from the prototype's sheet J, translated: globs
            // about two harmonic lines across and the rim fully dissolved. Its
            // third term was a `Ragged` rim wobble, retired once `Fuzz` 1 was
            // found to mask it; the radius band carries the size it added.
            wash_size: 1.0,
            wash_fuzz: 1.0,
            wash_lobe: 0.55,
            wash_refract: 0.85,
            wash_layers: 0.5,
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
        self.analyzer_softness = clamp(self.analyzer_softness, fresh.analyzer_softness, 0.0, 1.0);
        self.note_glow = clamp(self.note_glow, fresh.note_glow, 0.0, 1.0);
        self.cloud_depth = clamp(self.cloud_depth, fresh.cloud_depth, 0.0, 1.0);
        self.cloud_speed =
            clamp(self.cloud_speed, fresh.cloud_speed, CLOUD_SPEED_MIN, CLOUD_SPEED_MAX);
        self.cloud_direction = if self.cloud_direction.is_finite() {
            self.cloud_direction.rem_euclid(CLOUD_DIRECTION_MAX)
        } else {
            fresh.cloud_direction
        };
        self.scale_size = clamp(self.scale_size, fresh.scale_size, CLOUD_SIZE_MIN, CLOUD_SIZE_MAX);
        self.scale_variety = clamp(self.scale_variety, fresh.scale_variety, 0.0, 1.0);
        self.scale_refract =
            clamp(self.scale_refract, fresh.scale_refract, SCALE_REFRACT_MIN, SCALE_REFRACT_MAX);
        self.wash_size = clamp(self.wash_size, fresh.wash_size, CLOUD_SIZE_MIN, CLOUD_SIZE_MAX);
        self.wash_fuzz = clamp(self.wash_fuzz, fresh.wash_fuzz, 0.0, 1.0);
        self.wash_lobe = clamp(self.wash_lobe, fresh.wash_lobe, 0.0, 1.0);
        self.wash_refract = clamp(self.wash_refract, fresh.wash_refract, 0.0, 1.0);
        self.wash_layers = clamp(self.wash_layers, fresh.wash_layers, 0.0, 1.0);
        self
    }

    /// Which effects these settings draw. Read off SANITIZED values — a NaN
    /// compares false to everything and would switch an effect off that the
    /// sanitizer is about to give its fresh value back to.
    pub fn effects(self) -> SpectralEffects {
        SpectralEffects {
            soft: self.pitch_softness > 0.0 || self.time_softness > 0.0,
            contours: self.contour_strength > 0.0,
            // Zero refraction takes the ordinary picture path, including its
            // exact palette lookup, and spends nothing building a texture.
            cloud: self.cloud_depth > 0.0
                && match self.cloud_style {
                    CloudStyle::Mosaic => self.scale_refract != 0.0,
                    CloudStyle::Watercolor => self.wash_refract != 0.0,
                },
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, serde::Serialize, serde::Deserialize)]
#[serde(default)]
pub struct AtmosphereSettings {
    pub enabled: bool,
    pub nebula_depth: f32,
    pub nebula_scale: f32,
    pub nebula_speed: f32,
    pub breath_amount: f32,
    pub breath_speed: f32,
}

impl Default for AtmosphereSettings {
    fn default() -> Self {
        Self {
            enabled: true,
            nebula_depth: 0.139_642_13,
            nebula_scale: 0.581_716_2,
            nebula_speed: 1.0,
            breath_amount: 0.582_938_5,
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
        self.nebula_depth = clamp(self.nebula_depth, fresh.nebula_depth, 0.0, 1.0);
        self.nebula_scale =
            clamp(self.nebula_scale, fresh.nebula_scale, NEBULA_SCALE_MIN, NEBULA_SCALE_MAX);
        self.nebula_speed =
            clamp(self.nebula_speed, fresh.nebula_speed, NEBULA_SPEED_MIN, NEBULA_SPEED_MAX);
        self.breath_amount = clamp(self.breath_amount, fresh.breath_amount, 0.0, 1.0);
        self.breath_speed =
            clamp(self.breath_speed, fresh.breath_speed, BREATH_SPEED_MIN, BREATH_SPEED_MAX);
        self
    }

    /// Identity comes from the lattice coordinates, which survive a camera
    /// rebase. The modulation never feeds back into the carried glow history.
    pub fn breath(self, position: LatticePos, now: f64) -> f32 {
        if !self.enabled || self.breath_amount == 0.0 || self.breath_speed == 0.0 {
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
