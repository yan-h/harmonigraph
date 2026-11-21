use super::*;
use crate::lattice_maps::{MapDocument, MapEdit, MapPlayback, MapView};
use crate::params::ParamKey;
use crate::tests::probe::{events_into, fresh_picture, press, themed};
use harmonigraph_core::lattice_map::{Follow, TuningEngine};
use std::cell::RefCell;

/// A Hub with `engine` and `follow` selected and `maps` saved maps, recording
/// the map and tuning edits the pane sends. `pending` is whether audio has yet
/// to adopt them.
struct Edits {
    engine: TuningEngine,
    follow: Follow,
    maps: usize,
    pending: bool,
    offsets: crate::lattice_maps::MapOffsets,
    edits: RefCell<Vec<MapEdit>>,
    tuning: RefCell<Vec<ConfigEdit>>,
}
impl Edits {
    fn new(engine: TuningEngine, follow: Follow, maps: usize) -> Self {
        Self {
            engine,
            follow,
            maps,
            pending: false,
            offsets: Default::default(),
            edits: Default::default(),
            tuning: Default::default(),
        }
    }
}
impl ParamBackend for Edits {
    fn get(&self, key: ParamKey) -> f32 {
        key.default_value()
    }
    fn set(&self, _: ParamKey, _: f32) {}
    fn lattice_maps(&self) -> Option<MapView> {
        let mut document = MapDocument::default();
        for n in 1..self.maps {
            document.capture(Default::default(), format!("Map {n}"));
        }
        let playback = MapPlayback {
            engine: self.engine,
            follow: self.follow,
            map: Some(Default::default()),
            ..Default::default()
        };
        Some(MapView {
            playback,
            names: document.names(),
            offsets: self.offsets,
            followed: harmonigraph_core::LatticePos::ORIGIN,
            unadopted: self.pending.then_some(playback),
            edit_shape: false,
            can_undo: false,
            full: false,
        })
    }
    fn edit_lattice_map(&self, edit: MapEdit) {
        self.edits.borrow_mut().push(edit);
    }
    fn submit_tuning(&self, edit: ConfigEdit) -> Option<bool> {
        self.tuning.borrow_mut().push(edit);
        Some(true)
    }
}

/// Where the first text painted as `needle` sits, nudged inside it.
fn text_at(out: &egui::FullOutput, needle: &str) -> Option<egui::Pos2> {
    out.shapes.iter().find_map(|shape| match &shape.shape {
        egui::Shape::Text(text) if text.galley.text().starts_with(needle) => {
            Some(text.pos + egui::vec2(4.0, 4.0))
        }
        _ => None,
    })
}

/// Where `glyph` is painted on the row whose text starts at `row`: the
/// row's own move or delete button, drawn whether or not it is enabled.
fn glyph_on_row(out: &egui::FullOutput, row: egui::Pos2, glyph: &str) -> Option<egui::Pos2> {
    out.shapes.iter().find_map(|shape| match &shape.shape {
        egui::Shape::Text(text)
            if text.galley.text() == glyph
                && (text.visual_bounding_rect().center().y - row.y).abs() < 8.0 =>
        {
            Some(text.visual_bounding_rect().center())
        }
        _ => None,
    })
}

/// One click at `at`: the pointer arrives a frame before the press, because
/// egui resolves the widget under it from the previous pass.
fn click(frame: &mut impl FnMut(Vec<egui::Event>) -> egui::FullOutput, at: egui::Pos2) {
    frame(vec![egui::Event::PointerMoved(at)]);
    frame(vec![press(at, true)]);
    frame(vec![press(at, false)]);
}

