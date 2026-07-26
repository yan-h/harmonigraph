//! The shared UI shell: dockable panes, cross-pane hover state, and the
//! per-frame root function. Both the standalone harness and the plugin
//! editor call [`root_ui`] once per egui frame; everything else is internal.

pub mod layout;
pub mod params;
pub mod theme;
/// Haloed label text, collected as glyphs and drawn by one callback.
pub(crate) mod text;
pub mod widgets;
mod panes;
mod perf;

/// Folding a pane sideways, which egui_dock's own collapse arrow only does
/// downwards.
mod fold;

pub use layout::{Layout, Placement, PRESETS};

use perf::PerfStats;

use std::collections::VecDeque;

use egui_dock::{DockArea, DockState, NodeIndex};
use lattice_core::{LatticePos, NoteTracker, PitchClass, Tuning};
use lattice_render::wgpu::TextureFormat;
use lattice_scene::{Camera, FrameParams, ViewConfig};
use params::ParamBackend;

/// Scrollback for the debug console pane. Shells and panes log via
/// [`SharedState::log`].
#[derive(Default)]
pub struct Console {
    lines: VecDeque<String>,
}

impl Console {
    /// Lines kept before the oldest is dropped.
    const MAX_LINES: usize = 500;

    pub fn log(&mut self, line: impl Into<String>) {
        if self.lines.len() == Self::MAX_LINES {
            self.lines.pop_front();
        }
        self.lines.push_back(line.into());
    }

    pub fn lines(&self) -> impl Iterator<Item = &str> {
        self.lines.iter().map(String::as_str)
    }

    pub fn clear(&mut self) {
        self.lines.clear();
    }
}

/// The Spectral pane's analysis window length, picked in the Spectrum
/// settings tab: longer windows resolve bass pitch more sharply but
/// respond more slowly (the tradeoff is physics, not implementation).
#[derive(Clone, Copy, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub enum SpectrumWindow {
    /// 4096 samples (~85 ms at 48 kHz).
    Fast,
    /// 8192 samples (~171 ms): the default balance.
    Balanced,
    /// 16384 samples (~341 ms).
    Precise,
}

impl SpectrumWindow {
    pub fn samples(self) -> usize {
        match self {
            SpectrumWindow::Fast => 4096,
            SpectrumWindow::Balanced => 8192,
            SpectrumWindow::Precise => 16384,
        }
    }
}

/// What the Spectral pane's axis gridlines are labeled with.
#[derive(Clone, Copy, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub enum SpectrumLabels {
    /// A gridline at every C, labeled with Bitwig octave numbers.
    Notes,
    /// Gridlines on the analyzer-standard 1-2-5 series (20, 50, 100, ...
    /// 10k, 20k Hz).
    Frequency,
}

/// Which way the Spectral pane's pitch axis runs.
///
/// The pane is written once against an abstract (pitch, depth) plane and
/// mapped onto the screen at draw time, so every element — gridlines,
/// spectrum curve, piano roll — turns together.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub enum SpectralOrientation {
    /// Follow the pane's shape: time runs along the LONG side (so a
    /// scrolling spectrogram gets the room), pitch across the short one.
    /// Reads correctly both under the lattice and beside it, no setting to
    /// change.
    #[default]
    Auto,
    /// "Across": time runs left(now)->right(past) along the pane, pitch
    /// climbs bottom->top, and the spectrum sits on the left, joined to the
    /// spectrogram. (Serialized name kept from when this meant pitch axis.)
    Horizontal,
    /// "Upright": time runs top(now)->bottom(past) down the pane, pitch runs
    /// left->right, and the spectrum sits on top, joined to the spectrogram.
    Vertical,
}

impl SpectralOrientation {
    /// Whether TIME (the spectrogram/roll axis) runs vertically down the
    /// pane, with pitch across it. Resolves [`Auto`](Self::Auto) to the
    /// pane's long side.
    fn is_time_vertical(self, rect: egui::Rect) -> bool {
        match self {
            SpectralOrientation::Auto => rect.height() > rect.width(),
            SpectralOrientation::Horizontal => false,
            SpectralOrientation::Vertical => true,
        }
    }
}

/// What colors a note in the piano roll.
#[derive(Clone, Copy, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub enum RollColor {
    /// The lattice's own channel colors, so a note is the same color here
    /// as the node it lit up.
    Channel,
    /// The pitch gradient every channel-9..13 voice uses, applied to all
    /// channels — reads as a single ramp low-to-high.
    Pitch,
    /// One flat accent color: the roll recedes and the lattice leads.
    Accent,
}

/// The color ramp a spectrogram cell's intensity maps through. A set of
/// looks to pick from — the spectrogram is a heatmap, and the palette is
/// most of its character. Intensity always runs dark (quiet) to bright
/// (loud); these differ only in the hues it passes through.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub enum SpectrogramColor {
    /// Grayscale: black to white. The classic, and the most neutral over
    /// the roll's own colors.
    Mono,
    /// Black → navy → blue → cyan → white. The cool ramp.
    Ice,
    /// Black → violet → teal → green → yellow. A perceptually even ramp
    /// (viridis-like) where every step reads as an equal change — the
    /// default, because an even ramp is the one that reads a heatmap's
    /// quiet detail honestly.
    #[default]
    Aurora,
    /// Black → indigo → magenta → orange → cream. Warmer than
    /// [`Self::Aurora`] and evenly stepped like it.
    ///
    /// The aliases absorb palettes that used to exist: Heat (the familiar
    /// black-red-orange-yellow-white spectrogram, dropped for spending most
    /// of its range in the reds — Magma is that warmth evenly stepped),
    /// Pitch, which tinted each cell with the lattice's own low-to-high
    /// pitch color, and Paper, an inverted ramp for light backgrounds.
    /// Without them a blob still naming one wouldn't just lose its palette:
    /// the parse would fail and drop the WHOLE persist, layout and camera
    /// with it.
    #[serde(alias = "Heat", alias = "Pitch", alias = "Paper")]
    Magma,
}

/// What counts as "the take is done", and so when a video gets rendered.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub enum RenderTrigger {
    /// When you switch Record take off. Predictable, and works no matter how
    /// the transport behaves.
    #[default]
    OnDisarm,
    /// As soon as the transport stops after recording something — so a
    /// play-through, or an audio export, produces a video with nothing
    /// further to click. Recording disarms itself at the same moment.
    ///
    /// Falls back gracefully: if a host stops calling `process` the
    /// instant a render finishes, the stop is never observed and the take
    /// simply waits for you to disarm it, as before.
    OnTransportStop,
    /// When the arranger loop first repeats: exactly one loop is recorded, the
    /// take ends at the loop's end, and that pass renders — no catching the
    /// stop by hand. Meant for looped recording, where a manual stop is always
    /// a beat or two off.
    ///
    /// Detected by the transport wrapping, so **looping must be enabled**.
    /// Hosts don't reliably tell a plugin where the loop markers are (Bitwig
    /// doesn't flag its loop as active, so nih-plug's loop range is `None`), so
    /// with looping off there is nothing to wrap on and it waits for you to
    /// disarm, like [`OnDisarm`](Self::OnDisarm).
    AtLoopEnd,
}

/// How a finished take gets turned into a video, edited in the Video
/// pane's Record section and persisted with the UI state.
///
/// The plugin cannot render video itself — that is `lattice-offline`, a
/// separate binary with a headless GPU device and an ffmpeg pipe, and
/// nothing about it belongs inside a real-time audio plugin. What the
/// plugin can do is *run* it, the moment a take is complete.
#[derive(Clone, Debug, serde::Serialize, serde::Deserialize)]
pub struct RenderConfig {
    /// Record the plugin's audio input into the take (see
    /// `SharedState::take_audio`, which mirrors this at runtime).
    #[serde(default)]
    pub record_audio: bool,
    /// Run the renderer as soon as a take finishes.
    #[serde(default)]
    pub auto_render: bool,
    /// What "finishes" means; see [`RenderTrigger`].
    #[serde(default)]
    pub trigger: RenderTrigger,
    /// Path to the `lattice-offline` binary. Empty means the
    /// conventional install location, which `update-plugin.sh` writes to.
    #[serde(default)]
    pub renderer_path: String,
    /// Bounced audio to pass as `--audio`: it feeds the spectrum curve
    /// and is muxed into the video. Empty renders silent, with no
    /// spectrum — the roll and the lattice are unaffected.
    #[serde(default)]
    pub audio_path: String,
    /// Take-time (seconds) where the bounce starts — empty means auto-align to
    /// the MIDI onsets, a number passes `--align`. A string so "empty = auto"
    /// reads naturally and it matches the other free-text fields.
    #[serde(default)]
    pub audio_offset: String,
    /// Extra flags, split on whitespace (no shell quoting):
    /// `--size 3840x2160 --layout side-by-side`.
    #[serde(default)]
    pub extra_args: String,
    /// Whole-song playhead spectrogram: lay the take out at once and sweep a
    /// playhead through it, instead of the live scrolling window. Read by the
    /// offline renderer from the take; `--playhead` on the command line also
    /// turns it on. Needs audio.
    #[serde(default)]
    pub playhead: bool,
    /// The composed video frame — aspect ratio and the lattice/spectral split.
    /// Edited and previewed in the Video pane; the offline renderer reads it
    /// to compose the same picture.
    #[serde(default)]
    pub frame: RenderFrame,
}

impl Default for RenderConfig {
    fn default() -> Self {
        RenderConfig {
            record_audio: false,
            auto_render: false,
            trigger: RenderTrigger::OnDisarm,
            renderer_path: String::new(),
            audio_path: String::new(),
            audio_offset: String::new(),
            extra_args: "--size 1920x1080".into(),
            playhead: false,
            frame: RenderFrame::default(),
        }
    }
}

/// The video frame the Video pane composes: an aspect ratio plus the
/// lattice/spectral split. Aspect is size-agnostic (the render's resolution is
/// chosen separately); the split feeds [`Layout::split`], so the plugin's live
/// preview and the offline renderer build the identical frame.
#[derive(Clone, Copy, Debug, serde::Serialize, serde::Deserialize)]
pub struct RenderFrame {
    /// Frame aspect numerator (e.g. 16 of 16:9). Drives the preview letterbox
    /// and the render's default resolution.
    #[serde(default = "default_aspect_w")]
    pub aspect_w: u32,
    #[serde(default = "default_aspect_h")]
    pub aspect_h: u32,
    /// The lattice's share of the frame, `0..1` (the rest is the spectral pane).
    #[serde(default = "default_frame_split")]
    pub split: f32,
    /// Lattice over spectral, rather than side by side.
    #[serde(default)]
    pub stacked: bool,
}

fn default_aspect_w() -> u32 {
    16
}
fn default_aspect_h() -> u32 {
    9
}
fn default_frame_split() -> f32 {
    0.68
}

