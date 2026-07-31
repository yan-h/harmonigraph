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

/// Quantize a value in cents to integer microcents. This is the single
/// f32→integer boundary for tuning: the host params and MIDI arrive as f32
/// cents and are rounded here, once, after which all lattice pitch math is
/// exact integer arithmetic. Done in f64 so the multiply itself doesn't
/// lose bits for large cent values.
pub fn microcents(cents: f32) -> i32 {
    (f64::from(cents) * f64::from(CENTS_TO_MICROCENTS)).round() as i32
}

/// Convert integer microcents back to cents, the inverse of [`microcents`],
/// for the display / host-param boundary. Takes any integer that widens
/// losslessly to `i64` (the `u32` pitch-class storage and the `i32` tuning
/// steps both do); every microcent value in the lattice fits in `i32`, so
/// widening before the `as f32` cast is bit-identical to casting directly.
fn cents(microcents: impl Into<i64>) -> f32 {
    microcents.into() as f32 / CENTS_TO_MICROCENTS as f32
}

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
/// relationship to count as meantone.
///
/// A cent is a hair either side of "on it", not a family window: the pairs
/// this is meant to catch — 12-TET (400 = 4·700 − 2400), quarter-comma, any
/// preset or learned chord that IS a meantone — land on the identity to
/// within rounding, so the tolerance only has to cover the f32/microcent
/// slop and a value typed to two decimals. Anything wider starts claiming
/// tunings that were deliberately set a little off it, and tempering the
/// comma out of a tuning that keeps 2¢ of it is a change to the picture
/// nobody asked for.
///
/// It is also the width of the third bar's magnet (see the UI's
/// `meantone_third_bar`), so the same number says how far a drag has to pull
/// to release the mode. Narrow is the right side to err on there too: the
/// bar is 80¢ end to end, so a wide magnet swallows a real part of the range.
pub const MEANTONE_TOLERANCE: f32 = 1.0;

/// Whether a fifth/third pair is (close to) a meantone temperament: the
/// major third within [`MEANTONE_TOLERANCE`] of four fifths minus two
/// octaves. Both halves of the UI's auto-detect are this one question —
/// a tuning that answers yes engages the mode, and an edit of the third
/// that answers no releases it.
pub fn is_meantone(fifth_cents: f32, third_cents: f32) -> bool {
    (third_cents - meantone_third(fifth_cents)).abs() <= MEANTONE_TOLERANCE
}

/// A pitch class in microcents, always in `[0, OCTAVE_MICROCENTS)`.
#[derive(PartialEq, Eq, PartialOrd, Ord, Copy, Clone, Debug, Hash)]
pub struct PitchClass(u32);

impl PitchClass {
    pub fn from_cents(cents: f32) -> Self {
        // `f32::rem_euclid(1200.0)` can round *up* to exactly 1200.0 for
        // tiny negative inputs (a negative `c_offset`, a downward tuning
        // bend, a negative lattice sum), and `.round()` then yields
        // OCTAVE_MICROCENTS — outside the [0, OCTAVE) invariant. Left
        // uncorrected, such a value equals neither the origin it represents
        // (breaking Eq/Hash for map keys) nor prints as 0 (it shows
        // "1200.00¢"). Fold it back with the modulo.
        let micro = (cents.rem_euclid(1200.0) * CENTS_TO_MICROCENTS as f32).round() as u32;
        PitchClass(micro % OCTAVE_MICROCENTS)
    }

    pub fn from_midi_note(note: u8) -> Self {
        PitchClass(u32::from(note % 12) * 100 * CENTS_TO_MICROCENTS)
    }

    pub fn to_cents(self) -> f32 {
        cents(self.0)
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
        cents(self.0)
    }
}

/// Adjustable tuning of the lattice, stored in integer microcents so lattice
/// pitch arithmetic is exact. The f32 cent values from the host params (and
/// MIDI) are quantized once, in [`Tuning::from_cents`]; everything downstream
/// — [`Tuning::pitch_class`], [`Tuning::matches`], the meantone lock — is
/// integer math, and only display converts back to cents.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Tuning {
    /// Offset of the lattice origin from concert C, in microcents.
    pub c_offset: i32,
    /// Size of the prime-3 step (a perfect fifth), in microcents.
    pub three: i32,
    /// Size of the prime-5 step (a major third), in microcents.
    pub five: i32,
    /// Size of the prime-7 step (a harmonic seventh), in microcents.
    pub seven: i32,
    /// How far a played pitch may sit from a node's pitch class and still
    /// light that node up, in microcents.
    pub tolerance: i32,
}

