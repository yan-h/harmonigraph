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

/// The count rule's constants, for the ear to settle (#1896 §3.4): a
/// spelling's count of recent attacks halves once per this many seconds.
pub const KEY_HALF_LIFE_S: f64 = 8.0;
/// The count the spelling just past an edge of the key's seven needs before
/// the key moves toward it.
pub const SLIDE_STRIKES: f64 = 3.0;
/// How many times the count of the spelling at the window's other edge it
/// needs: F♯ against F, B♭ against B.
pub const SLIDE_MARGIN: f64 = 2.0;
/// The count under which a spelling is out of use.
pub const OUT_OF_USE: f64 = 0.25;

/// The spellings a decoded `(channel, key)` can carry
/// ([`fifths_for_midi`](crate::tuning::fifths_for_midi)): sixteen banks of
/// twelve, from Ch 9's D♭ to Ch 8's F♯.
const LOWEST: i32 = -5 - 12 * 8;
const SPELLINGS: usize = 16 * 12;

/// Follows the spellings in use by moving the key, a whole number of fifths
/// from where the Position sets the strip (§3.4; simplified to this one
/// state on 2026-11-23, `docs/intent/tuning.md`). The key's seven spellings
/// run from the strip's flat column, F to B at Position 0.
#[derive(Clone, Debug)]
pub struct Follower {
    /// Each spelling's count of recent attacks, as of sample `at`.
    counts: [f64; SPELLINGS],
    at: i64,
    /// Fifths the key stands from where the Position places the strip, only
    /// as far as [`PLACES`] lets it.
    key: i32,
}
impl Default for Follower {
    fn default() -> Self {
        Self { counts: [0.0; SPELLINGS], at: 0, key: 0 }
    }
}
impl Follower {
    /// Back to the Position, with nothing counted: every reset of the context.
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

