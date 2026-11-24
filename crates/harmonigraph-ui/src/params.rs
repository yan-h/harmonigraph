//! The bridge between UI widgets and "wherever parameters actually live".
//!
//! In the plugin, automatable parameters live in the CLAP shell's parameter
//! table and must be changed through its `EditorHost` so the host sees the
//! automation. In the standalone harness they're plain values. Panes only
//! talk to this trait, so every pane works in both shells.
//!
//! [`ParamKey`] itself — which parameter, and the id it is known by outside
//! the program — lives in `harmonigraph-take`, because a take records
//! parameter changes by that id and so the identity is part of the take format
//! rather than of any one shell. It is re-exported here so a pane sees the key
//! and the seam that edits it together, which is how every call site already
//! reads.

pub use harmonigraph_take::params::ParamKey;

/// Which of the plugin's audio inputs feeds every audio-derived picture.
///
/// Shell capability rather than view state: the standalone has only its mock
/// synth and the offline renderer has only the take's recorded audio, so only a
/// plugin backend offers this choice. The plugin maps it onto a host parameter
/// that saves its index.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum AnalysisInput {
    #[default]
    Main,
    Sidechain,
}

impl AnalysisInput {
    /// Every input, in host order: `HOST[i].index() == i`.
    pub const HOST: [Self; 2] = [Self::Main, Self::Sidechain];
    pub const HOST_MAX: i32 = Self::HOST.len() as i32 - 1;

    /// The host parameter integer. Saved projects hold these, so a number
    /// never moves.
    pub fn index(self) -> i32 {
        match self {
            Self::Main => 0,
            Self::Sidechain => 1,
        }
    }
    /// The input at a host integer, clamped into range as the host's label
    /// clamps, so the two never disagree.
    pub fn from_index(value: i32) -> Self {
        Self::HOST[value.clamp(0, Self::HOST_MAX) as usize]
    }
    /// What the host and the editor call this input.
    pub fn label(self) -> &'static str {
        match self {
            Self::Main => "Main",
            Self::Sidechain => "Sidechain",
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
    fn lattice_maps(&self) -> Option<crate::lattice_maps::MapView> {
        None
    }
    fn edit_lattice_map(&self, _edit: crate::lattice_maps::MapEdit) {}
    /// Live plugin instances; absent in offline and standalone pictures.
    fn tuning_instances(&self) -> Vec<TuningInstance> {
        Vec::new()
    }
    fn edit_tuning_instance(&self, _id: u64, _edit: InstanceEdit) {}

    /// The shell's tuning configuration as the editor draws it, when the
    /// shell owns one (the plugin).
    fn configuration(&self) -> Option<ConfigurationView> {
        None
    }
    /// Apply `edit` to the shell's configuration and return it as
    /// [`configuration`](Self::configuration) now reads it, for the rest of
    /// the frame to draw (the plugin). None selects the synchronous path the
    /// standalone and offline shells use, where the UI's own reducer takes the
    /// edit.
    fn submit_tuning(
        &self,
        _edit: harmonigraph_core::configuration::ConfigEdit,
    ) -> Option<ConfigurationView> {
        None
    }

    /// A movement channel owned by this backend. None leaves that channel
    /// in the appearance snapshot (standalone or a take without camera records).
    fn camera_value(&self, _key: ParamKey) -> Option<f32> {
        None
    }

    /// A movement channel's value before the host's modulation, which
    /// [`Self::set`] writes: [`Self::camera_value`] where nothing modulates it.
    fn camera_base(&self, key: ParamKey) -> Option<f32> {
        self.camera_value(key)
    }

    fn get(&self, key: ParamKey) -> f32;
    fn set(&self, key: ParamKey, value: f32);
    /// Which live audio input the shell is analyzing, or `None` when the shell
    /// has no choice of inputs to offer.
    fn analysis_input(&self) -> Option<AnalysisInput> {
        None
    }
    /// Change the live analysis input. A one-shot host parameter gesture in the
    /// plugin; a no-op in shells that return `None` above.
    fn set_analysis_input(&self, input: AnalysisInput) {
        let _ = input;
    }
    /// Start of a continuous edit (e.g. drag). Default: no-op.
    fn begin_set(&self, key: ParamKey) {
        let _ = key;
    }
    /// End of a continuous edit. Default: no-op.
    fn end_set(&self, key: ParamKey) {
        let _ = key;
    }
}

/// A sounding MIDI key and its current output pitch, independent of Show.
#[derive(Clone, Copy, Debug)]
pub struct TuningVoice {
    /// Channel * 128 + key: stable across tuning and pitch-bend changes.
    pub key: u16,
    /// MIDI note units, including tuning and channel bend.
    pub pitch: f32,
}

/// Cosmetic snapshots and main-thread commands, never audio-owned references.
#[derive(Clone, Debug)]
pub struct TuningInstance {
    pub id: u64,
    /// User override; empty follows the host track name.
    pub name: String,
    pub display_name: String,
    pub is_hub: bool,
    pub retune: bool,
    pub show: bool,
    pub voices: Vec<TuningVoice>,
    pub notes_in: u64,
    pub notes_out: u64,
    pub misses: u64,
    pub delay: u32,
    pub max_delay: u32,
    pub delay_text: String,
    pub status: String,
    /// Whether the row's name is drawn in the fault colour: before the host
    /// first activates the instance, while a status bit is set, and while
    /// missed corrections are counted (`docs/intent/tuning.md`). Decided
    /// where the status is, rather than read back out of `status`'s wording.
    pub faulted: bool,
    pub last_pitch: Option<(f64, f64)>,
}

#[derive(Clone, Debug)]
pub enum InstanceEdit {
    Retune(bool),
    Show(bool),
    Name(String),
    Delay(u32),
    Reset,
}

/// The plugin's tuning configuration, the document its main thread owns
/// (#1938), with what its audio thread reports about it.
#[derive(Clone, Copy, Debug)]
pub struct ConfigurationView {
    /// The configuration as the editor draws it: the saved one, or with
    /// `board` the lattice-board's tuning drawn over it.
    pub resolved: harmonigraph_core::configuration::ResolvedConfig,
    /// Whether `resolved` is drawn in the lattice-board's tuning
    /// (`ResolvedConfig::drawn`), decided once with it so every control in a
    /// frame agrees about which value it shows.
    pub board: bool,
    /// The bits below.
    pub status: u32,
    /// The configuration's revision while the audio thread has not adopted
    /// it, or `None` once it has: the request the pending label waits on.
    pub unadopted: Option<u64>,
}

impl ConfigurationView {
    /// Host input was lost: a take refuses its configuration until a reset.
    pub const INPUT_LOST: u32 = 2;
    /// Learn heard more distinct classes than it reads.
    pub const TOO_MANY_CLASSES: u32 = 16;
}
