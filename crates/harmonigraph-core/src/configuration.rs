//! Effective musical configuration. One serialized owner applies semantic edits;
//! display and policy consumers copy the same resolved value. No serde or I/O.

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
}
impl Default for PolicyConfig {
    fn default() -> Self {
        crate::policy::CONFIG
    }
}
impl PolicyConfig {
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
        self
    }
    /// Fixed configuration mailbox representation, shared by edits and snapshots.
    /// The reset flags take bits 26 and 27 of the second word.
    pub fn words(self) -> [i32; 10] {
        [
            3,
            i32::from(self.radius)
                | i32::from(self.axes) << 8
                | i32::from(self.reset_stop) << 26
                | i32::from(self.reset_loop) << 27,
            i32::from(self.pitch_flexibility),
            i32::from(self.half_life_ms),
            i32::from(self.register),
            self.tolerance as i32,
            self.silence_ms as i32,
            self.keyboard[0],
            self.keyboard[1],
            self.keyboard[2],
        ]
    }
    pub fn from_words(w: [i32; 10]) -> Self {
        Self {
            radius: w[1] as u8,
            axes: (w[1] >> 8) as u8,
            reset_stop: w[1] & (1 << 26) != 0,
            reset_loop: w[1] & (1 << 27) != 0,
            pitch_flexibility: w[2].clamp(1, 100) as u16,
            half_life_ms: w[3] as u16,
            register: w[4] as u16,
            tolerance: w[5].max(0) as u32,
            silence_ms: w[6].max(0) as u32,
            keyboard: [w[7], w[8], w[9]],
        }
        .sanitize()
    }
}

/// Intent from a possibly stale display. Only selected fields replace the
/// owner's values; snapshots and restores still carry a complete policy.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct PolicyEdit {
    values: PolicyConfig,
    mask: u16,
}

impl PolicyEdit {
    const ALL: u16 = (1 << 12) - 1;
    const DERIVE_KEYBOARD: u16 = 1 << 12;

    pub fn changed(before: PolicyConfig, after: PolicyConfig) -> Self {
        let values = after.sanitize();
        let changed = [
            before.radius != values.radius,
            before.axes != values.axes,
            before.pitch_flexibility != values.pitch_flexibility,
            before.half_life_ms != values.half_life_ms,
            before.register != values.register,
            before.tolerance != values.tolerance,
            before.silence_ms != values.silence_ms,
            before.reset_stop != values.reset_stop,
            before.reset_loop != values.reset_loop,
            before.keyboard[0] != values.keyboard[0],
            before.keyboard[1] != values.keyboard[1],
            before.keyboard[2] != values.keyboard[2],
        ];
        let mask = changed
            .into_iter()
            .enumerate()
            .fold(0, |mask, (i, changed)| mask | (u16::from(changed) << i));
        Self { values, mask }
    }

    /// Complete synchronous adoption, not an edit from an observed UI value.
    pub fn all(values: PolicyConfig) -> Self {
        Self { values, mask: Self::ALL }
    }

    /// Derive from the owner's fifth after applying this transaction's fields.
    /// An explicit click remains an action even if the display looks derived.
    pub fn derive_keyboard(mut self) -> Self {
        self.mask |= Self::DERIVE_KEYBOARD;
        self
    }

    pub fn is_empty(self) -> bool {
        self.mask == 0
    }

    /// The mask occupies an otherwise unused word in the existing mailbox.
    pub fn words(self) -> (i32, [i32; 10]) {
        (i32::from(self.mask), self.values.sanitize().words())
    }

    pub fn from_words(mask: i32, values: [i32; 10]) -> Self {
        Self {
            values: PolicyConfig::from_words(values),
            mask: mask as u16 & (Self::ALL | Self::DERIVE_KEYBOARD),
        }
    }