#[test]
fn every_map_control_needs_no_fold_opened() {
    // A fresh context, where a fold would start closed.
    let ctx = themed();
    let params = Edits::new(TuningEngine::LatticeMap, Follow::Off, 2);
    let mut state = fresh_picture();
    let size = egui::vec2(420.0, 2000.0);
    let mut frame = |events| {
        events_into(&ctx, size, egui::Rect::from_min_size(egui::Pos2::ZERO, size), events, |ui| {
            tuning_pane(ui, &mut state, &params, 0.0);
        })
    };
    frame(vec![]);
    let out = frame(vec![]);
    assert!(text_at(&out, "Fifths").is_some(), "the offsets must be drawn without a fold");
    for (label, expected) in [
        ("Map 1", "Select(1)"),
        ("Edit shape on lattice", "EditShape(true)"),
        ("Duplicate as new map", "Duplicate"),
    ] {
        let at = text_at(&out, label).unwrap_or_else(|| panic!("{label} must be visible"));
        click(&mut frame, at);
        let last = params.edits.borrow_mut().pop().map(|edit| format!("{edit:?}"));
        assert_eq!(last.as_deref(), Some(expected), "{label}");
    }
}

/// The pane sends the whole policy it shows, and "Derive from fifth" derives
/// the third and seventh from the fifth on screen.
#[test]
fn derive_from_fifth_sends_the_shown_policy_with_the_shown_fifths_keyboard() {
    let ctx = themed();
    let params = Edits::new(TuningEngine::Adaptive, Follow::Off, 1);
    let mut state = fresh_picture();
    let shown =
        PolicyConfig { pitch_flexibility: 37, keyboard: [695_000_000, 1, 2], ..Default::default() };
    state.runtime.adaptive_policy = shown;
    let size = egui::vec2(420.0, 2000.0);
    let mut frame = |events| {
        events_into(&ctx, size, egui::Rect::from_min_size(egui::Pos2::ZERO, size), events, |ui| {
            tuning_pane(ui, &mut state, &params, 0.0);
        })
    };
    frame(vec![]);
    let out = frame(vec![]);
    let at = text_at(&out, "Derive from fifth").expect("the button must be visible");
    click(&mut frame, at);
    let sent: Vec<_> = params.tuning.borrow().iter().map(|edit| edit.policy).collect();
    let keyboard = harmonigraph_core::tuning::fifth_generated(695_000_000);
    assert_eq!(sent, [Some(PolicyConfig { keyboard, ..shown })]);
}

/// Decode fifths and the Decoded fifth are drawn in the lattice-board's section under Pass
/// through too, and an edit of either sends the whole policy shown: here a
/// double-click on the bar, which puts it back on its fresh value, and a click
/// on the toggle.
#[test]
fn the_decoding_controls_send_the_shown_policy_under_pass_through() {
    let ctx = themed();
    let params = Edits::new(TuningEngine::Off, Follow::Off, 1);
    let mut state = fresh_picture();
    let shown =
        PolicyConfig { pitch_flexibility: 37, decoded_fifth: 700_000_000, ..Default::default() };
    state.runtime.adaptive_policy = shown;
    let size = egui::vec2(420.0, 2000.0);
    let mut frame = |events| {
        events_into(&ctx, size, egui::Rect::from_min_size(egui::Pos2::ZERO, size), events, |ui| {
            tuning_pane(ui, &mut state, &params, 0.0);
        })
    };
    frame(vec![]);
    let at = text_at(&frame(vec![]), "Decoded fifth").expect("the bar must be visible");
    frame(vec![egui::Event::PointerMoved(at)]);
    for _ in 0..2 {
        frame(vec![press(at, true)]);
        frame(vec![press(at, false)]);
    }
    let sent = params.tuning.borrow().last().and_then(|edit| edit.policy).expect("an edit");
    // The bar holds cents as an f32, a few microcents short of the default.
    let fresh = PolicyConfig::default().decoded_fifth;
    assert!((sent.decoded_fifth - fresh).abs() < 100, "{sent:?}");
    assert_eq!(sent, PolicyConfig { decoded_fifth: sent.decoded_fifth, ..shown });
    let at = text_at(&frame(vec![]), "Decode fifths").expect("the toggle is drawn");
    frame(vec![egui::Event::PointerMoved(at)]);
    frame(vec![press(at, true)]);
    frame(vec![press(at, false)]);
    let sent = params.tuning.borrow().last().and_then(|edit| edit.policy).expect("an edit");
    assert_eq!(
        sent,
        PolicyConfig { decode_fifths: true, ..shown },
        "the toggle sends the policy shown"
    );
}

