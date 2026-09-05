//! Effective musical configuration. One serialized owner applies semantic edits;
//! display and policy consumers copy the same resolved value. No serde or I/O.

use crate::{Comma, LearnedTuning, Tempered, Tuning};

pub mod timeline;

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

/// The assignment policy has not shipped yet. Version zero explicitly carries
/// no assignment domain or weights. #621 selects those constants; camera reach
/// and display tolerance must never fill these fields.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct PolicyConfig {
    pub version: u32,
    pub domain: [u16; 3],
    pub candidate_radius: u32,
    pub context_radius: u32,
    pub context_weight: u16,
    pub history_weight: u16,
    pub origin_weight: u16,
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
    },
    Learn(LearnedTuning),
    /// Host modulation can change the committed raw axes while learning's
    /// complete-evidence judgement still describes the chord that was heard.
    LearnResolved {
        learned: LearnedTuning,
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
                policy: PolicyConfig::default(),
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
            ConfigMutation::Restore { raw, modes } => {
                self.raw = raw;
                self.modes = modes;
                self.judged = [None; Comma::COUNT];
            }
            ConfigMutation::Edit(edit) => {
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
            ConfigMutation::Learn(learned) | ConfigMutation::LearnResolved { learned, .. } => {
                for (axis, value) in [
                    &mut self.raw.c_offset,
                    &mut self.raw.three,
                    &mut self.raw.five,
                    &mut self.raw.seven,
                ]
                .into_iter()
                .zip([
                    learned.c_offset,
                    learned.three,
                    learned.five,
                    learned.seven,
                ]) {
                    if let Some(value) = value {
                        *axis = crate::tuning::microcents(value);
                    }
                }
                self.modes = learned_modes(learned, self.modes);
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
        reducer.apply(ConfigMutation::Learn(LearnedTuning {
            three: Some(700.0),
            ..Default::default()
        }));
        assert!(reducer.resolved().modes.tempered.has(Comma::Syntonic));
        reducer.apply(ConfigMutation::Learn(LearnedTuning {
            three: Some(700.0),
            five: Some(crate::tuning::FIVE_JUST),
            ..Default::default()
        }));
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
