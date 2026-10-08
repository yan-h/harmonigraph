//! Exercise the actual handles rather than only their mapping formulas.
use super::*;
use egui::{Event, Modifiers, PointerButton, Pos2, Ui, Vec2};

fn frame(
    ctx: &egui::Context,
    events: Vec<Event>,
    draw: &mut impl FnMut(&mut Ui),
) -> egui::FullOutput {
    ctx.run_ui(
        egui::RawInput {
            screen_rect: Some(egui::Rect::from_min_size(Pos2::ZERO, egui::vec2(320.0, 1000.0))),
            events,
            ..Default::default()
        },
        |ui| draw(ui),
    )
}
fn press(p: Pos2, pressed: bool) -> Event {
    Event::PointerButton {
        pos: p,
        button: PointerButton::Primary,
        pressed,
        modifiers: Modifiers::NONE,
    }
}
fn drag(mut draw: impl FnMut(&mut Ui), handle: usize, delta: Vec2) {
    let ctx = crate::tests::probe::themed_at(1.0);
    let output = frame(&ctx, vec![], &mut draw);
    let handles: Vec<_> = output
        .shapes
        .iter()
        .filter_map(|s| match &s.shape {
            egui::Shape::Circle(c)
                if (c.radius - 4.0).abs() < 0.01 && c.fill == crate::theme::accent() =>
            {
                Some(c.center)
            }
            _ => None,
        })
        .collect();
    let p = handles[handle];
    frame(&ctx, vec![Event::PointerMoved(p)], &mut draw);
    frame(&ctx, vec![press(p, true)], &mut draw);
    frame(&ctx, vec![Event::PointerMoved(p + delta)], &mut draw);
    frame(&ctx, vec![press(p + delta, false)], &mut draw);
}