/// What a double-click on the control labelled `label` sends under `engine`,
/// with Decode fifths on at a 700¢ Decoded fifth, the third linked, and the
/// configuration drawn in the board's tuning under Pass through, each on a
/// fresh pane: two double-clicks in one window count as a triple-click.
fn double_clicked(engine: TuningEngine, label: &str) -> Vec<ConfigEdit> {
    let ctx = themed();
    let params = Edits::new(engine, Follow::Off, 1);
    let mut state = fresh_picture();
    state.runtime.adaptive_policy =
        PolicyConfig { decode_fifths: true, decoded_fifth: 700_000_000, ..Default::default() };
    state.runtime.board_drawn = engine == TuningEngine::Off;
    state.runtime.tempered.syntonic = true;
    let size = egui::vec2(420.0, 2000.0);
    let mut frame = |events| {
        events_into(&ctx, size, egui::Rect::from_min_size(egui::Pos2::ZERO, size), events, |ui| {
            tuning_pane(ui, &mut state, &params, 0.0);
        })
    };
    frame(vec![]);
    let at = text_at(&frame(vec![]), label).unwrap_or_else(|| panic!("{label} is drawn"));
    frame(vec![egui::Event::PointerMoved(at)]);
    for _ in 0..2 {
        frame(vec![press(at, true)]);
        frame(vec![press(at, false)]);
    }
    params.tuning.take()
}

/// While Pass through decodes the board, Tuning's Perfect fifth is the
/// Decoded fifth: a double-click on it puts the Decoded fifth back on its
/// fresh value and leaves the lattice's own fifth alone, while the third and
/// the presets, which would edit saved intervals nobody sees, do nothing.
/// Under Lattice Map the same double-click edits the lattice's fifth.
#[test]
fn the_perfect_fifth_edits_the_decoded_fifth_while_the_board_is_drawn() {
    let [edit] = double_clicked(TuningEngine::Off, "Perfect fifth")[..] else {
        panic!("one edit");
    };
    let policy = edit.policy.expect("a Decoded fifth edit");
    let fresh = PolicyConfig::default().decoded_fifth;
    assert!((policy.decoded_fifth - fresh).abs() < 100, "{policy:?}");
    assert_eq!(edit.axes, [None; 5], "the lattice's fifth moved");
    // The third is linked, so its label carries the badge.
    for label in ["↔ · Major third", "Just"] {
        assert_eq!(double_clicked(TuningEngine::Off, label), [], "{label} edited");
    }
    let [edit] = double_clicked(TuningEngine::LatticeMap, "Perfect fifth")[..] else {
        panic!("one edit");
    };
    assert!(edit.axes[TuningAxis::Three.index()].is_some() && edit.policy.is_none());
}

/// With Learn on, whether the pane's first frame draws the armed ring at
/// `now`, and whether that frame holds the loop at full rate for it, with the
/// pane `below` points down a 2000-point screen and Decode fifths as given.
fn learn_ring_drawn(engine: TuningEngine, decoding: bool, below: f32) -> (bool, Option<f64>) {
    let ctx = themed();
    let params = Edits::new(engine, Follow::Off, 1);
    let mut state = fresh_picture();
    state.runtime.learn_active = true;
    state.runtime.adaptive_policy.decode_fifths = decoding;
    let size = egui::vec2(420.0, 2000.0);
    let now = 0.0;
    let ring = theme::armed().gamma_multiply(learn_pulse(now));
    let output =
        events_into(&ctx, size, egui::Rect::from_min_size(egui::Pos2::ZERO, size), vec![], |ui| {
            ui.add_space(below);
            tuning_pane(ui, &mut state, &params, now);
        });
    let drawn = output.shapes.iter().any(|shape| {
        matches!(&shape.shape, egui::Shape::Rect(rect)
            if rect.stroke.color == ring && rect.stroke.width == theme::LINE_WIDTH)
    });
    (drawn, state.surfaces.moving_this_frame.after(crate::Reason::LearnPulse))
}

