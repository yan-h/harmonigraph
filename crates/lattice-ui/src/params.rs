//! The bridge between UI widgets and "wherever parameters actually live".
//!
//! In the plugin, automatable parameters live in nice-plug's parameter system
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
    /// Seconds a released note keeps fading. ONE time for every note
    /// indicator — the pitch class core, the octave glyphs, and the
    /// melody/bass marks all ride it, so a release is a single gesture
    /// across the whole node instead of layers ending at different moments.
    Fade,
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
    pub const ALL: [ParamKey; 8] = [
        ParamKey::COffset,
        ParamKey::Three,
        ParamKey::Five,
        ParamKey::Seven,
        ParamKey::Tolerance,
        ParamKey::Fade,
        ParamKey::DarkestPitch,
        ParamKey::BrightestPitch,
    ];

    /// The structural tuning parameters (Tuning section of the UI).
    pub const TUNING: [ParamKey; 5] = [
        ParamKey::COffset,
        ParamKey::Three,
        ParamKey::Five,
        ParamKey::Seven,
        ParamKey::Tolerance,
    ];

    /// Pitch->color gradient endpoints (Color group of the Appearance UI).
    pub const COLOR: [ParamKey; 2] = [ParamKey::DarkestPitch, ParamKey::BrightestPitch];

    pub fn label(self) -> &'static str {
        match self {
            ParamKey::COffset => "C offset (¢)",
            ParamKey::Three => "Perfect fifth (¢)",
            ParamKey::Five => "Major third (¢)",
            ParamKey::Seven => "Harmonic seventh (¢)",
            ParamKey::Tolerance => "Tolerance (¢)",
            ParamKey::Fade => "Fade (s)",
            ParamKey::DarkestPitch => "Darkest pitch",
            ParamKey::BrightestPitch => "Brightest pitch",
        }
    }

    /// The name shown to the host (parameter lists, automation lanes).
    /// Deliberately distinct from [`label`](Self::label): hosts get
    /// spelled-out units, the narrow in-plugin UI gets symbols.
    pub fn host_name(self) -> &'static str {
        match self {
            ParamKey::COffset => "C Offset (cents)",
            ParamKey::Three => "Perfect Fifth (cents)",
            ParamKey::Five => "Major Third (cents)",
            ParamKey::Seven => "Harmonic Seventh (cents)",
            ParamKey::Tolerance => "Tuning Tolerance (cents)",
            ParamKey::Fade => "Note Fade (sec)",
            ParamKey::DarkestPitch => "Darkest Pitch",
            ParamKey::BrightestPitch => "Brightest Pitch",
        }
    }

    /// The stable string id this parameter is known by outside the
    /// program: the host's automation lane, a saved project, a recorded
    /// take. **These strings must match the `#[id = "..."]` attributes on
    /// `MidiLattice3dParams`** — changing one orphans every project that
    /// automates it, which is why `Fade` still carries its pre-merge
    /// `pitch-class-fade` id.
    pub fn id(self) -> &'static str {
        match self {
            ParamKey::COffset => "tuning-c-offset",
            ParamKey::Three => "tuning-three",
            ParamKey::Five => "tuning-five",
            ParamKey::Seven => "tuning-seven",
            ParamKey::Tolerance => "tuning-tolerance",
            ParamKey::Fade => "pitch-class-fade",
            ParamKey::DarkestPitch => "darkest-pitch",
            ParamKey::BrightestPitch => "brightest-pitch",
        }
    }

    /// The inverse of [`id`](Self::id). Unknown ids give `None` — a take
    /// recorded by a newer build may name a parameter this one has never
    /// heard of, and skipping it beats refusing to render.
    pub fn from_id(id: &str) -> Option<ParamKey> {
        ParamKey::ALL.into_iter().find(|key| key.id() == id)
    }

    /// Default value: 12-TET tuning as in v1, so the lattice matches a
    /// plain MIDI keyboard until the user dials in a tuning. Shells may
    /// deliberately override (the standalone harness demos a just lattice).
    pub fn default_value(self) -> f32 {
        match self {
            ParamKey::COffset => 0.0,
            ParamKey::Three => tuning::THREE_12TET,
            ParamKey::Five => tuning::FIVE_12TET,
            ParamKey::Seven => tuning::SEVEN_12TET,
            ParamKey::Tolerance => 0.5,
            ParamKey::Fade => 1.0,
            ParamKey::DarkestPitch => 24.0,
            ParamKey::BrightestPitch => 108.0,
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
            ParamKey::Fade => 0.0..=100.0,
            // Same ranges as v1's grid params.
            ParamKey::DarkestPitch => 0.0..=60.0,
            ParamKey::BrightestPitch => 60.0..=120.0,
        }
    }

    pub fn logarithmic(self) -> bool {
        matches!(
            self,
            ParamKey::Tolerance | ParamKey::Fade
        )
    }

    /// Skew steepness for the [`logarithmic`](Self::logarithmic) params
    /// (more negative = more resolution at the low end). The plugin feeds
    /// this to nice-plug's skewed ranges; the UI's eased ValueBars only
    /// consult `logarithmic()`. Meaningless for linear params.
    pub fn skew_steepness(self) -> f32 {
        match self {
            ParamKey::Tolerance => -2.5,
            _ => -2.0,
        }
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
/// The f32 cent params are quantized to microcents here, once, at the
/// param boundary; all lattice pitch math downstream is exact integers.
pub fn tuning_from_params(params: &dyn ParamBackend) -> lattice_core::Tuning {
    lattice_core::Tuning::from_cents(
        params.get(ParamKey::COffset),
        params.get(ParamKey::Three),
        params.get(ParamKey::Five),
        params.get(ParamKey::Seven),
        params.get(ParamKey::Tolerance),
    )
}
