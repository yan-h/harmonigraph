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
//! Every source is measured from where it RESTS — full velocity, no pressure,
//! timbre at 0.5, gain at unity — so a note at rest draws exactly as its pane's
//! own controls say, and a source pushes it one way or the other from there.
//! The thickness therefore starts from each pane's own note width. The glow
//! starts from [`IntensitySettings::glow_base`], which is the only bloom the
//! lattice and the roll have, and the opacity from a base of its own.
//!
//! A display nothing is routed to stays at rest, so fresh settings draw every
//! project as it was before these existed. The trail never reads any of this;
//! it records pitch classes only.

use crate::view::finite_or;
use harmonigraph_core::Expressions;

/// The top of every weight's bar and of the opacity base's, which run up from
/// 0. No weight is negative: a source only ever pushes its display the way it
/// moves from rest.
pub const INTENSITY_WEIGHT_MAX: f32 = 2.0;
/// The narrowest and widest the gain range can be set to, in dB.
pub const GAIN_RANGE_MIN: f32 = 3.0;
/// See [`GAIN_RANGE_MIN`].
pub const GAIN_RANGE_MAX: f32 = 60.0;
/// The ends of the Thickness max bar, as multiples of a pane's note width. 1
/// lets a source only thin a note.
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
    /// What the source's distance from rest is multiplied by before its
    /// display adds it, 0 to [`INTENSITY_WEIGHT_MAX`].
    pub weight: f32,
}

impl Default for IntensitySource {
    fn default() -> Self {
        Self { target: IntensityTarget::Off, weight: 1.0 }
    }
}

/// Every source's route, the bloom and opacity at rest, and the thickness's
/// ceiling.
#[derive(Clone, Copy, Debug, PartialEq, serde::Serialize, serde::Deserialize)]
#[serde(default)]
pub struct IntensitySettings {
    /// The note-on velocity, less 1: 0 for a note played as hard as it goes,
    /// -1 for one played at nothing.
    pub velocity: IntensitySource,
    /// The note's gain, in dB off unity divided by
    /// [`gain_range`](Self::gain_range): 0 at unity, so it adds a boost and
    /// takes away a cut.
    pub gain: IntensitySource,
    /// Pressure, 0 unpressed to 1.
    pub pressure: IntensitySource,
    /// Timbre less 0.5, where an untouched lane sits: -0.5 to 0.5.
    pub timbre: IntensitySource,
    /// How many dB of gain make one weight's worth: at 24, +12 dB adds half
    /// the gain's weight and -24 dB takes a whole one away.
    pub gain_range: f32,
    /// How much a note at rest blooms, in × of the reference halo: the whole
    /// of the lattice's and the roll's bloom, 0 for none. The sources routed
    /// to Glow add to it.
    ///
    /// One value for both panes, where each used to have a bar of its own
    /// (the lattice's `bloom_strength` and the roll's `note_glow`). The fresh
    /// value is the lattice's.
    pub glow_base: f32,
    /// Where a note at rest sits on the opacity display, before the sources
    /// routed to it add in. Read only while something is routed there.
    ///
    /// Named apart from the base it replaced, which read its sources from 0
    /// rather than from rest: a saved value of that shape would hide every
    /// note here.
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
            gain_range: 24.0,
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
    /// rest. Unbounded here; [`IntensitySettings::bloom`] adds the base and
    /// bounds the two.
    pub glow: f32,
    /// The note's width as a multiple of its pane's note width, from 0 to
    /// [`IntensitySettings::thickness_max`]: 1 at rest.
    pub thickness: f32,
}

