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

/// The septimal kleisma (225/224, ~7.712¢): the gap between two just fifths
/// plus two just thirds (less an octave) and a just harmonic seventh. Marvel
/// temperaments temper it out.
pub const SEPTIMAL_KLEISMA: f32 = 2.0 * THREE_JUST + 2.0 * FIVE_JUST - 1200.0 - SEVEN_JUST;

/// The meantone major third implied by a given fifth: four fifths stacked,
/// dropped two octaves. In any meantone temperament the major third equals
/// this exactly, so the prime-5 (thirds) axis stops being independent of
/// the prime-3 (fifths) axis — the defining property of meantone.
pub fn meantone_third(fifth_cents: f32) -> f32 {
    4.0 * fifth_cents - 2.0 * 1200.0
}

/// The marvel harmonic seventh implied by a given fifth and third: two of
/// each, dropped an octave. In any marvel temperament the harmonic seventh
/// equals this exactly, so the prime-7 axis stops being independent of the
/// two below it — the same shape as meantone one prime up, and the reason
/// the two modes are one mechanism.
pub fn marvel_seventh(fifth_cents: f32, third_cents: f32) -> f32 {
    2.0 * fifth_cents + 2.0 * third_cents - 1200.0
}

/// How close (in cents) the tuning axes must sit to a comma's identity for
/// that comma to count as tempered out.
///
/// Half a cent is "on it", not a family window: the tunings this is meant to
/// catch — 12-TET (400 = 4·700 − 2400, 1000 = 2·700 + 2·400 − 1200),
/// quarter-comma, any preset or learned chord that IS one of these
/// temperaments — land on the identity to within rounding, so the tolerance
/// only has to cover the f32/microcent slop and a value typed to two decimals
/// (which this still admits, either side). Anything wider starts claiming
/// tunings that were deliberately set a little off it, and tempering a comma
/// out of a tuning that keeps a cent of it is a change to the picture nobody
/// asked for.
///
/// It is also the width of the derived bar's magnet (see the UI's
/// `tempered_bar`), so the same number says how far a drag has to pull to
/// release a mode. At this width that is a pixel or two of an 80¢ bar, which
/// makes it a release threshold rather than a snap anyone will feel: a mode is
/// easy to leave and effectively unreachable by dragging alone. Reaching one
/// is what the presets, learn and typing a value are for.
pub const TEMPER_TOLERANCE: f32 = 0.5;

/// Whether a fifth/third pair is (close to) a meantone temperament: the
/// major third within [`TEMPER_TOLERANCE`] of four fifths minus two
/// octaves. Both halves of the UI's auto-detect are this one question —
/// a tuning that answers yes engages the mode, and an edit of the third
/// that answers no releases it.
pub fn is_meantone(fifth_cents: f32, third_cents: f32) -> bool {
    (third_cents - meantone_third(fifth_cents)).abs() <= TEMPER_TOLERANCE
}

/// Whether a fifth/third/seventh triple is (close to) a marvel temperament:
/// the harmonic seventh within [`TEMPER_TOLERANCE`] of two fifths plus two
/// thirds minus an octave.
///
/// The THIRD it reads is the one the lattice is using, so when meantone holds
/// as well this asks about the derived third rather than the inert param —
/// the two identities compose, and tempering both makes the seventh ten
/// fifths up (septimal meantone).
pub fn is_marvel(fifth_cents: f32, third_cents: f32, seventh_cents: f32) -> bool {
    (seventh_cents - marvel_seventh(fifth_cents, third_cents)).abs() <= TEMPER_TOLERANCE
}

/// A comma the lattice can temper out.
///
/// Each one names an identity between the tuning axes: while it holds, one
/// axis is DERIVED from the ones below it rather than set, and lattice
/// positions a comma apart become one pitch — so they must also become one
/// name (see [`LatticePos::respell`](crate::coords::LatticePos::respell)).
/// Everything else about a comma — its ratio, the temperament it defines, the
/// axis it pins — hangs off this enum, so the UI's tempering section is a loop
/// over [`Comma::ALL`] rather than a switch per comma.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Comma {
    /// 81/80. Tempering it out is meantone: the major third is four fifths.
    Syntonic,
    /// 225/224. Tempering it out is marvel: the harmonic seventh is two
    /// fifths plus two thirds.
    SeptimalKleisma,
}