impl Default for RenderFrame {
    fn default() -> Self {
        RenderFrame { aspect_w: 16, aspect_h: 9, split: 0.68, stacked: false }
    }
}

/// Everything the Spectral pane's display is configured by, edited in the
/// Spectrum settings tab and persisted with the UI state.
#[derive(Clone, Copy, Debug, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct SpectrumConfig {
    /// Horizontal (pitch left-to-right) or vertical (pitch bottom-to-top),
    /// or Auto to follow the pane's shape; see [`SpectralOrientation`]. Those
    /// are the only orientations offered — the spectrum always stands up from
    /// its baseline with pitch ascending, so there is nothing to flip.
    #[serde(default)]
    pub orientation: SpectralOrientation,
    pub window: SpectrumWindow,
    /// Bottom of the dB height scale: what reads as silence. A full-scale
    /// sine sits at 0 dB.
    pub floor_db: f32,
    /// Top of the dB height scale: what reads as full height (and as the
    /// brightest spectrogram cell). 0 dB — a full-scale sine — is where it
    /// starts and the loudest it goes; pulling it down lifts quiet material
    /// into the whole picture instead of the bottom of it.
    ///
    /// The pair is one control, like the pitch range: the window on the
    /// spectrum's dynamics, movable at either end.
    #[serde(default = "default_ceiling_db")]
    pub ceiling_db: f32,
    /// Display inertia: 0 = every refresh lands instantly, 0.9 = slow.
    pub smoothing: f32,
    /// Spectral tilt, in the convention analyzers use: the reference
    /// slope in dB/octave that displays as FLAT, one of [`TILT_STEPS`]
    /// (0, -1.5 .. -6). 0 draws raw power; -3 makes pink noise read
    /// flat; -4.5 flattens typical musical material. The display lifts
    /// treble by the magnitude, pivoting at 1 kHz — there is no
    /// bass-emphasizing direction, matching convention.
    ///
    /// `default_tilt`, not `default`, so a blob saved before the field
    /// existed loads with the slope a fresh install gets rather than the
    /// raw-power 0 a bare f32 default would hand it.
    #[serde(default = "default_tilt")]
    pub tilt: f32,
    /// Axis gridline labeling.
    #[serde(default = "default_labels")]
    pub labels: SpectrumLabels,
    /// Overall size of the pane's own markings — the gridline labels above and
    /// the pitch readout that follows the pointer — as a multiple of their
    /// built-in sizes.
    ///
    /// Fixed against the zoom, unlike the note names: a marking says what the
    /// axis is, and the axis is the one thing on the pane that does not change
    /// size when the range is zoomed.
    #[serde(default = "default_one")]
    pub marking_scale: f32,
    /// Strength of the light edge drawn along the spectrum's profile and
    /// around each note ribbon, 0 = none. On a roll note it is the whole of
    /// the rim — how bright the keyline is, and whether it is drawn at all.
    /// See `panes::roll::keyline`.
    #[serde(default = "default_keyline")]
    pub keyline: f32,
    /// Displayed pitch range, as (fractional) MIDI note numbers. The
    /// analyzer always covers `SPECTRUM_MIN_MIDI..=SPECTRUM_MAX_MIDI`
    /// (~16 Hz to ~16.7 kHz); this only zooms the view.
    ///
    /// MIDI rather than Hz because the axis is linear in MIDI note, which
    /// makes this both the number the pane wants and — since a semitone is a
    /// constant frequency RATIO — a logarithmic frequency scale. The control
    /// drags it linearly and reads it out in Hz.
    #[serde(default = "default_low_midi")]
    pub low_midi: f32,
    #[serde(default = "default_high_midi")]
    pub high_midi: f32,
    /// Migration only. The range used to be a pair of Bitwig octave numbers,
    /// so it could only land on C boundaries; `migrate_legacy` folds an older
    /// blob's pair into `low_midi`/`high_midi` and nothing writes them again.
    ///
    /// A sentinel rather than `Option`, because the old blobs wrote a bare
    /// `low_octave: 1` and RON only reads that into an `Option` if it is
    /// spelled `Some(1)` — the field would never populate, and the failed
    /// parse would take the whole persist down with it.
    #[serde(default = "no_legacy_octave", skip_serializing, alias = "low_octave")]
    legacy_low_octave: i32,
    #[serde(default = "no_legacy_octave", skip_serializing, alias = "high_octave")]
    legacy_high_octave: i32,

    // ---- Piano roll -------------------------------------------------
    // The played-note timeline (lattice-core's NoteRoll) drawn over the
    // same pitch axis, occupying the far end of the depth axis. Time runs
    // away from the spectrum: a note leaving the roll's near edge meets
    // the spectrum peak it is making.
    /// Draw the incoming MIDI's history at all.
    #[serde(default = "default_true")]
    pub show_roll: bool,
    /// Share of the pane's depth given to the roll (the rest is the
    /// spectrum). 0 hides it; 1 gives the whole pane to the roll. Set by
    /// dragging the divider in the Spectral pane itself
    /// (`panes::spectral::drag_split`) — there is no bar for it.
    #[serde(default = "default_roll_fraction")]
    pub roll_fraction: f32,
    /// Seconds of history the roll's depth spans.
    #[serde(default = "default_roll_seconds")]
    pub roll_seconds: f32,
    /// Note ribbon width, in semitones of the pitch axis. This IS the note's
    /// painted width — a note is a solid rectangle of its own color, with
    /// nothing straddling its boundary.
    #[serde(default = "default_roll_thickness")]
    pub roll_thickness: f32,
    /// Points shaved off a released note's tail, so repeated notes at one
    /// pitch stay separate instead of merging into one bar.
    ///
    /// The tail only: a held note still reaches the now-line, and no onset
    /// ever moves off the moment it was played.
    #[serde(default = "default_roll_color")]
    pub roll_color: RollColor,
    /// Write each note's name over its ribbon, at the moment it was struck —
    /// see [`panes::names`]. `default_true`, not `default`, or a state blob
    /// saved before this field existed would load with them off, contradicting
    /// the struct's own default, which is what a fresh install gets.
    #[serde(default = "default_true")]
    pub note_names: bool,
    /// Overall size of those names, as a multiple of their built-in size.
    ///
    /// Rides on top of the pitch zoom, which already grows a name as the range
    /// narrows so that it keeps its footing on the ribbon it is written on —
    /// see `panes::spectral::name_zoom`. This says how big it is at the zoom
    /// you are at.
    #[serde(default = "default_one")]
    pub note_name_scale: f32,

    // ---- Spectrogram ------------------------------------------------
    // A frequency-vs-time heatmap of the analyzed audio, drawn in the
    // roll's depth region on the roll's own time axis — so each column of
    // spectral energy lines up with the notes that made it.
    /// Draw the spectrogram heatmap (over the roll's time window).
    /// `default_true`, not `default`, or every state blob saved before this
    /// field existed loads with the spectrogram off — contradicting the
    /// struct's own default, which is what a fresh install gets. A blob that
    /// really did turn it off carries `false` and still round-trips.
    #[serde(default = "default_true")]
    pub show_spectrogram: bool,
    /// The heatmap's color ramp.
    #[serde(default)]
    pub spectrogram_color: SpectrogramColor,
    /// Overall spectrogram opacity, so it can sit under the notes without
    /// swamping them. (For the heatmap alone, turn the note ribbons off with
    /// `show_roll`.)
    #[serde(default = "default_spectrogram_opacity")]
    pub spectrogram_opacity: f32,
    /// Give the heatmap its own dB window instead of sharing the curve's.
    ///
    /// Off — the default, and what the heatmap always did — means it reads
    /// `floor_db`/`ceiling_db` like the curve. They answer different
    /// questions, though: the curve wants a range that keeps peaks on the
    /// pane, the heatmap one that separates quiet detail from the background,
    /// and those rarely coincide. `serde(default)` is false, so an existing
    /// blob keeps the shared behaviour it was saved with.
    #[serde(default)]
    pub spectrogram_own_range: bool,
    /// The heatmap's own dB window, used only while `spectrogram_own_range`.
    #[serde(default = "default_spectrogram_floor_db")]
    pub spectrogram_floor_db: f32,
    #[serde(default = "default_ceiling_db")]
    pub spectrogram_ceiling_db: f32,
    /// Contrast curve on the heatmap's 0..1 level: 1 is linear, below 1 lifts
    /// quiet detail toward the bright end, above 1 pushes it into the dark.
    ///
    /// Separate from the dB window on purpose. Moving the floor DISCARDS
    /// everything under it; gamma keeps the whole range and only changes how
    /// it is spread, so background hiss can be pushed down without losing the
    /// quiet partials just above it.
    #[serde(default = "default_one")]
    pub spectrogram_gamma: f32,
}

fn default_spectrogram_floor_db() -> f32 {
    -60.0
}

fn default_one() -> f32 {
    1.0
}

fn default_spectrogram_opacity() -> f32 {
    0.85
}

/// Enough of an edge to hold a shape against a bright spectrogram cell,
/// little enough that it doesn't read as a second outline of its own.
fn default_keyline() -> f32 {
    0.3
}

/// The default pitch range is the analyzer's whole axis — the zoom starts
/// showing everything there is.
fn default_low_midi() -> f32 {
    lattice_core::spectrum::SPECTRUM_MIN_MIDI
}

fn default_high_midi() -> f32 {
    lattice_core::spectrum::SPECTRUM_MAX_MIDI
}

/// "This blob had no octave-numbered range", out of the domain the old
/// control could produce (-1..=9).
fn no_legacy_octave() -> i32 {
    i32::MIN
}

impl SpectrumConfig {
    /// Fold an older blob's octave-numbered pitch range into the continuous
    /// one. A pre-Hz blob carries no `low_midi`, so serde would hand it the
    /// full-axis default and silently throw away the zoom the user had set.
    fn migrate_legacy(&mut self) {
        let (low, high) = (self.legacy_low_octave, self.legacy_high_octave);
        (self.legacy_low_octave, self.legacy_high_octave) =
            (no_legacy_octave(), no_legacy_octave());
        if low != no_legacy_octave() && high != no_legacy_octave() {
            let midi = |octave: i32| lattice_core::notes::octave_start_midi(octave) as f32;
            self.low_midi = midi(low);
            self.high_midi = midi(high);
        }
        // Then fit whatever came out to the axis the analyzer actually covers.
        // Every source of this pair can be off it: an octave pair reaches C-1
        // and C9, a blob written while the axis ran 16 Hz to 16.7 kHz carries
        // its old ends, and a hand-edited one can say anything. A range past
        // the axis draws a band with no buckets behind it; an inverted one
        // divides by zero in PitchScale.
        let (floor, ceil) = (default_low_midi(), default_high_midi());
        self.low_midi = self.low_midi.clamp(floor, ceil - PITCH_RANGE_MIN_SPAN);
        self.high_midi = self.high_midi.clamp(self.low_midi + PITCH_RANGE_MIN_SPAN, ceil);
    }
}

