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
    /// minus a syntonic comma, each prime-7 step like two fifths down and
    /// a septimal comma.
    ///
    /// The letter is a pure function of the fifth count, so the spelling
    /// decomposes along the lattice axes: the letter carries prime 3, and
    /// each higher prime gets its own independent mark. Those two fifth
    /// shifts -- `+4` for prime 5, `-2` for prime 7 -- are the ones the
    /// Functional Just System derives, which is what makes a name here
    /// readable as extended-Pythagorean rather than as its own dialect.
    ///
    /// Both comma counts are NEGATED axis steps, and that is not a quirk:
    /// stepping up either axis lands a comma BELOW the Pythagorean note
    /// that shares the spelling. One prime-5 step up is `E-` (5/4 sits
    /// 81/80 under four fifths); one prime-7 step up is `B♭` with one
    /// septimal mark down (7/4 sits 64/63 under two fifths down).
    pub fn note_name(&self) -> NoteName {
        const NOTE_NAMES: [char; 7] = ['F', 'C', 'G', 'D', 'A', 'E', 'B'];
        let letter_names_idx = 1 + self.threes + self.fives * 4 - self.sevens * 2;
        NoteName {
            letter: NOTE_NAMES[letter_names_idx.rem_euclid(7) as usize],
            sharps: letter_names_idx.div_euclid(7),
            syntonic_commas: -self.fives,
            septimal_commas: -self.sevens,
        }
    }

    /// Where this position sits once `tempered`'s commas are tempered out:
    /// the same pitch, spelled on the axes those commas leave independent.
    /// Every name the UI draws goes through here, so two positions a
    /// temperament has made into one pitch also get one name.
    ///
    /// A collapse, not a mark dropped. Under meantone one prime-5 step IS
    /// four fifths, so `(t, f, s)` spells as `(t + 4f, 0, s)`, and `E-` and
    /// `E` both come out `E`. The septimal mark survives that, and has to:
    /// meantone is 5-limit and leaves 64/63 alone, so dropping the mark would
    /// name two pitches it keeps apart alike.
    ///
    /// Under marvel one prime-7 step is two fifths plus two thirds, so
    /// `(t, f, s)` spells as `(t + 2s, f + 2s, 0)`: the septimal mark goes and
    /// a harmonic seventh comes back as `A♯-2`, which is what 7/4 is once
    /// 225/224 is gone. Alone that is more marks rather than fewer — honest,
    /// and rare, since a tuning has to sit ON 225/224 to get here without
    /// meantone. With both tempered the two collapses compose into
    /// `(t + 4f + 10s, 0, 0)`, every name is a plain letter, and the seventh
    /// is the augmented sixth `A♯` — septimal meantone's own spelling.
    ///
    /// Down the primes, which is [`Comma::ALL`](crate::tuning::Comma::ALL)
    /// reversed: the kleisma folds the sevens onto the fives and threes, and
    /// the syntonic then folds those fives away too. The other order leaves
    /// fives behind.
    pub fn respell(self, tempered: crate::tuning::Tempered) -> LatticePos {
        let mut pos = self;
        if tempered.septimal_kleisma {
            pos = LatticePos::new(pos.threes + 2 * pos.sevens, pos.fives + 2 * pos.sevens, 0);
        }
        if tempered.syntonic {
            pos = LatticePos::new(pos.threes + 4 * pos.fives, 0, pos.sevens);
        }
        pos
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
        LatticePos::new(self.threes - rhs.threes, self.fives - rhs.fives, self.sevens - rhs.sevens)
    }
}