    pub fn apply_to(self, current: &mut PolicyConfig) {
        if self.is_empty() {
            return;
        }
        if self.mask & 1 != 0 {
            current.radius = self.values.radius;
        }
        if self.mask & (1 << 1) != 0 {
            current.axes = self.values.axes;
        }
        if self.mask & (1 << 2) != 0 {
            current.pitch_flexibility = self.values.pitch_flexibility;
        }
        if self.mask & (1 << 3) != 0 {
            current.half_life_ms = self.values.half_life_ms;
        }
        if self.mask & (1 << 4) != 0 {
            current.register = self.values.register;
        }
        if self.mask & (1 << 5) != 0 {
            current.tolerance = self.values.tolerance;
        }
        if self.mask & (1 << 6) != 0 {
            current.silence_ms = self.values.silence_ms;
        }
        if self.mask & (1 << 7) != 0 {
            current.reset_stop = self.values.reset_stop;
        }
        if self.mask & (1 << 8) != 0 {
            current.reset_loop = self.values.reset_loop;
        }
        for i in 0..3 {
            if self.mask & (1 << (9 + i)) != 0 {
                current.keyboard[i] = self.values.keyboard[i];
            }
        }
        *current = current.sanitize();
        if self.mask & Self::DERIVE_KEYBOARD != 0 {
            current.keyboard = crate::tuning::fifth_generated(current.keyboard[0]);
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ResolvedConfig {
    pub revision: u64,
    pub tuning: Tuning,
    pub modes: TuningModes,
    pub policy: PolicyConfig,
}

/// A semantic UI transaction. Every populated field changes together. The five
/// axis indices are origin, three, five, seven and display tolerance.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct ConfigEdit {
    pub axes: [Option<i32>; 5],
    pub tempered: [Option<bool>; Comma::COUNT],
    pub learning: Option<bool>,
    pub policy: Option<PolicyEdit>,
}

impl ConfigEdit {
    pub fn axis(index: usize, microcents: i32) -> Self {
        let mut edit = Self::default();
        edit.axes[index] = Some(microcents);
        edit
    }

    /// Switch off at the interval currently heard and displayed, including a
    /// value derived by another link. Send the value and flag as one edit so
    /// host parameters and project saves retain that value too.
    pub fn temper(comma: Comma, on: bool, current: Tuning) -> Self {
        let mut edit = Self::default();
        edit.tempered[comma.index()] = Some(on);
        if !on {
            edit.axes[comma.index() + 2] = Some(match comma {
                Comma::Syntonic => crate::tuning::microcents(current.five_cents()),
                Comma::SeptimalKleisma => crate::tuning::microcents(current.seven_cents()),
            });
        }
        edit
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
    /// With retuning active, only the origin and keyboard are learned: the
    /// lattice intervals remain the target. `raw` is the committed host input.
    LearnResolved {
        learned: LearnedTuning,
        retuning: bool,
        raw: Tuning,
    },
}

/// One owner for edits and their effective tuning. Independent interval edits
/// recognize relationships; a linked interval follows edits of its generators.
/// Restore and display observation never rejudge an unchanged tuning.
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

impl ConfigReducer {
    pub fn new(raw: Tuning, modes: TuningModes) -> Self {
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
    /// Synchronous shells adapt actual parameter changes into edits. CLAP
    /// supplies the edited axes directly, including same-value typed entries.
    pub fn sync_display(&mut self, raw: Tuning, modes: TuningModes, policy: PolicyConfig) -> bool {
        let before =
            [self.raw.c_offset, self.raw.three, self.raw.five, self.raw.seven, self.raw.tolerance];
        let after = [raw.c_offset, raw.three, raw.five, raw.seven, raw.tolerance];
        self.apply(ConfigMutation::Edit(ConfigEdit {
            axes: std::array::from_fn(|i| (before[i] != after[i]).then_some(after[i])),
            tempered: std::array::from_fn(|i| {
                let comma = Comma::ALL[i];
                (self.modes.tempered.has(comma) != modes.tempered.has(comma))
                    .then_some(modes.tempered.has(comma))
            }),
            learning: Some(modes.learning),
            policy: Some(PolicyEdit::all(policy)),
        }))
    }

    /// Returns false only on revision exhaustion. The caller retains its command
    /// and enters explicit recovery; no counter wraps into an old identity.
    pub fn apply(&mut self, mutation: ConfigMutation) -> bool {
        let previous = self.clone();
        match mutation {
            ConfigMutation::Restore { raw, modes, policy } => {
                self.resolved.policy = policy.sanitize();
                self.raw = raw;
                self.modes = modes;
            }
            ConfigMutation::Edit(edit) => {
                if let Some(policy) = edit.policy {
                    policy.apply_to(&mut self.resolved.policy);
                }
                let axes = [
                    &mut self.raw.c_offset,
                    &mut self.raw.three,
                    &mut self.raw.five,
                    &mut self.raw.seven,
                    &mut self.raw.tolerance,
                ];
                for (axis, value) in axes.into_iter().zip(edit.axes) {
                    if let Some(value) = value {
                        *axis = value;
                    }
                }
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
            ConfigMutation::LearnResolved { learned, retuning, raw } => {
                if let Some(three) = learned.three {
                    self.resolved.policy.keyboard =
                        crate::tuning::fifth_generated(crate::tuning::microcents(three));
                }
                if !retuning {
                    self.modes = learned_modes(learned, self.modes);
                }
                self.raw = raw;
            }
        }
        self.resolve();
        // Display tolerance and acknowledgements are not musical changes.
        let mut before = previous.resolved;
        before.tuning.tolerance = self.resolved.tuning.tolerance;
        if before != self.resolved {
            let Some(revision) = previous.resolved.revision.checked_add(1) else {
                *self = previous;
                return false;
            };
            self.resolved.revision = revision;
        }
        true
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

    #[test]
    fn a_stale_policy_edit_preserves_the_keyboard_learned_since_observation() {
        let mut reducer = ConfigReducer::default();
        let observed = reducer.resolved().policy;
        let edit =
            PolicyEdit::changed(observed, PolicyConfig { pitch_flexibility: 37, ..observed });
        reducer.apply(ConfigMutation::LearnResolved {
            learned: LearnedTuning { three: Some(696.578), ..Default::default() },
            retuning: true,
            raw: reducer.raw(),
        });
        let learned = reducer.resolved();
        assert_ne!(learned.policy.keyboard, observed.keyboard);
        reducer
            .apply(ConfigMutation::Edit(ConfigEdit { policy: Some(edit), ..Default::default() }));
        assert_eq!(reducer.resolved().policy.keyboard, learned.policy.keyboard);
        assert_eq!(reducer.resolved().policy.pitch_flexibility, 37);
        assert_eq!(reducer.resolved().revision, learned.revision + 1);
    }

    fn enter(reducer: &mut ConfigReducer, index: usize, cents: f32) {
        assert!(reducer.apply(ConfigMutation::Edit(ConfigEdit::axis(index, microcents(cents)))));
    }

    fn switch(reducer: &mut ConfigReducer, comma: Comma, on: bool) {
        let edit = ConfigEdit::temper(comma, on, reducer.resolved().tuning);
        assert!(reducer.apply(ConfigMutation::Edit(edit)));
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
        let released = reducer.resolved();
        for _ in 0..3 {
            reducer.sync_display(reducer.raw(), released.modes, released.policy);
        }
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

    #[test]
    fn learn_uses_unconstrained_evidence_and_leaves_unevidenced_links_alone() {
        let mut reducer = ConfigReducer::default();
        for comma in Comma::ALL {
            switch(&mut reducer, comma, false);
        }
        let mut raw = reducer.raw();
        raw.three = microcents(696.58);
        reducer.apply(ConfigMutation::LearnResolved {
            learned: LearnedTuning { three: Some(696.58), ..Default::default() },
            retuning: false,
            raw,
        });
        assert_eq!(reducer.resolved().modes.tempered, Tempered::default());
        raw.five = microcents(386.32);
        raw.seven = microcents(965.8);
        reducer.apply(ConfigMutation::LearnResolved {
            learned: LearnedTuning {
                three: Some(696.58),
                five: Some(386.32),
                seven: Some(965.8),
                ..Default::default()
            },
            retuning: false,
            raw,
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
        let before = reducer.resolved();
        reducer.apply(ConfigMutation::LearnResolved { learned, retuning: true, raw });
        assert_eq!(reducer.resolved().tuning, before.tuning);
        assert_eq!(reducer.resolved().modes, before.modes);
    }
}
