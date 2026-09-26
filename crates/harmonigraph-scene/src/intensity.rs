//! How a note is drawn from how it is played. Velocity, gain, pressure and
//! timbre each have independent weights for opacity and thickness.
//! Contributions add before each display's final cap.
//!
//! Velocity, pressure and linear gain contribute nonnegative amounts above
//! each display's base. Timbre is the signed exception: 0.5 adds nothing,
//! and its endpoints subtract or add one whole weight. Thickness is measured
//! in multiples of each pane's reference note width. Opacity has a base of
//! its own. Bases apply even with no routes enabled.
//!
//! Fresh settings leave every source off. The trail never reads any of this;
//! it records pitch classes only.

use crate::view::finite_or;
use harmonigraph_core::Expressions;

/// The top of every weight's bar, which runs up from 0. Only timbre can
/// contribute a negative amount.
pub const INTENSITY_WEIGHT_MAX: f32 = 2.0;
/// The ends of the Thickness max bar, as multiples of a pane's note width.
/// The base can be anywhere between zero and this ceiling.
pub const THICKNESS_MAX_RANGE: std::ops::RangeInclusive<f32> = 1.0..=4.0;
/// Which display a source drives.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum IntensityTarget {
    /// The lattice's octave slices and the roll's ribbons.
    Opacity,
    /// Width in multiples of each pane's reference note width.
    Thickness,
}

impl IntensityTarget {
    pub const ALL: [Self; 2] = [Self::Opacity, Self::Thickness];
}

/// A source's independent target weights. None disables that mapping;
/// Some(0) keeps it enabled at zero strength. Weights range from 0 to
/// [`INTENSITY_WEIGHT_MAX`]; only timbre can contribute a negative amount.
#[derive(Clone, Copy, Debug, Default, PartialEq, serde::Serialize, serde::Deserialize)]
#[serde(default)]
pub struct IntensitySource {
    pub opacity: Option<f32>,
    pub thickness: Option<f32>,
}

impl IntensitySource {
    pub fn weight(&self, target: IntensityTarget) -> Option<f32> {
        match target {
            IntensityTarget::Opacity => self.opacity,
            IntensityTarget::Thickness => self.thickness,
        }
    }

    pub fn weight_mut(&mut self, target: IntensityTarget) -> &mut Option<f32> {
        match target {
            IntensityTarget::Opacity => &mut self.opacity,
            IntensityTarget::Thickness => &mut self.thickness,
        }
    }
}

/// Every source's route, opacity and thickness bases, and the thickness ceiling.
#[derive(Clone, Copy, Debug, PartialEq, serde::Serialize, serde::Deserialize)]
#[serde(default)]
pub struct IntensitySettings {
    /// The note-on velocity, from 0 to 1: full velocity adds the whole weight.
    pub velocity: IntensitySource,
    /// The note's linear gain: silence adds nothing, unity adds the whole
    /// weight, and boosts can add more. A cut never subtracts from the base.
    pub gain: IntensitySource,
    /// Pressure, 0 unpressed to 1.
    pub pressure: IntensitySource,
    /// Timbre remapped from 0..1 to -1..1, centered on the untouched 0.5.
    pub timbre: IntensitySource,
    /// The opacity base, 0..1, before weighted contributions. Applies with or
    /// without routes; only timbre can take it below this base.
    pub opacity_rest: f32,
    /// Starting thickness, as a multiple of each pane's reference note width.
    /// Weighted contributions use the same units. Bounded by `thickness_max`.
    pub thickness_base: f32,
    /// The widest a note can be drawn, as a multiple of its pane's note width.
    pub thickness_max: f32,
}

impl Default for IntensitySettings {
    fn default() -> Self {
        Self {
            velocity: IntensitySource::default(),
            gain: IntensitySource::default(),
            pressure: IntensitySource::default(),
            timbre: IntensitySource::default(),
            opacity_rest: 1.0,
            thickness_base: 1.0,
            thickness_max: 2.0,
        }
    }
}

