//! How a note is drawn from how it is played: each of its sources — velocity
//! and the host's per-note gain, pressure and timbre — is ROUTED to one
//! display with a weight of its own, and each display adds up what is routed
//! to it.
//!
//! One target per source rather than a matrix of every source against every
//! display, so the picture says plainly what each source does. The sources on
//! one display ADD rather than multiply, so a soft velocity and a gain boost
//! give each other headroom; the cap is on the sum, never on one input.
//!
//! A display nothing is routed to stays at full, so fresh settings draw every
//! project as it was before these existed. The trail never reads any of this;
//! it records pitch classes only.

use crate::view::finite_or;
use harmonigraph_core::Expressions;

/// The ends of every weight's bar and every base's.
pub const INTENSITY_WEIGHT_MAX: f32 = 2.0;
/// The narrowest and widest the gain range can be set to, in dB.
pub const GAIN_RANGE_MIN: f32 = 3.0;
/// See [`GAIN_RANGE_MIN`].
pub const GAIN_RANGE_MAX: f32 = 60.0;

/// Which display a source drives.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub enum IntensityTarget {
    /// Drives nothing.
    #[default]
    Off,
    /// The note's opacity: the lattice's octave slices and the roll's ribbons.
    Opacity,
    /// The light it gives off: the lattice's node glow and the roll's bloom.
    Glow,
    /// How thick it is drawn. Nothing draws this yet (#1130 step 4).
    Thickness,
}

impl IntensityTarget {
    /// Every target, for the settings picker. Guarded exhaustively so a new
    /// display cannot miss it.
    pub const ALL: [Self; 4] = {
        const fn covered(target: IntensityTarget) {
            use IntensityTarget::*;
            match target {
                Off | Opacity | Glow | Thickness => (),
            }
        }
        covered(IntensityTarget::Off);
        [Self::Off, Self::Opacity, Self::Glow, Self::Thickness]
    };
}

/// One source's route: which display it drives, and how hard.
#[derive(Clone, Copy, Debug, PartialEq, serde::Serialize, serde::Deserialize)]
#[serde(default)]
pub struct IntensitySource {
    pub target: IntensityTarget,
    /// What the source's value is multiplied by before its display adds it;
    /// negative turns it around.
    pub weight: f32,
}

impl Default for IntensitySource {
    fn default() -> Self {
        Self { target: IntensityTarget::Off, weight: 1.0 }
    }
}

/// Every source's route, and what each display starts from.
#[derive(Clone, Copy, Debug, PartialEq, serde::Serialize, serde::Deserialize)]
#[serde(default)]
pub struct IntensitySettings {
    /// The note-on velocity, 0 to 1.
    pub velocity: IntensitySource,
    /// The note's gain, in dB off unity divided by
    /// [`gain_range`](Self::gain_range): 0 at unity, so it adds a boost and
    /// takes away a cut.
    pub gain: IntensitySource,
    /// Pressure, 0 to 1.
    pub pressure: IntensitySource,
    /// Timbre, 0 to 1, with 0.5 where an untouched lane sits.
    pub timbre: IntensitySource,
    /// How many dB of gain make one weight's worth: at 24, +12 dB adds half
    /// the gain's weight and -24 dB takes a whole one away.
    pub gain_range: f32,
    /// Where each display's sum starts before the sources routed to it add in.
    /// Read only while something is routed there.
    pub opacity_base: f32,
    /// See [`opacity_base`](Self::opacity_base).
    pub glow_base: f32,
    /// See [`opacity_base`](Self::opacity_base).
    pub thickness_base: f32,
}

impl Default for IntensitySettings {
    fn default() -> Self {
        Self {
            velocity: IntensitySource::default(),
            gain: IntensitySource::default(),
            pressure: IntensitySource::default(),
            timbre: IntensitySource::default(),
            gain_range: 24.0,
            opacity_base: 0.0,
            glow_base: 0.0,
            thickness_base: 0.0,
        }
    }
}

/// What a note's sources come to on each display, each 0 to 1 with 1 as the
/// note drawn in full.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct IntensityReading {
    pub opacity: f32,
    pub glow: f32,
    pub thickness: f32,
}

impl IntensityReading {
    /// Every display in full, which is what a display nothing drives reads.
    pub const FULL: Self = Self { opacity: 1.0, glow: 1.0, thickness: 1.0 };

    /// The larger of the two on each display.
    pub fn max(self, other: Self) -> Self {
        Self {
            opacity: self.opacity.max(other.opacity),
            glow: self.glow.max(other.glow),
            thickness: self.thickness.max(other.thickness),
        }
    }
}

impl Default for IntensityReading {
    fn default() -> Self {
        Self::FULL
    }
}

impl IntensitySettings {
    /// Every value finite and on its bar, falling back to the fresh value.
    pub fn sanitized(mut self) -> Self {
        let fresh = Self::default();
        let bar = |value: f32, fallback: f32| {
            finite_or(value, fallback).clamp(-INTENSITY_WEIGHT_MAX, INTENSITY_WEIGHT_MAX)
        };
        for source in [&mut self.velocity, &mut self.gain, &mut self.pressure, &mut self.timbre] {
            source.weight = bar(source.weight, fresh.velocity.weight);
        }
        self.gain_range =
            finite_or(self.gain_range, fresh.gain_range).clamp(GAIN_RANGE_MIN, GAIN_RANGE_MAX);
        self.opacity_base = bar(self.opacity_base, fresh.opacity_base);
        self.glow_base = bar(self.glow_base, fresh.glow_base);
        self.thickness_base = bar(self.thickness_base, fresh.thickness_base);
        self
    }

