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
    // Camera movement only; projection and appearance remain editor settings.
    // The Note fade and the Pitch color range were keys here until #1744 and
    // are look settings now, beside the curve and the gradient they pair
    // with. The tuning axes were keys until #1756 and are plugin settings now,
    // which a take records in its configuration records.
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
    pub const ALL: [ParamKey; 5] = {
        use ParamKey::*;
        // Exhaustive, and the compiler checks it. The arm is `()` because
        // what is wanted is the coverage error, not the value.
        const fn covered(key: ParamKey) {
            match key {
                CameraYaw | CameraPitch | CameraDistance | CameraPanX | CameraPanY => (),
            }
        }
        covered(CameraYaw);
        [CameraYaw, CameraPitch, CameraDistance, CameraPanX, CameraPanY]
    };

    pub const CAMERA: [ParamKey; 5] = Self::ALL;

    pub fn label(self) -> &'static str {
        match self {
            ParamKey::CameraYaw => "Horizontal angle",
            ParamKey::CameraPitch => "Vertical angle",
            ParamKey::CameraDistance => "Camera distance",
            ParamKey::CameraPanX => "Camera pan X",
            ParamKey::CameraPanY => "Camera pan Y",
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
        }
    }

    /// The stable string id this parameter is known by outside the
    /// program: the host's automation lane, a saved project, a recorded
    /// take. **These strings are the ids of the plugin's camera rows**
    /// (`Row::id` in harmonigraph-plugin) — changing one orphans every
    /// project that automates it.
    pub fn id(self) -> &'static str {
        match self {
            ParamKey::CameraYaw => "camera-yaw",
            ParamKey::CameraPitch => "camera-pitch",
            ParamKey::CameraDistance => "camera-distance",
            ParamKey::CameraPanX => "camera-pan-x",
            ParamKey::CameraPanY => "camera-pan-y",
        }
    }

    /// The inverse of [`id`](Self::id). Unknown ids give `None` — a take
    /// recorded by a newer build may name a parameter this one has never
    /// heard of, and an older one a parameter since retired; skipping it
    /// beats refusing to render.
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
        }
    }

    pub fn range(self) -> RangeInclusive<f32> {
        match self {
            ParamKey::CameraYaw => -std::f32::consts::PI..=std::f32::consts::PI,
            ParamKey::CameraPitch => -1.5..=1.5,
            ParamKey::CameraDistance => 2.0..=24.0,
            ParamKey::CameraPanX => -128.0..=128.0,
            ParamKey::CameraPanY => -128.0..=128.0,
        }
    }

    /// Numeric UI units: displayed units per stored unit, suffix and precision.
    /// Host automation and takes continue to store camera units.
    pub fn unit(self) -> (f32, &'static str, usize) {
        match self {
            ParamKey::CameraYaw => (180.0 / std::f32::consts::PI, "°", 1),
            ParamKey::CameraPitch => (180.0 / std::f32::consts::PI, "°", 1),
            ParamKey::CameraDistance => (1.0, "", 2),
            ParamKey::CameraPanX => (1.0, " cells", 2),
            ParamKey::CameraPanY => (1.0, " cells", 2),
        }
    }
}
