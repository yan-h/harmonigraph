//! Saved controls for the lattice and spectral atmosphere prototypes.

use harmonigraph_core::LatticePos;

/// Display transfer after shared measurement and optional smoothing.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub enum SpectrogramStyle {
    Plain,
    Blur,
    #[default]
    Lava,
}

/// Which texture the atmosphere layer draws over the spectrogram.
///
/// Two constructions, not two presets of one: [`CloudStyle::Water`] is a pile of
/// soft domes joined by a soft union and lit by a leaning sun, and
/// [`CloudStyle::Wash`] is a field of translucent watercolour globs whose tone is
/// paper minus pigment and which has no light in it at all. They share the
/// blurred light field, the palette and the drift clock, and nothing else.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub enum CloudStyle {
    #[default]
    Water,
    Wash,
}

/// Independent spectrogram diffusion, analyzer shading and note light.
#[derive(Clone, Copy, Debug, PartialEq, serde::Serialize, serde::Deserialize)]
#[serde(default)]
pub struct SpectralAtmosphere {
    pub style: SpectrogramStyle,
    pub pitch_softness: f32,
    pub time_softness: f32,
    pub spread: f32,
    pub contours: f32,
    pub contour_softness: f32,
    pub analyzer_softness: f32,
    pub note_glow: f32,
    /// Refracting scale clouds (prototype): how much of the picture the cloud
    /// takes over where it is thick, 0 for no clouds at all.
    pub cloud_depth: f32,
    /// Cloud size and drift speed, as multipliers of a reference size and a
    /// slow drift, like the lattice nebula's.
    pub cloud_scale: f32,
    pub cloud_speed: f32,
    /// Size of one scale, as a multiplier on how many of them cross a cloud.
    pub scale_size: f32,
    /// How much the scales differ in size from each other. 0 is one radius for
    /// every glob in the field, which is the most regular texture there is; 1
    /// draws each from the whole band the layer's coverage proof allows.
    pub scale_variety: f32,
    /// How far a scale bends the light behind it, in SCALE WIDTHS — the
    /// refraction, and the whole reason the layer reads as a lens rather than
    /// as something painted over the picture. 0 leaves the light where it is.
    pub scale_refract: f32,
    /// Where a scale reads the light: at 0 where its own face POINTS, at 1 at
    /// the scale's CENTRE — which is one value across the whole scale, so the
    /// picture comes apart into flat quantized patches instead of bending
    /// smoothly.
    pub scale_facet: f32,
    /// How domed the scales are, which is what gives them faces to catch the
    /// light with. 0 is a smooth body with no scales in it at all.
    pub scale_relief: f32,
    /// How far each scale rocks on its own slow clock, so the shading on its
    /// face sways even under a picture that is holding still. 0 leaves every
    /// face where the sound puts it.
    pub scale_rock: f32,
    /// How much light a face turned away from the sun still keeps: 0 lets it go
    /// black, 1 flattens the shading away entirely.
    pub scale_shade_floor: f32,
    /// Which texture the layer draws. `Water` is the refracting scale clouds
    /// above; `Wash` is the watercolour glob field below, and every `wash_`
    /// setting belongs to it alone.
    pub cloud_style: CloudStyle,
    /// How big one glob is, as a multiplier on how many of them cross the cloud
    /// frame. Larger is bigger, like `scale_size`.
    pub wash_size: f32,
    /// How much the globs differ in size from each other, over the band the
    /// layer's coverage proof allows.
    pub wash_variety: f32,
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
    /// How far the light a glob reads is carried from the close material toward
    /// the wide blur.
    pub wash_soften: f32,
    /// How fast each glob's centre turns about its own cell, on its own hashed
    /// rate. A rate rather than a distance: the centre swings round the offset
    /// the jitter already gave it, so the field can never stir a glob out of the
    /// neighbourhood a pixel searches.
    pub wash_wander: f32,
    /// How far up the dark end the wash is pulled back to the picture's own
    /// black. The paper is lifted by a constant so a glob over a ridge does not
    /// read as a shadow on it, and that same constant is what keeps silence off
    /// the palette's floor; this scales the tone away again where the glob found
    /// no light, and leaves every brighter tone exactly where it is. 0 is the
    /// lifted paper everywhere.
    pub wash_black: f32,
}

