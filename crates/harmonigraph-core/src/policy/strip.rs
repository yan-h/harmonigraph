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

/// The flat column following moves the strip to, `moved` fifths from where
/// the Position fifths `position` place it, clamped to [`PLACES`]. The
/// follower stores only movement the clamp lets through, so the strip
/// stands `moved` fifths from its set place.
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

/// A set of spellings, the ones a decoded `(channel, key)` can carry.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Spellings([u64; SPELLINGS / 64]);
impl Spellings {
    pub fn insert(&mut self, fifths: i32) {
        if let Some(i) = index(fifths) {
            self.0[i / 64] |= 1 << (i % 64);
        }
    }
    pub fn contains(&self, fifths: i32) -> bool {
        index(fifths).is_some_and(|i| self.0[i / 64] & (1 << (i % 64)) != 0)
    }
    fn len(&self) -> u32 {
        self.0.iter().map(|word| word.count_ones()).sum()
    }
    fn within(&self, other: &Spellings) -> bool {
        self.0.iter().zip(other.0).all(|(mine, theirs)| mine & !theirs == 0)
    }
    fn iter(&self) -> impl Iterator<Item = i32> + '_ {
        (0..SPELLINGS).filter(|&i| self.0[i / 64] & (1 << (i % 64)) != 0).map(|i| i as i32 + LOWEST)
    }
}
fn index(fifths: i32) -> Option<usize> {
    usize::try_from(fifths - LOWEST).ok().filter(|&i| i < SPELLINGS)
}

/// Follows the spellings in use by sliding the strip away from where the
/// Position fifths set it (§3.4), in two parts. The **key** moves a fifth at
/// a time by the count rule alone. Around it, each attack group takes an
/// **excursion** of a fifth either way or none, chosen so the group's fifths
/// sound pure where they can ([`Follower::decide`]). Every onset of a group is
/// counted ([`Follower::strike`]) before the group is decided.
#[derive(Clone, Debug)]
pub struct Follower {
    /// Each spelling's count of attacks, as of the attack sample beside it.
    counts: [(f64, i64); SPELLINGS],
    /// Fifths the key has moved from where the Position places the strip,
    /// only as far as [`PLACES`] lets it.
    key: i32,
    /// The last group's excursion from the key, −1..=1.
    excursion: i32,
    /// The last group's spellings and the flat column it was tuned in: the
    /// common tones a next group should not retune.
    last: Option<(Spellings, i32)>,
}
impl Default for Follower {
    fn default() -> Self {
        Self { counts: [(0.0, 0); SPELLINGS], key: 0, excursion: 0, last: None }
    }
}
impl Follower {
    /// Back to the set key, with nothing counted: every reset of the context.
    pub fn clear(&mut self) {
        *self = Self::default();
    }

    /// Fifths the strip stands from its set place: the key's and the last
    /// group's excursion together.
    pub fn offset(&self) -> i32 {
        self.key + self.excursion
    }

    /// The flat column the strip stands at, set by the Position fifths
    /// `position` and moved by following.
    pub fn flat(&self, position: i32) -> i32 {
        moved_column(position, self.offset())
    }

    fn count(&self, fifths: i32, at: i64, per_second: f64) -> f64 {
        let Some(&(count, then)) = index(fifths).map(|i| &self.counts[i]) else {
            return 0.0;
        };
        // A steady clock that went back across a reactivation ages nothing.
        count * 0.5f64.powf((at - then).max(0) as f64 / per_second / KEY_HALF_LIFE_S)
    }

    /// Counts an attack of `fifths` at sample `at`.
    pub fn strike(&mut self, fifths: i32, at: i64, per_second: f64) {
        if let Some(i) = index(fifths) {
            self.counts[i] = (self.count(fifths, at, per_second) + 1.0, at);
        }
    }