#[test]
fn drift_edits_vector_and_compass_keeps_speed_independent() {
    let (mut angle, mut speed) = (0.0, 5.0);
    drag(|ui| drift(ui, &mut angle, Some(&mut speed)), 0, egui::vec2(-12.0, 20.0));
    assert!(angle > 30.0 && angle < 150.0, "{angle}");
    assert_ne!(speed, 5.0);
    let before = speed;
    drag(|ui| drift(ui, &mut angle, None), 0, egui::vec2(-20.0, -20.0));
    assert_eq!(speed, before);
}
#[test]
fn cabinet_endpoint_sets_angle_and_length_within_the_quadrant() {
    let (mut a, mut l) = (0.3, 0.4);
    drag(|ui| cabinet(ui, &mut a, &mut l), 0, egui::vec2(40.0, -60.0));
    assert!((0.0..=std::f32::consts::FRAC_PI_2).contains(&a));
    assert!(a > 0.3 && l > 0.4 && l <= 1.0);
}
#[test]
fn softness_handle_edits_both_axes() {
    let (mut p, mut t) = (20.0, 20.0);
    drag(|ui| softness(ui, &mut p, &mut t), 0, egui::vec2(30.0, -20.0));
    assert!(p > 20.0 && t > 20.0);
}
#[test]
fn threshold_can_keep_a_band_below_zero_without_changing_the_gate() {
    let (mut gate, mut band) = (0.05, 0.1);
    drag(
        |ui| {
            threshold(ui, &mut gate, &mut band);
        },
        1,
        egui::vec2(-80.0, 0.0),
    );
    assert_eq!(gate, 0.05);
    assert_eq!(band, harmonigraph_scene::SPECTRAL_HYSTERESIS_MAX);
}
#[test]
fn fade_duration_gesture_is_bracketed_and_shape_does_not_edit_time() {
    let (mut t, mut shape) = (0.5, 0.3);
    let (mut starts, mut stops) = (0, 0);
    drag(
        |ui| {
            let r = fade(ui, &mut t, &mut shape);
            starts += usize::from(r.drag_started());
            stops += usize::from(r.drag_stopped());
        },
        0,
        egui::vec2(45.0, 0.0),
    );
    assert!(t > 0.5);
    assert_eq!((starts, stops), (1, 1));
    let before = t;
    drag(
        |ui| {
            fade(ui, &mut t, &mut shape);
        },
        1,
        egui::vec2(0.0, 12.0),
    );
    assert_eq!(t, before);
    assert!(shape < 0.3);
}
#[test]
fn response_times_are_independent_and_zero_is_reachable() {
    let (mut rise, mut fall) = (0.4, 0.7);
    drag(
        |ui| {
            response(ui, &mut rise, &mut fall, 2.0, ["Rise", "Fall"], 1000.0);
        },
        0,
        egui::vec2(-150.0, 0.0),
    );
    assert_eq!(rise, 0.0);
    assert_eq!(fall, 0.7);
}
/// The first rect of `height` filled with the well colour: a plot's well or a
/// bar's track.
fn well(output: &egui::FullOutput, height: f32) -> egui::Rect {
    output
        .shapes
        .iter()
        .find_map(|s| match &s.shape {
            egui::Shape::Rect(r)
                if r.fill == crate::theme::well() && (r.rect.height() - height).abs() < 0.1 =>
            {
                Some(r.rect)
            }
            _ => None,
        })
        .unwrap()
}
fn press_drag(ctx: &egui::Context, draw: &mut impl FnMut(&mut Ui), from: Pos2, to: Pos2) {
    frame(ctx, vec![Event::PointerMoved(from)], draw);
    frame(ctx, vec![press(from, true)], draw);
    frame(ctx, vec![Event::PointerMoved(to)], draw);
    frame(ctx, vec![press(to, false)], draw);
}
#[test]
fn a_star_layer_dot_moves_its_own_layer_alone_and_idles_exactly() {
    for (kind, initial) in
        [(Depth::Speed, [0.1, 0.2, 0.4, 0.6, 0.8]), (Depth::Size, [0.7, 1.5, 4.0, 11.0, 32.0])]
    {
        let mut values = initial;
        let ctx = crate::tests::probe::themed_at(1.0);
        for _ in 0..8 {
            frame(&ctx, vec![], &mut |ui| depth(ui, &mut values, 5, kind, 1.0));
        }
        assert_eq!(values, initial);
        drag(|ui| depth(ui, &mut values, 5, kind, 1.0), 1, egui::vec2(0.0, -12.0));
        assert!(values[1] > initial[1]);
        assert_eq!([values[0], values[2], values[3], values[4]], [0, 2, 3, 4].map(|k| initial[k]));
    }
}
/// A line pressed out on the plot's empty space, top left to bottom right,
/// sets every drawn layer onto it, and a depth `Star layers` leaves out keeps
/// its own value.
#[test]
fn a_line_across_the_star_plot_sets_every_drawn_layer_onto_it() {
    let initial = [0.3, 0.4, 0.5, 0.6, 0.7];
    let mut values = initial;
    let ctx = crate::tests::probe::themed_at(1.0);
    let mut draw = |ui: &mut Ui| depth(ui, &mut values, 3, Depth::Speed, 1.0);
    let output = frame(&ctx, vec![], &mut draw);
    let inner = well(&output, super::plot::height(1.0)).shrink(6.0);
    let outside = egui::vec2(4.0, 4.0);
    press_drag(&ctx, &mut draw, inner.left_top() - outside, inner.right_bottom() + outside);
    for (k, want) in [(0, 1.0), (1, initial[1]), (2, 0.5), (3, initial[3]), (4, 0.0)] {
        assert!((values[k] - want).abs() < 1e-5, "layer {k}: {values:?}");
    }
}
/// A line let go of while the plot is not drawn leaves no press point
/// behind: the next line runs from its own press.
#[test]
fn a_line_lost_mid_drag_does_not_anchor_the_next_one() {
    let values = std::cell::RefCell::new([0.5; 5]);
    let ctx = crate::tests::probe::themed_at(1.0);
    let mut draw = |ui: &mut Ui| depth(ui, &mut values.borrow_mut(), 3, Depth::Speed, 1.0);
    let output = frame(&ctx, vec![], &mut draw);
    let inner = well(&output, super::plot::height(1.0)).shrink(6.0);
    let top_left = inner.left_top() - egui::vec2(4.0, 4.0);
    frame(&ctx, vec![Event::PointerMoved(top_left)], &mut draw);
    frame(&ctx, vec![press(top_left, true)], &mut draw);
    frame(&ctx, vec![Event::PointerMoved(inner.center())], &mut draw);
    frame(&ctx, vec![press(inner.center(), false)], &mut |_: &mut Ui| {});
    frame(&ctx, vec![], &mut draw);
    let low = |p: Pos2| p + egui::vec2(0.0, 4.0);
    let (from, to) = (low(inner.left_bottom()) - egui::vec2(4.0, 0.0), low(inner.right_bottom()));
    press_drag(&ctx, &mut draw, from, to + egui::vec2(4.0, 0.0));
    let after = *values.borrow();
    assert!([0, 2, 4].iter().all(|&k| after[k].abs() < 1e-5), "{after:?}");
}
/// The bar under the plot slides every drawn layer together, by the same
/// octaves for a size, and leaves a layer that is not drawn where it was.
#[test]
fn the_star_range_bar_moves_every_drawn_layer_together() {
    let initial = [1.0, 2.0, 4.0, 3.0, 8.0];
    let mut values = initial;
    let ctx = crate::tests::probe::themed_at(1.0);
    let mut draw = |ui: &mut Ui| depth(ui, &mut values, 3, Depth::Size, 1.0);
    let output = frame(&ctx, vec![], &mut draw);
    let bar = well(&output, crate::theme::ROW_HEIGHT);
    // The span's middle, 1 to 8 px in octaves on a track of 0.5 to 64.
    let x = |octaves: f32| bar.left() + bar.width() * (octaves + 1.0) / 7.0;
    let from = egui::pos2(x(1.5), bar.center().y);
    press_drag(&ctx, &mut draw, from, from + egui::vec2(15.0, 0.0));
    let shift = (values[0] / initial[0]).log2();
    assert!(shift > 0.1, "{values:?}");
    for k in [2, 4] {
        assert!(((values[k] / initial[k]).log2() - shift).abs() < 1e-4, "{values:?}");
    }
    assert_eq!((values[1], values[3]), (initial[1], initial[3]));
}
/// The bar keeps its own pair through a drag: an end dragged off level
/// layers comes back the way it went, rather than clamping against the end
/// that followed it, and a double click, which resets a range bar to its
/// whole axis, spreads no layer across it.
#[test]
fn the_star_range_bar_drags_level_layers_both_ways_and_ignores_a_double_click() {
    let values = std::cell::RefCell::new([0.5; 5]);
    let ctx = crate::tests::probe::themed_at(1.0);
    let mut draw = |ui: &mut Ui| depth(ui, &mut values.borrow_mut(), 5, Depth::Speed, 1.0);
    let output = frame(&ctx, vec![], &mut draw);
    let bar = well(&output, crate::theme::ROW_HEIGHT);
    // Just left of the level pair, which a closed span gives its low end.
    let at = egui::pos2(bar.center().x - 3.0, bar.center().y);
    let to = |dx: f32| at - egui::vec2(dx, 0.0);
    frame(&ctx, vec![Event::PointerMoved(at)], &mut draw);
    frame(&ctx, vec![press(at, true)], &mut draw);
    frame(&ctx, vec![Event::PointerMoved(to(40.0))], &mut draw);
    let out = values.borrow()[0];
    frame(&ctx, vec![Event::PointerMoved(to(20.0))], &mut draw);
    frame(&ctx, vec![press(to(20.0), false)], &mut draw);
    let back = *values.borrow();
    assert!(out < back[0] && back[0] < 0.5, "out to {out}, back to {back:?}");
    assert!(back.iter().all(|v| *v == back[0]), "{back:?}");
    frame(&ctx, vec![Event::PointerMoved(to(30.0))], &mut draw);
    for _ in 0..2 {
        frame(&ctx, vec![press(to(30.0), true)], &mut draw);
        frame(&ctx, vec![press(to(30.0), false)], &mut draw);
    }
    assert_eq!(*values.borrow(), back);
}
#[test]
fn shadow_sliders_update_the_preview_without_switching_kernel() {
    for kernel in
        [harmonigraph_scene::ShadowKernel::Distance, harmonigraph_scene::ShadowKernel::Gaussian]
    {
        let mut s = harmonigraph_scene::ShadowStyle {
            kernel,
            width: 0.2,
            depth: 0.3,
            ..Default::default()
        };
        let ctx = crate::tests::probe::themed_at(1.0);
        let mut draw = |ui: &mut Ui| shadow(ui, &mut s, 1.0, true);
        let mut output = frame(&ctx, vec![], &mut draw);
        for row in 0..3 {
            let colors = |output: &egui::FullOutput| {
                output
                    .shapes
                    .iter()
                    .find_map(|s| match &s.shape {
                        egui::Shape::Mesh(mesh) => {
                            Some(mesh.vertices.iter().map(|v| v.color).collect::<Vec<_>>())
                        }
                        _ => None,
                    })
                    .unwrap()
            };
            let before = colors(&output);
            let bars: Vec<_> = output
                .shapes
                .iter()
                .filter_map(|s| match &s.shape {
                    egui::Shape::Rect(r)
                        if r.fill == crate::theme::well()
                            && (r.rect.height() - crate::theme::ROW_HEIGHT).abs() < 0.1 =>
                    {
                        Some(r.rect)
                    }
                    _ => None,
                })
                .collect();
            let bar = bars[row];
            let from = bar.center();
            let to = egui::pos2(bar.left() + 0.8 * bar.width(), bar.center().y);
            frame(&ctx, vec![Event::PointerMoved(from)], &mut draw);
            frame(&ctx, vec![press(from, true)], &mut draw);
            frame(&ctx, vec![Event::PointerMoved(to)], &mut draw);
            output = frame(&ctx, vec![press(to, false)], &mut draw);
            assert_ne!(colors(&output), before, "{kernel:?}, row {row}");
        }
        assert!(s.width > 0.2 && s.depth > 0.3);
        assert_eq!(s.kernel, kernel);
    }
}

