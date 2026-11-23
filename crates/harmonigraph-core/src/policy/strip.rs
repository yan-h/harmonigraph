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

/// Follows the music by moving the strip a fifth at a time, and only to keep
/// a new note pure (§3.4; simplified on 2026-11-23, `docs/intent/tuning.md`).
#[derive(Clone, Copy, Debug, Default)]
pub struct Follower {
    /// Fifths the strip stands from where the Position places it.
    moved: i32,
    /// The node the last note took: the context when nothing is held, so a
    /// staccato arpeggio is heard as one.
    last: Option<LatticePos>,
}
impl Follower {
    /// Back to the Position: every reset of the context.
    pub fn clear(&mut self) {
        *self = Self::default();
    }

    /// The flat column the strip stands at under the Position fifths
    /// `position`.
    pub fn flat(&self, position: i32) -> i32 {
        moved_column(position, self.moved)
    }

    /// Fifths the strip stands from where the Position fifths `position`
    /// place it.
    pub fn moved(&self, position: i32) -> i32 {
        self.flat(position) - flat_column(position)
    }

    /// An attack group's spellings `struck`, over the nodes the held notes
    /// sound at, `held`, before any of the group is tuned. The strip stays
    /// where it is if every fifth and minor third the struck notes make, with
    /// each other and with the context, sounds pure there; otherwise it moves
    /// the fewest fifths that make them pure. The context is the held notes,
    /// or with nothing held the last note. So D over a held F major moves it
    /// a fifth flat, D 10/9 under A, and E♭ after a C in its sharpest column a
    /// fifth sharp, a 6/5. Held notes keep the node they sound at.
    pub fn attack(&mut self, struck: &[i32], held: &[LatticePos], position: i32) {
        let context = if held.is_empty() { self.last.as_slice() } else { held };
        let pure = |moved: i32| {
            let flat = moved_column(position, moved);
            struck.iter().enumerate().all(|(i, &k)| {
                let others = struck[i + 1..].iter().map(|&j| node(j, flat));
                others.chain(context.iter().copied()).all(|other| in_tune(node(k, flat), other))
            })
        };
        if !pure(self.moved) {
            // Outward a fifth at a time, so a step the clamp would not let
            // the strip sound is never the one found.
            let reach = PLACES.end() - PLACES.start();
            let steps = (1..=reach).flat_map(|d| [-d, d]);
            if let Some(step) = steps.into_iter().find(|&step| pure(self.moved + step)) {
                self.moved += step;
            }
        }
        self.last = struck.last().map(|&k| node(k, self.flat(position)));
    }
}

