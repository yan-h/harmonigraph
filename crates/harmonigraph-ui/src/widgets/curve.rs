//! [`GlowCurveBar`]: the node glow's three interior falloff levels, edited on
//! the curve they make rather than as three numbers on separate rows.

use egui::{CornerRadius, Response, Sense, TextStyle, Ui, Vec2};
use harmonigraph_scene::GlowCurve;

use super::bar::{aimed_at, bar_radius, bar_width, grabbed, release_grab, BAR_TEXT_PAD};
use crate::theme;

/// Height of the whole editor at the design UI scale.
const CURVE_HEIGHT: f32 = 62.0;
/// Space above the plot for its name and three-value readout.
const HEADER_HEIGHT: f32 = 19.0;
/// Clear space around the curve and its handles.
const PLOT_INSET: f32 = 7.0;
/// Radius of one of the three editable points.
const HANDLE_RADIUS: f32 = 4.0;
/// The curve is finer than one vertex per four screen points in an ordinary
/// settings column, so its cubic reads as a bend rather than a polyline.
const CURVE_SEGMENTS: usize = 64;

/// The editor's declared height at one UI scale, for the pane sweep that holds
/// every settings control to the space it asks for.
#[cfg(test)]
pub(crate) fn glow_curve_height(scale: f32) -> f32 {
    CURVE_HEIGHT * scale
}

/// The curve's stroke colour. Separate from `ValueBar`'s preview colour so a
/// settings test can tell this editor's path from the Fade curve above it.
fn curve_color() -> egui::Color32 {
    theme::accent().gamma_multiply(0.72)
}

/// Which handle a drag has hold of, and how far from its centre the press
/// landed in curve-level units.
#[derive(Clone, Copy, Debug, Default)]
struct Grab {
    index: usize,
    offset: f32,
}

pub struct GlowCurveBar<'a> {
    curve: &'a mut GlowCurve,
}

impl<'a> GlowCurveBar<'a> {
    pub fn new(curve: &'a mut GlowCurve) -> Self {
        GlowCurveBar { curve }
    }

    pub fn show(self, ui: &mut Ui) -> Response {
        let scale = theme::ui_scale(ui.ctx());
        let width = bar_width(ui);
        let (rect, mut response) =
            ui.allocate_exact_size(Vec2::new(width, CURVE_HEIGHT * scale), Sense::click_and_drag());
        let plot = egui::Rect::from_min_max(
            egui::pos2(rect.left() + PLOT_INSET * scale, rect.top() + HEADER_HEIGHT * scale),
            egui::pos2(rect.right() - PLOT_INSET * scale, rect.bottom() - PLOT_INSET * scale),
        );
        let point = |index: usize, level: f32| {
            egui::pos2(
                plot.left() + plot.width() * (index as f32 + 1.0) / 4.0,
                plot.bottom() - plot.height() * level,
            )
        };
        let level_at = |y: f32| ((plot.bottom() - y) / plot.height().max(1.0)).clamp(0.0, 1.0);

        let grab_id = response.id.with("grab");
        if response.double_clicked() {
            *self.curve = GlowCurve::default();
            response.mark_changed();
        }
        if response.dragged() {
            if let Some(pointer) = response.interact_pointer_pos() {
                let grab = grabbed(ui, grab_id, |ui| {
                    let aim = aimed_at(ui, pointer);
                    let controls = self.curve.controls();
                    let index = (0..3)
                        .min_by(|&a, &b| {
                            point(a, controls[a])
                                .distance_sq(aim)
                                .total_cmp(&point(b, controls[b]).distance_sq(aim))
                        })
                        .unwrap_or(0);
                    Grab { index, offset: level_at(aim.y) - controls[index] }
                });
                let before = *self.curve;
                self.curve.set_control(grab.index, level_at(pointer.y) - grab.offset);
                if *self.curve != before {
                    response.mark_changed();
                }
            }
        }
        if response.drag_stopped() {
            release_grab::<Grab>(ui, grab_id);
        }

        let painter = ui.painter();
        painter.rect_filled(rect, CornerRadius::same(bar_radius(scale)), theme::well());

        // The guides name the fixed x positions of the three handles. Faint
        // enough to remain construction lines rather than extra curve data.
        for index in 0..3 {
            let x = point(index, 0.0).x;
            painter.line_segment(
                [egui::pos2(x, plot.top()), egui::pos2(x, plot.bottom())],
                egui::Stroke::new(1.0 * scale, theme::hairline().gamma_multiply(0.45)),
            );
        }
        painter.line_segment(
            [plot.left_bottom(), plot.right_bottom()],
            egui::Stroke::new(1.0 * scale, theme::hairline()),
        );

        let path = (0..=CURVE_SEGMENTS)
            .map(|i| {
                let p = i as f32 / CURVE_SEGMENTS as f32;
                egui::pos2(
                    plot.left() + plot.width() * p,
                    plot.bottom() - plot.height() * self.curve.sample(p),
                )
            })
            .collect();
        painter.add(egui::Shape::line(path, egui::Stroke::new(1.5 * scale, curve_color())));

        let aimed = response.hover_pos().map(|hover| {
            let controls = self.curve.controls();
            (0..3)
                .min_by(|&a, &b| {
                    point(a, controls[a])
                        .distance_sq(hover)
                        .total_cmp(&point(b, controls[b]).distance_sq(hover))
                })
                .unwrap_or(0)
        });
        for (index, level) in self.curve.controls().into_iter().enumerate() {
            let fill = if aimed == Some(index) || response.dragged() {
                theme::text()
            } else {
                theme::accent()
            };
            painter.circle_filled(point(index, level), HANDLE_RADIUS * scale, fill);
            painter.circle_stroke(
                point(index, level),
                HANDLE_RADIUS * scale,
                egui::Stroke::new(1.0 * scale, theme::panel()),
            );
        }

        let text_color = if response.hovered() || response.dragged() {
            theme::text()
        } else {
            theme::text_dim()
        };
        let body = TextStyle::Body.resolve(ui.style());
        let mono = TextStyle::Monospace.resolve(ui.style());
        let label = painter.layout_no_wrap("Curve".to_owned(), body, text_color);
        let [quarter, half, three_quarters] = self.curve.controls();
        let values = painter.layout_no_wrap(
            format!(
                "{:.0} · {:.0} · {:.0}%",
                quarter * 100.0,
                half * 100.0,
                three_quarters * 100.0,
            ),
            mono,
            theme::text(),
        );
        let pad = BAR_TEXT_PAD * scale;
        let y = rect.top() + (HEADER_HEIGHT * scale - label.size().y) * 0.5;
        painter.galley(egui::pos2(rect.left() + pad, y), label, text_color);
        painter.galley(egui::pos2(rect.right() - pad - values.size().x, y), values, theme::text());

        response.on_hover_cursor(egui::CursorIcon::ResizeVertical)
    }
}