#[test]
fn spectrum_buttons_form_a_compact_rectangle_and_select_each_edge() {
    use crate::SpectralOrientation::{Bottom, Left, Right, Top};
    for scale in [0.7, 1.0, 1.5] {
        let ctx = crate::tests::probe::themed_scaled(scale);
        let mut edge = Left;
        let output = frame(&ctx, vec![], &mut |ui| spectrum_edge(ui, &mut edge));
        let controls = [(Left, "Left"), (Right, "Right"), (Top, "Top"), (Bottom, "Bottom")].map(
            |(value, label)| {
                let at = output
                    .shapes
                    .iter()
                    .find_map(|s| match &s.shape {
                        egui::Shape::Text(t) if t.galley.text() == label => {
                            Some(t.pos + t.galley.size() * 0.5)
                        }
                        _ => None,
                    })
                    .unwrap();
                let rect = output
                    .shapes
                    .iter()
                    .filter_map(|s| match &s.shape {
                        egui::Shape::Rect(r) if r.rect.contains(at) => Some(r.rect),
                        _ => None,
                    })
                    .min_by(|a, b| a.area().total_cmp(&b.area()))
                    .unwrap();
                (value, rect)
            },
        );
        let [(_, left), (_, right), (_, top), (_, bottom)] = controls;
        assert_eq!(left.top(), right.top());
        assert_eq!(left.bottom(), right.bottom());
        assert_eq!(top.top(), left.top());
        assert_eq!(bottom.bottom(), left.bottom());
        assert_eq!(top.left(), bottom.left());
        assert_eq!(top.right(), bottom.right());
        assert!(top.left() > left.right() && top.right() < right.left());
        assert!(top.bottom() < bottom.top());
        assert!((top.height() - bottom.height()).abs() < 0.1);
        assert!(left.height() > 2.0 * top.height() && left.height() < 2.4 * top.height());
        assert!(right.right() - left.left() <= 180.0 * scale);
        for (value, rect) in controls {
            let at = rect.center();
            {
                let mut draw = |ui: &mut Ui| spectrum_edge(ui, &mut edge);
                frame(&ctx, vec![Event::PointerMoved(at)], &mut draw);
                frame(&ctx, vec![press(at, true)], &mut draw);
                frame(&ctx, vec![press(at, false)], &mut draw);
            }
            assert_eq!(edge, value);
        }
    }
}

