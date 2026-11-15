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

#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum ParamKey {
    /// Seconds a note takes to arrive, and to leave — both ends of it, on
    /// the one dial and never at once. The octave glyphs and the melody/bass
    /// marks ride it together, so a note is a single gesture across the whole
    /// node instead of layers starting and ending at different moments; a
    /// Stagger spread stretches the slices to this × (1 + spread) in all. The
    /// background glow runs on its own attack and release, and the Fade-out
    /// delay holds a release before it starts.
    Fade,
    /// Pitch (MIDI note) shown darkest on the gradient. Every note is
    /// colored by pitch, whatever channel carried it.
    DarkestPitch,
    /// Pitch shown brightest on the gradient.
    BrightestPitch,
    // Camera movement only; projection and appearance remain editor settings.
    CameraYaw,
    CameraPitch,
    CameraDistance,
    CameraPanX,
    CameraPanY,
}

impl ParamKey {
    /// Every key, for the host's parameter list and the round-trip tests.
    ///
    /// Built from an exhaustive `match` rather than written out as a literal,
    /// so the list cannot fall behind the enum — the same guard
    /// `SpectralOrientation::ALL` in `harmonigraph-ui` uses, for the same
    /// reason.
    pub const ALL: [ParamKey; 8] = {
        use ParamKey::*;
        // Exhaustive, and the compiler checks it. The arm is `()` because
        // what is wanted is the coverage error, not the value.
        const fn covered(key: ParamKey) {
            match key {
                Fade | DarkestPitch | BrightestPitch | CameraYaw | CameraPitch | CameraDistance
                | CameraPanX | CameraPanY => (),
            }
        }
        covered(Fade);
        [
            Fade,
            DarkestPitch,
            BrightestPitch,
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

    /// Pitch->color gradient endpoints (the Pitch color range on the
    /// Mappings page).
    pub const COLOR: [ParamKey; 2] = [ParamKey::DarkestPitch, ParamKey::BrightestPitch];

    pub fn label(self) -> &'static str {
        match self {
            ParamKey::CameraYaw => "Horizontal angle",
            ParamKey::CameraPitch => "Vertical angle",
            ParamKey::CameraDistance => "Camera distance",
            ParamKey::CameraPanX => "Camera pan X",
            ParamKey::CameraPanY => "Camera pan Y",

            // No unit in the name: the readout carries it (see `unit`).
            ParamKey::Fade => "Note fade",
            ParamKey::DarkestPitch => "Low color pitch",
            ParamKey::BrightestPitch => "High color pitch",
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

            ParamKey::Fade => "Note Fade (sec)",
            ParamKey::DarkestPitch => "Low Color Pitch",
            ParamKey::BrightestPitch => "High Color Pitch",
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
            ParamKey::CameraYaw => "camera-yaw",
            ParamKey::CameraPitch => "camera-pitch",
            ParamKey::CameraDistance => "camera-distance",
            ParamKey::CameraPanX => "camera-pan-x",
            ParamKey::CameraPanY => "camera-pan-y",

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

    /// The value a fresh plugin starts at.
    pub fn default_value(self) -> f32 {
        match self {
            ParamKey::CameraYaw => 0.4,
            ParamKey::CameraPitch => 0.3,
            ParamKey::CameraDistance => 12.0,
            ParamKey::CameraPanX => 1.0,
            ParamKey::CameraPanY => 0.0,

            // Both ends of a note, so this is read twice: as an arrival long
            // enough to take the hard edge off a note-on, and as a release
            // long enough to read as a fade rather than a cut. A third of a
            // second, as captured from the DAW on 2026-10-07. A
            // whole second keeps released notes up long enough to blur which
            // of them are still sounding, and puts a stab's arrival behind
            // the ear by a beat.
            ParamKey::Fade => 0.339_419_16,
            // C1 to C8, the seven-octave keyboard range. Notes outside it are
            // not lost: the ramp holds its end color past either end, so an
            // outlying note reads as the nearest end rather than wrapping.
            ParamKey::DarkestPitch => 24.0,
            ParamKey::BrightestPitch => 108.0,
        }
    }

    pub fn range(self) -> RangeInclusive<f32> {
        match self {
            ParamKey::CameraYaw => -std::f32::consts::PI..=std::f32::consts::PI,
            ParamKey::CameraPitch => -1.5..=1.5,
            ParamKey::CameraDistance => 2.0..=24.0,
            ParamKey::CameraPanX => -128.0..=128.0,
            ParamKey::CameraPanY => -128.0..=128.0,

            // A second, which is where a fade stops being a release and
            // starts being a held image of a note that is already over: past
            // it the lattice shows a chord the player has left, and the
            // reading of what is DOWN goes with it. The same line the mark
            // Delay is drawn on, so the two note-wide times share one scale
            // and can be read against each other.
            ParamKey::Fade => 0.0..=1.0,
            // Both ends span the whole MIDI range so the pair reads as one
            // two-handle control (the Mappings page's Pitch color range); ordering is
            // kept by the range bar's min span, not by a hard 60-note split.
            ParamKey::DarkestPitch => 0.0..=120.0,
            ParamKey::BrightestPitch => 0.0..=120.0,
        }
    }

    /// Numeric UI units: displayed units per stored unit, suffix and precision.
    /// Host automation and takes continue to store cents, seconds and MIDI pitch.
    pub fn unit(self) -> (f32, &'static str, usize) {
        match self {
            ParamKey::CameraYaw => (180.0 / std::f32::consts::PI, "°", 1),
            ParamKey::CameraPitch => (180.0 / std::f32::consts::PI, "°", 1),
            ParamKey::CameraDistance => (1.0, "", 2),
            ParamKey::CameraPanX => (1.0, " cells", 2),
            ParamKey::CameraPanY => (1.0, " cells", 2),

            ParamKey::Fade => (1000.0, " ms", 0),
            ParamKey::DarkestPitch | ParamKey::BrightestPitch => (1.0, " MIDI", 0),
        }
    }
}
