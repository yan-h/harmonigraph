//! Effective musical configuration. One serialized owner applies semantic edits;
//! display and policy consumers copy the same resolved value. No serde or I/O.

use crate::lattice_map::TuningEngine;
use crate::{Comma, LearnedTuning, Tempered, Tuning};

/// Explicit interval links and Learn state. Recognition runs only on tuning edits.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct TuningModes {
    pub tempered: Tempered,
    pub learning: bool,
}

impl Default for TuningModes {
    fn default() -> Self {
        Self { tempered: Tempered { syntonic: true, septimal_kleisma: true }, learning: false }
    }
}

/// Musical controls, stored as integers so configuration equality is exact.
/// Weights are thousandths; silence and the half-life are milliseconds (zero
/// means never, and no decay, respectively).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct PolicyConfig {
    pub radius: u8,
    pub axes: u8,
    /// Cents at which the exponential pitch penalty reaches one.
    pub pitch_flexibility: u16,
    /// Held and released contributions alike halve in weight once per this
    /// long between their attack and the newest attack in context. A release
    /// does not restart it.
    pub half_life_ms: u16,
    /// Every octave between a context note and the note being scored
    /// multiplies that note's harmonic vote by this.
    pub register: u16,
    pub tolerance: u32,
    pub silence_ms: u32,
    pub reset_stop: bool,
    pub reset_loop: bool,
    /// How the player's controller renders primes 3, 5 and 7, in microcents,
    /// from the lattice's own C offset. A key may only become a node this
    /// tuning would render at the pitch the key sent.
    pub keyboard: [i32; 3],
    /// Every source with Retune on reads its MIDI channels as comma banks
    /// over a chain of fifths: the lattice-board's encoding (yan-h/lattice-board).
    pub decode_fifths: bool,
    /// Under Adaptive with Decode fifths on, the strip slides by fifths to
    /// follow the spellings in use (`policy::strip::Follower`); off, it stays
    /// where the Position fifths put it.
    pub follow_spellings: bool,
    /// The fifth, in microcents, a decoded source sounds its chain at: what
    /// its keyboard declares rather than the lattice's own.
    pub decoded_fifth: i32,
}
impl Default for PolicyConfig {
    fn default() -> Self {
        crate::policy::CONFIG
    }
}
impl PolicyConfig {
    /// Whether the lattice is drawn in the lattice-board's tuning under
    /// `engine`: Pass through with Decode fifths on, where the board's chain
    /// sounds at the Decoded fifth and nothing retunes it.
    pub fn board_drawn(&self, engine: TuningEngine) -> bool {
        engine == TuningEngine::Off && self.decode_fifths
    }
    /// Whether Learn runs under `engine`: Pass through alone, where nothing
    /// retunes and the lattice draws what is played (#1938 C). Lattice Map
    /// reads its own map instead, Adaptive's retuning target is the lattice
    /// itself, and with Decode fifths on the lattice-board declares its
    /// tuning, which the lattice is drawn in under Pass through.
    pub fn learns(&self, engine: TuningEngine) -> bool {
        engine == TuningEngine::Off && !self.decode_fifths
    }
    pub fn sanitize(mut self) -> Self {
        self.radius = self.radius.clamp(1, 5);
        self.axes = self.axes.clamp(1, 3);
        self.pitch_flexibility = self.pitch_flexibility.clamp(1, 100);
        self.half_life_ms = self.half_life_ms.min(20_000);
        // Zero would leave a context note in another register no vote at all,
        // and a context entirely in other registers an empty average.
        self.register = self.register.clamp(10, 1000);
        self.tolerance = self.tolerance.min(20_000_000);
        self.silence_ms = self.silence_ms.min(120_000);
        self.keyboard = self.keyboard.map(|v| v.clamp(0, 1_200_000_000));
        self.decoded_fifth = TuningAxis::Three.clamp(self.decoded_fifth);
        self
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ResolvedConfig {
    pub revision: u64,
    pub tuning: Tuning,
    pub modes: TuningModes,
    pub policy: PolicyConfig,
}

impl ResolvedConfig {
    /// Whether Learn acts under `engine`: armed, and an engine it runs under
    /// ([`PolicyConfig::learns`]).
    pub fn learn_acts(&self, engine: TuningEngine) -> bool {
        self.modes.learning && self.policy.learns(engine)
    }
    /// This configuration as the lattice is drawn and a take records it under
    /// `engine`. Where [`PolicyConfig::board_drawn`], the fifth is the Decoded fifth
    /// with Meantone linked, since the board names a third as four fifths;
    /// the seventh keeps its value unless Marvel derives it. The saved axes
    /// are untouched, so Lattice Map still tunes from them.
    pub fn drawn(mut self, engine: TuningEngine) -> Self {
        if self.policy.board_drawn(engine) {
            self.tuning.three = self.policy.decoded_fifth;
            self.modes.tempered.syntonic = true;
            for comma in Comma::ALL {
                if self.modes.tempered.has(comma) {
                    self.tuning.temper(comma);
                }
            }
        }
        self
    }
}

/// How far each prime's interval may sit from just intonation, in cents.
pub const MAX_TUNING_OFFSET: f32 = 40.0;

/// One of the five tuning axes: the lattice origin, the three intervals that
/// span it, and the match tolerance. Each owns its place in [`Tuning::axes`]
/// and [`ConfigEdit::axes`], its default and the range the reducer holds it
/// in, which is every value an editor can show.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TuningAxis {
    /// Offset of the lattice origin (C) in cents.
    COffset,
    /// Perfect fifth (prime 3).
    Three,
    /// Major third (prime 5).
    Five,
    /// Harmonic seventh (prime 7).
    Seven,
    /// How far a played pitch may sit from a node and still light it.
    Tolerance,
}

impl TuningAxis {
    pub const ALL: [TuningAxis; 5] =
        [Self::COffset, Self::Three, Self::Five, Self::Seven, Self::Tolerance];