/// On the pane's first frame, so the engine it reads is this frame's rather
/// than one an earlier frame left behind.
#[test]
fn the_learn_ring_is_drawn_only_where_learn_acts() {
    assert_eq!(
        learn_ring_drawn(TuningEngine::Off, false, 0.0),
        (true, Some(crate::CHROME_CUE)),
        "Pass through learns, so the ring pulses at about 30 Hz"
    );
    assert_eq!(
        learn_ring_drawn(TuningEngine::LatticeMap, false, 0.0),
        (false, None),
        "Lattice Map suspends Learn, and its still ring holds nothing"
    );
    assert_eq!(
        learn_ring_drawn(TuningEngine::Off, true, 0.0),
        (false, None),
        "Decode fifths suspends Learn"
    );
    // Scrolled out of the settings page, the pulse is seen nowhere.
    assert!(
        learn_ring_drawn(TuningEngine::Off, false, 3000.0).1.is_none(),
        "a ring out of view held the loop"
    );
}

/// Each row's buttons act on that row's map, selected or not, and are
/// disabled where they could do nothing: Move up on the first row, Move down
/// on the last, and Delete map while only one map is saved.
#[test]
fn each_rows_buttons_act_on_its_own_map() {
    for (maps, row, glyph, expected) in [
        (2, "C · 3×4", "↓", Some("Move(0, Later)")),
        (2, "C · 3×4", "↑", None),
        (2, "Map 1", "↑", Some("Move(1, Earlier)")),
        (2, "Map 1", "↓", None),
        (2, "Map 1", "×", Some("Delete(1)")),
        (1, "C · 3×4", "×", None),
    ] {
        let ctx = themed();
        let params = Edits::new(TuningEngine::LatticeMap, Follow::Off, maps);
        let mut state = fresh_picture();
        let size = egui::vec2(420.0, 2000.0);
        let mut frame = |events| {
            events_into(
                &ctx,
                size,
                egui::Rect::from_min_size(egui::Pos2::ZERO, size),
                events,
                |ui| {
                    tuning_pane(ui, &mut state, &params, 0.0);
                },
            )
        };
        frame(vec![]);
        let out = frame(vec![]);
        let at = text_at(&out, row).expect("the map's row");
        let button = glyph_on_row(&out, at, glyph).expect("every row draws all three buttons");
        click(&mut frame, button);
        let sent = params.edits.borrow().last().map(|edit| format!("{edit:?}"));
        assert_eq!(sent.as_deref(), expected, "{maps} saved maps, {row}, {glyph}");
    }
}

/// The lines that change while music plays or an edit lands never stand
/// above a map control, so none of them moves a control under the pointer:
/// the follow readout is drawn at zero as well, and the pending notice ends
/// the pane.
#[test]
fn map_status_lines_never_move_a_map_control() {
    let ctx = themed();
    let params = Edits { pending: true, ..Edits::new(TuningEngine::LatticeMap, Follow::Thirds, 1) };
    let mut state = fresh_picture();
    let size = egui::vec2(420.0, 2000.0);
    let frame = |state: &mut _| {
        events_into(&ctx, size, egui::Rect::from_min_size(egui::Pos2::ZERO, size), vec![], |ui| {
            tuning_pane(ui, state, &params, 0.0)
        })
    };
    frame(&mut state);
    // The notice waits for one requested state to go unadopted that long.
    state.runtime.backdate_pending_labels();
    let out = frame(&mut state);
    assert!(
        text_at(&out, "Moved: fifths +0, thirds +0").is_some(),
        "the follow readout must stand while following has not moved the map"
    );
    let pending = text_at(&out, "Map state pending audio adoption").expect("the pending notice");
    for control in
        ["Edit shape on lattice", "Duplicate as new map", "Fifths", "Reset context on loop / seek"]
    {
        let at = text_at(&out, control).unwrap_or_else(|| panic!("{control} must be drawn"));
        assert!(at.y < pending.y, "the pending notice stands above {control}");
    }
}

