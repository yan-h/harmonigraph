//! The bridge between UI widgets and "wherever parameters actually live".
//!
//! In the plugin, automatable parameters live in nih-plug's parameter system
//! and must be changed through a `ParamSetter` so the host sees the
//! automation. In the standalone harness they're plain values. Panes only
//! talk to this trait, so every pane works in both shells.

use std::ops::RangeInclusive;

use lattice_core::tuning;

#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum ParamKey {
    /// Offset of the lattice origin (C) in cents.
    COffset,
    /// Perfect fifth size in cents (prime 3 axis).
    Three,
    /// Major third size in cents (prime 5 axis).
    Five,
    /// Harmonic seventh size in cents (prime 7 axis).
    Seven,
    /// Node matching tolerance in cents.
    Tolerance,
    /// Seconds a note stays highlighted after release.
    HighlightTime,
    /// Seconds an octave indicator keeps fading after release.
    OctaveHighlightTime,
    /// Pitch (MIDI note) shown darkest on pitch-colored channels (10-14 in
    /// MIDI convention).
    DarkestPitch,
    /// Pitch shown brightest on pitch-colored channels.
    BrightestPitch,
}

/// Range for the tuning of each prime harmonic, in cents around just
/// intonation (same policy as v1).
pub const MAX_TUNING_OFFSET: f32 = 40.0;

impl ParamKey {
    pub const ALL: [ParamKey; 9] = [
        ParamKey::COffset,
        ParamKey::Three,
        ParamKey::Five,
        ParamKey::Seven,
        ParamKey::Tolerance,
        ParamKey::HighlightTime,
        ParamKey::OctaveHighlightTime,
        ParamKey::DarkestPitch,
        ParamKey::BrightestPitch,
    ];

    pub fn label(self) -> &'static str {
        match self {
            ParamKey::COffset => "C offset (¢)",
            ParamKey::Three => "Perfect fifth (¢)",
            ParamKey::Five => "Major third (¢)",
            ParamKey::Seven => "Harmonic seventh (¢)",
            ParamKey::Tolerance => "Tolerance (¢)",
            ParamKey::HighlightTime => "Pitch class fade (s)",
            ParamKey::OctaveHighlightTime => "Octave fade (s)",
            ParamKey::DarkestPitch => "Darkest pitch",
            ParamKey::BrightestPitch => "Brightest pitch",
        }
    }

    pub fn range(self) -> RangeInclusive<f32> {
        match self {
            ParamKey::COffset => -600.0..=600.0,
            ParamKey::Three => {
                tuning::THREE_JUST - MAX_TUNING_OFFSET..=tuning::THREE_JUST + MAX_TUNING_OFFSET
            }
            ParamKey::Five => {
                tuning::FIVE_JUST - MAX_TUNING_OFFSET..=tuning::FIVE_JUST + MAX_TUNING_OFFSET
            }
            ParamKey::Seven => {
                tuning::SEVEN_JUST - MAX_TUNING_OFFSET..=tuning::SEVEN_JUST + MAX_TUNING_OFFSET
            }
            ParamKey::Tolerance => 0.001..=49.999,
            ParamKey::HighlightTime => 0.0..=100.0,
            ParamKey::OctaveHighlightTime => 0.0..=100.0,
            // Same ranges as v1's grid params.
            ParamKey::DarkestPitch => 0.0..=60.0,
            ParamKey::BrightestPitch => 60.0..=120.0,
        }
    }

    pub fn logarithmic(self) -> bool {
        matches!(
            self,
            ParamKey::Tolerance | ParamKey::HighlightTime | ParamKey::OctaveHighlightTime
        )
    }
}

/// Read/write access to the automatable parameters.
///
/// Continuous edits (drags) should be bracketed with `begin_set`/`end_set`
/// so hosts record them as one automation gesture. `set` alone is fine for
/// one-shot changes (preset buttons, typed values) — backends wrap those in
/// an implicit gesture.
pub trait ParamBackend {
    fn get(&self, key: ParamKey) -> f32;
    fn set(&self, key: ParamKey, value: f32);
    /// Start of a continuous edit (e.g. drag). Default: no-op.
    fn begin_set(&self, key: ParamKey) {
        let _ = key;
    }
    /// End of a continuous edit. Default: no-op.
    fn end_set(&self, key: ParamKey) {
        let _ = key;
    }
}

/// Assemble a [`lattice_core::Tuning`] from the current parameter values.
pub fn tuning_from_params(params: &dyn ParamBackend) -> lattice_core::Tuning {
    lattice_core::Tuning {
        c_offset: params.get(ParamKey::COffset),
        three: params.get(ParamKey::Three),
        five: params.get(ParamKey::Five),
        seven: params.get(ParamKey::Seven),
        tolerance: params.get(ParamKey::Tolerance),
    }
}
