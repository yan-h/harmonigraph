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
    pub version: u32,
    pub radius: u8,
    pub axes: u8,
    pub harmonic: u16,
    pub pitch_scale: u16,
    /// A released note weighs this much against a held note struck at the
    /// same moment.
    pub released: u16,
    /// Held and released contributions alike halve in weight once per this
    /// long between their attack and the newest attack in context. A release
    /// does not restart it.
    pub half_life_ms: u16,
    /// A released note keeps this much of its weight each time a note lands
    /// on a lattice node no held or remembered note occupies. Repeating a
    /// node, in any octave, costs nothing.
    pub new_note: u16,
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
        self.version = 2;
        self.radius = self.radius.clamp(1, 5);
        self.axes = self.axes.clamp(1, 3);
        self.harmonic = self.harmonic.min(20_000);
        self.pitch_scale = self.pitch_scale.clamp(1, 100);
        self.released = self.released.min(1000);
        self.half_life_ms = self.half_life_ms.min(20_000);
        self.new_note = self.new_note.min(1000);
        // Zero would leave a context note in another register no vote at all,
        // and a context entirely in other registers an empty average.
        self.register = self.register.clamp(10, 1000);
        self.tolerance = self.tolerance.min(20_000_000);
        self.silence_ms = self.silence_ms.min(120_000);
        self.keyboard = self.keyboard.map(|v| v.clamp(0, 1_200_000_000));
        self
    }
    /// Fixed configuration mailbox representation, shared by edits and snapshots.
    /// The new-note factor takes bits 16–25 of the second word, so the reset
    /// flags sit above it.
    pub fn words(self) -> [i32; 10] {
        [
            2,
            i32::from(self.radius)
                | i32::from(self.axes) << 8
                | i32::from(self.new_note) << 16
                | i32::from(self.reset_stop) << 26
                | i32::from(self.reset_loop) << 27,
            i32::from(self.harmonic) | i32::from(self.pitch_scale) << 16,
            i32::from(self.released) | i32::from(self.half_life_ms) << 16,
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
            version: 2,
            radius: w[1] as u8,
            axes: (w[1] >> 8) as u8,
            new_note: ((w[1] >> 16) & 0x3ff) as u16,
            reset_stop: w[1] & (1 << 26) != 0,
            reset_loop: w[1] & (1 << 27) != 0,
            harmonic: w[2] as u16,
            pitch_scale: (w[2] >> 16) as u16,
            released: w[3] as u16,
            half_life_ms: (w[3] >> 16) as u16,
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
    /// The keyboard tuning follows the learned fifth and the shared C offset
    /// the learned C either way. The lattice axes and their comma judgement
    /// follow only while no source is `retuning`: with retuning off the lattice
    /// is a picture of the input, with it on it is the target.
    Learn {
        learned: LearnedTuning,
        retuning: bool,
    },
    /// Host modulation can change the committed raw axes while learning's
    /// complete-evidence judgement still describes the chord that was heard.
    LearnResolved {
        learned: LearnedTuning,
        retuning: bool,
        raw: Tuning,
    },
}

/// Pure comma resolver, used by the CLAP audio owner and synchronously by the
/// standalone/legacy display adapter. The judged key is raw axes, per comma;
/// derived values, unrelated axes, tolerance and UI state decide no verdict.
#[derive(Clone, Debug)]
pub struct ConfigReducer {
    raw: Tuning,
    modes: TuningModes,
    judged: [Option<(i32, i32, i32)>; Comma::COUNT],
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

    /// Restore a legacy adapter's judgement cache without opening any verdict.
    /// Audio callers never infer commands by comparing raw parameter snapshots.
    pub fn set_judged(&mut self, judged: [Option<(i32, i32, i32)>; Comma::COUNT]) {
        self.judged = judged;
    }

    /// Synchronous display adapter only. CLAP must submit explicit commands;
    /// polling these raw values there would lose automation and commit identity.
    pub fn sync_display(
        &mut self,
        raw: Tuning,
        modes: TuningModes,
        judged: [Option<(i32, i32, i32)>; Comma::COUNT],
    ) -> bool {
        self.raw = raw;
        self.modes = modes;
        self.judged = judged;
        self.apply(ConfigMutation::Edit(ConfigEdit::default()))
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
                        // An explicit release judges the newly committed raw axes
                        // too. A stale host value is never a new command here.
                        self.judged[i] = Some(judged_axes(comma, self.raw));
                    }
                    if let Some(on) = edit.auto[i] {
                        self.modes.auto[i] = on;
                        if on {
                            self.judged[i] = None;
                        }
                    }
                }
                if let Some(on) = edit.learning {
                    self.modes.learning = on;
                }
            }
            ConfigMutation::Learn { learned, retuning }
            | ConfigMutation::LearnResolved { learned, retuning, .. } => {
                if let Some(three) = learned.three {
                    self.resolved.policy.keyboard =
                        crate::tuning::fifth_generated(crate::tuning::microcents(three));
                }
                let written = if retuning { 1 } else { 4 };
                for (axis, value) in [
                    &mut self.raw.c_offset,
                    &mut self.raw.three,
                    &mut self.raw.five,
                    &mut self.raw.seven,
                ]
                .into_iter()
                .zip([learned.c_offset, learned.three, learned.five, learned.seven])
                .take(written)
                {
                    if let Some(value) = value {
                        *axis = crate::tuning::microcents(value);
                    }
                }
                if !retuning {
                    self.modes = learned_modes(learned, self.modes);
                }
                if let ConfigMutation::LearnResolved { raw, .. } = mutation {
                    self.raw = raw;
                }
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
            let axes = judged_axes(comma, self.raw);
            if self.modes.auto[i] && !self.modes.tempered.has(comma) && self.judged[i] != Some(axes)
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
        reducer.apply(ConfigMutation::Learn {
            learned: LearnedTuning { three: Some(700.0), ..Default::default() },
            retuning: false,
        });
        assert!(reducer.resolved().modes.tempered.has(Comma::Syntonic));
        reducer.apply(ConfigMutation::Learn {
            learned: LearnedTuning {
                three: Some(700.0),
                five: Some(crate::tuning::FIVE_JUST),
                ..Default::default()
            },
            retuning: false,
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

    #[test]
    fn learning_moves_the_lattice_only_while_no_source_retunes() {
        let fifth = crate::tuning::THREE_JUST - crate::tuning::SYNTONIC_COMMA / 4.0;
        let learned = LearnedTuning {
            c_offset: Some(10.0),
            three: Some(fifth),
            five: Some(crate::tuning::FIVE_JUST),
            ..Default::default()
        };
        let before = ConfigReducer::default().resolved().tuning;
        for retuning in [false, true] {
            let mut reducer = ConfigReducer::default();
            reducer.apply(ConfigMutation::Learn { learned, retuning });
            let resolved = reducer.resolved();
            assert_eq!(resolved.policy.keyboard, crate::tuning::fifth_generated(microcents(fifth)));
            assert_eq!(resolved.tuning.c_offset, microcents(10.0));
            let axes = |t: Tuning| (t.three, t.five, t.seven);
            if retuning {
                assert_eq!(axes(resolved.tuning), axes(before));
            } else {
                assert_eq!(axes(reducer.raw()).0, microcents(fifth));
                assert_eq!(axes(reducer.raw()).1, microcents(crate::tuning::FIVE_JUST));
            }
        }
    }
}
