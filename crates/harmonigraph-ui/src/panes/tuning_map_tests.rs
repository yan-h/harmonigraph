use super::*;
use crate::lattice_maps::{MapDocument, MapEdit, MapPlayback, MapView};
use crate::tests::probe::{events_into, fresh_picture, press, themed};
use harmonigraph_core::lattice_map::TuningEngine;
use std::cell::RefCell;

#[derive(Default)]
struct Edits(RefCell<Vec<MapEdit>>);
impl ParamBackend for Edits {
    fn get(&self, key: ParamKey) -> f32 {
        key.default_value()
    }
    fn set(&self, _: ParamKey, _: f32) {}
    fn lattice_maps(&self) -> Option<MapView> {
        Some(MapView {
            playback: MapPlayback {
                engine: TuningEngine::LatticeMap,
                map: Some(Default::default()),
                ..Default::default()
            },
            names: MapDocument::default().names(),
            offsets: Default::default(),
            pending: false,
            edit_shape: false,
            can_undo: false,
            full: false,
        })
    }
    fn edit_lattice_map(&self, edit: MapEdit) {
        self.0.borrow_mut().push(edit);
    }
}

#[test]
fn shape_editing_and_duplicate_need_no_fold_opened() {
    // A fresh context has every subsection at its default, folded.
    let ctx = themed();
    let params = Edits::default();
    let mut state = fresh_picture();
    let size = egui::vec2(420.0, 1000.0);
    let mut frame = |events| {
        events_into(&ctx, size, egui::Rect::from_min_size(egui::Pos2::ZERO, size), events, |ui| {
            map_controls(ui, &mut state, &params);
        })
    };
    let text = |out: &egui::FullOutput, needle: &str| {
        out.shapes.iter().find_map(|shape| match &shape.shape {
            egui::Shape::Text(text) if text.galley.text().starts_with(needle) => {
                Some(text.pos + egui::vec2(4.0, 4.0))
            }
            _ => None,
        })
    };
    frame(vec![]);
    let out = frame(vec![]);
    assert!(text(&out, "Map offsets").is_some());
    assert!(text(&out, "Fifths").is_none(), "the fixture must leave the offsets folded");
    for (label, expected) in
        [("Edit shape on lattice", "EditShape(true)"), ("Duplicate as new map", "Duplicate")]
    {
        let at = text(&out, label).unwrap_or_else(|| panic!("{label} must be visible"));
        frame(vec![egui::Event::PointerMoved(at)]);
        frame(vec![press(at, true)]);
        frame(vec![press(at, false)]);
        let last = params.0.borrow_mut().pop().map(|edit| format!("{edit:?}"));
        assert_eq!(last.as_deref(), Some(expected), "{label}");
    }
}
