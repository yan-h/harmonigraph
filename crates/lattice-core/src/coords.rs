//! Integer coordinates on the 3-axis harmonic lattice.

/// A position on the lattice: the number of steps along each prime harmonic
/// axis from the origin (C).
#[derive(PartialEq, Eq, Copy, Clone, Debug, Hash)]
pub struct LatticePos {
    /// Steps along the prime-3 axis (perfect fifths).
    pub threes: i32,
    /// Steps along the prime-5 axis (major thirds).
    pub fives: i32,
    /// Steps along the prime-7 axis (harmonic sevenths).
    pub sevens: i32,
}

impl LatticePos {
    pub const ORIGIN: LatticePos = LatticePos { threes: 0, fives: 0, sevens: 0 };

    pub const fn new(threes: i32, fives: i32, sevens: i32) -> Self {
        LatticePos { threes, fives, sevens }
    }

    /// Musical name of this position, ported from v1's
    /// `PrimeCountVector::note_name_info`: the letter walks the chain of
    /// fifths (F C G D A E B), each prime-5 step acts like four fifths
    /// minus a syntonic comma, each prime-7 step like two fifths down.
    pub fn note_name(&self) -> NoteName {
        const NOTE_NAMES: [char; 7] = ['F', 'C', 'G', 'D', 'A', 'E', 'B'];
        let letter_names_idx = 1 + self.threes + self.fives * 4 - self.sevens * 2;
        NoteName {
            letter: NOTE_NAMES[letter_names_idx.rem_euclid(7) as usize],
            sharps: letter_names_idx.div_euclid(7),
            syntonic_commas: -self.fives,
        }
    }

    /// Whether two positions are exactly one unit step apart along exactly
    /// one prime axis — i.e. separated by one interval. Chord edges connect
    /// such pairs.
    pub fn is_adjacent(self, other: LatticePos) -> bool {
        let d = self - other;
        d.threes.abs() + d.fives.abs() + d.sevens.abs() == 1
    }
}

impl std::ops::Sub for LatticePos {
    type Output = LatticePos;
    fn sub(self, rhs: LatticePos) -> LatticePos {
        LatticePos::new(
            self.threes - rhs.threes,
            self.fives - rhs.fives,
            self.sevens - rhs.sevens,
        )
    }
}

/// Iterate every lattice position within the given per-axis extents
/// (inclusive). The scene layer uses this to enumerate displayable nodes.
pub fn positions_within(
    threes: std::ops::RangeInclusive<i32>,
    fives: std::ops::RangeInclusive<i32>,
    sevens: std::ops::RangeInclusive<i32>,
) -> impl Iterator<Item = LatticePos> {
    threes.flat_map(move |t| {
        let fives = fives.clone();
        let sevens = sevens.clone();
        fives.flat_map(move |f| {
            let sevens = sevens.clone();
            sevens.map(move |s| LatticePos::new(t, f, s))
        })
    })
}

/// A note's spelled name: letter, sharps (negative = flats), and syntonic
/// comma adjustments. Formats with real accidentals, e.g. `G`, `F♯`,
/// `E♭-`, `B♭2+2` (the UI's font stack guarantees the glyphs).
#[derive(PartialEq, Eq, Copy, Clone, Debug)]
pub struct NoteName {
    pub letter: char,
    /// Positive = sharps, negative = flats.
    pub sharps: i32,
    pub syntonic_commas: i32,
}

impl NoteName {
    /// The same spelling with the syntonic-comma marks dropped. In a
    /// meantone temperament the syntonic comma is tempered out, so `E-`
    /// (one just third up) and `E` (four fifths up) name the same pitch;
    /// meantone mode spells both without the comma.
    pub fn without_syntonic_commas(self) -> NoteName {
        NoteName { syntonic_commas: 0, ..self }
    }

    /// The accidental mark alone: `♯`, `♯2`, `♭3`, or empty for a natural.
    /// Empty when there is no accidental.
    pub fn accidental_mark(&self) -> String {
        mark(if self.sharps > 0 { '\u{266F}' } else { '\u{266D}' }, self.sharps)
    }

    /// The syntonic-comma mark alone: `+`, `-2`, ..., empty for none.
    pub fn comma_mark(&self) -> String {
        mark(if self.syntonic_commas > 0 { '+' } else { '-' }, self.syntonic_commas)
    }
}