    /// The attack group at sample `at`, before any of it is tuned: counts
    /// `struck`, the group's spellings, then moves the key at most a fifth,
    /// unless that would change the column of a node in `held`.
    ///
    /// - **A chord**, two spellings or more, holding a spelling just past one
    ///   edge of the key's seven (F♯ or B♭ in C) moves it toward that one, so
    ///   D–F♯–A from C sounds pure at once; holding both, it does not move.
    /// - **Otherwise the count rule**, so a lone passing tone moves nothing:
    ///   toward a spelling just past an edge once it has replaced the one at
    ///   the other edge, and back toward the Position once the spelling that
    ///   step pushes out is out of use.
    pub fn attack(
        &mut self,
        struck: &[i32],
        held: &[LatticePos],
        at: i64,
        per_second: f64,
        position: i32,
    ) {
        // A steady clock that went back across a reactivation ages nothing.
        let decay = 0.5f64.powf((at - self.at).max(0) as f64 / per_second / KEY_HALF_LIFE_S);
        self.counts.iter_mut().for_each(|count| *count *= decay);
        self.at = at;
        let index = |k: i32| usize::try_from(k - LOWEST).ok().filter(|&i| i < SPELLINGS);
        struck.iter().filter_map(|&k| index(k)).for_each(|i| self.counts[i] += 1.0);
        let count = |k| index(k).map_or(0.0, |i| self.counts[i]);
        let replaced = |past: i32, other: i32| {
            count(past) >= SLIDE_STRIKES && count(past) >= SLIDE_MARGIN * count(other)
        };
        let set = flat_column(position);
        let flat = moved_column(position, self.key);
        let chord = struck.iter().any(|&k| k != struck[0]);
        let sharp = struck.contains(&(flat + 7));
        let flat_side = struck.contains(&(flat - 1));
        let step = if chord && (sharp || flat_side) {
            i32::from(sharp) - i32::from(flat_side)
        } else if replaced(flat + 7, flat) || flat < set && count(flat) < OUT_OF_USE {
            1
        } else if replaced(flat - 1, flat + 6) || flat > set && count(flat + 6) < OUT_OF_USE {
            -1
        } else {
            0
        };
        let moved = moved_column(position, self.key + step);
        let holds = held.iter().all(|node| (moved..=moved + 3).contains(&node.threes));
        // Only movement the clamp lets through is kept.
        self.key = if holds { moved } else { flat } - set;
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
    const F_SHARP: i32 = 6;
    const STRIKES: usize = SLIDE_STRIKES.ceil() as usize;
    const C: [i32; 3] = [0, 4, 1];

    /// One attack group at `seconds` over `held` spellings, at Position 0:
    /// the flat column it is tuned in.
    fn group(follower: &mut Follower, struck: &[i32], held: &[i32], seconds: f64) -> i32 {
        let held: Vec<_> = held.iter().map(|&k| node(k, HOME)).collect();
        follower.attack(struck, &held, (seconds * RATE) as i64, RATE, 0);
        follower.flat(0)
    }

    /// A follower whose key lone F♯s have moved to G.
    fn in_g() -> Follower {
        let mut follower = Follower::default();
        (0..STRIKES).for_each(|_| _ = group(&mut follower, &[F_SHARP], &[], 0.0));
        follower
    }

    /// The fifths among `chord` that sound wolves in the strip from `flat`.
    fn wolves(chord: &[i32], flat: i32) -> usize {
        let pure = |k: i32| node(k + 1, flat) - node(k, flat) == LatticePos::new(1, 0, 0);
        chord.iter().filter(|&&k| chord.contains(&(k + 1)) && !pure(k)).count()
    }

    /// The count rule: F♯ as often as F moves nothing; once both have faded,
    /// F♯ moves the key at the strike that reaches the count, and back once
    /// F♯ is out of use.
    #[test]
    fn the_count_rule_moves_the_key_and_brings_it_back() {
        let mut follower = Follower::default();
        for fifths in [-1, F_SHARP].repeat(STRIKES) {
            assert_eq!(group(&mut follower, &[fifths], &[], 0.0), HOME, "as often as F");
        }
        let faded = 10.0 * KEY_HALF_LIFE_S;
        let slides: Vec<_> =
            (0..STRIKES).map(|_| group(&mut follower, &[F_SHARP], &[], faded)).collect();
        assert_eq!(slides[STRIKES - 2..], [HOME, HOME + 1], "at the count, not before");
        let fade = faded + (STRIKES as f64 / OUT_OF_USE).log2() * KEY_HALF_LIFE_S;
        assert_eq!(group(&mut follower, &[0], &[], 0.95 * fade), HOME + 1);
        assert_eq!(group(&mut follower, &[0], &[], 1.05 * fade), HOME, "home");
    }

    /// Triads modulating by a fifth, I–IV–V–I in each key: the first chord
    /// with the new key's spelling moves the key toward it, so every chord
    /// sounds pure, and the key is home by the last C section's V. A–C♯–E
    /// from G moves it on to D; from C, two fifths past, it moves nothing.
    #[test]
    fn a_chord_with_a_foreign_spelling_moves_the_key_and_sounds_pure() {
        let [f, g, d, b_flat] = [[-1, 3, 0], [1, 5, 2], [2, 6, 3], [-2, 2, -1]];
        let up = [C, f, g, C, g, C, d, g, C, f, g, C];
        let down = [C, f, g, C, f, b_flat, C, f, C, f, g, C];
        for (progression, turn, moved) in [(up, d, 1), (down, b_flat, -1)] {
            let mut follower = Follower::default();
            let mut flats = Vec::new();
            for (index, chord) in progression.into_iter().enumerate() {
                let flat = group(&mut follower, &chord, &[], index as f64);
                assert_eq!(wolves(&chord, flat), 0, "{progression:?}, chord {index}");
                assert!(chord != turn || flat == HOME + moved, "{turn:?} moves the key");
                flats.push(flat);
            }
            assert_eq!(flats[10..], [HOME; 2], "home again");
        }
        let flat = group(&mut in_g(), &[3, 7, 4], &[], 1.0);
        assert_eq!((flat, wolves(&[3, 7, 4], flat)), (HOME + 2, 0), "A–C♯–E from G");
        // A spelling further out moves nothing: A major and C♯ minor from C
        // stay home, and pure.
        for chord in [[3, 7, 4], [7, 4, 8]] {
            let flat = group(&mut Follower::default(), &chord, &[], 0.0);
            assert_eq!((flat, wolves(&chord, flat)), (HOME, 0), "{chord:?} from C");
        }
    }

    /// Chords inside the key's seven never move it: a C pentatonic vamp and
    /// sus and quartal voicings, at the Position and at G, with E at one node
    /// throughout. A diatonic ii keeps its wolf, D–A.
    #[test]
    fn a_chord_inside_the_key_never_moves_it() {
        let voicings: [&[i32]; 6] =
            [&[3, 2, 1], &[0, 2, 4, 1, 3], &[2, 4, 1, 3], &[-1, 0, 1], &[0, 2, 1], &[2, 1, 3]];
        for (mut follower, key) in [(Follower::default(), 0), (in_g(), 1)] {
            for (index, voicing) in voicings.repeat(3).into_iter().enumerate() {
                let shifted: Vec<i32> = voicing.iter().map(|k| k + key).collect();
                let flat = group(&mut follower, &shifted, &[], index as f64 / 2.0);
                assert_eq!((flat, node(4, flat)), (HOME + key, node(4, HOME + key)), "{index}");
            }
        }
        let mut follower = Follower::default();
        for (index, chord) in [C, [2, -1, 3], [3, 0, 4], [-1, 3, 0], C].iter().enumerate() {
            let flat = group(&mut follower, chord, &[], index as f64);
            assert_eq!(wolves(chord, flat), usize::from(index == 1), "ii keeps D–A: {index}");
        }
    }

    /// A lone F♯ passing over a held C major moves nothing.
    #[test]
    fn passing_tones_do_not_move_the_key() {
        let mut follower = Follower::default();
        for bar in 0..2 {
            assert_eq!(group(&mut follower, &C, &[], bar as f64), HOME);
            assert_eq!(group(&mut follower, &[F_SHARP], &C, bar as f64 + 0.5), HOME, "{bar}");
        }
    }

    /// No move changes a held note's column: neither D major nor F♯s that
    /// have replaced F move the key over a held F; once F is let go they do,
    /// and D major over a held D does.
    #[test]
    fn a_held_note_s_column_is_never_moved() {
        let mut follower = Follower::default();
        assert_eq!(group(&mut follower, &[2, F_SHARP, 3], &[-1], 0.0), HOME, "chord over F");
        for _ in 0..STRIKES {
            assert_eq!(group(&mut follower, &[F_SHARP], &[-1], 0.0), HOME, "F held");
        }
        assert_eq!(group(&mut follower, &[F_SHARP], &[], 0.0), HOME + 1, "F let go");
        let flat = group(&mut Follower::default(), &[F_SHARP, 3], &[2], 0.0);
        assert_eq!(flat, HOME + 1, "over D");
    }

    /// A chord past the strip's sharpest place stores nothing, so back at
    /// Position 0 the strip is home.
    #[test]
    fn a_clamped_key_stores_only_the_movement_it_sounds() {
        let mut follower = Follower::default();
        follower.attack(&[15, 16, 19], &[], 0, RATE, 20);
        assert_eq!((follower.flat(20), follower.flat(0)), (flat_column(20), HOME));
    }
}