impl Default for Tuning {
    /// 12-TET defaults, matching midi_lattice v1.
    fn default() -> Self {
        Tuning::from_cents(0.0, THREE_12TET, FIVE_12TET, SEVEN_12TET, 0.5)
    }
}

impl Tuning {
    /// Build a tuning from cent values, quantizing each to microcents. This
    /// is the only place cents become microcents; keeping it at the
    /// param/MIDI boundary is what lets all lattice math stay exact.
    pub fn from_cents(c_offset: f32, three: f32, five: f32, seven: f32, tolerance: f32) -> Self {
        Tuning {
            c_offset: microcents(c_offset),
            three: microcents(three),
            five: microcents(five),
            seven: microcents(seven),
            tolerance: microcents(tolerance),
        }
    }

    /// The 12-TET default with the three prime steps retuned to just.
    pub fn just() -> Self {
        Tuning::from_cents(0.0, THREE_JUST, FIVE_JUST, SEVEN_JUST, 0.5)
    }

    /// The pitch class of a lattice position under this tuning. Pure integer
    /// microcent arithmetic — `count * step` is exact, so algebraically equal
    /// pitches (e.g. meantone comma-equivalents) come out bit-identical
    /// instead of drifting by an f32 ulp. i64 accumulation: a single term can
    /// reach ~28 · 1e9, past i32.
    pub fn pitch_class(&self, pos: crate::coords::LatticePos) -> PitchClass {
        let total = i64::from(self.c_offset)
            + i64::from(pos.threes) * i64::from(self.three)
            + i64::from(pos.fives) * i64::from(self.five)
            + i64::from(pos.sevens) * i64::from(self.seven);
        PitchClass(total.rem_euclid(i64::from(OCTAVE_MICROCENTS)) as u32)
    }

    /// Whether a played pitch class matches a node's pitch class within the
    /// configured tolerance.
    pub fn matches(&self, played: PitchClass, node: PitchClass) -> bool {
        played.distance_to(node) <= PitchClassDistance(self.tolerance.max(0) as u32)
    }

    /// Lock the major third to four perfect fifths minus two octaves — the
    /// defining meantone identity — exactly, in integer microcents. Because
    /// `4 * three` is exact, every syntonic-comma-equivalent pair (four
    /// fifths vs. one third) then collapses to a single pitch class.
    pub fn lock_meantone(&mut self) {
        let five = 4 * i64::from(self.three) - 2 * i64::from(OCTAVE_MICROCENTS);
        self.five = five as i32;
    }

