//! The shared UI shell: dockable panes, cross-pane hover state, and the
//! per-frame root function. Both the standalone harness and the plugin
//! editor call [`root_ui`] once per egui frame; everything else is internal.

pub mod params;
pub mod theme;
pub mod widgets;
mod panes;

use std::collections::VecDeque;

use egui_dock::{DockArea, DockState, NodeIndex};
use lattice_core::{LatticePos, NoteTracker, Tuning};
use lattice_render::wgpu::TextureFormat;
use lattice_scene::{Camera, ViewConfig};
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
    pub camera: Camera,
    /// The pitch-class node the pointer is over, if any — shared so *every*
    /// pane can highlight it (lattice glow, tuning pane readout, ...).
    pub hovered: Option<LatticePos>,
    pub console: Console,
    /// Surface format of the shell's swapchain; the lattice render pipeline
    /// must match it.
    pub target_format: TextureFormat,
    dock: DockState<panes::Tab>,
}

impl SharedState {
    pub fn new(target_format: TextureFormat) -> Self {
        // Default layout: big lattice view, tuning on the right, console and
        // spectral stub tucked below it. Users can re-dock at runtime.
        // TODO: persist the layout (DockState is serde-serializable).
        let mut dock = DockState::new(vec![panes::Tab::Lattice]);
        let surface = dock.main_surface_mut();
        let [_, right] = surface.split_right(NodeIndex::root(), 0.72, vec![panes::Tab::Settings]);
        surface.split_below(right, 0.55, vec![panes::Tab::Console, panes::Tab::Spectral, panes::Tab::Notes]);

        SharedState {
            tracker: NoteTracker::new(),
            tuning: Tuning::default(),
            view: ViewConfig::default(),
            camera: Camera::default(),
            hovered: None,
            console: Console::default(),
            target_format,
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
    state.tuning = params::tuning_from_params(params);
    state.view.pitch_class_fade_time = params.get(params::ParamKey::PitchClassFade);
    state.view.octave_fade_time = params.get(params::ParamKey::OctaveFade);
    state.view.darkest_pitch = params.get(params::ParamKey::DarkestPitch);
    state.view.brightest_pitch = params.get(params::ParamKey::BrightestPitch);
    // Voices must outlive the LONGER of the two fades or the octave
    // indicators get truncated when the note highlight ends first.
    state.tracker.prune(
        now,
        state.view.pitch_class_fade_time.max(state.view.octave_fade_time),
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
    // decaying voices); otherwise poll at 20 Hz so newly arriving MIDI
    // still shows up promptly. egui repaints on input events by itself,
    // so interaction never waits on this. The plugin shell additionally
    // requests a repaint the moment it drains new note events.
    if state.tracker.voices().next().is_some() {
        ui.ctx().request_repaint();
    } else {
        ui.ctx()
            .request_repaint_after(std::time::Duration::from_millis(50));
    }
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
    }

    #[test]
    fn corrupt_persist_is_ignored() {
        let mut state = SharedState::new(TextureFormat::Bgra8Unorm);
        let default_distance = state.camera.distance;
        state.load_persist("not json at all");
        assert_eq!(state.camera.distance, default_distance);
    }
}