/// How a test leaves the name field after typing in it.
#[derive(Clone, Copy, Debug)]
enum Leave {
    Key(egui::Key),
    /// A click on the selected row's number, drawn left of the field and so
    /// before it in the frame.
    Number,
    /// Cmd-click on a bar above, which opens a typed entry that takes the
    /// keyboard before the name field is drawn.
    TypeIntoBar,
    /// More typing in the frame the host takes the keyboard, then Enter once
    /// it is given back: the late keystroke is part of the name.
    EnterAfterHostTookKeyboard,
}

/// The name field renames when it is left — by Enter, or by a click on
/// another control, even one that takes the keyboard before the field is
/// drawn — never on a keystroke, since each rename has the host reread the
/// Map parameter's names; Escape keeps the old name.
#[test]
fn the_name_field_renames_when_left_and_not_while_typing() {
    let renamed = Some("Rename(0, \"C · 3×4 B\")");
    for (leave, expected) in [
        (Leave::Key(egui::Key::Enter), renamed),
        (Leave::Key(egui::Key::Escape), None),
        (Leave::Number, renamed),
        (Leave::TypeIntoBar, renamed),
        (Leave::EnterAfterHostTookKeyboard, Some("Rename(0, \"C · 3×4 BC\")")),
    ] {
        let ctx = themed();
        let params = Edits::new(TuningEngine::LatticeMap, Follow::Off, 1);
        let mut state = fresh_picture();
        let size = egui::vec2(420.0, 2000.0);
        let focused = std::cell::Cell::new(true);
        let mut frame = |modifiers, events| {
            let screen = egui::Rect::from_min_size(egui::Pos2::ZERO, size);
            let raw = egui::RawInput {
                screen_rect: Some(screen),
                events,
                modifiers,
                focused: focused.get(),
                ..Default::default()
            };
            ctx.run_ui(raw, |ui| {
                let mut child = ui.new_child(egui::UiBuilder::new().max_rect(screen));
                tuning_pane(&mut child, &mut state, &params, 0.0);
            })
        };
        let none = egui::Modifiers::NONE;
        frame(none, vec![]);
        let out = frame(none, vec![]);
        let field = text_at(&out, "C · 3×4").expect("the name field");
        let number = out.shapes.iter().find_map(|shape| match &shape.shape {
            egui::Shape::Text(text) if text.galley.text() == "1" => {
                Some(text.pos + egui::vec2(2.0, 4.0))
            }
            _ => None,
        });
        let number = number.expect("the map number");
        let bar = text_at(&out, "C pitch offset").expect("a bar above the field");
        fn click(
            frame: &mut impl FnMut(egui::Modifiers, Vec<egui::Event>) -> egui::FullOutput,
            modifiers: egui::Modifiers,
            at: egui::Pos2,
        ) {
            frame(modifiers, vec![egui::Event::PointerMoved(at)]);
            frame(modifiers, vec![press(at, true)]);
            frame(modifiers, vec![press(at, false)]);
        }
        click(&mut frame, none, field);
        // Move to the end, wherever the click put the cursor.
        let key_event = |key| egui::Event::Key {
            key,
            physical_key: None,
            pressed: true,
            repeat: false,
            modifiers: egui::Modifiers::NONE,
        };
        frame(none, vec![key_event(egui::Key::End)]);
        frame(none, vec![egui::Event::Text(" B".into())]);
        let typed = text_at(&frame(none, vec![]), "C · 3×4 B");
        assert!(typed.is_some(), "the typing must reach the field");
        assert!(params.edits.borrow().is_empty(), "typing sent {:?}", params.edits.borrow());
        match leave {
            Leave::Key(key) => {
                frame(none, vec![key_event(key)]);
            }
            Leave::Number => click(&mut frame, none, number),
            Leave::TypeIntoBar => click(&mut frame, egui::Modifiers::COMMAND, bar),
            Leave::EnterAfterHostTookKeyboard => {
                focused.set(false);
                frame(none, vec![egui::Event::Text("C".into()), egui::Event::WindowFocused(false)]);
                frame(none, vec![]);
                focused.set(true);
                frame(none, vec![egui::Event::WindowFocused(true)]);
                frame(none, vec![key_event(egui::Key::Enter)]);
            }
        }
        frame(none, vec![]);
        let sent = params.edits.borrow().first().map(|edit| format!("{edit:?}"));
        assert_eq!(sent.as_deref(), expected, "left by {leave:?}");
    }
}

