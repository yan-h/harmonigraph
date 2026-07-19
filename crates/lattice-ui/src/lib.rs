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
pub struct Console {
    lines: VecDeque<String>,
    max_lines: usize,
}

impl Default for Console {
    fn default() -> Self {
        Console { lines: VecDeque::new(), max_lines: 500 }
    }
}

impl Console {
    pub fn log(&mut self, line: impl Into<String>) {
        if self.lines.len() == self.max_lines {
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
/// window (egui-baseview hands the plugin editor exactly that; the
/// standalone harness wraps a frameless CentralPanel). `now` is seconds on
/// the shell's clock (the same clock used to timestamp `NoteEvent`s).
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
        pitch_class_fade_time: params.get(params::ParamKey::PitchClassFade),
        octave_fade_time: params.get(params::ParamKey::OctaveFade),
        darkest_pitch: params.get(params::ParamKey::DarkestPitch),
        brightest_pitch: params.get(params::ParamKey::BrightestPitch),
    };
    // Voices must outlive the LONGER of the two fades or the octave
    // indicators get truncated when the note highlight ends first.
    state.tracker.prune(
        now,
        state
            .frame_params
            .pitch_class_fade_time
            .max(state.frame_params.octave_fade_time),
    );

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
mod tests {
    use super::*;

    #[test]
    fn persist_round_trips_camera_and_view() {
        let mut state = SharedState::new(TextureFormat::Bgra8Unorm);
        state.camera.yaw = 1.23;
        state.camera.distance = 42.0;
        state.view.extent_sevens = 3;
        // Non-default values throughout, so the fields prove they
        // round-trip rather than matching the defaults by luck.
        state.view.outer_style = lattice_scene::OuterStyle::Rings;
        // Radius 0 is the off state; this proves it (and solidity) persist.
        state.view.core_radius = 0.0;
        state.view.core_solidity = 0.4;
        state.view.outer_inner = 0.1;
        state.view.outer_outer = 0.7;
        state.view.outer_backdrop = true;
        state.view.outer_solidity = 0.3;
        state.view.idle_marker = lattice_scene::IdleMarker::Dot;
        state.view.idle_radius = 0.31;
        state.view.meantone = true;
        state.camera_presets.push(CameraPreset {
            name: "reading".into(),
            yaw: 0.7,
            pitch: 0.2,
        });
        let saved = state.save_persist();

        let mut restored = SharedState::new(TextureFormat::Bgra8Unorm);
        restored.load_persist(&saved);
        assert_eq!(restored.camera.yaw, 1.23);
        assert_eq!(restored.camera.distance, 42.0);
        assert_eq!(restored.view.extent_sevens, 3);
        assert_eq!(restored.view.outer_style, lattice_scene::OuterStyle::Rings);
        assert_eq!(restored.view.core_radius, 0.0, "off (radius 0) round-trips");
        assert_eq!(restored.view.core_solidity, 0.4);
        assert_eq!(restored.view.outer_inner, 0.1);
        assert_eq!(restored.view.outer_outer, 0.7);
        assert!(restored.view.outer_backdrop);
        assert_eq!(restored.view.outer_solidity, 0.3);
        assert_eq!(restored.view.idle_marker, lattice_scene::IdleMarker::Dot);
        assert_eq!(restored.view.idle_radius, 0.31);
        assert!(restored.view.meantone);
        assert_eq!(restored.camera_presets.len(), 1);
        assert_eq!(restored.camera_presets[0].name, "reading");
        assert_eq!(restored.camera_presets[0].yaw, 0.7);
    }

    #[test]
    fn removed_node_styles_in_old_persist_blobs_load_as_steady() {
        // Breathe/Sparks and the later-trimmed Wire/Corona/… set no longer
        // exist; serde aliases must absorb them so an old blob still restores
        // (a failed parse would silently drop the WHOLE persist — layout,
        // camera, everything). "Wire" is one of the removed names.
        let mut state = SharedState::new(TextureFormat::Bgra8Unorm);
        state.camera.yaw = 1.23;
        state.view.node_style = lattice_scene::NodeStyle::Vortex;
        let saved = state.save_persist().replace("node_style:Vortex", "node_style:Wire");
        assert_ne!(saved, state.save_persist(), "replacement must have hit");

        let mut restored = SharedState::new(TextureFormat::Bgra8Unorm);
        restored.load_persist(&saved);
        assert_eq!(restored.view.node_style, lattice_scene::NodeStyle::Steady);
        assert_eq!(restored.camera.yaw, 1.23, "rest of the blob still restores");
    }

    #[test]
    fn removed_octave_styles_in_old_persist_blobs_load_as_dots() {
        // Petals/Flares and the merged Bumps no longer exist as variants;
        // serde aliases must absorb each so an old blob still restores
        // rather than dropping the whole persist. Inject the dead tokens
        // as strings (the enum can't name them anymore).
        for removed in ["Petals", "Flares", "Bumps"] {
            let mut state = SharedState::new(TextureFormat::Bgra8Unorm);
            state.view.outer_style = lattice_scene::OuterStyle::Slices;
            let saved = state
                .save_persist()
                .replace("outer_style:Slices", &format!("outer_style:{removed}"));
            assert_ne!(saved, state.save_persist(), "replacement must have hit for {removed}");

            let mut restored = SharedState::new(TextureFormat::Bgra8Unorm);
            restored.load_persist(&saved);
            assert_eq!(
                restored.view.outer_style,
                lattice_scene::OuterStyle::Dots,
                "{removed} folds to Dots"
            );
        }
    }

    #[test]
    fn pre_rename_octave_style_and_slice_band_fields_still_load() {
        // The outer layer's fields were renamed (octave_style ->
        // outer_style, slice_inner/outer -> outer_inner/outer); aliases
        // must keep blobs with the old names loading.
        let mut state = SharedState::new(TextureFormat::Bgra8Unorm);
        state.view.outer_style = lattice_scene::OuterStyle::Slices;
        state.view.outer_inner = 0.25;
        state.view.outer_outer = 0.85;
        let saved = state
            .save_persist()
            .replace("outer_style:", "octave_style:")
            .replace("outer_inner:", "slice_inner:")
            .replace("outer_outer:", "slice_outer:");
        assert_ne!(saved, state.save_persist(), "replacements must have hit");

        let mut restored = SharedState::new(TextureFormat::Bgra8Unorm);
        restored.load_persist(&saved);
        assert_eq!(restored.view.outer_style, lattice_scene::OuterStyle::Slices);
        assert_eq!(restored.view.outer_inner, 0.25);
        assert_eq!(restored.view.outer_outer, 0.85);
    }

    #[test]
    fn pre_radius_off_core_modes_fold_onto_radius_and_solidity() {
        // Pre-radius-off blobs wrote a `core_style` token the current layout
        // no longer serializes; loading one must fold it into radius (0 =
        // off) + solidity so the look is preserved. Inject the dead token
        // ahead of `core_solidity` (the enum still deserializes it).
        for (token, off, solidity) in
            [("Orb", false, 1.0), ("Glow", false, 0.0), ("None", false, 0.0), ("Empty", true, 1.0)]
        {
            let state = SharedState::new(TextureFormat::Bgra8Unorm);
            let saved = state
                .save_persist()
                .replace("core_solidity:", &format!("core_style:{token},core_solidity:"));
            assert_ne!(saved, state.save_persist(), "injection must have hit for {token}");

            let mut restored = SharedState::new(TextureFormat::Bgra8Unorm);
            restored.load_persist(&saved);
            if off {
                assert_eq!(restored.view.core_radius, 0.0, "{token} folds to off");
            } else {
                assert!(restored.view.core_radius > 0.0, "{token} stays on");
                assert_eq!(restored.view.core_solidity, solidity, "{token}");
            }
        }
    }

    #[test]
    fn node_body_experiment_blobs_fold_into_core_and_outer() {
        // Blobs saved by the one-build NodeBody experiment carry a
        // node_body field the current layout no longer writes; loading one
        // must both parse and fold the body into the core/outer split
        // (Beads = the core glow, solidity 0, plus dots-on-a-hoop, i.e. Dots
        // with the backdrop). They wrote the legacy core_style:Orb.
        let state = SharedState::new(TextureFormat::Bgra8Unorm);
        let saved = state
            .save_persist()
            .replace("core_solidity:", "core_style:Orb,node_body:Beads,core_solidity:");
        assert_ne!(saved, state.save_persist(), "injection must have hit");

        let mut restored = SharedState::new(TextureFormat::Bgra8Unorm);
        restored.load_persist(&saved);
        assert_eq!(restored.view.core_solidity, 0.0, "octave-only body is the glow end");
        assert!(restored.view.core_radius > 0.0, "still on");
        assert_eq!(restored.view.outer_style, lattice_scene::OuterStyle::Dots);
        assert!(restored.view.outer_backdrop, "Beads' hoop rides the backdrop");
        assert_eq!(
            restored.view.node_body,
            lattice_scene::LegacyNodeBody::Disc,
            "shim consumed on load"
        );
    }

    #[test]
    fn corrupt_persist_is_ignored() {
        let mut state = SharedState::new(TextureFormat::Bgra8Unorm);
        let default_distance = state.camera.distance;
        state.load_persist("not json at all");
        assert_eq!(state.camera.distance, default_distance);
    }

    #[derive(Default)]
    struct RecordingBackend {
        sets: std::cell::RefCell<Vec<(params::ParamKey, f32)>>,
    }

    impl ParamBackend for RecordingBackend {
        fn get(&self, _key: params::ParamKey) -> f32 {
            0.0
        }
        fn set(&self, key: params::ParamKey, value: f32) {
            self.sets.borrow_mut().push((key, value));
        }
    }

    /// Drive the real root_ui (dock, hover, everything) with a synthetic wheel
    /// event over the lattice pane and return the camera distance after it.
    /// `modifiers` picks whether egui routes the wheel to a scroll delta (plain)
    /// or a zoom factor (COMMAND, egui's default zoom modifier).
    fn distance_after_wheel_over_lattice(modifiers: egui::Modifiers) -> (f32, f32) {
        let mut state = SharedState::new(TextureFormat::Bgra8Unorm);
        let backend = RecordingBackend::default();
        let start = state.camera.distance;

        let ctx = egui::Context::default();
        let screen = egui::Rect::from_min_size(egui::pos2(0.0, 0.0), egui::vec2(1000.0, 800.0));
        // A point solidly inside the top-left leaf, which holds the Lattice tab
        // alone (see default_dock): past the tab bar, left of the split.
        let over_lattice = egui::pos2(150.0, 150.0);

        let run_frame = |state: &mut SharedState, ctx: &egui::Context, t: f64, wheel: bool| {
            let mut events = vec![egui::Event::PointerMoved(over_lattice)];
            if wheel {
                events.push(egui::Event::MouseWheel {
                    unit: egui::MouseWheelUnit::Line,
                    // Positive y = scroll up = zoom in (both the scroll and the
                    // zoom-factor paths map an upward wheel to a smaller distance).
                    delta: egui::vec2(0.0, 1.0),
                    phase: egui::TouchPhase::Move,
                    modifiers,
                });
            }
            let raw = egui::RawInput {
                screen_rect: Some(screen),
                time: Some(t),
                events,
                ..Default::default()
            };
            let _ = ctx.run_ui(raw, |ui| root_ui(ui, state, &backend, t));
        };

        // Warm-up passes so the pointer registers and egui's top-widget-at-
        // pointer resolution (which reads the previous pass) sees the lattice
        // under the pointer before the wheel pass.
        run_frame(&mut state, &ctx, 0.0, false);
        run_frame(&mut state, &ctx, 1.0 / 60.0, false);
        run_frame(&mut state, &ctx, 2.0 / 60.0, true);

        (start, state.camera.distance)
    }

    /// Repro for "mouse-wheel scroll to zoom no longer works": a plain wheel over
    /// the lattice (egui delivers it as a scroll delta) must zoom in.
    #[test]
    fn scroll_over_lattice_zooms_the_camera() {
        let (start, after) = distance_after_wheel_over_lattice(egui::Modifiers::NONE);
        assert!(after < start, "plain scroll should zoom in ({start} -> {after})");
    }

    /// A wheel egui classifies as a zoom gesture (modifier+scroll / trackpad
    /// pinch) arrives as `zoom_delta`, not a scroll delta. The lattice must zoom
    /// on that too — the old handler only read the scroll delta and did nothing.
    #[test]
    fn zoom_gesture_over_lattice_zooms_the_camera() {
        let (start, after) = distance_after_wheel_over_lattice(egui::Modifiers::COMMAND);
        assert!(after < start, "zoom-gesture wheel should zoom in ({start} -> {after})");
    }

    #[test]
    fn learn_step_writes_params_only_when_the_chord_changes() {
        let mut state = SharedState::new(TextureFormat::Bgra8Unorm);
        let backend = RecordingBackend::default();
        state.learn_active = true;
        // Hold C and G (a 12-TET fifth: within learn range of just).
        for note in [60u8, 67] {
            state.tracker.handle_event(lattice_core::NoteEvent {
                time: 0.0,
                channel: 0,
                note,
                kind: lattice_core::NoteEventKind::On { velocity: 1.0 },
            });
        }

        learn_step(&mut state, &backend);
        let first = backend.sets.borrow().clone();
        assert!(
            first.iter().any(|(k, v)| *k == params::ParamKey::Three && *v == 700.0),
            "the fifth should be learned from C+G, got {first:?}"
        );

        // Same chord again: change detection must suppress further writes.
        learn_step(&mut state, &backend);
        assert_eq!(backend.sets.borrow().len(), first.len());

        // Disarming clears the memory so re-arming re-learns.
        state.learn_active = false;
        learn_step(&mut state, &backend);
        state.learn_active = true;
        learn_step(&mut state, &backend);
        assert_eq!(backend.sets.borrow().len(), first.len() * 2);
    }

    /// Hold `notes` as channel-0 voices, each optionally bent by a per-note
    /// tuning offset (cents). Used to synthesize just vs 12-TET chords.
    fn hold_chord(state: &mut SharedState, notes: &[(u8, f32)]) {
        for &(note, cents) in notes {
            state.tracker.handle_event(lattice_core::NoteEvent {
                time: 0.0,
                channel: 0,
                note,
                kind: lattice_core::NoteEventKind::On { velocity: 1.0 },
            });
            if cents != 0.0 {
                state.tracker.handle_event(lattice_core::NoteEvent {
                    time: 0.0,
                    channel: 0,
                    note,
                    kind: lattice_core::NoteEventKind::Tuning { semitones: cents / 100.0 },
                });
            }
        }
    }

    #[test]
    fn learn_enables_meantone_from_a_12tet_triad() {
        let mut state = SharedState::new(TextureFormat::Bgra8Unorm);
        let backend = RecordingBackend::default();
        state.learn_active = true;
        // Plain 12-TET C-E-G pins a 700¢ fifth and a 400¢ third; since
        // 400 = 4·700 − 2400 this triad IS a meantone.
        hold_chord(&mut state, &[(60, 0.0), (64, 0.0), (67, 0.0)]);
        learn_step(&mut state, &backend);
        assert!(state.view.meantone, "a 12-TET triad should engage meantone");
    }

    #[test]
    fn learn_disables_meantone_from_a_just_triad() {
        let mut state = SharedState::new(TextureFormat::Bgra8Unorm);
        let backend = RecordingBackend::default();
        state.learn_active = true;
        state.view.meantone = true; // start engaged
        // C + a JUST major third (386.31¢) + G. The just third sits a full
        // syntonic comma below four fifths, so this is not a meantone.
        let just_offset = lattice_core::tuning::FIVE_JUST - 400.0;
        hold_chord(&mut state, &[(60, 0.0), (64, just_offset), (67, 0.0)]);
        learn_step(&mut state, &backend);
        assert!(!state.view.meantone, "a just third should release meantone");
    }

    #[test]
    fn learn_leaves_meantone_unchanged_without_a_third() {
        let mut state = SharedState::new(TextureFormat::Bgra8Unorm);
        let backend = RecordingBackend::default();
        state.learn_active = true;
        state.view.meantone = true;
        // A bare fifth fixes no third, so the meantone flag is left alone.
        hold_chord(&mut state, &[(60, 0.0), (67, 0.0)]);
        learn_step(&mut state, &backend);
        assert!(state.view.meantone, "a bare fifth shouldn't change the flag");
    }

    #[test]
    fn audio_spectrum_shows_while_flowing_and_hides_after() {
        let mut spectrum = AudioSpectrum::default();
        let config = SpectrumConfig::default();
        assert!(spectrum.display(0.0, &config).is_none(), "no audio yet");

        // A 440 Hz sine, long enough to fill the analysis window.
        let sine: Vec<f32> = (0..9_000)
            .map(|i| 0.5 * (std::f32::consts::TAU * 440.0 * i as f32 / 48_000.0).sin())
            .collect();
        spectrum.push_samples(&sine, 48_000.0, 1.0);
        let (levels, _peaks) = spectrum.display(1.0, &config).expect("audio is flowing");
        let peak = levels
            .iter()
            .enumerate()
            .max_by(|a, b| a.1.total_cmp(b.1))
            .map(|(i, _)| i as i32)
            .unwrap();
        assert!((peak - 114).abs() <= 1, "440 Hz should peak at A4 (bucket 114), got {peak}");

        // Once samples stop, the curve hides instead of freezing.
        assert!(spectrum.display(1.0 + AudioSpectrum::HOLD_SECONDS + 0.1, &config).is_none());
    }

    #[test]
    fn spectrum_config_round_trips_through_persist() {
        let mut state = SharedState::new(TextureFormat::Bgra8Unorm);
        state.spectrum_config.show_audio = true;
        state.spectrum_config.floor_db = -48.0;
        state.spectrum_config.window = SpectrumWindow::Precise;
        state.spectrum_config.low_octave = 1;
        let saved = state.save_persist();

        let mut restored = SharedState::new(TextureFormat::Bgra8Unorm);
        restored.load_persist(&saved);
        assert!(restored.spectrum_config.show_audio);
        assert_eq!(restored.spectrum_config.floor_db, -48.0);
        assert_eq!(restored.spectrum_config.window, SpectrumWindow::Precise);
        assert_eq!(restored.spectrum_config.low_octave, 1);
    }
}