    /// Decides the attack group at sample `at`, once its onsets are counted
    /// and before they are tuned. `sounding` is its spellings and the held
    /// notes', `struck` the group's own, `held` each held note's spelling and
    /// the node it sounds at, and `position` the Position fifths.
    ///
    /// - **The key**, by the count rule, for single notes and melodies: the
    ///   seven spellings from the key's flat column are the key's; it moves
    ///   toward a spelling just past either edge once that has replaced the
    ///   spelling at the other, and back toward the Position once the
    ///   spelling that step pushes out has fallen out of use. It moves only
    ///   where its excursions can still hold every held note and add no wolf
    ///   the group did not already have, so the step itself adds no wolf; a
    ///   later group, whose spellings all lie inside the moved key's seven,
    ///   takes that key's strip and its wolf, which may be one the old key
    ///   left pure (Yan's (b)).
    /// - **The excursion**, −1..=1 around the key, per group: a strip that
    ///   holds every held note where it sounds, if one does; then none at all
    ///   unless the group holds a spelling outside the key's seven, so a
    ///   pentatonic or sus voicing and a diatonic ii keep the key's strip and
    ///   its wolf (Yan's (b), #1941); then the fewest wolves among the
    ///   group's fifths (D–A on F C G D), so D major there slides sharp and
    ///   sounds pure (Yan, #1896: "I run into wolf fifths when modulating up
    ///   or down by fifths playing triads only"), a struck spelling counted at
    ///   its copy in the strip as well as where any held copy sounds; then the
    ///   fewest common tones with the last group retuned, so B♭ major after F
    ///   major slides flat and keeps its F; then none toward the side away
    ///   from the foreign spelling, so B♭ major from C slides flat; then the
    ///   fewest sounding spellings retuned from where the strip last stood, so
    ///   a lone C♯ after D major stays where D major stood; then no
    ///   excursion; then nearer the Position.
    ///
    /// The strip stands at most a fifth from the key, so no run of chords
    /// walks it away, and a vamp inside the key's seven never moves it, so
    /// no note of it pumps between two commas. A
    /// chord rolled by hand is decided note by note, so one whose held notes
    /// pin the strip it needs keeps its wolf.
    pub fn decide(
        &mut self,
        at: i64,
        per_second: f64,
        sounding: &Spellings,
        struck: &Spellings,
        held: &[(i32, LatticePos)],
        position: i32,
    ) {
        // Only movement the clamp lets through is kept: a key at a clamped
        // Position would otherwise run on unseen, and land the strip far off
        // once the Position comes back.
        self.key = moved_column(position, self.key) - flat_column(position);
        let count = |fifths| self.count(fifths, at, per_second);
        let key_flat = moved_column(position, self.key);
        let replaced = |past: i32, other: i32| {
            count(past) >= SLIDE_STRIKES && count(past) >= SLIDE_MARGIN * count(other)
        };
        let toward = match (replaced(key_flat + 7, key_flat), replaced(key_flat - 1, key_flat + 6))
        {
            (true, _) => 1,
            (false, true) => -1,
            (false, false) => 0,
        };
        let back = match self.key.signum() {
            1 if count(key_flat + 6) < OUT_OF_USE => -1,
            -1 if count(key_flat) < OUT_OF_USE => 1,
            _ => 0,
        };
        let step = if toward != 0 { toward } else { back };
        let best_now = self.best(sounding, struck, held, position, self.key);
        if step != 0 {
            let moved = self.best(sounding, struck, held, position, self.key + step);
            let holds = |best: &Choice| best.holds || held.is_empty();
            if holds(&moved) && moved.wolves.within(&best_now.wolves) {
                self.key += step;
            }
        }
        let best = self.best(sounding, struck, held, position, self.key);
        self.excursion = best.flat - moved_column(position, self.key);
        self.last = Some((*sounding, best.flat));
    }