    /// Whether any source drives `target`. A display none does reads in full
    /// and ignores its base.
    pub fn routes_to(&self, target: IntensityTarget) -> bool {
        target != IntensityTarget::Off
            && [self.velocity, self.gain, self.pressure, self.timbre]
                .iter()
                .any(|source| source.target == target)
    }

    /// What one note, played at `velocity` with its expressions standing at
    /// `expressions`, comes to on every display.
    pub fn read(&self, velocity: f32, expressions: Expressions) -> IntensityReading {
        // Silence is -inf dB. Bounded far past any range the bar offers, so it
        // still pulls a sum down past 0 at the smallest weight while staying
        // finite: a -inf meeting a +inf is a NaN.
        const DB_BOUND: f32 = 1.0e4;
        let db = (20.0 * expressions.gain.log10()).clamp(-DB_BOUND, DB_BOUND);
        let sources = [
            (self.velocity, velocity),
            (self.gain, db / self.gain_range.max(GAIN_RANGE_MIN)),
            (self.pressure, expressions.pressure),
            (self.timbre, expressions.timbre),
        ];
        let display = |target: IntensityTarget, base: f32| {
            if !self.routes_to(target) {
                return 1.0;
            }
            // A zero weight is skipped rather than multiplied, which is what
            // keeps a silent note's bounded dB out of a sum it has no part in.
            let sum = base
                + sources
                    .iter()
                    .filter(|(source, _)| source.target == target && source.weight != 0.0)
                    .map(|(source, value)| source.weight * value)
                    .sum::<f32>();
            // `clamp` passes a NaN through, and a host's value is not
            // guaranteed to be a number; such a note draws in full rather than
            // not at all.
            finite_or(sum, 1.0).clamp(0.0, 1.0)
        };
        IntensityReading {
            opacity: display(IntensityTarget::Opacity, self.opacity_base),
            glow: display(IntensityTarget::Glow, self.glow_base),
            thickness: display(IntensityTarget::Thickness, self.thickness_base),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn gain(gain: f32) -> Expressions {
        Expressions { gain, ..Expressions::NEUTRAL }
    }

    fn to(target: IntensityTarget, weight: f32) -> IntensitySource {
        IntensitySource { target, weight }
    }

    /// Fresh, every display is in full whatever a note carries, so a project
    /// that never touches these draws as it did.
    #[test]
    fn fresh_settings_draw_every_note_in_full() {
        let quiet = Expressions { pressure: 0.0, gain: 0.0, timbre: 0.0 };
        assert_eq!(IntensitySettings::default().read(0.1, quiet), IntensityReading::FULL);
    }

    /// Each source drives the display it is routed to and no other, at its own
    /// weight, over that display's base.
    #[test]
    fn a_source_drives_only_its_own_display() {
        let settings = IntensitySettings {
            velocity: to(IntensityTarget::Opacity, 1.0),
            pressure: to(IntensityTarget::Glow, 0.5),
            glow_base: 0.25,
            ..Default::default()
        };
        let pressed = Expressions { pressure: 1.0, ..Expressions::NEUTRAL };
        let reading = settings.read(0.4, pressed);
        assert_eq!(reading, IntensityReading { opacity: 0.4, glow: 0.75, thickness: 1.0 });
    }

    /// The sources on one display ADD, and the cap is on the sum: half velocity
    /// plus +6 dB comes out louder than either, and +12 dB on a loud note
    /// stops at 1.
    #[test]
    fn sources_on_one_display_add_and_the_sum_is_capped() {
        let settings = IntensitySettings {
            velocity: to(IntensityTarget::Opacity, 1.0),
            gain: to(IntensityTarget::Opacity, 1.0),
            ..Default::default()
        };
        let opacity = |velocity, level_db: f32| {
            settings.read(velocity, gain(10.0f32.powf(level_db / 20.0))).opacity
        };
        assert!((opacity(0.5, 6.0) - 0.75).abs() < 1e-4);
        assert!((opacity(0.5, -6.0) - 0.25).abs() < 1e-4);
        assert_eq!(opacity(0.9, 12.0), 1.0);
    }

    /// Silence is -inf dB. It takes a positively weighted display to 0, a
    /// negatively weighted one to 1, and at weight 0 it is no NaN; nor is a
    /// value that is not a number.
    #[test]
    fn silence_is_bounded_whatever_the_weight() {
        let silent = gain(0.0);
        let weighted = |weight| IntensitySettings {
            gain: to(IntensityTarget::Glow, weight),
            glow_base: 1.0,
            ..Default::default()
        };
        assert_eq!(weighted(0.5).read(1.0, silent).glow, 0.0);
        assert_eq!(weighted(-0.5).read(1.0, silent).glow, 1.0);
        assert_eq!(weighted(0.0).read(1.0, silent).glow, 1.0);
        let nan = Expressions { pressure: f32::NAN, ..Expressions::NEUTRAL };
        let pressed = IntensitySettings {
            pressure: to(IntensityTarget::Thickness, 1.0),
            ..Default::default()
        };
        assert_eq!(pressed.read(1.0, nan).thickness, 1.0);
    }
}
