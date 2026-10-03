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
                    display_name: ["Harmonigraph input", "Bass", "Reference keyboard"][id as usize]
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
    for width in [88.0, 104.0, 300.0] {
        let params = Instances::new();
        let ctx = themed();
        let size = egui::vec2(width, 800.0);
        let frame = |events| {
            events_into(
                &ctx,
                size,
                egui::Rect::from_min_size(egui::Pos2::ZERO, size),
                events,
                |ui| instance_section(ui, &params),
            )
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
}

#[test]
fn live_instance_controls_fit_a_narrow_settings_column() {
    for selected in [0, 1] {
        let params = Instances::new();
        params.0.borrow_mut().rotate_left(selected);
        for width in [88.0, 104.0, 120.0, 160.0, 240.0, 300.0, 420.0] {
            let ctx = themed();
            let size = egui::vec2(width, 2400.0);
            let mut used = 0.0;
            ctx.all_styles_mut(|style| style.animation_time = 0.0);
            let mut frame = |events| {
                events_into(
                    &ctx,
                    size,
                    egui::Rect::from_min_size(egui::Pos2::ZERO, size),
                    events,
                    |ui| {
                        instance_section(ui, &params);
                        used = ui.min_rect().width();
                    },
                )
            };
            frame(vec![]);
            let output = frame(vec![]);
            let details = output
                .shapes
                .iter()
                .find_map(|shape| match &shape.shape {
                    egui::Shape::Text(text) if text.galley.text() == "Instance details" => {
                        Some(text.pos + egui::vec2(4.0, 4.0))
                    }
                    _ => None,
                })
                .expect("the selected instance has a details heading");
            frame(vec![egui::Event::PointerMoved(details)]);
            frame(vec![press(details, true)]);
            frame(vec![press(details, false)]);
            let output = frame(vec![]);
            let delay = output
                .shapes
                .iter()
                .find_map(|shape| match &shape.shape {
                    egui::Shape::Text(text) if text.galley.text() == "Tuning delay" => {
                        Some(text.pos + egui::vec2(4.0, 4.0))
                    }
                    _ => None,
                })
                .expect("the global delay heading is drawn");
            frame(vec![egui::Event::PointerMoved(delay)]);
            frame(vec![press(delay, true)]);
            frame(vec![press(delay, false)]);
            let shapes = frame(vec![]).shapes;
            assert!(
                shapes.iter().any(|shape| matches!(&shape.shape,
                egui::Shape::Text(text) if text.galley.text() == "Apply to all tuners")),
                "the global delay controls must actually be open at {width}px for {selected}"
            );
            if selected == 1 {
                assert!(
                    shapes.iter().any(|shape| matches!(&shape.shape,
                    egui::Shape::Text(text) if text.galley.text() == "Buffers of delay")),
                    "the selected tuner details must actually be open"
                );
            }
            // Controls must fit before clipping. TextEdit legitimately clips
            // its scrollable contents, so check only the text it actually paints.
            for shape in &shapes {
                let bounds = match &shape.shape {
                    egui::Shape::Rect(rect) => Some(rect.rect),
                    egui::Shape::Text(text) => Some(
                        egui::Rect::from_min_size(text.pos, text.galley.size())
                            .intersect(shape.clip_rect),
                    ),
                    _ => None,
                };
                if let Some(bounds) = bounds {
                    assert!(
                        bounds.left() >= -1.0 && bounds.right() <= width + 1.0,
                        "{width}px column, selected {selected}, paints outside its edge: {bounds:?}; nearby text {:?}",
                        shapes.iter().filter_map(|shape| match &shape.shape {
                            egui::Shape::Text(text) if (text.pos.y - bounds.top()).abs() < 30.0 => Some(text.galley.text()),
                            _ => None,
                        }).collect::<Vec<_>>()
                    );
                }
            }
            assert!(used <= width + 1.0, "{width}px column expanded to {used}px");
            if width >= 240.0 {
                let heading_x = |label: &str| {
                    shapes
                        .iter()
                        .find_map(|shape| match &shape.shape {
                            egui::Shape::Text(text) if text.galley.text() == label => {
                                Some(text.pos.x)
                            }
                            _ => None,
                        })
                        .expect("the wide table's flag heading is painted")
                };
                let columns = [heading_x("Retune"), heading_x("Show")];
                let boxes: Vec<_> = shapes
                    .iter()
                    .filter_map(|shape| match &shape.shape {
                        egui::Shape::Rect(rect)
                            if (rect.rect.width() - crate::widgets::CHECKBOX_BOX).abs() < 0.1
                                && (rect.rect.height() - crate::widgets::CHECKBOX_BOX).abs()
                                    < 0.1 =>
                        {
                            Some(rect.rect)
                        }
                        _ => None,
                    })
                    .collect();
                assert_eq!(boxes.len(), 6, "the three live rows must each paint both flags");
                for checkbox in boxes {
                    assert!(
                        columns.iter().any(|x| (checkbox.left() - x).abs() < 1.0),
                        "checkbox {checkbox:?} is outside both flag columns {columns:?}"
                    );
                }
            }
        }
    }
}

#[test]
fn an_individual_show_flag_remains_reachable_in_a_narrow_column() {
    for width in [88.0, 104.0, 120.0] {
        let params = Instances::new();
        let ctx = themed();
        let size = egui::vec2(width, 900.0);
        let frame = |events| {
            events_into(
                &ctx,
                size,
                egui::Rect::from_min_size(egui::Pos2::ZERO, size),
                events,
                |ui| instance_section(ui, &params),
            )
        };
        frame(vec![]);
        let output = frame(vec![]);
        let labels: Vec<_> = output
            .shapes
            .iter()
            .filter_map(|shape| match &shape.shape {
                egui::Shape::Text(text) if text.galley.text() == "Show" => Some(text.pos),
                _ => None,
            })
            .collect();
        assert_eq!(labels.len(), 3, "each live instance must identify its own Show action");
        let at = labels[1] + egui::vec2(4.0, 4.0);
        assert!(at.x < width, "the Bass Show control was clipped out of reach");
        let before = params.0.borrow().clone();
        frame(vec![egui::Event::PointerMoved(at)]);
        frame(vec![press(at, true)]);
        frame(vec![press(at, false)]);
        let after = params.0.borrow();
        for (old, new) in before.iter().zip(after.iter()) {
            assert_eq!(new.retune, old.retune, "Show edited the independent Retune flag");
            assert_eq!(
                new.show,
                if old.id == 1 { !old.show } else { old.show },
                "the individual Show control did not edit just Bass at width {width}"
            );
        }
    }
}

#[test]
fn instance_note_counts_share_the_name_row() {
    for width in [120.0, 300.0] {
        let params = Instances::new();
        let ctx = themed();
        let size = egui::vec2(width, 800.0);
        let frame = || {
            events_into(
                &ctx,
                size,
                egui::Rect::from_min_size(egui::Pos2::ZERO, size),
                vec![],
                |ui| instance_section(ui, &params),
            )
        };
        frame();
        let output = frame();
        let text_bounds = |label: &str| {
            output
                .shapes
                .iter()
                .find_map(|shape| match &shape.shape {
                    egui::Shape::Text(text) if text.galley.text() == label => {
                        Some(egui::Rect::from_min_size(text.pos, text.galley.size()))
                    }
                    _ => None,
                })
                .unwrap_or_else(|| panic!("missing {label} at {width}px"))
        };
        for row in params.0.borrow().iter() {
            let name = text_bounds(&row.display_name);
            let count = text_bounds(&row.held.to_string());
            assert!(
                (name.center().y - count.center().y).abs() < 2.0,
                "the live count stays beside the name"
            );
        }
        assert!(!output.shapes.iter().any(|shape| matches!(&shape.shape, egui::Shape::Text(text) if text.galley.text().contains(" held") || text.galley.text().contains(" out"))));
    }
}
