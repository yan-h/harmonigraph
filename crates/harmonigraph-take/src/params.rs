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
    // Camera movement only; projection and appearance remain editor settings.
    // The Note fade and the Pitch color range were keys here until #1744 and
    // are look settings now, beside the curve and the gradient they pair with.
    CameraYaw,
    CameraPitch,
    CameraDistance,
    CameraPanX,
    CameraPanY,
}

/// Range for the tuning of each prime harmonic, in cents around just
/// intonation (same policy as v1).
pub const MAX_TUNING_OFFSET: f32 = 40.0;

impl ParamKey {
    /// Every key, for the host's parameter list and the round-trip tests.
    ///
    /// Built from an exhaustive `match` rather than written out as a literal,
    /// so the list cannot fall behind the enum — the same guard
    /// `SpectralOrientation::ALL` in `harmonigraph-ui` uses, for the same
    /// reason.
    pub const ALL: [ParamKey; 10] = {
        use ParamKey::*;
        // Exhaustive, and the compiler checks it. The arm is `()` because
        // what is wanted is the coverage error, not the value.
        const fn covered(key: ParamKey) {
            match key {
                COffset | Three | Five | Seven | Tolerance | CameraYaw | CameraPitch
                | CameraDistance | CameraPanX | CameraPanY => (),
            }
        }
        covered(COffset);
        [
            COffset,
            Three,
            Five,
            Seven,
            Tolerance,
            CameraYaw,
            CameraPitch,
            CameraDistance,
            CameraPanX,
            CameraPanY,
        ]
    };

    pub const CAMERA: [ParamKey; 5] = [
        Self::CameraYaw,
        Self::CameraPitch,
        Self::CameraDistance,
        Self::CameraPanX,
        Self::CameraPanY,
    ];

    /// The structural tuning parameters (Tuning section of the UI).
    pub const TUNING: [ParamKey; 5] =
        [ParamKey::COffset, ParamKey::Three, ParamKey::Five, ParamKey::Seven, ParamKey::Tolerance];

    pub fn label(self) -> &'static str {
        match self {
            ParamKey::CameraYaw => "Horizontal angle",
            ParamKey::CameraPitch => "Vertical angle",
            ParamKey::CameraDistance => "Camera distance",
            ParamKey::CameraPanX => "Camera pan X",
            ParamKey::CameraPanY => "Camera pan Y",

            ParamKey::COffset => "C pitch offset",
            ParamKey::Three => "Perfect fifth",
            ParamKey::Five => "Major third",
            ParamKey::Seven => "Harmonic seventh",
            ParamKey::Tolerance => "Note match tolerance",
        }
    }

    /// The name shown to the host (parameter lists, automation lanes).
    /// Deliberately distinct from [`label`](Self::label): hosts get
    /// spelled-out units, the narrow in-plugin UI gets symbols.
    pub fn host_name(self) -> &'static str {
        match self {
            ParamKey::CameraYaw => "Camera Yaw (rad)",
            ParamKey::CameraPitch => "Camera Pitch (rad)",
            ParamKey::CameraDistance => "Camera Distance",
            ParamKey::CameraPanX => "Camera Pan X (cells)",
            ParamKey::CameraPanY => "Camera Pan Y (cells)",

            ParamKey::COffset => "C Pitch Offset (cents)",
            ParamKey::Three => "Perfect Fifth (cents)",
            ParamKey::Five => "Major Third (cents)",
            ParamKey::Seven => "Harmonic Seventh (cents)",
            ParamKey::Tolerance => "Note Match Tolerance (cents)",
        }
    }

    /// The stable string id this parameter is known by outside the
    /// program: the host's automation lane, a saved project, a recorded
    /// take. **These strings must match the `#[id = "..."]` attributes on
    /// `HarmonigraphParams`** — changing one orphans every project that
    /// automates it.
    pub fn id(self) -> &'static str {
        match self {
            ParamKey::CameraYaw => "camera-yaw",
            ParamKey::CameraPitch => "camera-pitch",
            ParamKey::CameraDistance => "camera-distance",
            ParamKey::CameraPanX => "camera-pan-x",
            ParamKey::CameraPanY => "camera-pan-y",

            ParamKey::COffset => "tuning-c-offset",
            ParamKey::Three => "tuning-three",
            ParamKey::Five => "tuning-five",
            ParamKey::Seven => "tuning-seven",
            ParamKey::Tolerance => "tuning-tolerance",
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
            ParamKey::CameraYaw => 0.4,
            ParamKey::CameraPitch => 0.3,
            ParamKey::CameraDistance => 12.0,
            ParamKey::CameraPanX => 1.0,
            ParamKey::CameraPanY => 0.0,

            ParamKey::COffset => 0.0,
            ParamKey::Three => tuning::THREE_12TET,
            ParamKey::Five => tuning::FIVE_12TET,
            ParamKey::Seven => tuning::SEVEN_12TET,
            ParamKey::Tolerance => 0.5,
        }
    }

    pub fn range(self) -> RangeInclusive<f32> {
        match self {
            ParamKey::CameraYaw => -std::f32::consts::PI..=std::f32::consts::PI,
            ParamKey::CameraPitch => -1.5..=1.5,
            ParamKey::CameraDistance => 2.0..=24.0,
            ParamKey::CameraPanX => -128.0..=128.0,
            ParamKey::CameraPanY => -128.0..=128.0,

            ParamKey::COffset => -600.0..=600.0,
            ParamKey::Three => {
                tuning::THREE_JUST - MAX_TUNING_OFFSET..=tuning::THREE_JUST + MAX_TUNING_OFFSET
            }
            ParamKey::Five => {
                // A released link must fit in the host parameter without a
                // pitch jump. Include every third the fifth can derive.
                let fifth = ParamKey::Three.range();
                tuning::meantone_third(*fifth.start()).floor()
                    ..=tuning::meantone_third(*fifth.end()).ceil()
            }
            ParamKey::Seven => {
                let (fifth, third) = (ParamKey::Three.range(), ParamKey::Five.range());
                tuning::marvel_seventh(*fifth.start(), *third.start()).floor()
                    ..=tuning::marvel_seventh(*fifth.end(), *third.end()).ceil()
            }
            ParamKey::Tolerance => 0.001..=49.999,
        }
    }

    /// Whether the bar and the host-facing range are skewed toward the low
    /// end. Only the Tolerance is: it lives in its first hundredth, a
    /// hundredth of a cent being a real setting and forty cents an absurd
    /// one, so a linear bar would spend its whole travel on values nobody
    /// picks.
    pub fn logarithmic(self) -> bool {
        matches!(self, ParamKey::Tolerance)
    }

    /// Numeric UI units: displayed units per stored unit, suffix and precision.
    /// Host automation and takes continue to store cents and camera units.
    pub fn unit(self) -> (f32, &'static str, usize) {
        match self {
            ParamKey::CameraYaw => (180.0 / std::f32::consts::PI, "°", 1),
            ParamKey::CameraPitch => (180.0 / std::f32::consts::PI, "°", 1),
            ParamKey::CameraDistance => (1.0, "", 2),
            ParamKey::CameraPanX => (1.0, " cells", 2),
            ParamKey::CameraPanY => (1.0, " cells", 2),

            ParamKey::Tolerance => (1.0, "¢", 3),
            ParamKey::COffset | ParamKey::Three | ParamKey::Five | ParamKey::Seven => (1.0, "¢", 2),
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
