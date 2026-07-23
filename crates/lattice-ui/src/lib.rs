//! The shared UI shell: dockable panes, cross-pane hover state, and the
//! per-frame root function. Both the standalone harness and the plugin
//! editor call [`root_ui`] once per egui frame; everything else is internal.

pub mod layout;
pub mod params;
pub mod theme;
pub mod widgets;
mod panes;
mod perf;

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
/// spectrum curve, voice bars, piano roll — turns together.
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
    /// pane's long side. (The boolean matches the old pitch-axis flag, so
    /// only its interpretation in [`Axes`](crate::panes) changed.)
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
    /// Edited and previewed in the Render pane; the offline renderer reads it
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

/// The video frame the Render pane composes: an aspect ratio plus the
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
#[derive(Clone, Copy, Debug, serde::Serialize, serde::Deserialize)]
pub struct SpectrumConfig {
    /// Horizontal (pitch left-to-right) or vertical (pitch bottom-to-top),
    /// or Auto to follow the pane's shape; see [`SpectralOrientation`]. Those
    /// are the only orientations offered — the spectrum always stands up from
    /// its baseline with pitch ascending, so there is nothing to flip.
    #[serde(default)]
    pub orientation: SpectralOrientation,
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
    /// Stroke width of a note's outline, in pixels (notes are drawn as hollow
    /// outlines so the spectrogram shows through them).
    #[serde(default = "default_roll_outline_width")]
    pub roll_outline_width: f32,
    #[serde(default = "default_roll_color")]
    pub roll_color: RollColor,
    /// Scale a note's opacity by its velocity.
    #[serde(default = "default_true")]
    pub roll_velocity_alpha: bool,
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

fn default_roll_outline_width() -> f32 {
    1.5
}

fn default_roll_color() -> RollColor {
    RollColor::Channel
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
            orientation: SpectralOrientation::Horizontal,
            show_audio: true,
            window: SpectrumWindow::Balanced,
            floor_db: -60.0,
            smoothing: 0.55,
            tilt: 0.0,
            labels: SpectrumLabels::Notes,
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
            roll_outline_width: default_roll_outline_width(),
            roll_color: default_roll_color(),
            roll_velocity_alpha: true,
            roll_grid_seconds: default_roll_grid_seconds(),
            roll_now_line: true,
            show_spectrogram: false,
            spectrogram_color: SpectrogramColor::default(),
            spectrogram_opacity: default_spectrogram_opacity(),
            spectrogram_smoothing: 0.0,
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
    analyzer: lattice_core::spectrum::SpectrumAnalyzer,
    /// Smoothed display buckets (power; the pane maps to height).
    display: SpectrumBuckets,
    /// Decaying per-bucket maxima for the peak-hold outline.
    peaks: SpectrumBuckets,
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
    /// The spectrogram's pixels, uploaded and sampled with bilinear filtering
    /// so the heatmap reads as a smooth image rather than a mesh of interpolated
    /// triangles. One texture per drawing surface — index 0 the docked Spectral
    /// pane (and the offline render), index 1 the Render pane's preview — so two
    /// live spectrograms in one frame don't overwrite each other's texture.
    /// Runtime-only; created lazily on first draw.
    spectrogram_tex: [Option<egui::TextureHandle>; 2],
}

/// One column of the spectrogram: the raw power spectrum at a moment, on the
/// shell clock, so it can be placed on the roll's time axis.
pub struct SpectrogramColumn {
    pub time: f64,
    pub power: Box<SpectrumBuckets>,
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
    /// Analyze the entire `samples` buffer at the live FFT cadence, one raw
    /// column per hop, `time`-stamped in take time (`time_origin` is the take
    /// time of sample 0). Raw and unsmoothed exactly like the live ring — the
    /// spectrogram applies its own temporal smoothing when it draws.
    ///
    /// Pure: `(samples, rate, config)` in, columns out, no clock or RNG, so a
    /// render built on it stays byte-identical between runs.
    pub fn precompute(
        samples: &[f32],
        sample_rate: f32,
        time_origin: f64,
        start: f64,
        span: f64,
        config: &SpectrumConfig,
    ) -> WholeSong {
        let mut analyzer = lattice_core::spectrum::SpectrumAnalyzer::new(sample_rate);
        analyzer.set_fft_size(config.window.samples());
        let sr = (sample_rate as f64).max(1.0);
        let hop = AudioSpectrum::FFT_INTERVAL;
        let total = samples.len();
        let mut columns = Vec::new();
        // Feed the buffer in one-hop chunks; once the window has filled every
        // hop yields a column, exactly as the live `display` loop does.
        let (mut fed, mut k) = (0usize, 1usize);
        loop {
            let end = ((k as f64 * hop * sr).round() as usize).min(total);
            if end > fed {
                analyzer.push_samples(&samples[fed..end]);
                fed = end;
            }
            if let Some(power) = analyzer.pitch_spectrum() {
                columns.push(SpectrogramColumn {
                    time: time_origin + end as f64 / sr,
                    power: Box::new(power),
                });
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
            analyzer: lattice_core::spectrum::SpectrumAnalyzer::new(48_000.0),
            display: [0.0; lattice_core::spectrum::SPECTRUM_BINS],
            peaks: [0.0; lattice_core::spectrum::SPECTRUM_BINS],
            last_fft: None,
            last_samples: None,
            history: VecDeque::new(),
            spectrogram_tex: [None, None],
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
    pub fn display(
        &mut self,
        now: f64,
        config: &SpectrumConfig,
    ) -> Option<(&SpectrumBuckets, &SpectrumBuckets)> {
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
    fn push_history(&mut self, now: f64, power: SpectrumBuckets) {
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

    /// Whether audio has arrived within the hold window — i.e. the spectrum is
    /// still live. Drives continuous repaint so the curve and spectrogram stay
    /// smooth even when no MIDI is animating the frame. Reads true only while
    /// samples are actually arriving (the shell pushes them when the spectrum is
    /// shown), so it idles cleanly once audio stops.
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
    /// One-shot: set by the Render pane's "Render now" button, consumed by the
    /// shell to render the last take with the CURRENT settings. Runtime-only.
    pub render_now: bool,
    /// Whether a take has been recorded this session — the shell sets it so the
    /// Render pane can offer "Render now". Runtime-only.
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
    /// The Spectral pane's settings (Spectrum tab; persisted).
    pub spectrum_config: SpectrumConfig,
    /// Offline playhead render: the whole take's spectrogram laid out
    /// statically with a playhead at `now`, instead of the live scrolling
    /// window. `Some` only in the offline renderer. Runtime-only, never
    /// persisted (mirrors `learn_active`).
    pub whole_song: Option<WholeSong>,
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

/// The default pane arrangement: big lattice with the Spectral pane
/// beside it on the right (sharing the pitch intuition: what sounds is
/// what lights up), the tuning column further right, console and notes
/// tucked below that. Users can re-dock at runtime; the result persists
/// via UiPersist, and the View pane's "Reset layout" button returns here.
fn default_dock() -> DockState<panes::Tab> {
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
            panes::Tab::Render,
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
            learn_active: false,
            last_learned_classes: None,
            camera_presets: Vec::new(),
            preset_name: String::new(),
            take_supported: false,
            take_recording: false,
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
        state.dock = default_dock();
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
        perf::Workload {
            active_voices: state.tracker.voices().count(),
            held_voices: state.tracker.held_count(),
            visible_nodes: state.view.visible_count(),
            render_scale: state.view.render_scale,
            animating,
        },
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
        // Offline: pixels-per-point already scales text, so no extra factor;
        // one spectrogram per frame, so texture slot 0.
        Pane::Spectral => panes::spectral::spectral_pane(ui, state, now, 1.0, 0),
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