    /// The best excursion around `key` for this group, by the order
    /// [`Follower::decide`] gives.
    fn best(
        &self,
        sounding: &Spellings,
        struck: &Spellings,
        held: &[(i32, LatticePos)],
        position: i32,
        key: i32,
    ) -> Choice {
        let key_flat = moved_column(position, key);
        // The side of the key's seven, `key_flat..=key_flat + 6`, a spelling
        // outside it lies on: only a group holding one may take an excursion
        // (Yan's (b), #1941), and it leans toward that side.
        let outside = |k: i32| if k > key_flat + 6 { 1 } else { -i32::from(k < key_flat) };
        let foreign = sounding.iter().any(|k| outside(k) != 0);
        let sharp = sounding.iter().any(|k| outside(k) == 1);
        let flat_side = sounding.iter().any(|k| outside(k) == -1);
        let side = i32::from(sharp) - i32::from(flat_side);
        let choices = (-1..=1).map(|excursion| {
            let flat = moved_column(position, key + excursion);
            // Where each sounding spelling sounds: a held note where it is
            // held, and a struck one at its copy in this strip, so a spelling
            // both held and struck sounds at both.
            let copies = |k: i32| {
                let struck = struck.contains(k).then(|| node(k, flat));
                held.iter().filter(move |&&(s, _)| s == k).map(|&(_, n)| n).chain(struck)
            };
            let holds = held.iter().all(|&(k, n)| node(k, flat) == n);
            let mut wolves = Spellings::default();
            sounding
                .iter()
                .filter(|&k| {
                    sounding.contains(k + 1)
                        && copies(k).any(|low| {
                            copies(k + 1).any(|high| high - low != LatticePos::new(1, 0, 0))
                        })
                })
                .for_each(|k| wolves.insert(k));
            // Spellings this strip retunes from where the strip last stood:
            // first the last group's own, the common tones; then any.
            let (last, last_flat) = self.last.unwrap_or((Spellings::default(), key_flat));
            let moved = |k: i32| node(k, flat) != node(k, last_flat);
            let common = sounding.iter().filter(|&k| last.contains(k) && moved(k)).count();
            let retuned = sounding.iter().filter(|&k| moved(k)).count();
            let rank = (
                !holds,
                !foreign && excursion != 0,
                wolves.len(),
                common,
                side != 0 && excursion != 0 && excursion != side,
                retuned,
                excursion != 0,
                (key + excursion).abs(),
            );
            (rank, Choice { flat, holds, wolves })
        });
        choices.min_by_key(|(rank, _)| *rank).map(|(_, choice)| choice).unwrap()
    }
}

/// One excursion [`Follower::best`] weighed.
#[derive(Clone, Copy, Debug)]
struct Choice {
    flat: i32,
    /// Every held note sounds at its node in this strip.
    holds: bool,
    /// The fifths it leaves wolves, by their lower spelling.
    wolves: Spellings,
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

    /// One attack group struck at `at`, over `held` notes (each spelling with
    /// the node it sounds at), at Position `position`: the flat column it is
    /// tuned in.
    fn group_at(
        follower: &mut Follower,
        struck: &[i32],
        held: &[(i32, LatticePos)],
        at: i64,
        position: i32,
    ) -> i32 {
        struck.iter().for_each(|&k| follower.strike(k, at, RATE));
        let mut sounding = Spellings::default();
        struck.iter().chain(held.iter().map(|(k, _)| k)).for_each(|&k| sounding.insert(k));
        let mut group = Spellings::default();
        struck.iter().for_each(|&k| group.insert(k));
        follower.decide(at, RATE, &sounding, &group, held, position);
        follower.flat(position)
    }
    fn group(follower: &mut Follower, struck: &[i32], at: i64) -> i32 {
        group_at(follower, struck, &[], at, 0)
    }

    /// A follower whose key three F♯s have moved to G.
    fn in_g() -> Follower {
        let mut follower = Follower::default();
        for _ in 0..strikes() {
            group(&mut follower, &[F_SHARP], 0);
        }
        assert_eq!(follower.offset(), 1, "in G");
        follower
    }