#[test]
fn follow_harmony_shows_adaptive_context_under_lattice_map() {
    let drawn = |engine, follow| {
        let ctx = themed();
        let params = Edits::new(engine, follow, 1);
        let mut state = fresh_picture();
        let size = egui::vec2(420.0, 2000.0);
        let mut frame = || {
            events_into(
                &ctx,
                size,
                egui::Rect::from_min_size(egui::Pos2::ZERO, size),
                vec![],
                |ui| tuning_pane(ui, &mut state, &params, 0.0),
            )
        };
        frame();
        let out = frame();
        // Adaptive's block title, then every control the context has, none
        // of them behind a fold.
        let controls = [
            "Half-life",
            "Same-note tolerance",
            "Silence reset",
            "Reset context on stop",
            "Reset context on loop / seek",
        ];
        (text_at(&out, "Context").is_some(), controls.map(|label| text_at(&out, label).is_some()))
    };
    let (title, controls) = drawn(TuningEngine::Adaptive, Follow::Off);
    assert!(title, "Adaptive keeps its Context block");
    assert_eq!(controls, [true; 5], "Adaptive's context is drawn without a fold");
    let (title, controls) = drawn(TuningEngine::LatticeMap, Follow::Thirds);
    assert!(!title, "Follow's context sits in its own block");
    assert_eq!(controls, [true; 5], "Follow reads the context, reachable without opening a fold");
    let (title, controls) = drawn(TuningEngine::LatticeMap, Follow::Off);
    assert!(!title && controls == [false; 5], "nothing reads it with Follow off");
}

/// An offset lane is a value bar: a double-click puts it back on zero, sent
/// to the host as one gesture.
#[test]
fn a_double_click_resets_an_offset_lane_in_one_gesture() {
    let ctx = themed();
    let mut params = Edits::new(TuningEngine::LatticeMap, Follow::Off, 1);
    params.offsets.extension.threes = 3;
    let mut state = fresh_picture();
    let size = egui::vec2(420.0, 2000.0);
    let mut frame = |events| {
        events_into(&ctx, size, egui::Rect::from_min_size(egui::Pos2::ZERO, size), events, |ui| {
            tuning_pane(ui, &mut state, &params, 0.0);
        })
    };
    frame(vec![]);
    let at = text_at(&frame(vec![]), "Coarse").expect("the Coarse lane is drawn");
    frame(vec![egui::Event::PointerMoved(at)]);
    for _ in 0..2 {
        frame(vec![press(at, true)]);
        frame(vec![press(at, false)]);
    }
    frame(vec![]);
    let edits: Vec<_> = params.edits.borrow().iter().map(|edit| format!("{edit:?}")).collect();
    assert_eq!(
        edits,
        [
            "BeginOffset(Fifths, Extension)",
            "Offset(Fifths, Extension, 0)",
            "EndOffset(Fifths, Extension)"
        ]
    );
}