/// Closest the two ends of the pitch range may come: one octave, which is
/// what the octave-pair control guaranteed before it went continuous.
pub(crate) const PITCH_RANGE_MIN_SPAN: f32 = 12.0;

/// How far the roll's time span may be taken, in seconds. Named because two
/// controls now set it — the Analyzer tab's Span bar and the drag across the
/// picture — and a gesture that clamped to its own idea of the limits would
/// push the bar past its own ends. The maximum is also what
/// [`AudioSpectrum::HISTORY_MAX_SECONDS`] is sized to reach back to, so the
/// heatmap can fill the widest span the axis offers.
pub(crate) const ROLL_SECONDS_MIN: f32 = 1.0;
pub(crate) const ROLL_SECONDS_MAX: f32 = 600.0;

/// The level range's domain, in dB. The top is a full-scale sine, the
/// loudest thing a bucket can hold; the bottom is well under any noise
/// floor worth looking at.
pub(crate) const LEVEL_MIN_DB: f32 = -100.0;
pub(crate) const LEVEL_MAX_DB: f32 = 0.0;

/// Closest the two ends of the level range may come. A window narrower than
/// this is all edge and no picture — and, unclamped, a collapsed one divides
/// by zero in `loudness` and paints the NaN geometry egui panics on.
pub(crate) const LEVEL_RANGE_MIN_SPAN: f32 = 12.0;

/// A full-scale sine reads as full height, which is what the pane did before
/// the ceiling was adjustable at all.
fn default_ceiling_db() -> f32 {
    LEVEL_MAX_DB
}

fn default_true() -> bool {
    true
}

fn default_labels() -> SpectrumLabels {
    SpectrumLabels::Notes
}

fn default_roll_fraction() -> f32 {
    0.55
}

fn default_roll_seconds() -> f32 {
    12.0
}

/// Thin: a note is a line through the spectrogram at its own pitch, not a
/// slab over it. At 0.3 semitones a semitone of pitch axis still separates
/// two neighbouring keys, which is what makes the roll readable when the
/// pitch range is zoomed out over the whole spectrum.
fn default_roll_thickness() -> f32 {
    0.3
}

/// A hairline of background between repeats — enough to read two taps as
/// two, little enough that a note's length is still its length.
fn default_roll_color() -> RollColor {
    RollColor::Channel
}

/// The tilt settings offered, per analyzer convention (-1.5 dB/oct
/// increments; see [`SpectrumConfig::tilt`]).
pub const TILT_STEPS: [f32; 5] = [0.0, -1.5, -3.0, -4.5, -6.0];

/// The slope that flattens typical musical material — what the analyzer is
/// looked at through nearly all the time, so it is where it starts. Raw
/// power (0) buries everything above a couple of kHz.
fn default_tilt() -> f32 {
    -4.5
}

impl Default for SpectrumConfig {
    fn default() -> Self {
        SpectrumConfig {
            orientation: SpectralOrientation::Horizontal,
            window: SpectrumWindow::Balanced,
            floor_db: -60.0,
            ceiling_db: default_ceiling_db(),
            smoothing: 0.55,
            tilt: default_tilt(),
            labels: SpectrumLabels::Notes,
            marking_scale: default_one(),
            keyline: default_keyline(),
            low_midi: default_low_midi(),
            high_midi: default_high_midi(),
            legacy_low_octave: no_legacy_octave(),
            legacy_high_octave: no_legacy_octave(),
            show_roll: true,
            roll_fraction: default_roll_fraction(),
            roll_seconds: default_roll_seconds(),
            roll_thickness: default_roll_thickness(),
            roll_color: default_roll_color(),
            note_names: true,
            note_name_scale: default_one(),
            show_spectrogram: true,
            spectrogram_color: SpectrogramColor::default(),
            spectrogram_opacity: default_spectrogram_opacity(),
            spectrogram_own_range: false,
            spectrogram_floor_db: default_spectrogram_floor_db(),
            spectrogram_ceiling_db: default_ceiling_db(),
            spectrogram_gamma: default_one(),
        }
    }
}

/// One power value per pitch-spectrum bucket, the array the analyzer fills
/// and the pane draws. See [`lattice_core::spectrum::SPECTRUM_BINS`].
type SpectrumBuckets = [f32; lattice_core::spectrum::SPECTRUM_BINS];

/// Audio-derived pitch spectrum shown in the Spectral pane. The shell
/// feeds mono samples every frame from wherever its audio comes from
/// (plugin: input bus via a ring buffer; standalone: the mock synth); the
/// pane asks for a display refresh when it draws. Runtime-only.
pub struct AudioSpectrum {
    /// One analyzer per input channel, combined in the power domain — see
    /// [`ChannelBank`](lattice_core::spectrum::ChannelBank).
    analyzer: lattice_core::spectrum::ChannelBank,
    /// Smoothed display buckets (power; the pane maps to height).
    display: SpectrumBuckets,
    /// FRAMES pushed since this analyzer was made, and the count at which the
    /// next FFT falls due. The column grid is a function of these two and
    /// nothing else — see [`push_samples`](AudioSpectrum::push_samples).
    ///
    /// Frames, not samples: a stereo stream carries two samples per instant, and
    /// a hop is an amount of TIME. Counting samples would halve the hop the
    /// moment the input went stereo.
    frames_seen: u64,
    next_hop: u64,
    /// Shell time of sample 0: what turns a sample count into a timestamp.
    ///
    /// Smoothed rather than taken fresh, on exactly the reasoning behind the
    /// plugin's own `ClockMapper` (and with its constants). A shell drains its
    /// audio ring on frame boundaries but the ring fills in audio BLOCKS, so the
    /// number of samples a frame brings swings by a block either way while `now`
    /// advances by a frame — several ms of wobble in what any one batch implies
    /// about where sample 0 was. Stamping columns from a fresh estimate would
    /// pass that wobble straight into their spacing, which is what the sample
    /// grid exists to remove: at an 8 ms hop, +-5 ms of it is enough to leave a
    /// 12.8 ms slab empty. Smoothed, the grid is exactly even and still follows
    /// the shell clock.
    anchor: Option<f64>,
    /// When samples last arrived; the curve hides once the source stops
    /// (closed input bus, switched-off synth) rather than freezing.
    last_samples: Option<f64>,
    /// Timestamped raw spectra, one per FFT, for the spectrogram — oldest
    /// first. Raw (unsmoothed) so time isn't blurred across columns.
    /// Bounded by age and, by construction, by memory: see
    /// [`SpectrumHistory`](lattice_core::SpectrumHistory) and
    /// [`AudioSpectrum::push_history`].
    history: SpectrumHistory,
    /// One per drawing surface — index 0 the docked Spectral pane (and the
    /// offline render), index 1 the Video pane's preview — so two live
    /// spectrograms in one frame don't overwrite each other's work.
    spectrogram: [SpectrogramSurface; 2],
}

/// One drawing surface's heatmap: the uploaded texture, and the three caches
/// that describe it.
///
/// They are held together because they describe ONE texture between them and
/// can only be trusted as a set — the cache validates the texture, the ring
/// records which slabs its columns hold, and either one kept without it is a
/// statement about pixels that no longer exist. As four parallel arrays they
/// were four chances to update three.
///
/// Runtime-only, never persisted, and each rebuilds itself from
/// [`AudioSpectrum::history`] when dropped, so `None` is always a safe state.
#[derive(Default)]
pub(crate) struct SpectrogramSurface {
    /// The heatmap's pixels, sampled with bilinear filtering so it reads as a
    /// smooth image rather than a mesh of interpolated triangles. Created
    /// lazily on first draw.
    tex: Option<egui::TextureHandle>,
    /// Validates [`Self::tex`]: while the key still matches, the whole build
    /// (aggregate -> colour -> upload) is skipped and only the scrolling quad
    /// is redrawn. See [`SpectrogramKey`].
    cache: Option<SpectrogramCache>,
    /// Live-only incremental aggregator: keeps the slab grid across frames so a
    /// rebuild folds only new columns instead of rescanning the whole window.
    /// See `panes::spectrogram::SpectrogramAgg`.
    agg: Option<crate::panes::spectrogram::SpectrogramAgg>,
    /// Which slabs [`Self::tex`]'s columns currently hold, so a new one can be
    /// written on its own instead of repainting the whole heatmap. `None`
    /// whenever the texture was built the full-width way, which now means only
    /// the offline whole-song render.
    ring: Option<crate::panes::spectrogram::SpectrogramRing>,
    /// Times the ring has been restarted — re-blanked and every column
    /// repainted — by [`Restart`](crate::panes::spectrogram::Restart) reason.
    /// Kept HERE rather than on the ring, which does not survive its own
    /// restart. Read out by the performance overlay beside the aggregator's
    /// rebuild count; see
    /// [`SpectrogramAgg::rebuilds`](crate::panes::spectrogram::SpectrogramAgg::rebuilds).
    restarts: [u32; crate::panes::spectrogram::Restart::COUNT],
}

impl SpectrogramSurface {
    /// Drop everything that describes the CURRENT context's texture, so the
    /// next draw uploads a fresh one.
    ///
    /// Three of the four go, and which three is the whole point of naming this:
    /// the cache validates the released texture, and the ring records which
    /// slabs its columns held — kept across a context change, the ring would
    /// write one fresh column into a brand new texture and call the other
    /// thousands valid, a heatmap of uninitialized memory. The aggregator is
    /// derived from the STORE rather than from the texture, so it survives; it
    /// is the one piece a new context does not invalidate.
    fn release_texture(&mut self) {
        self.tex = None;
        self.cache = None;
        self.ring = None;
    }
}

/// One column of the spectrogram, and the age-tiered store they live in — both
/// pure data, so they live in the core crate. See
/// [`lattice_core::spectrogram`] for why a column is bytes of dB rather than
/// floats of power, and why old ones are merged.
pub use lattice_core::spectrogram::{SpectrogramColumn, SpectrumHistory};

/// The inputs the built spectrogram image depends on. Equal keys mean the
/// uploaded texture is still valid, so `draw_spectrogram` skips the rebuild
/// (aggregate -> smooth -> color -> texture upload) and only redraws the quad,
/// which scrolls with `now` every frame regardless. The FFT refreshes at
/// ~20 Hz while the pane redraws at frame rate, so most frames hit.
///
/// Staleness-safe by construction — every way the image can change moves a
/// field: a fresh column moves `newest_bits` (even in a saturated ring, where
/// the count holds), the oldest column scrolling out of the window moves
/// `first`, a resize/zoom moves the layout fields, and a palette, dB window or
/// contrast change moves `cfg`/`frame`. Floats compare by bit pattern so
/// equality is exact and free of NaN quirks.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct SpectrogramKey {
    /// Everything that decides a column's PIXELS, shared with the ring — which
    /// asks the same question about the same columns, one scroll at a time. See
    /// [`ColumnStyle`](crate::panes::spectrogram::ColumnStyle).
    style: crate::panes::spectrogram::ColumnStyle,
    /// ...and which columns are in the WINDOW, which is what a scroll changes
    /// and the ring survives.
    first: usize,
    cols_len: usize,
    newest_bits: u64,
    whole: bool,
}

