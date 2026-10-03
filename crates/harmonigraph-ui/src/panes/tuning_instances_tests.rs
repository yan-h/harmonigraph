use super::*;
use crate::params::{InstanceEdit, TuningInstance};
use crate::tests::probe::{events_into, fresh_picture, press, themed};
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
                    pitches: (0..=id).map(|n| 48.0 + n as f32 * 12.0).collect(),
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
        let state = fresh_picture();
        let ctx = themed();
        let size = egui::vec2(width, 800.0);
        let frame = |events| {
            events_into(
                &ctx,
                size,
                egui::Rect::from_min_size(egui::Pos2::ZERO, size),
                events,
                |ui| instance_section(ui, &state, &params),
            )
        };
        frame(vec![]);
        for retune in [true, false] {
            let label = match (width < 220.0, retune) {
                (true, true) => "Retune all",
                (true, false) => "Show all",
                (false, true) => "Retune",
                (false, false) => "Show",
            };
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
                    .map(|row| if retune { row.show } else { row.retune })
                    .collect();
                frame(vec![egui::Event::PointerMoved(at)]);
                frame(vec![press(at, true)]);
                frame(vec![press(at, false)]);
                let rows = params.0.borrow();
                assert!(rows.iter().all(|row| if retune {
                    row.retune == enabled
                } else {
                    row.show == enabled
                }));
                let after: Vec<_> =
                    rows.iter().map(|row| if retune { row.show } else { row.retune }).collect();
                assert_eq!(after, untouched, "the other switch remains independent");
            }
        }
    }
}

