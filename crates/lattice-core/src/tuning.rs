//! Pitch classes and adjustable tunings for the 3/5/7 harmonic axes.
//!
//! A pitch class is stored as an integer number of microcents (1/1_000_000
//! cent) modulo one octave, following the representation from midi_lattice
//! v1: integer storage sidesteps float comparison/ordering/precision issues
//! when pitch classes are used as map keys.

/// Just tunings for primes 3, 5, and 7, in cents.
pub const THREE_JUST: f32 = 701.955;
pub const FIVE_JUST: f32 = 386.313_72;
pub const SEVEN_JUST: f32 = 968.825_9;

/// 12-TET approximations for primes 3, 5, and 7, in cents.
pub const THREE_12TET: f32 = 700.0;
pub const FIVE_12TET: f32 = 400.0;
pub const SEVEN_12TET: f32 = 1000.0;

pub const CENTS_TO_MICROCENTS: u32 = 1_000_000;
pub const OCTAVE_MICROCENTS: u32 = 1_200 * CENTS_TO_MICROCENTS;

/// The syntonic comma (81/80, ~21.506¢): the gap between four just fifths
/// and a just major third. Meantone temperaments temper it out.
pub const SYNTONIC_COMMA: f32 = 4.0 * THREE_JUST - 2.0 * 1200.0 - FIVE_JUST;

/// The meantone major third implied by a given fifth: four fifths stacked,
/// dropped two octaves. In any meantone temperament the major third equals
/// this exactly, so the prime-5 (thirds) axis stops being independent of
/// the prime-3 (fifths) axis — the defining property of meantone.
pub fn meantone_third(fifth_cents: f32) -> f32 {
    4.0 * fifth_cents - 2.0 * 1200.0
}

/// How close (in cents) a fifth/third pair must sit to the meantone
/// relationship to count as meantone. Well under the syntonic comma
/// (~21.5¢) so just intonation is correctly rejected, while 12-TET
/// (400 = 4·700 − 2400) and the meantone family are accepted.
pub const MEANTONE_TOLERANCE: f32 = 5.0;

/// Whether a fifth/third pair is (close to) a meantone temperament: the
/// major third within [`MEANTONE_TOLERANCE`] of four fifths minus two
/// octaves. Used to auto-engage meantone mode when learning a chord.
pub fn is_meantone(fifth_cents: f32, third_cents: f32) -> bool {
    (third_cents - meantone_third(fifth_cents)).abs() <= MEANTONE_TOLERANCE
}

/// A pitch class in microcents, always in `[0, OCTAVE_MICROCENTS)`.
#[derive(PartialEq, Eq, PartialOrd, Ord, Copy, Clone, Debug, Hash)]
pub struct PitchClass(u32);

impl PitchClass {
    pub fn from_cents(cents: f32) -> Self {
        PitchClass((cents.rem_euclid(1200.0) * CENTS_TO_MICROCENTS as f32).round() as u32)
    }

