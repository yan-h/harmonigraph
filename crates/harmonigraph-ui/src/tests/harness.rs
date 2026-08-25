//! Fixtures more than one suite needs: a recording [`ParamBackend`], the
//! [`DockHarness`] that drives a whole dock through egui frames, and the
//! settings-pane painters the width and scale suites both measure.
//!
//! Everything here needs a whole dock. What a single pane or widget needs is
//! in [`super::probe`], and [`press`] is re-exported from there so a suite
//! reaching for the harness gets both.

use crate::*;

pub(super) use super::probe::{fresh, press};

/// A docked tab's drawn surface, or `None` when it is not on screen — which
/// is how a suite aims a gesture at a pane it can only name.
///
/// `viewport` is the tab BODY; the picture panes drop their margin, so it is
/// exactly the drawn surface. `Rect::NOTHING` until the dock has laid out once
/// (a first frame, or a freshly loaded layout) — and STALE while the leaf is
/// collapsed, which is why the flag is checked rather than the rect: a folded
/// leaf keeps the viewport it had when it was open.
pub(super) fn pane_body(state: &SharedState, tab: &panes::Tab) -> Option<egui::Rect> {
    let path = state.workspace.dock.find_tab(tab)?;
    let egui_dock::Node::Leaf(leaf) = &state.workspace.dock[path.surface][path.node] else {
        return None;
    };
    (!leaf.collapsed && leaf.active == path.tab && leaf.viewport.is_positive())
        .then_some(leaf.viewport)
}

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

/// The window every dock fixture opens in unless it is about some other size:
/// wide enough that the default layout's four leaves are all real panes rather
/// than slivers, and the size the pane coordinates written into these suites
/// (a point inside the top-left leaf, the settings column at x ~700..1000) are
/// read against.
pub(super) const DEFAULT_WINDOW: egui::Vec2 = egui::vec2(1000.0, 800.0);

impl DockHarness {
    pub(super) fn new() -> Self {
        DockHarness::at(DEFAULT_WINDOW)
    }

    /// A harness whose window is `size`, for the suites that are about a
    /// particular one — the plugin's narrowest editor, a window short enough
    /// that a settings pane overflows it.
    pub(super) fn at(size: egui::Vec2) -> Self {
        // The real faces and sizes, like every other fixture that measures a
        // width (`settings_pane_at_width`, `settings_pane_at_scale`, and the
        // spectral ones that say "the real Iosevka metrics" outright). `root_ui`
        // does NOT install them for us: its only style hook is `set_ui_scale`,
        // which at scale 1.0 on a context that never had the theme returns
        // early rather than applying one. So a harness that skips this lays
        // every string out in egui's 12.5pt fallback, and anything measuring
        // text is measuring the wrong font — which is the whole job of
        // `every_settings_tab_fits_on_its_tab_bar`.
        DockHarness {
            ctx: super::probe::themed(),
            backend: RecordingBackend::default(),
            screen: egui::Rect::from_min_size(egui::pos2(0.0, 0.0), size),
            t: 0.0,
        }
    }

    /// The same with the chrome dialled to `scale`, which is the UI-scale
    /// setting rather than the device's pixel ratio: it resizes every font,
    /// margin and bar in the dock, so a window has to be scaled with it or the
    /// sweep is also a sweep over how much overflows.
    ///
    /// The STATE is where the scale really lives, which is why this takes one:
    /// `root_ui` calls `set_ui_scale` from `state.ui_scale` on every frame, so
    /// a context dialled up on its own is reset by the first frame drawn on it
    /// and the sweep silently measures the design size at every step. Dialling
    /// the context as well is what puts the first frame at the scale rather
    /// than one frame behind it.
    pub(super) fn scaled(size: egui::Vec2, scale: f32, state: &mut SharedState) -> Self {
        state.ui_scale = scale;
        let mut harness = DockHarness::at(size);
        harness.ctx = super::probe::themed_scaled(scale);
        harness
    }

    pub(super) fn frame(
        &mut self,
        state: &mut SharedState,
        events: Vec<egui::Event>,
    ) -> egui::FullOutput {
        self.frame_timed(state, events).0
    }

    /// The clock the NEXT frame will run at, so a fixture that feeds the state
    /// before drawing it — audio into the analyzer, note-ons into the tracker —
    /// can stamp them at the time the frame will read them. Feeding at some
    /// other clock puts the picture one frame away from its own input, which
    /// for the spectrogram is a column of black down the now-line.
    pub(super) fn next_time(&self) -> f64 {
        self.t + 1.0 / 60.0
    }

