use super::*;
use crate::lattice_maps::{MapDocument, MapEdit, MapPlayback, MapView};
use crate::tests::probe::{events_into, fresh_picture, press, themed};
use harmonigraph_core::lattice_map::TuningEngine;
use std::cell::Cell;

struct Audition(Cell<bool>);
impl ParamBackend for Audition {
    fn get(&self, key: ParamKey) -> f32 {
        key.default_value()
    }
    fn set(&self, _: ParamKey, _: f32) {}
    fn lattice_maps(&self) -> Option<MapView> {
        Some(MapView {
            playback: MapPlayback {
                engine: TuningEngine::LatticeMap,
                map: Some(Default::default()),
                audition: self.0.get(),
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
        assert!(matches!(edit, MapEdit::Return));
        self.0.set(false);
    }
}

#[test]
fn audition_can_be_seen_and_ended_with_map_editing_folded() {
    // A fresh context reproduces reopening the editor while audition remains active.
    let ctx = themed();
    let params = Audition(Cell::new(true));
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
    assert!(text(&out, "Saved map").is_some());
    assert!(text(&out, "Map editing").is_some());
    assert!(text(&out, "Audition working copy").is_none(), "editing details must be folded");
    assert!(text(&out, "Audition shape").is_some(), "the selected map is overridden: explain why");
    let at = text(&out, "Return to arrangement").expect("audition needs a visible exit");
    frame(vec![egui::Event::PointerMoved(at)]);
    frame(vec![press(at, true)]);
    frame(vec![press(at, false)]);
    assert!(!params.0.get(), "Return must end audition without opening the editing fold");
    assert!(text(&frame(vec![]), "Audition shape").is_none());
}
