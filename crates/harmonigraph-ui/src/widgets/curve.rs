//! [`GlowCurveBar`]: the node glow's one interior falloff point, edited on the
//! global curve it shapes rather than as two numbers on separate rows.

use egui::{CornerRadius, Response, Sense, TextStyle, Ui, Vec2};
use harmonigraph_scene::GlowCurve;

use super::bar::{aimed_at, bar_radius, bar_width, grabbed, release_grab, BAR_TEXT_PAD};
use crate::theme;

/// Height of the whole editor at the design UI scale.
const CURVE_HEIGHT: f32 = 62.0;
/// Space above the plot for its name and point readout.
const HEADER_HEIGHT: f32 = 19.0;
/// Clear space around the curve and its handles.
const PLOT_INSET: f32 = 7.0;
/// Radius of the editable point.
const HANDLE_RADIUS: f32 = 4.0;
/// The curve is finer than one vertex per four screen points in an ordinary
/// settings column, so its global bend reads as a curve rather than a polyline.
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

/// How far from the point's centre a drag landed, in curve coordinates.
#[derive(Clone, Copy, Debug, Default)]
struct Grab {
    distance_offset: f32,
    level_offset: f32,
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
        let point = |distance: f32, level: f32| {
            egui::pos2(plot.left() + plot.width() * distance, plot.bottom() - plot.height() * level)
        };
        let coordinates_at = |position: egui::Pos2| {
            [
                (position.x - plot.left()) / plot.width().max(1.0),
                (plot.bottom() - position.y) / plot.height().max(1.0),
            ]
        };

        let grab_id = response.id.with("grab");
        if response.double_clicked() {
            *self.curve = GlowCurve::default();
            response.mark_changed();
        }
        if response.dragged() {
            if let Some(pointer) = response.interact_pointer_pos() {
                let grab = grabbed(ui, grab_id, |ui| {
                    let aim = aimed_at(ui, pointer);
                    let [distance, level] = self.curve.point();
                    let [aim_distance, aim_level] = coordinates_at(aim);
                    Grab {
                        distance_offset: aim_distance - distance,
                        level_offset: aim_level - level,
                    }
                });
                let before = *self.curve;
                let [distance, level] = coordinates_at(pointer);
                self.curve.set_point(distance - grab.distance_offset, level - grab.level_offset);
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

        let [distance, level] = self.curve.point();
        let handle = point(distance, level);
        let guide = egui::Stroke::new(1.0 * scale, theme::hairline().gamma_multiply(0.45));
        painter.line_segment([egui::pos2(handle.x, plot.top()), handle], guide);
        painter.line_segment([egui::pos2(plot.left(), handle.y), handle], guide);
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

        let fill =
            if response.hovered() || response.dragged() { theme::text() } else { theme::accent() };
        painter.circle_filled(handle, HANDLE_RADIUS * scale, fill);
        painter.circle_stroke(
            handle,
            HANDLE_RADIUS * scale,
            egui::Stroke::new(1.0 * scale, theme::panel()),
        );

        let text_color = if response.hovered() || response.dragged() {
            theme::text()
        } else {
            theme::text_dim()
        };
        let body = TextStyle::Body.resolve(ui.style());
        let mono = TextStyle::Monospace.resolve(ui.style());
        let label = painter.layout_no_wrap("Curve".to_owned(), body, text_color);
        let values = painter.layout_no_wrap(
            format!("{:.0} across · {:.0}% left", distance * 100.0, level * 100.0),
            mono,
            theme::text(),
        );
        let pad = BAR_TEXT_PAD * scale;
        let y = rect.top() + (HEADER_HEIGHT * scale - label.size().y) * 0.5;
        painter.galley(egui::pos2(rect.left() + pad, y), label, text_color);
        painter.galley(egui::pos2(rect.right() - pad - values.size().x, y), values, theme::text());

        response.on_hover_cursor(egui::CursorIcon::Crosshair)
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
    fn the_editor_draws_the_curve_its_point_describes() {
        let curve = GlowCurve { distance: 0.7, level: 0.35 };
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

    /// A pointer gesture moves the curve point freely on both axes. Its x
    /// coordinate is a setting in its own right rather than a fixed guide.
    #[test]
    fn a_drag_moves_both_coordinates_of_the_curve_point() {
        const W: f32 = 320.0;
        let ctx = crate::tests::probe::themed();
        let screen = egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(W, 100.0));
        let rect = std::cell::Cell::new(egui::Rect::NOTHING);
        let mut curve = GlowCurve { distance: 0.4, level: 0.3 };
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
        let at = |distance: f32, level: f32| {
            egui::pos2(plot.left() + plot.width() * distance, plot.bottom() - plot.height() * level)
        };
        let from = at(curve.distance, curve.level);
        let to = at(0.78, 0.62);
        frame(&mut curve, vec![egui::Event::PointerMoved(from)]);
        frame(
            &mut curve,
            vec![egui::Event::PointerMoved(from), crate::tests::probe::press(from, true)],
        );
        frame(&mut curve, vec![egui::Event::PointerMoved(from + (to - from).normalized() * 8.0)]);
        frame(&mut curve, vec![egui::Event::PointerMoved(to)]);

        assert!(
            (curve.distance - 0.78).abs() < 1e-5 && (curve.level - 0.62).abs() < 1e-5,
            "the point landed at {:?} rather than [0.78, 0.62]",
            curve.point(),
        );
    }
}
