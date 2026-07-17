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

    /// Chebyshev distance from the origin; used for culling the (infinite)
    /// lattice to a displayable region.
    pub fn radius(&self) -> i32 {
        self.threes.abs().max(self.fives.abs()).max(self.sevens.abs())
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
        // The comma count only appears past one: two just thirds up is
        // G# lowered by two syntonic commas.
        assert_eq!(LatticePos::new(0, 2, 0).note_name().to_string(), "G\u{266F}-2");
        assert_eq!(LatticePos::new(0, 0, 1).note_name().to_string(), "B\u{266D}");
        // Flats stack: two fifths down from C is Bb... one fifth down is F.
        assert_eq!(LatticePos::new(-1, 0, 0).note_name().to_string(), "F");
    }

    #[test]
    fn positions_within_counts() {
        let count = positions_within(-2..=2, -1..=1, 0..=0).count();
        assert_eq!(count, (5 * 3));
    }
}


/// A note's spelled name: letter, sharps (negative = flats), and syntonic
/// comma adjustments. Formats with real accidentals, e.g. `G`, `F♯`,
/// `E♭-`, `B♭♭+2` (the UI's font stack guarantees the glyphs).
#[derive(PartialEq, Eq, Copy, Clone, Debug)]
pub struct NoteName {
    pub letter: char,
    /// Positive = sharps, negative = flats.
    pub sharps: i32,
    pub syntonic_commas: i32,
}

impl std::fmt::Display for NoteName {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.letter)?;
        for _ in 0..self.sharps.abs() {
            write!(f, "{}", if self.sharps > 0 { '\u{266F}' } else { '\u{266D}' })?;
        }
        // A single comma is common enough that the count is noise: bare
        // `+`/`-` for one, `+2`/`-2`... beyond.
        if self.syntonic_commas != 0 {
            write!(f, "{}", if self.syntonic_commas > 0 { '+' } else { '-' })?;
            if self.syntonic_commas.abs() > 1 {
                write!(f, "{}", self.syntonic_commas.abs())?;
            }
        }
        Ok(())
    }
}
