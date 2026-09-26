//! How loud a note is drawn: one intensity per note per moment, made of its
//! velocity and the host's per-note expressions, which each display then reads
//! through a floor of its own.
//!
//! ONE number rather than a matrix of source against display. Adding a display
//! then costs one floor rather than a row of weights, and the four sources add
//! up rather than multiplying, so a soft velocity and a gain boost give each
//! other headroom. The cap is on the sum, never on one input.
//!
//! The trail never reads this; it records pitch classes only.

use crate::view::finite_or;
use harmonigraph_core::Expressions;

/// The ends of every weight's bar, and of the offset's.
pub const INTENSITY_WEIGHT_MAX: f32 = 2.0;
/// The narrowest and widest the gain range can be set to, in dB.
pub const GAIN_RANGE_MIN: f32 = 3.0;
/// See [`GAIN_RANGE_MIN`].
pub const GAIN_RANGE_MAX: f32 = 60.0;

/// The settings the intensity is made from, and each display's floor.
///
/// Fresh, the offset is 1 and every weight 0, so every note is at full
/// intensity whatever the floors say and a project that never opens these
/// looks as it did before they existed.
#[derive(Clone, Copy, Debug, PartialEq, serde::Serialize, serde::Deserialize)]
#[serde(default)]
pub struct IntensitySettings {
    /// Where the sum starts before any source adds to it.
    pub offset: f32,
    /// Weight of the note-on velocity, 0 to 1.
    pub velocity: f32,
    /// Weight of the note's gain, in dB off unity and divided by
    /// [`gain_range`](Self::gain_range).
    pub gain: f32,
    /// Weight of pressure, 0 to 1.
    pub pressure: f32,
    /// Weight of timbre, 0 to 1 with 0.5 where an untouched lane sits.
    pub timbre: f32,
    /// How many dB of gain move the intensity by one weight's worth: at 24,
    /// +12 dB adds half the gain weight and -24 dB takes a whole one away.
    pub gain_range: f32,
    /// The note's opacity at zero intensity, on the lattice's octave slices and
    /// the roll's ribbons alike; 1 is a fade that ignores intensity.
    pub fade_floor: f32,
    /// How much light a note gives off at zero intensity, as a share of the
    /// global glow: the lattice's halo and the roll's bloom alike; 1 is a glow
    /// that ignores intensity.
    pub glow_floor: f32,
}

impl Default for IntensitySettings {
    fn default() -> Self {
        Self {
            offset: 1.0,
            velocity: 0.0,
            gain: 0.0,
            pressure: 0.0,
            timbre: 0.0,
            gain_range: 24.0,
            fade_floor: 0.2,
            glow_floor: 0.0,
        }
    }
}

impl IntensitySettings {
    /// Every value finite and on its bar, falling back to the fresh value.
    pub fn sanitized(mut self) -> Self {
        let fresh = Self::default();
        let weight = |value: f32, fallback: f32| {
            finite_or(value, fallback).clamp(-INTENSITY_WEIGHT_MAX, INTENSITY_WEIGHT_MAX)
        };
        self.offset = weight(self.offset, fresh.offset);
        self.velocity = weight(self.velocity, fresh.velocity);
        self.gain = weight(self.gain, fresh.gain);
        self.pressure = weight(self.pressure, fresh.pressure);
        self.timbre = weight(self.timbre, fresh.timbre);
        self.gain_range =
            finite_or(self.gain_range, fresh.gain_range).clamp(GAIN_RANGE_MIN, GAIN_RANGE_MAX);
        self.fade_floor = finite_or(self.fade_floor, fresh.fade_floor).clamp(0.0, 1.0);
        self.glow_floor = finite_or(self.glow_floor, fresh.glow_floor).clamp(0.0, 1.0);
        self
    }

