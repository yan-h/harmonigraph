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
    /// How much sky the clouds leave: the threshold the cloud field stands on,
    /// so the range runs from a clear pane to an overcast whose thin places the
    /// picture still shows through.
    pub cloud_cover: f32,
    /// Size of one scale, as a multiplier on how many of them cross a cloud.
    pub scale_size: f32,
    /// How far a scale bends the light behind it, in SCALE WIDTHS — the
    /// refraction, and the whole reason the layer reads as a lens rather than
    /// as something painted over the picture. 0 leaves the light where it is.
    pub scale_refract: f32,
    /// How domed the scales are, which is what gives them faces to catch the
    /// light with. 0 is a smooth body with no scales in it at all.
    pub scale_relief: f32,
    /// The specular lobe on a scale's face: the sparkle that travels as the
    /// picture scrolls under it.
    pub scale_glint: f32,
    /// Light a cloud shows with nothing sounding under it, so a cloud over
    /// silence is visible rather than black.
    pub cloud_ambient: f32,
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
            cloud_cover: 0.45,
            scale_size: 2.2,
            scale_refract: 0.30,
            scale_relief: 0.35,
            scale_glint: 0.4,
            cloud_ambient: 0.12,
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
        self.cloud_cover = clamp(self.cloud_cover, fresh.cloud_cover, 0.0, 1.0);
        self.scale_size = clamp(self.scale_size, fresh.scale_size, 0.25, 4.0);
        self.scale_refract = clamp(self.scale_refract, fresh.scale_refract, 0.0, 1.0);
        self.scale_relief = clamp(self.scale_relief, fresh.scale_relief, 0.0, 1.0);
        self.cloud_ambient = clamp(self.cloud_ambient, fresh.cloud_ambient, 0.0, 1.0);
        self.scale_glint = clamp(self.scale_glint, fresh.scale_glint, 0.0, 1.0);
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
