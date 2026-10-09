use super::*;
use crate::params::{InstanceEdit, TuningInstance, TuningVoice};
use crate::tests::probe::{events_into, press, themed};
use std::cell::RefCell;

// Geometry and interaction probes use instant fades; timing has its own probe.
fn fresh_picture() -> PictureState {
    let mut state = crate::tests::probe::fresh_picture();
    state.runtime.frame_params.fade_time = 0.0;
    state
}

struct Instances(RefCell<Vec<TuningInstance>>);
impl Instances {
    fn new() -> Self {
        Self(RefCell::new(crate::tests::probe::tuning_instances()))
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
    for width in [crate::theme::SETTINGS_MIN_CONTENT, 300.0] {
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
            let label = if retune { "Retune" } else { "Show" };
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
        // 190 is under the settings floor on purpose: a window too narrow to
        // hold the floor (1.5x scale, pictures folded, 400pt) still gets there.
        for width in [190.0, crate::theme::SETTINGS_MIN_CONTENT, 240.0, 300.0, 420.0] {
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
fn instance_voice_dots_fit_below_names_without_moving_them() {
    for width in [crate::theme::SETTINGS_MIN_CONTENT, 300.0] {
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
            for (dot, pitch) in below.iter().zip(&row.voices) {
                assert_eq!(dot.fill, note_color(&state, pitch.pitch, 1.0));
                assert!(
                    (dot.center.y - dot.radius - ink_bottom - 2.0).abs() < 0.1,
                    "the visible gap below {} must be two pixels",
                    row.display_name
                );
            }
            assert_eq!(
                below.len() as u64,
                row.voices.len() as u64,
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
                row.voices = (0..held)
                    .map(|n| TuningVoice { key: n as u16, pitch: 48.0 + n as f32 })
                    .collect();
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
                    instance_voice_dots(
                        ui,
                        strip,
                        &vec![VoiceDot { pitch: 60.0, level: 1.0 }; held as usize],
                        &state,
                    );
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
                    &pitches.map(|pitch| VoiceDot { pitch, level: 1.0 }),
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

#[test]
fn source_dots_carry_the_lattice_envelope_through_bends_releases_and_retriggers() {
    let mut state = crate::tests::probe::fresh_picture();
    state.runtime.frame_params.fade_time = 1.0;
    let env = state.appearance.view.envelope(&state.runtime.frame_params);
    let mut dots = SourceDots::default();
    let mut voices = [TuningVoice { key: 60, pitch: 60.0 }, TuningVoice { key: 188, pitch: 60.0 }];
    assert!(dots.step(&voices, 0.0, &env));
    assert!(dots.step(&voices, 0.1, &env));
    let arriving = env.carried(0.0, 0.1, true);
    assert!(arriving > 0.0 && arriving < 1.0);
    assert_eq!(dots.dots[&60].level, arriving);
    assert_eq!(dots.dots.len(), 2, "unisons on different channels keep separate dots");
    voices[0].pitch = 60.7;
    dots.step(&voices[..1], 0.1, &env);
    assert_eq!(dots.dots[&60].level, arriving, "same-frame passes do not advance twice");
    assert_eq!(dots.dots[&60].pitch, 60.7, "bends update color without restarting arrival");
    dots.step(&voices[..1], 0.15, &env);
    let departing = env.carried(arriving, 0.05, false);
    assert!((dots.dots[&188].level - departing).abs() < 1e-6);
    assert!(
        departing > 0.0 && departing < arriving,
        "a rapid release fades instead of blinking off"
    );
    dots.step(&voices, 0.2, &env);
    assert!(
        (dots.dots[&188].level - env.carried(departing, 0.05, true)).abs() < 1e-6,
        "a repeated note reverses continuously from its fading level"
    );
    dots.step(&voices, 2.0, &env);
    assert!(dots.dots.values().all(|dot| dot.level == 1.0));
    dots.step(&[], 2.1, &env);
    assert!(dots.dots.values().all(|dot| dot.level > 0.0 && dot.level < 1.0));
    assert!(!dots.step(&[], 4.0, &env));
    assert!(dots.dots.is_empty(), "completed tails are discarded");
    let instant =
        harmonigraph_core::Envelope { attack_time: 0.0, fade_time: 0.0, shape: env.shape };
    assert!(!dots.step(&voices, 4.0, &instant));
    assert!(dots.dots.values().all(|dot| dot.level == 1.0));
    assert!(!dots.step(&[], 4.0, &instant));
    assert!(dots.dots.is_empty(), "zero-duration fades remain immediate");
}

#[test]
fn fading_dots_use_opacity_and_give_up_space_continuously() {
    let state = fresh_picture();
    let ctx = themed();
    let size = egui::vec2(160.0, 40.0);
    let mut previous = None;
    for level in [1.0, 0.5, 0.0] {
        let voices = [VoiceDot { pitch: 48.0, level }, VoiceDot { pitch: 60.0, level: 1.0 }];
        let output = events_into(
            &ctx,
            size,
            egui::Rect::from_min_size(egui::Pos2::ZERO, size),
            vec![],
            |ui| {
                instance_voice_dots(
                    ui,
                    egui::Rect::from_min_size(egui::Pos2::ZERO, size),
                    &voices,
                    &state,
                );
            },
        );
        let circles: Vec<_> = output
            .shapes
            .iter()
            .filter_map(|shape| match &shape.shape {
                egui::Shape::Circle(circle) => Some(circle),
                _ => None,
            })
            .collect();
        let held = circles.last().unwrap();
        if let Some(x) = previous {
            assert!(held.center.x < x, "the surviving dot moves toward center during the fade");
        }
        previous = Some(held.center.x);
        if level > 0.0 {
            assert_eq!(circles[0].fill, note_color(&state, 48.0, level));
        } else {
            assert_eq!(
                held.center.x,
                size.x / 2.0,
                "removing the zero-width tail cannot shift the survivor"
            );
        }
    }
}

#[test]
fn source_button_rows_use_the_slider_gap() {
    let params = Instances::new();
    let state = fresh_picture();
    let ctx = themed();
    let size = egui::vec2(300.0, 800.0);
    let mut gap = 0.0;
    let mut frame = || {
        events_into(&ctx, size, egui::Rect::from_min_size(egui::Pos2::ZERO, size), vec![], |ui| {
            gap = ui.spacing().item_spacing.y;
            instance_section(ui, &state, &params);
        })
    };
    frame();
    let output = frame();
    let buttons: Vec<_> = params
        .0
        .borrow()
        .iter()
        .map(|row| {
            output
                .shapes
                .iter()
                .find_map(|shape| match &shape.shape {
                    egui::Shape::Text(text) if text.galley.text() == row.display_name => {
                        let ink = text.galley.mesh_bounds.translate(text.pos.to_vec2());
                        Some((ink.top() - 3.0, ink.bottom() + 7.5))
                    }
                    _ => None,
                })
                .unwrap()
        })
        .collect();
    for pair in buttons.windows(2) {
        assert!(
            (pair[1].0 - pair[0].1 - gap).abs() < 0.1,
            "source buttons {buttons:?} must use the slider gap of {gap}"
        );
    }
}

#[test]
fn removing_a_faded_dot_at_overflow_does_not_jump_the_survivors() {
    let state = fresh_picture();
    let ctx = themed();
    let size = egui::vec2(60.0, 4.0); // Ten slots: eleven voices really overflow.
    let mut voices: Vec<_> =
        (0..11).map(|key| VoiceDot { pitch: 36.0 + key as f32 * 4.0, level: 1.0 }).collect();
    let paint = |voices: &[VoiceDot]| {
        events_into(&ctx, size, egui::Rect::from_min_size(egui::Pos2::ZERO, size), vec![], |ui| {
            instance_voice_dots(
                ui,
                egui::Rect::from_min_size(egui::Pos2::ZERO, size),
                voices,
                &state,
            );
        })
        .shapes
        .into_iter()
        .filter_map(|shape| match shape.shape {
            egui::Shape::Circle(circle) => Some(circle),
            _ => None,
        })
        .collect::<Vec<_>>()
    };
    let full = paint(&voices);
    assert_eq!(full.iter().filter(|circle| circle.radius < 1.0).count(), 3);
    voices[0].level = 0.00001;
    let before = paint(&voices);
    voices.remove(0);
    let after = paint(&voices);
    for pitch in [40.0, 44.0, 48.0] {
        let color = note_color(&state, pitch, 1.0);
        let x = |circles: &[egui::epaint::CircleShape]| {
            circles
                .iter()
                .find(|circle| circle.radius > 1.0 && circle.fill == color)
                .unwrap()
                .center
                .x
        };
        assert!(
            (x(&before) - x(&after)).abs() < 0.01,
            "removing a nearly invisible tail moved held pitch {pitch}: {} -> {}",
            x(&before),
            x(&after)
        );
    }
    assert_eq!(after.len(), 10, "the former overflow voices become ordinary dots");
}
