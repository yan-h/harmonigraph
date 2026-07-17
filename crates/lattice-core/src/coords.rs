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
    fn positions_within_counts() {
        let count = positions_within(-2..=2, -1..=1, 0..=0).count();
        assert_eq!(count, 5 * 3 * 1);
    }
}
