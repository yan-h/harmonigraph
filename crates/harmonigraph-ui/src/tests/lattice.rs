//! The lattice pane's own input — wheel and zoom gestures onto the camera
//! — and the learn mode that writes tuning params back from held notes.

use crate::*;
use harmonigraph_render::wgpu::TextureFormat;
use super::harness::*;

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
/// on that too — a handler that only reads the scroll delta does nothing here.
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
        state.tracker.handle_event(harmonigraph_core::NoteEvent {
            time: 0.0,
            channel: 0,
            note,
            kind: harmonigraph_core::NoteEventKind::On { velocity: 1.0 },
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
        state.tracker.handle_event(harmonigraph_core::NoteEvent {
            time: 0.0,
            channel: 0,
            note,
            kind: harmonigraph_core::NoteEventKind::On { velocity: 1.0 },
        });
        if cents != 0.0 {
            state.tracker.handle_event(harmonigraph_core::NoteEvent {
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
    let just_offset = harmonigraph_core::tuning::FIVE_JUST - 400.0;
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

/// One switch governs every automatic meantone decision, learn included.
#[test]
fn learn_leaves_meantone_alone_when_the_auto_detect_is_off() {
    let mut state = SharedState::new(TextureFormat::Bgra8Unorm);
    let backend = RecordingBackend::default();
    state.learn_active = true;
    state.view.meantone_auto = false;
    state.view.meantone = true;
    // The just triad that DOES release the mode with the detect on.
    let just_offset = harmonigraph_core::tuning::FIVE_JUST - 400.0;
    hold_chord(&mut state, &[(60, 0.0), (64, just_offset), (67, 0.0)]);
    learn_step(&mut state, &backend);
    assert!(state.view.meantone, "with the detect off, learn only retunes the axes");
}

/// A backend that answers with a real tuning, which [`RecordingBackend`]
/// does not: it reports 0.0 for every key, and a 0¢ fifth is not a tuning any
/// detection could sensibly fire on.
///
/// Writes land only on [`flush`](Self::flush), which is the plugin's own
/// behaviour rather than a convenience for the tests: nih-plug queues a `set`
/// for the host and the parameter "will only be changed when the output event
/// is written", so `get` reports the value being written away FROM for a
/// frame or more afterwards. A backend that applied writes instantly would
/// pass whatever the shell does with the frames in between.
struct TuningBackend {
    three: std::cell::Cell<f32>,
    five: std::cell::Cell<f32>,
    seven: std::cell::Cell<f32>,
    queued: std::cell::RefCell<Vec<(params::ParamKey, f32)>>,
}

impl TuningBackend {
    fn new(three: f32, five: f32) -> Self {
        TuningBackend {
            three: std::cell::Cell::new(three),
            five: std::cell::Cell::new(five),
            // 0¢ until a test says otherwise: no comma identity can fire on
            // it, so the tests about the syntonic comma are undisturbed by
            // the septimal one running beside them.
            seven: std::cell::Cell::new(0.0),
            queued: std::cell::RefCell::new(Vec::new()),
        }
    }

    /// The same backend with a harmonic seventh, for the tests that ask
    /// about the septimal kleisma.
    fn with_seven(self, seven: f32) -> Self {
        self.seven.set(seven);
        self
    }

    /// Let the host catch up: every queued write takes effect.
    fn flush(&self) {
        for (key, value) in self.queued.borrow_mut().drain(..) {
            match key {
                params::ParamKey::Three => self.three.set(value),
                params::ParamKey::Five => self.five.set(value),
                params::ParamKey::Seven => self.seven.set(value),
                _ => {}
            }
        }
    }
}

impl ParamBackend for TuningBackend {
    fn get(&self, key: params::ParamKey) -> f32 {
        match key {
            params::ParamKey::Three => self.three.get(),
            params::ParamKey::Five => self.five.get(),
            params::ParamKey::Seven => self.seven.get(),
            // A workable matching window; the rest are irrelevant here and
            // 0 is a legal value for each.
            params::ParamKey::Tolerance => 0.5,
            _ => 0.0,
        }
    }
    fn set(&self, key: params::ParamKey, value: f32) {
        self.queued.borrow_mut().push((key, value));
    }
}

/// The auto-detect engages the mode from the tuning alone — no learn, no
/// switch. Quarter-comma meantone: a 696.58¢ fifth whose four-stack lands on
/// the just major third.
#[test]
fn a_meantone_tuning_engages_the_mode_by_itself() {
    let mut state = SharedState::new(TextureFormat::Bgra8Unorm);
    assert!(state.view.meantone_auto, "the auto-detect is on out of the box");
    assert!(!state.view.meantone, "and the mode starts off");
    let three = harmonigraph_core::tuning::THREE_JUST
        - harmonigraph_core::tuning::SYNTONIC_COMMA / 4.0;
    let params = TuningBackend::new(three, harmonigraph_core::tuning::FIVE_JUST);
    begin_frame(&mut state, &params, 0.0);
    assert!(state.view.meantone, "quarter-comma meantone should engage the mode");
    // Engaging it is only half the job: the lattice has to be using the
    // derived third, exactly, or comma-equivalent nodes stay two pitches.
    let octave = i64::from(harmonigraph_core::tuning::OCTAVE_MICROCENTS);
    assert_eq!(
        i64::from(state.tuning.five),
        4 * i64::from(state.tuning.three) - 2 * octave,
    );
}

/// Just intonation keeps the syntonic comma, so nothing engages: this is the
/// case the tolerance exists to reject.
#[test]
fn just_intonation_does_not_engage_meantone() {
    let mut state = SharedState::new(TextureFormat::Bgra8Unorm);
    let params = TuningBackend::new(
        harmonigraph_core::tuning::THREE_JUST,
        harmonigraph_core::tuning::FIVE_JUST,
    );
    begin_frame(&mut state, &params, 0.0);
    assert!(!state.view.meantone, "a just third is a comma away from four fifths");
}

/// With the detect off, even 12-TET — which IS a meantone — leaves the mode
/// exactly as the user set it.
#[test]
fn the_auto_detect_off_leaves_the_mode_alone() {
    let mut state = SharedState::new(TextureFormat::Bgra8Unorm);
    state.view.meantone_auto = false;
    let params = TuningBackend::new(
        harmonigraph_core::tuning::THREE_12TET,
        harmonigraph_core::tuning::FIVE_12TET,
    );
    begin_frame(&mut state, &params, 0.0);
    assert!(!state.view.meantone, "the detect is off; nothing should engage");
}

/// The detect ENGAGES only. Dragging the fifth moves the derived third out
/// from under the third param — which is inert while the lock holds — so a
/// detect that also released would drop the mode the moment the fifth moved,
/// which is the one thing meantone is for.
#[test]
fn dragging_the_fifth_does_not_drop_an_engaged_meantone() {
    let mut state = SharedState::new(TextureFormat::Bgra8Unorm);
    state.view.meantone = true;
    // 4·690 − 2400 = 360¢: the stale third param is 40¢ away, far outside
    // the tolerance, and irrelevant while the lock holds.
    let params = TuningBackend::new(690.0, 400.0);
    begin_frame(&mut state, &params, 0.0);
    assert!(state.view.meantone, "the mode must survive a fifth that moved");
    assert!((state.tuning.five_cents() - 360.0).abs() < 0.001, "the third follows the fifth");
}

/// Releasing by dragging the third writes the dragged value into the param
/// (see `panes::tuning`), and the detect must then leave it released — the
/// magnet only reaches `TEMPER_TOLERANCE`.
#[test]
fn a_third_dragged_clear_of_the_magnet_stays_released() {
    let mut state = SharedState::new(TextureFormat::Bgra8Unorm);
    // Either side of the window, as a FRACTION of it: the two cases have to
    // stay just outside and just inside whatever the tolerance is set to,
    // and fixed offsets stop straddling it the moment it narrows.
    let tolerance = harmonigraph_core::tuning::TEMPER_TOLERANCE;
    let params = TuningBackend::new(
        harmonigraph_core::tuning::THREE_12TET,
        harmonigraph_core::tuning::FIVE_12TET + tolerance * 1.5,
    );
    begin_frame(&mut state, &params, 0.0);
    assert!(!state.view.meantone, "past the tolerance nothing pulls it back");
    // Just inside, though, and the magnet takes it.
    let mut state = SharedState::new(TextureFormat::Bgra8Unorm);
    let params = TuningBackend::new(
        harmonigraph_core::tuning::THREE_12TET,
        harmonigraph_core::tuning::FIVE_12TET + tolerance * 0.5,
    );
    begin_frame(&mut state, &params, 0.0);
    assert!(state.view.meantone, "inside the tolerance the mode engages");
}

/// The switch still means something with the detect on: pressed ON at a
/// tuning the detect would never claim (just intonation, a whole comma out),
/// it snaps the third to four fifths and stays there. The detect never
/// releases, so it cannot argue.
#[test]
fn the_switch_snaps_a_non_meantone_tuning_with_the_detect_on() {
    let mut state = SharedState::new(TextureFormat::Bgra8Unorm);
    let params = TuningBackend::new(
        harmonigraph_core::tuning::THREE_JUST,
        harmonigraph_core::tuning::FIVE_JUST,
    );
    begin_frame(&mut state, &params, 0.0);
    assert!(!state.view.meantone, "just intonation is not detected as meantone");

    // What the switch does, which is all it does.
    state.view.meantone = true;
    for frame in 0..3 {
        begin_frame(&mut state, &params, frame as f64);
        assert!(state.view.meantone, "frame {frame} dropped a hand-set lock");
    }
    // Snapped: the lattice's third is four fifths, not the just third the
    // param still holds.
    let octave = i64::from(harmonigraph_core::tuning::OCTAVE_MICROCENTS);
    assert_eq!(
        i64::from(state.tuning.five),
        4 * i64::from(state.tuning.three) - 2 * octave,
    );
}

/// And the OFF direction, which is the one the detect could undo — the
/// reported "sometimes I have to press it twice". The mode was engaged from
/// this very tuning, so a detect that judged it again would re-engage it on
/// the next frame, and the press would do nothing you could see.
#[test]
fn the_switch_releases_under_the_detect_until_the_tuning_changes() {
    let mut state = SharedState::new(TextureFormat::Bgra8Unorm);
    let params = TuningBackend::new(
        harmonigraph_core::tuning::THREE_12TET,
        harmonigraph_core::tuning::FIVE_12TET,
    );
    begin_frame(&mut state, &params, 0.0);
    assert!(state.view.meantone, "12-TET engages by itself");

    // The switch, in full: it writes nothing but the flag.
    state.view.meantone = false;
    for frame in 1..4 {
        begin_frame(&mut state, &params, frame as f64);
        assert!(!state.view.meantone, "frame {frame} re-engaged a tuning already judged");
    }

    // A tuning that has moved is a fresh question, and this one is still a
    // meantone: quarter-comma, both axes written together.
    let three = harmonigraph_core::tuning::THREE_JUST
        - harmonigraph_core::tuning::SYNTONIC_COMMA / 4.0;
    params.set(params::ParamKey::Three, three);
    params.set(params::ParamKey::Five, harmonigraph_core::tuning::meantone_third(three));
    params.flush();
    begin_frame(&mut state, &params, 4.0);
    assert!(state.view.meantone, "a new meantone tuning should engage again");
}

/// Switching the detect ON asks it about the tuning already loaded, which
/// takes clearing the verdict: `begin_frame` records every pair it sees,
/// running or not, so the pair in front of it has been "judged" by the time
/// the switch is pressed and would be skipped until the tuning next moved.
/// A project saved with the detect off is the case that never recovers on
/// its own.
#[test]
fn switching_the_detect_on_asks_it_about_the_tuning_already_there() {
    let mut state = SharedState::new(TextureFormat::Bgra8Unorm);
    state.view.meantone_auto = false;
    let params = TuningBackend::new(
        harmonigraph_core::tuning::THREE_12TET,
        harmonigraph_core::tuning::FIVE_12TET,
    );
    for frame in 0..3 {
        begin_frame(&mut state, &params, frame as f64);
    }
    assert!(!state.view.meantone, "the detect is off; nothing should engage");

    // The Auto switch, in full: the flag and the cleared verdict.
    state.view.meantone_auto = true;
    state.temper_judged[harmonigraph_core::Comma::Syntonic.index()] = None;
    begin_frame(&mut state, &params, 3.0);
    assert!(state.view.meantone, "switching the detect on left 12-TET unjudged");
}

/// A tuning write the host has not reported back must not be judged on the
/// value it is moving away FROM. In the plugin every `set` is queued for the
/// host, so for a frame or more `get` still answers with the old value —
/// and the two edits that MEAN "this is not meantone" both write while the
/// mode is being switched off: dragging the third clear of the magnet (here)
/// and the Just preset. Judged afresh, that stale pair re-locks the mode the
/// edit was undoing.
#[test]
fn an_in_flight_tuning_write_is_not_judged_before_it_lands() {
    let mut state = SharedState::new(TextureFormat::Bgra8Unorm);
    let params = TuningBackend::new(
        harmonigraph_core::tuning::THREE_12TET,
        harmonigraph_core::tuning::FIVE_12TET,
    );
    begin_frame(&mut state, &params, 0.0);
    assert!(state.view.meantone, "12-TET engages by itself");

    // What the third bar does when a drag escapes the magnet: drop the mode
    // and write the dragged value. The host has not heard about it yet.
    state.view.meantone = false;
    params.set(params::ParamKey::Five, harmonigraph_core::tuning::FIVE_12TET + 2.0);
    begin_frame(&mut state, &params, 1.0);
    assert!(!state.view.meantone, "the stale pair re-locked the mode mid-write");

    // And once it lands, the pair it lands on is judged on its own terms.
    params.flush();
    begin_frame(&mut state, &params, 2.0);
    assert!(!state.view.meantone, "a third 2¢ off four fifths is not a meantone");
}

/// The septimal comma's detect, on the tuning every project opens at: 12-TET
/// tempers 225/224 out (1000 = 2·700 + 2·400 − 1200) exactly as it tempers
/// 81/80 out, so both modes engage from the same tuning without either being
/// asked for.
#[test]
fn a_marvel_tuning_engages_the_mode_by_itself() {
    let mut state = SharedState::new(TextureFormat::Bgra8Unorm);
    assert!(state.view.marvel_auto, "the septimal detect is on out of the box");
    assert!(!state.view.marvel, "and the mode starts off");
    let params = TuningBackend::new(
        harmonigraph_core::tuning::THREE_12TET,
        harmonigraph_core::tuning::FIVE_12TET,
    )
    .with_seven(harmonigraph_core::tuning::SEVEN_12TET);
    begin_frame(&mut state, &params, 0.0);
    assert!(state.view.marvel, "12-TET tempers out the septimal kleisma too");
    // Engaging it is only half the job: the lattice has to be using the
    // derived seventh, exactly, or the sevens sheet stays a separate set of
    // pitches from the home sheet it now spells as.
    let octave = i64::from(harmonigraph_core::tuning::OCTAVE_MICROCENTS);
    assert_eq!(
        i64::from(state.tuning.seven),
        2 * i64::from(state.tuning.three) + 2 * i64::from(state.tuning.five) - octave,
    );
    // Which is the whole point: 7/4 and ten fifths are one pitch class.
    let seventh = state.tuning.pitch_class(harmonigraph_core::LatticePos::new(0, 0, 1));
    let tenth_fifth = state.tuning.pitch_class(harmonigraph_core::LatticePos::new(10, 0, 0));
    assert_eq!(seventh, tenth_fifth);
}

/// Just intonation keeps every comma, the septimal kleisma included: the just
/// seventh sits 7.7¢ under two fifths plus two thirds, which is a lot more
/// than the tolerance.
#[test]
fn just_intonation_does_not_engage_marvel() {
    let mut state = SharedState::new(TextureFormat::Bgra8Unorm);
    let params = TuningBackend::new(
        harmonigraph_core::tuning::THREE_JUST,
        harmonigraph_core::tuning::FIVE_JUST,
    )
    .with_seven(harmonigraph_core::tuning::SEVEN_JUST);
    begin_frame(&mut state, &params, 0.0);
    assert!(!state.view.marvel, "a just seventh is a kleisma away");
    assert!(!state.view.meantone, "and a just third a syntonic comma away");
}

/// The two locks compose, and the order is what makes them: the septimal
/// identity reads the third, so with both engaged it must read the third
/// MEANTONE is deriving rather than the inert param under it. That is what
/// turns the pair into septimal meantone — a seventh of ten fifths.
#[test]
fn the_two_locks_compose_into_septimal_meantone() {
    let mut state = SharedState::new(TextureFormat::Bgra8Unorm);
    // Quarter-comma meantone, whose derived third is the just one — and a
    // seventh param nowhere near anything, to prove it is not being read.
    let three = harmonigraph_core::tuning::THREE_JUST
        - harmonigraph_core::tuning::SYNTONIC_COMMA / 4.0;
    let params = TuningBackend::new(three, harmonigraph_core::tuning::FIVE_JUST).with_seven(940.0);
    state.view.marvel = true;
    begin_frame(&mut state, &params, 0.0);
    assert!(state.view.meantone, "quarter-comma engages the syntonic lock");
    let octave = i64::from(harmonigraph_core::tuning::OCTAVE_MICROCENTS);
    assert_eq!(
        i64::from(state.tuning.seven),
        10 * i64::from(state.tuning.three) - 5 * octave,
        "the seventh follows the DERIVED third, not the param",
    );
    // 965.78¢, three cents under the just seventh: septimal meantone's own
    // seventh, which is what makes it spell as the augmented sixth.
    assert!((state.tuning.seven_cents() - 965.784).abs() < 0.01);
}

/// Each comma's detect judges only the axes its own identity reads. A seventh
/// that moves says nothing about the syntonic comma, so it must not re-open
/// that verdict — otherwise dragging the seventh would re-engage a meantone
/// the user had just switched off, which is the same "press it twice" bug the
/// judged-once rule exists to prevent, one axis over.
#[test]
fn a_seventh_that_moves_does_not_re_open_the_meantone_question() {
    let mut state = SharedState::new(TextureFormat::Bgra8Unorm);
    // The septimal mode is not what this is about; leave it out of the way.
    state.view.marvel_auto = false;
    let params = TuningBackend::new(
        harmonigraph_core::tuning::THREE_12TET,
        harmonigraph_core::tuning::FIVE_12TET,
    )
    .with_seven(harmonigraph_core::tuning::SEVEN_12TET);
    begin_frame(&mut state, &params, 0.0);
    assert!(state.view.meantone, "12-TET engages by itself");

    state.view.meantone = false;
    for (frame, seven) in [(1.0, 900.0), (2.0, 1010.0)] {
        params.set(params::ParamKey::Seven, seven);
        params.flush();
        begin_frame(&mut state, &params, frame);
        assert!(!state.view.meantone, "a seventh at {seven}¢ re-locked the meantone");
    }
    // The fifth or the third moving IS a fresh question, and this one is
    // still a meantone.
    params.set(params::ParamKey::Three, 700.5);
    params.set(params::ParamKey::Five, 402.0);
    params.flush();
    begin_frame(&mut state, &params, 3.0);
    assert!(state.view.meantone, "402 = 4·700.5 − 2400 is a meantone again");
}

/// The septimal lock survives its own axes moving, for the reason the
/// syntonic one does: with the mode on, the seventh param is inert and the
/// derived seventh follows the fifth and third, so a detect that also
/// released would drop the lock the moment either moved.
#[test]
fn dragging_the_fifth_does_not_drop_an_engaged_marvel() {
    let mut state = SharedState::new(TextureFormat::Bgra8Unorm);
    state.view.marvel = true;
    // 2·690 + 2·400 − 1200 = 980¢: the stale seventh param is 20¢ away and
    // irrelevant while the lock holds.
    let params = TuningBackend::new(690.0, 400.0).with_seven(1000.0);
    begin_frame(&mut state, &params, 0.0);
    assert!(state.view.marvel, "the mode must survive a fifth that moved");
    assert!(
        (state.tuning.seven_cents() - 980.0).abs() < 0.001,
        "the seventh follows the fifth and third",
    );
}

/// Learn settles the septimal question the same way it settles the syntonic
/// one — from a chord that pins down every axis the identity reads.
#[test]
fn learn_enables_marvel_from_a_12tet_seventh() {
    let mut state = SharedState::new(TextureFormat::Bgra8Unorm);
    let backend = RecordingBackend::default();
    state.learn_active = true;
    // C-E-G-B♭ in plain 12-TET: a 700¢ fifth, a 400¢ third and a 1000¢
    // seventh, which is 2·700 + 2·400 − 1200 exactly.
    hold_chord(&mut state, &[(60, 0.0), (64, 0.0), (67, 0.0), (70, 0.0)]);
    learn_step(&mut state, &backend);
    assert!(state.view.marvel, "a 12-TET seventh chord tempers out 225/224");
    assert!(state.view.meantone, "and 81/80 with it");
}

/// A chord with no seventh in it fixes nothing about the septimal comma, so
/// that mode is left exactly as it was — while the syntonic one, whose
/// identity the chord does state in full, is still settled.
#[test]
fn learn_leaves_marvel_unchanged_without_a_seventh() {
    let mut state = SharedState::new(TextureFormat::Bgra8Unorm);
    let backend = RecordingBackend::default();
    state.learn_active = true;
    state.view.marvel = true;
    let just_offset = harmonigraph_core::tuning::FIVE_JUST - 400.0;
    hold_chord(&mut state, &[(60, 0.0), (64, just_offset), (67, 0.0)]);
    learn_step(&mut state, &backend);
    assert!(state.view.marvel, "a triad shouldn't change the septimal flag");
    assert!(!state.view.meantone, "the just third still releases meantone");
}
