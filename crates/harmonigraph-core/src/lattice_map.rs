//! Explicit MIDI assignments. Geometry and fixed generator labels never depend
//! on acoustic proximity, temperament spelling, or earlier notes.
use crate::policy::ContextPitch;
use crate::{LatticePos, Tuning};

pub const COORDINATE_LIMIT: i32 = 4096;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum TuningEngine {
    Off,
    #[default]
    Adaptive,
    LatticeMap,
}

/// Whether Lattice Map moves the selected map by itself, and along which axes.
///
/// A prototype of adaptive tuning from the map's side: the map keeps its
/// shape and the Hub slides it one step at a time so that a chord's intervals
/// take their simplest spellings. A thirds step moves one column of the default
/// shape by a diesis, which is almost never a spelling anyone means; a fifths
/// step moves one row by a syntonic comma, which is the classic ambiguity, so
/// it costs more and is taken only when it clearly helps.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Follow {
    #[default]
    Off,
    Thirds,
    ThirdsAndFifths,
}

/// The distinct pitch classes of one attack group a follow decision weighs:
/// all twelve, fixed so the decision runs on the audio thread without
/// allocating.
pub const FOLLOW_GROUP: usize = 12;

/// What each step costs a follow decision, in the bits [`excess`] counts.
/// A wolf fifth is about 5.5 bits and a Pythagorean third 4, so a fifths step
/// pays for itself only against an error of that kind.
const FIFTH_STEP: f64 = 1.0;
const THIRD_STEP: f64 = 0.25;
/// What a released note weighs against one sounding in the same chord. A note
/// just heard keeps its pitch when nothing else decides, which is what makes
/// the map drift rather than snap back; but it must not outvote the chord's own
/// intervals, or C E G followed by D F A would keep D's wolf to stay level with
/// the G before it.
pub const FOLLOW_RELEASED: f64 = 0.5;

/// The simplest 5-limit spelling of each 12-TET class, as (fifths, thirds).
const SIMPLEST: [(i32, i32); 12] = [
    (0, 0),
    (-1, -1),
    (2, 0),
    (1, -1),
    (0, 1),
    (-1, 0),
    (2, 1),
    (1, 0),
    (0, -1),
    (-1, 1),
    (-2, 0),
    (1, 1),
];

/// Tenney height without the twos: `log2` of the odd part of the ratio.
fn height(v: LatticePos) -> f64 {
    const LOG2_3: f64 = 1.584_962_500_721_156;
    const LOG2_5: f64 = 2.321_928_094_887_362;
    const LOG2_7: f64 = 2.807_354_922_057_604;
    f64::from(v.threes.unsigned_abs()) * LOG2_3
        + f64::from(v.fives.unsigned_abs()) * LOG2_5
        + f64::from(v.sevens.unsigned_abs()) * LOG2_7
}

/// How many bits more complex an interval is spelled than its 12-TET class
/// needs: zero for a 3/2 or a 5/4, about 5.5 for the wolf fifth 40/27.
pub fn excess(v: LatticePos) -> f64 {
    let (fifths, thirds) = SIMPLEST[LatticeMap::midi_class(v)];
    height(v) - height(LatticePos::new(fifths, thirds, 0))
}

