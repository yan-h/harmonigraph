//! Adaptive for spelled input: every spelling takes its one copy inside a
//! strip of the lattice four fifths wide, which the key places (#1896).
//!
//! A spelling `k` (its place on the chain of fifths, C = 0) has just copies
//! one syntonic comma apart along one diagonal: `(k, 0)`, `(k - 4, 1)`,
//! `(k + 4, -1)` and so on. A strip of four columns holds exactly one of them,
//! so the comma, the guess Adaptive's search kept getting wrong, is one
//! integer the key sets rather than something each chord decides. There is no
//! candidate, no score and no memory: the same spelling in the same strip is
//! the same node whenever it is struck, so nothing drifts.
//!
//! The price is the wolf. Every fourth fifth along the chain is the one that
//! leaves the strip's sharp edge, 40/27 rather than 3/2: with the strip on
//! F C G D that is D–A, F♯–C♯ and B♭–F.
use crate::{LatticePos, Tuning};

/// The strip's flat column at Position 0: F C G D, the default Lattice Map's
/// columns, so its notes are that map's continued up and down.
pub const HOME: i32 = -1;

/// The flat columns the strip may stand at: every column within eleven
/// fifths of C, the board's E𝄫 to C𝄪 less its outermost spellings (#1896
/// §3.1). No note is then more than about 59¢ from its key's pitch on a
/// quarter-comma board, whatever its spelling, since that distance depends on
/// the column alone.
const PLACES: std::ops::RangeInclusive<i32> = -11..=8;

/// The flat column of the strip Lattice Map's Position fifths `position`
/// places, clamped to [`PLACES`].
pub fn flat_column(position: i32) -> i32 {
    HOME.saturating_add(position).clamp(*PLACES.start(), *PLACES.end())
}

/// The node spelling `fifths` takes in the strip whose flat column is `flat`:
/// its one copy with `flat <= threes <= flat + 3`.
pub fn node(fifths: i32, flat: i32) -> LatticePos {
    let row = (fifths - flat).div_euclid(4);
    LatticePos::new(fifths - 4 * row, row, 0)
}

/// The node spelling `fifths` takes in the strip from `flat`, and what to add
/// to `declared`, the onset's pitch in microcents as its key declares it (the
/// chain of fifths at the Decoded fifth), so that it sounds that node in
/// `tuning`, in the octave nearest the one it was played in: the strip's
/// [`LatticeMap::assignment`](crate::lattice_map::LatticeMap::assignment).
pub fn assignment(fifths: i32, flat: i32, declared: i64, tuning: Tuning) -> (LatticePos, i64) {
    let node = node(fifths, flat);
    let octave = i64::from(crate::tuning::OCTAVE_MICROCENTS);
    let class = i64::from(tuning.pitch_class(node).microcents());
    let octaves = (declared - class + octave / 2).div_euclid(octave);
    (node, class + octaves * octave - declared)
}

/// The follower's constants, for the ear to settle (#1896 §3.4): a spelling's
/// count of recent attacks halves once per this many seconds between attacks.
/// Not Adaptive's Half-life, which is read at a chord's timescale.
pub const KEY_HALF_LIFE_S: f64 = 8.0;
/// The count the spelling just past an edge of the key's seven needs before
/// the strip slides toward it.
pub const SLIDE_STRIKES: f64 = 3.0;
/// How many times the count of the spelling at the window's other edge it
/// needs: F♯ against F, B♭ against B.
pub const SLIDE_MARGIN: f64 = 2.0;
/// The count under which a spelling is out of use, so a slide back toward the
/// set key that pushes it out of the window may go ahead.
pub const OUT_OF_USE: f64 = 0.25;

/// The spellings a decoded `(channel, key)` can carry
/// ([`fifths_for_midi`](crate::tuning::fifths_for_midi)): sixteen banks of
/// twelve, from Ch 9's D♭ to Ch 8's F♯.
const LOWEST: i32 = -5 - 12 * 8;
const SPELLINGS: usize = 16 * 12;

