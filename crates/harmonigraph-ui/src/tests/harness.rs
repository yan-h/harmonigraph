//! Fixtures more than one suite needs: a recording [`ParamBackend`], the
//! [`DockHarness`] that drives a whole dock through egui frames, and the
//! settings-pane painters the width and scale suites both measure.

use crate::*;
use harmonigraph_render::wgpu::TextureFormat;

#[derive(Default)]
pub(super) struct RecordingBackend {
    pub(super) sets: std::cell::RefCell<Vec<(params::ParamKey, f32)>>,
}

impl ParamBackend for RecordingBackend {
    fn get(&self, _key: params::ParamKey) -> f32 {
        0.0
    }
    fn set(&self, key: params::ParamKey, value: f32) {
        self.sets.borrow_mut().push((key, value));
    }
}

/// A harness that runs the REAL dock — `root_ui`, egui_dock, tab bodies and
/// all — one frame per call, so a pane's pointer handling is tested through
/// every layer that sits between it and the mouse.
pub(super) struct DockHarness {
    pub(super) ctx: egui::Context,
    backend: RecordingBackend,
    pub(super) screen: egui::Rect,
    t: f64,
}

impl DockHarness {
    pub(super) fn new() -> Self {
        DockHarness {
            ctx: egui::Context::default(),
            backend: RecordingBackend::default(),
            screen: egui::Rect::from_min_size(egui::pos2(0.0, 0.0), egui::vec2(1000.0, 800.0)),
            t: 0.0,
        }
    }

    pub(super) fn frame(&mut self, state: &mut SharedState, events: Vec<egui::Event>) -> egui::FullOutput {
        self.t += 1.0 / 60.0;
        let raw = egui::RawInput {
            screen_rect: Some(self.screen),
            time: Some(self.t),
            events,
            ..Default::default()
        };
        let t = self.t;
        let backend = &self.backend;
        self.ctx.run_ui(raw, |ui| root_ui(ui, state, backend, t))
    }

    /// Answer a sideways fold's resize the way a shell does — the window it
    /// asked for, never below the floor it holds (see `fold`). Without this
    /// the harness is a host that refuses every resize, which is a state the
    /// fold layout has its own handling for.
    pub(super) fn resize(&mut self, state: &mut SharedState) {
        if let Some(change) = state.take_window_width_change() {
            let width = (self.screen.width() + change).max(state.min_window_width);
            self.screen.max.x = self.screen.min.x + width;
        }
    }

    /// Frames until a fold has the window it asked for. A fold is a two-step —
    /// the frame that asks and the frame drawn at the size it was given — and
    /// one fold can release another, so this runs a few.
    pub(super) fn settle_folds(&mut self, state: &mut SharedState) -> egui::FullOutput {
        let mut output = None;
        for _ in 0..4 {
            self.resize(state);
            output = Some(self.frame(state, vec![]));
        }
        output.expect("a settled frame")
    }

    /// A click on the collapse arrow of the leaf holding `tab`, settled.
    ///
    /// The ARROW, not the tab name: egui_dock reaches `set_collapsed` from its
    /// own square at the left end of the tab bar, and clicking the title only
    /// selects a tab.
    pub(super) fn collapse_click(&mut self, state: &mut SharedState, tab: panes::Tab) -> egui::FullOutput {
        let path = state.dock.find_tab(&tab).expect("tab is in the dock");
        let rect = state.dock[path.surface][path.node].rect().expect("the leaf is laid out");
        let at = rect.left_top() + egui::vec2(12.0, crate::theme::TAB_BAR_HEIGHT * 0.5);
        self.frame(state, vec![egui::Event::PointerMoved(at)]);
        self.frame(state, vec![egui::Event::PointerMoved(at), press(at, true)]);
        self.frame(state, vec![press(at, false)]);
        self.settle_folds(state)
    }

    /// Two warm-ups: egui resolves the top widget at the pointer from the
    /// previous pass, so a widget has to exist before the press.
    pub(super) fn settle(&mut self, state: &mut SharedState) {
        self.frame(state, vec![]);
        self.frame(state, vec![]);
    }

    /// A point inside the Spectral pane's picture, mid-pitch and deep into the
    /// roll/spectrogram region — clear of the divider, which sits at 45% of
    /// the depth axis by default and would otherwise take the drag.
    pub(super) fn spectral_grab(&self, state: &SharedState) -> egui::Pos2 {
        self.spectral_grab_at(state, 0.8)
    }

    /// The same, at a chosen fraction along the depth (time) axis — which side
    /// of the divider a drag starts on decides whether it is the Span's.
    /// Left, the default orientation, runs depth rightward.
    pub(super) fn spectral_grab_at(&self, state: &SharedState, depth: f32) -> egui::Pos2 {
        // Asked of the Spectral pane BY NAME, so a dock that has taken it off
        // screen trips this rather than aiming the drag somewhere else.
        // `perf_overlay_area` answers the same question and is the wrong
        // oracle for it: it now falls back to the Lattice pane's body, which
        // is a perfectly good rect that a drag orbits the camera in — the
        // grab would land in the wrong pane and the test would fail three
        // asserts later, naming the analyzer.
        let rect = crate::pane_body(state, &panes::Tab::Spectral)
            .expect("the Spectral pane should be visible in the default dock");
        rect.lerp_inside(egui::vec2(depth, 0.5))
    }
}