    pub fn index(self) -> usize {
        self as usize
    }

    /// 12-TET, so the lattice matches a plain MIDI keyboard until the tuning
    /// is dialled in.
    pub fn default_cents(self) -> f32 {
        Tuning::default().cents(self)
    }

    /// In cents.
    pub fn range(self) -> std::ops::RangeInclusive<f32> {
        use crate::tuning::{marvel_seventh, meantone_third, THREE_JUST};
        match self {
            Self::COffset => -600.0..=600.0,
            Self::Three => THREE_JUST - MAX_TUNING_OFFSET..=THREE_JUST + MAX_TUNING_OFFSET,
            // A released link keeps its value without a pitch jump, so each
            // derived interval's range holds every value its generators derive.
            Self::Five => {
                let fifth = Self::Three.range();
                meantone_third(*fifth.start()).floor()..=meantone_third(*fifth.end()).ceil()
            }
            Self::Seven => {
                let (fifth, third) = (Self::Three.range(), Self::Five.range());
                marvel_seventh(*fifth.start(), *third.start()).floor()
                    ..=marvel_seventh(*fifth.end(), *third.end()).ceil()
            }
            Self::Tolerance => 0.001..=49.999,
        }
    }

    /// `microcents`, held in [`range`](Self::range).
    pub fn clamp(self, microcents: i32) -> i32 {
        let range = self.range();
        microcents.clamp(
            crate::tuning::microcents(*range.start()),
            crate::tuning::microcents(*range.end()),
        )
    }
}

/// A semantic UI transaction. Every populated field changes together. The
/// axes are indexed by [`TuningAxis::index`].
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct ConfigEdit {
    pub axes: [Option<i32>; 5],
    pub tempered: [Option<bool>; Comma::COUNT],
    pub learning: Option<bool>,
    /// The whole adaptive policy, as the editor shows it.
    pub policy: Option<PolicyConfig>,
}

impl ConfigEdit {
    pub fn axis(axis: TuningAxis, microcents: i32) -> Self {
        let mut edit = Self::default();
        edit.axes[axis.index()] = Some(microcents);
        edit
    }

