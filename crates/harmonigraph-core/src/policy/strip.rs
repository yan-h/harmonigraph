//! Adaptive for spelled input: every spelling takes its one copy inside a
//! strip of the lattice four fifths wide, which the key places (#1896).
//!
//! A spelling `k` (its place on the chain of fifths, C = 0) has just copies
//! one syntonic comma apart along one diagonal: `(k, 0)`, `(k - 4, 1)`,
//! `(k + 4, -1)` and so on. A strip of four columns holds exactly one of them,
//! so the comma, the guess Adaptive's search kept getting wrong, is one
//! integer the key sets rather than something each chord decides. There is no
//! candidate and no score: the same spelling in the same strip is the same
//! node whenever it is struck, so nothing drifts.
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

/// The flat column `moved` fifths from where the Position fifths `position`
/// place the strip, clamped to [`PLACES`].
pub fn moved_column(position: i32, moved: i32) -> i32 {
    flat_column(position).saturating_add(moved).clamp(*PLACES.start(), *PLACES.end())
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

/// Follows the spellings in use by moving the key, a whole number of fifths
/// from where the Position sets the strip (§3.4; simplified on 2026-11-23,
/// `docs/intent/tuning.md`). The key's seven spellings run from the strip's
/// flat column, F to B at Position 0.
#[derive(Clone, Copy, Debug, Default)]
pub struct Follower {
    /// Fifths the key stands from where the Position places the strip, only
    /// as far as [`PLACES`] lets it.
    key: i32,
}
impl Follower {
    /// Back to the Position: every reset of the context.
    pub fn clear(&mut self) {
        *self = Self::default();
    }

    /// Fifths the key stands from the Position.
    pub fn key(&self) -> i32 {
        self.key
    }

    /// The flat column the strip stands at under the Position fifths
    /// `position`.
    pub fn flat(&self, position: i32) -> i32 {
        moved_column(position, self.key)
    }

    /// An attack group's spellings `struck`, over the held notes' spellings
    /// `held`, before any of the group is tuned. Two steps:
    ///
    /// - **Bring the struck notes inside.** The key moves the fewest fifths
    ///   that put every struck spelling inside its seven: F♯ in C moves it a
    ///   fifth sharp, B♭ a fifth flat. A group no seven spellings hold
    ///   moves nothing.
    /// - **Clear a wolf.** If two sounding spellings a fifth apart straddle
    ///   the strip's sharp edge (D–A at Position 0), the key moves a fifth to
    ///   the side where no sounding fifth is a wolf and its seven still hold
    ///   every sounding spelling, if exactly one side does: D over a held F
    ///   major moves it flat. D and A alone, which either side clears, wait
    ///   for the third.
    ///
    /// Held notes keep the node they sound at.
    pub fn attack(&mut self, struck: &[i32], held: &[i32], position: i32) {
        let (Some(&low), Some(&high)) = (struck.iter().min(), struck.iter().max()) else {
            return;
        };
        let flat = self.flat(position);
        let step = if high - low > 6 {
            0
        } else if high > flat + 6 {
            high - flat - 6
        } else {
            (low - flat).min(0)
        };
        self.step(step, position);
        let sounding = || struck.iter().chain(held);
        let wolf = |flat: i32| {
            sounding().any(|&k| sounding().any(|&j| j == k + 1) && (k - flat).rem_euclid(4) == 3)
        };
        let clears = |flat: i32| !wolf(flat) && sounding().all(|&k| (flat..=flat + 6).contains(&k));
        if wolf(self.flat(position)) {
            let sides = [-1, 1].map(|side| clears(moved_column(position, self.key + side)));
            if sides[0] != sides[1] {
                self.step(if sides[0] { -1 } else { 1 }, position);
            }
        }
    }

    /// Moves the key `step` fifths, keeping only what the clamp at
    /// [`PLACES`] lets the strip sound.
    fn step(&mut self, step: i32, position: i32) {
        self.key = moved_column(position, self.key.saturating_add(step)) - flat_column(position);
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

    /// The fifths among `chord` that sound wolves in the strip from `flat`.
    fn wolves(chord: &[i32], flat: i32) -> usize {
        let pure = |k: i32| node(k + 1, flat) - node(k, flat) == LatticePos::new(1, 0, 0);
        chord.iter().filter(|&&k| chord.contains(&(k + 1)) && !pure(k)).count()
    }

    /// Plays `chord` one note at a time in `order`, each over the ones before.
    fn arpeggio(follower: &mut Follower, chord: [i32; 3], order: [usize; 3]) {
        for (count, &i) in order.iter().enumerate() {
            let held: Vec<i32> = order[..count].iter().map(|&i| chord[i]).collect();
            follower.attack(&[chord[i]], &held, 0);
        }
    }

    /// Major and minor triads moving by a fifth, up and down from C, each
    /// played as a staggered arpeggio in every order: the key moves the way
    /// they move, and the next time the triad is played every fifth in it is
    /// pure. Struck together, they are pure at once.
    #[test]
    fn a_triad_moved_a_fifth_moves_the_key_in_any_order() {
        let major = |root: i32| [root, root + 4, root + 1];
        let minor = |root: i32| [root, root - 3, root + 1];
        let orders = [[0, 1, 2], [0, 2, 1], [1, 0, 2], [1, 2, 0], [2, 0, 1], [2, 1, 0]];
        type Triad = fn(i32) -> [i32; 3];
        let cases: [(Triad, [i32; 5], [i32; 5]); 4] = [
            (major, [0, 1, 2, 3, 4], [0, 0, 1, 2, 3]),
            (major, [0, -1, -2, -3, -4], [0, 0, -1, -2, -3]),
            (minor, [3, 4, 5, 6, 7], [0, 0, 1, 2, 3]),
            (minor, [3, 2, 1, 0, -1], [0, -1, -2, -3, -4]),
        ];
        for (triad, roots, keys) in cases {
            for order in orders {
                let mut follower = Follower::default();
                for (root, key) in roots.into_iter().zip(keys) {
                    arpeggio(&mut follower, triad(root), order);
                    assert_eq!(follower.key(), key, "{:?} {order:?}", triad(root));
                    assert_eq!(wolves(&triad(root), follower.flat(0)), 0, "{:?}", triad(root));
                }
            }
        }
        for chord in [[2, 6, 3], [-2, 2, -1], [-3, 1, -2], [3, 7, 4], [2, -1, 3]] {
            let mut follower = Follower::default();
            follower.attack(&chord, &[], 0);
            assert_eq!(wolves(&chord, follower.flat(0)), 0, "{chord:?} struck together");
        }
    }

    /// D struck over a held F major moves the key flat, so D–A is pure; D
    /// over a held A alone waits, as either side clears it, and F♯ then
    /// moves it sharp. A pentatonic set, whose wolf no side clears, and C
    /// major's notes never move it.
    #[test]
    fn a_wolf_moves_the_key_to_the_one_side_that_clears_it() {
        let mut follower = Follower::default();
        follower.attack(&[2], &[-1, 3, 0], 0);
        assert_eq!((follower.key(), wolves(&[-1, 3, 0, 2], follower.flat(0))), (-1, 0), "F A C D");
        let mut follower = Follower::default();
        follower.attack(&[2], &[3], 0);
        assert_eq!(follower.key(), 0, "D over A");
        follower.attack(&[6], &[3, 2], 0);
        assert_eq!(follower.key(), 1, "then F♯");
        let mut follower = Follower::default();
        for k in [0, 2, 4, 1, 3] {
            follower.attack(&[k], &[0, 2, 4, 1, 3], 0);
        }
        follower.attack(&[0, 4, 1], &[], 0);
        assert_eq!(follower.key(), 0, "pentatonic, then C major");
    }

    /// A group no seven spellings hold moves nothing, and the key stores no
    /// movement past the clamp, so back at Position 0 the strip is home.
    #[test]
    fn the_key_moves_only_where_it_can_hold_the_group() {
        let mut follower = Follower::default();
        follower.attack(&[-2, 6], &[], 0);
        assert_eq!(follower.key(), 0, "B♭ and F♯ together");
        follower.attack(&[15, 16, 19], &[], 20);
        assert_eq!((follower.flat(20), follower.flat(0)), (flat_column(20), HOME));
    }
}