pub(super) fn press(pos: egui::Pos2, pressed: bool) -> egui::Event {
    egui::Event::PointerButton {
        pos,
        button: egui::PointerButton::Primary,
        pressed,
        modifiers: egui::Modifiers::default(),
    }
}

/// The projections a settings sweep has to cover, default first.
///
/// Only the Tuning pane's content turns on this, and it turns on it hard:
/// `frame_controls` hides the whole camera-angle half — Camera yaw and pitch,
/// the Angle presets, the Save-angle row — under Cabinet, which has a fixed
/// viewpoint and no angle to set, and hides the two cabinet knobs under the
/// others. `Camera::default()` IS Cabinet, so a fixture that takes the default
/// and stops there never draws that half of the pane at all.
pub(super) const PROJECTIONS: [harmonigraph_scene::Projection; 3] = [
    harmonigraph_scene::Projection::Cabinet,
    harmonigraph_scene::Projection::Perspective,
    harmonigraph_scene::Projection::Orthographic,
];

/// Every settings tab, and the tabs that share the column with them.
pub(super) const SETTINGS_TABS: [panes::Tab; 8] = [
    panes::Tab::Tuning,
    panes::Tab::Nodes,
    panes::Tab::Scene,
    panes::Tab::Analyzer,
    panes::Tab::Video,
    panes::Tab::Panel,
    panes::Tab::Console,
    panes::Tab::Notes,
];

/// One settings pane whose content box is `width` points wide, as the shapes it
/// emitted. Driven through [`panes::Viewer`] rather than the dock, so a sweep
/// over widths costs one pane each instead of a whole window, and the width
/// under test is the pane's own rather than a window size minus chrome.
///
/// The dock's nesting IS reproduced, though, because the one thing it does that
/// a bare `Ui` does not is the thing these tests are about: egui_dock clips the
/// tab body to the whole body rect and only THEN insets it by
/// `tab_body.inner_margin` via a `Frame`, which does not clip. So a pane's clip
/// rect sits a margin's width OUTSIDE its content box, and a harness without
/// the margin cannot tell a control clamped to the content box from one clamped
/// to the painted edge — they are the same number there.
///
/// Tall on purpose (a pane's controls are a column, and the point here is the
/// other axis) and with the take controls switched on — and a render in
/// flight — so the Video tab draws the record button, the Options field, and
/// the progress bar a real session has.
pub(super) fn settings_pane_at_width(
    tab: panes::Tab,
    width: f32,
    projection: harmonigraph_scene::Projection,
) -> Vec<egui::epaint::ClippedShape> {
    let mut state = SharedState::new(TextureFormat::Bgra8Unorm);
    state.take.supported = true;
    state.take.last_ready = true;
    state.take.render_progress = Some(FIXTURE_RENDER);
    state.camera.projection = projection;
    // A saved angle, so the Angle row has the button a real session gives it.
    state.camera_presets.push(CameraPreset { name: "Front".into(), yaw: 0.0, pitch: 0.0 });
    let backend = RecordingBackend::default();
    let ctx = egui::Context::default();
    crate::theme::apply_theme(&ctx);
    let margin = crate::theme::PANE_INNER_MARGIN;
    let body = egui::Rect::from_min_size(
        egui::Pos2::ZERO,
        egui::vec2(width + 2.0 * margin, 2400.0),
    );
    let out = ctx.run_ui(
        egui::RawInput { screen_rect: Some(body), time: Some(0.0), ..Default::default() },
        |ui| {
            // The body ui's clip is the whole body (the screen here); the pane
            // ui inside it is inset, exactly as the dock's Frame leaves it.
            let mut pane =
                ui.new_child(egui::UiBuilder::new().max_rect(body.shrink(margin)));
            let mut tab = tab;
            let mut viewer = panes::Viewer { state: &mut state, params: &backend, now: 0.0 };
            egui_dock::TabViewer::ui(&mut viewer, &mut pane, &mut tab);
        },
    );
    out.shapes
}

/// The projections worth drawing `tab` at: all of them for Tuning, whose
/// content depends on it (see [`PROJECTIONS`]), and the default alone for the
/// panes that draw the same thing either way.
pub(super) fn projections_for(tab: panes::Tab) -> &'static [harmonigraph_scene::Projection] {
    if tab == panes::Tab::Tuning { &PROJECTIONS } else { &PROJECTIONS[..1] }
}

/// The bar tracks a pane drew, by width. A `ValueBar`/`RangeBar` track is the
/// one thing in a settings pane painted as a `BAR_HEIGHT`-tall rect in
/// `well()`: the accent fill over it is the same height in a different color,
/// and the record button's own `well()` panel is taller.
/// The render the pane fixtures have in flight, so the Video pane's progress
/// bar is drawn in every sweep over the settings panes rather than only in the
/// test below — it takes the column's width like every other bar, and that is
/// what the sweeps are for. The two digits of `done` against three of `total`
/// also put the padded readout through them.
pub(super) const FIXTURE_RENDER: RenderProgress = RenderProgress { done: 120, total: 990 };

/// Whether the leaf holding `tab` is folded away.
pub(super) fn collapsed(state: &SharedState, tab: panes::Tab) -> bool {
    let path = state.dock.find_tab(&tab).expect("tab is in the dock");
    state.dock[path.surface][path.node].is_collapsed()
}
