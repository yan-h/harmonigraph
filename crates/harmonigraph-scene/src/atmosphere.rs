//! Saved controls for the lattice's dusk prototype.

use harmonigraph_core::LatticePos;

#[derive(Clone, Copy, Debug, PartialEq, serde::Serialize, serde::Deserialize)]
#[serde(default)]
pub struct AtmosphereSettings {
    pub enabled: bool,
    pub nebula_depth: f32,
    pub nebula_scale: f32,
    pub nebula_speed: f32,
    pub dusk_strength: f32,
    pub dusk_warmth: f32,
    pub dusk_speed: f32,
    pub dusk_response: f32,
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
            dusk_strength: 1.0,
            dusk_warmth: 0.45,
            dusk_speed: 1.0,
            dusk_response: 0.25,
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
        self.dusk_strength = clamp(self.dusk_strength, fresh.dusk_strength, 0.0, 3.0);
        self.dusk_warmth = clamp(self.dusk_warmth, fresh.dusk_warmth, 0.0, 1.0);
        self.dusk_speed = clamp(self.dusk_speed, fresh.dusk_speed, 0.0, 4.0);
        self.dusk_response = clamp(self.dusk_response, fresh.dusk_response, 0.0, 1.0);
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