    /// The fifths among `chord` that sound wolves in the strip from `flat`.
    fn wolves(chord: &[i32], flat: i32) -> usize {
        chord
            .iter()
            .filter(|&&k| {
                chord.contains(&(k + 1))
                    && node(k + 1, flat) - node(k, flat) != LatticePos::new(1, 0, 0)
            })
            .count()
    }

    /// F♯ as often as F moves nothing; once both have faded, F♯ moves the key
    /// a fifth at the strike that reaches the count, from F C G D to
    /// C G D A, and not before.
    #[test]
    fn the_strip_slides_once_a_spelling_past_the_key_has_replaced_the_one_opposite() {
        let mut follower = Follower::default();
        for fifths in [F, F_SHARP] {
            for _ in 0..strikes() {
                assert_eq!(group(&mut follower, &[fifths], 0), HOME, "as often as F");
            }
        }
        let at = second(10.0 * KEY_HALF_LIFE_S);
        for _ in 1..strikes() {
            assert_eq!(group(&mut follower, &[F_SHARP], at), HOME);
        }
        assert_eq!(group(&mut follower, &[F_SHARP], at), HOME + 1, "C G D A");
    }

    /// Once the spelling that moved the key is out of use, it steps back
    /// toward the Position.
    #[test]
    fn the_strip_returns_toward_the_set_key_once_its_reason_is_out_of_use() {
        let mut follower = Follower::default();
        for _ in 0..strikes() {
            group(&mut follower, &[F_SHARP], 0);
        }
        assert_eq!(follower.offset(), 1);
        // The half-lives F♯'s count takes to fall to `OUT_OF_USE`.
        let fade = (strikes() as f64 / OUT_OF_USE).log2() * KEY_HALF_LIFE_S;
        assert_eq!(group(&mut follower, &[C], second(0.95 * fade)), HOME + 1);
        assert_eq!(group(&mut follower, &[C], second(1.05 * fade)), HOME, "home");
    }

    /// A held note keeps its node: with F held, the key may move, but the
    /// strip stays where F sounds until F ends.
    #[test]
    fn a_slide_waits_while_a_note_in_its_column_is_held() {
        let mut follower = Follower::default();
        let f = [(F, node(F, HOME))];
        for _ in 0..strikes() {
            assert_eq!(group_at(&mut follower, &[F_SHARP], &f, 0, 0), HOME, "F held");
        }
        assert_eq!(group(&mut follower, &[F_SHARP], 0), HOME + 1, "F let go");
    }

    /// Triad modulation slides at its first chord with a spelling outside the
    /// key's seven, toward that spelling, and sounds pure: D–F♯–A and
    /// B♭–D–F from C, A–C♯–E from G. B♭ major after F major slides flat too,
    /// keeping its F.
    #[test]
    fn a_chord_whose_fifth_straddles_the_strip_slides_it_toward_that_fifth() {
        for (chord, moved) in [([2, 6, 3], 1), ([-2, 2, -1], -1)] {
            let mut follower = Follower::default();
            assert_eq!(group(&mut follower, &[0, 4, 1], 0), HOME);
            let flat = group(&mut follower, &chord, 1);
            assert_eq!((flat, wolves(&chord, flat)), (HOME + moved, 0), "{chord:?} from C");
        }
        let mut follower = Follower::default();
        assert_eq!(group(&mut follower, &[-1, 3, 0], 0), HOME);
        assert_eq!(group(&mut follower, &[-2, 2, -1], 1), HOME - 1, "B♭ major after F major");
        let mut follower = in_g();
        let a_major = [3, 7, 4];
        let flat = group(&mut follower, &a_major, 1);
        assert_eq!((flat, wolves(&a_major, flat)), (HOME + 2, 0), "A–C♯–E from G");
    }