#[test]
fn color_popup_edits_its_real_skin_coordinates() {
    let ctx = crate::tests::probe::themed_at(1.0);
    let (mut hue, mut amount) = (180.0, 0.3);
    let mut draw = |ui: &mut Ui| {
        ui.set_max_width(120.0);
        skin_color(ui, "Accent", &mut hue, &mut amount, ["Hue", "Amount"], |h, s| {
            let [r, g, b] = harmonigraph_scene::skin::accent_color(h, s);
            egui::Color32::from_rgb(r, g, b)
        })
    };
    frame(&ctx, vec![], &mut draw);
    let click = egui::pos2(30.0, 10.0);
    frame(&ctx, vec![Event::PointerMoved(click)], &mut draw);
    frame(&ctx, vec![press(click, true)], &mut draw);
    frame(&ctx, vec![press(click, false)], &mut draw);
    let output = frame(&ctx, vec![], &mut draw);
    let handle = output
        .shapes
        .iter()
        .find_map(|s| match &s.shape {
            egui::Shape::Circle(c) if (c.radius - 4.0).abs() < 0.01 => Some(c.center),
            _ => None,
        })
        .expect("the color popup must actually be open");
    frame(&ctx, vec![Event::PointerMoved(handle)], &mut draw);
    frame(&ctx, vec![press(handle, true)], &mut draw);
    frame(&ctx, vec![Event::PointerMoved(handle + egui::vec2(30.0, -10.0))], &mut draw);
    frame(&ctx, vec![press(handle + egui::vec2(30.0, -10.0), false)], &mut draw);
    assert!(hue > 180.0 && amount > 0.3);
}

