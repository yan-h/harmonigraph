//! Saved controls for the lattice and spectral atmosphere prototypes.

use harmonigraph_core::LatticePos;

/// Which texture the atmosphere layer draws over the spectrogram.
///
/// Two constructions, not two presets of one: [`CloudStyle::Mosaic`] is a pile
/// of soft domes joined by a soft union and lit by a leaning sun, and
/// [`CloudStyle::Watercolor`] is a field of translucent globs whose tone is
/// paper minus pigment and which has no light in it at all. They share the
/// blurred light field, the palette and the drift clock, and nothing else.
///
/// **These name what Yan sees on the page, and the code under each keeps the
/// name of its own CONSTRUCTION** — `scale_*` and `dome_*` for the mosaic's
/// lit relief, `wash_*` for the watercolour's laid-over globs. That split is
/// not new and is not an oversight: the variant was `Water` over `scale_*`
/// fields before it was `Mosaic` over them. A menu entry names a look and may
/// be renamed whenever the look is better described; a field names the thing
/// the arithmetic builds, and renaming one silently resets it to its default
/// in every blob that carries it.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub enum CloudStyle {
    #[default]
    Mosaic,
    Watercolor,
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
    /// How far the levels are gathered into terraces, 0 for none. What the
    /// `Lava` style used to switch on whole.
    pub contour_strength: f32,
    pub contours: f32,
    pub contour_softness: f32,
    pub analyzer_softness: f32,
    pub note_glow: f32,
    /// Refracting scale clouds (prototype): how much of the picture the cloud
    /// takes over where it is thick, 0 for no clouds at all.
    pub cloud_depth: f32,
    /// Drift speed, as a multiplier on a slow crossing like the lattice
    /// nebula's. The cloud FRAME it drifts in is fixed in the shader
    /// (`CLOUD_UNITS`): it used to be a dial, and it was a second copy of
    /// `scale_size`/`wash_size` for the texture's size and of this one for its
    /// travel.
    pub cloud_speed: f32,
    /// Size of one scale, as a multiplier on how many of them cross a cloud.
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
    /// How domed the scales are, which is what gives them faces to catch the
    /// light with. 0 is a smooth body with no scales in it at all. It also sets
    /// how dark a face turned away from the sun may get, which used to be its
    /// own `shade_floor` dial: the floor falls as the relief rises, since a
    /// floor decides nothing where there is no tilt to shade.
    pub scale_relief: f32,
    /// How far each scale rocks on its own slow clock, so the shading on its
    /// face sways even under a picture that is holding still. 0 leaves every
    /// face where the sound puts it.
    pub scale_rock: f32,
    /// Which texture the layer draws. `Mosaic` is the refracting scale clouds
    /// above; `Watercolor` is the glob field below, and every `wash_` setting
    /// belongs to it alone.
    pub cloud_style: CloudStyle,
    /// How big one glob is, as a multiplier on how many of them cross the cloud
    /// frame. Larger is bigger, like `scale_size`.
    pub wash_size: f32,
    /// One dial over everything that dissolves a glob's rim: how far it feathers
    /// into what lies beneath, how far it bleeds into what is about to cover it,
    /// and — falling as those rise — how much of the tide line is left.
    pub wash_fuzz: f32,
    /// How far the shared fine-scale wobble carries a glob's rim off its circle.
    pub wash_ragged: f32,
    /// How far the shared domain warp carries glob space off the grid: 0 is
    /// bubbles, the top of the dial is shearing lobes.
    pub wash_lobe: f32,
    /// How far each glob's tone is pulled to the light at its own centre. 0
    /// leaves the picture exactly where it is.
    pub wash_refract: f32,
    /// How dark the pigment pools along the edge a later glob lays over this one.
    pub wash_pool: f32,
    /// How much extra pigment settles where globs are piled deepest.
    pub wash_grain: f32,
    /// How opaque the finer octave's wash is over the coarse one. 0 draws the
    /// coarse octave alone and skips the finer one's work.
    pub wash_layers: f32,
    /// How much of the picture's own black the wash gives back at the dark end.
    /// The paper is lifted by a constant so a glob over a ridge does not read as
    /// a shadow on it, and that same constant is what keeps silence off the
    /// palette's floor; this scales the tone away again where the glob found no
    /// light, and leaves every brighter tone exactly where it is. 0 is the
    /// lifted paper everywhere, 1 is silence on the palette's floor, and
    /// everything between is a share of the way — an AMOUNT, so the dial has no
    /// step anywhere on it.
    pub wash_black: f32,
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
            // Full strength is what the `Lava` style drew, and that style was
            // the fresh one.
            contour_strength: 1.0,
            contours: 7.0,
            contour_softness: 0.15,
            analyzer_softness: 0.5,
            note_glow: 0.5,
            cloud_depth: 1.0,
            cloud_speed: 1.0,
            // 1.0x now draws what `cloud_scale` 0.5 against `scale_size` 2.2
            // drew, because `SCALE_CELLS` carries the retired dial's default.
            scale_size: 1.0,
            scale_variety: 0.5,
            scale_refract: 0.30,
            scale_relief: 0.35,
            scale_rock: 0.0,
            cloud_style: CloudStyle::Mosaic,
            // J2 "dissolved" from the prototype's sheet J, translated: globs
            // about two harmonic lines across, the rim fully dissolved, the
            // wobble at the top of what the coverage proof allows.
            wash_size: 1.0,
            wash_fuzz: 1.0,
            wash_ragged: 1.0,
            wash_lobe: 0.55,
            wash_refract: 0.85,
            wash_pool: 0.5,
            wash_grain: 0.0,
            wash_layers: 0.5,
            // Not J2's: the prototype was stills over one loud passage and
            // never showed what the lift does to a quiet pane. All of it puts
            // silence back on the palette's floor and leaves the bands alone —
            // the same picture the dial drew at 50% while it was a knee width,
            // since the knee it had there is the one the shader now keeps.
            wash_black: 1.0,
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
        self.pitch_softness = clamp(self.pitch_softness, fresh.pitch_softness, 0.0, 300.0);
        self.time_softness = clamp(self.time_softness, fresh.time_softness, 0.0, 2000.0);
        self.spread = clamp(self.spread, fresh.spread, 0.0, 1.0);
        self.contour_strength = clamp(self.contour_strength, fresh.contour_strength, 0.0, 1.0);
        self.contours = clamp(self.contours, fresh.contours, 2.0, 64.0).round();
        self.contour_softness = clamp(self.contour_softness, fresh.contour_softness, 0.01, 0.5);
        self.analyzer_softness = clamp(self.analyzer_softness, fresh.analyzer_softness, 0.0, 1.0);
        self.note_glow = clamp(self.note_glow, fresh.note_glow, 0.0, 1.0);
        self.cloud_depth = clamp(self.cloud_depth, fresh.cloud_depth, 0.0, 1.0);
        self.cloud_speed = clamp(self.cloud_speed, fresh.cloud_speed, 0.0, 20.0);
        self.scale_size = clamp(self.scale_size, fresh.scale_size, 0.25, 4.0);
        self.scale_variety = clamp(self.scale_variety, fresh.scale_variety, 0.0, 1.0);
        self.scale_refract = clamp(self.scale_refract, fresh.scale_refract, -1.0, 1.0);
        self.scale_relief = clamp(self.scale_relief, fresh.scale_relief, 0.0, 1.0);
        self.scale_rock = clamp(self.scale_rock, fresh.scale_rock, 0.0, 1.0);
        self.wash_size = clamp(self.wash_size, fresh.wash_size, 0.25, 4.0);
        self.wash_fuzz = clamp(self.wash_fuzz, fresh.wash_fuzz, 0.0, 1.0);
        self.wash_ragged = clamp(self.wash_ragged, fresh.wash_ragged, 0.0, 1.0);
        self.wash_lobe = clamp(self.wash_lobe, fresh.wash_lobe, 0.0, 1.0);
        self.wash_refract = clamp(self.wash_refract, fresh.wash_refract, 0.0, 1.0);
        self.wash_pool = clamp(self.wash_pool, fresh.wash_pool, 0.0, 1.0);
        self.wash_grain = clamp(self.wash_grain, fresh.wash_grain, 0.0, 1.0);
        self.wash_layers = clamp(self.wash_layers, fresh.wash_layers, 0.0, 1.0);
        self.wash_black = clamp(self.wash_black, fresh.wash_black, 0.0, 1.0);
        self
    }

    /// Which effects these settings draw. Read off SANITIZED values — a NaN
    /// compares false to everything and would switch an effect off that the
    /// sanitizer is about to give its fresh value back to.
    pub fn effects(self) -> SpectralEffects {
        SpectralEffects {
            soft: self.pitch_softness > 0.0 || self.time_softness > 0.0,
            contours: self.contour_strength > 0.0,
            cloud: self.cloud_depth > 0.0,
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
        self.nebula_scale = clamp(self.nebula_scale, fresh.nebula_scale, 0.25, 4.0);
        self.nebula_speed = clamp(self.nebula_speed, fresh.nebula_speed, 0.0, 20.0);
        self.breath_amount = clamp(self.breath_amount, fresh.breath_amount, 0.0, 1.0);
        self.breath_speed = clamp(self.breath_speed, fresh.breath_speed, 0.0, 4.0);
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
