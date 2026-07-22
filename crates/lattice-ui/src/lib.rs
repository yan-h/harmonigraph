//! The shared UI shell: dockable panes, cross-pane hover state, and the
//! per-frame root function. Both the standalone harness and the plugin
//! editor call [`root_ui`] once per egui frame; everything else is internal.

pub mod params;
pub mod theme;
pub mod widgets;
mod panes;
mod perf;

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
/// spectrum curve, voice bars, piano roll — turns together.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub enum SpectralOrientation {
    /// Follow the pane's shape: a pane taller than it is wide goes
    /// vertical, otherwise horizontal. Means the pane reads correctly
    /// both under the lattice and beside it, with no setting to change.
    #[default]
    Auto,
    /// Pitch left-to-right; the spectrum grows upward from the bottom.
    Horizontal,
    /// Pitch bottom-to-top; the spectrum grows rightward from the left.
    /// The classic piano-roll orientation.
    Vertical,
}

impl SpectralOrientation {
    /// Resolve [`Auto`](Self::Auto) against the pane it is drawing into.
    fn is_vertical(self, rect: egui::Rect) -> bool {
        match self {
            SpectralOrientation::Auto => rect.height() > rect.width(),
            SpectralOrientation::Horizontal => false,
            SpectralOrientation::Vertical => true,
        }
    }
}

/// Where the Spectral pane sits in the default pane arrangement. Changing
/// it rebuilds the dock (the arrangement is otherwise persisted forever).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub enum SpectralPlacement {
    /// A wide strip under the lattice (the original layout): what sounds
    /// is directly under what lights up.
    #[default]
    Below,
    /// A tall strip between the lattice and the settings column. Paired
    /// with [`SpectralOrientation::Auto`] this turns the pane vertical,
    /// which is also the natural piano-roll orientation.
    Right,
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
    /// Black → deep red → orange → yellow → white. The familiar "heat"
    /// spectrogram; reads loudest as hottest.
    #[default]
    Heat,
    /// Black → navy → blue → cyan → white. Cool counterpart to Heat.
    Ice,
    /// Black → violet → teal → green → yellow. A perceptually even ramp
    /// (viridis-like) where every step reads as an equal change.
    Aurora,
    /// Each cell takes the lattice's own low-to-high pitch color, dimmed by
    /// intensity — so the spectrogram speaks the same color language as the
    /// nodes and the Pitch-colored roll.
    Pitch,
}

/// What counts as "the take is done", and so when a video gets rendered.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub enum RenderTrigger {
    /// When you switch Record take off. Predictable, and the only option
    /// that works when the transport is looping.
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
}

/// How a finished take gets turned into a video, edited in the View
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
    /// Extra flags, split on whitespace (no shell quoting):
    /// `--size 3840x2160 --layout side-by-side`.
    #[serde(default)]
    pub extra_args: String,
}

impl Default for RenderConfig {
    fn default() -> Self {
        RenderConfig {
            record_audio: false,
            auto_render: false,
            trigger: RenderTrigger::OnDisarm,
            renderer_path: String::new(),
            audio_path: String::new(),
            extra_args: "--size 1920x1080".into(),
        }
    }
}

