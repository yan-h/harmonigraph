//! Fixtures more than one suite needs: a recording [`ParamBackend`], the
//! [`DockHarness`] that drives a whole dock through egui frames, and the
//! settings-pane painters the width and scale suites both measure.
//!
//! Everything here needs a whole dock. What a single pane or widget needs is
//! in [`super::probe`], and [`press`] is re-exported from there so a suite
//! reaching for the harness gets both.

use crate::widgets::range_probe::Placed;
use crate::*;
use harmonigraph_take::RenderProgress;

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
    /// The live tuning instances the Tuning pane's Sources table lists.
    pub(super) instances: Vec<params::TuningInstance>,
    /// The live analysis input, or `None` for a shell with no choice of one.
    pub(super) input: Option<params::AnalysisInput>,
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
    fn tuning_instances(&self) -> Vec<params::TuningInstance> {
        self.instances.clone()
    }
    fn analysis_input(&self) -> Option<params::AnalysisInput> {
        self.input
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
        unadopted: None,
        names: crate::lattice_maps::MapDocument::default().names(),
        edit_shape: false,
        can_undo: false,
        full: false,
    }
}

/// A Hub playing Lattice Map: a saved map selected and sounding, following
/// moved off the offsets, Edit shape on with an edit to undo, offsets off
/// zero, every slot used, and the whole still pending adoption. With Learn
/// held on (which the case's state does, [`SettingsCase::state`]) that draws
/// every conditional row of `map_controls` but one: "Map unavailable", which
/// a sounding map excludes.
pub(super) fn lattice_map_view() -> crate::lattice_maps::MapView {
    use harmonigraph_core::{lattice_map::Follow, LatticePos};
    let mut view = engine_view(harmonigraph_core::lattice_map::TuningEngine::LatticeMap);
    view.playback.map = Some(Default::default());
    view.playback.follow = Follow::ThirdsAndFifths;
    view.followed = LatticePos { threes: -2, fives: 1, sevens: 0 };
    view.offsets.fine = LatticePos { threes: -7, fives: 3, sevens: -1 };
    view.offsets.extension = LatticePos { threes: -1, fives: 0, sevens: 2 };
    view.unadopted = Some(view.playback);
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
    /// `root_ui` calls `set_ui_scale` from `state.workspace.interaction.prefs.ui_scale` on every frame, so
    /// a context dialled up on its own is reset by the first frame drawn on it
    /// and the sweep silently measures the design size at every step. Dialling
    /// the context as well is what puts the first frame at the scale rather
    /// than one frame behind it.
    pub(super) fn scaled(size: egui::Vec2, scale: f32, state: &mut SharedState) -> Self {
        state.workspace.interaction.prefs.ui_scale = scale;
        let mut harness = DockHarness::at(size);
        harness.ctx = super::probe::themed_scaled(scale);
        harness
    }

    /// The same backed by a Hub running `engine`, for what only a Hub's
    /// Tuning pane draws.
    pub(super) fn with_engine(
        mut self,
        engine: harmonigraph_core::lattice_map::TuningEngine,
    ) -> Self {
        self.backend.maps = Some(engine_view(engine));
        self
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
        self.pass(state, events, |_| {})
    }

    /// One frame with `overlay` drawn over the dock in the same pass, for a
    /// fixture that needs a control of its own rather than one the dock
    /// happens to draw today: `root_ui` runs first, as it does for every
    /// frame, and the overlay's widgets meet the pointer as the dock's do.
    pub(super) fn frame_over(
        &mut self,
        state: &mut SharedState,
        events: Vec<egui::Event>,
        overlay: impl FnMut(&egui::Context),
    ) -> egui::FullOutput {
        self.pass(state, events, overlay).0
    }

    fn pass(
        &mut self,
        state: &mut SharedState,
        events: Vec<egui::Event>,
        mut overlay: impl FnMut(&egui::Context),
    ) -> (egui::FullOutput, f64) {
        self.t += 1.0 / 60.0;
        // Held as a shell holds them: the modifiers its key events carry. egui
        // matches a shortcut against these, not the event's own.
        let modifiers = events
            .iter()
            .rev()
            .find_map(|event| match event {
                egui::Event::Key { modifiers, .. } => Some(*modifiers),
                _ => None,
            })
            .unwrap_or_default();
        let raw = egui::RawInput {
            screen_rect: Some(self.screen),
            time: Some(self.t),
            events: crate::tests::probe::holding(&self.ctx, modifiers, events),
            ..Default::default()
        };
        let t = self.t;
        let backend = &self.backend;
        let mut ui_ms = 0.0;
        let output = crate::tests::probe::run(&self.ctx, raw, |ui| {
            let start = std::time::Instant::now();
            root_ui(ui, state, backend, t, "shell-tag-fixture @0123456");
            ui_ms = start.elapsed().as_secs_f64() * 1000.0;
            overlay(ui.ctx());
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

/// The modes a page draws controls of its own for, read off the editor's own
/// option tables, so a mode added to a choice row is swept and range-guarded
/// without a list here. Every one rather than every one that differs from
/// today's defaults, because a default moving is how the sweeps lost the
/// Stars editor, Scales and the Adaptive section without a line of them
/// changing (#1484).
///
/// Projection turns the Camera block hard: `view::camera` hides the whole
/// camera-angle half (Camera yaw and pitch, the Angle presets, the Save-angle
/// row) under Cabinet, and the two cabinet knobs under the others.
pub(super) fn projections() -> impl Iterator<Item = harmonigraph_scene::Projection> {
    crate::panes::view::PROJECTION_CHOICES.into_iter().map(|(projection, _, _)| projection)
}

/// The spectrogram's textures, as [`projections`] reads the projections.
pub(super) fn textures() -> impl Iterator<Item = harmonigraph_scene::CloudStyle> {
    crate::panes::spectral::TEXTURE_CHOICES.into_iter().map(|(style, _, _)| style)
}

/// The lattice glow's materials, as [`projections`] reads the projections.
pub(super) fn materials() -> impl Iterator<Item = harmonigraph_scene::LatticeMaterial> {
    crate::panes::MATERIAL_CHOICES.into_iter().map(|(material, _, _)| material)
}

/// Every settings tab, in the column's own order.
pub(super) const SETTINGS_PANES: &[panes::Tab] = crate::workspace::Section::Settings.tabs();

/// Switch on (or off) every section a fresh state hides behind a setting:
/// the lattice's audio ring, read as a spectrum; the roll, the spectrogram
/// and the note names; the perf overlay's Frame breakdown; and the take
/// controls.
/// Off also zeroes the glow pattern's contrast.
///
/// One door for both fixtures that light a page: the range guard's
/// `enabled` scenarios and the sweeps' [`Variant::Lit`]. It moves no value a
/// bar shows beyond the switches, so a poisoned load keeps everything else.
pub(super) fn set_optional_sections(state: &mut SharedState, on: bool) {
    use harmonigraph_scene::SpectralReading;
    let a = &mut state.picture.appearance;
    a.view.spectral_reading = if on { SpectralReading::Spectrum } else { SpectralReading::Fold };
    a.view.spectral_ring_width = if on { 0.1 } else { 0.0 };
    a.spectrum.show_roll = on;
    a.spectrum.show_spectrogram = on;
    a.spectrum.note_names = on;
    if !on {
        a.view.atmosphere.texture_depth = 0.0;
    }
    let interaction = &mut state.workspace.interaction;
    interaction.prefs.show_perf = on;
    interaction.take.supported = on;
    interaction.take.last_take = on.then(|| "music.take".into());
}

/// What one settings sweep case changes from a fresh state: one choice that
/// swaps which controls a page draws, with everything else left fresh.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(super) enum Variant {
    Fresh,
    /// The Lattice page's Camera block (see [`projections`]).
    Projection(harmonigraph_scene::Projection),
    /// The spectrogram's texture, whose own controls the Analyzer page draws.
    Texture(harmonigraph_scene::CloudStyle),
    /// The lattice glow's material, whose own controls the Lattice page draws.
    Material(harmonigraph_scene::LatticeMaterial),
    /// Every section a fresh state hides switched on
    /// ([`set_optional_sections`]), with a ring middle that leaves the audio
    /// ring room to draw.
    Lit,
    /// The Hub's note retuning on a backend that has an engine selector;
    /// Lattice Map plays [`lattice_map_view`].
    Engine(harmonigraph_core::lattice_map::TuningEngine),
    /// Velocity and gain routed to Opacity, so the Mappings page draws a
    /// weight bar beside a Delete button for each.
    Routed,
    /// Pass through decoding the lattice-board, so the Tuning page draws the
    /// Decoded fifth in the Perfect fifth's place, the interval rows greyed,
    /// and the note that says why.
    Board,
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
        // Every section open, though a fresh editor folds the lattice-board's:
        // `open_settings_pane` opens subsections, not sections.
        state.workspace.interaction.prefs.folded_sections.clear();
        // Learn on under Lattice Map, which says it is suspended there.
        state.picture.runtime.learn_active = self.variant
            == Variant::Engine(harmonigraph_core::lattice_map::TuningEngine::LatticeMap);
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
            // What a frame's configuration view sets (`observe_configuration`),
            // which these fixtures do not run.
            Variant::Board => state.picture.runtime.board_drawn = true,
            Variant::Projection(projection) => appearance.camera.projection = projection,
            Variant::Texture(style) => appearance.spectrum.atmosphere.cloud_style = style,
            Variant::Material(material) => appearance.view.atmosphere.material_style = material,
            Variant::Routed => {
                let intensity = &mut appearance.view.intensity;
                for source in [&mut intensity.velocity, &mut intensity.gain] {
                    *source.weight_mut(harmonigraph_scene::IntensityTarget::Opacity) = Some(1.0);
                }
            }
            Variant::Lit => {
                set_optional_sections(&mut state, true);
                // The fresh middle is a look and free to grow, and a ring
                // refused for room greys every bar that sizes it.
                state.picture.appearance.view.ring_inner = 0.3;
            }
        }
        state
    }

    pub(super) fn backend(self) -> RecordingBackend {
        use harmonigraph_core::lattice_map::TuningEngine;
        let maps = match self.variant {
            Variant::Engine(TuningEngine::LatticeMap) => Some(lattice_map_view()),
            Variant::Engine(engine) => Some(engine_view(engine)),
            Variant::Board => Some(engine_view(TuningEngine::Off)),
            _ => None,
        };
        // A plugin's backend, so the controls only a live host offers are
        // drawn too: the Sources table with its Source details and Tuning
        // delay folds, and the Analyzer's Audio input row.
        RecordingBackend {
            maps,
            instances: super::probe::tuning_instances(),
            input: Some(params::AnalysisInput::Main),
            ..Default::default()
        }
    }
}

/// The cases the layout sweeps draw: each page fresh, then every texture on
/// the Analyzer page, every material and projection on the Lattice page,
/// every retuning engine and the lattice-board's tuning ([`Variant::Board`])
/// on the Tuning page, the Mappings page with two mappings routed
/// ([`Variant::Routed`]), and [`Variant::Lit`] on the
/// Lattice, Analyzer and System pages — all against a plugin's backend
/// ([`SettingsCase::backend`]), with every fold open.
///
/// Not every state a page can be in. Left out, among others: a greyed
/// control's lit form (Texture mix or the glow at 0 grey the controls under
/// them); a tuner selected in the Sources table,
/// whose Source details adds a delay row the Hub's lacks; a configuration
/// view (the range guard draws one, at 600 pt) or a status notice; and held
/// voices.
///
/// Every texture, material, projection and engine, for the reason
/// [`projections`] gives.
pub(super) fn settings_cases() -> Vec<SettingsCase> {
    use harmonigraph_core::lattice_map::TuningEngine;
    use panes::Tab;
    let engines = TuningEngine::HOST;
    let mut cases = Vec::new();
    for &pane in SETTINGS_PANES {
        let variants: Vec<Variant> = match pane {
            Tab::AnalyzerSettings => {
                textures().map(Variant::Texture).chain([Variant::Lit]).collect()
            }
            // Every material at the fresh projection, then every projection
            // at the fresh material, Cabinet named rather than left to the
            // fresh camera: the Camera block and the glow's material are
            // independent blocks of the page.
            Tab::LatticeSettings => materials()
                .map(Variant::Material)
                .chain(projections().map(Variant::Projection))
                .chain([Variant::Lit])
                .collect(),
            Tab::Mappings => vec![Variant::Fresh, Variant::Routed],
            Tab::System => vec![Variant::Fresh, Variant::Lit],
            // No engine selector at all, then each engine behind one, then
            // the lattice-board's tuning drawn under Pass through.
            Tab::Tuning => std::iter::once(Variant::Fresh)
                .chain(engines.map(Variant::Engine))
                .chain([Variant::Board])
                .collect(),
            _ => vec![Variant::Fresh],
        };
        cases.extend(variants.into_iter().map(|variant| SettingsCase { pane, variant }));
    }
    cases
}

/// One settings page whose content box is `width` points wide, every fold on
/// it open, as the shapes it emitted. Driven
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
    open_settings_pane(&super::probe::themed(), case, &mut case.state(), width).0
}