/// A validated spectrogram build: its [`SpectrogramKey`] plus the scalars the
/// quad needs, so a cache hit can place the already-uploaded texture without
/// re-running the pixel pipeline. The texture itself stays in
/// [`SpectrogramSurface::tex`].
pub(crate) struct SpectrogramCache {
    key: SpectrogramKey,
    /// Where the build put its slabs in the texture, kept exactly as it
    /// computed them — a hit hands this straight back to the quad.
    layout: crate::panes::spectrogram::TexLayout,
}

impl SpectrogramKey {
    /// Pack the image's inputs into a key: the style every column shares, plus
    /// which columns this build drew. `newest` is stored as a bit pattern so
    /// equality is exact.
    pub(crate) fn new(
        style: crate::panes::spectrogram::ColumnStyle,
        first: usize,
        cols_len: usize,
        newest: f64,
        whole: bool,
    ) -> Self {
        SpectrogramKey { style, first, cols_len, newest_bits: newest.to_bits(), whole }
    }

    /// What the ring compares — the part of this key a scroll leaves alone.
    pub(crate) fn style(&self) -> &crate::panes::spectrogram::ColumnStyle {
        &self.style
    }
}

impl SpectrogramCache {
    pub(crate) fn new(
        key: SpectrogramKey,
        layout: crate::panes::spectrogram::TexLayout,
    ) -> Self {
        SpectrogramCache { key, layout }
    }

    /// Whether a freshly computed key matches this cached build's — i.e. the
    /// uploaded texture is still the right one to draw.
    pub(crate) fn matches(&self, key: &SpectrogramKey) -> bool {
        &self.key == key
    }

    /// The scalars the scrolling quad needs.
    pub(crate) fn geometry(&self) -> crate::panes::spectrogram::TexLayout {
        self.layout
    }
}

/// The whole take's spectrogram, precomputed for the offline renderer's
/// playhead mode: every column of the full audio, laid out statically along
/// the time axis with a playhead at `now`, instead of the live scrolling
/// window. `Some` only in the offline renderer — the live ring
/// ([`AudioSpectrum::history`]) is bounded and cannot hold a whole song.
/// Runtime-only, never persisted (like [`SharedState::learn_active`]).
pub struct WholeSong {
    /// Take time at the near edge: the playhead sits here at the render's
    /// start.
    pub start: f64,
    /// Seconds spanned across the depth axis — the render's duration.
    pub span: f64,
    /// Every spectrogram column, oldest first.
    pub columns: Vec<SpectrogramColumn>,
    /// The whole take's notes, laid out from the start. The live tracker only
    /// holds notes replayed up to `now`, so the roll would otherwise fill in as
    /// the playhead reached them; the render wants the whole piece at once. Set
    /// by the offline renderer; empty in the spectrogram-only bounce preview.
    pub roll: lattice_core::NoteRoll,
}

impl WholeSong {
    /// Analyze the entire `samples` buffer, one raw column per hop,
    /// `time`-stamped in take time (`time_origin` is the take time of sample 0).
    /// Raw, exactly like the live store: the heatmap reads what was measured,
    /// and blurring one column into the next is not something it has done since
    /// temporal smoothing was dropped.
    ///
    /// The hop is the live one, EXCEPT that a long take stretches it: this build
    /// spans the whole song rather than a scrolling window, so its time axis is
    /// cut into `span / WHOLE_SONG_SLAB_CAP` slabs at best, and columns finer
    /// than that are aggregated away by the MAX the moment they are drawn. A
    /// three-minute take at the live rate would hold 22 500 columns (86 MB) to
    /// display 4096 of them. Scaling the hop to the slab keeps the same
    /// [`COLUMNS_PER_SLAB`](crate::panes::spectrogram::COLUMNS_PER_SLAB) margin
    /// the live path has — every slab still gets a column, none goes empty — for
    /// a quarter of the memory.
    ///
    /// `samples` is INTERLEAVED, `channels` per frame, and the channels are
    /// combined exactly as the live path combines them — same
    /// [`ChannelBank`](lattice_core::spectrum::ChannelBank), same power sum. That
    /// is the point of sharing the type rather than repeating the arithmetic: a
    /// render that summed its channels differently from the pane would differ
    /// from the look that was dialed in, and only for stereo-wide material, which
    /// is the hardest kind of difference to attribute.
    ///
    /// Pure: `(samples, channels, rate, config)` in, columns out, no clock or
    /// RNG, so a render built on it stays byte-identical between runs.
    pub fn precompute(
        samples: &[f32],
        channels: usize,
        sample_rate: f32,
        time_origin: f64,
        start: f64,
        span: f64,
        config: &SpectrumConfig,
    ) -> WholeSong {
        let mut analyzer = lattice_core::spectrum::ChannelBank::new(sample_rate, channels);
        analyzer.set_fft_size(config.window.samples());
        let channels = analyzer.channels();
        let sr = (sample_rate as f64).max(1.0);
        let hop = (span / crate::panes::spectrogram::WHOLE_SONG_SLAB_CAP as f64
            / crate::panes::spectrogram::COLUMNS_PER_SLAB)
            .max(AudioSpectrum::FFT_INTERVAL);
        let total = samples.len() / channels; // frames
        let mut columns = Vec::new();
        // Feed the buffer in one-hop chunks; once the window has filled every
        // hop yields a column, exactly as the live `push_samples` loop does.
        let (mut fed, mut k) = (0usize, 1usize);
        loop {
            let end = ((k as f64 * hop * sr).round() as usize).min(total);
            if end > fed {
                analyzer.push_frames(&samples[fed * channels..end * channels]);
                fed = end;
            }
            if let Some(power) = analyzer.power_sum() {
                // The middle of the window this spectrum measured, exactly as
                // the live path stamps it — `end` is where that window ENDS.
                // The take's notes are laid out from their own timestamps, so a
                // render is where a half-window offset would show up most: the
                // ribbons are placed perfectly and the heatmap would not be.
                let center = time_origin + end as f64 / sr - analyzer.window_center_offset();
                columns.push(SpectrogramColumn::from_power(center, &power));
            }
            if end >= total {
                break;
            }
            k += 1;
        }
        // The roll is filled in separately by the renderer (it needs the notes,
        // not the audio); the bounce preview leaves it empty.
        WholeSong { start, span, columns, roll: lattice_core::NoteRoll::default() }
    }
}

impl Default for AudioSpectrum {
    fn default() -> Self {
        AudioSpectrum {
            analyzer: lattice_core::spectrum::ChannelBank::new(48_000.0, 1),
            display: [0.0; lattice_core::spectrum::SPECTRUM_BINS],
            frames_seen: 0,
            next_hop: 0,
            anchor: None,
            last_samples: None,
            history: SpectrumHistory::default(),
            spectrogram: [SpectrogramSurface::default(), SpectrogramSurface::default()],
        }
    }
}

impl AudioSpectrum {
    /// Forget the spectrogram textures, so the next draw uploads fresh ones
    /// into whatever context is current. See
    /// [`SharedState::release_context_resources`].
    fn release_textures(&mut self) {
        for surface in &mut self.spectrogram {
            surface.release_texture();
        }
    }

    /// Seconds of AUDIO between FFTs (125 columns a second), measured in
    /// samples rather than on the shell clock — see
    /// [`push_samples`](Self::push_samples).
    ///
    /// The heatmap does not repaint itself for every column (see
    /// `write_ring`), so the cost of a column is O(pitch pixels) rather than
    /// O(pitch pixels x slabs) and the rate buys smoothness at the newest
    /// edge almost for free. It costs no REACH either: the store coarsens
    /// with age (see [`SpectrumHistory`]), so the rate sets the resolution of
    /// the recent stretch and barely touches how far back the heatmap goes.
    ///
    /// At 8 ms this is a picture setting and not an analysis one: the window
    /// is untouched, so what a column RESOLVES is unchanged, and overlapping
    /// that same window more finely just draws the time axis at 2.5x the
    /// resolution 20 ms reaches (via
    /// [`live_slab`](crate::panes::spectrogram::live_slab), whose ladder is
    /// rungs of THIS interval, so the picture's grid tracks it).
    /// It costs 0.37 ms of FFT per column — 4.7% of a core, against 1.9% at
    /// 20 ms — and one more [`SpectrumHistory`] tier to hold the same reach.
    pub(crate) const FFT_INTERVAL: f64 = 0.008;
    /// How long after the last samples the curve keeps drawing.
    const HOLD_SECONDS: f64 = 0.5;
    /// Per-batch gain and restart threshold for the sample-count anchor (see
    /// the field). The plugin's `ClockMapper` solves the same problem for MIDI
    /// event times with the same two numbers.
    const ANCHOR_SMOOTHING: f64 = 0.05;
    const ANCHOR_SNAP: f64 = 1.0;

