//! Which automatable parameter a value belongs to.
//!
//! A take records parameter changes by NAME — [`ParamKey::id`] — so the
//! renderer can put them back where they came from however the program's
//! internals have moved since. That makes the identity part of the take
//! format rather than part of any one shell, which is why it lives here
//! beside [`ParamRecord`](crate::ParamRecord) and not in the UI that edits it.
//!
//! The editing seam stays in `harmonigraph-ui`: `ParamBackend`, the trait each
//! shell implements to read and write these through its own parameter system.
//! A take names a parameter; it does not edit one. `harmonigraph-ui`
//! re-exports `ParamKey` beside that trait, so a pane still sees the two
//! together.

use std::ops::RangeInclusive;

use harmonigraph_core::tuning;


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
    /// `HarmonigraphParams`** — changing one orphans every project that
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
            // A tenth of a second: long enough that a release reads as a
            // fade rather than a cut, short enough that the node is clear
            // before the next one lands at playing tempo. A whole second
            // keeps released notes up long enough to blur which of them are
            // still sounding.
            ParamKey::Fade => 0.1,
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
            // Both ends span the whole MIDI range so the pair reads as one
            // two-handle control (the Nodes pane's Color range); ordering is
            // kept by the range bar's min span, not by a hard 60-note split.
            ParamKey::DarkestPitch => 0.0..=120.0,
            ParamKey::BrightestPitch => 0.0..=120.0,
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