/// Everything the Spectral pane's display is configured by, edited in the
/// Spectrum settings tab and persisted with the UI state.
#[derive(Clone, Copy, Debug, serde::Serialize, serde::Deserialize)]
pub struct SpectrumConfig {
    /// Which way the pitch axis runs; see [`SpectralOrientation`].
    #[serde(default)]
    pub orientation: SpectralOrientation,
    /// Reverse the pitch axis (high notes first).
    #[serde(default)]
    pub flip_pitch: bool,
    /// Put the baseline on the opposite edge: the spectrum hangs down
    /// instead of standing up (and the roll flows the other way).
    #[serde(default)]
    pub flip_depth: bool,
    /// Where the pane sits when the layout is (re)built.
    #[serde(default)]
    pub placement: SpectralPlacement,
    /// Analyze and overlay the shell's audio (plugin: the input bus;
    /// standalone: a synth on the held notes).
    #[serde(default = "default_true")]
    pub show_audio: bool,
    pub window: SpectrumWindow,
    /// Bottom of the dB height scale; a full-scale sine sits at 0 dB.
    pub floor_db: f32,
    /// Display inertia: 0 = every refresh lands instantly, 0.9 = slow.
    pub smoothing: f32,
    /// Spectral tilt, in the convention analyzers use: the reference
    /// slope in dB/octave that displays as FLAT, one of [`TILT_STEPS`]
    /// (0, -1.5 .. -6). 0 draws raw power; -3 makes pink noise read
    /// flat; -4.5 flattens typical musical material. The display lifts
    /// treble by the magnitude, pivoting at 1 kHz — there is no
    /// bass-emphasizing direction, matching convention.
    #[serde(default)]
    pub tilt: f32,
    /// Axis gridline labeling.
    #[serde(default = "default_labels")]
    pub labels: SpectrumLabels,
    /// Fill under the curve instead of a bare line.
    pub fill: bool,
    /// Keep a slowly decaying outline at each bucket's recent maximum.
    pub peak_hold: bool,
    /// MIDI-derived bars at each voice's actual pitch.
    pub show_voice_bars: bool,
    /// Displayed octave range, in Bitwig octave numbers (C-1..C9 = full
    /// axis). The analyzer always covers the full axis; this only zooms
    /// the view.
    pub low_octave: i32,
    pub high_octave: i32,

    // ---- Piano roll -------------------------------------------------
    // The played-note timeline (lattice-core's NoteRoll) drawn over the
    // same pitch axis, occupying the far end of the depth axis. Time runs
    // away from the spectrum: a note leaving the roll's near edge meets
    // the spectrum peak it is making.
    /// Draw the incoming MIDI's history at all.
    #[serde(default = "default_true")]
    pub show_roll: bool,
    /// Share of the pane's depth given to the roll (the rest is the
    /// spectrum). 0 hides it; 1 gives the whole pane to the roll.
    #[serde(default = "default_roll_fraction")]
    pub roll_fraction: f32,
    /// Seconds of history the roll's depth spans.
    #[serde(default = "default_roll_seconds")]
    pub roll_seconds: f32,
    /// Note ribbon width, in semitones of the pitch axis.
    #[serde(default = "default_roll_thickness")]
    pub roll_thickness: f32,
    /// Corner rounding of an unbent note, as a fraction of half its width.
    #[serde(default = "default_roll_rounding")]
    pub roll_rounding: f32,
    /// Overall roll opacity.
    #[serde(default = "default_roll_opacity")]
    pub roll_opacity: f32,
    /// Note interior opacity as a fraction of `roll_opacity`: 1 is solid,
    /// lower lets the spectrogram (and each note's own fundamental) show
    /// through the ribbon. Below 1 an outline is drawn regardless of
    /// `roll_outline`, so a hollow note stays clearly bounded.
    #[serde(default = "default_roll_fill")]
    pub roll_fill: f32,
    #[serde(default = "default_roll_color")]
    pub roll_color: RollColor,
    /// Scale a note's opacity by its velocity.
    #[serde(default = "default_true")]
    pub roll_velocity_alpha: bool,
    /// How much a note dims as it ages toward the far edge (0 = not at
    /// all, 1 = to nothing).
    #[serde(default = "default_roll_age_fade")]
    pub roll_age_fade: f32,
    /// Outline every note in its own color, brightened.
    #[serde(default)]
    pub roll_outline: bool,
    /// Mark each note's attack with a bright cap.
    #[serde(default = "default_true")]
    pub roll_onsets: bool,
    /// Keep still-held notes at full brightness regardless of age fade,
    /// so what is sounding stands out from what has been.
    #[serde(default = "default_true")]
    pub roll_highlight_held: bool,
    /// Seconds between the roll's time gridlines; 0 draws none.
    #[serde(default = "default_roll_grid_seconds")]
    pub roll_grid_seconds: f32,
    /// Draw the line where the roll meets the spectrum ("now").
    #[serde(default = "default_true")]
    pub roll_now_line: bool,