impl IntensityReading {
    /// Every display at rest, which is what a display nothing drives reads.
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
        self.gain_range =
            finite_or(self.gain_range, fresh.gain_range).clamp(GAIN_RANGE_MIN, GAIN_RANGE_MAX);
        self.glow_base = finite_or(self.glow_base, fresh.glow_base).clamp(0.0, BLOOM_MAX);
        self.opacity_rest = bar(self.opacity_rest, fresh.opacity_rest);
        self.thickness_max = finite_or(self.thickness_max, fresh.thickness_max)
            .clamp(*THICKNESS_MAX_RANGE.start(), *THICKNESS_MAX_RANGE.end());
        self
    }

    /// Whether any source drives `target`. A display none does reads at rest.
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
        // Silence is -inf dB. Bounded far past any range the bar offers, so it
        // still pulls a sum down past 0 at the smallest weight while staying
        // finite: a -inf meeting a +inf is a NaN.
        const DB_BOUND: f32 = 1.0e4;
        let db = (20.0 * expressions.gain.log10()).clamp(-DB_BOUND, DB_BOUND);
        let sources = [
            (self.velocity, velocity - 1.0),
            (self.gain, db / self.gain_range.max(GAIN_RANGE_MIN)),
            (self.pressure, expressions.pressure),
            (self.timbre, expressions.timbre - 0.5),
        ];
        // What the sources routed to `target` add up to, or None where none
        // is. A host's value is not guaranteed to be a number, and `clamp`
        // passes a NaN through; such a sum reads 0, so the note draws at rest
        // rather than not at all.
        let sum = |target: IntensityTarget| {
            self.routes_to(target).then(|| {
                // A zero weight is skipped rather than multiplied, which is
                // what keeps a silent note's bounded dB out of a sum it has no
                // part in.
                let sum = sources
                    .iter()
                    .filter(|(source, _)| source.target == target && source.weight != 0.0)
                    .map(|(source, value)| source.weight * value)
                    .sum::<f32>();
                finite_or(sum, 0.0)
            })
        };
        let rest = IntensityReading::REST;
        IntensityReading {
            opacity: sum(IntensityTarget::Opacity)
                .map_or(rest.opacity, |sum| (self.opacity_rest + sum).clamp(0.0, 1.0)),
            glow: sum(IntensityTarget::Glow).unwrap_or(rest.glow),
            thickness: sum(IntensityTarget::Thickness).map_or(rest.thickness, |sum| {
                (rest.thickness + sum).clamp(0.0, self.thickness_max.max(0.0))
            }),
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

    /// Each source drives the display it is routed to and no other, at its own
    /// weight, measured from where it rests: a soft note is dimmer than one
    /// played in full, and pressure adds bloom over the base.
    #[test]
    fn a_source_drives_only_its_own_display_from_rest() {
        let settings = IntensitySettings {
            velocity: to(IntensityTarget::Opacity, 1.0),
            pressure: to(IntensityTarget::Glow, 0.5),
            timbre: to(IntensityTarget::Thickness, 1.0),
            glow_base: 0.25,
            ..Default::default()
        };
        let pressed = Expressions { pressure: 1.0, timbre: 0.25, ..Expressions::NEUTRAL };
        let reading = settings.read(0.4, pressed);
        assert!((reading.opacity - 0.4).abs() < 1e-6, "{reading:?}");
        assert_eq!((reading.glow, reading.thickness), (0.5, 0.75));
        assert_eq!(settings.bloom(reading), 0.75);
        let at_rest = settings.read(1.0, Expressions::NEUTRAL);
        assert_eq!(at_rest, IntensityReading::REST, "a note at rest draws as the panes say");
    }

    /// The sources on one display ADD, and the cap is on the sum: a gain boost
    /// makes up for a soft velocity, and a cut on a loud note only goes so far
    /// as the display's floor.
    #[test]
    fn sources_on_one_display_add_and_the_sum_is_capped() {
        let settings = IntensitySettings {
            velocity: to(IntensityTarget::Thickness, 1.0),
            gain: to(IntensityTarget::Thickness, 1.0),
            thickness_max: 1.5,
            ..Default::default()
        };
        let thickness = |velocity, level_db: f32| {
            settings.read(velocity, gain(10.0f32.powf(level_db / 20.0))).thickness
        };
        assert!((thickness(0.5, 6.0) - 0.75).abs() < 1e-4);
        assert!((thickness(1.0, 6.0) - 1.25).abs() < 1e-4);
        assert_eq!(thickness(1.0, 24.0), 1.5, "held at Thickness max");
        assert_eq!(thickness(0.2, -24.0), 0.0);
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

    /// Silence is -inf dB. It takes a weighted display to its floor, and at
    /// weight 0 it is no NaN; nor is a value that is not a number.
    #[test]
    fn silence_is_bounded_whatever_the_weight() {
        let silent = gain(0.0);
        let weighted = |weight| IntensitySettings {
            gain: to(IntensityTarget::Opacity, weight),
            ..Default::default()
        };
        assert_eq!(weighted(0.5).read(1.0, silent).opacity, 0.0);
        assert_eq!(weighted(0.0).read(1.0, silent).opacity, 1.0);
        let glowing = IntensitySettings {
            gain: to(IntensityTarget::Glow, 0.5),
            glow_base: 1.0,
            ..Default::default()
        };
        assert_eq!(glowing.bloom(glowing.read(1.0, silent)), 0.0);
        let nan = Expressions { pressure: f32::NAN, ..Expressions::NEUTRAL };
        let pressed = IntensitySettings {
            pressure: to(IntensityTarget::Thickness, 1.0),
            ..Default::default()
        };
        assert_eq!(pressed.read(1.0, nan).thickness, 1.0);
    }
}