/// Follows the spellings in use by sliding the strip a fifth at a time away
/// from where the Position fifths set it, as a key change would (§3.4): the
/// seven spellings from the strip's flat column are the key's, and the strip
/// slides toward a spelling just past either edge once it has replaced the
/// spelling at the other, and back toward the set key once that spelling
/// has fallen out of use. It reads spellings and never an interval.
#[derive(Clone, Debug)]
pub struct Follower {
    /// Each spelling's count of attacks, as of the attack sample beside it.
    counts: [(f64, i64); SPELLINGS],
    /// Fifths the strip has slid from its set place.
    offset: i32,
}
impl Default for Follower {
    fn default() -> Self {
        Self { counts: [(0.0, 0); SPELLINGS], offset: 0 }
    }
}
impl Follower {
    /// Back to the set key, with nothing counted: every reset of the context.
    pub fn clear(&mut self) {
        *self = Self::default();
    }

    /// Fifths the strip has slid from its set place.
    pub fn offset(&self) -> i32 {
        self.offset
    }

    fn count(&self, fifths: i32, at: i64, per_second: f64) -> f64 {
        let Some(&(count, then)) =
            usize::try_from(fifths - LOWEST).ok().and_then(|i| self.counts.get(i))
        else {
            return 0.0;
        };
        // A steady clock that went back across a reactivation ages nothing.
        count * 0.5f64.powf((at - then).max(0) as f64 / per_second / KEY_HALF_LIFE_S)
    }