    /// Feed mono samples from the shell, analyzing one spectrum per
    /// [`FFT_INTERVAL`](Self::FFT_INTERVAL) of audio in them. `now` is the shell
    /// clock also passed to [`root_ui`], and dates the NEWEST sample of the
    /// batch — which is what a shell draining its audio ring at frame time
    /// means by it.
    ///
    /// The FFT runs here, on a grid of sample counts, rather than in
    /// [`display`](Self::display) on a grid of frames. That is the whole point:
    /// the old gate (`now - last_fft >= FFT_INTERVAL`, evaluated once per UI
    /// pass) could only fire ON a frame boundary, so a 20 ms interval on a 60 Hz
    /// display fired every 33.3 ms — SLOWER than the 32 ms slabs the heatmap was
    /// cutting the window into. Slabs went empty and were filled by duplicating
    /// their neighbour (`JITTER_SLABS`), so a held column scrolled past about
    /// once a second; the columns that did arrive sat at a phase inside their
    /// slab that drifted with the frame clock; and capping the frame rate
    /// coarsened the picture in proportion. Counting samples makes the column
    /// grid exact, evenly spaced, and independent of how often — or how evenly —
    /// the shell draws.
    ///
    /// The smoothing and peak-hold decay of the CURVE moved here with it, for
    /// the same reason: both are per-column, so leaving them on the frame clock
    /// would have made their time constants frame-rate dependent.
    ///
    /// One call therefore costs as many FFTs as the audio it is handed contains
    /// hops, where the old one cost exactly one. Normally that is a frame's
    /// worth (two or three), and the worst case is a batch as large as the
    /// shell's audio ring — 1.37 s in the plugin, 170 columns, ~60 ms — reachable
    /// only by an editor that has been closed or stalled for that long, which
    /// then gets its heatmap back-filled with audio that really did happen.
    pub fn push_samples(
        &mut self,
        samples: &[f32],
        channels: usize,
        sample_rate: f32,
        now: f64,
        config: &SpectrumConfig,
    ) {
        if samples.is_empty() {
            return;
        }
        // Any of the three empties the analyzers' rings, so nothing comes out
        // until they have refilled. The hop grid keeps its phase across that gap
        // rather than restarting on it.
        self.analyzer.set_channels(channels);
        self.analyzer.set_fft_size(config.window.samples());
        self.analyzer.set_sample_rate(sample_rate);
        self.last_samples = Some(now);

        // FRAMES throughout: `samples` is interleaved, and a hop is an amount of
        // time. A partial frame at the end is left for the next batch, so the
        // de-interleaving in `push_frames` can never slip a channel.
        let channels = self.analyzer.channels();
        let batch = samples.len() / channels;
        if batch == 0 {
            return;
        }
        let sr = f64::from(sample_rate.max(1.0));
        let hop = ((Self::FFT_INTERVAL * sr).round() as u64).max(1);
        // Columns fall on multiples of `hop` frames from the start of the
        // stream. Left at zero the first boundary would be frame 1 and every one
        // after it a frame early, which is harmless but makes the grid
        // impossible to state (or to test) in whole hops.
        if self.next_hop == 0 {
            self.next_hop = hop;
        }

        // Re-anchor the frame count on the shell clock: the last frame of this
        // batch is at `now`. Smoothed, so the columns below are evenly spaced;
        // snapped when the estimate moves further than any wobble could, which
        // is a stream that restarted — a transport jump, a sample-rate change
        // (the count is re-divided by a different rate, so the anchor moves by
        // minutes), or the first batch after the pane was switched on.
        let total = self.frames_seen + batch as u64;
        let candidate = now - total.saturating_sub(1) as f64 / sr;
        let anchor = match self.anchor {
            Some(prev) if (candidate - prev).abs() <= Self::ANCHOR_SNAP => {
                prev + (candidate - prev) * Self::ANCHOR_SMOOTHING
            }
            _ => candidate,
        };
        self.anchor = Some(anchor);

        let mut fed = 0usize; // frames
        while fed < batch {
            // Feed exactly up to the next hop boundary, so a spectrum is taken
            // at every multiple of `hop` frames and nowhere else. `max(1)`
            // keeps the loop moving if a sample-rate change ever leaves the
            // boundary behind us; the next line puts the grid back on its feet.
            let want = self.next_hop.saturating_sub(self.frames_seen).max(1) as usize;
            let take = want.min(batch - fed);
            self.analyzer.push_frames(&samples[fed * channels..(fed + take) * channels]);
            self.frames_seen += take as u64;
            fed += take;
            if self.frames_seen < self.next_hop {
                break; // The batch ran out before the boundary.
            }
            self.next_hop = self.frames_seen + hop;
            let Some(fresh) = self.analyzer.power_sum() else { continue };

            let alpha = 1.0 - config.smoothing.clamp(0.0, 0.95);
            for (shown, new) in self.display.iter_mut().zip(&fresh) {
                *shown += (new - *shown) * alpha;
            }
            // Keep the RAW spectrum for the spectrogram (the smoothed
            // `display` would smear one column into the next). Retention is
            // span-INDEPENDENT (see `push_history`): shrinking the span and
            // widening it again must not lose the history in between.
            //
            // Stamped at the middle of the window it measured, not at the
            // boundary itself — see `window_center_offset`. This is what lets a
            // ridge sit under the note ribbon that made it, which is the entire
            // point of drawing the two on one time axis. The boundary is where
            // the newest frame fed so far sits on the anchored grid, so
            // consecutive columns are exactly `hop` frames apart.
            let boundary = anchor + self.frames_seen.saturating_sub(1) as f64 / sr;
            self.push_history(boundary - self.analyzer.window_center_offset(), &fresh);
        }
    }

    /// The curve to draw, or None while no audio is flowing. The levels are
    /// maintained per column by [`push_samples`](Self::push_samples); this
    /// only decides whether they are still live.
    pub fn display(&self, now: f64) -> Option<&SpectrumBuckets> {
        self.last_samples
            .is_some_and(|t| now - t <= Self::HOLD_SECONDS)
            .then_some(&self.display)
    }

    /// The most history ever kept, span-independent: the longest span the roll
    /// offers (`roll_seconds` max, 600 s) plus 10 s of headroom so a column is
    /// ready the instant the window reaches back to it. Nothing older is
    /// retained even at the maximum span.
    ///
    /// This is the ONLY thing that decides reach — no memory backstop binds
    /// first. Storing a bucket as a byte of dB and coarsening old columns
    /// (see [`SpectrumHistory`]) puts the full span at about 30 MB, so the
    /// cap can simply be the span. Keeping every column at full rate forever
    /// would instead cost 160 MB to reach only ~3.5 minutes at 50 Hz, drawing
    /// a heatmap over the recent stretch and bare roll beyond it.
    ///
    /// Raising it is cheap and sub-linear: another
    /// [`SpectrumHistory::COARSE_COLUMNS`] (~4 MB) doubles the reach. The unit
    /// test `spectrum_history_reaches_the_retention_cap` is what keeps the
    /// structure sized for whatever this says.
    const HISTORY_MAX_SECONDS: f64 = 610.0;

    /// Append one raw spectrum to the store, trimming anything past
    /// `HISTORY_MAX_SECONDS` of age. The store bounds its own memory (older
    /// columns merge, and its last tier's overflow is dropped), so there is no
    /// separate column-count backstop to keep in step with the FFT rate.
    ///
    /// Retention is deliberately NOT keyed to the current span: trimming to the
    /// live `roll_seconds` meant shrinking the span popped columns off the
    /// front, and widening it again could never bring them back — the span
    /// control silently erased spectrogram history. The heatmap simply reads
    /// back as far as the span asks; anything it isn't showing yet stays in the
    /// store until it ages past the cap.
    fn push_history(&mut self, now: f64, power: &SpectrumBuckets) {
        self.history.push(SpectrogramColumn::from_power(now, power));
        self.history.trim_older_than(now - Self::HISTORY_MAX_SECONDS);
    }

    /// The spectrogram columns, oldest first. Empty until audio has flowed.
    pub fn history(&self) -> &SpectrumHistory {
        &self.history
    }

    /// Fallbacks taken across both surfaces since the plugin was opened: full
    /// re-aggregations of the window, and ring restarts.
    ///
    /// Both are CORRECT and both are expensive, which is the whole problem —
    /// they draw the right picture at many times the cost, so nothing on screen
    /// distinguishes a working cache from one that has quietly stopped. The
    /// overlay turns them into a rate, where "climbing" is the entire diagnosis.
    pub(crate) fn spectrogram_fallbacks(
        &self,
    ) -> (u32, [u32; crate::panes::spectrogram::Restart::COUNT]) {
        self.spectrogram.iter().fold(
            (0, [0; crate::panes::spectrogram::Restart::COUNT]),
            |(rebuilds, mut restarts), s| {
                for (total, surface) in restarts.iter_mut().zip(s.restarts) {
                    *total += surface;
                }
                (rebuilds + s.agg.as_ref().map_or(0, |a| a.rebuilds()), restarts)
            },
        )
    }

    /// Forget the spectrogram history (paired with clearing the roll).
    pub fn clear_history(&mut self) {
        self.history.clear();
    }

    /// Whether audio has arrived within the hold window — i.e. the spectrum is
    /// still live. Drives continuous repaint so the curve and spectrogram stay
    /// smooth even when no MIDI is animating the frame. Reads true only while
    /// samples are actually arriving (the shell pushes them when the spectrum is
    /// shown), so it idles cleanly once audio stops.
    /// How far behind `now` the newest column sits even when nothing is wrong:
    /// half the analysis window, because that is where a spectrum belongs on a
    /// time axis (see
    /// [`window_center_offset`](lattice_core::spectrum::SpectrumAnalyzer::window_center_offset)).
    ///
    /// The heatmap's near edge has to allow for this or it reads a perfectly
    /// healthy stream as stale and stops the strip short of the now-line — by
    /// 171 ms on the Precise window, which is a visible gap that widens and
    /// narrows as the window is changed.
    pub fn column_lag(&self) -> f64 {
        self.analyzer.window_center_offset()
    }

    pub fn is_flowing(&self, now: f64) -> bool {
        self.last_samples.is_some_and(|t| now - t <= Self::HOLD_SECONDS)
    }
}