    /// A chord all of whose spellings lie inside the key's seven never moves
    /// the strip (Yan's (b), #1941): pentatonic and sus voicings keep the
    /// key's wolf, at the Position and at a followed key, round after round.
    #[test]
    fn a_pentatonic_voicing_never_slides_the_strip() {
        let voicings: [&[i32]; 6] =
            [&[3, 2, 1], &[0, 2, 4, 1, 3], &[2, 4, 1, 3], &[-1, 0, 1], &[0, 2, 1], &[2, 1, 3]];
        for (name, mut follower, key_flat) in
            [("C", Follower::default(), HOME), ("G", in_g(), HOME + 1)]
        {
            for round in 0..3 {
                for (index, voicing) in voicings.iter().enumerate() {
                    // In G, the same shapes a fifth up.
                    let shifted: Vec<i32> = voicing.iter().map(|k| k + key_flat - HOME).collect();
                    let at = second((round * voicings.len() + index) as f64 / 2.0);
                    let flat = group(&mut follower, &shifted, at);
                    assert_eq!(flat, key_flat, "in {name}, {shifted:?}, round {round}");
                }
            }
        }
    }

    /// I–ii–vi–IV–I in C: a diatonic ii keeps its wolf, D–A, every time; with
    /// a D major for ii, its F♯ slides the strip and D major sounds pure. C
    /// keeps 1/1 throughout.
    #[test]
    fn a_diatonic_ii_keeps_its_wolf_and_a_secondary_dominant_slides() {
        let ii: [&[i32]; 5] = [&[0, 4, 1], &[2, -1, 3], &[3, 0, 4], &[-1, 3, 0], &[0, 4, 1]];
        let secondary: [&[i32]; 5] = [&[0, 4, 1], &[2, 6, 3], &[3, 0, 4], &[-1, 3, 0], &[0, 4, 1]];
        for (progression, ii_wolves) in [(ii, 1), (secondary, 0)] {
            let mut follower = Follower::default();
            for (round, chord) in progression.into_iter().enumerate() {
                let flat = group(&mut follower, chord, second(round as f64));
                assert_eq!(node(C, flat), LatticePos::ORIGIN, "{progression:?} {round}: C 1/1");
                let expected = if round == 1 { ii_wolves } else { 0 };
                assert_eq!(wolves(chord, flat), expected, "{progression:?} {round}");
            }
        }
    }

    /// At a key the count rule followed to G, chords inside its seven take
    /// its own strip: E–A–D keeps G's wolf, A–E, and A minor keeps C at 1/1.
    #[test]
    fn a_followed_key_tunes_its_chords_around_it() {
        for chord in [[4, 3, 2], [3, 0, 4]] {
            let flat = group(&mut in_g(), &chord, 1);
            assert_eq!((flat, node(C, flat)), (HOME + 1, LatticePos::ORIGIN), "{chord:?}");
        }
    }

    /// A held note sounds where it was struck, whatever the Position since:
    /// A held from Position 0, the lane moved to +1, an E struck against it
    /// takes the strip that holds A, so E–A is a pure fifth.
    #[test]
    fn a_lane_move_keeps_a_held_note_s_fifths_pure() {
        let mut follower = Follower::default();
        let a = node(3, HOME);
        let flat = group_at(&mut follower, &[4], &[(3, a)], 0, 1);
        assert_eq!(node(4, flat) - a, LatticePos::new(1, 0, 0), "E a fifth over the held A");
    }

    /// No slide moves a held note's column: D major over a held F keeps its
    /// wolf rather than retune F; over a held D it slides.
    #[test]
    fn a_chord_slide_never_moves_a_held_note_s_column() {
        let mut follower = Follower::default();
        let flat = group_at(&mut follower, &[2, 6, 3], &[(F, node(F, HOME))], 0, 0);
        assert_eq!(node(F, flat), node(F, HOME), "F's column pinned");
        let mut follower = Follower::default();
        assert_eq!(group_at(&mut follower, &[6, 3], &[(2, node(2, HOME))], 0, 0), HOME + 1);
    }

