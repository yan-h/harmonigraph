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
    dock: DockState<panes::Tab>,
}

impl SharedState {
    pub fn new(target_format: TextureFormat) -> Self {
        // Default layout: big lattice view, tuning on the right, console and
        // spectral stub tucked below it. Users can re-dock at runtime; the
        // result persists via UiPersist.
        let mut dock = DockState::new(vec![panes::Tab::Lattice]);
        let surface = dock.main_surface_mut();
        let [_, right] = surface.split_right(
            NodeIndex::root(),
            0.72,
            vec![panes::Tab::Tuning, panes::Tab::View, panes::Tab::Appearance],
        );
        surface.split_below(right, 0.55, vec![panes::Tab::Console, panes::Tab::Spectral, panes::Tab::Notes]);

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
            dock,
        }
    }

    pub fn log(&mut self, line: impl Into<String>) {
        self.console.log(line);
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
    // awareness of its own. The Five param is left untouched (inert while
    // the lock is on).
    if state.view.meantone {
        state.tuning.five = lattice_core::tuning::meantone_third(state.tuning.three);
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

    // DockState has to be moved out while panes borrow the rest of `state`.
    let mut dock = std::mem::replace(&mut state.dock, DockState::new(vec![]));
    DockArea::new(&mut dock)
        .style(theme::dock_style(ui.style()))
        // The pane set is fixed; closing/collapsing chrome is just noise.
        .show_close_buttons(false)
        .show_leaf_close_all_buttons(false)
        .show_leaf_collapse_buttons(false)
        .show_inside(ui, &mut panes::Viewer { state, params, now });
    state.dock = dock;

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
        state.view.octave_style = lattice_scene::OctaveStyle::TicksCaps;
        state.view.meantone = true;
        let saved = state.save_persist();

        let mut restored = SharedState::new(TextureFormat::Bgra8Unorm);
        restored.load_persist(&saved);
        assert_eq!(restored.camera.yaw, 1.23);
        assert_eq!(restored.camera.distance, 42.0);
        assert_eq!(restored.view.extent_sevens, 3);
        assert_eq!(
            restored.view.octave_style,
            lattice_scene::OctaveStyle::TicksCaps
        );
        assert!(restored.view.meantone);
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
}
