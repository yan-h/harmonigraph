//! Effective musical configuration. One serialized owner applies semantic edits;
//! display and policy consumers copy the same resolved value. No serde or I/O.

use crate::{Comma, LearnedTuning, Tempered, Tuning};

/// Runtime mode state. Judgements and command acknowledgements are deliberately
/// separate: neither is part of a saved musical setting.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct TuningModes {
    pub tempered: Tempered,
    pub auto: [bool; Comma::COUNT],
    pub learning: bool,
}

impl Default for TuningModes {
    fn default() -> Self {
        Self { tempered: Tempered::default(), auto: [true; Comma::COUNT], learning: false }
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
    pub auto: [Option<bool>; Comma::COUNT],
    pub learning: Option<bool>,
    pub policy: Option<PolicyConfig>,
}

impl ConfigEdit {
    pub fn axis(index: usize, microcents: i32) -> Self {
        let mut edit = Self::default();
        edit.axes[index] = Some(microcents);
        edit
    }

    pub fn unlock(comma: Comma, microcents: i32) -> Self {
        let mut edit = Self::axis(comma.index() + 2, microcents);
        edit.tempered[comma.index()] = Some(false);
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
    /// The keyboard tuning follows the learned fifth either way. The comma
    /// judgement follows the learned chord only while no source is
    /// `retuning`: with retuning off the lattice is a picture of the input,
    /// with it on it is the target. `raw` is what the learned edit committed,
    /// which is where the axes it moved arrive, and host modulation may have
    /// moved them since. A comma the chord engages stays engaged; one it
    /// releases `resolve` may engage again from that `raw`.
    LearnResolved {
        learned: LearnedTuning,
        retuning: bool,
        raw: Tuning,
    },
}

/// Pure comma resolver, used by the CLAP audio owner and synchronously by the
/// standalone/legacy display adapter. Each comma judges the axes after earlier
/// commas derive them, before its own derivation. Only those inputs key its verdict.
/// An explicit release instead holds until the comma's raw axes change, so an
/// earlier comma's switch moving the derived third does not undo it.
#[derive(Clone, Debug)]
pub struct ConfigReducer {
    raw: Tuning,
    modes: TuningModes,
    judged: [Option<(i32, i32, i32)>; Comma::COUNT],
    released: [Option<(i32, i32, i32)>; Comma::COUNT],
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
            judged: [None; Comma::COUNT],
            released: [None; Comma::COUNT],
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
    pub fn judged(&self) -> [Option<(i32, i32, i32)>; Comma::COUNT] {
        self.judged
    }

    /// Ask the next display observation to judge this comma again.
    pub fn recheck(&mut self, comma: Comma) {
        self.judged[comma.index()] = None;
        self.released[comma.index()] = None;
    }

    /// A new display or restored appearance has no verdict about its tuning yet.
    pub fn recheck_all(&mut self) {
        self.judged = [None; Comma::COUNT];
        self.released = [None; Comma::COUNT];
    }

