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

/// How a NOTE TIME reads out: two decimals and the unit on the number.
///
/// One function for both of them — the mark Delay is a view setting and the
/// Fade is a param, so they are built by different code and would otherwise
/// carry two copies of this literal. They are one second each and are read
/// against each other constantly (see [`ParamKey::logarithmic`]); retuning the
/// readout has to move both or it makes them look like different kinds of
/// setting again.
pub fn seconds(v: f32) -> String {
    format!("{v:.2} s")
}


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
    /// Pitch (MIDI note) shown darkest on the gradient. Every note is
    /// colored by pitch, whatever channel carried it.
    DarkestPitch,
    /// Pitch shown brightest on the gradient.
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
            // No unit in the name: the readout carries it (see `display`).
            ParamKey::Fade => "Fade",
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
            // Both ends of a note, so this is read twice: as an arrival long
            // enough to take the hard edge off a note-on, and as a release
            // long enough to read as a fade rather than a cut while still
            // clearing the node before the next one lands at playing tempo.
            // 0.15 is where those two agree. A whole second keeps released
            // notes up long enough to blur which of them are still sounding,
            // and puts a stab's arrival behind the ear by a beat.
            ParamKey::Fade => 0.15,
            // G♯0 to F6 — a shade under six octaves, set inside the MIDI
            // range rather than at its ends (C0 to C7 is the whole of what a
            // keyboard reaches, and the octaves nobody plays spend the
            // gradient's arc on nothing). Notes outside it are not lost: the
            // ramp holds its end color past either end, so an outlying bass
            // note reads as the bottom of the range rather than wrapping.
            ParamKey::DarkestPitch => 32.478_26,
            ParamKey::BrightestPitch => 100.691_56,
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
            // A second, which is where a fade stops being a release and
            // starts being a held image of a note that is already over: past
            // it the lattice shows a chord the player has left, and the
            // reading of what is DOWN goes with it. The same line the mark
            // Delay is drawn on, so the two note-wide times share one scale
            // and can be read against each other.
            ParamKey::Fade => 0.0..=1.0,
            // Both ends span the whole MIDI range so the pair reads as one
            // two-handle control (the Nodes pane's Color range); ordering is
            // kept by the range bar's min span, not by a hard 60-note split.
            ParamKey::DarkestPitch => 0.0..=120.0,
            ParamKey::BrightestPitch => 0.0..=120.0,
        }
    }

    /// Whether the bar and the host-facing range are skewed toward the low
    /// end. Only the Tolerance is: it lives in its first hundredth, a
    /// hundredth of a cent being a real setting and forty cents an absurd
    /// one, so a linear bar would spend its whole travel on values nobody
    /// picks.
    ///
    /// The Fade is NOT. It runs 0..1s against the mark Delay's own linear
    /// second, and a hundredth of that — the readout's resolution — is already
    /// a couple of pixels of travel, so there is no crushed low end for an
    /// ease to rescue. What an ease would cost is the two bars agreeing: they
    /// are read against each other constantly, and a Fade whose middle is at
    /// 0.2s cannot be eyeballed against a Delay whose middle is at 0.5s.
    pub fn logarithmic(self) -> bool {
        matches!(self, ParamKey::Tolerance)
    }

    /// How the value READS OUT where a bare decimal would not say what it is.
    /// `None` leaves the bar's plain formatting.
    ///
    /// The unit rides the NUMBER rather than the name, matching the Delay bar
    /// above the Fade — a bar whose name carries the unit and one whose
    /// readout does look like two different kinds of setting, and these two
    /// are the same kind.
    pub fn display(self) -> Option<fn(f32) -> String> {
        match self {
            ParamKey::Fade => Some(seconds),
            _ => None,
        }
    }

    /// Skew steepness for the [`logarithmic`](Self::logarithmic) params
    /// (more negative = more resolution at the low end). The plugin feeds
    /// this to nice-plug's skewed ranges; the UI's eased ValueBars only
    /// consult `logarithmic()`. Meaningless for linear params, which is
    /// every key but the Tolerance — the arm below is what keeps this
    /// total, not a second setting.
    pub fn skew_steepness(self) -> f32 {
        match self {
            ParamKey::Tolerance => -2.5,
            _ => -2.0,
        }
    }
}