/// Every glow-curve path in a paint list, for the settings test that holds the
/// editor to the curve the renderer receives.
#[cfg(test)]
pub(crate) fn glow_curve_paths(shapes: &[egui::Shape]) -> Vec<Vec<egui::Pos2>> {
    shapes
        .iter()
        .filter_map(|shape| match shape {
            egui::Shape::Path(path)
                if path.stroke.color == egui::epaint::ColorMode::Solid(curve_color()) =>
            {
                Some(path.points.clone())
            }
            _ => None,
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::widgets::probe;

    #[test]
    fn the_editor_draws_the_curve_its_handles_describe() {
        let curve = GlowCurve { quarter: 0.7, half: 0.35, three_quarters: 0.2 };
        let shapes = probe::shapes(320.0, |ui| {
            GlowCurveBar::new(&mut curve.clone()).show(ui);
        });
        let paths = glow_curve_paths(&shapes);
        assert_eq!(paths.len(), 1, "the curve editor drew {} curve paths", paths.len());
        let points = &paths[0];
        let (left, right) = (points[0].x, points[points.len() - 1].x);
        let (top, bottom) = (points[0].y, points[points.len() - 1].y);
        assert!(left < right && top < bottom, "the falloff does not descend across the editor");
        for point in points {
            let p = (point.x - left) / (right - left);
            let want = top + (bottom - top) * (1.0 - curve.sample(p));
            assert!(
                (point.y - want).abs() < 0.02,
                "at {p} across the reach the editor draws {} rather than {want}",
                point.y,
            );
        }
    }

    /// A pointer gesture moves the handle it starts on and leaves its two
    /// neighbours alone. The fixed x positions are labels for distance, not
    /// handles that reorder when one level passes another.
    #[test]
    fn a_drag_moves_only_the_curve_handle_it_was_aimed_at() {
        const W: f32 = 320.0;
        let ctx = crate::tests::probe::themed();
        let screen = egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(W, 100.0));
        let rect = std::cell::Cell::new(egui::Rect::NOTHING);
        let mut curve = GlowCurve { quarter: 0.8, half: 0.5, three_quarters: 0.2 };
        let mut time = 0.0;
        let mut frame = |curve: &mut GlowCurve, events| {
            time += 1.0 / 60.0;
            let _ = ctx.run_ui(
                egui::RawInput {
                    screen_rect: Some(screen),
                    time: Some(time),
                    events,
                    ..Default::default()
                },
                |ui| rect.set(GlowCurveBar::new(curve).show(ui).rect),
            );
        };
        frame(&mut curve, vec![]);
        let editor = rect.get();
        let plot = egui::Rect::from_min_max(
            egui::pos2(editor.left() + PLOT_INSET, editor.top() + HEADER_HEIGHT),
            egui::pos2(editor.right() - PLOT_INSET, editor.bottom() - PLOT_INSET),
        );
        let at = |level: f32| {
            egui::pos2(plot.left() + plot.width() * 0.75, plot.bottom() - plot.height() * level)
        };
        let from = at(curve.three_quarters);
        let to = at(0.4);
        frame(&mut curve, vec![egui::Event::PointerMoved(from)]);
        frame(
            &mut curve,
            vec![egui::Event::PointerMoved(from), crate::tests::probe::press(from, true)],
        );
        frame(&mut curve, vec![egui::Event::PointerMoved(from + (to - from).normalized() * 8.0)]);
        frame(&mut curve, vec![egui::Event::PointerMoved(to)]);

        assert_eq!(curve.quarter, 0.8, "the near handle moved with the far one");
        assert_eq!(curve.half, 0.5, "the middle handle moved with the far one");
        assert!(
            (curve.three_quarters - 0.4).abs() < 1e-5,
            "the far handle landed at {} rather than 0.4",
            curve.three_quarters,
        );
    }
}
