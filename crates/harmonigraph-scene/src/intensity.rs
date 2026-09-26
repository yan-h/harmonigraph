//! How a note is drawn from how it is played: each of its sources — velocity
//! and the host's per-note gain, pressure and timbre — is ROUTED to one
//! display with a weight of its own, and each display adds up what is routed
//! to it.
//!
//! One target per source rather than a matrix of every source against every
//! display, so the picture says plainly what each source does. The sources on
//! one display ADD rather than multiply; the cap is on the sum, never on one
//! contribution.
//!
//! Velocity, pressure and linear gain contribute nonnegative amounts above
//! each display's base. Timbre is the signed exception: 0.5 adds nothing,
//! and its endpoints subtract or add one whole weight. Thickness starts from
//! each pane's own note width. Glow starts from [`IntensitySettings::glow_base`],
//! which is the only bloom the lattice and the roll have, and opacity from a
//! base of its own. Bases apply even with no routes enabled.
//!
//! Fresh settings leave every source off. The trail never reads any of this;
//! it records pitch classes only.

use crate::view::finite_or;
use harmonigraph_core::Expressions;

/// The top of every weight's bar, which runs up from 0. Only timbre can
/// contribute a negative amount.
pub const INTENSITY_WEIGHT_MAX: f32 = 2.0;
/// The ends of the Thickness max bar, as multiples of a pane's note width.
/// At 1 there is no room to thicken; timbre can still thin a note.
pub const THICKNESS_MAX_RANGE: std::ops::RangeInclusive<f32> = 1.0..=4.0;
/// The top of the Bloom base bar, and the most a routed note blooms.
pub const BLOOM_MAX: f32 = 2.0;
/// The least a pane's bloom pass runs at while something is routed to Glow, so
/// a note can bloom over a Bloom base of 0. Below it a note at rest blooms a
/// little less than the base alone would draw, which is too faint to see.
/// Whatever else lights the bloom's input without a share of its own (the
/// markers, the roll's lead) blooms at this floor too.
pub const BLOOM_REFERENCE_FLOOR: f32 = 0.05;

/// Which display a source drives.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub enum IntensityTarget {
    /// Drives nothing.
    #[default]
    Off,
    /// The note's opacity: the lattice's octave slices and the roll's ribbons.
    Opacity,
    /// How much the note blooms, added to the Bloom base: the halo round its
    /// lattice slices and round its roll ribbon. The lattice's node glow does
    /// not read it.
    Glow,
    /// How thick it is drawn, as a multiple of its pane's note width: the
    /// roll's ribbons, across pitch about their center line, and the lattice's
    /// lit slices, out from the band's inner edge.
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
    /// What the source's amount is multiplied by before its display adds it,
    /// 0 to [`INTENSITY_WEIGHT_MAX`]. Timbre's amount spans -1 to 1.
    pub weight: f32,
}

impl Default for IntensitySource {
    fn default() -> Self {
        Self { target: IntensityTarget::Off, weight: 1.0 }
    }
}

/// Every source's route, the bloom and opacity bases, and the thickness's
/// ceiling.
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
    /// How much a note blooms before contributions, in × of the reference halo: the whole
    /// of the lattice's and the roll's bloom, 0 for none. The sources routed
    /// to Glow add to it.
    ///
    /// One value for both panes, where each used to have a bar of its own
    /// (the lattice's `bloom_strength` and the roll's `note_glow`). The fresh
    /// value is the lattice's.
    pub glow_base: f32,
    /// The opacity base, 0..1, before weighted contributions. Applies with or
    /// without routes; only timbre can take it below this base.
    pub opacity_rest: f32,
    /// The widest a note can be drawn, as a multiple of its pane's note width.
    /// Read only while something is routed to Thickness.
    pub thickness_max: f32,
}

impl Default for IntensitySettings {
    fn default() -> Self {
        Self {
            velocity: IntensitySource::default(),
            gain: IntensitySource::default(),
            pressure: IntensitySource::default(),
            timbre: IntensitySource::default(),
            glow_base: 0.633_927_7,
            opacity_rest: 1.0,
            thickness_max: 2.0,
        }
    }
}

