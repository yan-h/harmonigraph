//! The [`BendPlot`]: where along its range a gradient spends its change, and
//! which of its channels do.

use egui::{CornerRadius, Response, Sense, Stroke, Ui, Vec2};
use harmonigraph_scene::{Bend, Gradient};

use super::bar::{bar_radius, bar_width, HANDLE_INSET};
use crate::theme;

/// Rows the plot stands, so a corner at 90% of the range is a drag of a few
/// points rather than a single one.
const PLOT_ROWS: f32 = 3.0;

/// Points the curve is drawn with along each axis. Sampled evenly across the
/// range AND evenly up it, so a steep stretch near one end gets as many
/// points as a flat one and draws without facets.
const CURVE_SEGMENTS: usize = 64;

/// Height of a [`BendPlot`]'s well at this scale. Shared with the scale
/// census, which finds bars by their full-width well and has to know this one
/// is three rows on purpose.
pub(crate) fn bend_plot_height(scale: f32) -> f32 {
    theme::row_height(scale) * PLOT_ROWS
}

/// The part of the well the curve is drawn in: inset like a bar's track, so
/// both ends of the range are places the handle can stand rather than edges
/// it merges into.
fn plot_area(well: egui::Rect, scale: f32) -> egui::Rect {
    well.shrink(HANDLE_INSET * scale + 2.0 * scale)
}

/// The gradient's [`Bend`]: a row of on/off buttons naming the channels that
/// follow the curve, over a plot of the range (across) against how far those
/// channels have come (up). Drag anywhere on the plot to move the knee's
/// corner; double-click to straighten it.
///
/// The handle is the CORNER, where the knee's two straight lines meet, and the
/// lines are drawn faintly through it: the curve rounds inside the corner
/// rather than through it, which is what a compressor's knee does too.
///
/// A second picture of the range rather than a mark on the bars above, whose
/// tracks run along each channel's own VALUE (the hue circle, the `L*` axis)
/// and not along the range at all.
///
/// No readout: the curve's shape is the setting, and the preview under the
/// group shows what it does to the colors.
pub struct BendPlot<'a> {
    gradient: &'a mut Gradient,
    home: Gradient,
}

impl<'a> BendPlot<'a> {
    pub fn new(gradient: &'a mut Gradient) -> Self {
        BendPlot { gradient, home: Gradient::default() }
    }

    /// The gradient a double-click takes the corner back to.
    pub fn home(mut self, home: Gradient) -> Self {
        self.home = home;
        self
    }