impl Comma {
    /// Every comma the tempering section offers, in the order it lists them:
    /// up the primes, which is also the order they must be applied in — the
    /// septimal identity reads the third, so a tempered third has to be
    /// derived before the seventh is derived from it.
    ///
    /// Built from an exhaustive `match` rather than written out as a bare
    /// literal, so a third comma cannot join the enum without joining this
    /// list too — the same guard `SpectralOrientation::ALL` in
    /// `harmonigraph-ui` uses, for the same reason. The order above is still
    /// the one enforced elsewhere; this only adds the compiler's proof that
    /// nothing is missing from it.
    pub const ALL: [Comma; 2] = {
        use Comma::*;
        // Exhaustive, and the compiler checks it. The arm is `()` because
        // what is wanted is the coverage error, not the value.
        const fn covered(comma: Comma) {
            match comma {
                Syntonic | SeptimalKleisma => (),
            }
        }
        covered(Syntonic);
        [Syntonic, SeptimalKleisma]
    };
    /// How many there are, for per-comma arrays indexed by [`Self::index`].
    pub const COUNT: usize = Comma::ALL.len();

    /// Position in [`Self::ALL`], for indexing per-comma state.
    pub fn index(self) -> usize {
        match self {
            Comma::Syntonic => 0,
            Comma::SeptimalKleisma => 1,
        }
    }

    /// The ratio, written the way it is said.
    pub fn ratio(self) -> &'static str {
        match self {
            Comma::Syntonic => "81/80",
            Comma::SeptimalKleisma => "225/224",
        }
    }

    /// The comma's own name.
    pub fn comma_name(self) -> &'static str {
        match self {
            Comma::Syntonic => "syntonic comma",
            Comma::SeptimalKleisma => "septimal kleisma",
        }
    }

    /// The temperament that tempers it out — what the mode is called.
    pub fn temperament(self) -> &'static str {
        match self {
            Comma::Syntonic => "Meantone",
            Comma::SeptimalKleisma => "Marvel",
        }
    }

    /// Its size in cents, at just tuning.
    pub fn size_cents(self) -> f32 {
        match self {
            Comma::Syntonic => SYNTONIC_COMMA,
            Comma::SeptimalKleisma => SEPTIMAL_KLEISMA,
        }
    }

    /// The axis the identity derives — the one that stops being independent,
    /// and whose bar is where the mode is let go of.
    pub fn derived_axis_name(self) -> &'static str {
        match self {
            Comma::Syntonic => "major third",
            Comma::SeptimalKleisma => "harmonic seventh",
        }
    }

    /// What it derives that axis from, in words. With
    /// [`Self::derived_axis_name`] this is the identity as a hover can say it:
    /// "the major third follows four perfect fifths (minus two octaves)".
    pub fn derived_from(self) -> &'static str {
        match self {
            Comma::Syntonic => "four perfect fifths (minus two octaves)",
            Comma::SeptimalKleisma => "two fifths plus two thirds (minus an octave)",
        }
    }

    /// The value the derived axis takes while this comma is tempered out.
    /// Both take the axes BELOW the one they derive, so the third passed here
    /// is the one in use — derived itself, if meantone holds too.
    pub fn derived(self, fifth_cents: f32, third_cents: f32) -> f32 {
        match self {
            Comma::Syntonic => meantone_third(fifth_cents),
            Comma::SeptimalKleisma => marvel_seventh(fifth_cents, third_cents),
        }
    }

    /// Whether the tuning sits within [`TEMPER_TOLERANCE`] of this comma's
    /// identity — the whole of the UI's auto-detect, one comma at a time.
    pub fn is_tempered(self, fifth_cents: f32, third_cents: f32, seventh_cents: f32) -> bool {
        match self {
            Comma::Syntonic => is_meantone(fifth_cents, third_cents),
            Comma::SeptimalKleisma => is_marvel(fifth_cents, third_cents, seventh_cents),
        }
    }
}

/// Which commas are being tempered out, as a set — the display's whole view
/// of the tempering modes, and what a note name is spelled against.
///
/// A set rather than one comma at a time because the spellings COMPOSE: with
/// both tempered the seventh is ten fifths up and every name on the lattice
/// is a plain letter (septimal meantone), which neither comma gives alone.
///
/// The default is the empty set — the just reading, where every axis is
/// independent and every comma is a real distance.
#[derive(Clone, Copy, Default, PartialEq, Eq, Debug)]
pub struct Tempered {
    pub syntonic: bool,
    pub septimal_kleisma: bool,
}

impl Tempered {
    pub fn has(self, comma: Comma) -> bool {
        match comma {
            Comma::Syntonic => self.syntonic,
            Comma::SeptimalKleisma => self.septimal_kleisma,
        }
    }

    /// The same set with one comma set either way, for building one up.
    pub fn with(mut self, comma: Comma, on: bool) -> Self {
        match comma {
            Comma::Syntonic => self.syntonic = on,
            Comma::SeptimalKleisma => self.septimal_kleisma = on,
        }
        self
    }
}