#[test]
fn numeric_entries_use_display_units_and_reject_non_finite_input() {
    for (unit, suffix, text, want) in [
        (1000.0, " ms", "250", 0.25),
        (100.0, "%", "75%", 0.75),
        (1.0, "×", "NaN", 0.5),
        (1.0, "×", "inf", 0.5),
    ] {
        let ctx = crate::tests::probe::themed_at(1.0);
        let mut value = 0.5;
        let field = std::cell::Cell::new(egui::Rect::NOTHING);
        let mut draw = |ui: &mut Ui| {
            let plot = plot::Plot::with_fields(ui, "Exact entry", 1);
            plot.fields(ui, |ui| {
                field.set(
                    plot::value_bar(ui, &mut value, 0.0..=1.0, ["Value", "Value"], unit, suffix)
                        .rect,
                );
            });
        };
        frame(&ctx, vec![], &mut draw);
        let at = field.get().center();
        frame(&ctx, vec![Event::PointerMoved(at)], &mut draw);
        for _ in 0..2 {
            frame(&ctx, vec![press(at, true)], &mut draw);
            frame(&ctx, vec![press(at, false)], &mut draw);
        }
        frame(&ctx, vec![], &mut draw);
        let key = |key, modifiers| Event::Key {
            key,
            physical_key: None,
            pressed: true,
            repeat: false,
            modifiers,
        };
        frame(&ctx, vec![key(egui::Key::A, Modifiers::COMMAND)], &mut draw);
        frame(
            &ctx,
            vec![Event::Text(text.to_owned()), key(egui::Key::Enter, Modifiers::NONE)],
            &mut draw,
        );
        assert!((value - want).abs() < 1e-6, "{text} stored {value}");
    }
}

#[test]
fn compact_values_fit_beside_the_picture() {
    for scale in [0.7, 1.0, 1.5] {
        for width in [crate::theme::SETTINGS_MIN_CONTENT, 240.0, 320.0] {
            let ctx = crate::tests::probe::themed_scaled(scale);
            let screen = egui::vec2(800.0, 500.0);
            let pane =
                egui::Rect::from_min_size(egui::pos2(20.0, 20.0), egui::vec2(width * scale, 450.0));
            for count in [1, 2, 3] {
                crate::tests::probe::frame_into(&ctx, screen, pane, |ui| {
                    let plot = plot::Plot::with_fields(ui, "Control", count);
                    plot.fields(ui, |ui| {
                        for i in 0..count {
                            let field = ui
                                .push_id(i, |ui| {
                                    plot::value_bar(
                                        ui,
                                        &mut 6.0,
                                        0.0..=6.0,
                                        ["Exact value", "Value"],
                                        1000.0,
                                        " ms",
                                    )
                                })
                                .inner;
                            assert!(
                                plot.response.rect.expand(0.1).contains_rect(field.rect),
                                "{width}/{scale}: value escaped control"
                            );
                            assert!(
                                !plot.rect.expand(8.0 * scale).intersects(field.rect),
                                "value covers a plot handle"
                            );
                        }
                    });
                    let after = ui.label("Next setting").rect;
                    assert!(after.top() >= plot.response.rect.bottom());
                    let three_rows =
                        crate::theme::row_height(scale) * 3.0 + ui.spacing().item_spacing.y * 2.0;
                    assert!(
                        (plot.response.rect.height() - three_rows).abs() < 0.1,
                        "compact control must be three rows tall"
                    );
                });
            }
        }
    }
}