/// Everything the UI reads and mutates each frame. One instance lives in the
/// shell (inside the editor state in the plugin, inside the app in the
/// standalone harness).
pub struct SharedState {
    pub tracker: NoteTracker,
    /// Snapshot of the tuning parameters, refreshed each frame in
    /// [`root_ui`] so core/scene code never touches the param system.
    pub tuning: Tuning,
    pub view: ViewConfig,
    /// Per-frame mirrors of the appearance parameters, refreshed alongside
    /// `tuning` (the param system owns the real values; these are never
    /// persisted).
    pub frame_params: FrameParams,
    pub camera: Camera,
    /// The pitch-class node the pointer is over, if any — shared so *every*
    /// pane can highlight it (lattice glow, tuning pane readout, ...).
    pub hovered: Option<LatticePos>,
    pub console: Console,
    /// Surface format of the shell's swapchain; the lattice render pipeline
    /// must match it.
    pub target_format: TextureFormat,
    /// The color the lattice pane is painted ONTO, which only the sevens
    /// knockout reads (see [`lattice_scene::Scene::background`]). Defaults
    /// to the skin's panel, which is what `egui_dock` fills a tab body with
    /// — right for the plugin and the standalone harness.
    ///
    /// A shell that composes its panes differently MUST set this: the
    /// offline renderer clears to its layout's own background and draws the
    /// panes over that, several shades darker than the panel, and a
    /// knockout clearing to the wrong ground shows up as a disc that is
    /// visibly too light. Exported video is the one place this is hardest
    /// to notice and most expensive to get wrong.
    pub background: glam::Vec4,
    /// While true, tuning params continuously re-learn from the held notes
    /// (v1's learn mode). Runtime-only; never persisted.
    pub learn_active: bool,
    /// Held pitch classes the last learn ran against (change detection).
    last_learned_classes: Option<Vec<PitchClass>>,
    /// User-saved camera angles, applied like the built-in Flat/Isometric
    /// presets (persisted; see the Frame pane).
    pub camera_presets: Vec<CameraPreset>,
    /// Entry buffer for naming a new preset. Runtime-only.
    pub preset_name: String,
    /// Take recording, for offline video rendering. The shell owns the
    /// actual recorder; these three fields are the whole contract.
    /// Runtime-only — a take is a deliberate act, never resumed on load.
    ///
    /// `take_supported` gates the control: shells that cannot record
    /// (or a build without a writer) simply don't show it, rather than
    /// offering a button that does nothing.
    pub take_supported: bool,
    /// Toggled by the Video pane, acted on by the shell.
    pub take_recording: bool,
    /// Whether the transport is actually rolling (capture is happening), as
    /// opposed to armed-and-waiting. Drives the record indicator: a steady dot
    /// while rolling, a breathing one while it waits. Shell-set, runtime-only.
    pub take_rolling: bool,
    /// One-shot: set by the Video pane's "Render now" button, consumed by the
    /// shell to render the last take with the CURRENT settings. Runtime-only.
    pub render_now: bool,
    /// Whether a take has been recorded this session — the shell sets it so the
    /// Video pane can offer "Render now". Runtime-only.
    pub last_take_ready: bool,
    /// Record the input bus alongside the notes, so the render has a
    /// spectrum and a soundtrack without a separate bounce. Persisted
    /// with the render settings rather than the take state, since it is
    /// a preference, not a live flag.
    pub take_audio: bool,
    /// What to do with a take once it is finished (persisted).
    pub render_config: RenderConfig,
    /// Shell-supplied one-liner shown under the toggle: where the file is
    /// going, how many events, or what went wrong.
    pub take_status: String,
    /// Audio-derived spectrum for the Spectral pane. Runtime-only.
    pub spectrum: AudioSpectrum,
    /// The Spectral pane's settings (Analyzer tab; persisted).
    pub spectrum_config: SpectrumConfig,
    /// Offline playhead render: the whole take's spectrogram laid out
    /// statically with a playhead at `now`, instead of the live scrolling
    /// window. `Some` only in the offline renderer. Runtime-only, never
    /// persisted (mirrors `learn_active`).
    pub whole_song: Option<WholeSong>,
    /// Set by the Panel pane's "Reset layout" button; consumed by root_ui
    /// AFTER the frame's DockArea writes the dock back (panes run inside
    /// that pass, so a direct write from one would be overwritten).
    reset_layout: bool,
    dock: DockState<panes::Tab>,
    /// The split fractions the sideways folds are holding onto, so an unfolded
    /// pane comes back the width it was (see [`fold`]).
    folds: fold::Folds,
    /// GPU time of the lattice's passes in milliseconds, as f32 bits, written
    /// by the render callback and read by the performance overlay. 0 means no
    /// reading — the device didn't grant timestamp queries, or none has landed
    /// yet.
    ///
    /// An atomic rather than a return value because the measurement crosses a
    /// boundary the call stack doesn't: it is produced inside egui-wgpu's
    /// paint callback, several frames after the frame that asked for it. Same
    /// shape the plugin already uses to publish its sample rate.
    ///
    /// Runtime-only, never persisted, and never read by the offline renderer —
    /// which also never asks for the feature, so it has no timer to begin with.
    pub(crate) lattice_stats: std::sync::Arc<lattice_render::LatticeStats>,
    /// How many note segments the docked roll handed its paint callback last
    /// frame — the geometry `verts` does NOT see, four vertices at a time
    /// instead of several hundred.
    ///
    /// Reported so the roll's load stays visible while it draws from its own
    /// vertex buffer rather than egui's: without it the overlay would show
    /// the cost vanish with nothing standing in its place, and "is the roll
    /// drawing at all" would have no answer. An atomic for the same reason
    /// `lattice_stats` is one — the roll draws from a `&SharedState`.
    ///
    /// Only the docked pane (surface 0) publishes; the Render preview is a
    /// second roll on screen and reporting its count as THE count would be
    /// wrong, exactly as it is for the preview's lattice.
    pub(crate) roll_notes: std::sync::atomic::AtomicU32,
    /// What the label callback's copy of egui's font atlas holds (see
    /// [`text::AtlasMirror`]). Behind a lock for the same reason the other
    /// per-frame publishing is behind atomics: labels are drawn from a
    /// `&SharedState`. Taken once per flush, uncontended.
    pub(crate) font_atlas: std::sync::Mutex<text::AtlasMirror>,
    /// Milliseconds the shell spent tessellating egui's shapes last frame,
    /// or 0 where the shell doesn't measure it (the standalone's eframe loop
    /// isn't ours to instrument). Set by the shell before `root_ui`.
    ///
    /// Its own field rather than part of the frame's CPU time because it is
    /// not the same work: `ui cpu` covers building the UI, which only APPENDS
    /// shapes, and this covers turning those shapes into triangles afterwards.
    /// A cost can be entirely in one and invisible in the other.
    pub tess_ms: f32,
    /// Milliseconds the GPU spent on egui's own render pass last frame, or 0
    /// where the shell doesn't measure it. Set by the shell before `root_ui`.
    ///
    /// Disjoint from [`Self::gpu_ms`], which brackets only the lattice's own
    /// passes: between them they cover the frame's GPU work, and the two were
    /// separated because the lattice turned out to be the cheap half.
    pub egui_gpu_ms: f32,
    /// Milliseconds the shell spent on its own per-frame work before the UI
    /// ran — draining the event rings and reconciling the take — or 0 where
    /// the shell doesn't measure it.
    ///
    /// Separate from the frame's CPU time because that starts at the dock
    /// build: this stretch scales with events ARRIVING rather than with what
    /// is drawn, and there was no reading it could show up in.
    pub shell_ms: f32,
    /// Milliseconds the previous frame blocked acquiring the surface — the
    /// vsync wait. Large here with every cost small means the frame is early,
    /// not slow.
    pub acquire_ms: f32,
    /// Milliseconds the previous frame callback took end to end, or 0 where
    /// the shell doesn't measure it.
    ///
    /// The other readings are stages of it. This is the total, and against the
    /// interval between frames it answers what no stage can: whether a long
    /// frame was SLOW, or just late being asked for.
    pub tick_ms: f32,
    /// Milliseconds of that callback spent inside the renderer. `tick_ms`
    /// minus this is the egui half — the UI closure plus egui's own
    /// end-of-pass work — so the two bracket the whole frame between them.
    pub render_ms: f32,
    /// The renderer's stages. `upload_ms` also covers paint callbacks'
    /// `prepare`, so the lattice's own buffer writes are inside it.
    pub upload_ms: f32,
    /// Of that, `update_buffers` itself. The difference is the command-encoder
    /// creation, the renderer's write lock and the MSAA resize, which the
    /// upload reading also spans.
    pub ubuf_ms: f32,
    /// Of the uploads, the TEXTURE half — the rest is buffer uploads, and
    /// with them the paint callbacks' `prepare`.
    pub texture_ms: f32,
    /// How many primitives and vertices the previous frame uploaded — the
    /// volume behind the upload cost, rather than another duration.
    pub prims: u32,
    pub verts: u32,
    pub encode_ms: f32,
    pub submit_ms: f32,
    /// Upper bound on how often the UI is drawn, in frames per second;
    /// `None` leaves it uncapped (as fast as the display can present).
    /// Persisted.
    ///
    /// Read by the shells to pace themselves, and by [`root_ui`] only to
    /// schedule repaints — never by any drawing code. The offline renderer
    /// steps its own clock and never reaches `root_ui`, so a recorded frame
    /// cannot depend on this and the determinism test stays honest.
    ///
    /// The repaint request alone cannot enforce this, and shells must not
    /// rely on it: egui takes the SMALLEST delay any caller asks for in a
    /// pass, and a zero-delay `request_repaint` (an input event, a hover
    /// animation, the plugin's own MIDI-drain repaint) additionally forces
    /// the following pass to zero. A cap expressed that way evaporates
    /// exactly when the UI is busy — the case it exists for. The plugin
    /// therefore drives its window's frame timer from this value, which is a
    /// hard bound because a frame that is never asked for is never drawn.
    pub fps_cap: Option<f32>,
    /// Rolling frame-rate / CPU / memory numbers for the performance overlay.
    /// Runtime-only; filled and drawn by [`root_ui`], never by the offline
    /// renderer (so recorded frames stay deterministic).
    perf: PerfStats,
}

/// A saved camera angle: what the built-in Flat/Isometric buttons are,
/// but user-defined. Only the orbit angles — projection, zoom, and pan
/// are deliberately not captured, so a preset composes with any of them.
#[derive(Clone, Debug, serde::Serialize, serde::Deserialize)]
pub struct CameraPreset {
    pub name: String,
    pub yaw: f32,
    pub pitch: f32,
}

/// The default pane arrangement: big lattice with the Spectral pane
/// beside it on the right (sharing the pitch intuition: what sounds is
/// what lights up), the tuning column further right, console and notes
/// tucked below that. Users can re-dock at runtime; the result persists
/// via UiPersist, and the Panel pane's "Reset layout" button returns here.
fn default_dock() -> DockState<panes::Tab> {
    let mut dock = DockState::new(vec![panes::Tab::Lattice]);
    let surface = dock.main_surface_mut();
    let [lattice, right] = surface.split_right(
        NodeIndex::root(),
        0.72,
        vec![
            // Reading outward from the picture: what the lattice is (its
            // tuning, and how it's framed), then how a note is drawn, the
            // scene around it, the analyzer, video export, and the plugin's
            // own render/layout knobs last.
            panes::Tab::Tuning,
            panes::Tab::Nodes,
            panes::Tab::Scene,
            panes::Tab::Analyzer,
            panes::Tab::Video,
            panes::Tab::Panel,
        ],
    );
    // Notes first so it sits left of Console and is the selected tab by
    // default (egui_dock makes tab index 0 active).
    surface.split_below(right, 0.55, vec![panes::Tab::Notes, panes::Tab::Console]);
    // Spectral as a column just right of the lattice: what sounds is directly
    // beside what lights up. Paired with the "Across" default orientation
    // (SpectrumConfig::default). Drag it wherever from here — egui_dock docks
    // it freely, and in Auto the Spectral pane's orientation follows the shape
    // it lands.
    surface.split_right(lattice, 0.72, vec![panes::Tab::Spectral]);
    dock
}

impl SharedState {
    pub fn new(target_format: TextureFormat) -> Self {
        let dock = default_dock();

        SharedState {
            tracker: NoteTracker::new(),
            tuning: Tuning::default(),
            view: ViewConfig::default(),
            frame_params: FrameParams::default(),
            camera: Camera::default(),
            hovered: None,
            console: Console::default(),
            target_format,
            background: lattice_scene::skin::panel_color(),
            learn_active: false,
            last_learned_classes: None,
            camera_presets: Vec::new(),
            preset_name: String::new(),
            take_supported: false,
            take_recording: false,
            take_rolling: false,
            render_now: false,
            last_take_ready: false,
            take_audio: false,
            render_config: RenderConfig::default(),
            take_status: String::new(),
            spectrum: AudioSpectrum::default(),
            spectrum_config: SpectrumConfig::default(),
            whole_song: None,
            reset_layout: false,
            dock,
            folds: fold::Folds::default(),
            lattice_stats: {
                let stats = lattice_render::LatticeStats::default();
                stats
                    .gpu_ms
                    .store(lattice_render::GPU_TIME_PENDING, std::sync::atomic::Ordering::Relaxed);
                std::sync::Arc::new(stats)
            },
            roll_notes: std::sync::atomic::AtomicU32::new(0),
            font_atlas: Default::default(),
            tess_ms: 0.0,
            egui_gpu_ms: 0.0,
            shell_ms: 0.0,
            acquire_ms: 0.0,
            tick_ms: 0.0,
            render_ms: 0.0,
            upload_ms: 0.0,
            ubuf_ms: 0.0,
            texture_ms: 0.0,
            prims: 0,
            verts: 0,
            encode_ms: 0.0,
            submit_ms: 0.0,
            fps_cap: None,
            perf: PerfStats::default(),
        }
    }

