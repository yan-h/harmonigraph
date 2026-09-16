//! Explicit MIDI assignments. Geometry and fixed generator labels never depend
//! on acoustic proximity, temperament spelling, or earlier notes.
use crate::{LatticePos, Tuning};

pub const COORDINATE_LIMIT: i32 = 4096;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum TuningEngine {
    Off,
    #[default]
    Adaptive,
    LatticeMap,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct LatticeMap {
    /// One coordinate per base MIDI class, relative to C at the shape origin.
    pub nodes: [LatticePos; 12],
    pub position: LatticePos,
}

impl Default for LatticeMap {
    fn default() -> Self {
        let mut nodes = [LatticePos::ORIGIN; 12];
        // Four fifth rows, three third columns: F C G D / A E B F# /
        // C# G# D# A#. C is the zero-correction origin under just axes.
        for fifth in -1..=2 {
            for third in 0..=2 {
                let node = LatticePos::new(fifth, third, 0);
                nodes[Self::midi_class(node)] = node;
            }
        }
        Self { nodes, position: LatticePos::ORIGIN }
    }
}

impl LatticeMap {
    pub fn midi_class(node: LatticePos) -> usize {
        (7 * i64::from(node.threes) + 4 * i64::from(node.fives) + 10 * i64::from(node.sevens))
            .rem_euclid(12) as usize
    }

    pub fn valid(&self) -> bool {
        let bounded = |p: LatticePos| {
            [p.threes, p.fives, p.sevens]
                .into_iter()
                .all(|v| v.abs_diff(0) <= COORDINATE_LIMIT as u32)
        };
        bounded(self.position)
            && self.nodes.iter().enumerate().all(|(i, &p)| bounded(p) && Self::midi_class(p) == i)
    }

    pub fn node(&self, midi: u8) -> LatticePos {
        let index = (usize::from(midi % 12) + 12 - Self::midi_class(self.position)) % 12;
        self.nodes[index] + self.position
    }

    /// Full unwrapped correction. Subtract the fixed MIDI generator interval,
    /// never the nearest octave; this preserves even more than an octave of drift.
    pub fn correction(&self, midi: u8, tuning: Tuning) -> i64 {
        let p = self.node(midi);
        i64::from(tuning.c_offset)
            + i64::from(p.threes) * (i64::from(tuning.three) - 700_000_000)
            + i64::from(p.fives) * (i64::from(tuning.five) - 400_000_000)
            + i64::from(p.sevens) * (i64::from(tuning.seven) - 1_000_000_000)
    }

    /// The exact destination determines the replaced slot. No source-selection
    /// or temperament canonicalization is involved.
    pub fn replace(&mut self, destination: LatticePos) -> bool {
        let relative = destination - self.position;
        let index = Self::midi_class(relative);
        let mut next = *self;
        next.nodes[index] = relative;
        if next == *self || !next.valid() {
            return false;
        }
        *self = next;
        true
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn travel_rotates_labels_and_keeps_unwrapped_drift() {
        let mut map = LatticeMap::default();
        assert!(map.valid());
        for (p, midi) in [(LatticePos::new(50, 0, 0), 2), (LatticePos::new(0, 0, 100), 4)] {
            map.position = p;
            assert_eq!(map.node(midi), p);
            let expected = i64::from(p.threes) * (i64::from(Tuning::just().three) - 700_000_000)
                + i64::from(p.sevens) * (i64::from(Tuning::just().seven) - 1_000_000_000);
            assert_eq!(map.correction(midi, Tuning::just()), expected);
            assert_eq!(map.correction(midi + 60, Tuning::just()), expected);
            assert_eq!(map.correction(midi, Tuning::default()), 0);
            assert!(map.valid());
        }
        map.position = LatticePos::new(50, 0, 0);
        assert!((map.correction(2, Tuning::just()) as f64 / 1e6 - 97.75).abs() < 0.01);
    }

    #[test]
    fn substitution_preserves_exact_copy_and_travels_with_the_region() {
        let mut map = LatticeMap::default();
        let before = map;
        assert!(map.replace(LatticePos::new(4, 0, 0)));
        assert_eq!(map.nodes.iter().zip(before.nodes).filter(|(a, b)| **a != *b).count(), 1);
        assert!(
            ((map.correction(4, Tuning::just()) - before.correction(4, Tuning::just())) as f64
                / 1e6
                - 21.506)
                .abs()
                < 0.01
        );
        let mut meantone = Tuning::just();
        meantone.temper(crate::Comma::Syntonic);
        assert_eq!(map.correction(4, meantone), before.correction(4, meantone));
        map.position = LatticePos::new(51, 0, 0);
        let destination = LatticePos::new(51, 1, 0);
        assert!(map.replace(destination));
        assert_eq!(map.node(1), destination);
        assert!(!map.replace(destination));
        assert!(map.valid());
    }
}