/// What a note's sources come to on each display.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct IntensityReading {
    /// 0 to 1, with 1 as the note drawn in full.
    pub opacity: f32,
    /// The note's width as a multiple of its pane's note width, from 0 to
    /// [`IntensitySettings::thickness_max`]: `thickness_base` before contributions.
    pub thickness: f32,
}

/// Possible target values before clipping. Gain is sampled at unity for the
/// upper end; `gain_boosts` says higher host gain can extend it farther.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct IntensityReach {
    pub min: f32,
    pub max: f32,
    pub gain_boosts: bool,
}

impl IntensityReading {
    /// The default bases, also used for unlit slots.
    pub const REST: Self = Self { opacity: 1.0, thickness: 1.0 };

    /// The larger of the two on each display.
    pub fn max(self, other: Self) -> Self {
        Self {
            opacity: self.opacity.max(other.opacity),
            thickness: self.thickness.max(other.thickness),
        }
    }
}

impl Default for IntensityReading {
    fn default() -> Self {
        Self::REST
    }
}

impl IntensitySettings {
    /// Every value finite and on its bar, falling back to the fresh value.
    pub fn sanitized(mut self) -> Self {
        let fresh = Self::default();
        let bar =
            |value: f32, fallback: f32| finite_or(value, fallback).clamp(0.0, INTENSITY_WEIGHT_MAX);
        for source in [&mut self.velocity, &mut self.gain, &mut self.pressure, &mut self.timbre] {
            for target in IntensityTarget::ALL {
                if let Some(weight) = source.weight_mut(target) {
                    *weight = bar(*weight, 1.0);
                }
            }
        }
        self.opacity_rest = finite_or(self.opacity_rest, fresh.opacity_rest).clamp(0.0, 1.0);
        self.thickness_max = finite_or(self.thickness_max, fresh.thickness_max)
            .clamp(*THICKNESS_MAX_RANGE.start(), *THICKNESS_MAX_RANGE.end());
        self.thickness_base =
            finite_or(self.thickness_base, fresh.thickness_base).clamp(0.0, self.thickness_max);
        self
    }

    /// Whether any source is routed to `target`.
    pub fn routes_to(&self, target: IntensityTarget) -> bool {
        [self.velocity, self.gain, self.pressure, self.timbre]
            .iter()
            .any(|source| source.weight(target).is_some())
    }

    /// What one note, played at `velocity` with its expressions standing at
    /// `expressions`, comes to on every display.
    pub fn read(&self, velocity: f32, expressions: Expressions) -> IntensityReading {
        let raw = self.unclamped(velocity, expressions);
        IntensityReading {
            opacity: raw.opacity.clamp(0.0, 1.0),
            thickness: raw.thickness.clamp(0.0, self.thickness_max.max(1.0)),
        }
    }

    /// The same calculation the picture uses, before its target caps. The UI
    /// needs the uncapped values to distinguish clipping from reaching an end.
    pub fn reach(&self, target: IntensityTarget) -> IntensityReach {
        let value = |reading: IntensityReading| match target {
            IntensityTarget::Opacity => reading.opacity,
            IntensityTarget::Thickness => reading.thickness,
        };
        IntensityReach {
            min: value(self.unclamped(0.0, Expressions { pressure: 0.0, gain: 0.0, timbre: 0.0 })),
            max: value(self.unclamped(1.0, Expressions { pressure: 1.0, gain: 1.0, timbre: 1.0 })),
            gain_boosts: self.gain.weight(target).is_some_and(|weight| weight > 0.0),
        }
    }