    pub fn log(&mut self, line: impl Into<String>) {
        self.console.log(line);
    }

    /// Discard the (persisted) dock arrangement and return to the default
    /// layout. Camera, view settings, and presets are untouched. Takes
    /// effect at the end of the frame (see the `reset_layout` field).
    pub fn reset_dock_layout(&mut self) {
        self.reset_layout = true;
    }

    /// Serialize the parts of the UI worth restoring across sessions
    /// (dock layout, camera, view settings). Parameters are NOT included —
    /// they live in the host's plugin state.
    pub fn save_persist(&self) -> String {
        // RON rather than JSON: dock layout rects can be NaN (before first
        // layout), which JSON cannot round-trip.
        ron::to_string(&UiPersist {
            version: UI_PERSIST_VERSION,
            dock: self.dock.clone(),
            folds: self.folds.clone(),
            camera: self.camera,
            view: self.view.clone(),
            camera_presets: self.camera_presets.clone(),
            spectrum: self.spectrum_config,
            render: self.render_config.clone(),
            fps_cap: self.fps_cap,
        })
        .unwrap_or_default()
    }

    /// Restore state saved by [`save_persist`]. Unknown/corrupt input is
    /// ignored (fresh defaults win over a broken restore).
    /// Drop everything that belongs to a particular egui context. Shells MUST
    /// call this whenever they build one.
    ///
    /// The plugin's editor creates a brand new `Context` every time its window
    /// opens, while this state lives on across them — so a `TextureHandle`
    /// taken from the previous one survives into the new window looking
    /// perfectly valid. It isn't: `set` on it reaches a context nobody is
    /// drawing any more, and its id names a texture the new renderer never
    /// allocated. The spectrogram simply vanished after hiding and re-showing
    /// the window, and stayed gone, because nothing ever asked for a fresh
    /// handle.
    pub fn release_context_resources(&mut self) {
        self.spectrum.release_textures();
    }

    pub fn load_persist(&mut self, serialized: &str) {
        if let Ok(persist) = ron::from_str::<UiPersist>(serialized) {
            // A pre-reorg (version 0) layout names the old tabs and is missing
            // the split-out Scene and new Panel tabs, so those controls would
            // be unreachable. The old tab names still deserialize (Tab's serde
            // aliases), which is what lets the settings below survive — but the
            // arrangement itself is stale, so refresh it to the new default.
            // Everything else the user dialed in is kept.
            // The folds go with the dock they were measured against: they name
            // splits by index, so a fresh arrangement has to start with none
            // rather than with fractions pointing into a tree that is gone.
            // Same reason "Reset layout" clears them.
            self.dock = if persist.version < UI_PERSIST_VERSION {
                self.folds.clear();
                default_dock()
            } else {
                self.folds = persist.folds;
                persist.dock
            };
            self.camera = persist.camera;
            self.view = persist.view;
            // Fold fields from older blob layouts (the NodeBody
            // experiment) into the current core/outer split.
            self.view.migrate_legacy();
            self.camera_presets = persist.camera_presets;
            self.spectrum_config = persist.spectrum;
            // Same job for the pitch range, which used to be a pair of
            // octave numbers.
            self.spectrum_config.migrate_legacy();
            self.render_config = persist.render;
            self.fps_cap = persist.fps_cap;
        }
    }
}

/// The current [`UiPersist`] layout version. Bumped when the `Tab` set changes
/// shape (rename/split/add/merge) so `load_persist` can refresh a stale dock
/// instead of stranding the user with missing tabs.
///
/// 2: Tuning and Frame merged into one tab. A version-1 layout has both, and
/// they now name the same variant — without the refresh the dock would open
/// with the merged pane in it twice.
const UI_PERSIST_VERSION: u32 = 2;

/// On-disk format of [`SharedState::save_persist`]. Bump thoughtfully; a
/// failed deserialize silently falls back to defaults.
#[derive(serde::Serialize, serde::Deserialize)]
struct UiPersist {
    /// serde(default) reads a pre-versioning blob as version 0, which
    /// [`SharedState::load_persist`] treats as "refresh the dock layout".
    #[serde(default)]
    version: u32,
    dock: DockState<panes::Tab>,
    /// serde(default) keeps pre-sideways-fold blobs loadable (as nothing
    /// folded, which is what they were).
    #[serde(default)]
    folds: fold::Folds,
    camera: Camera,
    view: ViewConfig,
    /// serde(default) keeps pre-preset persisted blobs loadable.
    #[serde(default)]
    camera_presets: Vec<CameraPreset>,
    /// serde(default) keeps pre-Spectrum-tab blobs loadable.
    #[serde(default)]
    spectrum: SpectrumConfig,
    #[serde(default)]
    render: RenderConfig,
    /// serde(default) keeps pre-cap blobs loadable (as uncapped).
    #[serde(default)]
    fps_cap: Option<f32>,
}

/// Parse just the render frame out of a persisted UI-state blob — so the
/// offline renderer can default its size and layout to what the take was
/// composed for, without building a whole [`SharedState`].
pub fn render_frame_from_persist(serialized: &str) -> Option<RenderFrame> {
    ron::from_str::<UiPersist>(serialized).ok().map(|persist| persist.render.frame)
}

/// Draw one frame of the whole UI into `ui`, which is expected to cover the
/// window (egui-baseview hands the plugin editor exactly that; eframe hands
/// the standalone harness the same via its `App::ui` hook).
///
/// The shell contract, which is otherwise only discoverable by reading both
/// shells: before calling this, feed the frame's MIDI into `state.tracker`
/// and its audio samples into `state.spectrum`. `now` is seconds on the
/// shell's clock, and must be the SAME clock that timestamped those
/// `NoteEvent`s — envelopes are derived from the difference.
/// End a drag whose release is never coming, because it is holding every
/// scroll area in the editor hostage.
///
/// egui decides whether a `ScrollArea` may take the wheel with
/// `ui.rect_contains_pointer(outer_rect) && ui.ctx().dragged_id().is_none()`
/// (`scroll_area.rs`). That second test is GLOBAL — not "is this area being
/// dragged" but "is anything anywhere being dragged" — so a single stale drag
/// silently stops the wheel in every settings pane at once, and stays that way
/// until something clears it.
///
/// A stale drag is easy to come by in a plugin window. egui ends a drag on the
/// button release, and deliberately keeps one alive when the pointer merely
/// leaves the viewport ("when dragging a slider and the mouse leaves the
/// viewport, we still want the drag to work" — `input_state`, which is why
/// `PointerGone` does not clear the pressed button either). But a plugin
/// editor is a guest inside a host window: let go outside it, or let the host
/// take focus mid-drag, and the release is delivered somewhere that is not us.
/// egui then believes the button is still down forever.
///
/// So: no pointer, or no focus, means no drag. The gesture it costs is
/// resuming a drag that wandered out of the window and came back, which is
/// what egui's rule buys; the gesture it saves is scrolling any settings pane,
/// which is otherwise dead until the next click. Only drags started INSIDE the
/// window and released inside it survive, and those are all of them in
/// practice.
///
/// Not a `Sense::drag` problem in any one pane — a ValueBar strands the wheel
/// exactly as well as the Analyzer's pan does, which is why this sits once at
/// the root rather than in whichever pane the drag came from.
fn end_stranded_drag(ctx: &egui::Context) {
    if ctx.dragged_id().is_none() {
        return;
    }
    // Focus is read from the EVENT as well as the flag. `InputState::focused`
    // comes from `RawInput::focused`, which starts true and only moves if an
    // integration sets it — and the plugin's (vendored egui-baseview) reports
    // focus by pushing `WindowFocused` and filling in `ViewportInfo`, never
    // that field. The flag alone would therefore be true forever in the one
    // shell this is most needed in. Reading both means neither a shell that
    // sets the flag nor one that only sends the event is missed, and a shell
    // that says nothing either way (the offline renderer, the tests) is
    // untouched.
    let lost = ctx.input(|i| {
        i.pointer.latest_pos().is_none()
            || !i.focused
            || i.events.iter().any(|e| matches!(e, egui::Event::WindowFocused(false)))
    });
    if lost {
        ctx.stop_dragging();
    }
}