    /// Switch off at the interval currently heard and displayed, including a
    /// value derived by another link. Send the value and flag as one edit so
    /// project saves retain that value too.
    pub fn temper(comma: Comma, on: bool, current: Tuning) -> Self {
        let mut edit = Self::default();
        edit.tempered[comma.index()] = Some(on);
        if !on {
            let axis = comma.derived_axis();
            edit.axes[axis.index()] = Some(current.axes()[axis.index()]);
        }
        edit
    }
}

impl Comma {
    /// The interval this comma derives from its generators while tempered.
    pub fn derived_axis(self) -> TuningAxis {
        match self {
            Comma::Syntonic => TuningAxis::Five,
            Comma::SeptimalKleisma => TuningAxis::Seven,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum ConfigMutation {
    Edit(ConfigEdit),
    Restore {
        raw: Tuning,
        modes: TuningModes,
        policy: PolicyConfig,
    },
    /// Learn classifies its unconstrained evidence before imposing any links.
    /// Every learned value replaces its raw one, and the keyboard follows the
    /// learned fifth.
    LearnResolved {
        learned: LearnedTuning,
    },
}

/// One owner for edits and their effective tuning. Independent interval edits
/// recognize relationships; a linked interval follows edits of its generators.
/// Restore never rejudges an unchanged tuning. Every raw axis it takes in is
/// held in its [`TuningAxis::range`].
#[derive(Clone, Debug)]
pub struct ConfigReducer {
    raw: Tuning,
    modes: TuningModes,
    resolved: ResolvedConfig,
}

impl Default for ConfigReducer {
    fn default() -> Self {
        Self::new(Tuning::default(), TuningModes::default())
    }
}

/// `raw` with each axis held in its range.
fn clamped(raw: Tuning) -> Tuning {
    let axes = raw.axes();
    Tuning::from_axes(TuningAxis::ALL.map(|axis| axis.clamp(axes[axis.index()])))
}

impl ConfigReducer {
    pub fn new(raw: Tuning, modes: TuningModes) -> Self {
        let raw = clamped(raw);
        let mut reducer = Self {
            raw,
            modes,
            resolved: ResolvedConfig {
                revision: 0,
                tuning: raw,
                modes,
                policy: crate::policy::CONFIG,
            },
        };
        reducer.resolve();
        reducer
    }

    pub fn raw(&self) -> Tuning {
        self.raw
    }
    pub fn resolved(&self) -> ResolvedConfig {
        self.resolved
    }
    /// A new revision whenever the resolved configuration changes musically.
    /// The counter is 64 bits, a few million years of one edit per sample.
    pub fn apply(&mut self, mutation: ConfigMutation) {
        let previous = self.resolved;
        match mutation {
            ConfigMutation::Restore { raw, modes, policy } => {
                self.resolved.policy = policy.sanitize();
                self.raw = clamped(raw);
                self.modes = modes;
            }
            ConfigMutation::Edit(edit) => {
                if let Some(policy) = edit.policy {
                    self.resolved.policy = policy.sanitize();
                }
                let mut axes = self.raw.axes();
                for axis in TuningAxis::ALL {
                    if let Some(value) = edit.axes[axis.index()] {
                        axes[axis.index()] = axis.clamp(value);
                    }
                }
                self.raw = Tuning::from_axes(axes);
                // A switch-off carries a frozen interval, not a new independent
                // tuning entry. It must not recognize a different released link.
                let entered = [
                    edit.axes[1].is_some(),
                    edit.axes[2].is_some() && edit.tempered[0].is_none(),
                    edit.axes[3].is_some() && edit.tempered[1].is_none(),
                ];
                let mut tuning = self.raw;
                for comma in Comma::ALL {
                    let i = comma.index();
                    let on = if let Some(on) = edit.tempered[i] {
                        on
                    } else if entered[i + 1]
                        || (!self.modes.tempered.has(comma) && entered[..=i].iter().any(|v| *v))
                    {
                        comma.is_tempered(
                            tuning.three_cents(),
                            tuning.five_cents(),
                            tuning.seven_cents(),
                        )
                    } else {
                        self.modes.tempered.has(comma)
                    };
                    self.modes.tempered = self.modes.tempered.with(comma, on);
                    if on {
                        tuning.temper(comma);
                    }
                }
                if let Some(on) = edit.learning {
                    self.modes.learning = on;
                }
            }
            ConfigMutation::LearnResolved { learned } => {
                if let Some(three) = learned.three {
                    self.resolved.policy.keyboard =
                        crate::tuning::fifth_generated(crate::tuning::microcents(three));
                }
                self.modes = learned_modes(learned, self.modes);
                let mut axes = self.raw.axes();
                let found = [learned.c_offset, learned.three, learned.five, learned.seven];
                for (axis, cents) in TuningAxis::ALL.into_iter().zip(found) {
                    if let Some(cents) = cents {
                        axes[axis.index()] = axis.clamp(crate::tuning::microcents(cents));
                    }
                }
                self.raw = Tuning::from_axes(axes);
            }
        }
        self.resolve();
        // Display tolerance is not a musical change.
        let mut before = previous;
        before.tuning.tolerance = self.resolved.tuning.tolerance;
        if before != self.resolved {
            self.resolved.revision = previous.revision + 1;
        }
    }

    fn resolve(&mut self) {
        let mut tuning = self.raw;
        for comma in Comma::ALL {
            if self.modes.tempered.has(comma) {
                tuning.temper(comma);
            }
        }
        self.resolved.tuning = tuning;
        self.resolved.modes = self.modes;
    }
}

/// Only relationships fully evidenced by the learned chord can change. The
/// played intervals decide both flags, never values imposed by an existing link.
pub fn learned_modes(learned: LearnedTuning, mut modes: TuningModes) -> TuningModes {
    if let (Some(three), Some(five)) = (learned.three, learned.five) {
        modes.tempered.syntonic = Comma::Syntonic.is_tempered(three, five, 0.0);
        if let Some(seven) = learned.seven {
            // Use the newly recognized third, never an older imposed link.
            let five =
                if modes.tempered.syntonic { crate::tuning::meantone_third(three) } else { five };
            modes.tempered.septimal_kleisma =
                Comma::SeptimalKleisma.is_tempered(three, five, seven);
        }
    }
    modes
}

const _: () = assert!(std::mem::size_of::<ResolvedConfig>() <= 128);
const _: () = assert!(std::mem::align_of::<ResolvedConfig>() <= 8);

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tuning::{microcents, FIVE_JUST, THREE_JUST};

    fn enter(reducer: &mut ConfigReducer, index: usize, cents: f32) {
        let edit = ConfigEdit::axis(TuningAxis::ALL[index], microcents(cents));
        reducer.apply(ConfigMutation::Edit(edit));
    }

    fn switch(reducer: &mut ConfigReducer, comma: Comma, on: bool) {
        let edit = ConfigEdit::temper(comma, on, reducer.resolved().tuning);
        reducer.apply(ConfigMutation::Edit(edit));
    }

    /// The reducer is the one door the raw axes come through, so it holds each
    /// in the range an editor can show, whichever way it arrives.
    #[test]
    fn every_way_in_holds_the_raw_axes_in_their_ranges() {
        let wild = Tuning::from_axes([2_000_000_000, 1, 1_500_000_000, -5, 60_000_000]);
        let held = |raw: Tuning| {
            TuningAxis::ALL.into_iter().all(|axis| axis.range().contains(&raw.cents(axis)))
        };
        let mut reducer = ConfigReducer::new(wild, TuningModes::default());
        assert!(held(reducer.raw()), "new: {:?}", reducer.raw());
        let edit = ConfigEdit { axes: wild.axes().map(Some), ..Default::default() };
        reducer = ConfigReducer::default();
        reducer.apply(ConfigMutation::Edit(edit));
        assert!(held(reducer.raw()), "edit: {:?}", reducer.raw());
        reducer = ConfigReducer::default();
        let policy = PolicyConfig::default();
        reducer.apply(ConfigMutation::Restore { raw: wild, modes: TuningModes::default(), policy });
        assert!(held(reducer.raw()), "restore: {:?}", reducer.raw());
        reducer = ConfigReducer::default();
        let learned = LearnedTuning {
            c_offset: Some(5000.0),
            three: Some(500.0),
            five: Some(900.0),
            seven: Some(1.0),
        };
        reducer.apply(ConfigMutation::LearnResolved { learned });
        assert!(held(reducer.raw()), "learn: {:?}", reducer.raw());
    }

    #[test]
    fn manual_entries_recognize_and_release_links_with_matching_names_and_pitches() {
        let mut reducer = ConfigReducer::new(
            Tuning::just(),
            TuningModes { tempered: Tempered::default(), ..Default::default() },
        );
        enter(&mut reducer, 1, 696.58);
        assert!(reducer.resolved().modes.tempered.syntonic);
        enter(&mut reducer, 3, 965.80);
        let config = reducer.resolved();
        assert!(config.modes.tempered.septimal_kleisma);
        for (a, b) in [
            (crate::LatticePos::new(0, 1, 0), crate::LatticePos::new(4, 0, 0)),
            (crate::LatticePos::new(0, 0, 1), crate::LatticePos::new(2, 2, 0)),
        ] {
            assert_eq!(config.tuning.pitch_class(a), config.tuning.pitch_class(b));
            assert_eq!(
                a.respell(config.modes.tempered).note_name(),
                b.respell(config.modes.tempered).note_name()
            );
        }
        enter(&mut reducer, 2, 390.0);
        assert!(!reducer.resolved().modes.tempered.syntonic);
        assert_eq!(reducer.resolved().tuning.five, microcents(390.0));
        assert!(reducer.resolved().modes.tempered.septimal_kleisma);
        enter(&mut reducer, 3, 968.0);
        assert!(!reducer.resolved().modes.tempered.septimal_kleisma);
    }

    #[test]
    fn generator_edits_follow_links_and_switch_off_keeps_the_displayed_value() {
        let mut reducer = ConfigReducer::default();
        enter(&mut reducer, 1, 695.0);
        assert_eq!(reducer.resolved().tuning.five, microcents(380.0));
        assert_eq!(reducer.resolved().tuning.seven, microcents(950.0));
        switch(&mut reducer, Comma::SeptimalKleisma, false);
        switch(&mut reducer, Comma::Syntonic, false);
        assert_eq!(reducer.raw().five, microcents(380.0));
        assert_eq!(reducer.raw().seven, microcents(950.0));
        enter(&mut reducer, 0, 3.0);
        enter(&mut reducer, 4, 2.0);
        enter(&mut reducer, 3, 951.0);
        assert!(!reducer.resolved().modes.tempered.syntonic);
        // Entering even the same number is an explicit new tuning request.
        enter(&mut reducer, 2, 380.0);
        assert!(reducer.resolved().modes.tempered.syntonic);
    }

    #[test]
    fn presets_and_restore_apply_atomically_without_rejudging_saved_switches() {
        let mut reducer = ConfigReducer::default();
        reducer.apply(ConfigMutation::Edit(ConfigEdit {
            axes: [
                None,
                Some(microcents(THREE_JUST)),
                Some(microcents(FIVE_JUST)),
                Some(microcents(crate::tuning::SEVEN_JUST)),
                None,
            ],
            ..Default::default()
        }));
        assert_eq!(reducer.resolved().modes.tempered, Tempered::default());
        switch(&mut reducer, Comma::Syntonic, true);
        assert!(reducer.resolved().modes.tempered.syntonic);
        let modes = TuningModes { tempered: Tempered::default(), learning: false };
        reducer.apply(ConfigMutation::Restore {
            raw: Tuning::default(),
            modes,
            policy: PolicyConfig::default(),
        });
        assert_eq!(reducer.resolved().modes, modes);
        let revision = reducer.resolved().revision;
        enter(&mut reducer, 4, 2.0);
        assert_eq!(reducer.resolved().revision, revision, "display tolerance is not musical");
    }

    /// Pass through with Decode fifths on draws the lattice in the board's
    /// tuning, from a just lattice with both links released: third-comma, so
    /// its linked third (379.1¢) is no 5:4 left standing. Marvel, released,
    /// keeps the seventh; linked, it derives the seventh from the board's.
    #[test]
    fn pass_through_with_decoding_draws_the_lattice_in_the_board_s_tuning() {
        let mut reducer = ConfigReducer::default();
        for comma in Comma::ALL {
            switch(&mut reducer, comma, false);
        }
        for (index, cents) in [(1, THREE_JUST), (2, FIVE_JUST)] {
            enter(&mut reducer, index, cents);
        }
        let policy = PolicyConfig {
            decode_fifths: true,
            decoded_fifth: 694_786_840,
            ..PolicyConfig::default()
        };
        reducer
            .apply(ConfigMutation::Edit(ConfigEdit { policy: Some(policy), ..Default::default() }));
        let just = reducer.resolved();
        let board = just.drawn(TuningEngine::Off);
        assert_eq!(board.tuning.three, 694_786_840);
        assert!(board.modes.tempered.syntonic && !board.modes.tempered.septimal_kleisma);
        assert_eq!(i64::from(board.tuning.five), 4 * 694_786_840 - 2_400_000_000);
        assert_eq!(board.tuning.seven, just.tuning.seven);
        for engine in [TuningEngine::LatticeMap, TuningEngine::Adaptive] {
            assert_eq!(just.drawn(engine), just, "{engine:?} draws the saved axes");
        }
        let off = ResolvedConfig { policy: PolicyConfig::default(), ..just };
        assert_eq!(off.drawn(TuningEngine::Off), off, "decoding off draws the saved axes");
        switch(&mut reducer, Comma::SeptimalKleisma, true);
        let marvel = reducer.resolved().drawn(TuningEngine::Off);
        assert_eq!(
            i64::from(marvel.tuning.seven),
            2 * 694_786_840 + 2 * i64::from(board.tuning.five) - 1_200_000_000
        );
    }

    #[test]
    fn learn_uses_unconstrained_evidence_and_leaves_unevidenced_links_alone() {
        let mut reducer = ConfigReducer::default();
        for comma in Comma::ALL {
            switch(&mut reducer, comma, false);
        }
        reducer.apply(ConfigMutation::LearnResolved {
            learned: LearnedTuning { three: Some(696.58), ..Default::default() },
        });
        assert_eq!(reducer.resolved().modes.tempered, Tempered::default());
        reducer.apply(ConfigMutation::LearnResolved {
            learned: LearnedTuning {
                three: Some(696.58),
                five: Some(386.32),
                seven: Some(965.8),
                ..Default::default()
            },
        });
        assert_eq!(reducer.resolved().modes.tempered, TuningModes::default().tempered);
        let learned = LearnedTuning {
            three: Some(700.0),
            five: Some(386.0),
            seven: Some(972.0),
            ..Default::default()
        };
        let modes = learned_modes(learned, reducer.resolved().modes);
        assert!(!modes.tempered.syntonic);
        assert!(
            modes.tempered.septimal_kleisma,
            "Marvel uses played 386, not the old linked third"
        );
    }
}
