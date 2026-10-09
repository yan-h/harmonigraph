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
    state.workspace.layout_runtime.body(*tab)
}

#[derive(Default)]
pub(super) struct RecordingBackend {
    pub(super) sets: std::cell::RefCell<Vec<(params::ParamKey, f32)>>,
    /// The Hub's map view — which note retuning engine it runs, and the map
    /// it plays — or `None` for a backend with no engine selector, which the
    /// Tuning pane reads as Pass through.
    pub(super) maps: Option<crate::lattice_maps::MapView>,
}

impl ParamBackend for RecordingBackend {
    fn get(&self, _key: params::ParamKey) -> f32 {
        0.0
    }
    fn set(&self, key: params::ParamKey, value: f32) {
        self.sets.borrow_mut().push((key, value));
    }
    fn lattice_maps(&self) -> Option<crate::lattice_maps::MapView> {
        self.maps.clone()
    }
}

/// A Hub's map view with `engine` selected and no maps saved: what a fixture
/// that draws one engine's controls hands the Tuning pane.
pub(super) fn engine_view(
    engine: harmonigraph_core::lattice_map::TuningEngine,
) -> crate::lattice_maps::MapView {
    crate::lattice_maps::MapView {
        playback: crate::lattice_maps::MapPlayback { engine, ..Default::default() },
        offsets: Default::default(),
        followed: harmonigraph_core::LatticePos::ORIGIN,
        pending: false,
        names: crate::lattice_maps::MapDocument::default().names(),
        edit_shape: false,
        can_undo: false,
        full: false,
    }
}

/// A Hub playing Lattice Map with every control under it live: a saved map
/// selected and sounding, following moved off the offsets, Edit shape on with
/// an edit to undo, offsets off zero, every slot used, and the whole still
/// pending adoption — so each conditional row of `map_controls` is drawn.
pub(super) fn lattice_map_view() -> crate::lattice_maps::MapView {
    use harmonigraph_core::{lattice_map::Follow, LatticePos};
    let mut view = engine_view(harmonigraph_core::lattice_map::TuningEngine::LatticeMap);
    view.playback.map = Some(Default::default());
    view.playback.follow = Follow::ThirdsAndFifths;
    view.followed = LatticePos { threes: -2, fives: 1, sevens: 0 };
    view.offsets.fine = LatticePos { threes: -7, fives: 3, sevens: -1 };
    view.offsets.extension = LatticePos { threes: -1, fives: 0, sevens: 2 };
    view.pending = true;
    view.edit_shape = true;
    view.can_undo = true;
    view.full = true;
    view
}

