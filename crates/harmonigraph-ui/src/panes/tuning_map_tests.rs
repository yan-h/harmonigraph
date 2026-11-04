use super::*;
use crate::lattice_maps::{MapDocument, MapEdit, MapPlayback, MapView};
use crate::tests::probe::{events_into, fresh_picture, press, themed};
use harmonigraph_core::lattice_map::{Follow, TuningEngine};
use std::cell::RefCell;

/// A Hub with `engine` and `follow` selected and `maps` saved maps, recording
/// the edits the pane sends.
struct Edits {
    engine: TuningEngine,
    follow: Follow,
    maps: usize,
    edits: RefCell<Vec<MapEdit>>,
}
impl Edits {
    fn new(engine: TuningEngine, follow: Follow, maps: usize) -> Self {
        Self { engine, follow, maps, edits: Default::default() }
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
            pending: false,
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
        }
        let sent = params.edits.borrow().last().map(|edit| format!("{edit:?}"));
        assert_eq!(sent.as_deref(), expected, "{maps} saved maps");
    }
}

/// The name field renames on Enter, never on a keystroke, since each rename
/// has the host reread the Map parameter's names; Escape keeps the old name.
#[test]
fn the_name_field_renames_on_enter_and_not_while_typing() {
    for (key, expected) in
        [(egui::Key::Enter, Some("Rename(0, \"C · 3×4 B\")")), (egui::Key::Escape, None)]
    {
        let ctx = themed();
        let params = Edits::new(TuningEngine::LatticeMap, Follow::Off, 1);
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
        let field = text_at(&frame(vec![]), "C · 3×4").expect("the name field");
        click(&mut frame, field);
        // Move to the end, wherever the click put the cursor.
        let key_event = |key| egui::Event::Key {
            key,
            physical_key: None,
            pressed: true,
            repeat: false,
            modifiers: egui::Modifiers::NONE,
        };
        frame(vec![key_event(egui::Key::End)]);
        frame(vec![egui::Event::Text(" B".into())]);
        assert!(text_at(&frame(vec![]), "C · 3×4 B").is_some(), "the typing must reach the field");
        assert!(params.edits.borrow().is_empty(), "typing sent {:?}", params.edits.borrow());
        frame(vec![key_event(key)]);
        frame(vec![]);
        let sent = params.edits.borrow().last().map(|edit| format!("{edit:?}"));
        assert_eq!(sent.as_deref(), expected, "{key:?}");
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