    // ---- Spectrogram ------------------------------------------------
    // A frequency-vs-time heatmap of the analyzed audio, drawn in the
    // roll's depth region on the roll's own time axis — so each column of
    // spectral energy lines up with the notes that made it.
    /// Draw the spectrogram heatmap (over the roll's time window).
    #[serde(default)]
    pub show_spectrogram: bool,
    /// The heatmap's color ramp.
    #[serde(default)]
    pub spectrogram_color: SpectrogramColor,
    /// Overall spectrogram opacity, so it can sit under the notes without
    /// swamping them. (For the heatmap alone, turn the note ribbons off with
    /// `show_roll`.)
    #[serde(default = "default_spectrogram_opacity")]
    pub spectrogram_opacity: f32,
    /// Temporal smoothing of the heatmap, 0 = off. Blends each time column
    /// with its neighbors (symmetric, so no directional smear) to average out
    /// fast beating/chorus wobble, at the cost of some time-sharpness.
    #[serde(default)]
    pub spectrogram_smoothing: f32,
}

fn default_spectrogram_opacity() -> f32 {
    0.85
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

fn default_roll_thickness() -> f32 {
    0.8
}

fn default_roll_rounding() -> f32 {
    0.5
}

fn default_roll_opacity() -> f32 {
    0.9
}

fn default_roll_fill() -> f32 {
    1.0
}

fn default_roll_color() -> RollColor {
    RollColor::Channel
}

fn default_roll_age_fade() -> f32 {
    0.45
}

fn default_roll_grid_seconds() -> f32 {
    1.0
}

/// The tilt settings offered, per analyzer convention (-1.5 dB/oct
/// increments; see [`SpectrumConfig::tilt`]).
pub const TILT_STEPS: [f32; 5] = [0.0, -1.5, -3.0, -4.5, -6.0];

impl Default for SpectrumConfig {
    fn default() -> Self {
        SpectrumConfig {
            orientation: SpectralOrientation::Auto,
            flip_pitch: false,
            flip_depth: false,
            placement: SpectralPlacement::Below,
            show_audio: true,
            window: SpectrumWindow::Balanced,
            floor_db: -60.0,
            smoothing: 0.55,
            tilt: 0.0,
            labels: SpectrumLabels::Notes,
            fill: true,
            peak_hold: false,
            show_voice_bars: true,
            low_octave: -1,
            high_octave: 9,
            show_roll: true,
            roll_fraction: default_roll_fraction(),
            roll_seconds: default_roll_seconds(),
            roll_thickness: default_roll_thickness(),
            roll_rounding: default_roll_rounding(),
            roll_opacity: default_roll_opacity(),
            roll_fill: default_roll_fill(),
            roll_color: default_roll_color(),
            roll_velocity_alpha: true,
            roll_age_fade: default_roll_age_fade(),
            roll_outline: false,
            roll_onsets: true,
            roll_highlight_held: true,
            roll_grid_seconds: default_roll_grid_seconds(),
            roll_now_line: true,
            show_spectrogram: false,
            spectrogram_color: SpectrogramColor::default(),
            spectrogram_opacity: default_spectrogram_opacity(),
            spectrogram_smoothing: 0.0,
        }
    }
}

/// Audio-derived pitch spectrum shown in the Spectral pane. The shell
/// feeds mono samples every frame from wherever its audio comes from
/// (plugin: input bus via a ring buffer; standalone: the mock synth); the
/// pane asks for a display refresh when it draws. Runtime-only.
pub struct AudioSpectrum {
    analyzer: lattice_core::spectrum::SpectrumAnalyzer,
    /// Smoothed display buckets (power; the pane maps to height).
    display: [f32; lattice_core::spectrum::SPECTRUM_BINS],
    /// Decaying per-bucket maxima for the peak-hold outline.
    peaks: [f32; lattice_core::spectrum::SPECTRUM_BINS],
    /// When the FFT last ran, on the shell clock. The FFT is throttled
    /// well below frame rate — it feeds a meter, not an oscilloscope.
    last_fft: Option<f64>,
    /// When samples last arrived; the curve hides once the source stops
    /// (closed input bus, switched-off synth) rather than freezing.
    last_samples: Option<f64>,
    /// Timestamped raw spectra, one per FFT, for the spectrogram — oldest
    /// first. Raw (unsmoothed) so time isn't blurred across columns.
    /// Bounded by age and count (see [`AudioSpectrum::push_history`]).
    history: VecDeque<SpectrogramColumn>,
    /// The spectrogram's pixels, uploaded once and sampled with bilinear
    /// filtering so the heatmap reads as a smooth image rather than a mesh of
    /// interpolated triangles. Runtime-only; created lazily on first draw.
    spectrogram_tex: Option<egui::TextureHandle>,
}

/// One column of the spectrogram: the raw power spectrum at a moment, on the
/// shell clock, so it can be placed on the roll's time axis.
pub struct SpectrogramColumn {
    pub time: f64,
    pub power: Box<[f32; lattice_core::spectrum::SPECTRUM_BINS]>,
}

impl Default for AudioSpectrum {
    fn default() -> Self {
        AudioSpectrum {
            analyzer: lattice_core::spectrum::SpectrumAnalyzer::new(48_000.0),
            display: [0.0; lattice_core::spectrum::SPECTRUM_BINS],
            peaks: [0.0; lattice_core::spectrum::SPECTRUM_BINS],
            last_fft: None,
            last_samples: None,
            history: VecDeque::new(),
            spectrogram_tex: None,
        }
    }
}

impl AudioSpectrum {
    /// Seconds between FFTs (20 Hz refresh).
    const FFT_INTERVAL: f64 = 0.05;
    /// How long after the last samples the curve keeps drawing.
    const HOLD_SECONDS: f64 = 0.5;
    /// Peak-hold half-life in seconds.
    const PEAK_HALF_LIFE: f64 = 1.2;

    /// Feed mono samples from the shell. `now` is the shell clock also
    /// passed to [`root_ui`].
    pub fn push_samples(&mut self, samples: &[f32], sample_rate: f32, now: f64) {
        if samples.is_empty() {
            return;
        }
        self.analyzer.set_sample_rate(sample_rate);
        self.analyzer.push_samples(samples);
        self.last_samples = Some(now);
    }

    /// Advance the display (runs the FFT at most every FFT_INTERVAL under
    /// the config's window/smoothing) and return (levels, peak-holds) to
    /// draw, or None while no audio is flowing.
    #[allow(clippy::type_complexity)]
    pub fn display(
        &mut self,
        now: f64,
        config: &SpectrumConfig,
    ) -> Option<(
        &[f32; lattice_core::spectrum::SPECTRUM_BINS],
        &[f32; lattice_core::spectrum::SPECTRUM_BINS],
    )> {
        // A window change mid-stream just refills the ring; the display
        // holds its last values until the new window fills.
        self.analyzer.set_fft_size(config.window.samples());

        if !self.last_samples.is_some_and(|t| now - t <= Self::HOLD_SECONDS) {
            return None;
        }
        if self.last_fft.is_none_or(|t| now - t >= Self::FFT_INTERVAL) {
            if let Some(fresh) = self.analyzer.pitch_spectrum() {
                let alpha = 1.0 - config.smoothing.clamp(0.0, 0.95);
                let dt = self.last_fft.map_or(Self::FFT_INTERVAL, |t| now - t);
                let decay = 0.5f32.powf((dt / Self::PEAK_HALF_LIFE) as f32);
                for ((shown, peak), new) in
                    self.display.iter_mut().zip(&mut self.peaks).zip(fresh)
                {
                    *shown += (new - *shown) * alpha;
                    *peak = if config.peak_hold {
                        (*peak * decay).max(*shown)
                    } else {
                        // Track the live level while off, so switching the
                        // outline on starts from now, not stale maxima.
                        *shown
                    };
                }
                // Keep the RAW spectrum for the spectrogram (the smoothed
                // `display` would smear one column into the next).
                self.push_history(now, fresh);
                self.last_fft = Some(now);
            }
        }
        Some((&self.display, &self.peaks))
    }

    /// The longest roll window (`roll_seconds` max), plus a margin, is the
    /// most history the spectrogram can ever show; drop older columns.
    const HISTORY_SECONDS: f64 = 130.0;
    /// Backstop on the column count regardless of timing.
    const HISTORY_MAX: usize = 4000;

    /// Append one raw spectrum to the ring, trimming the far past.
    fn push_history(
        &mut self,
        now: f64,
        power: [f32; lattice_core::spectrum::SPECTRUM_BINS],
    ) {
        self.history.push_back(SpectrogramColumn { time: now, power: Box::new(power) });
        let oldest_kept = now - Self::HISTORY_SECONDS;
        while self
            .history
            .front()
            .is_some_and(|c| c.time < oldest_kept || self.history.len() > Self::HISTORY_MAX)
        {
            self.history.pop_front();
        }
    }

    /// The spectrogram columns, oldest first. Empty until audio has flowed.
    pub fn history(&self) -> &VecDeque<SpectrogramColumn> {
        &self.history
    }

    /// Forget the spectrogram history (paired with clearing the roll).
    pub fn clear_history(&mut self) {
        self.history.clear();
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
    /// While true, tuning params continuously re-learn from the held notes
    /// (v1's learn mode). Runtime-only; never persisted.
    pub learn_active: bool,
    /// Held pitch classes the last learn ran against (change detection).
    last_learned_classes: Option<Vec<PitchClass>>,
    /// User-saved camera angles, applied like the built-in Flat/Isometric
    /// presets (persisted; see the View pane).
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
    /// Toggled by the View pane, acted on by the shell.
    pub take_recording: bool,
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
    /// The Spectral pane's settings (Spectrum tab; persisted).
    pub spectrum_config: SpectrumConfig,
    /// Set by the View pane's "Reset layout" button; consumed by root_ui
    /// AFTER the frame's DockArea writes the dock back (panes run inside
    /// that pass, so a direct write from one would be overwritten).
    reset_layout: bool,
    dock: DockState<panes::Tab>,
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

/// The default pane arrangement: big lattice with the Spectral pane in
/// its own strip (below it by default — sharing the pitch intuition: what
/// sounds is what lights up — or beside it, per `placement`), tuning
/// column on the right, console and notes tucked below that. Users can
/// re-dock at runtime; the result persists via UiPersist, and the View
/// pane's "Reset layout" button returns here.
fn default_dock(placement: SpectralPlacement) -> DockState<panes::Tab> {
    let mut dock = DockState::new(vec![panes::Tab::Lattice]);
    let surface = dock.main_surface_mut();
    let [lattice, right] = surface.split_right(
        NodeIndex::root(),
        0.72,
        vec![
            panes::Tab::Tuning,
            panes::Tab::View,
            panes::Tab::Appearance,
            panes::Tab::Spectrum,
        ],
    );
    // Notes first so it sits left of Console and is the selected tab by
    // default (egui_dock makes tab index 0 active).
    surface.split_below(right, 0.55, vec![panes::Tab::Notes, panes::Tab::Console]);
    // The fractions differ because the strip's short side is what matters:
    // a wide strip under the lattice needs less of the height than a tall
    // strip beside it needs of the (already narrowed) width.
    match placement {
        SpectralPlacement::Below => {
            surface.split_below(lattice, 0.76, vec![panes::Tab::Spectral]);
        }
        SpectralPlacement::Right => {
            surface.split_right(lattice, 0.72, vec![panes::Tab::Spectral]);
        }
    }
    dock
}

impl SharedState {
    pub fn new(target_format: TextureFormat) -> Self {
        let dock = default_dock(SpectralPlacement::default());

        SharedState {
            tracker: NoteTracker::new(),
            tuning: Tuning::default(),
            view: ViewConfig::default(),
            frame_params: FrameParams::default(),
            camera: Camera::default(),
            hovered: None,
            console: Console::default(),
            target_format,
            learn_active: false,
            last_learned_classes: None,
            camera_presets: Vec::new(),
            preset_name: String::new(),
            take_supported: false,
            take_recording: false,
            take_audio: false,
            render_config: RenderConfig::default(),
            take_status: String::new(),
            spectrum: AudioSpectrum::default(),
            spectrum_config: SpectrumConfig::default(),
            reset_layout: false,
            dock,
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
            dock: self.dock.clone(),
            camera: self.camera,
            view: self.view.clone(),
            camera_presets: self.camera_presets.clone(),
            spectrum: self.spectrum_config,
            render: self.render_config.clone(),
        })
        .unwrap_or_default()
    }

    /// Restore state saved by [`save_persist`]. Unknown/corrupt input is
    /// ignored (fresh defaults win over a broken restore).
    pub fn load_persist(&mut self, serialized: &str) {
        if let Ok(persist) = ron::from_str::<UiPersist>(serialized) {
            self.dock = persist.dock;
            self.camera = persist.camera;
            self.view = persist.view;
            // Fold fields from older blob layouts (the NodeBody
            // experiment) into the current core/outer split.
            self.view.migrate_legacy();
            self.camera_presets = persist.camera_presets;
            self.spectrum_config = persist.spectrum;
            self.render_config = persist.render;
        }
    }
}

/// On-disk format of [`SharedState::save_persist`]. Bump thoughtfully; a
/// failed deserialize silently falls back to defaults.
#[derive(serde::Serialize, serde::Deserialize)]
struct UiPersist {
    dock: DockState<panes::Tab>,
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
pub fn root_ui(ui: &mut egui::Ui, state: &mut SharedState, params: &dyn ParamBackend, now: f64) {
    begin_frame(state, params, now);

    // Frameless mode hides every tab bar (the Lattice and Spectral panes
    // meet with no chrome between them — clean for captures). The pane
    // separators keep their regular width, so the spacing between windows
    // matches framed mode. No tab bar also means no way to click the View
    // pane back if it's hidden, so Esc always restores.
    if state.view.frameless && ui.input(|i| i.key_pressed(egui::Key::Escape)) {
        state.view.frameless = false;
    }
    let mut dock_style = theme::dock_style(ui.style());
    if state.view.frameless {
        dock_style.tab_bar.height = 0.0;
    }

    // DockState has to be moved out while panes borrow the rest of `state`.
    let mut dock = std::mem::replace(&mut state.dock, DockState::new(vec![]));
    // Time the whole dock build — every pane's layout and the scene
    // derivation — as the GUI thread's own per-frame CPU cost. The wgpu draw
    // is submitted inside and finishes off-thread, so this is CPU, not GPU.
    let cpu_start = std::time::Instant::now();
    DockArea::new(&mut dock)
        .style(dock_style)
        // The pane set is fixed, so closing chrome stays off — but the
        // collapse arrow earns its pixels: the Lattice and Spectral panes
        // fold down to their tab bar when screen space is tight.
        .show_close_buttons(false)
        .show_leaf_close_all_buttons(false)
        .show_leaf_collapse_buttons(true)
        .show_inside(ui, &mut panes::Viewer { state, params, now });
    let cpu_ms = cpu_start.elapsed().as_secs_f32() * 1000.0;
    state.dock = dock;
    // Deferred from the View pane's button: replacing the dock BEFORE the
    // write-back above would be silently undone.
    if std::mem::take(&mut state.reset_layout) {
        state.dock = default_dock(state.spectrum_config.placement);
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
    let animating =
        state.tracker.voices().next().is_some() || state.learn_active || roll_scrolling(state, now);
    if animating {
        ui.ctx().request_repaint();
    } else {
        ui.ctx().request_repaint_after(IDLE_REPAINT_INTERVAL);
    }

    // Performance overlay: fold this frame's numbers in and, if it's on, draw
    // the corner HUD. Interactive path only — the offline renderer never
    // reaches root_ui, so nothing here touches a recorded frame.
    let dt = ui.input(|i| i.stable_dt);
    state.perf.record(
        dt,
        cpu_ms,
        now,
        state.tracker.voices().count(),
        state.tracker.held_count(),
        state.view.visible_count(),
        state.view.render_scale,
        animating,
    );
    if state.view.show_perf {
        perf::draw_overlay(ui.ctx(), ui.max_rect(), &state.perf);
    }
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
        Pane::Spectral => panes::spectral::spectral_pane(ui, state, now),
    }
}

/// Repaint cadence while nothing animates: newly arriving MIDI shows up
/// within one poll even without an input event.
const IDLE_REPAINT_INTERVAL: std::time::Duration = std::time::Duration::from_millis(50);

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