/// Where every bar on `case`'s page was laid, at a content box `width` across,
/// every fold open (see [`settings_pane_at_width`]).
pub(super) fn bars_at_width(case: SettingsCase, width: f32) -> Vec<Placed> {
    open_settings_pane(&super::probe::themed(), case, &mut case.state(), width).1
}

/// `case`'s page drawn on `ctx` into a content box `width` across with every
/// fold on it open: the shapes the last frame emitted, and where each bar in
/// that frame was laid.
///
/// The folds are subsections, opened by the flag [`panes::open_subsections`]
/// sets rather than by a click on each, since a subsection's fold lives in
/// egui's memory under an id only the pane can name, and the one switch that
/// opens every fold at once (`Memory::set_everything_is_visible`) also shows
/// every tooltip on the page, which is exactly what a geometry sweep must not
/// draw.
pub(super) fn open_settings_pane(
    ctx: &egui::Context,
    case: SettingsCase,
    state: &mut SharedState,
    width: f32,
) -> (Vec<egui::epaint::ClippedShape>, Vec<Placed>) {
    let backend = case.backend();
    panes::open_subsections(ctx);
    let frame = |state: &mut SharedState| {
        tab_body_with(ctx, state, case.pane, (width, PANE_HEIGHT), 0.0, &backend, vec![]).shapes
    };
    frame(state);
    // The pending notices wait for one request to go unadopted that long:
    // past it, so the page draws them too.
    state.picture.runtime.backdate_pending_labels();
    let mut shapes = Vec::new();
    let bars = crate::widgets::range_probe::placed(|| shapes = frame(state));
    (shapes, bars)
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
    crate::tests::probe::run(
        ctx,
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