    /// One note's intensity, 0 to 1, from its velocity and where its
    /// expressions stand.
    ///
    /// A term whose weight is 0 is skipped rather than multiplied, and the
    /// gain's dB are bounded before they are weighed: silence is -inf dB, and
    /// 0 times that, or a +inf gain term meeting a -inf one, is a NaN.
    pub fn intensity(&self, velocity: f32, expressions: Expressions) -> f32 {
        // Far past any range the bar offers, so silence still pulls the sum
        // down past 0 at the smallest weight, while staying finite.
        const DB_BOUND: f32 = 1.0e4;
        let term = |weight: f32, value: f32| if weight == 0.0 { 0.0 } else { weight * value };
        let db = (20.0 * expressions.gain.log10()).clamp(-DB_BOUND, DB_BOUND);
        let sum = self.offset
            + term(self.velocity, velocity)
            + term(self.gain, db / self.gain_range.max(GAIN_RANGE_MIN))
            + term(self.pressure, expressions.pressure)
            + term(self.timbre, expressions.timbre);
        // `clamp` passes a NaN through, and a host's value is not guaranteed
        // to be a number; such a note draws at full rather than not at all.
        finite_or(sum, 1.0).clamp(0.0, 1.0)
    }

    /// The opacity a note of this `intensity` draws at.
    pub fn fade(&self, intensity: f32) -> f32 {
        read_through(self.fade_floor, intensity)
    }

    /// How much light a note of this `intensity` gives off, as a share of the
    /// glow it would give at full.
    pub fn glow(&self, intensity: f32) -> f32 {
        read_through(self.glow_floor, intensity)
    }

    /// [`fade`](Self::fade) of [`intensity`](Self::intensity).
    pub fn note_fade(&self, velocity: f32, expressions: Expressions) -> f32 {
        self.fade(self.intensity(velocity, expressions))
    }
}

/// A display reading `intensity` through its `floor`: the floor at zero
/// intensity, full at full intensity, and a straight line between.
fn read_through(floor: f32, intensity: f32) -> f32 {
    floor + (1.0 - floor) * intensity
}

#[cfg(test)]
mod tests {
    use super::*;

    fn gain(gain: f32) -> Expressions {
        Expressions { gain, ..Expressions::NEUTRAL }
    }

    /// Fresh, a note is at full intensity whatever it carries, so a project
    /// that never touches these draws as it did.
    #[test]
    fn fresh_settings_ignore_every_source() {
        let fresh = IntensitySettings::default();
        let quiet = Expressions { pressure: 0.0, gain: 0.0, timbre: 0.0 };
        assert_eq!(fresh.intensity(0.1, quiet), 1.0);
        assert_eq!(fresh.note_fade(0.1, quiet), 1.0);
    }

    /// The sources ADD, and the cap is on the sum: half velocity plus +6 dB
    /// comes out louder than either, and a +12 dB boost on a loud note stops
    /// at 1.
    #[test]
    fn sources_add_and_the_sum_is_capped() {
        let settings =
            IntensitySettings { offset: 0.0, velocity: 1.0, gain: 1.0, ..Default::default() };
        let quarter_up = 10.0f32.powf(6.0 / 20.0);
        assert!((settings.intensity(0.5, gain(quarter_up)) - 0.75).abs() < 1e-4);
        assert_eq!(settings.intensity(0.9, gain(4.0)), 1.0);
        let cut = settings.intensity(0.5, gain(10.0f32.powf(-6.0 / 20.0)));
        assert!((cut - 0.25).abs() < 1e-4, "{cut}");
    }

    /// Silence is -inf dB. It takes a positively weighted note to 0, a
    /// negatively weighted one to 1, and at weight 0 it is no NaN.
    #[test]
    fn silence_is_bounded_whatever_the_weight() {
        let silent = gain(0.0);
        let weighted = |gain| IntensitySettings { gain, ..Default::default() };
        assert_eq!(weighted(0.5).intensity(1.0, silent), 0.0);
        assert_eq!(weighted(-0.5).intensity(1.0, silent), 1.0);
        assert_eq!(weighted(0.0).intensity(1.0, silent), 1.0);
        let nan = Expressions { pressure: f32::NAN, ..Expressions::NEUTRAL };
        let pressed = IntensitySettings { pressure: 1.0, ..Default::default() };
        assert_eq!(pressed.intensity(1.0, nan), 1.0);
    }

    /// A display reads intensity through its floor: the floor at zero, full at
    /// one, and a floor of 1 ignores intensity entirely.
    #[test]
    fn a_floor_is_the_value_at_zero() {
        let at = |fade_floor, intensity| {
            IntensitySettings { fade_floor, ..Default::default() }.fade(intensity)
        };
        assert_eq!(at(0.2, 0.0), 0.2);
        assert_eq!(at(0.2, 1.0), 1.0);
        assert!((at(0.2, 0.5) - 0.6).abs() < 1e-6);
        assert_eq!(at(1.0, 0.0), 1.0);
    }
}
