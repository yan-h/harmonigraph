//! The Spectral pane's gestures: the divider between spectrum and roll,
//! and the drags and wheel that pan and zoom its two axes.

use super::harness::*;

/// Drag the Spectral pane's spectrum/spectrogram divider through the REAL
/// dock — `root_ui`, egui_dock, the tab body's ScrollArea and all.
///
/// The pane's own tests drive `spectral_pane` into a bare child Ui, which
/// skips every layer the dock puts between the pointer and the handle. Any
/// of those could swallow the drag (the ScrollArea registers a drag-sensing
/// background widget of its own), and the failure would look exactly like
/// "dragging doesn't work" — silent, with the handle still lighting up on
/// hover. So the assertion is that the split actually MOVED.
#[test]
fn the_spectral_divider_drags_through_the_dock() {
    let mut state = fresh();
    let mut h = DockHarness::new();
    h.settle(&mut state);

    // Ask egui where the handle actually landed rather than deriving the
    // dock's arithmetic here, which would just re-encode the layout.
    let handle = egui::Id::new(("spectral-split", 0usize));
    let band = h.ctx.read_response(handle).expect("the split handle never registered").rect;
    let grab = band.center();
    let before = state.spectrum_config.roll_fraction;

    // Left (the default orientation) puts the divider upright, so the drag
    // that moves it runs along x — pushing it away from the spectrum.
    h.frame(&mut state, vec![egui::Event::PointerMoved(grab)]);
    assert!(
        h.ctx.read_response(handle).is_some_and(|r| r.hovered()),
        "the handle should light up under the pointer",
    );
    h.frame(&mut state, vec![egui::Event::PointerMoved(grab), press(grab, true)]);
    let target = grab + egui::vec2(40.0, 0.0);
    h.frame(&mut state, vec![egui::Event::PointerMoved(target)]);
    assert!(
        h.ctx.read_response(handle).is_some_and(|r| r.dragged()),
        "the handle should be dragged",
    );
    h.frame(&mut state, vec![press(target, false)]);

    let after = state.spectrum_config.roll_fraction;
    assert!(
        after < before - 0.1,
        "the split should have moved with the pointer ({before} -> {after})",
    );
}

/// Dragging the Spectral pane's picture pans the pitch range, through the real
/// dock. Panning DOWN the axis (dragging toward higher pitch) has to bring
/// lower pitches into view, the way grabbing any picture does.
#[test]
fn dragging_the_spectral_picture_pans_the_pitch_range() {
    let mut state = fresh();
    // Start zoomed in, so there is room to pan in both directions.
    state.spectrum_config.low_midi = 48.0;
    state.spectrum_config.high_midi = 84.0;
    let mut h = DockHarness::new();
    h.settle(&mut state);

    let grab = h.spectral_grab(&state);
    let before = state.spectrum_config;
    // Left (the default orientation) climbs in pitch UP the screen, so a
    // drag toward higher pitch is a drag toward smaller y.
    h.frame(&mut state, vec![egui::Event::PointerMoved(grab), press(grab, true)]);
    let target = grab + egui::vec2(0.0, -60.0);
    h.frame(&mut state, vec![egui::Event::PointerMoved(target)]);
    h.frame(&mut state, vec![press(target, false)]);

    let after = state.spectrum_config;
    assert!(
        after.low_midi < before.low_midi - 1.0,
        "the range should have followed the pointer down the axis ({} -> {})",
        before.low_midi,
        after.low_midi,
    );
    assert!(
        ((after.high_midi - after.low_midi) - (before.high_midi - before.low_midi)).abs() < 1e-3,
        "a pan moves the range without resizing it",
    );
    assert_eq!(
        after.roll_seconds, before.roll_seconds,
        "a drag across the pitch axis is the range's; the Span must not breathe with it",
    );
}

/// Dragging along the time axis zooms the roll's Span instead — the picture is
/// anchored at the now-line, so pulling it toward the past spreads it out and
/// the seconds it spans shrink. The pitch range stays where it was: one drag
/// moves one axis.
#[test]
fn dragging_the_spectral_picture_along_time_zooms_the_span() {
    let mut state = fresh();
    let mut h = DockHarness::new();
    h.settle(&mut state);

    let grab = h.spectral_grab(&state);
    let before = state.spectrum_config;
    // Left runs time rightward (now at the left), so dragging right is
    // dragging toward the past.
    h.frame(&mut state, vec![egui::Event::PointerMoved(grab), press(grab, true)]);
    let target = grab + egui::vec2(120.0, 0.0);
    h.frame(&mut state, vec![egui::Event::PointerMoved(target)]);
    h.frame(&mut state, vec![press(target, false)]);

    let zoomed = state.spectrum_config;
    assert!(
        zoomed.roll_seconds < before.roll_seconds * 0.75,
        "dragging toward the past should have zoomed in ({} -> {})",
        before.roll_seconds,
        zoomed.roll_seconds,
    );
    assert_eq!(
        (zoomed.low_midi, zoomed.high_midi),
        (before.low_midi, before.high_midi),
        "the Span's drag is not the pitch range's",
    );

    // And back: the mapping is exponential in the drag, so the same distance
    // the other way returns the span it started on. Grabbed from the same spot
    // (the far end of a rightward drag can land outside the pane, where a press
    // is nobody's drag).
    let back = grab - egui::vec2(120.0, 0.0);
    h.frame(&mut state, vec![egui::Event::PointerMoved(grab), press(grab, true)]);
    h.frame(&mut state, vec![egui::Event::PointerMoved(back)]);
    h.frame(&mut state, vec![press(back, false)]);
    let restored = state.spectrum_config.roll_seconds;
    assert!(
        (restored - before.roll_seconds).abs() < 0.1,
        "dragging back should restore the span ({} -> {restored})",
        before.roll_seconds,
    );
}