    /// Strikes past a clamped Position move the key only as far as the strip
    /// can go: sixteen C♯s at Position +20 slide it one column, and back at
    /// the Position of C an A lands near C's strip, not eleven fifths flat;
    /// strikes past the flat edge at Position −20 store nothing.
    #[test]
    fn a_clamped_slide_stores_only_the_movement_it_makes() {
        const C_SHARP: i32 = 7;
        let mut follower = Follower::default();
        for at in 0..16 {
            group_at(&mut follower, &[C_SHARP], &[], at, 20);
        }
        assert_eq!(follower.flat(20), flat_column(20) - 1, "one column flat");
        let flat = group_at(&mut follower, &[3], &[], 16, 0);
        assert!((HOME - 1..=HOME).contains(&flat), "A lands near C's strip: {flat}");
        // At the flat edge the strip cannot move, so nothing is stored.
        let mut follower = Follower::default();
        for at in 0..16 {
            group_at(&mut follower, &[flat_column(-20) - 1], &[], at, -20);
        }
        assert_eq!(group_at(&mut follower, &[3], &[], 16, 0), HOME, "A at C's strip");
    }

    /// A spelling struck again while an earlier copy of it is held sounds at
    /// its copy in the strip, so its fifths there count: holding F from C's
    /// strip at Position +2, F–C–G–A–E keeps its struck F–C pure.
    #[test]
    fn a_struck_copy_of_a_held_spelling_is_scored_where_it_sounds() {
        let mut follower = Follower::default();
        let flat = group_at(&mut follower, &[F, C, 1, 3, 4], &[(F, node(F, HOME))], 0, 2);
        assert_eq!(node(C, flat) - node(F, flat), LatticePos::new(1, 0, 0), "F–C pure at {flat}");
    }

    /// A lone foreign spelling keeps the copy the last group tuned it near:
    /// C♯ after D major from C stays in D major's strip.
    #[test]
    fn a_lone_foreign_note_stays_where_the_last_chord_stood() {
        let mut follower = Follower::default();
        let d_major = group(&mut follower, &[2, F_SHARP, 3], 0);
        assert_eq!(d_major, HOME + 1, "D major slides sharp");
        let flat = group(&mut follower, &[7], 1);
        assert_eq!(node(7, flat), LatticePos::new(3, 1, 0), "C♯ at D major's copy");
    }

    /// A key move that would put the chord struck with it on a wolf waits:
    /// F♯ has replaced F, but A minor's A–E would be C G D A's wolf.
    #[test]
    fn a_count_slide_adds_no_wolf() {
        let mut follower = Follower::default();
        for _ in 1..strikes() {
            group(&mut follower, &[F_SHARP], 0);
        }
        let flat = group(&mut follower, &[3, 0, 4, F_SHARP], 0);
        assert_eq!(wolves(&[3, 0, 4], flat), 0, "A minor pure");
        assert_eq!(group(&mut follower, &[F_SHARP], 0), HOME + 1, "alone, it slides");
    }

    /// A C pentatonic vamp at the key G (F♯ having replaced F): A–D–G,
    /// C–D–E–G–A, D–E–G–A, three rounds. E keeps one pitch: none of them
    /// holds a spelling outside G's seven, so none slides (Yan's (b), #1941).
    /// Before (b), the chord rule purified D–E–G–A and not the pentatonic
    /// chord, and E moved between 5/4 and 81/64 each round.
    #[test]
    fn a_pentatonic_vamp_at_a_followed_key_keeps_each_pitch() {
        let mut follower = in_g();
        let vamp: [&[i32]; 3] = [&[3, 2, 1], &[0, 2, 4, 1, 3], &[2, 4, 1, 3]];
        let mut e = Vec::new();
        for round in 0..3 {
            for (index, chord) in vamp.into_iter().enumerate() {
                let flat = group(&mut follower, chord, second((3 * round + index) as f64 / 2.0));
                if chord.contains(&4) {
                    e.push(node(4, flat));
                }
            }
        }
        assert!(e.iter().all(|&node| node == e[0]), "E keeps one pitch: {e:?}");
    }
}