/// Whether two nodes a fifth or a minor third apart by spelling sound it
/// pure: 3/2 or 6/5, either way up. Other intervals pass.
fn in_tune(a: LatticePos, b: LatticePos) -> bool {
    let spelling = |n: LatticePos| n.threes + 4 * n.fives;
    let (step, low, high) = match spelling(b) - spelling(a) {
        1 | -3 => (spelling(b) - spelling(a), a, b),
        -1 | 3 => (spelling(a) - spelling(b), b, a),
        _ => return true,
    };
    high - low == if step == 1 { LatticePos::new(1, 0, 0) } else { LatticePos::new(1, -1, 0) }
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

    /// Whether every fifth and minor third among `nodes` sounds pure.
    fn pure(nodes: &[LatticePos]) -> bool {
        nodes.iter().all(|&a| nodes.iter().all(|&b| in_tune(a, b)))
    }

    /// Plays `chord` one note at a time in `order`, held to the chord's end
    /// or `staccato`: the nodes it sounded.
    fn arpeggio(
        follower: &mut Follower,
        chord: &[i32],
        order: &[usize],
        staccato: bool,
    ) -> Vec<LatticePos> {
        let mut sounded: Vec<LatticePos> = Vec::new();
        for &i in order {
            follower.attack(&[chord[i]], if staccato { &[] } else { &sounded }, 0);
            sounded.push(node(chord[i], follower.flat(0)));
        }
        sounded
    }

    const MAJOR: fn(i32) -> [i32; 3] = |root| [root, root + 4, root + 1];
    const MINOR: fn(i32) -> [i32; 3] = |root| [root, root - 3, root + 1];

    /// Major and minor triads moving by a fifth, up and down from C, each
    /// arpeggiated twice in every order, held or staccato: the second time
    /// each triad is pure.
    #[test]
    fn a_triad_moved_a_fifth_is_pure_the_second_time_in_any_order() {
        let orders = [[0, 1, 2], [0, 2, 1], [1, 0, 2], [1, 2, 0], [2, 0, 1], [2, 1, 0]];
        let roots = [[0, 1, 2, 3, 4], [0, -1, -2, -3, -4]];
        for (triad, roots) in [MAJOR, MINOR].into_iter().flat_map(|t| roots.map(|r| (t, r))) {
            for (order, staccato) in orders.into_iter().flat_map(|o| [(o, false), (o, true)]) {
                let mut follower = Follower::default();
                for root in roots {
                    arpeggio(&mut follower, &triad(root), &order, staccato);
                    let again = arpeggio(&mut follower, &triad(root), &order, staccato);
                    assert!(pure(&again), "{:?} {order:?} staccato {staccato}", triad(root));
                }
            }
        }
    }

    /// Yan's cases: F then A♭ and C minor then B leave the strip; C then E♭,
    /// with C in the strip's sharpest column, moves it sharp; C minor then F,
    /// with C in its flat column, and D over a held F major move it flat; and
    /// C then F, from wherever the strip and the note before left C, never
    /// sound the wolf.
    #[test]
    fn a_note_out_of_tune_with_its_context_moves_the_strip() {
        let play = |moved: i32, notes: &[i32], staccato: bool| {
            let mut follower = Follower { moved, ..Follower::default() };
            arpeggio(&mut follower, notes, &(0..notes.len()).collect::<Vec<_>>(), staccato);
            follower.moved - moved
        };
        for staccato in [false, true] {
            assert_eq!(play(0, &[-1, -4], staccato), 0, "F then A♭");
            assert_eq!(play(0, &[0, -3, 1, 5], staccato), 0, "C E♭ G then B");
            assert_eq!(play(-2, &[0, -3], staccato), 1, "C then E♭");
        }
        assert_eq!(play(1, &[0, -3, 1, -1], false), -1, "C E♭ G then F");
        assert_eq!(play(0, &[-1, 3, 0, 2], false), -1, "F A C then D");
        for moved in -6..=6 {
            for before in -6..=7 {
                let mut follower = Follower { moved, ..Follower::default() };
                let sounded = arpeggio(&mut follower, &[before, 0, -1], &[0, 1, 2], true);
                assert!(in_tune(sounded[1], sounded[2]), "C then F from {moved} after {before}");
            }
        }
    }

    /// 2,000 random major and minor triads on every root from G♭ to F♯,
    /// held or staccato, each arpeggiated twice: the second time is pure.
    #[test]
    fn random_triads_are_pure_the_second_time() {
        let mut seed = 1u32;
        let mut next = |n: usize| {
            seed = seed.wrapping_mul(1_103_515_245).wrapping_add(12_345);
            (seed >> 16) as usize % n
        };
        let orders = [[0, 1, 2], [0, 2, 1], [1, 0, 2], [1, 2, 0], [2, 0, 1], [2, 1, 0]];
        for staccato in [false, true] {
            let mut follower = Follower::default();
            for _ in 0..2_000 {
                let triad = [MAJOR, MINOR][next(2)](next(13) as i32 - 6);
                let order = orders[next(6)];
                arpeggio(&mut follower, &triad, &order, staccato);
                let again = arpeggio(&mut follower, &triad, &order, staccato);
                assert!(pure(&again), "{triad:?} {order:?} staccato {staccato}");
            }
        }
    }
}