    /// Synchronous display adapter only. CLAP must submit explicit commands;
    /// polling these raw values there would lose automation and commit identity.
    pub fn sync_display(&mut self, raw: Tuning, modes: TuningModes, policy: PolicyConfig) -> bool {
        // A changed switch after observation is an explicit display choice.
        // An unjudged comma instead belongs to a fresh view or an Auto recheck.
        let tempered = std::array::from_fn(|i| {
            let comma = Comma::ALL[i];
            (self.judged[i].is_some()
                && self.modes.tempered.has(comma) != modes.tempered.has(comma))
            .then_some(modes.tempered.has(comma))
        });
        self.raw = raw;
        self.modes = modes;
        // Policy and modes arrive together: resolving a policy edit sooner
        // would consume a pending Auto recheck against the old display modes.
        self.apply(ConfigMutation::Edit(ConfigEdit {
            tempered,
            policy: Some(policy),
            ..Default::default()
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
                self.judged = [None; Comma::COUNT];
                self.released = [None; Comma::COUNT];
            }
            ConfigMutation::Edit(edit) => {
                if let Some(policy) = edit.policy {
                    self.resolved.policy = policy.sanitize();
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
                for comma in Comma::ALL {
                    let i = comma.index();
                    if let Some(on) = edit.tempered[i] {
                        self.modes.tempered = self.modes.tempered.with(comma, on);
                        self.released[i] = (!on).then(|| judged_axes(comma, self.raw));
                    }
                    if let Some(on) = edit.auto[i] {
                        self.modes.auto[i] = on;
                        // Explicitly enabling Auto in the same command still
                        // asks to recheck.
                        if on {
                            self.judged[i] = None;
                            self.released[i] = None;
                        }
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
            let i = comma.index();
            let axes = judged_axes(comma, tuning);
            if self.modes.tempered.has(comma)
                || self.released[i] != Some(judged_axes(comma, self.raw))
            {
                self.released[i] = None;
            }
            if self.released[i].is_none()
                && self.modes.auto[i]
                && !self.modes.tempered.has(comma)
                && self.judged[i] != Some(axes)
            {
                self.modes.tempered = self.modes.tempered.with(
                    comma,
                    comma.is_tempered(
                        tuning.three_cents(),
                        tuning.five_cents(),
                        tuning.seven_cents(),
                    ),
                );
            }
            self.judged[i] = Some(axes);
            if self.modes.tempered.has(comma) {
                tuning.temper(comma);
            }
        }
        self.resolved.tuning = tuning;
        self.resolved.modes = self.modes;
    }
}

pub fn judged_axes(comma: Comma, tuning: Tuning) -> (i32, i32, i32) {
    match comma {
        Comma::Syntonic => (tuning.three, tuning.five, 0),
        Comma::SeptimalKleisma => (tuning.three, tuning.five, tuning.seven),
    }
}

fn learned_axes(
    comma: Comma,
    learned: LearnedTuning,
    modes: TuningModes,
) -> Option<(f32, f32, f32)> {
    let (three, five) = (learned.three?, learned.five?);
    match comma {
        Comma::Syntonic => Some((three, five, 0.0)),
        Comma::SeptimalKleisma => {
            let five = if modes.tempered.has(Comma::Syntonic) {
                crate::tuning::meantone_third(three)
            } else {
                five
            };
            Some((three, five, learned.seven?))
        }
    }
}

/// Learning alone may release an Auto comma when all of its required axes are
/// evidenced. Syntonic derivation precedes the septimal verdict.
pub fn learned_modes(learned: LearnedTuning, mut modes: TuningModes) -> TuningModes {
    for comma in Comma::ALL {
        if modes.auto[comma.index()] {
            if let Some((three, five, seven)) = learned_axes(comma, learned, modes) {
                modes.tempered = modes.tempered.with(comma, comma.is_tempered(three, five, seven));
            }
        }
    }
    modes
}

const _: () = assert!(std::mem::size_of::<ResolvedConfig>() <= 128);
const _: () = assert!(std::mem::align_of::<ResolvedConfig>() <= 8);

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tuning::microcents;

    #[test]
    fn marvel_judges_the_effective_third_and_preserves_an_explicit_release() {
        let mut reducer = ConfigReducer::new(
            Tuning::from_cents(0.0, 700.0, 386.0, 1000.0, 0.5),
            TuningModes::default(),
        );
        assert!(!reducer.resolved().modes.tempered.has(Comma::SeptimalKleisma));
        reducer.apply(ConfigMutation::Edit(ConfigEdit {
            tempered: [Some(true), None],
            ..Default::default()
        }));
        assert!(reducer.resolved().modes.tempered.has(Comma::SeptimalKleisma));
        reducer.apply(ConfigMutation::Edit(ConfigEdit {
            tempered: [None, Some(false)],
            ..Default::default()
        }));
        let judged = reducer.judged();
        reducer.apply(ConfigMutation::Edit(ConfigEdit::axis(2, microcents(387.0))));
        assert_eq!(reducer.judged()[1], judged[1], "Meantone still derives the same third");
        assert!(!reducer.resolved().modes.tempered.has(Comma::SeptimalKleisma));
        reducer.apply(ConfigMutation::Edit(ConfigEdit {
            auto: [None, Some(true)],
            ..Default::default()
        }));
        assert!(reducer.resolved().modes.tempered.has(Comma::SeptimalKleisma));
    }

    #[test]
    fn releasing_meantone_after_marvel_does_not_re_engage_marvel() {
        // Not exactly four fifths, so releasing Meantone moves Marvel's third.
        let mut reducer = ConfigReducer::new(
            Tuning::from_cents(0.0, 700.0, 400.1, 1000.0, 0.5),
            TuningModes::default(),
        );
        assert!(reducer.resolved().modes.tempered.has(Comma::SeptimalKleisma));
        reducer.apply(ConfigMutation::Edit(ConfigEdit {
            tempered: [None, Some(false)],
            ..Default::default()
        }));
        reducer.apply(ConfigMutation::Edit(ConfigEdit {
            tempered: [Some(false), None],
            ..Default::default()
        }));
        let modes = reducer.resolved().modes;
        assert!(!modes.tempered.has(Comma::Syntonic));
        assert!(!modes.tempered.has(Comma::SeptimalKleisma), "the user's release holds");
    }

    #[test]
    fn explicit_release_dependency_keys_and_auto_recheck_are_distinct() {
        let mut reducer = ConfigReducer::default();
        assert!(reducer.resolved().modes.tempered.has(Comma::Syntonic));
        let release = ConfigEdit { tempered: [Some(false); 2], ..Default::default() };
        reducer.apply(ConfigMutation::Edit(release));
        let revision = reducer.resolved().revision;
        reducer.apply(ConfigMutation::Edit(ConfigEdit::axis(4, microcents(2.0))));
        assert_eq!(reducer.resolved().revision, revision, "tolerance is display only");
        reducer.apply(ConfigMutation::Edit(ConfigEdit::axis(3, microcents(999.0))));
        assert!(!reducer.resolved().modes.tempered.has(Comma::Syntonic));
        reducer.apply(ConfigMutation::Edit(ConfigEdit::axis(1, microcents(700.0))));
        assert!(
            !reducer.resolved().modes.tempered.has(Comma::Syntonic),
            "same-value external automation is a command but no new judgement"
        );
        let mut recheck = ConfigEdit::default();
        recheck.auto[0] = Some(true);
        reducer.apply(ConfigMutation::Edit(recheck));
        assert!(reducer.resolved().modes.tempered.has(Comma::Syntonic));
        reducer.apply(ConfigMutation::Edit(ConfigEdit::unlock(Comma::Syntonic, microcents(390.0))));
        assert!(!reducer.resolved().modes.tempered.has(Comma::Syntonic));
        assert_eq!(reducer.raw().five, microcents(390.0));
    }

    #[test]
    fn learning_can_release_with_complete_evidence_but_not_a_bare_fifth() {
        let mut reducer = ConfigReducer::default();
        // `raw` carries what the owner's learned edit committed.
        let mut raw = reducer.raw();
        raw.three = microcents(700.0);
        reducer.apply(ConfigMutation::LearnResolved {
            learned: LearnedTuning { three: Some(700.0), ..Default::default() },
            retuning: false,
            raw,
        });
        assert!(reducer.resolved().modes.tempered.has(Comma::Syntonic));
        raw.five = microcents(crate::tuning::FIVE_JUST);
        reducer.apply(ConfigMutation::LearnResolved {
            learned: LearnedTuning {
                three: Some(700.0),
                five: Some(crate::tuning::FIVE_JUST),
                ..Default::default()
            },
            retuning: false,
            raw,
        });
        assert!(!reducer.resolved().modes.tempered.has(Comma::Syntonic));
        let mut modes = TuningModes {
            tempered: Tempered { syntonic: true, septimal_kleisma: false },
            ..Default::default()
        };
        let learned = LearnedTuning {
            three: Some(700.0),
            five: Some(386.0),
            seven: Some(972.0),
            ..Default::default()
        };
        modes.auto[0] = false;
        assert!(
            !learned_modes(learned, modes).tempered.has(Comma::SeptimalKleisma),
            "septimal sees derived 400, not played 386"
        );
    }
}