/// One accidental or comma mark, counted rather than repeated. Out on the
/// far reaches of the lattice a name picks up marks fast, and spelling them
/// out (`C♯♯♯♯♯++++`) makes a label far wider than the node it sits on; v1
/// counted instead, and so do we. A single mark is common enough that the
/// count would be noise, so it stays bare: `♯`, then `♯2`, `♯3`...
fn mark(sign: char, count: i32) -> String {
    match count.abs() {
        0 => String::new(),
        1 => sign.to_string(),
        n => format!("{sign}{n}"),
    }
}

impl std::fmt::Display for NoteName {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}{}{}", self.letter, self.accidental_mark(), self.comma_mark())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn note_names_walk_the_lattice() {
        // Origin is C; one fifth up is G; one just third up is E lowered
        // by a syntonic comma; one harmonic seventh up from C is Bb-ish.
        assert_eq!(LatticePos::ORIGIN.note_name().to_string(), "C");
        assert_eq!(LatticePos::new(1, 0, 0).note_name().to_string(), "G");
        assert_eq!(LatticePos::new(0, 1, 0).note_name().to_string(), "E-");
        assert_eq!(LatticePos::new(0, 0, 1).note_name().to_string(), "B\u{266D}");
        // Flats stack: two fifths down from C is Bb... one fifth down is F.
        assert_eq!(LatticePos::new(-1, 0, 0).note_name().to_string(), "F");
    }

    #[test]
    fn note_names_count_accidentals_and_commas() {
        // Both kinds of mark are counted, never repeated: a name far out on
        // the lattice has to fit on the node it labels.
        let name = NoteName { letter: 'B', sharps: -2, syntonic_commas: 2 };
        assert_eq!(name.to_string(), "B\u{266D}2+2");
        let name = NoteName { letter: 'F', sharps: 2, syntonic_commas: 0 };
        assert_eq!(name.to_string(), "F\u{266F}2");
        let name = NoteName { letter: 'C', sharps: 5, syntonic_commas: 4 };
        assert_eq!(name.to_string(), "C\u{266F}5+4");
        // A single mark shows as a bare sign; the count appears past one.
        let name = NoteName { letter: 'A', sharps: 0, syntonic_commas: 1 };
        assert_eq!(name.to_string(), "A+");
        // Two just thirds up: G♯ lowered by two commas.
        assert_eq!(LatticePos::new(0, 2, 0).note_name().to_string(), "G\u{266F}-2");
    }

    #[test]
    fn marks_are_readable_separately_for_stacked_labels() {
        // The lattice draws the two marks in their own column, one above
        // the other, so it needs them apart rather than as one string.
        let name = NoteName { letter: 'E', sharps: -3, syntonic_commas: -1 };
        assert_eq!(name.accidental_mark(), "\u{266D}3");
        assert_eq!(name.comma_mark(), "-");
        let natural = NoteName { letter: 'G', sharps: 0, syntonic_commas: 0 };
        assert_eq!(natural.accidental_mark(), "");
        assert_eq!(natural.comma_mark(), "");
    }

    #[test]
    fn meantone_spelling_drops_commas() {
        // One just third up is E- in just spelling, plain E in meantone.
        assert_eq!(
            LatticePos::new(0, 1, 0).note_name().without_syntonic_commas().to_string(),
            "E"
        );
        // Two just thirds up: G♯-2 becomes plain G♯.
        assert_eq!(
            LatticePos::new(0, 2, 0).note_name().without_syntonic_commas().to_string(),
            "G\u{266F}"
        );
        // A name with no comma is unchanged (four fifths up is already E).
        assert_eq!(
            LatticePos::new(4, 0, 0).note_name().without_syntonic_commas().to_string(),
            "E"
        );
        // Sharps/flats survive; only the comma is removed.
        let name = NoteName { letter: 'B', sharps: -2, syntonic_commas: 2 };
        assert_eq!(name.without_syntonic_commas().to_string(), "B\u{266D}2");
    }

    #[test]
    fn adjacency_is_one_step_on_one_axis() {
        let origin = LatticePos::ORIGIN;
        assert!(origin.is_adjacent(LatticePos::new(1, 0, 0)));
        assert!(origin.is_adjacent(LatticePos::new(0, -1, 0)));
        // Not adjacent: itself, diagonals, two steps.
        assert!(!origin.is_adjacent(origin));
        assert!(!origin.is_adjacent(LatticePos::new(1, 1, 0)));
        assert!(!origin.is_adjacent(LatticePos::new(2, 0, 0)));
    }

    #[test]
    fn positions_within_counts() {
        let count = positions_within(-2..=2, -1..=1, 0..=0).count();
        assert_eq!(count, (5 * 3));
    }
}