    fn unclamped(&self, velocity: f32, expressions: Expressions) -> IntensityReading {
        let sources = [
            (self.velocity, finite_or(velocity, 0.0).clamp(0.0, 1.0)),
            (self.gain, finite_or(expressions.gain, 0.0).max(0.0)),
            (self.pressure, finite_or(expressions.pressure, 0.0).clamp(0.0, 1.0)),
            (self.timbre, 2.0 * finite_or(expressions.timbre, 0.5).clamp(0.0, 1.0) - 1.0),
        ];
        // Invalid inputs contribute nothing without erasing other sources.
        // Saturate overflow from extreme finite gain before returning a
        // reading; each target then clamps the base plus the whole sum.
        let sum = |target: IntensityTarget| {
            sources
                .iter()
                .filter_map(|(source, value)| source.weight(target).map(|weight| (weight, value)))
                .filter(|(weight, _)| *weight != 0.0)
                .map(|(weight, value)| weight * value)
                .sum::<f32>()
                .clamp(-f32::MAX, f32::MAX)
        };
        IntensityReading {
            opacity: self.opacity_rest + sum(IntensityTarget::Opacity),
            thickness: self.thickness_base + sum(IntensityTarget::Thickness),
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
        let mut source = IntensitySource::default();
        *source.weight_mut(target) = Some(weight);
        source
    }

    /// Fresh, every display is at rest whatever a note carries, so a project
    /// that never touches these draws as it did.
    #[test]
    fn fresh_settings_draw_every_note_at_rest() {
        let quiet = Expressions { pressure: 0.0, gain: 0.0, timbre: 0.0 };
        assert_eq!(IntensitySettings::default().read(0.1, quiet), IntensityReading::REST);
    }

    #[test]
    fn bases_apply_without_routes_and_with_zero_weights() {
        let mut settings =
            IntensitySettings { opacity_rest: 0.3, thickness_base: 0.4, ..Default::default() };
        for target in IntensityTarget::ALL {
            settings.velocity = to(target, 0.0);
            let reading = settings.read(1.0, Expressions::NEUTRAL);
            assert_eq!(reading.opacity, 0.3);
            assert_eq!(reading.thickness, 0.4);
        }
    }

    #[test]
    fn one_source_drives_multiple_targets_with_independent_weights() {
        let mut settings = IntensitySettings {
            pressure: IntensitySource { opacity: Some(0.4), thickness: Some(1.2) },
            opacity_rest: 0.1,
            thickness_base: 0.5,
            ..Default::default()
        };
        let expression = Expressions { pressure: 0.5, ..Expressions::NEUTRAL };
        let reading = settings.read(0.0, expression);
        assert!((reading.opacity - 0.3).abs() < 1e-6);
        assert!((reading.thickness - 1.1).abs() < 1e-6);
        settings.pressure.opacity = None;
        let removed = settings.read(0.0, expression);
        assert_eq!(removed.opacity, settings.opacity_rest);
        assert_eq!(removed.thickness, reading.thickness);
    }

    /// A source affects only its enabled targets, adding above the base except
    /// for timbre, whose lower half can subtract.
    #[test]
    fn a_source_drives_only_its_own_display_from_base() {
        let settings = IntensitySettings {
            velocity: to(IntensityTarget::Opacity, 1.0),
            timbre: to(IntensityTarget::Thickness, 1.0),
            opacity_rest: 0.2,
            ..Default::default()
        };
        let pressed = Expressions { pressure: 1.0, timbre: 0.25, ..Expressions::NEUTRAL };
        let reading = settings.read(0.4, pressed);
        assert!((reading.opacity - 0.6).abs() < 1e-6, "{reading:?}");
        assert_eq!(reading.thickness, 0.5);
        assert_eq!(
            settings.read(0.0, Expressions::NEUTRAL),
            IntensityReading { opacity: 0.2, thickness: 1.0 },
        );
    }

    #[test]
    fn reach_keeps_clipping_and_gain_headroom_visible() {
        let settings = IntensitySettings {
            opacity_rest: 0.25,
            velocity: to(IntensityTarget::Opacity, 0.5),
            pressure: to(IntensityTarget::Opacity, 0.25),
            timbre: to(IntensityTarget::Opacity, 0.5),
            gain: to(IntensityTarget::Thickness, 0.25),
            thickness_base: 0.5,
            ..Default::default()
        };
        assert_eq!(
            settings.reach(IntensityTarget::Opacity),
            IntensityReach { min: -0.25, max: 1.5, gain_boosts: false }
        );
        assert_eq!(
            settings
                .read(1.0, Expressions { pressure: 1.0, timbre: 1.0, ..Expressions::NEUTRAL })
                .opacity,
            1.0
        );
        assert_eq!(
            settings.reach(IntensityTarget::Thickness),
            IntensityReach { min: 0.5, max: 0.75, gain_boosts: true }
        );
        assert_eq!(settings.read(1.0, gain(2.0)).thickness, 1.0);
        let repaired =
            IntensitySettings { thickness_base: 3.0, thickness_max: 1.5, ..settings }.sanitized();
        assert_eq!(repaired.thickness_base, 1.5);
    }

    /// The cap is on the sum: negative timbre can offset additions even when
    /// those additions alone would exceed the ceiling.
    #[test]
    fn sources_on_one_display_add_and_the_sum_is_capped() {
        let settings = IntensitySettings {
            velocity: to(IntensityTarget::Thickness, 1.0),
            gain: to(IntensityTarget::Thickness, 1.0),
            timbre: to(IntensityTarget::Thickness, 1.0),
            thickness_max: 1.5,
            ..Default::default()
        };
        let thickness = |velocity, gain, timbre| {
            settings.read(velocity, Expressions { gain, timbre, ..Expressions::NEUTRAL }).thickness
        };
        assert_eq!(thickness(0.5, 0.25, 0.0), 0.75);
        assert_eq!(thickness(1.0, 0.25, 0.0), 1.25);
        assert_eq!(thickness(1.0, 1.0, 0.5), 1.5, "held at Thickness max");
        assert_eq!(thickness(0.0, 0.0, 0.0), 0.0);
    }

    #[test]
    fn timbre_endpoints_add_and_subtract_the_whole_weight() {
        for target in IntensityTarget::ALL {
            let mut settings = IntensitySettings { timbre: to(target, 1.0), ..Default::default() };
            for (timbre, expected) in
                [(0.0, 0.0_f32), (0.25, 0.5), (0.5, 1.0), (0.75, 1.5), (1.0, 2.0)]
            {
                let reading = settings.read(1.0, Expressions { timbre, ..Expressions::NEUTRAL });
                match target {
                    IntensityTarget::Opacity => assert_eq!(reading.opacity, expected.min(1.0)),
                    IntensityTarget::Thickness => assert_eq!(reading.thickness, expected),
                }
            }
            // Opacity needs a lower base to leave room for the full +1.
            settings.opacity_rest = 0.0;
            if target == IntensityTarget::Opacity {
                assert_eq!(
                    settings.read(1.0, Expressions { timbre: 1.0, ..Expressions::NEUTRAL }).opacity,
                    1.0
                );
                *settings.timbre.weight_mut(target) = Some(0.5);
                assert_eq!(
                    settings.read(1.0, Expressions { timbre: 1.0, ..Expressions::NEUTRAL }).opacity,
                    0.5
                );
            }
        }
    }

    /// Gain cuts contribute less, never below the base. Unity adds the whole
    /// weight; zero or invalid input cannot erase the other contributions.
    #[test]
    fn gain_is_nonnegative_and_invalid_inputs_contribute_nothing() {
        let silent = gain(0.0);
        let weighted = |weight| IntensitySettings {
            gain: to(IntensityTarget::Opacity, weight),
            opacity_rest: 0.2,
            ..Default::default()
        };
        assert_eq!(weighted(0.5).read(1.0, silent).opacity, 0.2);
        assert_eq!(weighted(0.0).read(1.0, silent).opacity, 0.2);
        assert!((weighted(0.5).read(1.0, gain(0.5)).opacity - 0.45).abs() < 1e-6);
        assert_eq!(weighted(0.5).read(1.0, gain(1.0)).opacity, 0.7);
        assert_eq!(weighted(0.5).read(1.0, gain(2.0)).opacity, 1.0);
        assert_eq!(weighted(2.0).read(1.0, gain(f32::MAX)).opacity, 1.0);
        let nan = Expressions { pressure: f32::NAN, ..Expressions::NEUTRAL };
        let pressed = IntensitySettings {
            pressure: to(IntensityTarget::Thickness, 1.0),
            velocity: to(IntensityTarget::Thickness, 0.5),
            ..Default::default()
        };
        assert_eq!(pressed.read(1.0, nan).thickness, 1.5);
    }
}
