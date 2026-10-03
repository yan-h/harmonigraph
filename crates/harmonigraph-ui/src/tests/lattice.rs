//! The lattice pane's own input — wheel and zoom gestures onto the camera
//! — and the learn mode that writes tuning params back from held notes.

use super::harness::*;
use crate::*;

/// Drive the real root_ui (dock, hover, everything) with a synthetic wheel
/// event over the lattice pane and return the camera distance after it.
/// `modifiers` picks whether egui routes the wheel to a scroll delta (plain)
/// or a zoom factor (COMMAND, egui's default zoom modifier).
fn distance_after_wheel_over_lattice(modifiers: egui::Modifiers) -> (f32, f32) {
    let mut state = fresh();
    let mut h = DockHarness::new();
    let start = state.picture.appearance.camera.distance;

    // A point solidly inside the top-left section, which holds the Lattice tab
    // alone (see default_dock): past the tab bar, left of the split.
    let over_lattice = egui::pos2(150.0, 150.0);
    let moved = || vec![egui::Event::PointerMoved(over_lattice)];

    // Warm-up passes so the pointer registers and egui's top-widget-at-
    // pointer resolution (which reads the previous pass) sees the lattice
    // under the pointer before the wheel pass.
    h.frame(&mut state, moved());
    h.frame(&mut state, moved());
    let mut wheel = moved();
    wheel.push(egui::Event::MouseWheel {
        unit: egui::MouseWheelUnit::Line,
        // Positive y = scroll up = zoom in (both the scroll and the
        // zoom-factor paths map an upward wheel to a smaller distance).
        delta: egui::vec2(0.0, 1.0),
        phase: egui::TouchPhase::Move,
        modifiers,
    });
    h.frame(&mut state, wheel);

    (start, state.picture.appearance.camera.distance)
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
/// on that too — a handler that only reads the scroll delta does nothing here.
#[test]
fn zoom_gesture_over_lattice_zooms_the_camera() {
    let (start, after) = distance_after_wheel_over_lattice(egui::Modifiers::COMMAND);
    assert!(after < start, "zoom-gesture wheel should zoom in ({start} -> {after})");
}

/// Navigation goes home on a double-click, while the projection and Cabinet
/// drafting choices remain available even when another projection is active.
#[test]
fn double_click_resets_lattice_navigation_but_keeps_drafting_choices() {
    for projection in PROJECTIONS {
        let mut state = fresh();
        let mut h = DockHarness::new();
        let at = egui::pos2(150.0, 150.0); // Inside the default Lattice section.
        h.frame(&mut state, vec![egui::Event::PointerMoved(at)]);
        h.frame(&mut state, vec![egui::Event::PointerMoved(at)]);

        let camera = &mut state.picture.appearance.camera;
        camera.projection = projection;
        camera.cabinet_angle = 30f32.to_radians();
        camera.cabinet_scale = 0.8;
        camera.target = glam::vec3(0.25, -0.25, 0.0);
        camera.yaw = 0.9;
        camera.pitch = 0.5;
        camera.distance = 8.0;
        let view = &mut state.picture.appearance.view;
        view.center_threes = 3;
        view.center_fives = -2;
        view.max_sevens = 1;
        view.center_sevens = 1;

        h.frame(&mut state, vec![press(at, true)]);
        h.frame(&mut state, vec![press(at, false)]);
        assert_eq!(
            state.picture.appearance.camera.distance, 8.0,
            "the first click must not reset {projection:?} navigation"
        );
        h.frame(&mut state, vec![press(at, true)]);
        h.frame(&mut state, vec![press(at, false)]);

        let camera = state.picture.appearance.camera;
        let home = harmonigraph_scene::Camera::default();
        assert_eq!(camera.projection, projection);
        assert_eq!(camera.cabinet_angle, 30f32.to_radians());
        assert_eq!(camera.cabinet_scale, 0.8);
        assert_eq!(camera.target, home.target);
        assert_eq!(camera.yaw, home.yaw);
        assert_eq!(camera.pitch, home.pitch);
        assert_eq!(camera.distance, home.distance);
        let view = &state.picture.appearance.view;
        assert_eq!((view.center_threes, view.center_fives, view.center_sevens), (0, 0, 1));
    }
}

/// A wheel notch during a drag on the lattice is one gesture, not a stale drag.
///
/// `end_stranded_drag` reads a wheel as a hand that is not on a button, and the
/// pointer standing on the widget being dragged is the whole of what keeps that
/// rule off zooming while dragging the view. This is the case that says the
/// exception is real: the notch zooms, and the drag is still following the
/// pointer after it.
///
/// The drag is measured at the camera TARGET rather than at the yaw because
/// the fresh projection is Cabinet, where a plain drag pans (orbiting a
/// fixed-viewpoint projection is meaningless — see `lattice_pane`). Which of
/// the two the gesture is doing is not this test's question; that it is still
/// doing it is.
#[test]
fn a_wheel_over_the_lattice_does_not_end_the_drag_it_is_zooming() {
    let mut state = fresh();
    let mut h = DockHarness::new();
    let ctx = h.ctx.clone();
    // Inside the top-left section, which holds the Lattice tab alone.
    let at = egui::pos2(150.0, 150.0);
    h.frame(&mut state, vec![egui::Event::PointerMoved(at)]);
    h.frame(&mut state, vec![press(at, true)]);
    h.frame(&mut state, vec![egui::Event::PointerMoved(at + egui::vec2(30.0, 0.0))]);
    assert!(ctx.dragged_id().is_some(), "the press on the lattice started no drag");

    let (moved_to, distance) =
        (state.picture.appearance.camera.target, state.picture.appearance.camera.distance);
    h.frame(
        &mut state,
        vec![egui::Event::MouseWheel {
            unit: egui::MouseWheelUnit::Line,
            delta: egui::vec2(0.0, 1.0),
            phase: egui::TouchPhase::Move,
            modifiers: egui::Modifiers::NONE,
        }],
    );
    assert!(
        state.picture.appearance.camera.distance < distance,
        "the wheel did not zoom the camera it was over"
    );
    assert!(ctx.dragged_id().is_some(), "the wheel ended the drag it was zooming");

    h.frame(&mut state, vec![egui::Event::PointerMoved(at + egui::vec2(60.0, 0.0))]);
    assert_ne!(
        state.picture.appearance.camera.target, moved_to,
        "the drag stopped following the pointer"
    );
}

#[test]
fn learn_step_writes_params_only_when_the_chord_changes() {
    let mut state = fresh();
    let backend = RecordingBackend::default();
    state.picture.runtime.learn_active = true;
    // Hold C and G (a 12-TET fifth: within learn range of just).
    for note in [60u8, 67] {
        state.picture.runtime.tracker.handle_event(harmonigraph_core::NoteEvent::on(
            0.0,
            harmonigraph_core::SourceId::DIRECT,
            0,
            note,
            1.0,
        ));
    }

    state.picture.runtime.learn_step(&mut state.picture.appearance, &backend);
    let first = backend.sets.borrow().clone();
    assert!(
        first.iter().any(|(k, v)| *k == params::ParamKey::Three && *v == 700.0),
        "the fifth should be learned from C+G, got {first:?}"
    );

    // Same chord again: change detection must suppress further writes.
    state.picture.runtime.learn_step(&mut state.picture.appearance, &backend);
    assert_eq!(backend.sets.borrow().len(), first.len());

    // Disarming clears the memory so re-arming re-learns.
    state.picture.runtime.learn_active = false;
    state.picture.runtime.learn_step(&mut state.picture.appearance, &backend);
    state.picture.runtime.learn_active = true;
    state.picture.runtime.learn_step(&mut state.picture.appearance, &backend);
    assert_eq!(backend.sets.borrow().len(), first.len() * 2);
}

/// Hold `notes` as channel-0 voices, each optionally bent by a per-note
/// tuning offset (cents). Used to synthesize just vs 12-TET chords.
fn hold_chord(state: &mut SharedState, notes: &[(u8, f32)]) {
    for &(note, cents) in notes {
        state.picture.runtime.tracker.handle_event(harmonigraph_core::NoteEvent::on(
            0.0,
            harmonigraph_core::SourceId::DIRECT,
            0,
            note,
            1.0,
        ));
        if cents != 0.0 {
            state.picture.runtime.tracker.handle_event(harmonigraph_core::NoteEvent {
                source: harmonigraph_core::SourceId::DIRECT,
                time: 0.0,
                channel: 0,
                note,
                kind: harmonigraph_core::NoteEventKind::Tuning { semitones: cents / 100.0 },
            });
        }
    }
}

#[test]
fn learn_enables_meantone_from_a_12tet_triad() {
    let mut state = unlocked();
    let backend = RecordingBackend::default();
    state.picture.runtime.learn_active = true;
    // Plain 12-TET C-E-G pins a 700¢ fifth and a 400¢ third; since
    // 400 = 4·700 − 2400 this triad IS a meantone.
    hold_chord(&mut state, &[(60, 0.0), (64, 0.0), (67, 0.0)]);
    state.picture.runtime.learn_step(&mut state.picture.appearance, &backend);
    assert!(state.picture.appearance.view.meantone, "a 12-TET triad should engage meantone");
}

#[test]
fn learn_disables_meantone_from_a_just_triad() {
    let mut state = fresh();
    let backend = RecordingBackend::default();
    state.picture.runtime.learn_active = true;
    state.picture.appearance.view.meantone = true; // start engaged

    // C + a JUST major third (386.31¢) + G. The just third sits a full
    // syntonic comma below four fifths, so this is not a meantone.
    let just_offset = harmonigraph_core::tuning::FIVE_JUST - 400.0;
    hold_chord(&mut state, &[(60, 0.0), (64, just_offset), (67, 0.0)]);
    state.picture.runtime.learn_step(&mut state.picture.appearance, &backend);
    assert!(!state.picture.appearance.view.meantone, "a just third should release meantone");
}

#[test]
fn learn_leaves_meantone_unchanged_without_a_third() {
    let mut state = fresh();
    let backend = RecordingBackend::default();
    state.picture.runtime.learn_active = true;
    state.picture.appearance.view.meantone = true;
    // A bare fifth fixes no third, so the meantone flag is left alone.
    hold_chord(&mut state, &[(60, 0.0), (67, 0.0)]);
    state.picture.runtime.learn_step(&mut state.picture.appearance, &backend);
    assert!(state.picture.appearance.view.meantone, "a bare fifth shouldn't change the flag");
}

/// An unlinked starting point makes each recognition assertion exercise Learn.
fn unlocked() -> SharedState {
    let mut state = fresh();
    state.picture.appearance.view.meantone = false;
    state.picture.appearance.view.marvel = false;
    state
}

/// Standalone's backend applies writes immediately. CLAP's delayed adoption is
/// covered through its real configuration mailbox in the plugin tests.
struct TuningBackend(std::cell::Cell<[f32; 5]>);
impl Default for TuningBackend {
    fn default() -> Self {
        Self(std::cell::Cell::new(params::ParamKey::TUNING.map(|key| key.default_value())))
    }
}
impl ParamBackend for TuningBackend {
    fn get(&self, key: params::ParamKey) -> f32 {
        params::ParamKey::TUNING
            .iter()
            .position(|&k| k == key)
            .map_or_else(|| key.default_value(), |i| self.0.get()[i])
    }
    fn set(&self, key: params::ParamKey, value: f32) {
        if let Some(i) = params::ParamKey::TUNING.iter().position(|&k| k == key) {
            let mut values = self.0.get();
            values[i] = value;
            self.0.set(values);
        }
    }
}

#[test]
fn standalone_edits_recognize_links_and_release_without_a_readout_jump() {
    use harmonigraph_core::configuration::ConfigEdit;
    use harmonigraph_core::Comma;
    let mut state = fresh();
    let backend = TuningBackend::default();
    begin_frame(&mut state.picture, &backend, 0.0);
    state.picture.runtime.edit_tuning(
        &mut state.picture.appearance,
        &backend,
        ConfigEdit::axis(1, harmonigraph_core::tuning::microcents(696.58)),
    );
    let linked = state.picture.runtime.tuning;
    for comma in Comma::ALL.into_iter().rev() {
        let edit = ConfigEdit::temper(comma, false, state.picture.runtime.tuning);
        state.picture.runtime.edit_tuning(&mut state.picture.appearance, &backend, edit);
    }
    for frame in 1..4 {
        begin_frame(&mut state.picture, &backend, frame as f64);
        assert!(!state.picture.appearance.view.meantone);
        assert!(!state.picture.appearance.view.marvel);
        assert!((state.picture.runtime.tuning.five_cents() - linked.five_cents()).abs() < 0.001);
        assert!((state.picture.runtime.tuning.seven_cents() - linked.seven_cents()).abs() < 0.001);
    }
    // A typed same-value entry is new input even though polling cannot see it.
    state.picture.runtime.edit_tuning(
        &mut state.picture.appearance,
        &backend,
        ConfigEdit::axis(2, harmonigraph_core::tuning::microcents(linked.five_cents())),
    );
    assert!(state.picture.appearance.view.meantone);
    assert!(state.picture.appearance.view.marvel);
    state.picture.runtime.edit_tuning(
        &mut state.picture.appearance,
        &backend,
        ConfigEdit::axis(2, 390_000_000),
    );
    begin_frame(&mut state.picture, &backend, 4.0);
    assert!(!state.picture.appearance.view.meantone);
    assert_eq!(state.picture.runtime.tuning.five_cents(), 390.0);
    assert!(state.picture.appearance.view.marvel, "the seventh follows the edited third");
}

#[test]
fn external_parameter_changes_only_recognize_related_intervals() {
    use harmonigraph_core::configuration::ConfigEdit;
    use harmonigraph_core::Comma;
    let mut state = fresh();
    let backend = TuningBackend::default();
    begin_frame(&mut state.picture, &backend, 0.0);
    let edit = ConfigEdit::temper(Comma::Syntonic, false, state.picture.runtime.tuning);
    state.picture.runtime.edit_tuning(&mut state.picture.appearance, &backend, edit);
    backend.set(params::ParamKey::Seven, 980.0);
    begin_frame(&mut state.picture, &backend, 1.0);
    assert!(!state.picture.appearance.view.meantone);
    assert!(!state.picture.appearance.view.marvel);
    backend.set(params::ParamKey::Three, 696.58);
    backend.set(params::ParamKey::Five, 386.32);
    begin_frame(&mut state.picture, &backend, 2.0);
    assert!(state.picture.appearance.view.meantone);
}

#[test]
fn learn_enables_marvel_from_a_12tet_seventh() {
    let mut state = unlocked();
    let backend = RecordingBackend::default();
    state.picture.runtime.learn_active = true;
    // C-E-G-B♭ in plain 12-TET: a 700¢ fifth, a 400¢ third and a 1000¢
    // seventh, which is 2·700 + 2·400 − 1200 exactly.
    hold_chord(&mut state, &[(60, 0.0), (64, 0.0), (67, 0.0), (70, 0.0)]);
    state.picture.runtime.learn_step(&mut state.picture.appearance, &backend);
    assert!(state.picture.appearance.view.marvel, "a 12-TET seventh chord tempers out 225/224");
    assert!(state.picture.appearance.view.meantone, "and 81/80 with it");
}

#[test]
fn learn_leaves_marvel_unchanged_without_a_seventh() {
    let mut state = fresh();
    let backend = RecordingBackend::default();
    state.picture.runtime.learn_active = true;
    state.picture.appearance.view.marvel = true;
    let just_offset = harmonigraph_core::tuning::FIVE_JUST - 400.0;
    hold_chord(&mut state, &[(60, 0.0), (64, just_offset), (67, 0.0)]);
    state.picture.runtime.learn_step(&mut state.picture.appearance, &backend);
    assert!(state.picture.appearance.view.marvel, "a triad shouldn't change the septimal flag");
    assert!(!state.picture.appearance.view.meantone, "the just third still releases meantone");
}

#[test]
fn learn_measures_the_septimal_comma_against_the_derived_third() {
    let mut state = unlocked();
    let backend = RecordingBackend::default();
    state.picture.runtime.learn_active = true;
    // A 700¢ fifth, a third 0.4¢ sharp, a seventh 0.6¢ sharp. The third is
    // inside the meantone tolerance, so the lattice's third becomes 400.0 and
    // the marvel seventh 1000.0 — which the played 1000.6 misses by 0.6¢.
    hold_chord(&mut state, &[(60, 0.0), (64, 0.4), (67, 0.0), (70, 0.6)]);
    state.picture.runtime.learn_step(&mut state.picture.appearance, &backend);
    assert!(
        state.picture.appearance.view.meantone,
        "a third 0.4¢ off four fifths is still a meantone"
    );
    assert!(
        !state.picture.appearance.view.marvel,
        "the seventh is 0.6¢ off the derived third's marvel seventh, not 0.2¢ off the played one",
    );
}

/// The window the docked lattice draws reaches the panes that describe the
/// picture — and reaches them as a WHOLE frame's answer rather than as
/// whatever had been drawn by the time they were asked.
///
/// The dock draws its panes in the order the user has arranged them, so a
/// band reading this frame's window would answer from the reach or from the
/// picture depending on where the lattice section sits in the layout. `drawn` is
/// the previous frame's, rotated in `begin_frame`, which is one answer for
/// every reader whatever the arrangement.
#[test]
fn the_window_the_lattice_drew_reaches_the_panes_that_describe_it() {
    let mut state = fresh();
    assert!(state.picture.surfaces.drawn.is_none(), "nothing has drawn yet");
    assert_eq!(
        state.picture.shown(),
        state.picture.appearance.view.reach(),
        "with no picture to describe, the reach is what there is",
    );

    let mut h = DockHarness::new();
    h.frame(&mut state, Vec::new());
    h.frame(&mut state, Vec::new());

    let drawn = state.picture.surfaces.drawn.expect("the docked lattice published no window");
    assert_eq!(
        state.picture.shown(),
        drawn,
        "the readers are not being given the picture's window"
    );
    assert_ne!(
        drawn,
        state.picture.appearance.view.reach(),
        "the published window is the reach, so nothing says it came from a camera",
    );
    // The docked pane's own, at the docked pane's own aspect — a window a
    // section of this shape really produces, not the whole editor's.
    assert!(
        drawn.count() < state.picture.appearance.view.reach().count(),
        "the lattice section is a fraction of the window, so its cabinet view is \
         well inside the reach: {} nodes against {}",
        drawn.count(),
        state.picture.appearance.view.reach().count(),
    );

    // Export has no pointer/chrome, but its spectral pane still describes
    // the camera's picture. A zoomed-out window must not fall back to reach.
    let mut state = fresh();
    state.picture.appearance.camera.projection = harmonigraph_scene::Projection::Perspective;
    state.picture.appearance.camera.distance = harmonigraph_scene::Camera::MAX_DISTANCE;
    let backend = RecordingBackend::default();
    begin_frame(&mut state.picture, &backend, 0.0);
    let expected =
        state.picture.appearance.view.scrolled(&state.picture.appearance.camera, 16.0 / 9.0);
    assert_ne!(expected, state.picture.appearance.view.reach());
    super::probe::painted_full(egui::vec2(640.0, 360.0), |ui| {
        crate::draw_pane(ui, Pane::Lattice, &mut state.picture, 0.0, 0);
    });
    assert_eq!(state.picture.surfaces.drawn_this_frame, Some(expected));
    begin_frame(&mut state.picture, &backend, 1.0 / 60.0);
    assert_eq!(state.picture.shown(), expected, "export discarded the camera window");
}

/// The lattice pane stands on a ground it paints itself, and paints the same
/// one it hands the scene.
///
/// Both claims in one test on purpose. A fill and a scene ground that disagree
/// do not show up as a bug: the picture looks plausible and stays wrong. What
/// holds the two together is that neither can move without failing here.
#[test]
fn the_lattice_pane_paints_the_ground_it_hands_the_scene() {
    let mut state = fresh();
    let screen = egui::vec2(600.0, 500.0);
    let shapes = super::probe::painted_full(screen, |ui| {
        crate::draw_pane(ui, Pane::Lattice, &mut state.picture, 0.0, 0);
    })
    .shapes;

    let ground = state.picture.background_ink();
    assert_eq!(
        ground,
        crate::theme::picture(),
        "a fresh state stands the lattice somewhere other than the ground \
         every other picture pane paints",
    );
    // Shrunk by a point before asking who covers it: the claim is "the whole
    // pane", and a rect that matches the pane exactly is a float comparison
    // away from failing on a pane whose size is not a round number.
    let pane = egui::Rect::from_min_size(egui::pos2(0.0, 0.0), screen).shrink(1.0);
    let painted = shapes.iter().any(|cs| match &cs.shape {
        egui::Shape::Rect(r) => r.fill == ground && r.rect.contains_rect(pane),
        _ => false,
    });
    assert!(
        painted,
        "the lattice pane drew no ground of its own, so it is showing whatever \
         is behind it — the dock's tab body in the plugin",
    );
}

/// And it paints the SHELL's ground, not the skin's.
///
/// The offline renderer clears its frame to the render layout's background and
/// hands the same color to the state, which a hand-written layout may set to
/// something other than the skin's picture ground — so a fill that reached for
/// the theme instead of the field would paint the skin's colour over every
/// such frame, in the one place it is hardest to notice. This is the test that
/// fails for that.
#[test]
fn the_pane_paints_the_shells_ground_rather_than_the_skins() {
    let mut state = fresh();
    // Nothing in the skin, so a fill that went to the theme cannot match it
    // by coincidence.
    state.picture.set_background((7, 9, 11));
    let screen = egui::vec2(600.0, 500.0);
    let shapes = super::probe::painted_full(screen, |ui| {
        crate::draw_pane(ui, Pane::Lattice, &mut state.picture, 0.0, 0);
    })
    .shapes;

    let pane = egui::Rect::from_min_size(egui::pos2(0.0, 0.0), screen).shrink(1.0);
    let covering = |fill: egui::Color32| {
        shapes.iter().any(|cs| match &cs.shape {
            egui::Shape::Rect(r) => r.fill == fill && r.rect.contains_rect(pane),
            _ => false,
        })
    };
    assert!(
        covering(egui::Color32::from_rgb(7, 9, 11)),
        "the pane did not paint the ground the shell set",
    );
    assert!(
        !covering(crate::theme::picture()),
        "the pane painted the skin's ground over the shell's own",
    );
}
