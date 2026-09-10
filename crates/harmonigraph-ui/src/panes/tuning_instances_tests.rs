use super::*;
use crate::params::{InstanceEdit, TuningInstance};
use crate::tests::probe::{events_into, press, themed};
use std::cell::RefCell;

struct Instances(RefCell<Vec<TuningInstance>>);
impl Instances {
    fn new() -> Self {
        Self(RefCell::new(
            (0..3)
                .map(|id| TuningInstance {
                    id,
                    name: ["Harmonigraph input", "Bass", "Reference keyboard"][id as usize]
                        .to_owned(),
                    is_hub: id == 0,
                    retune: id == 0,
                    show: id == 1,
                    held: id + 1,
                    notes_in: 34,
                    notes_out: 34,
                    misses: 0,
                    delay: 1,
                    max_delay: if id == 0 { 1 } else { 16 },
                    delay_text: "Tuning delay 1x buffer - 512 samples / 10.67 ms".to_owned(),
                    status: "No faults".to_owned(),
                    last_pitch: Some((6000.0, 5986.3)),
                })
                .collect(),
        ))
    }
}
impl ParamBackend for Instances {
    fn get(&self, key: ParamKey) -> f32 {
        key.default_value()
    }
    fn set(&self, _: ParamKey, _: f32) {}
    fn tuning_instances(&self) -> Vec<TuningInstance> {
        self.0.borrow().clone()
    }
    fn edit_tuning_instance(&self, id: u64, edit: InstanceEdit) {
        let mut rows = self.0.borrow_mut();
        let row = rows.iter_mut().find(|row| row.id == id).unwrap();
        match edit {
            InstanceEdit::Retune(value) => row.retune = value,
            InstanceEdit::Show(value) => row.show = value,
            _ => panic!("unexpected edit"),
        }
    }
}

#[test]
fn mixed_global_controls_enable_every_instance_then_disable_independently() {
    let params = Instances::new();
    let ctx = themed();
    let size = egui::vec2(300.0, 800.0);
    let frame = |events| {
        events_into(&ctx, size, egui::Rect::from_min_size(egui::Pos2::ZERO, size), events, |ui| {
            instance_controls(ui, &params)
        })
    };
    frame(vec![]);
    for label in ["Retune all", "Show all"] {
        for enabled in [true, false] {
            let output = frame(vec![]);
            let at = output
                .shapes
                .iter()
                .find_map(|shape| match &shape.shape {
                    egui::Shape::Text(text) if text.galley.text() == label => {
                        Some(text.pos + egui::vec2(4.0, 4.0))
                    }
                    _ => None,
                })
                .expect("the global control is drawn");
            let untouched: Vec<_> = params
                .0
                .borrow()
                .iter()
                .map(|row| if label == "Retune all" { row.show } else { row.retune })
                .collect();
            frame(vec![egui::Event::PointerMoved(at)]);
            frame(vec![press(at, true)]);
            frame(vec![press(at, false)]);
            let rows = params.0.borrow();
            assert!(rows.iter().all(|row| if label == "Retune all" {
                row.retune == enabled
            } else {
                row.show == enabled
            }));
            let after: Vec<_> = rows
                .iter()
                .map(|row| if label == "Retune all" { row.show } else { row.retune })
                .collect();
            assert_eq!(after, untouched, "the other switch remains independent");
        }
    }
}

#[test]
fn live_instance_controls_fit_a_narrow_settings_column() {
    let params = Instances::new();
    for width in [240.0, 300.0, 420.0] {
        let ctx = themed();
        let size = egui::vec2(width, 900.0);
        let mut used = 0.0;
        for _ in 0..2 {
            events_into(
                &ctx,
                size,
                egui::Rect::from_min_size(egui::Pos2::ZERO, size),
                vec![],
                |ui| {
                    instance_controls(ui, &params);
                    used = ui.min_rect().width();
                },
            );
        }
        assert!(used <= width + 1.0, "{width}px column expanded to {used}px");
    }
}