/// A pitch class in microcents, always in `[0, OCTAVE_MICROCENTS)`.
#[derive(PartialEq, Eq, PartialOrd, Ord, Copy, Clone, Debug, Hash)]
pub struct PitchClass(u32);

impl PitchClass {
    /// Fold exact absolute pitch into one octave without a float conversion.
    pub fn from_microcents(value: i64) -> Self {
        Self(value.rem_euclid(i64::from(OCTAVE_MICROCENTS)) as u32)
    }

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

    /// Temper one comma out: pin the axis its identity derives to the value
    /// the identity gives, exactly, in integer microcents. Because the
    /// multiples are exact, every pair of positions a comma apart then
    /// collapses to a single pitch class — four fifths vs. one third for the
    /// syntonic, two fifths plus two thirds vs. one seventh for the kleisma.
    ///
    /// Apply commas in [`Comma::ALL`] order: the septimal identity reads the
    /// third, so tempering the syntonic first is what makes the two compose
    /// into septimal meantone (a seventh of ten fifths) rather than leaving
    /// the seventh derived from a third the lattice is not using.
    pub fn temper(&mut self, comma: Comma) {
        match comma {
            Comma::Syntonic => {
                let five = 4 * i64::from(self.three) - 2 * i64::from(OCTAVE_MICROCENTS);
                self.five = five as i32;
            }
            Comma::SeptimalKleisma => {
                let seven = 2 * i64::from(self.three) + 2 * i64::from(self.five)
                    - i64::from(OCTAVE_MICROCENTS);
                self.seven = seven as i32;
            }
        }
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

    learn_sorted_classes(&classes)
}

/// Allocation-free inference from sorted, distinct classes. The confirmed-state
/// component supplies its fixed scratch slice here, preserving deterministic ties.
pub fn learn_sorted_classes(classes: &[PitchClass]) -> LearnedTuning {
    learn_sorted_classes_counted(classes, &mut 0)
}

pub(crate) fn learn_sorted_classes_counted(
    classes: &[PitchClass],
    pair_visits: &mut usize,
) -> LearnedTuning {
    debug_assert!(classes.windows(2).all(|pair| pair[0] < pair[1]));
    let [three, five, seven] = learn_primes(classes, pair_visits);
    LearnedTuning { c_offset: learn_c_offset(classes), three, five, seven }
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
        if cents > 600.0 {
            cents - 1200.0
        } else {
            cents
        }
    })
}

