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

/// `Copy` here is load-bearing rather than a convenience. `MapEditor::undo` is
/// a `Vec` of these, and the AUDIO thread clears it — `AudioMaps::adopt` calls
/// `MapEditor::restore`. `Vec::clear` drops its elements in place and never
/// frees the buffer, so the only way that clear could reach an allocator is a
/// `Drop` impl on the element type; `Copy` forbids one, and forbids the owned
/// field (a `String`, a `Box`) that would need one. The invariant is therefore
/// compiler-enforced, and what breaks it is removing `Copy` from this struct —
/// not an undo entry growing a heap field, which would not compile (#893, #924).
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

    pub fn node(&self, midi: i64) -> LatticePos {
        let index = (midi.rem_euclid(12) as usize + 12 - Self::midi_class(self.position)) % 12;
        self.nodes[index] + self.position
    }

    /// Full unwrapped correction. Subtract the fixed MIDI generator interval,
    /// never the nearest octave; this preserves even more than an octave of drift.
    pub fn correction(&self, midi: i64, tuning: Tuning) -> i64 {
        let p = self.node(midi);
        i64::from(tuning.c_offset)
            + i64::from(p.threes) * (i64::from(tuning.three) - 700_000_000)
            + i64::from(p.fives) * (i64::from(tuning.five) - 400_000_000)
            + i64::from(p.sevens) * (i64::from(tuning.seven) - 1_000_000_000)
    }

    /// The 12-TET key an incoming onset selects, from its pitch in microcents.
    ///
    /// Selection is by sounding pitch rather than by key number, so a keyboard
    /// that already applies its own tuning — quarter-comma meantone, an MTS
    /// scale, a bend held at the attack — still lands on the key it is playing.
    /// A source further than half a semitone from its written key selects the
    /// key it actually sounds; the map has twelve slots and nothing else to
    /// offer a pitch that far out.
    pub fn rounded_key(pitch: i64) -> i64 {
        pitch.div_euclid(100_000_000) + i64::from(pitch.rem_euclid(100_000_000) >= 50_000_000)
    }

    /// The node one incoming onset lands on and what it takes to put it there.
    ///
    /// `pitch` is the onset's whole sounding pitch in microcents — key number,
    /// per-note tuning and channel bend together. The map states an absolute
    /// pitch, so the returned correction is the difference to it and the
    /// incoming detune is spent selecting the slot rather than added to the
    /// map's own offset. Correcting from wherever the source arrived would
    /// tune a pre-tuned E twice and move it off the map by its own tuning.
    pub fn assignment(&self, pitch: i64, tuning: Tuning) -> (LatticePos, i64) {
        let key = Self::rounded_key(pitch);
        (self.node(key), key * 100_000_000 + self.correction(key, tuning) - pitch)
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

    #[test]
    fn a_pre_tuned_source_selects_by_rounded_key_and_is_not_corrected_twice() {
        let map = LatticeMap::default();
        let just = Tuning::just();
        let (c, e) = (map.correction(60, just), map.correction(64, just));
        assert!(((e - c) as f64 / 1e6 + 13.686286).abs() < 0.001, "the map's E is 5/4");
        // A keyboard already sending that E arrives ON the map. The old rule
        // added the map's offset to whatever came in and bent it 13.7¢ again.
        assert_eq!(map.assignment(64 * 100_000_000 + e, just), (map.node(64), 0));
        // Every detune inside the key resolves to the one pitch the map states.
        for detune in [-49_999_999, e, -1, 0, 1, 49_999_999] {
            let pitch = 64 * 100_000_000 + detune;
            let (node, correction) = map.assignment(pitch, just);
            assert_eq!(node, map.node(64));
            assert_eq!(pitch + correction, 64 * 100_000_000 + e);
        }
        // Past half a semitone the sounding pitch picks the neighbouring key,
        // and gets that key's assignment rather than a spot between the two.
        for (detune, key) in [(50_000_000, 65), (99_000_000, 65), (-60_000_000, 63)] {
            let pitch = 64 * 100_000_000 + detune;
            let (node, correction) = map.assignment(pitch, just);
            assert_eq!(node, map.node(key));
            assert_eq!(pitch + correction, key * 100_000_000 + map.correction(key, just));
        }
        // A bend under the bottom key rounds without wrapping to the top of
        // the lattice, which is what an i64 rem rather than rem_euclid would do.
        assert_eq!(LatticeMap::rounded_key(-1), 0);
        assert_eq!(LatticeMap::rounded_key(-60_000_000), -1);
    }
}