/// A harness that runs the REAL dock — `root_ui`, the workspace, tab bodies and
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
    /// `root_ui` calls `set_ui_scale` from `state.workspace.interaction.ui_scale` on every frame, so
    /// a context dialled up on its own is reset by the first frame drawn on it
    /// and the sweep silently measures the design size at every step. Dialling
    /// the context as well is what puts the first frame at the scale rather
    /// than one frame behind it.
    pub(super) fn scaled(size: egui::Vec2, scale: f32, state: &mut SharedState) -> Self {
        state.workspace.interaction.ui_scale = scale;
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
            root_ui(ui, state, backend, t, "shell-tag-fixture @0123456");
            ui_ms = start.elapsed().as_secs_f64() * 1000.0;
        });
        (output, ui_ms)
    }

    /// Answer a sideways fold's resize the way a shell does — the window it
    /// asked for, never below the floor it holds (see `fold`). Without this
    /// the harness is a host that refuses every resize, which is a state the
    /// fold layout has its own handling for.
    pub(super) fn resize(&mut self, state: &mut SharedState) {
        if let Some(change) = state.workspace.take_window_size_change() {
            self.screen.max = self.screen.min
                + (self.screen.size() + change).max(state.workspace.min_window_size);
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
    /// Tab labels select a destination; the leading button folds its section.
    pub(super) fn collapse_click(
        &mut self,
        state: &mut SharedState,
        tab: panes::Tab,
    ) -> egui::FullOutput {
        let rect = state.workspace.layout_runtime.rects[workspace::Section::of(tab) as usize];
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
    /// Written for Left, which runs depth rightward — the orientation the
    /// tests that aim with it pin rather than inherit.
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
/// Only the Camera block's content turns on this, and it turns on it hard:
/// `view::camera` hides the whole camera-angle half — Camera yaw and pitch, the
/// Angle presets, the Save-angle row — under Cabinet, which has a fixed
/// viewpoint and no angle to set, and hides the two cabinet knobs under the
/// others. `Camera::default()` IS Cabinet, so a fixture that takes the default
/// and stops there never draws that half of the pane at all.
pub(super) const PROJECTIONS: [harmonigraph_scene::Projection; 3] = [
    harmonigraph_scene::Projection::Cabinet,
    harmonigraph_scene::Projection::Perspective,
    harmonigraph_scene::Projection::Orthographic,
];

/// Every settings tab, in the column's own order.
pub(super) const SETTINGS_PANES: &[panes::Tab] = crate::workspace::Section::Settings.tabs();

/// What one settings sweep case changes from a fresh state: one choice that
/// swaps which controls a page draws, with everything else left fresh.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(super) enum Variant {
    Fresh,
    /// The Lattice page's Camera block (see [`PROJECTIONS`]).
    Projection(harmonigraph_scene::Projection),
    /// The spectrogram's texture, whose own controls the Analyzer page draws.
    Texture(harmonigraph_scene::CloudStyle),
    /// The lattice glow's material, whose own controls the Lattice page draws.
    Material(harmonigraph_scene::LatticeMaterial),
    /// The Hub's note retuning on a backend that has an engine selector;
    /// Lattice Map plays [`lattice_map_view`].
    Engine(harmonigraph_core::lattice_map::TuningEngine),
}

/// One settings page drawn one way: what the layout sweeps iterate.
#[derive(Clone, Copy, Debug)]
pub(super) struct SettingsCase {
    pub(super) pane: panes::Tab,
    pub(super) variant: Variant,
}

impl SettingsCase {
    pub(super) fn fresh(pane: panes::Tab) -> Self {
        SettingsCase { pane, variant: Variant::Fresh }
    }

    /// The state the case draws, which is the fresh one with the take
    /// controls switched on — and a render in flight — so the Video tab draws
    /// the record button, the Options field, and the progress bar a real
    /// session has, and a saved camera angle, so the Angle row has the button
    /// a real session gives it.
    pub(super) fn state(self) -> SharedState {
        let mut state = fresh();
        state.workspace.interaction.take.supported = true;
        state.workspace.interaction.take.last_take = Some("music.take".into());
        state.workspace.interaction.take.exports = vec![fixture_export()];
        state.workspace.interaction.camera_presets.push(CameraPreset {
            name: "Front".into(),
            yaw: 0.0,
            pitch: 0.0,
        });
        let appearance = &mut state.picture.appearance;
        match self.variant {
            Variant::Fresh | Variant::Engine(_) => {}
            Variant::Projection(projection) => appearance.camera.projection = projection,
            Variant::Texture(style) => appearance.spectrum.atmosphere.cloud_style = style,
            Variant::Material(material) => appearance.view.atmosphere.material_style = material,
        }
        state
    }

    pub(super) fn backend(self) -> RecordingBackend {
        use harmonigraph_core::lattice_map::TuningEngine;
        let maps = match self.variant {
            Variant::Engine(TuningEngine::LatticeMap) => Some(lattice_map_view()),
            Variant::Engine(engine) => Some(engine_view(engine)),
            _ => None,
        };
        RecordingBackend { maps, ..Default::default() }
    }

    /// How many closed folds the page draws, which [`open_settings_pane`]
    /// has to open: Keyboard and Context under Adaptive tuning; Map offsets,
    /// Manage selected saved map and Assignments under Lattice Map; the
    /// Video page's Exports.
    pub(super) fn folds(self) -> usize {
        use harmonigraph_core::lattice_map::TuningEngine;
        match (self.pane, self.variant) {
            (panes::Tab::Tuning, Variant::Engine(TuningEngine::Adaptive)) => 2,
            (panes::Tab::Tuning, Variant::Engine(TuningEngine::LatticeMap)) => 3,
            (panes::Tab::Video, _) => 1,
            _ => 0,
        }
    }
}

/// Every way of drawing every settings page that changes which controls are
/// on it: each page fresh, then every texture on the Analyzer page, every
/// material and projection on the Lattice page, and every retuning engine on
/// the Tuning page.
///
/// Every variant rather than every one that differs from today's defaults,
/// because a default moving is how the sweeps lost the Stars editor, Scales
/// and the Adaptive section without a line of them changing (#1484). The
/// matches are exhaustive so a new texture, material or engine does not
/// compile until it is listed here.
pub(super) fn settings_cases() -> Vec<SettingsCase> {
    use harmonigraph_core::lattice_map::TuningEngine;
    use harmonigraph_scene::{CloudStyle, LatticeMaterial};
    use panes::Tab;
    let textures = [CloudStyle::Watercolor, CloudStyle::Stars, CloudStyle::VelvetScales];
    let materials = [
        LatticeMaterial::None,
        LatticeMaterial::Watercolor,
        LatticeMaterial::Stars,
        LatticeMaterial::VelvetScales,
    ];
    let engines = [TuningEngine::Off, TuningEngine::Adaptive, TuningEngine::LatticeMap];
    for texture in textures {
        match texture {
            CloudStyle::Watercolor | CloudStyle::Stars | CloudStyle::VelvetScales => {}
        }
    }
    for material in materials {
        match material {
            LatticeMaterial::None
            | LatticeMaterial::Watercolor
            | LatticeMaterial::Stars
            | LatticeMaterial::VelvetScales => {}
        }
    }
    for engine in engines {
        match engine {
            TuningEngine::Off | TuningEngine::Adaptive | TuningEngine::LatticeMap => {}
        }
    }
    let mut cases = Vec::new();
    for &pane in SETTINGS_PANES {
        let variants: Vec<Variant> = match pane {
            Tab::AnalyzerSettings => textures.map(Variant::Texture).to_vec(),
            // Every material at the fresh projection, then every other
            // projection at the fresh material: the Camera block and the
            // glow's material are independent blocks of the page.
            Tab::LatticeSettings => materials
                .map(Variant::Material)
                .into_iter()
                .chain(PROJECTIONS[1..].iter().map(|&p| Variant::Projection(p)))
                .collect(),
            // No engine selector at all, then each engine behind one.
            Tab::Tuning => {
                std::iter::once(Variant::Fresh).chain(engines.map(Variant::Engine)).collect()
            }
            _ => vec![Variant::Fresh],
        };
        cases.extend(variants.into_iter().map(|variant| SettingsCase { pane, variant }));
    }
    cases
}

/// One settings page whose content box is `width` points wide, as the shapes it
/// emitted, with every fold on it open (see [`open_settings_pane`]). Driven
/// through [`panes::Viewer`] rather than the dock, so a sweep over widths costs
/// one pane each instead of a whole window, and the width under test is the
/// pane's own rather than a window size minus chrome.
///
/// The dock's nesting IS reproduced, though, because the one thing it does that
/// a bare `Ui` does not is the thing these tests are about: the workspace clips the
/// tab body to the whole body rect and only THEN insets it by
/// `tab_body.inner_margin` via a `Frame`, which does not clip. So a pane's clip
/// rect sits a margin's width OUTSIDE its content box, and a harness without
/// the margin cannot tell a control clamped to the content box from one clamped
/// to the painted edge — they are the same number there.
///
/// Tall on purpose: a pane's controls are a column, and the point here is the
/// other axis.
pub(super) fn settings_pane_at_width(
    case: SettingsCase,
    width: f32,
) -> Vec<egui::epaint::ClippedShape> {
    open_settings_pane(&super::probe::themed(), case, &mut case.state(), width)
}

/// `case`'s page drawn on `ctx` into a content box `width` across, after
/// opening every fold on it, as the shapes the last frame emitted.
///
/// Opened the way a person opens them — a click on each closed fold's arrow
/// in turn, until none is left — because a subsection's fold lives in egui's
/// memory under an id only the pane can name, and the one switch that opens
/// every fold at once (`Memory::set_everything_is_visible`) also shows every
/// tooltip on the page, which is exactly what a geometry sweep must not draw.
/// The pointer leaves before the last frame for the same reason.
///
/// The arrows are found by their paint, which nothing else reports: egui
/// records a widget's kind only for AccessKit, where a fold header is a
/// `Button` like any other. So the count opened is held to
/// [`SettingsCase::folds`]: an arrow restyled past [`closed_fold`]'s reading
/// opens nothing, and that must fail here rather than leave every sweep
/// measuring a folded page.
pub(super) fn open_settings_pane(
    ctx: &egui::Context,
    case: SettingsCase,
    state: &mut SharedState,
    width: f32,
) -> Vec<egui::epaint::ClippedShape> {
    let backend = case.backend();
    let mut now = 0.0;
    let frame = |state: &mut SharedState, now: f64, events: Vec<egui::Event>| {
        tab_body_with(ctx, state, case.pane, (width, PANE_HEIGHT), now, &backend, events).shapes
    };
    let mut shapes = frame(state, now, vec![]);
    let mut opened = 0;
    while let Some(at) = closed_fold(&shapes) {
        // More than any page holds; a fold that will not stay open would
        // otherwise loop here for ever.
        assert!(opened < 16, "{case:?} still has a closed fold after opening sixteen");
        // egui resolves the widget under the pointer from the previous pass,
        // so the pointer arrives a frame before the press.
        for events in [
            vec![egui::Event::PointerMoved(at)],
            vec![egui::Event::PointerMoved(at), press(at, true)],
            vec![press(at, false)],
        ] {
            now += 1.0 / 60.0;
            frame(state, now, events);
        }
        // A second on, past the fold's opening animation.
        now += 1.0;
        shapes = frame(state, now, vec![egui::Event::PointerGone]);
        opened += 1;
    }
    assert_eq!(opened, case.folds(), "{case:?} opened {opened} folds at {width}pt");
    shapes
}

/// How far a subsection indents its body on `ctx`, whose chrome scale it
/// follows: a bar in an open subsection is the page's column less this.
pub(super) fn fold_indent(ctx: &egui::Context) -> f32 {
    let mut indent = 0.0;
    let _ = ctx.run_ui(Default::default(), |ui| indent = ui.spacing().indent);
    indent
}

/// The centre of the first closed fold's arrow in `shapes`: a three-point
/// path pointing right — two points one above the other and the third out to
/// their right, level with their middle — in the exact proportions of one of
/// the two painters that draw a fold's arrow. `widgets::paint_chevron`
/// strokes an open chevron twice as tall as it reaches, for our sections and
/// subsections; egui's `paint_default_icon` fills a triangle as tall as it
/// reaches, for its own `CollapsingHeader`. Exact, because other right-pointing
/// triangles share the page (a gradient's flip button); a restyled arrow
/// then fails [`open_settings_pane`]'s count rather than going unopened.
fn closed_fold(shapes: &[egui::epaint::ClippedShape]) -> Option<egui::Pos2> {
    shapes.iter().find_map(|cs| match &cs.shape {
        egui::Shape::Path(path) if path.points.len() == 3 => {
            let mut points = [path.points[0], path.points[1], path.points[2]];
            points.sort_by(|a, b| a.x.total_cmp(&b.x));
            let [a, b, tip] = points;
            let (reach, span) = (tip.x - a.x, (a.y - b.y).abs());
            let shape = if path.closed { reach } else { 2.0 * reach };
            let pointing_right = reach > 0.5
                && (a.x - b.x).abs() < 0.01
                && (tip.y - (a.y + b.y) / 2.0).abs() < 0.01
                && (span - shape).abs() < 0.01;
            pointing_right.then(|| egui::pos2((a.x + tip.x) / 2.0, tip.y))
        }
        _ => None,
    })
}

/// The height every pane fixture is drawn at: taller than any settings pane's
/// content, so a column that reaches the bottom is the pane running out of
/// controls rather than out of window. [`tab_body_on`] asserts it, because a
/// bar below the clip skips its paint and a page that outgrew this would
/// otherwise go quietly unchecked past it.
pub(super) const PANE_HEIGHT: f32 = 5600.0;

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

/// The same with the Hub's note retuning set to Adaptive, for a fixture that
/// measures the adaptive controls.
pub(super) fn adaptive_tab_body(
    state: &mut SharedState,
    tab: panes::Tab,
    width: f32,
    height: f32,
) -> egui::FullOutput {
    let backend = RecordingBackend {
        maps: Some(engine_view(harmonigraph_core::lattice_map::TuningEngine::Adaptive)),
        ..Default::default()
    };
    tab_body_with(&super::probe::themed(), state, tab, (width, height), 0.0, &backend, vec![])
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
    tab_body_with(ctx, state, tab, (width, height), now, &RecordingBackend::default(), vec![])
}

fn tab_body_with(
    ctx: &egui::Context,
    state: &mut SharedState,
    tab: panes::Tab,
    (width, height): (f32, f32),
    now: f64,
    backend: &RecordingBackend,
    events: Vec<egui::Event>,
) -> egui::FullOutput {
    // The inset at the CONTEXT's chrome scale rather than at the design size,
    // so a fixture that scales the chrome measures the pane the dock would
    // actually give it — the margin scales with everything else.
    let margin = crate::theme::dock_pane_margin(crate::theme::ui_scale(ctx));
    let body =
        egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(width + margin.sum().x, height));
    ctx.run_ui(
        egui::RawInput { screen_rect: Some(body), time: Some(now), events, ..Default::default() },
        |ui| {
            // The body ui's clip is the whole body (the screen here); the pane
            // ui inside it is inset, exactly as the dock's Frame leaves it.
            let mut body_ui = ui.new_child(egui::UiBuilder::new().max_rect(body - margin));
            let mut tab = tab;
            let mut viewer = panes::Viewer {
                state: &mut state.picture,
                interaction: &mut state.workspace.interaction,
                params: backend,
                now,
            };
            viewer.ui(&mut body_ui, &mut tab);
            // A whole-page fixture must hold the whole page (see
            // [`PANE_HEIGHT`]); a shorter one is a window onto it on purpose.
            // Video is the exception by construction: its preview takes
            // whatever height the controls above it leave.
            if height >= PANE_HEIGHT && tab != panes::Tab::Video {
                let bottom = body_ui.min_rect().bottom();
                assert!(bottom <= body.bottom(), "{tab:?} runs to {bottom}, past {height}");
            }
        },
    )
}

/// The render the pane fixtures have in flight, so the Video pane's progress
/// bar is drawn in every sweep over the settings panes rather than only in the
/// test below — it takes the column's width like every other bar, and that is
/// what the sweeps are for. The two digits of `done` against three of `total`
/// also put the padded readout through them.
pub(super) const FIXTURE_RENDER: RenderProgress = RenderProgress { done: 120, total: 990 };

/// Whether the leaf holding `tab` is folded away.
pub(super) fn collapsed(state: &SharedState, tab: panes::Tab) -> bool {
    !state.workspace.layout.visible(tab)
}

pub(super) fn fixture_export() -> harmonigraph_take::render::ExportJob {
    harmonigraph_take::render::ExportJob {
        id: 7,
        take: "music.take".into(),
        output: "music.mp4".into(),
        size: Some([1280, 720]),
        state: harmonigraph_take::render::ExportStatus::Running,
        progress: FIXTURE_RENDER,
        detail: String::new(),
    }
}