    pub fn show(self, ui: &mut Ui) -> Response {
        let bend = &mut self.gradient.bend;
        // Wrapped, so a narrow column puts the buttons on a second line
        // rather than pushing the pane (and the plot under them) wider.
        ui.horizontal_wrapped(|ui| {
            super::label(ui, "Curve");
            ui.spacing_mut().item_spacing.x = theme::button_gap(theme::ui_scale(ui.ctx()));
            for (on, name) in [
                (&mut bend.hue, "Hue"),
                (&mut bend.lightness, "Brightness"),
                (&mut bend.chroma, "Saturation"),
            ] {
                ui.toggle_value(on, super::option_label(name))
                    .on_hover_text(format!("Whether {} follows the curve", name.to_lowercase()));
            }
        });

        let scale = theme::ui_scale(ui.ctx());
        let size = Vec2::new(bar_width(ui), bend_plot_height(scale));
        let (rect, mut response) = ui.allocate_exact_size(size, Sense::click_and_drag());
        let plot = plot_area(rect, scale);
        let pos_of = |t: f32, w: f32| {
            egui::pos2(plot.left() + t * plot.width(), plot.bottom() - w * plot.height())
        };

        if response.double_clicked() {
            let home = self.home.sanitized().bend;
            self.gradient.bend = Bend { at: home.at, share: home.share, ..self.gradient.bend };
            response.mark_changed();
        } else if response.dragged() || response.clicked() {
            if let Some(p) = response.interact_pointer_pos() {
                let across = (p.x - plot.left()) / plot.width().max(1.0);
                let up = (plot.bottom() - p.y) / plot.height().max(1.0);
                // Snapped to whole percentages, so a stored corner is a round number.
                let snap = |v: f32| (v * 100.0).round() / 100.0;
                let next =
                    Bend { at: snap(across), share: snap(up), ..self.gradient.bend }.sanitized();
                if next != self.gradient.bend {
                    self.gradient.bend = next;
                    response.mark_changed();
                }
            }
        }

        // ---- Paint ---- read back after the write, so the handle is under the
        // pointer on the frame it moved.
        let bend = self.gradient.bend.sanitized();
        let painter = ui.painter();
        painter.rect_filled(rect, CornerRadius::same(bar_radius(scale)), theme::well());
        let hairline = Stroke::new(scale, theme::hairline());
        let dot = pos_of(bend.at, bend.share);
        painter.line_segment([pos_of(0.0, 0.0), pos_of(1.0, 1.0)], hairline);
        painter.line_segment([pos_of(0.0, 0.0), dot], hairline);
        painter.line_segment([dot, pos_of(1.0, 1.0)], hairline);

        // Even steps across the range, and the same up it read back through
        // the curve's inverse, merged.
        let mut ts: Vec<f64> = (0..=CURVE_SEGMENTS)
            .flat_map(|i| {
                let s = i as f64 / CURVE_SEGMENTS as f64;
                [s, bend.inverse().warp(s)]
            })
            .collect();
        ts.sort_by(f64::total_cmp);
        let points = ts.into_iter().map(|t| pos_of(t as f32, bend.warp(t) as f32)).collect();
        let applies = bend.hue || bend.lightness || bend.chroma;
        let live = response.hovered() || response.dragged();
        let ink = if applies { theme::text() } else { theme::text_dim() };
        painter.add(egui::Shape::line(points, Stroke::new(1.5 * scale, ink)));
        let fill = if live { theme::text() } else { theme::text_dim() };
        painter.circle(dot, 4.0 * scale, fill, Stroke::new(scale, theme::panel()));

        response.on_hover_cursor(egui::CursorIcon::Crosshair)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn press(pos: egui::Pos2, pressed: bool) -> egui::Event {
        egui::Event::PointerButton {
            pos,
            button: egui::PointerButton::Primary,
            pressed,
            modifiers: egui::Modifiers::default(),
        }
    }

    /// Click a point on the plot, as fractions of its width and height from
    /// the bottom-left, `clicks` times through a real context.
    fn click(g: &mut Gradient, (x, y): (f32, f32), clicks: usize) {
        let ctx = crate::tests::probe::themed();
        let screen = egui::Rect::from_min_size(egui::pos2(0.0, 0.0), egui::vec2(300.0, 200.0));
        let plot = std::cell::Cell::new(egui::Rect::NOTHING);
        let mut t = 0.0;
        let mut frame = |g: &mut Gradient, events: Vec<egui::Event>| {
            t += 1.0 / 60.0;
            let _ = ctx.run_ui(
                egui::RawInput {
                    screen_rect: Some(screen),
                    time: Some(t),
                    events,
                    ..Default::default()
                },
                |ui| {
                    plot.set(BendPlot::new(g).show(ui).rect);
                },
            );
        };
        frame(g, vec![]);
        let rect = plot_area(plot.get(), theme::ui_scale(&ctx));
        let at = egui::pos2(rect.left() + rect.width() * x, rect.bottom() - rect.height() * y);
        frame(g, vec![egui::Event::PointerMoved(at)]);
        for _ in 0..clicks {
            frame(g, vec![press(at, true)]);
            frame(g, vec![press(at, false)]);
        }
    }

    #[test]
    fn a_click_moves_the_corner_and_a_double_click_straightens_it() {
        let mut g = Gradient::default();
        g.bend.lightness = false;
        click(&mut g, (0.9, 0.15), 1);
        assert_eq!(g.bend, Bend { at: 0.9, share: 0.15, lightness: false, ..Bend::default() });

        click(&mut g, (0.9, 0.15), 2);
        assert!(g.bend.is_straight(), "a double-click left the curve bent: {:?}", g.bend);
        assert!(!g.bend.lightness, "a double-click switched a channel back on");
    }
}