/// `Copy` here is load-bearing rather than a convenience: the AUDIO thread
/// copies whole maps into `AudioMaps`' fixed bank and into every timestamped
/// attack state, and `Copy` is what guarantees no owned field (a `String`, a
/// `Box`) ever makes those copies reach an allocator (#893, #924).
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

    /// Where a follow decision moves the map for a group of keys struck
    /// together, as the next follow offset on top of `self.position`.
    ///
    /// Candidates are one step either way along each axis `follow` allows,
    /// from `current`. Each is scored by the [`excess`] of every interval the
    /// group's nodes would make among themselves and against `context`, each
    /// interval to a context node weighted as Adaptive weighs it; a step adds
    /// its own cost, so the map stays put unless moving makes the chord
    /// simpler. The context is Adaptive's too: the held notes, or with nothing
    /// held the released ones, so a common tone keeps its pitch and the map
    /// drifts rather than snapping back. Ties keep the current place, and
    /// otherwise go to the candidate farther from the automated place.
    pub fn follow(
        &self,
        follow: Follow,
        current: LatticePos,
        context: &[ContextPitch],
        keys: &[i64],
    ) -> LatticePos {
        let fifths: &[i32] = match follow {
            Follow::Off => return LatticePos::ORIGIN,
            Follow::Thirds => &[0],
            Follow::ThirdsAndFifths => &[0, -1, 1],
        };
        let mut classes = [false; 12];
        for key in keys {
            classes[key.rem_euclid(12) as usize] = true;
        }
        let mut best = (f64::INFINITY, f64::NEG_INFINITY, current);
        for &df in fifths {
            for dt in [0, -1, 1] {
                let offset = current + LatticePos::new(df, dt, 0);
                let map = LatticeMap { position: self.position + offset, ..*self };
                if !map.valid() {
                    continue;
                }
                let mut nodes = [LatticePos::ORIGIN; 12];
                let mut count = 0;
                for (class, _) in classes.iter().enumerate().filter(|(_, struck)| **struck) {
                    nodes[count] = map.node(class as i64);
                    count += 1;
                }
                let mut cost = FIFTH_STEP * f64::from(df.abs()) + THIRD_STEP * f64::from(dt.abs());
                for (i, &node) in nodes[..count].iter().enumerate() {
                    cost +=
                        nodes[i + 1..count].iter().map(|&other| excess(other - node)).sum::<f64>();
                    cost += context
                        .iter()
                        .filter_map(|c| c.node.map(|other| c.weight * excess(other - node)))
                        .sum::<f64>();
                }
                let distance = height(offset);
                let tied = (cost - best.0).abs() <= 1e-9;
                if cost < best.0 - 1e-9 || (tied && distance > best.1) {
                    best = (cost, distance, offset);
                }
            }
        }
        best.2
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

    /// The table [`excess`] measures against is the brute-force answer.
    #[test]
    fn simplest_spellings_are_the_least_complex_of_each_class() {
        for (class, &(f, t)) in SIMPLEST.iter().enumerate() {
            let least = (-8..=8)
                .flat_map(|f| (-4..=4).map(move |t| LatticePos::new(f, t, 0)))
                .filter(|&v| LatticeMap::midi_class(v) == class)
                .map(height)
                .fold(f64::INFINITY, f64::min);
            assert!((height(LatticePos::new(f, t, 0)) - least).abs() < 1e-9, "class {class}");
        }
    }

    #[test]
    fn follow_mends_an_obvious_wolf_and_keeps_still_otherwise() {
        let map = LatticeMap::default();
        let both = Follow::ThirdsAndFifths;
        let chord = |offset: LatticePos, keys: &[i64]| {
            let map = LatticeMap { position: map.position + offset, ..map };
            keys.iter().map(|&k| map.node(k)).collect::<Vec<_>>()
        };
        let pure = |nodes: &[LatticePos]| {
            nodes.iter().all(|&a| nodes.iter().all(|&b| excess(b - a) < 1e-9 || a == b))
        };
        let (d_minor, c_major, a_flat) = ([62, 65, 69], [60, 64, 67], [68, 72, 75]);
        assert!(!pure(&chord(LatticePos::ORIGIN, &d_minor)), "the fixture must hold the wolf");
        // D F A from rest: one fifths step makes D 10/9 and the triad pure.
        let moved = map.follow(both, LatticePos::ORIGIN, &[], &d_minor);
        assert_eq!(moved, LatticePos::new(-1, 0, 0));
        assert!(pure(&chord(moved, &d_minor)));
        assert_eq!(
            map.follow(Follow::Thirds, LatticePos::ORIGIN, &[], &d_minor),
            LatticePos::ORIGIN
        );
        assert_eq!(map.follow(Follow::Off, moved, &[], &c_major), LatticePos::ORIGIN);
        // A chord already pure, and a lone note, never move the map.
        assert_eq!(map.follow(both, LatticePos::ORIGIN, &[], &c_major), LatticePos::ORIGIN);
        assert_eq!(map.follow(both, moved, &[], &[61]), moved);
        // Ab major takes a thirds step rather than a diminished fourth.
        let flat = map.follow(Follow::Thirds, LatticePos::ORIGIN, &[], &a_flat);
        assert_eq!(flat, LatticePos::new(0, -1, 0));
        assert!(pure(&chord(flat, &a_flat)));
        // G B D after the moved D minor follows its 10/9 D down a comma rather
        // than returning, whether that D is still held or was just released,
        // and with no context at all returning and drifting tie and it drifts.
        let context = |nodes: Vec<LatticePos>| {
            nodes
                .into_iter()
                .map(|node| ContextPitch { pitch: 0, node: Some(node), weight: 1.0 })
                .collect::<Vec<_>>()
        };
        let drifted = LatticePos::new(-2, 0, 0);
        let held = chord(moved, &[62]);
        assert_eq!(map.follow(both, moved, &context(held.clone()), &[67, 71]), drifted);
        assert!(pure(&[chord(drifted, &[67, 71]), held].concat()));
        let released = context(chord(moved, &d_minor));
        assert_eq!(map.follow(both, moved, &released, &[67, 71, 74]), drifted);
        assert_eq!(map.follow(both, moved, &[], &[67, 71, 74]), drifted);
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
