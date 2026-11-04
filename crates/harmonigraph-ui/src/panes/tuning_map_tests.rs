use super::*;
use crate::lattice_maps::{MapDocument, MapEdit, MapPlayback, MapView};
use crate::tests::probe::{events_into, fresh_picture, press, themed};
use harmonigraph_core::lattice_map::{Follow, TuningEngine};
use std::cell::RefCell;

/// A Hub with `engine` and `follow` selected and `maps` saved maps, recording
/// the edits the pane sends. `pending` is whether audio has yet to adopt them.
struct Edits {
    engine: TuningEngine,
    follow: Follow,
    maps: usize,
    pending: bool,
    edits: RefCell<Vec<MapEdit>>,
}
impl Edits {
    fn new(engine: TuningEngine, follow: Follow, maps: usize) -> Self {
        Self { engine, follow, maps, pending: false, edits: Default::default() }
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
        Some(MapView {
            playback: MapPlayback {
                engine: self.engine,
                follow: self.follow,
                map: Some(Default::default()),
                ..Default::default()
            },
            names: document.names(),
            offsets: Default::default(),
            followed: harmonigraph_core::LatticePos::ORIGIN,
            pending: self.pending,
            edit_shape: false,
            can_undo: false,
            full: false,
        })
    }
    fn edit_lattice_map(&self, edit: MapEdit) {
        self.edits.borrow_mut().push(edit);
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

/// Where the more button's three dots are painted: the middle of the three
/// small circles, which is the button's centre.
fn dots_at(out: &egui::FullOutput) -> Option<egui::Pos2> {
    let dots: Vec<_> = out
        .shapes
        .iter()
        .filter_map(|shape| match &shape.shape {
            egui::Shape::Circle(c) if (c.radius - 1.3).abs() < 0.01 => Some(c.center),
            _ => None,
        })
        .collect();
    (dots.len() == 3).then(|| dots[1])
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
    let params = Edits::new(TuningEngine::LatticeMap, Follow::Off, 1);
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
    for (label, expected) in
        [("Edit shape on lattice", "EditShape(true)"), ("Duplicate as new map", "Duplicate")]
    {
        let at = text_at(&out, label).unwrap_or_else(|| panic!("{label} must be visible"));
        click(&mut frame, at);
        let last = params.edits.borrow_mut().pop().map(|edit| format!("{edit:?}"));
        assert_eq!(last.as_deref(), Some(expected), "{label}");
    }
}

/// With Learn on, whether the pane draws the armed ring at `now`.
fn learn_ring_drawn(engine: TuningEngine) -> bool {
    let ctx = themed();
    let params = Edits::new(engine, Follow::Off, 1);
    let mut state = fresh_picture();
    state.runtime.learn_active = true;
    let size = egui::vec2(420.0, 2000.0);
    let now = 0.0;
    let mut frame = || {
        events_into(&ctx, size, egui::Rect::from_min_size(egui::Pos2::ZERO, size), vec![], |ui| {
            tuning_pane(ui, &mut state, &params, now);
        })
    };
    // The pane learns the engine from the previous frame's view.
    frame();
    let ring = theme::armed().gamma_multiply(learn_pulse(now));
    frame().shapes.iter().any(|shape| {
        matches!(&shape.shape, egui::Shape::Rect(rect)
            if rect.stroke.color == ring && rect.stroke.width == theme::LINE_WIDTH)
    })
}

#[test]
fn the_learn_ring_is_drawn_only_where_learn_acts() {
    assert!(learn_ring_drawn(TuningEngine::Off), "Pass through learns, so the ring pulses");
    assert!(!learn_ring_drawn(TuningEngine::LatticeMap), "Lattice Map suspends Learn");
}

/// Delete waits behind the dots and a confirming submenu, and the last saved
/// map offers neither.
#[test]
fn the_last_saved_map_cannot_be_deleted() {
    for (maps, expected) in [(1, None), (2, Some("Delete(0)"))] {
        let ctx = themed();
        ctx.all_styles_mut(|style| style.animation_time = 0.0);
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
        let dots = dots_at(&frame(vec![])).expect("the more button");
        click(&mut frame, dots);
        let delete = text_at(&frame(vec![]), "Delete map").expect("the open menu");
        click(&mut frame, delete);
        let confirm = text_at(&frame(vec![]), "Delete “");
        assert_eq!(confirm.is_some(), maps > 1, "{maps} saved maps: the confirmation");
        if let Some(confirm) = confirm {
            click(&mut frame, confirm);
            let after = frame(vec![]);
            assert!(text_at(&after, "Delete map").is_none(), "confirming must close the menu");
        }
        let sent = params.edits.borrow().last().map(|edit| format!("{edit:?}"));
        assert_eq!(sent.as_deref(), expected, "{maps} saved maps");
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
    let mut frame = || {
        events_into(&ctx, size, egui::Rect::from_min_size(egui::Pos2::ZERO, size), vec![], |ui| {
            tuning_pane(ui, &mut state, &params, 0.0)
        })
    };
    frame();
    let out = frame();
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
    /// A click on the selector's number, drawn left of the field and so
    /// before it in the frame.
    Number,
    /// Cmd-click on a bar above, which opens a typed entry that takes the
    /// keyboard before the name field is drawn.
    TypeIntoBar,
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
    ] {
        let ctx = themed();
        let params = Edits::new(TuningEngine::LatticeMap, Follow::Off, 1);
        let mut state = fresh_picture();
        let size = egui::vec2(420.0, 2000.0);
        let mut frame = |modifiers, events| {
            crate::tests::probe::held_into(
                &ctx,
                size,
                egui::Rect::from_min_size(egui::Pos2::ZERO, size),
                modifiers,
                events,
                |ui| {
                    tuning_pane(ui, &mut state, &params, 0.0);
                },
            )
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