impl Default for SpectralAtmosphere {
    fn default() -> Self {
        Self {
            style: SpectrogramStyle::Lava,
            pitch_softness: 35.0,
            time_softness: 120.0,
            spread: 0.25,
            contours: 7.0,
            contour_softness: 0.15,
            analyzer_softness: 0.5,
            note_glow: 0.5,
            cloud_depth: 1.0,
            cloud_scale: 0.5,
            cloud_speed: 1.0,
            scale_size: 2.2,
            scale_variety: 0.5,
            scale_refract: 0.30,
            scale_facet: 0.0,
            scale_relief: 0.35,
            scale_rock: 0.0,
            scale_shade_floor: 0.25,
            cloud_style: CloudStyle::Water,
            // J2 "dissolved" from the prototype's sheet J, translated: globs
            // about two harmonic lines across, the rim fully dissolved, the
            // wobble at the top of what the coverage proof allows.
            wash_size: 1.0,
            wash_variety: 1.0,
            wash_fuzz: 1.0,
            wash_ragged: 1.0,
            wash_lobe: 0.55,
            wash_refract: 0.85,
            wash_pool: 0.5,
            wash_grain: 0.0,
            wash_layers: 0.5,
            wash_soften: 0.0,
            wash_wander: 0.0,
            // Not J2's: the prototype was stills over one loud passage and
            // never showed what the lift does to a quiet pane. Half the band
            // puts silence back on the palette's floor and leaves the bands
            // alone.
            wash_black: 0.5,
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
        self.contours = clamp(self.contours, fresh.contours, 2.0, 64.0).round();
        self.contour_softness = clamp(self.contour_softness, fresh.contour_softness, 0.01, 0.5);
        self.analyzer_softness = clamp(self.analyzer_softness, fresh.analyzer_softness, 0.0, 1.0);
        self.note_glow = clamp(self.note_glow, fresh.note_glow, 0.0, 1.0);
        self.cloud_depth = clamp(self.cloud_depth, fresh.cloud_depth, 0.0, 1.0);
        self.cloud_scale = clamp(self.cloud_scale, fresh.cloud_scale, 0.25, 4.0);
        self.cloud_speed = clamp(self.cloud_speed, fresh.cloud_speed, 0.0, 20.0);
        self.scale_size = clamp(self.scale_size, fresh.scale_size, 0.25, 4.0);
        self.scale_variety = clamp(self.scale_variety, fresh.scale_variety, 0.0, 1.0);
        self.scale_refract = clamp(self.scale_refract, fresh.scale_refract, 0.0, 1.0);
        self.scale_facet = clamp(self.scale_facet, fresh.scale_facet, 0.0, 1.0);
        self.scale_relief = clamp(self.scale_relief, fresh.scale_relief, 0.0, 1.0);
        self.scale_rock = clamp(self.scale_rock, fresh.scale_rock, 0.0, 1.0);
        self.scale_shade_floor = clamp(self.scale_shade_floor, fresh.scale_shade_floor, 0.0, 1.0);
        self.wash_size = clamp(self.wash_size, fresh.wash_size, 0.25, 4.0);
        self.wash_variety = clamp(self.wash_variety, fresh.wash_variety, 0.0, 1.0);
        self.wash_fuzz = clamp(self.wash_fuzz, fresh.wash_fuzz, 0.0, 1.0);
        self.wash_ragged = clamp(self.wash_ragged, fresh.wash_ragged, 0.0, 1.0);
        self.wash_lobe = clamp(self.wash_lobe, fresh.wash_lobe, 0.0, 1.0);
        self.wash_refract = clamp(self.wash_refract, fresh.wash_refract, 0.0, 1.0);
        self.wash_pool = clamp(self.wash_pool, fresh.wash_pool, 0.0, 1.0);
        self.wash_grain = clamp(self.wash_grain, fresh.wash_grain, 0.0, 1.0);
        self.wash_layers = clamp(self.wash_layers, fresh.wash_layers, 0.0, 1.0);
        self.wash_soften = clamp(self.wash_soften, fresh.wash_soften, 0.0, 1.0);
        self.wash_wander = clamp(self.wash_wander, fresh.wash_wander, 0.0, 1.0);
        self.wash_black = clamp(self.wash_black, fresh.wash_black, 0.0, 1.0);
        self
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