/// What a note's sources come to on each display.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct IntensityReading {
    /// 0 to 1, with 1 as the note drawn in full.
    pub opacity: f32,
    /// How far the note's bloom stands off the Bloom base, in its × units: 0 at
    /// neutral timbre and zero unipolar sources. [`IntensitySettings::bloom`] adds the base and
    /// bounds the two.
    pub glow: f32,
    /// The note's width as a multiple of its pane's note width, from 0 to
    /// [`IntensitySettings::thickness_max`]: 1 before contributions.
    pub thickness: f32,
}

impl IntensityReading {
    /// The default bases, also used for unlit slots.
    pub const REST: Self = Self { opacity: 1.0, glow: 0.0, thickness: 1.0 };

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
            source.weight = bar(source.weight, fresh.velocity.weight);
        }
        self.glow_base = finite_or(self.glow_base, fresh.glow_base).clamp(0.0, BLOOM_MAX);
        self.opacity_rest = finite_or(self.opacity_rest, fresh.opacity_rest).clamp(0.0, 1.0);
        self.thickness_max = finite_or(self.thickness_max, fresh.thickness_max)
            .clamp(*THICKNESS_MAX_RANGE.start(), *THICKNESS_MAX_RANGE.end());
        self
    }

    /// Whether any source is routed to `target`.
    pub fn routes_to(&self, target: IntensityTarget) -> bool {
        target != IntensityTarget::Off
            && [self.velocity, self.gain, self.pressure, self.timbre]
                .iter()
                .any(|source| source.target == target)
    }

    /// The Bloom base, whatever a shell hands over: finite and on its bar.
    fn glow_at_rest(&self) -> f32 {
        finite_or(self.glow_base, 0.0).clamp(0.0, BLOOM_MAX)
    }

    /// How much a note reading `reading` blooms, in the Bloom base's × units:
    /// the base itself at rest, and never below 0 nor past [`BLOOM_MAX`].
    pub fn bloom(&self, reading: IntensityReading) -> f32 {
        (self.glow_at_rest() + reading.glow).clamp(0.0, BLOOM_MAX)
    }

    /// The strength both panes' bloom passes run at: the Bloom base, except
    /// that while something is routed to Glow it never drops below
    /// [`BLOOM_REFERENCE_FLOOR`], so a note can bloom over a base of 0. Each
    /// note's share of it is [`bloom_share`](Self::bloom_share).
    pub fn bloom_reference(&self) -> f32 {
        let base = self.glow_at_rest();
        if self.routes_to(IntensityTarget::Glow) {
            base.max(BLOOM_REFERENCE_FLOOR)
        } else {
            base
        }
    }

    /// How much of a note's ink its pane's bloom pass takes, against
    /// [`bloom_reference`](Self::bloom_reference), for a note blooming at
    /// `bloom` ([`bloom`](Self::bloom)): 1 at rest wherever the base is at or
    /// above the floor, and past 1 for a note blooming over the base. 1 where
    /// there is no bloom at all, since nothing reads it then.
    pub fn bloom_share(&self, bloom: f32) -> f32 {
        let reference = self.bloom_reference();
        if reference > 0.0 {
            bloom / reference
        } else {
            1.0
        }
    }

    /// What one note, played at `velocity` with its expressions standing at
    /// `expressions`, comes to on every display.
    pub fn read(&self, velocity: f32, expressions: Expressions) -> IntensityReading {
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
                .filter(|(source, _)| source.target == target && source.weight != 0.0)
                .map(|(source, value)| source.weight * value)
                .sum::<f32>()
                .clamp(-f32::MAX, f32::MAX)
        };
        IntensityReading {
            opacity: (self.opacity_rest + sum(IntensityTarget::Opacity)).clamp(0.0, 1.0),
            glow: sum(IntensityTarget::Glow),
            thickness: (1.0 + sum(IntensityTarget::Thickness))
                .clamp(0.0, self.thickness_max.max(1.0)),
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
            IntensitySettings { opacity_rest: 0.3, glow_base: 0.25, ..Default::default() };
        for target in IntensityTarget::ALL {
            settings.velocity = to(target, 0.0);
            let reading = settings.read(1.0, Expressions::NEUTRAL);
            assert_eq!(reading.opacity, 0.3);
            assert_eq!(settings.bloom(reading), 0.25);
            assert_eq!(reading.thickness, 1.0);
        }
    }

    /// Each source drives only its target, adding above the base except for
    /// timbre, whose lower half can subtract.
    #[test]
    fn a_source_drives_only_its_own_display_from_base() {
        let settings = IntensitySettings {
            velocity: to(IntensityTarget::Opacity, 1.0),
            pressure: to(IntensityTarget::Glow, 0.5),
            timbre: to(IntensityTarget::Thickness, 1.0),
            glow_base: 0.25,
            opacity_rest: 0.2,
            ..Default::default()
        };
        let pressed = Expressions { pressure: 1.0, timbre: 0.25, ..Expressions::NEUTRAL };
        let reading = settings.read(0.4, pressed);
        assert!((reading.opacity - 0.6).abs() < 1e-6, "{reading:?}");
        assert_eq!((reading.glow, reading.thickness), (0.5, 0.5));
        assert_eq!(settings.bloom(reading), 0.75);
        assert_eq!(
            settings.read(0.0, Expressions::NEUTRAL),
            IntensityReading { opacity: 0.2, glow: 0.0, thickness: 1.0 },
        );
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
        for target in [IntensityTarget::Opacity, IntensityTarget::Glow, IntensityTarget::Thickness]
        {
            let mut settings =
                IntensitySettings { timbre: to(target, 1.0), glow_base: 1.0, ..Default::default() };
            for (timbre, expected) in
                [(0.0, 0.0_f32), (0.25, 0.5), (0.5, 1.0), (0.75, 1.5), (1.0, 2.0)]
            {
                let reading = settings.read(1.0, Expressions { timbre, ..Expressions::NEUTRAL });
                match target {
                    IntensityTarget::Opacity => assert_eq!(reading.opacity, expected.min(1.0)),
                    IntensityTarget::Glow => assert_eq!(settings.bloom(reading), expected),
                    IntensityTarget::Thickness => assert_eq!(reading.thickness, expected),
                    IntensityTarget::Off => unreachable!(),
                }
            }
            // Opacity needs a lower base to leave room for the full +1.
            settings.opacity_rest = 0.0;
            if target == IntensityTarget::Opacity {
                assert_eq!(
                    settings.read(1.0, Expressions { timbre: 1.0, ..Expressions::NEUTRAL }).opacity,
                    1.0
                );
                settings.timbre.weight = 0.5;
                assert_eq!(
                    settings.read(1.0, Expressions { timbre: 1.0, ..Expressions::NEUTRAL }).opacity,
                    0.5
                );
            }
        }
    }

    /// The bloom pass runs at the base, so a note at rest takes the whole of
    /// its ink whether or not anything is routed to Glow; a routed note blooms
    /// over a base of 0 against the floor instead.
    #[test]
    fn a_notes_bloom_share_is_whole_at_rest_and_past_whole_over_the_base() {
        let base = |glow_base, routed: bool| IntensitySettings {
            pressure: to(if routed { IntensityTarget::Glow } else { IntensityTarget::Off }, 1.0),
            glow_base,
            ..Default::default()
        };
        let rest = IntensityReading::REST;
        for settings in [base(0.8, false), base(0.8, true)] {
            assert_eq!(settings.bloom_reference(), 0.8);
            assert_eq!(settings.bloom_share(settings.bloom(rest)), 1.0);
        }
        let pressed = Expressions { pressure: 0.4, ..Expressions::NEUTRAL };
        let routed = base(0.8, true);
        assert!((routed.bloom_share(routed.bloom(routed.read(1.0, pressed))) - 1.5).abs() < 1e-6);
        let dark = base(0.0, true);
        assert_eq!(dark.bloom_reference(), BLOOM_REFERENCE_FLOOR);
        let share = dark.bloom_share(dark.bloom(dark.read(1.0, pressed)));
        assert!((share - 0.4 / BLOOM_REFERENCE_FLOOR).abs() < 1e-4);
        assert_eq!(base(0.0, false).bloom_reference(), 0.0, "no pass");
        assert_eq!(routed.bloom(IntensityReading { glow: 5.0, ..rest }), BLOOM_MAX);
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
        let glowing = IntensitySettings {
            gain: to(IntensityTarget::Glow, 0.5),
            glow_base: 1.0,
            ..Default::default()
        };
        assert_eq!(glowing.bloom(glowing.read(1.0, silent)), 1.0);
        let nan = Expressions { pressure: f32::NAN, ..Expressions::NEUTRAL };
        let pressed = IntensitySettings {
            pressure: to(IntensityTarget::Thickness, 1.0),
            velocity: to(IntensityTarget::Thickness, 0.5),
            ..Default::default()
        };
        assert_eq!(pressed.read(1.0, nan).thickness, 1.5);
    }
}
