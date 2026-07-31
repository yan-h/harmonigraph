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
/// cannot: it reports 0.0 for every key, and a 0¢ fifth is not a tuning any
/// detection could sensibly fire on.
struct TuningBackend {
    three: f32,
    five: f32,
}

impl ParamBackend for TuningBackend {
    fn get(&self, key: params::ParamKey) -> f32 {
        match key {
            params::ParamKey::Three => self.three,
            params::ParamKey::Five => self.five,
            // A workable matching window; the rest are irrelevant here and
            // 0 is a legal value for each.
            params::ParamKey::Tolerance => 0.5,
            _ => 0.0,
        }
    }
    fn set(&self, _key: params::ParamKey, _value: f32) {}
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
    let params = TuningBackend { three, five: harmonigraph_core::tuning::FIVE_JUST };
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
    let params = TuningBackend {
        three: harmonigraph_core::tuning::THREE_JUST,
        five: harmonigraph_core::tuning::FIVE_JUST,
    };
    begin_frame(&mut state, &params, 0.0);
    assert!(!state.view.meantone, "a just third is a comma away from four fifths");
}

/// With the detect off, even 12-TET — which IS a meantone — leaves the mode
/// exactly as the user set it.
#[test]
fn the_auto_detect_off_leaves_the_mode_alone() {
    let mut state = SharedState::new(TextureFormat::Bgra8Unorm);
    state.view.meantone_auto = false;
    let params = TuningBackend {
        three: harmonigraph_core::tuning::THREE_12TET,
        five: harmonigraph_core::tuning::FIVE_12TET,
    };
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
    let params = TuningBackend { three: 690.0, five: 400.0 };
    begin_frame(&mut state, &params, 0.0);
    assert!(state.view.meantone, "the mode must survive a fifth that moved");
    assert!((state.tuning.five_cents() - 360.0).abs() < 0.001, "the third follows the fifth");
}

/// Releasing by dragging the third writes the dragged value into the param
/// (see `panes::tuning`), and the detect must then leave it released — the
/// magnet only reaches `MEANTONE_TOLERANCE`.
#[test]
fn a_third_dragged_clear_of_the_magnet_stays_released() {
    let mut state = SharedState::new(TextureFormat::Bgra8Unorm);
    // Either side of the window, as a FRACTION of it: the two cases have to
    // stay just outside and just inside whatever the tolerance is set to,
    // and fixed offsets stop straddling it the moment it narrows.
    let tolerance = harmonigraph_core::tuning::MEANTONE_TOLERANCE;
    let params = TuningBackend {
        three: harmonigraph_core::tuning::THREE_12TET,
        five: harmonigraph_core::tuning::FIVE_12TET + tolerance * 1.5,
    };
    begin_frame(&mut state, &params, 0.0);
    assert!(!state.view.meantone, "past the tolerance nothing pulls it back");
    // Just inside, though, and the magnet takes it.
    let mut state = SharedState::new(TextureFormat::Bgra8Unorm);
    let params = TuningBackend {
        three: harmonigraph_core::tuning::THREE_12TET,
        five: harmonigraph_core::tuning::FIVE_12TET + tolerance * 0.5,
    };
    begin_frame(&mut state, &params, 0.0);
    assert!(state.view.meantone, "inside the tolerance the mode engages");
}
