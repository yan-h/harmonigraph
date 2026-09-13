//! Saved controls for the lattice's dusk prototype.

use harmonigraph_core::LatticePos;

/// The particle budget is independent of pane size and the number of notes.
pub const ATMOSPHERE_MOTES_MAX: u32 = 384;

#[derive(Clone, Copy, Debug, PartialEq, serde::Serialize, serde::Deserialize)]
#[serde(default)]
pub struct AtmosphereSettings {
    pub enabled: bool,
    pub nebula_depth: f32,
    pub nebula_scale: f32,
    pub nebula_speed: f32,
    pub mote_count: u32,
    pub warm_fraction: f32,
    pub mote_size: f32,
    pub mote_brightness: f32,
    pub warm_color: [u8; 3],
    pub cool_color: [u8; 3],
    pub drift_amount: f32,
    pub drift_speed: f32,
    pub twinkle_amount: f32,
    pub twinkle_speed: f32,
    pub parallax: f32,
    pub breath_amount: f32,
    pub breath_speed: f32,
}

impl Default for AtmosphereSettings {
    fn default() -> Self {
        Self {
            enabled: true,
            nebula_depth: 0.75,
            nebula_scale: 1.0,
            nebula_speed: 1.0,
            mote_count: 96,
            warm_fraction: 0.25,
            mote_size: 1.0,
            mote_brightness: 1.0,
            warm_color: [230, 182, 100],
            cool_color: [100, 137, 180],
            drift_amount: 1.0,
            drift_speed: 1.0,
            twinkle_amount: 0.92,
            twinkle_speed: 1.0,
            parallax: 0.0,
            breath_amount: 0.18,
            breath_speed: 1.0,
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
        self.mote_count = self.mote_count.min(ATMOSPHERE_MOTES_MAX);
        self.warm_fraction = clamp(self.warm_fraction, fresh.warm_fraction, 0.0, 1.0);
        self.mote_size = clamp(self.mote_size, fresh.mote_size, 0.25, 4.0);
        self.mote_brightness = clamp(self.mote_brightness, fresh.mote_brightness, 0.0, 4.0);
        self.drift_amount = clamp(self.drift_amount, fresh.drift_amount, 0.0, 4.0);
        self.drift_speed = clamp(self.drift_speed, fresh.drift_speed, 0.0, 4.0);
        self.twinkle_amount = clamp(self.twinkle_amount, fresh.twinkle_amount, 0.0, 1.0);
        self.twinkle_speed = clamp(self.twinkle_speed, fresh.twinkle_speed, 0.0, 4.0);
        self.parallax = clamp(self.parallax, fresh.parallax, 0.0, 1.0);
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
