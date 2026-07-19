//! The shared UI shell: dockable panes, cross-pane hover state, and the
//! per-frame root function. Both the standalone harness and the plugin
//! editor call [`root_ui`] once per egui frame; everything else is internal.

pub mod params;
pub mod theme;
pub mod widgets;
mod panes;

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

/// Everything the Spectral pane's display is configured by, edited in the
/// Spectrum settings tab and persisted with the UI state.
#[derive(Clone, Copy, Debug, serde::Serialize, serde::Deserialize)]
pub struct SpectrumConfig {
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
}

fn default_true() -> bool {
    true
}

fn default_labels() -> SpectrumLabels {
    SpectrumLabels::Notes
}

/// The tilt settings offered, per analyzer convention (-1.5 dB/oct
/// increments; see [`SpectrumConfig::tilt`]).
pub const TILT_STEPS: [f32; 5] = [0.0, -1.5, -3.0, -4.5, -6.0];

impl Default for SpectrumConfig {
    fn default() -> Self {
        SpectrumConfig {
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
}

impl Default for AudioSpectrum {
    fn default() -> Self {
        AudioSpectrum {
            analyzer: lattice_core::spectrum::SpectrumAnalyzer::new(48_000.0),
            display: [0.0; lattice_core::spectrum::SPECTRUM_BINS],
            peaks: [0.0; lattice_core::spectrum::SPECTRUM_BINS],
            last_fft: None,
            last_samples: None,
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
                self.last_fft = Some(now);
            }
        }
        Some((&self.display, &self.peaks))
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
    /// Audio-derived spectrum for the Spectral pane. Runtime-only.
    pub spectrum: AudioSpectrum,
    /// The Spectral pane's settings (Spectrum tab; persisted).
    pub spectrum_config: SpectrumConfig,
    /// Set by the View pane's "Reset layout" button; consumed by root_ui
    /// AFTER the frame's DockArea writes the dock back (panes run inside
    /// that pass, so a direct write from one would be overwritten).
    reset_layout: bool,
    dock: DockState<panes::Tab>,
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
/// its own strip directly below it (sharing the pitch intuition: what
/// sounds is what lights up), tuning column on the right, console and
/// notes tucked below that. Users can re-dock at runtime; the result
/// persists via UiPersist, and the View pane's "Reset layout" button
/// returns here.
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
        ],
    );
    // Notes first so it sits left of Console and is the selected tab by
    // default (egui_dock makes tab index 0 active).
    surface.split_below(right, 0.55, vec![panes::Tab::Notes, panes::Tab::Console]);
    surface.split_below(lattice, 0.76, vec![panes::Tab::Spectral]);
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
            spectrum: AudioSpectrum::default(),
            spectrum_config: SpectrumConfig::default(),
            reset_layout: false,
            dock,
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
    DockArea::new(&mut dock)
        .style(dock_style)
        // The pane set is fixed, so closing chrome stays off — but the
        // collapse arrow earns its pixels: the Lattice and Spectral panes
        // fold down to their tab bar when screen space is tight.
        .show_close_buttons(false)
        .show_leaf_close_all_buttons(false)
        .show_leaf_collapse_buttons(true)
        .show_inside(ui, &mut panes::Viewer { state, params, now });
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
    if state.tracker.voices().next().is_some() || state.learn_active {
        ui.ctx().request_repaint();
    } else {
        ui.ctx().request_repaint_after(IDLE_REPAINT_INTERVAL);
    }
}

/// Repaint cadence while nothing animates: newly arriving MIDI shows up
/// within one poll even without an input event.
const IDLE_REPAINT_INTERVAL: std::time::Duration = std::time::Duration::from_millis(50);

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