/// The same drag begun over the SPECTRUM zooms the level range instead, and
/// leaves the Span alone: the depth axis is dB there rather than time, and a
/// drag moves what is under the hand. Dragging out along the curve, away from
/// the baseline it stands on, spreads it — so the dB window closes in, exactly
/// as pulling away from the now-line shortens the Span.
#[test]
fn a_drag_over_the_spectrum_zooms_the_level_and_not_the_span() {
    let mut state = fresh();
    // Zoomed in, so a stray pan would show up rather than sitting against the
    // clamp at the ends of the axis.
    state.spectrum_config.low_midi = 48.0;
    state.spectrum_config.high_midi = 84.0;
    let mut h = DockHarness::new();
    h.settle(&mut state);

    // The spectrum owns 0..0.45 of the depth axis by default. Left runs depth
    // rightward with the baseline at the divider, so the curve grows LEFTWARD
    // and that is the way out of it.
    let grab = h.spectral_grab_at(&state, 0.2);
    let before = state.spectrum_config;
    h.frame(&mut state, vec![egui::Event::PointerMoved(grab), press(grab, true)]);
    let target = grab + egui::vec2(-80.0, -20.0);
    h.frame(&mut state, vec![egui::Event::PointerMoved(target)]);
    h.frame(&mut state, vec![press(target, false)]);

    let after = state.spectrum_config;
    assert_eq!(
        after.roll_seconds, before.roll_seconds,
        "a drag begun over the spectrum has no time axis under it",
    );
    assert!(
        after.ceiling_db < before.ceiling_db - 3.0,
        "the level window should have closed, so the drag did reach the pane ({} -> {})",
        before.ceiling_db,
        after.ceiling_db,
    );
    // The floor is the baseline the zoom is about, and the pitch range is the
    // other axis' business: one drag moves one thing.
    assert_eq!(after.floor_db, before.floor_db, "the floor is this zoom's anchor");
    assert_eq!(
        (after.low_midi, after.high_midi),
        (before.low_midi, before.high_midi),
        "the Level's drag is not the pitch range's",
    );
}

/// The wheel zooms the pitch range, and touches NOTHING else — in particular
/// not the roll's time Span, which is the other thing a wheel over this pane
/// could plausibly have meant.
#[test]
fn the_wheel_zooms_the_pitch_range_and_leaves_the_time_span_alone() {
    let mut state = fresh();
    state.spectrum_config.low_midi = 36.0;
    state.spectrum_config.high_midi = 96.0;
    let mut h = DockHarness::new();
    h.settle(&mut state);

    let over = h.spectral_grab(&state);
    let before = state.spectrum_config;
    h.frame(&mut state, vec![egui::Event::PointerMoved(over)]);
    // Several notches, so the assertion isn't riding on egui's scroll smoothing
    // having fully caught up in one frame.
    for _ in 0..4 {
        h.frame(
            &mut state,
            vec![egui::Event::MouseWheel {
                unit: egui::MouseWheelUnit::Point,
                delta: egui::vec2(0.0, 40.0),
                phase: egui::TouchPhase::Move,
                modifiers: egui::Modifiers::default(),
            }],
        );
    }

    let after = state.spectrum_config;
    let (was, now) = (before.high_midi - before.low_midi, after.high_midi - after.low_midi);
    assert!(now < was - 1.0, "scrolling up should have zoomed in ({was} -> {now})");
    assert_eq!(
        after.roll_seconds, before.roll_seconds,
        "the wheel is the pitch range's, not the time axis's",
    );
    // Zoom is anchored on the pointer, which is the middle of the pane here,
    // so the pitch under it should not have moved.
    let mid = |c: &crate::SpectrumConfig| 0.5 * (c.low_midi + c.high_midi);
    assert!(
        (mid(&after) - mid(&before)).abs() < 1.0,
        "the pitch under the pointer should stay put ({} -> {})",
        mid(&before),
        mid(&after),
    );
}