pub fn root_ui(ui: &mut egui::Ui, state: &mut SharedState, params: &dyn ParamBackend, now: f64) {
    begin_frame(state, params, now);
    end_stranded_drag(ui.ctx());

    // Cleared before the panes run, so a frame with the roll hidden (or the
    // Spectral pane not on screen at all) reports zero notes rather than
    // whatever the last frame that had one reported.
    state.roll_notes.store(0, std::sync::atomic::Ordering::Relaxed);

    // Frameless mode hides every tab bar (the Lattice and Spectral panes
    // meet with no chrome between them — clean for captures). The pane
    // separators keep their regular width, so the spacing between windows
    // matches framed mode. No tab bar also means no way to click back to
    // the Panel pane (which holds the toggle) if it's hidden, so Esc always
    // restores.
    if state.view.frameless && ui.input(|i| i.key_pressed(egui::Key::Escape)) {
        state.view.frameless = false;
    }
    let mut dock_style = theme::dock_style(ui.style());
    if state.view.frameless {
        dock_style.tab_bar.height = 0.0;
    }

    // DockState has to be moved out while panes borrow the rest of `state`.
    let mut dock = std::mem::replace(&mut state.dock, DockState::new(vec![]));
    // Before the dock lays out: a pane collapsed inside a horizontal split
    // folds sideways to a rail, which is a split fraction, which is layout's
    // input. egui_dock's own vertical folds need nothing from us.
    state.folds.apply(&mut dock, &dock_style);
    // Time the whole dock build — every pane's layout and the scene
    // derivation — as the GUI thread's own per-frame CPU cost. The wgpu draw
    // is submitted inside and finishes off-thread, so this is CPU, not GPU.
    let cpu_start = std::time::Instant::now();
    DockArea::new(&mut dock)
        // Cloned because the rails are painted from the same style afterwards.
        .style(dock_style.clone())
        // The pane set is fixed, so closing chrome stays off — but the
        // collapse arrow earns its pixels: the Lattice and Spectral panes
        // fold down to their tab bar when screen space is tight.
        .show_close_buttons(false)
        .show_leaf_close_all_buttons(false)
        .show_leaf_collapse_buttons(true)
        .show_inside(ui, &mut panes::Viewer { state, params, now });
    let cpu_ms = cpu_start.elapsed().as_secs_f32() * 1000.0;
    // After it: the rails the folds left behind, which only this frame's
    // rectangles can place.
    fold::paint(ui, &dock, &dock_style);
    state.dock = dock;
    // Deferred from the Panel pane's button: replacing the dock BEFORE the
    // write-back above would be silently undone.
    if std::mem::take(&mut state.reset_layout) {
        state.dock = default_dock();
        state.folds.clear();
    }

    // Render continuously only while something is animating (sounding or
    // decaying voices); otherwise poll so newly arriving MIDI still shows
    // up promptly. egui repaints on input events by itself, so interaction
    // never waits on this. The plugin shell additionally requests a
    // repaint the moment it drains new note events.
    //
    // The piano roll keeps animating well past the last release fade — it
    // scrolls for as long as its window still reaches a played note — so
    // it gets its own say here. Without this the roll would advance in
    // 50 ms jerks once the voices died.
    //
    // Flowing audio counts too: the spectrum and spectrogram advance every
    // frame off the analyzer, so with audio playing but no MIDI they'd
    // otherwise crawl at the 50 ms idle poll.
    let animating = state.tracker.voices().next().is_some()
        || state.learn_active
        || roll_scrolling(state, now)
        || state.spectrum.is_flowing(now);
    if animating {
        // Uncapped means "as fast as the shell offers"; a cap turns that into
        // a minimum spacing between repaints. Only the request changes — the
        // frame that does get drawn is identical either way.
        match frame_interval(state.fps_cap) {
            Some(interval) => ui.ctx().request_repaint_after(interval),
            None => ui.ctx().request_repaint(),
        }
    } else {
        ui.ctx().request_repaint_after(IDLE_REPAINT_INTERVAL);
    }

    // Performance overlay: fold this frame's numbers in and, if it's on, draw
    // the corner HUD. Interactive path only — the offline renderer never
    // reaches root_ui, so nothing here touches a recorded frame.
    state.perf.record(
        perf::FrameCosts {
            shell_ms: state.shell_ms,
            cpu_ms,
            tess_ms: state.tess_ms,
            egui_gpu_ms: state.egui_gpu_ms,
            lattice_gpu_ms: f32::from_bits(
                state.lattice_stats.gpu_ms.load(std::sync::atomic::Ordering::Relaxed),
            ),
            prepare_ms: f32::from_bits(
                state.lattice_stats.prepare_ms.load(std::sync::atomic::Ordering::Relaxed),
            ),
            poll_ms: f32::from_bits(
                state.lattice_stats.poll_ms.load(std::sync::atomic::Ordering::Relaxed),
            ),
            write_ms: f32::from_bits(
                state.lattice_stats.write_ms.load(std::sync::atomic::Ordering::Relaxed),
            ),
            scene_ms: f32::from_bits(
                state.lattice_stats.scene_ms.load(std::sync::atomic::Ordering::Relaxed),
            ),
            acquire_ms: state.acquire_ms,
            tick_ms: state.tick_ms,
            render_ms: state.render_ms,
            upload_ms: state.upload_ms,
            ubuf_ms: state.ubuf_ms,
            texture_ms: state.texture_ms,
            prims: state.prims,
            verts: state.verts,
            roll_notes: state.roll_notes.load(std::sync::atomic::Ordering::Relaxed),
            spectrogram_fallbacks: state.spectrum.spectrogram_fallbacks(),
            encode_ms: state.encode_ms,
            submit_ms: state.submit_ms,
        },
        now,
        perf::Workload {
            active_voices: state.tracker.voices().count(),
            held_voices: state.tracker.held_count(),
            visible_nodes: state.view.visible_count(),
            render_scale: state.view.render_scale,
            animating,
        },
    );
    if state.view.show_perf {
        perf::draw_overlay(
            ui.ctx(),
            perf_overlay_area(state, ui.max_rect()),
            &state.perf,
            state.view.show_perf_detail,
        );
    }
}

/// Where the performance overlay hangs its top-right corner: the Spectral
/// pane's body, so the HUD sits over the spectrogram rather than over the
/// lattice, which is the picture being watched.
///
/// Falls back to `editor` (the whole window) whenever that pane is not on
/// screen — another tab selected in its leaf, or the leaf collapsed — so the
/// overlay never strands itself on a pane nobody can see.
fn perf_overlay_area(state: &SharedState, editor: egui::Rect) -> egui::Rect {
    let Some(path) = state.dock.find_tab(&panes::Tab::Spectral) else {
        return editor;
    };
    let egui_dock::Node::Leaf(leaf) = &state.dock[path.surface][path.node] else {
        return editor;
    };
    // `viewport` is the tab BODY; the picture panes drop their margin, so it
    // is exactly the drawn surface. `Rect::NOTHING` until the dock has laid
    // out once (a first frame, or a freshly loaded layout).
    if leaf.collapsed || leaf.active != path.tab || !leaf.viewport.is_positive() {
        return editor;
    }
    leaf.viewport
}

/// Everything that must happen once per frame before any pane draws:
/// refresh the per-frame mirrors of the parameters and age out voices
/// whose fade has completed.
///
/// [`root_ui`] calls this itself. It is public for shells that compose
/// their own layout instead of using the dock — the offline renderer
/// draws [`Pane`]s directly, and skipping this would leave it rendering
/// last frame's tuning against never-pruned voices.
pub fn begin_frame(state: &mut SharedState, params: &dyn ParamBackend, now: f64) {
    learn_step(state, params);

    state.tuning = params::tuning_from_params(params);
    // Meantone mode locks the major third to four perfect fifths: derive it
    // from the fifth here, so the whole pipeline (scene pitch classes,
    // matching, readouts) sees the locked value without any meantone
    // awareness of its own. The lock is exact in integer microcents, so
    // comma-equivalent nodes collapse to one pitch. The Five param is left
    // untouched (inert while the lock is on).
    if state.view.meantone {
        state.tuning.lock_meantone();
    }
    state.frame_params = FrameParams {
        fade_time: params.get(params::ParamKey::Fade),
        darkest_pitch: params.get(params::ParamKey::DarkestPitch),
        brightest_pitch: params.get(params::ParamKey::BrightestPitch),
    };
    // Every layer of a node now fades on this one time, so a voice is dead
    // to the display exactly when its envelope reaches zero.
    state.tracker.prune(now, state.frame_params.fade_time);
}

/// A pane that stands on its own, outside the dock.
///
/// Only the two *views* are here. The settings panes edit state that a
/// non-interactive renderer cannot change and a viewer should not see, so
/// they are deliberately unreachable this way.
#[derive(Clone, Copy, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub enum Pane {
    /// The 3D lattice.
    Lattice,
    /// The spectrum, voice bars and piano roll.
    Spectral,
}

/// Draw one pane's body into `ui`, filling it, with no dock or tab bar.
///
/// Callers must have run [`begin_frame`] for this `now` already. Panes
/// still read hover and pointer state from `ui`, so an offline caller
/// feeding synthetic input simply gets no hover — which is what a
/// recording wants.
pub fn draw_pane(ui: &mut egui::Ui, pane: Pane, state: &mut SharedState, now: f64) {
    match pane {
        Pane::Lattice => panes::lattice::lattice_pane(ui, state, now),
        // Offline: pixels-per-point already scales text, so no extra factor;
        // one spectrogram per frame, so texture slot 0.
        Pane::Spectral => panes::spectral::spectral_pane(ui, state, now, 1.0, 0),
    }
}

/// Repaint cadence while nothing animates: newly arriving MIDI shows up
/// within one poll even without an input event.
const IDLE_REPAINT_INTERVAL: std::time::Duration = std::time::Duration::from_millis(50);

/// The minimum spacing between repaints implied by a frame-rate cap, or
/// `None` when uncapped.
///
/// A cap that isn't a positive, finite rate is treated as uncapped rather
/// than turned into a zero or absurd interval. The control cannot produce
/// one, but a hand-edited persisted blob can, and "no cap" is the safe
/// reading of a nonsense value — a zero interval would merely be the
/// uncapped behaviour with extra steps, while a huge one would freeze the UI.
fn frame_interval(fps_cap: Option<f32>) -> Option<std::time::Duration> {
    match fps_cap {
        Some(fps) if fps.is_finite() && fps > 0.0 => {
            Some(std::time::Duration::from_secs_f32(1.0 / fps))
        }
        _ => None,
    }
}

/// Whether the piano roll still has something moving across it: its window
/// reaches back to a note that was sounding. Goes quiet once the last note
/// has scrolled off the far edge, so an idle plugin still idles.
fn roll_scrolling(state: &SharedState, now: f64) -> bool {
    let cfg = &state.spectrum_config;
    cfg.show_roll
        && cfg.roll_fraction > 0.0
        && state
            .tracker
            .roll()
            .latest_activity(now)
            .is_some_and(|last| now - last <= cfg.roll_seconds as f64)
}

/// One tick of learn mode (v1 semantics): while armed, whenever the set of
/// held pitch classes changes, re-infer the tuning and write it through the
/// param backend. Change-detected so the host only sees parameter sets when
/// something actually changed. No egui types — testable with a stub
/// backend.
fn learn_step(state: &mut SharedState, params: &dyn ParamBackend) {
    if !state.learn_active {
        state.last_learned_classes = None;
        return;
    }
    let mut classes: Vec<PitchClass> = state
        .tracker
        .voices()
        .filter(|v| v.state == lattice_core::VoiceState::Held)
        .map(|v| v.pitch_class)
        .collect();
    classes.sort_unstable();
    classes.dedup();
    if state.last_learned_classes.as_ref() == Some(&classes) {
        return;
    }
    if !classes.is_empty() {
        let learned = lattice_core::learn_tuning(&classes);
        for (value, key) in [
            (learned.c_offset, params::ParamKey::COffset),
            (learned.three, params::ParamKey::Three),
            (learned.five, params::ParamKey::Five),
            (learned.seven, params::ParamKey::Seven),
        ] {
            if let Some(value) = value {
                params.set(key, value);
            }
        }
        // Auto-engage (or release) meantone mode from what was learned:
        // when the chord pins down both a fifth and a third, turn meantone
        // on iff they sit in the meantone relationship. Chords that fix
        // only one of the two leave the mode as the user left it.
        if let (Some(three), Some(five)) = (learned.three, learned.five) {
            state.view.meantone = lattice_core::tuning::is_meantone(three, five);
        }
        state
            .console
            .log(format!("learn: {} held classes -> {:?}", classes.len(), learned));
    }
    state.last_learned_classes = Some(classes);
}

#[cfg(test)]
mod tests;

impl SharedState {
    /// Tell the state what ground the lattice is being composited over (see
    /// the `background` field). Takes sRGB bytes, the form every shell
    /// already has its background color in, so no shell needs glam to say it.
    pub fn set_background(&mut self, rgb: (u8, u8, u8)) {
        self.background = lattice_scene::skin::ground_color(rgb);
    }
}
