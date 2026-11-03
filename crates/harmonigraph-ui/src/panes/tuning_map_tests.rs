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

#[test]
fn shape_editing_and_duplicate_need_no_fold_opened() {
    // A fresh context has every subsection at its default, folded.
    let ctx = themed();
    let params = Edits::new(TuningEngine::LatticeMap, Follow::Off, 1);
    let mut state = fresh_picture();
    let size = egui::vec2(420.0, 1000.0);
    let mut frame = |events| {
        events_into(&ctx, size, egui::Rect::from_min_size(egui::Pos2::ZERO, size), events, |ui| {
            map_controls(ui, &mut state, &params);
        })
    };
    frame(vec![]);
    let out = frame(vec![]);
    assert!(text_at(&out, "Map offsets").is_some());
    assert!(text_at(&out, "Fifths").is_none(), "the fixture must leave the offsets folded");
    for (label, expected) in
        [("Edit shape on lattice", "EditShape(true)"), ("Duplicate as new map", "Duplicate")]
    {
        let at = text_at(&out, label).unwrap_or_else(|| panic!("{label} must be visible"));
        frame(vec![egui::Event::PointerMoved(at)]);
        frame(vec![press(at, true)]);
        frame(vec![press(at, false)]);
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
    let size = egui::vec2(420.0, 1000.0);
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

#[test]
fn the_last_saved_map_cannot_be_deleted() {
    for (maps, expected) in [(1, None), (2, Some("Delete(0)"))] {
        let ctx = themed();
        ctx.all_styles_mut(|style| style.animation_time = 0.0);
        let params = Edits::new(TuningEngine::LatticeMap, Follow::Off, maps);
        let mut state = fresh_picture();
        let size = egui::vec2(420.0, 1000.0);
        let mut frame = |events| {
            events_into(
                &ctx,
                size,
                egui::Rect::from_min_size(egui::Pos2::ZERO, size),
                events,
                |ui| {
                    map_controls(ui, &mut state, &params);
                },
            )
        };
        frame(vec![]);
        let out = frame(vec![]);
        // The fold starts closed, so Delete is not painted until it opens.
        let header = text_at(&out, "Manage selected saved map").expect("the fold's header");
        for events in [
            vec![egui::Event::PointerMoved(header)],
            vec![press(header, true)],
            vec![press(header, false)],
        ] {
            frame(events);
        }
        let delete = text_at(&frame(vec![]), "Delete saved map").expect("the open fold");
        for events in [
            vec![egui::Event::PointerMoved(delete)],
            vec![press(delete, true)],
            vec![press(delete, false)],
        ] {
            frame(events);
        }
        let sent = params.edits.borrow().last().map(|edit| format!("{edit:?}"));
        assert_eq!(sent.as_deref(), expected, "{maps} saved maps");
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
        // The fold, then every control the context has, none of them behind a
        // fold that was never opened.
        let controls = [
            "Half-life",
            "Same-note tolerance",
            "Silence reset",
            "Reset context on stop",
            "Reset context on loop / seek",
        ];
        (text_at(&out, "Context").is_some(), controls.map(|label| text_at(&out, label).is_some()))
    };
    let (fold, controls) = drawn(TuningEngine::Adaptive, Follow::Off);
    assert!(fold, "Adaptive keeps its Context fold");
    assert_eq!(controls, [false; 5], "Adaptive's fold starts closed");
    let (fold, controls) = drawn(TuningEngine::LatticeMap, Follow::Thirds);
    assert!(!fold, "Follow's context has no fold of its own");
    assert_eq!(controls, [true; 5], "Follow reads the context, reachable without opening a fold");
    let (fold, controls) = drawn(TuningEngine::LatticeMap, Follow::Off);
    assert!(!fold && controls == [false; 5], "nothing reads it with Follow off");
}