    /// Step sizes back in cents, for the display / host-param boundary.
    pub fn c_offset_cents(&self) -> f32 {
        cents(self.c_offset)
    }
    pub fn three_cents(&self) -> f32 {
        cents(self.three)
    }
    pub fn five_cents(&self) -> f32 {
        cents(self.five)
    }
    pub fn seven_cents(&self) -> f32 {
        cents(self.seven)
    }
    /// The matching tolerance in cents, for the display / host-param layer.
    pub fn tolerance_cents(&self) -> f32 {
        cents(self.tolerance)
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

    let [three, five, seven] = learn_primes(&classes);
    LearnedTuning { c_offset: learn_c_offset(&classes), three, five, seven }
}

/// The sounding pitch class closest to C, as a signed cents offset in
/// -600..=600, or `None` if nothing sounds within [`LEARN_C_RANGE_CENTS`].
fn learn_c_offset(classes: &[PitchClass]) -> Option<f32> {
    let c = PitchClass::from_cents(0.0);
    let mut best_c: Option<PitchClass> = None;
    for &pc in classes {
        if pc.distance_to(c) <= PitchClassDistance::from_cents(LEARN_C_RANGE_CENTS)
            && best_c.is_none_or(|b| pc.distance_to(c) < b.distance_to(c))
        {
            best_c = Some(pc);
        }
    }
    best_c.map(|pc| {
        let cents = pc.to_cents();
        if cents > 600.0 { cents - 1200.0 } else { cents }
    })
}

/// The best-fitting size for each prime axis (3, 5, 7), in cents, over
/// every pair of the sounding classes. Both directions of each interval
/// are candidates — a fourth implies a fifth, since octaves are assumed
/// perfectly tuned — and the candidate closest to just wins, if within
/// [`LEARN_RANGE_CENTS`].
fn learn_primes(classes: &[PitchClass]) -> [Option<f32>; 3] {
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
    best.map(|(_, found)| found.map(PitchClass::to_cents))
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
    fn tiny_negative_cents_fold_to_the_origin_not_a_full_octave() {
        // `f32::rem_euclid(1200.0)` rounds up to exactly 1200.0 for inputs a
        // hair below a multiple of an octave, so without the fold-back
        // `from_cents` would produce OCTAVE_MICROCENTS: a value that is
        // really the origin but compares unequal to it (breaking Eq/Hash for
        // map keys) and prints as "1200.00¢". These inputs arise in practice
        // from a slightly-flat `c_offset` or a downward tuning bend that
        // lands a hair below C.
        let origin = PitchClass::from_cents(0.0);
        assert_eq!(origin, PitchClass(0));
        for &cents in &[-1e-5_f32, -1e-6, -1e-7, -1200.0, -2400.0, -3600.0] {
            assert_eq!(
                PitchClass::from_cents(cents),
                origin,
                "from_cents({cents}) should fold back to the origin"
            );
        }
        // The [0, octave) invariant must hold across the boundary regardless
        // of whether a given input lands exactly on the origin: -1e-4¢ is a
        // genuine pitch class just below the octave (≈1199.9999¢), not the
        // origin, but it must still be strictly below one octave.
        for &cents in &[-1e-4_f32, -1e-5, -1e-8, 0.0, 1200.0, -1200.0, 2399.9999] {
            let pc = PitchClass::from_cents(cents);
            assert!(
                pc.to_cents() < 1200.0,
                "from_cents({cents}) = {pc} violates the [0, octave) invariant"
            );
        }
        // The fold must not disturb ordinary in-range values.
        assert!((PitchClass::from_cents(700.0).to_cents() - 700.0).abs() < 1e-3);
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
        let tuning = Tuning { tolerance: microcents(5.0), ..Tuning::just() };
        let node = tuning.pitch_class(LatticePos::new(1, 0, 0)); // 701.955
        assert!(tuning.matches(PitchClass::from_cents(700.0), node));
        assert!(!tuning.matches(PitchClass::from_cents(690.0), node));
    }

    #[test]
    fn meantone_comma_equivalents_are_bit_identical() {
        // Four fifths equal one major third plus two octaves in any meantone
        // temperament, so (t, f) and (t+4, f-1) are the same pitch. Integer
        // microcents make this hold *exactly* for every fifth and every
        // coordinate — the f32 pipeline drifted by up to ~0.004¢ for
        // non-power-of-two coordinates, which is what motivated this.
        for &three in &[700.0f32, 701.955, 696.5784, 700.0371, 703.4] {
            let mut tuning = Tuning::from_cents(0.0, three, 0.0, SEVEN_12TET, 0.5);
            tuning.lock_meantone();
            for t in -12..=12 {
                for f in -6..=6 {
                    let a = tuning.pitch_class(LatticePos::new(t, f, 0));
                    let b = tuning.pitch_class(LatticePos::new(t + 4, f - 1, 0));
                    assert_eq!(a, b, "three={three} ({t},{f}) vs ({},{})", t + 4, f - 1);
                }
            }
        }
    }

    #[test]
    fn lock_meantone_sets_the_exact_derived_third() {
        // The locked third is 4·fifth − 2 octaves, computed in integers.
        let mut tuning = Tuning::from_cents(0.0, 700.0, 386.0 /*ignored*/, SEVEN_12TET, 0.5);
        tuning.lock_meantone();
        // 12-TET: 4·700 − 2400 = 400¢.
        assert_eq!(tuning.five, microcents(400.0));
        // The general identity, evaluated in i64 (4·three overflows i32).
        let expected = 4 * i64::from(tuning.three) - 2 * i64::from(OCTAVE_MICROCENTS);
        assert_eq!(i64::from(tuning.five), expected);
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