    /// One frame, and the milliseconds spent inside `root_ui` itself.
    ///
    /// Timed from INSIDE the pass, so egui's own begin/end work stays outside
    /// the reading and the number is the same slice of the frame the perf
    /// overlay's `ui` row reports. Every frame pays the two `Instant::now`
    /// calls, which is nothing beside a frame and keeps one code path rather
    /// than a timed harness beside an untimed one.
    pub(super) fn frame_timed(
        &mut self,
        state: &mut SharedState,
        events: Vec<egui::Event>,
    ) -> (egui::FullOutput, f64) {
        self.t += 1.0 / 60.0;
        let raw = egui::RawInput {
            screen_rect: Some(self.screen),
            time: Some(self.t),
            events,
            ..Default::default()
        };
        let t = self.t;
        let backend = &self.backend;
        let mut ui_ms = 0.0;
        let output = self.ctx.run_ui(raw, |ui| {
            let start = std::time::Instant::now();
            root_ui(ui, state, backend, t);
            ui_ms = start.elapsed().as_secs_f64() * 1000.0;
        });
        (output, ui_ms)
    }

    /// Answer a sideways fold's resize the way a shell does — the window it
    /// asked for, never below the floor it holds (see `fold`). Without this
    /// the harness is a host that refuses every resize, which is a state the
    /// fold layout has its own handling for.
    pub(super) fn resize(&mut self, state: &mut SharedState) {
        if let Some(change) = state.workspace.take_window_width_change() {
            let width = (self.screen.width() + change).max(state.workspace.min_window_width);
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
    pub(super) fn collapse_click(
        &mut self,
        state: &mut SharedState,
        tab: panes::Tab,
    ) -> egui::FullOutput {
        let path = state.workspace.dock.find_tab(&tab).expect("tab is in the dock");
        let leaf = &state.workspace.dock[path.surface][path.node];
        let rect = leaf.rect().expect("the leaf is laid out");
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
        // screen trips this rather than aiming the drag somewhere else — at
        // the Lattice's body, say, which is a perfectly good rect that a drag
        // orbits the camera in, so the grab would land in the wrong pane and
        // the test would fail three asserts later, naming the analyzer.
        let rect = pane_body(state, &panes::Tab::Spectral)
            .expect("the Spectral pane should be visible in the default dock");
        rect.lerp_inside(egui::vec2(depth, 0.5))
    }
}

/// The projections a settings sweep has to cover, default first.
///
/// Only the Camera section's content turns on this, and it turns on it hard:
/// `view_pane` hides the whole camera-angle half — Camera yaw and pitch, the
/// Angle presets, the Save-angle row — under Cabinet, which has a fixed
/// viewpoint and no angle to set, and hides the two cabinet knobs under the
/// others. `Camera::default()` IS Cabinet, so a fixture that takes the default
/// and stops there never draws that half of the pane at all.
pub(super) const PROJECTIONS: [harmonigraph_scene::Projection; 3] = [
    harmonigraph_scene::Projection::Cabinet,
    harmonigraph_scene::Projection::Perspective,
    harmonigraph_scene::Projection::Orthographic,
];

pub(super) use crate::panes::display::DisplayPage;

/// One pane of the settings column as a sweep drives it: a tab of its own, or
/// one of the Display tab's pages, reached through Display's dispatch with that
/// page SELECTED — so every per-page count (short bars, gradient previews)
/// counts one page's, and each body is measured at the column's full width.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(super) enum SettingsPane {
    Tab(panes::Tab),
    Page(DisplayPage),
}

impl SettingsPane {
    /// The tab that carries this pane — with the page that shows it selected,
    /// which for a `Page` is the whole difference between measuring the pane
    /// and measuring whichever page was showing.
    pub(super) fn install(self, state: &mut SharedState) -> panes::Tab {
        match self {
            SettingsPane::Tab(tab) => tab,
            SettingsPane::Page(page) => {
                state.display_page = page;
                panes::Tab::Display
            }
        }
    }
}

/// Every settings pane, and the tabs that share the column with them.
pub(super) const SETTINGS_PANES: [SettingsPane; 8] = [
    SettingsPane::Tab(panes::Tab::Tuning),
    SettingsPane::Page(DisplayPage::Colors),
    SettingsPane::Page(DisplayPage::Lattice),
    SettingsPane::Page(DisplayPage::Analyzer),
    SettingsPane::Page(DisplayPage::System),
    SettingsPane::Tab(panes::Tab::Video),
    SettingsPane::Tab(panes::Tab::Console),
    SettingsPane::Tab(panes::Tab::Notes),
];

/// One settings pane whose content box is `width` points wide, as the shapes it
/// emitted. Driven through [`panes::Viewer`] rather than the dock, so a sweep
/// over widths costs one pane each instead of a whole window, and the width
/// under test is the pane's own rather than a window size minus chrome. A
/// [`SettingsPane::Page`] goes through the Display tab's own dispatch with that
/// page selected, so its body is measured under the real picker row.
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
    pane: SettingsPane,
    width: f32,
    projection: harmonigraph_scene::Projection,
) -> Vec<egui::epaint::ClippedShape> {
    let mut state = fresh();
    state.take.supported = true;
    state.take.last_ready = true;
    state.take.render_progress = Some(FIXTURE_RENDER);
    state.camera.projection = projection;
    // A saved angle, so the Angle row has the button a real session gives it.
    state.camera_presets.push(CameraPreset { name: "Front".into(), yaw: 0.0, pitch: 0.0 });
    let tab = pane.install(&mut state);
    tab_body(&mut state, tab, width, PANE_HEIGHT).shapes
}

