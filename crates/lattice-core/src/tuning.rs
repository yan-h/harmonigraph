//! Pitch classes and adjustable tunings for the 3/5/7 harmonic axes.
//!
//! A pitch class is stored as an integer number of microcents (1/1_000_000
//! cent) modulo one octave, following the representation from midi_lattice
//! v1: integer storage sidesteps float comparison/ordering/precision issues
//! when pitch classes are used as map keys.

/// Just tunings for primes 3, 5, and 7, in cents.
pub const THREE_JUST: f32 = 701.955001;
pub const FIVE_JUST: f32 = 386.313714;
pub const SEVEN_JUST: f32 = 968.825906;

/// 12-TET approximations for primes 3, 5, and 7, in cents.
pub const THREE_12TET: f32 = 700.0;
pub const FIVE_12TET: f32 = 400.0;
pub const SEVEN_12TET: f32 = 1000.0;

pub const CENTS_TO_MICROCENTS: u32 = 1_000_000;
pub const OCTAVE_MICROCENTS: u32 = 1_200 * CENTS_TO_MICROCENTS;

/// A pitch class in microcents, always in `[0, OCTAVE_MICROCENTS)`.
#[derive(PartialEq, Eq, PartialOrd, Ord, Copy, Clone, Debug, Hash)]
pub struct PitchClass(u32);

impl PitchClass {
    pub const fn from_microcents(microcents: u32) -> Self {
        PitchClass(microcents % OCTAVE_MICROCENTS)
    }

    pub fn from_cents(cents: f32) -> Self {
        PitchClass((cents.rem_euclid(1200.0) * CENTS_TO_MICROCENTS as f32).round() as u32)
    }

    pub fn from_midi_note(note: u8) -> Self {
        PitchClass(u32::from(note % 12) * 100 * CENTS_TO_MICROCENTS)
    }

    pub const fn to_microcents(self) -> u32 {
        self.0
    }

    pub fn to_cents(self) -> f32 {
        self.0 as f32 / CENTS_TO_MICROCENTS as f32
    }

    /// Distance to another pitch class, accounting for octave wraparound
    /// (e.g. 10¢ and 1190¢ are 20¢ apart, not 1180¢).
    pub fn distance_to(self, other: PitchClass) -> PitchClassDistance {
        let a = self.0.abs_diff(other.0);
        PitchClassDistance(a.min(OCTAVE_MICROCENTS - a))
    }
}

impl std::fmt::Display for PitchClass {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{:.2}¢", self.to_cents())
    }
}

/// An absolute distance between two pitch classes, in microcents.
#[derive(PartialEq, Eq, PartialOrd, Ord, Copy, Clone, Debug)]
pub struct PitchClassDistance(u32);

impl PitchClassDistance {
    pub fn from_cents(cents: f32) -> Self {
        PitchClassDistance((cents * CENTS_TO_MICROCENTS as f32).round() as u32)
    }

    pub fn to_cents(self) -> f32 {
        self.0 as f32 / CENTS_TO_MICROCENTS as f32
    }
}

/// Adjustable tuning of the lattice: the size in cents of each prime
/// harmonic step, plus a global offset for the origin (C) and the tolerance
/// used when matching played pitches to lattice nodes.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Tuning {
    /// Offset of the lattice origin from concert C, in cents.
    pub c_offset: f32,
    /// Size of the prime-3 step (a perfect fifth), in cents.
    pub three: f32,
    /// Size of the prime-5 step (a major third), in cents.
    pub five: f32,
    /// Size of the prime-7 step (a harmonic seventh), in cents.
    pub seven: f32,
    /// How far (in cents) a played pitch may be from a node's pitch class
    /// and still light that node up.
    pub tolerance: f32,
}

impl Default for Tuning {
    /// 12-TET defaults, matching midi_lattice v1.
    fn default() -> Self {
        Tuning {
            c_offset: 0.0,
            three: THREE_12TET,
            five: FIVE_12TET,
            seven: SEVEN_12TET,
            tolerance: 0.5,
        }
    }
}

impl Tuning {
    pub fn just() -> Self {
        Tuning {
            c_offset: 0.0,
            three: THREE_JUST,
            five: FIVE_JUST,
            seven: SEVEN_JUST,
            tolerance: 0.5,
        }
    }

    /// The pitch class of a lattice position under this tuning.
    pub fn pitch_class(&self, pos: crate::coords::LatticePos) -> PitchClass {
        PitchClass::from_cents(
            self.c_offset
                + pos.threes as f32 * self.three
                + pos.fives as f32 * self.five
                + pos.sevens as f32 * self.seven,
        )
    }

    /// Whether a played pitch class matches a node's pitch class within the
    /// configured tolerance.
    pub fn matches(&self, played: PitchClass, node: PitchClass) -> bool {
        played.distance_to(node) <= PitchClassDistance::from_cents(self.tolerance)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::coords::LatticePos;

    #[test]
    fn pitch_class_wraps_at_octave() {
        assert_eq!(PitchClass::from_cents(1250.0), PitchClass::from_cents(50.0));
        assert_eq!(PitchClass::from_cents(-100.0), PitchClass::from_cents(1100.0));
    }

    #[test]
    fn distance_wraps_around() {
        let a = PitchClass::from_cents(10.0);
        let b = PitchClass::from_cents(1190.0);
        assert_eq!(a.distance_to(b), PitchClassDistance::from_cents(20.0));
    }

    #[test]
    fn just_fifth_stack() {
        let tuning = Tuning::just();
        // Two just fifths = 1403.91¢ ≡ 203.91¢ (a just major second).
        let pc = tuning.pitch_class(LatticePos::new(2, 0, 0));
        assert!(pc.distance_to(PitchClass::from_cents(203.91)) <= PitchClassDistance::from_cents(0.01));
    }

    #[test]
    fn matching_respects_tolerance() {
        let tuning = Tuning { tolerance: 5.0, ..Tuning::just() };
        let node = tuning.pitch_class(LatticePos::new(1, 0, 0)); // 701.955
        assert!(tuning.matches(PitchClass::from_cents(700.0), node));
        assert!(!tuning.matches(PitchClass::from_cents(690.0), node));
    }
}