impl std::ops::Add for LatticePos {
    type Output = LatticePos;
    fn add(self, rhs: LatticePos) -> LatticePos {
        LatticePos::new(self.threes + rhs.threes, self.fives + rhs.fives, self.sevens + rhs.sevens)
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

/// A note's spelled name: letter, sharps (negative = flats), and one comma
/// count per prime axis above 3. Formats with real accidentals, e.g. `G`,
/// `F♯`, `E♭-`, `B♭2+2`, `B♭↓` (the UI's font stack guarantees the glyphs).
///
/// The septimal mark's `↑`/`↓` is the TEXT spelling, used by `Display` and
/// anywhere a name is quoted as a string. On the lattice itself the mark is
/// drawn as geometry rather than typeset -- see `panes::lattice`, which owns
/// that choice and the reason for it.
#[derive(PartialEq, Eq, Copy, Clone, Debug)]
pub struct NoteName {
    pub letter: char,
    /// Positive = sharps, negative = flats.
    pub sharps: i32,
    pub syntonic_commas: i32,
    /// Steps of the septimal comma (64/63), negative = lowered. One
    /// prime-7 step UP the lattice is one comma DOWN from the Pythagorean
    /// note that shares this spelling; see [`LatticePos::note_name`].
    pub septimal_commas: i32,
}

impl NoteName {
    /// The accidental mark alone: `♯`, `♯2`, `♭3`, or empty for a natural.
    /// Empty when there is no accidental.
    pub fn accidental_mark(&self) -> String {
        mark(if self.sharps > 0 { '\u{266F}' } else { '\u{266D}' }, self.sharps)
    }

    /// The syntonic-comma mark alone: `+`, `-2`, ..., empty for none.
    pub fn comma_mark(&self) -> String {
        mark(if self.syntonic_commas > 0 { '+' } else { '-' }, self.syntonic_commas)
    }

    /// The septimal-comma mark alone: `↑`, `↓2`, ..., empty for none.
    pub fn septimal_mark(&self) -> String {
        mark(if self.septimal_commas > 0 { '\u{2191}' } else { '\u{2193}' }, self.septimal_commas)
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
        write!(
            f,
            "{}{}{}{}",
            self.letter,
            self.accidental_mark(),
            self.comma_mark(),
            self.septimal_mark()
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A name with no septimal component -- the home sheet, where every
    /// test below that is not ABOUT the sevens axis lives.
    fn on_home(letter: char, sharps: i32, syntonic_commas: i32) -> NoteName {
        NoteName { letter, sharps, syntonic_commas, septimal_commas: 0 }
    }

    #[test]
    fn note_names_walk_the_lattice() {
        // Origin is C; one fifth up is G; one just third up is E lowered
        // by a syntonic comma; one harmonic seventh up from C is Bb-ish.
        assert_eq!(LatticePos::ORIGIN.note_name().to_string(), "C");
        assert_eq!(LatticePos::new(1, 0, 0).note_name().to_string(), "G");
        assert_eq!(LatticePos::new(0, 1, 0).note_name().to_string(), "E-");
        assert_eq!(LatticePos::new(0, 0, 1).note_name().to_string(), "B\u{266D}\u{2193}");
        // Flats stack: two fifths down from C is Bb... one fifth down is F.
        assert_eq!(LatticePos::new(-1, 0, 0).note_name().to_string(), "F");
    }

    #[test]
    fn a_sevens_step_no_longer_spells_like_two_fifths_down() {
        // The whole reason the septimal mark exists. 7/4 and 16/9 are 27
        // cents apart and both land on B♭; without a mark for the sevens
        // axis the name asserts they are the same pitch. The letter and
        // accidental still agree -- that is the Pythagorean spine -- and
        // the mark is the entire difference.
        let seventh = LatticePos::new(0, 0, 1).note_name();
        let two_fifths_down = LatticePos::new(-2, 0, 0).note_name();
        assert_eq!(seventh.letter, two_fifths_down.letter);
        assert_eq!(seventh.accidental_mark(), two_fifths_down.accidental_mark());
        assert_ne!(seventh.to_string(), two_fifths_down.to_string());
        assert_eq!(seventh.septimal_mark(), "\u{2193}");
        assert_eq!(two_fifths_down.septimal_mark(), "");
        // Down the axis marks the other way.
        assert_eq!(LatticePos::new(0, 0, -1).note_name().septimal_mark(), "\u{2191}");
    }

    #[test]
    fn note_names_count_accidentals_and_commas() {
        // Every kind of mark is counted, never repeated: a name far out on
        // the lattice has to fit on the node it labels.
        assert_eq!(on_home('B', -2, 2).to_string(), "B\u{266D}2+2");
        assert_eq!(on_home('F', 2, 0).to_string(), "F\u{266F}2");
        assert_eq!(on_home('C', 5, 4).to_string(), "C\u{266F}5+4");
        // A single mark shows as a bare sign; the count appears past one.
        assert_eq!(on_home('A', 0, 1).to_string(), "A+");
        // Two just thirds up: G♯ lowered by two commas.
        assert_eq!(LatticePos::new(0, 2, 0).note_name().to_string(), "G\u{266F}-2");
        // The septimal axis counts on the same terms -- five modulations
        // out is two characters wide, not five marks.
        assert_eq!(LatticePos::new(10, 0, 5).note_name().septimal_mark(), "\u{2193}5");
    }

    #[test]
    fn marks_are_readable_separately_for_stacked_labels() {
        // The lattice draws the marks in their own columns rather than as
        // one string, so it needs them apart.
        let name = NoteName { letter: 'E', sharps: -3, syntonic_commas: -1, septimal_commas: 2 };
        assert_eq!(name.accidental_mark(), "\u{266D}3");
        assert_eq!(name.comma_mark(), "-");
        assert_eq!(name.septimal_mark(), "\u{2191}2");
        let natural = on_home('G', 0, 0);
        assert_eq!(natural.accidental_mark(), "");
        assert_eq!(natural.comma_mark(), "");
        assert_eq!(natural.septimal_mark(), "");
    }

    /// The spelling under a set of tempered commas, as a string.
    fn spelled(pos: LatticePos, tempered: crate::tuning::Tempered) -> String {
        pos.respell(tempered).note_name().to_string()
    }

    #[test]
    fn meantone_spelling_drops_commas() {
        let meantone = crate::tuning::Tempered { syntonic: true, septimal_kleisma: false };
        // One just third up is E- in just spelling, plain E in meantone.
        assert_eq!(spelled(LatticePos::new(0, 1, 0), meantone), "E");
        // Two just thirds up: G♯-2 becomes plain G♯.
        assert_eq!(spelled(LatticePos::new(0, 2, 0), meantone), "G\u{266F}");
        // A name with no comma is unchanged (four fifths up is already E).
        assert_eq!(spelled(LatticePos::new(4, 0, 0), meantone), "E");
        // Sharps/flats survive; only the comma is folded away. Two just
        // thirds down from two fifths down is E♭2+2, and plain E♭2 once the
        // comma is gone.
        assert_eq!(LatticePos::new(-2, -2, 0).note_name().to_string(), "E\u{266D}2+2");
        assert_eq!(spelled(LatticePos::new(-2, -2, 0), meantone), "E\u{266D}2");
        // The septimal mark survives: meantone is 5-limit and leaves 64/63
        // alone, so dropping it would merge pitches meantone keeps distinct.
        assert_eq!(LatticePos::new(-2, 1, -1).note_name().to_string(), "E-\u{2191}");
        assert_eq!(spelled(LatticePos::new(-2, 1, -1), meantone), "E\u{2191}");
    }

    #[test]
    fn marvel_spelling_folds_the_sevens_axis_onto_the_home_sheet() {
        let marvel = crate::tuning::Tempered { syntonic: false, septimal_kleisma: true };
        // 7/4 is B♭↓ in just spelling. Temper 225/224 out and it is two
        // fifths plus two thirds instead: A♯-2, which carries no septimal
        // mark because there is no septimal comma left to mark.
        assert_eq!(spelled(LatticePos::new(0, 0, 1), marvel), "A\u{266F}-2");
        // The home sheet is untouched — nothing on it has a sevens step to
        // fold, so marvel alone leaves every 5-limit name exactly as it was.
        for pos in [LatticePos::new(0, 1, 0), LatticePos::new(4, 0, 0), LatticePos::new(-3, 2, 0)] {
            assert_eq!(spelled(pos, marvel), pos.note_name().to_string());
        }
    }

    #[test]
    fn both_commas_compose_into_septimal_meantone() {
        let both = crate::tuning::Tempered { syntonic: true, septimal_kleisma: true };
        // Ten fifths up, which is what the harmonic seventh becomes when both
        // commas go: the augmented sixth, and not a mark on it.
        assert_eq!(spelled(LatticePos::new(0, 0, 1), both), "A\u{266F}");
        assert_eq!(LatticePos::new(10, 0, 0).note_name().to_string(), "A\u{266F}");
        // Every name is a plain letter and accidental — the whole point of
        // the pair, and what makes a septimal-meantone lattice readable.
        for pos in positions_within(-4..=4, -2..=2, -1..=1) {
            let name = pos.respell(both).note_name();
            assert_eq!((name.syntonic_commas, name.septimal_commas), (0, 0), "{pos:?}");
        }
    }

    #[test]
    fn a_tempered_comma_apart_is_one_pitch_and_one_name() {
        // The contract the whole collapse rests on. Each comma's step is a
        // step to the same pitch once it is tempered out — checked against
        // the tuning, so the spelling claim is anchored to real pitch classes
        // rather than to another statement of the same arithmetic.
        use crate::tuning::{Comma, Tempered, Tuning};
        let step = |comma| match comma {
            // Four fifths minus a third, and two fifths plus two thirds
            // minus a seventh: an octave each, once tempered.
            Comma::Syntonic => LatticePos::new(4, -1, 0),
            Comma::SeptimalKleisma => LatticePos::new(2, 2, -1),
        };
        for comma in Comma::ALL {
            let tempered = Tempered::default().with(comma, true);
            let mut tuning = Tuning::just();
            tuning.temper(comma);
            let step = step(comma);
            for pos in positions_within(-4..=4, -2..=2, -1..=1) {
                let moved = LatticePos::new(
                    pos.threes + step.threes,
                    pos.fives + step.fives,
                    pos.sevens + step.sevens,
                );
                assert_eq!(tuning.pitch_class(pos), tuning.pitch_class(moved), "{pos:?}");
                assert_eq!(spelled(pos, tempered), spelled(moved, tempered), "{pos:?} {comma:?}");
                // Untempered they are two pitches with two names, which is
                // what makes this a temperament rather than a bug.
                assert_ne!(pos.note_name().to_string(), moved.note_name().to_string(), "{pos:?}");
            }
        }
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