/// The height every pane fixture is drawn at: taller than any settings pane's
/// content, so a column that reaches the bottom is the pane running out of
/// controls rather than out of window.
pub(super) const PANE_HEIGHT: f32 = 2400.0;

/// One tab's body painted into a content box `width` points across on a themed
/// context of its own, and the frame it produced.
pub(super) fn tab_body(
    state: &mut SharedState,
    tab: panes::Tab,
    width: f32,
    height: f32,
) -> egui::FullOutput {
    tab_body_on(&super::probe::themed(), state, tab, width, height, 0.0)
}

/// The same on a caller's context and clock — for a fixture that drives many
/// frames and wants one context across them.
pub(super) fn tab_body_on(
    ctx: &egui::Context,
    state: &mut SharedState,
    tab: panes::Tab,
    width: f32,
    height: f32,
    now: f64,
) -> egui::FullOutput {
    let backend = RecordingBackend::default();
    // The inset at the CONTEXT's chrome scale rather than at the design size,
    // so a fixture that scales the chrome measures the pane the dock would
    // actually give it — the margin scales with everything else.
    let margin = crate::theme::pane_inner_margin(crate::theme::ui_scale(ctx));
    let body =
        egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(width + 2.0 * margin, height));
    ctx.run_ui(
        egui::RawInput { screen_rect: Some(body), time: Some(now), ..Default::default() },
        |ui| {
            // The body ui's clip is the whole body (the screen here); the pane
            // ui inside it is inset, exactly as the dock's Frame leaves it.
            let mut body_ui = ui.new_child(egui::UiBuilder::new().max_rect(body.shrink(margin)));
            let mut tab = tab;
            let mut viewer = panes::Viewer { state, params: &backend, now };
            egui_dock::TabViewer::ui(&mut viewer, &mut body_ui, &mut tab);
        },
    )
}

/// The projections worth drawing `pane` at: all of them for the Lattice page,
/// whose Camera section depends on it (see [`PROJECTIONS`]), and the default
/// alone for the panes that draw the same thing either way.
pub(super) fn projections_for(pane: SettingsPane) -> &'static [harmonigraph_scene::Projection] {
    if pane == SettingsPane::Page(DisplayPage::Lattice) {
        &PROJECTIONS
    } else {
        &PROJECTIONS[..1]
    }
}

/// The render the pane fixtures have in flight, so the Video pane's progress
/// bar is drawn in every sweep over the settings panes rather than only in the
/// test below — it takes the column's width like every other bar, and that is
/// what the sweeps are for. The two digits of `done` against three of `total`
/// also put the padded readout through them.
pub(super) const FIXTURE_RENDER: RenderProgress = RenderProgress { done: 120, total: 990 };

/// Whether the leaf holding `tab` is folded away.
pub(super) fn collapsed(state: &SharedState, tab: panes::Tab) -> bool {
    let path = state.workspace.dock.find_tab(&tab).expect("tab is in the dock");
    state.workspace.dock[path.surface][path.node].is_collapsed()
}