/// The best-fitting size for each prime axis (3, 5, 7), in cents, over
/// every pair of the sounding classes. Both directions of each interval
/// are candidates — a fourth implies a fifth, since octaves are assumed
/// perfectly tuned — and the candidate closest to just wins, if within
/// [`LEARN_RANGE_CENTS`].
fn learn_primes(classes: &[PitchClass], pair_visits: &mut usize) -> [Option<f32>; 3] {
    let mut best = [
        (PitchClass::from_cents(THREE_JUST), None::<PitchClass>),
        (PitchClass::from_cents(FIVE_JUST), None),
        (PitchClass::from_cents(SEVEN_JUST), None),
    ];
    for (i, &a) in classes.iter().enumerate() {
        for &b in &classes[i + 1..] {
            *pair_visits += 1;
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
        assert!(
            pc.distance_to(PitchClass::from_cents(203.91)) <= PitchClassDistance::from_cents(0.01)
        );
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
        assert!(is_meantone(THREE_12TET, FIVE_12TET + TEMPER_TOLERANCE - 0.1));
        assert!(!is_meantone(THREE_12TET, FIVE_12TET + TEMPER_TOLERANCE + 0.1));
    }

    #[test]
    fn the_septimal_kleisma_is_the_gap_it_says_it_is() {
        // 225/224 in cents, from the ratio rather than from the axis sizes
        // the constant is built out of.
        let ratio = 1200.0 * (225.0f32 / 224.0).log2();
        assert!((SEPTIMAL_KLEISMA - ratio).abs() < 0.001, "{SEPTIMAL_KLEISMA} vs {ratio}");
        // Small: about a third of the syntonic comma, which is why a tuning
        // can sit on it without looking obviously tempered.
        assert!((SYNTONIC_COMMA / SEPTIMAL_KLEISMA - 2.79).abs() < 0.01);
    }

    #[test]
    fn is_marvel_accepts_12tet_rejects_just() {
        // 12-TET tempers it out too (1000 = 2·700 + 2·400 − 1200).
        assert!(is_marvel(THREE_12TET, FIVE_12TET, SEVEN_12TET));
        // Just intonation keeps the kleisma: the just seventh sits one below
        // two just fifths plus two just thirds.
        assert!(!is_marvel(THREE_JUST, FIVE_JUST, SEVEN_JUST));
        // Septimal meantone: with the syntonic tempered out first, the marvel
        // seventh is ten fifths minus five octaves. Quarter-comma, so the
        // third fed in is the derived one, not the just third it sits on.
        let quarter = THREE_JUST - SYNTONIC_COMMA / 4.0;
        let third = meantone_third(quarter);
        let seventh = marvel_seventh(quarter, third);
        assert!((seventh - (10.0 * quarter - 6000.0)).abs() < 0.001);
        assert!(is_marvel(quarter, third, seventh));
        // And that seventh is NOT the just one — 3¢ of septimal meantone.
        assert!((seventh - SEVEN_JUST).abs() > 2.0);
        // A seventh a hair (< tolerance) off still counts.
        assert!(is_marvel(THREE_12TET, FIVE_12TET, SEVEN_12TET + TEMPER_TOLERANCE - 0.1));
        assert!(!is_marvel(THREE_12TET, FIVE_12TET, SEVEN_12TET + TEMPER_TOLERANCE + 0.1));
    }

    #[test]
    fn every_comma_answers_for_itself() {
        // The enum is what the UI loops over, so each variant has to carry
        // the whole of its own identity: the detect, the derived value, and
        // the size it would leave in the tuning if it were not tempered.
        for comma in Comma::ALL {
            assert_eq!(Comma::ALL[comma.index()], comma);
            assert!(comma.size_cents() > 0.0);
            // 12-TET tempers out both, and the derived axis is what 12-TET
            // already has.
            assert!(comma.is_tempered(THREE_12TET, FIVE_12TET, SEVEN_12TET));
            let derived = comma.derived(THREE_12TET, FIVE_12TET);
            let expected = match comma {
                Comma::Syntonic => FIVE_12TET,
                Comma::SeptimalKleisma => SEVEN_12TET,
            };
            assert!((derived - expected).abs() < 0.001, "{comma:?}: {derived} vs {expected}");
            // Just intonation keeps every one of them.
            assert!(!comma.is_tempered(THREE_JUST, FIVE_JUST, SEVEN_JUST));
        }
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
            tuning.temper(Comma::Syntonic);
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
    fn kleisma_comma_equivalents_are_bit_identical() {
        // Two fifths plus two thirds equal one harmonic seventh plus an
        // octave in any marvel temperament, so (t, f, s) and (t+2, f+2, s-1)
        // are the same pitch — the same exactness the meantone lock has, one
        // prime up, and what lets the sevens sheet be respelled onto the
        // home sheet without naming two pitches alike.
        for &(three, five) in &[(700.0f32, 400.0f32), (701.955, 386.3137), (696.5784, 386.3137)] {
            let mut tuning = Tuning::from_cents(0.0, three, five, 0.0, 0.5);
            tuning.temper(Comma::SeptimalKleisma);
            for t in -8..=8 {
                for f in -4..=4 {
                    for s in -3..=3 {
                        let a = tuning.pitch_class(LatticePos::new(t, f, s));
                        let b = tuning.pitch_class(LatticePos::new(t + 2, f + 2, s - 1));
                        assert_eq!(a, b, "({three},{five}) ({t},{f},{s})");
                    }
                }
            }
        }
    }

    #[test]
    fn tempering_sets_the_exact_derived_axis() {
        // The derived third is 4·fifth − 2 octaves, computed in integers.
        let mut tuning = Tuning::from_cents(0.0, 700.0, 386.0 /*ignored*/, 968.0, 0.5);
        tuning.temper(Comma::Syntonic);
        // 12-TET: 4·700 − 2400 = 400¢.
        assert_eq!(tuning.five, microcents(400.0));
        // The general identity, evaluated in i64 (4·three overflows i32).
        let expected = 4 * i64::from(tuning.three) - 2 * i64::from(OCTAVE_MICROCENTS);
        assert_eq!(i64::from(tuning.five), expected);
        // The seventh follows the third the lattice is USING, so tempering in
        // ALL order lands on septimal meantone: 10·fifth − 5 octaves.
        tuning.temper(Comma::SeptimalKleisma);
        assert_eq!(tuning.seven, microcents(1000.0));
        assert_eq!(
            i64::from(tuning.seven),
            10 * i64::from(tuning.three) - 5 * i64::from(OCTAVE_MICROCENTS)
        );
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
        let classes = [PitchClass::from_cents(0.0), PitchClass::from_cents(1200.0 - THREE_JUST)];
        let learned = learn_tuning(&classes);
        let three = learned.three.unwrap();
        assert!((three - THREE_JUST).abs() < 0.01, "three = {three}");
    }

    #[test]
    fn nothing_close_learns_nothing() {
        // A tritone-ish dyad: no prime interval within range, no C.
        let classes = [PitchClass::from_cents(300.0), PitchClass::from_cents(900.0)];
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