    pub fn from_midi_note(note: u8) -> Self {
        PitchClass(u32::from(note % 12) * 100 * CENTS_TO_MICROCENTS)
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

impl std::ops::Add for PitchClass {
    type Output = PitchClass;
    fn add(self, rhs: PitchClass) -> PitchClass {
        PitchClass((self.0 + rhs.0) % OCTAVE_MICROCENTS)
    }
}

impl std::ops::Neg for PitchClass {
    type Output = PitchClass;
    fn neg(self) -> PitchClass {
        PitchClass((OCTAVE_MICROCENTS - self.0) % OCTAVE_MICROCENTS)
    }
}

impl std::ops::Sub for PitchClass {
    type Output = PitchClass;
    fn sub(self, rhs: PitchClass) -> PitchClass {
        self + -rhs
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
    /// The 12-TET default with the three prime steps retuned to just.
    pub fn just() -> Self {
        Tuning {
            three: THREE_JUST,
            five: FIVE_JUST,
            seven: SEVEN_JUST,
            ..Tuning::default()
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

/// Result of [`learn_tuning`]: parameters that could be inferred from the
/// sounding pitch classes. `None` = nothing close enough was sounding.
#[derive(Debug, Default, Clone, Copy, PartialEq)]
pub struct LearnedTuning {
    /// Offset of C from 0 in zero-centered cents (-600..=600).
    pub c_offset: Option<f32>,
    pub three: Option<f32>,
    pub five: Option<f32>,
    pub seven: Option<f32>,
}

/// How close an interval must be to its just value to be learned (v1).
const LEARN_RANGE_CENTS: f32 = 40.0;
/// How close a pitch class must be to C to retune the C offset (v1).
const LEARN_C_RANGE_CENTS: f32 = 50.0;

/// Infer tuning parameters from a set of sounding pitch classes, ported
/// from v1's tuning-learn button:
/// - C offset: the sounding pitch class closest to C (within 50 cents).
/// - Primes 3/5/7: over every pair of pitch classes, both interval
///   directions are candidates (a fourth implies a fifth, because octaves
///   are assumed perfectly tuned); the candidate closest to the just
///   interval wins, if within 40 cents.
pub fn learn_tuning(pitch_classes: &[PitchClass]) -> LearnedTuning {
    let mut classes = pitch_classes.to_vec();
    classes.sort_unstable();
    classes.dedup();

    let mut learned = LearnedTuning::default();

    // C offset.
    let c = PitchClass::from_cents(0.0);
    let mut best_c: Option<PitchClass> = None;
    for &pc in &classes {
        if pc.distance_to(c) <= PitchClassDistance::from_cents(LEARN_C_RANGE_CENTS)
            && best_c.is_none_or(|b| pc.distance_to(c) < b.distance_to(c))
        {
            best_c = Some(pc);
        }
    }
    learned.c_offset = best_c.map(|pc| {
        let cents = pc.to_cents();
        if cents > 600.0 { cents - 1200.0 } else { cents }
    });

    // Prime intervals.
    let mut best = [
        (PitchClass::from_cents(THREE_JUST), None::<PitchClass>),
        (PitchClass::from_cents(FIVE_JUST), None),
        (PitchClass::from_cents(SEVEN_JUST), None),
    ];
    for (i, &a) in classes.iter().enumerate() {
        for &b in &classes[i + 1..] {
            for interval in [a - b, b - a] {
                for (target, best_so_far) in &mut best {
                    let diff = interval.distance_to(*target);
                    if diff <= PitchClassDistance::from_cents(LEARN_RANGE_CENTS)
                        && best_so_far.is_none_or(|b| diff < b.distance_to(*target))
                    {
                        *best_so_far = Some(interval);
                    }
                }
            }
        }
    }
    learned.three = best[0].1.map(PitchClass::to_cents);
    learned.five = best[1].1.map(PitchClass::to_cents);
    learned.seven = best[2].1.map(PitchClass::to_cents);

    learned
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
    fn quarter_comma_meantone_third_is_just() {
        // The quarter-comma fifth is the just fifth flattened by a quarter
        // of the syntonic comma; four of them (minus two octaves) land on
        // exactly the just major third.
        let fifth = THREE_JUST - SYNTONIC_COMMA / 4.0;
        assert!((meantone_third(fifth) - FIVE_JUST).abs() < 0.001);
    }

    #[test]
    fn is_meantone_accepts_meantone_rejects_just() {
        // 12-TET is a meantone (400 = 4·700 − 2400).
        assert!(is_meantone(THREE_12TET, FIVE_12TET));
        // Quarter-comma meantone.
        let quarter = THREE_JUST - SYNTONIC_COMMA / 4.0;
        assert!(is_meantone(quarter, FIVE_JUST));
        // Just intonation keeps the comma, so it is NOT meantone: the just
        // third sits a full syntonic comma below four just fifths.
        assert!(!is_meantone(THREE_JUST, FIVE_JUST));
        // A third a hair (< tolerance) off still counts.
        assert!(is_meantone(THREE_12TET, FIVE_12TET + MEANTONE_TOLERANCE - 0.1));
        assert!(!is_meantone(THREE_12TET, FIVE_12TET + MEANTONE_TOLERANCE + 0.1));
    }

    #[test]
    fn matching_respects_tolerance() {
        let tuning = Tuning { tolerance: 5.0, ..Tuning::just() };
        let node = tuning.pitch_class(LatticePos::new(1, 0, 0)); // 701.955
        assert!(tuning.matches(PitchClass::from_cents(700.0), node));
        assert!(!tuning.matches(PitchClass::from_cents(690.0), node));
    }

    #[test]
    fn learns_just_intervals_from_a_chord() {
        // C + just E + just G, all as exact pitch classes.
        let classes = [
            PitchClass::from_cents(10.0), // slightly sharp C
            PitchClass::from_cents(10.0 + FIVE_JUST),
            PitchClass::from_cents(10.0 + THREE_JUST),
        ];
        let learned = learn_tuning(&classes);
        assert_eq!(learned.c_offset, Some(10.0));
        let three = learned.three.unwrap();
        assert!((three - THREE_JUST).abs() < 0.01, "three = {three}");
        let five = learned.five.unwrap();
        assert!((five - FIVE_JUST).abs() < 0.01, "five = {five}");
        assert_eq!(learned.seven, None);
    }

    #[test]
    fn fourth_implies_fifth() {
        // C and the F below it (inverted just fifth).
        let classes = [
            PitchClass::from_cents(0.0),
            PitchClass::from_cents(1200.0 - THREE_JUST),
        ];
        let learned = learn_tuning(&classes);
        let three = learned.three.unwrap();
        assert!((three - THREE_JUST).abs() < 0.01, "three = {three}");
    }

    #[test]
    fn nothing_close_learns_nothing() {
        // A tritone-ish dyad: no prime interval within range, no C.
        let classes = [
            PitchClass::from_cents(300.0),
            PitchClass::from_cents(900.0),
        ];
        assert_eq!(learn_tuning(&classes), LearnedTuning::default());
    }

    #[test]
    fn c_offset_learns_negative_from_a_flat_c() {
        // A pitch class just below C must come out as a small negative
        // offset, not +1190.
        let classes = [PitchClass::from_cents(1190.0)];
        assert_eq!(learn_tuning(&classes).c_offset, Some(-10.0));
    }
}