    /// Counts an attack of `fifths` at sample `at`, then slides the strip at
    /// most one fifth, and returns the flat column the attack is tuned in.
    /// `position` is the Position fifths the strip is set by, and `holds`
    /// says whether a sounding note sits in a column: a slide that would move
    /// a held note's column waits for an attack after it ends.
    pub fn strike(
        &mut self,
        fifths: i32,
        at: i64,
        per_second: f64,
        position: i32,
        holds: impl Fn(i32) -> bool,
    ) -> i32 {
        if let Ok(i) = usize::try_from(fifths - LOWEST) {
            if i < SPELLINGS {
                self.counts[i] = (self.count(fifths, at, per_second) + 1.0, at);
            }
        }
        let flat = flat_column(position + self.offset);
        let count = |fifths| self.count(fifths, at, per_second);
        // The window is flat..=flat + 6; a step sharp pushes out `flat` and
        // moves its column, a step flat pushes out `flat + 6` and moves
        // column `flat + 3`.
        let replaced = |past: i32, other: i32| {
            count(past) >= SLIDE_STRIKES && count(past) >= SLIDE_MARGIN * count(other)
        };
        // Toward a spelling that has replaced the one opposite; failing that,
        // back toward the set key once the spelling that step pushes out is
        // out of use.
        let toward = match (replaced(flat + 7, flat), replaced(flat - 1, flat + 6)) {
            (true, _) => 1,
            (false, true) => -1,
            (false, false) => 0,
        };
        let back = match self.offset.signum() {
            1 if count(flat + 6) < OUT_OF_USE => -1,
            -1 if count(flat) < OUT_OF_USE => 1,
            _ => 0,
        };
        let step = if toward != 0 { toward } else { back };
        let moved = flat_column(position + self.offset + step);
        let column = if step > 0 { flat } else { flat + 3 };
        if moved != flat && !holds(column) {
            self.offset += step;
            return moved;
        }
        flat
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Every spelling the board has lands in the strip, spelled as played,
    /// one comma copy per spelling: C♯ and D♭ are different nodes.
    #[test]
    fn each_spelling_takes_its_one_copy_inside_the_strip() {
        for flat in PLACES {
            for fifths in -40..=40 {
                let node = node(fifths, flat);
                assert!((flat..=flat + 3).contains(&node.threes), "{fifths} in {flat}: {node:?}");
                assert_eq!(node.threes + 4 * node.fives, fifths, "spelling kept");
                assert_eq!(node.sevens, 0);
            }
        }
        // F C G D: the default map's C♯ is 25/24 and its D♭ 16/15.
        assert_eq!(node(7, HOME), LatticePos::new(-1, 2, 0));
        assert_eq!(node(-5, HOME), LatticePos::new(-1, -1, 0));
        assert_eq!(node(3, HOME), LatticePos::new(-1, 1, 0), "A is 5/3");
        assert_eq!(node(3, HOME + 1), LatticePos::new(3, 0, 0), "a fifth sharper, 27/16");
    }

    /// The node sounds at its own pitch in the octave it was played in, on
    /// either side of the played pitch: a quarter-comma A lies above 5/3 and
    /// a quarter-comma G below 3/2.
    #[test]
    fn a_node_sounds_its_own_pitch_in_the_octave_played() {
        let tuning = Tuning { c_offset: 3_000_000, ..Tuning::just() };
        let octave = i64::from(crate::tuning::OCTAVE_MICROCENTS);
        let fifth: i64 = 696_578_428;
        for (node, fifths, key) in
            [(LatticePos::new(-1, 1, 0), 3, 69), (LatticePos::new(1, 0, 0), 1, 67)]
        {
            let class = i64::from(tuning.c_offset)
                + i64::from(node.threes) * i64::from(tuning.three)
                + i64::from(node.fives) * i64::from(tuning.five);
            let chain = (fifths * fifth).rem_euclid(octave);
            let declared = key / 12 * octave + chain + i64::from(tuning.c_offset);
            let (assigned, correction) = assignment(fifths as i32, HOME, declared, tuning);
            assert_eq!(assigned, node);
            let sounding = declared + correction;
            assert_eq!((sounding - class).rem_euclid(octave), 0, "{node:?}");
            assert!((sounding - declared).abs() < 50_000_000, "{node:?} at {sounding}");
        }
    }

    const RATE: f64 = 48_000.0;
    const F: i32 = -1;
    const F_SHARP: i32 = 6;
    const C: i32 = 0;

    fn second(n: f64) -> i64 {
        (n * RATE) as i64
    }

    /// Strikes on one sample that reach [`SLIDE_STRIKES`].
    fn strikes() -> usize {
        SLIDE_STRIKES.ceil() as usize
    }

    /// F♯ as often as F moves nothing; once both have faded, F♯ slides the
    /// strip a fifth at the strike that reaches the count, from F C G D to
    /// C G D A, and not before.
    #[test]
    fn the_strip_slides_once_a_spelling_past_the_key_has_replaced_the_one_opposite() {
        let free = |_| false;
        let mut follower = Follower::default();
        for fifths in [F, F_SHARP] {
            for _ in 0..strikes() {
                assert_eq!(
                    follower.strike(fifths, 0, RATE, 0, free),
                    HOME,
                    "F♯ only as often as F"
                );
            }
        }
        let at = second(10.0 * KEY_HALF_LIFE_S);
        for _ in 1..strikes() {
            assert_eq!(follower.strike(F_SHARP, at, RATE, 0, free), HOME);
        }
        assert_eq!(follower.strike(F_SHARP, at, RATE, 0, free), HOME + 1, "slid to C G D A");
        assert_eq!(follower.offset(), 1);
    }

    /// Once the spelling that took the strip away is out of use, the strip
    /// steps back toward the set key, one fifth an attack.
    #[test]
    fn the_strip_returns_toward_the_set_key_once_its_reason_is_out_of_use() {
        let free = |_| false;
        let mut follower = Follower::default();
        for _ in 0..strikes() {
            follower.strike(F_SHARP, 0, RATE, 0, free);
        }
        assert_eq!(follower.offset(), 1);
        // The half-lives F♯'s count takes to fall to `OUT_OF_USE`.
        let fade = (strikes() as f64 / OUT_OF_USE).log2() * KEY_HALF_LIFE_S;
        assert_eq!(follower.strike(C, second(0.95 * fade), RATE, 0, free), HOME + 1, "in use");
        assert_eq!(follower.strike(C, second(1.05 * fade), RATE, 0, free), HOME, "back home");
        assert_eq!(follower.offset(), 0);
    }

    /// A slide that would move a held note's column waits for an attack after
    /// that note ends; one that moves another column goes ahead.
    #[test]
    fn a_slide_waits_while_a_note_in_its_column_is_held() {
        let mut follower = Follower::default();
        let held_f = |column| column == HOME;
        for _ in 0..strikes() {
            assert_eq!(follower.strike(F_SHARP, 0, RATE, 0, held_f), HOME);
        }
        let held_d = |column| column == HOME + 3;
        assert_eq!(follower.strike(F_SHARP, 0, RATE, 0, held_d), HOME + 1, "F let go");
    }
}