#[test]
fn live_instance_controls_fit_a_narrow_settings_column() {
    for selected in [0, 1] {
        let params = Instances::new();
        let state = fresh_picture();
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
                        instance_section(ui, &state, &params);
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
                    egui::Shape::Text(text) if text.galley.text() == "Source details" => {
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
                let labels = [heading_x("Retune"), heading_x("Show")];
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
                assert_eq!(
                    boxes.len(),
                    8,
                    "the column headers and three rows each paint both flags"
                );
                let columns = [boxes[0].left(), boxes[1].left()];
                for (label, checkbox) in labels.iter().zip(&boxes[..2]) {
                    assert!(*label > checkbox.right(), "each header names its bulk checkbox");
                }
                assert!(!shapes.iter().any(|shape| matches!(&shape.shape,
                    egui::Shape::Text(text) if matches!(text.galley.text(), "Retune all" | "Show all"))),
                    "the table integrates bulk controls into its headers");
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
        let state = fresh_picture();
        let ctx = themed();
        let size = egui::vec2(width, 900.0);
        let frame = |events| {
            events_into(
                &ctx,
                size,
                egui::Rect::from_min_size(egui::Pos2::ZERO, size),
                events,
                |ui| instance_section(ui, &state, &params),
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
fn instance_voice_dots_fit_below_names_without_moving_them() {
    for width in [120.0, 300.0] {
        let params = Instances::new();
        let state = fresh_picture();
        let ctx = themed();
        let size = egui::vec2(width, 800.0);
        let frame = || {
            events_into(
                &ctx,
                size,
                egui::Rect::from_min_size(egui::Pos2::ZERO, size),
                vec![],
                |ui| instance_section(ui, &state, &params),
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
        let dots: Vec<_> = output
            .shapes
            .iter()
            .filter_map(|shape| match &shape.shape {
                egui::Shape::Circle(circle) => Some(circle),
                _ => None,
            })
            .collect();
        assert_eq!(dots.len(), 6, "the three rows have one, two and three voices");
        for row in params.0.borrow().iter() {
            let ink_bottom = output
                .shapes
                .iter()
                .find_map(|shape| match &shape.shape {
                    egui::Shape::Text(text) if text.galley.text() == row.display_name => {
                        Some(text.pos.y + text.galley.mesh_bounds.bottom())
                    }
                    _ => None,
                })
                .unwrap();
            let below: Vec<_> = dots
                .iter()
                .filter(|dot| dot.center.y > ink_bottom && dot.center.y < ink_bottom + 10.0)
                .collect();
            for (dot, pitch) in below.iter().zip(&row.pitches) {
                assert_eq!(dot.fill, note_color(&state, *pitch, 1.0));
                assert!(
                    (dot.center.y - dot.radius - ink_bottom - 2.0).abs() < 0.1,
                    "the visible gap below {} must be two pixels",
                    row.display_name
                );
            }
            assert_eq!(
                below.len() as u64,
                row.pitches.len() as u64,
                "dots sit tightly beneath {}",
                row.display_name
            );
        }
        assert!(
            !output.shapes.iter().any(|shape| matches!(&shape.shape,
            egui::Shape::Text(text) if text.galley.text().parse::<u64>().is_ok())),
            "no count occupies name space"
        );
        assert!(!output.shapes.iter().any(|shape| matches!(&shape.shape, egui::Shape::Text(text) if text.galley.text().contains(" held") || text.galley.text().contains(" out"))));
        let positions: Vec<_> = params
            .0
            .borrow()
            .iter()
            .map(|row| (row.display_name.clone(), text_bounds(&row.display_name).min))
            .collect();
        for held in [0, 9, 10, 64, 1] {
            for row in params.0.borrow_mut().iter_mut() {
                row.pitches = (0..held).map(|n| 48.0 + n as f32).collect();
            }
            let output = frame();
            for (name, before) in &positions {
                let after = output
                    .shapes
                    .iter()
                    .find_map(|shape| match &shape.shape {
                        egui::Shape::Text(text) if text.galley.text() == name => Some(text.pos),
                        _ => None,
                    })
                    .expect("the instance name remains visible");
                assert_eq!(after, *before, "{name} moved at {width}px with {held} notes");
            }
        }
    }
}

#[test]
fn voice_dot_overflow_uses_an_ellipsis_in_the_same_strip() {
    use crate::tests::probe::themed_scaled;
    for scale in [0.75, 1.0, 1.5] {
        let ctx = themed_scaled(scale);
        let state = fresh_picture();
        let size = egui::vec2(200.0 * scale, 100.0 * scale);
        let mut strip = egui::Rect::NOTHING;
        for held in [0, 3, 64] {
            let output = events_into(
                &ctx,
                size,
                egui::Rect::from_min_size(egui::Pos2::ZERO, size),
                vec![],
                |ui| {
                    strip = ui
                        .allocate_exact_size(
                            egui::vec2(60.0 * scale, 4.0 * scale),
                            egui::Sense::hover(),
                        )
                        .0;
                    instance_voice_dots(ui, strip, &vec![60.0; held as usize], &state);
                },
            );
            assert_eq!(strip.size(), egui::vec2(60.0 * scale, 4.0 * scale));
            let circles: Vec<_> = output
                .shapes
                .iter()
                .filter_map(|shape| match &shape.shape {
                    egui::Shape::Circle(circle) => Some(circle),
                    _ => None,
                })
                .collect();
            if !circles.is_empty() {
                let left = circles
                    .iter()
                    .map(|circle| circle.center.x - circle.radius)
                    .fold(f32::INFINITY, f32::min);
                let right = circles
                    .iter()
                    .map(|circle| circle.center.x + circle.radius)
                    .fold(f32::NEG_INFINITY, f32::max);
                assert!(
                    ((left + right) * 0.5 - strip.center().x).abs() < 0.01,
                    "the complete dot group is centered, including overflow"
                );
            }
            for circle in &circles {
                assert!(
                    strip.contains_rect(egui::Rect::from_center_size(
                        circle.center,
                        egui::Vec2::splat(2.0 * circle.radius)
                    )),
                    "dots must stay within the strip"
                );
            }
            if held < 64 {
                assert_eq!(circles.len() as u64, held);
            } else {
                assert!(circles.len() < 64, "the fixture must reach overflow");
                let small: Vec<_> = circles.iter().filter(|circle| circle.radius < scale).collect();
                assert_eq!(small.len(), 3, "overflow is an ellipsis");
                assert!(small.iter().all(|circle| circle.center.y == strip.center().y));
            }
        }
    }
}

#[test]
fn clicking_the_voice_dots_selects_the_source_button() {
    for width in [120.0, 300.0] {
        let params = Instances::new();
        let state = fresh_picture();
        let ctx = themed();
        let size = egui::vec2(width, 800.0);
        let selected = std::cell::Cell::new(0);
        let frame = |events| {
            events_into(
                &ctx,
                size,
                egui::Rect::from_min_size(egui::Pos2::ZERO, size),
                events,
                |ui| {
                    instance_controls(ui, &state, &params, &params.tuning_instances());
                    selected.set(
                        ui.data(|data| {
                            data.get_temp::<u64>(ui.id().with("tuning-instance-selection"))
                        })
                        .unwrap(),
                    );
                },
            )
        };
        frame(vec![]);
        let output = frame(vec![]);
        let dots: Vec<_> = output
            .shapes
            .iter()
            .filter_map(|shape| match &shape.shape {
                egui::Shape::Circle(circle) => Some(circle.center),
                _ => None,
            })
            .collect();
        assert_eq!(dots.len(), 6);
        let at = dots[1]; // The first dot in Bass, after the Hub's one dot.
        assert_eq!(selected.get(), 0);
        frame(vec![egui::Event::PointerMoved(at)]);
        frame(vec![press(at, true)]);
        frame(vec![press(at, false)]);
        assert_eq!(selected.get(), 1, "clicking a dot selects Bass at {width}px");
        let output = frame(vec![]);
        let ink = output
            .shapes
            .iter()
            .find_map(|shape| match &shape.shape {
                egui::Shape::Text(text) if text.galley.text() == "Bass" => {
                    Some(text.galley.mesh_bounds.translate(text.pos.to_vec2()))
                }
                _ => None,
            })
            .unwrap();
        let button = output
            .shapes
            .iter()
            .find_map(|shape| match &shape.shape {
                egui::Shape::Rect(rect) if rect.rect.contains(at) && rect.rect.height() < 40.0 => {
                    Some(rect.rect)
                }
                _ => None,
            })
            .expect("the selected button background includes its dots");
        assert!((ink.top() - button.top() - 3.0).abs() < 0.1, "3px above the lettering");
        assert!((button.bottom() - (at.y + 1.25) - 3.0).abs() < 0.1, "3px below the dots");
    }
}

#[test]
fn voice_dots_follow_the_current_pitch_palette() {
    let ctx = themed();
    let mut state = fresh_picture();
    let pitches = [36.0, 60.0, 84.0];
    let size = egui::vec2(160.0, 40.0);
    for (low, high) in [(24.0, 96.0), (48.0, 72.0)] {
        state.runtime.frame_params.darkest_pitch = low;
        state.runtime.frame_params.brightest_pitch = high;
        let output = events_into(
            &ctx,
            size,
            egui::Rect::from_min_size(egui::Pos2::ZERO, size),
            vec![],
            |ui| {
                instance_voice_dots(
                    ui,
                    egui::Rect::from_min_size(egui::pos2(0.0, 0.0), egui::vec2(120.0, 4.0)),
                    &pitches,
                    &state,
                );
            },
        );
        let colors: Vec<_> = output
            .shapes
            .iter()
            .filter_map(|shape| match &shape.shape {
                egui::Shape::Circle(circle) => Some(circle.fill),
                _ => None,
            })
            .collect();
        assert_eq!(colors, pitches.map(|pitch| note_color(&state, pitch, 1.0)));
        assert_ne!(colors[0], colors[2], "different pitches must have different colors");
    }
}