/// The pane now senses drags over its whole surface, which is exactly what
/// could have swallowed the divider's. It must not: the divider registers
/// after the pane, so egui leaves it on top, and a drag that starts on the
/// handle still resizes the split and does NOT pan the pitch.
#[test]
fn the_divider_still_wins_the_drag_over_the_pane_behind_it() {
    let mut state = fresh();
    let mut h = DockHarness::new();
    h.settle(&mut state);

    let handle = egui::Id::new(("spectral-split", 0usize));
    let band = h.ctx.read_response(handle).expect("the split handle never registered").rect;
    let grab = band.center();
    let before = state.spectrum_config;

    h.frame(&mut state, vec![egui::Event::PointerMoved(grab), press(grab, true)]);
    // A drag with a pitch-axis component, so a pane that stole it would show
    // up as a moved range rather than as nothing happening.
    let target = grab + egui::vec2(40.0, -30.0);
    h.frame(&mut state, vec![egui::Event::PointerMoved(target)]);
    h.frame(&mut state, vec![press(target, false)]);

    let after = state.spectrum_config;
    assert!(
        (after.roll_fraction - before.roll_fraction).abs() > 0.01,
        "the divider should still have moved",
    );
    assert_eq!(
        (after.low_midi, after.high_midi),
        (before.low_midi, before.high_midi),
        "dragging the divider must not pan the pitch range as well",
    );
    assert_eq!(
        after.roll_seconds, before.roll_seconds,
        "nor zoom the Span — the drag leans along time, which is the Span's gesture",
    );
    // And not the Level either. The grab is the band's CENTRE, which sits on
    // the split to within a float divide — so which of the two depth zooms a
    // stolen drag would run is decided by a last bit, and asserting on one of
    // them leaves the other as a way through.
    assert_eq!(after.ceiling_db, before.ceiling_db, "nor zoom the Level, on the other side");
}

/// The analyzer's PANE resizing resizes its spectrogram and leaves the
/// spectrum the size it was — the same claim the pane's own suite makes about
/// `hold_spectrum`, made here through the real dock, where the resize arrives
/// as a window that changed rather than as a number handed to the hold.
///
/// Which is the layer that could go wrong on its own: the hold is applied at
/// the dock's tab dispatch, from the size the tab body is about to hand the
/// pane, and nothing in the pane would notice if that size were a frame stale
/// or the body's rather than the picture's.
///
/// So the spectrum is measured off the DRAWN divider — egui's own rect for the
/// handle, which the pane registers at whatever depth it composed the frame at
/// — rather than off `roll_fraction`, which the hold deliberately leaves to
/// the dial. That is the only reading here that could tell a pane drawing its
/// hold from a pane drawing the dial.
#[test]
fn resizing_the_analyzer_resizes_the_spectrogram_and_not_the_spectrum() {
    let mut state = fresh();
    let mut h = DockHarness::new();
    h.settle(&mut state);
    // Left is the default orientation, so the analyzer's depth axis — the one
    // the divider cuts — runs along the column's WIDTH, which is what a wider
    // window gives it more of.
    let pane = |state: &crate::SharedState| {
        pane_body(state, &crate::panes::Tab::Spectral)
            .expect("the Spectral pane should be visible in the default dock")
    };
    let handle = egui::Id::new(("spectral-split", 0usize));
    // The spectrum's drawn length: its outer edge is the pane's left in this
    // orientation, and the divider's band is centred on the boundary.
    let spectrum = |h: &DockHarness, state: &crate::SharedState| {
        let band = h.ctx.read_response(handle).expect("the split handle never registered").rect;
        band.center().x - pane(state).left()
    };
    let (was, dialled) = (pane(&state).width(), spectrum(&h, &state));

    h.screen.max.x += 400.0;
    h.settle(&mut state);
    let (now, kept) = (pane(&state).width(), spectrum(&h, &state));
    assert!(now > was + 20.0, "the analyzer's pane should have grown ({was} -> {now})");
    assert!((kept - dialled).abs() < 1.0, "the spectrum went {dialled} -> {kept}");
    // The spectrogram is where the growth went, all of it.
    let far = now - kept;
    assert!(
        (far - (was - dialled) - (now - was)).abs() < 1.0,
        "the spectrogram got {far} of a pane that gained {}",
        now - was,
    );
    // And the dial the render composes from never moved.
    assert_eq!(
        state.spectrum_config.roll_fraction,
        crate::SpectrumConfig::default().roll_fraction,
        "resizing the editor moved the split a take would export with",
    );

    // Back: a window returned to its size returns the picture to its own.
    h.screen.max.x -= 400.0;
    h.settle(&mut state);
    assert!(
        (pane(&state).width() - was).abs() < 0.5 && (spectrum(&h, &state) - dialled).abs() < 1.0,
        "back at {} points the spectrum is {}, not the {dialled} it started at",
        pane(&state).width(),
        spectrum(&h, &state),
    );
}
