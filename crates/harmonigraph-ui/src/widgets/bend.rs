//! The [`BendPlot`]: where along its range each channel of a gradient spends
//! its change.

use egui::{Color32, CornerRadius, Response, Sense, Stroke, TextStyle, Ui, Vec2};
use harmonigraph_scene::{Bend, Gradient};

use super::bar::{bar_radius, bar_width, BAR_TEXT_PAD, HANDLE_INSET};
use crate::theme;

/// Rows the plot stands, so a bend at 90% of the range is a drag of a few
/// points rather than a single one.
const PLOT_ROWS: f32 = 3.0;

/// Points each curve is drawn with. The curve is smooth and a plot is a few
/// hundred points wide, so this is well past where a polyline shows facets.
const CURVE_SEGMENTS: usize = 96;

/// Height of a [`BendPlot`]'s well at this scale. Shared with the scale
/// census, which finds bars by their full-width well and has to know this one
/// is three rows on purpose.
pub(crate) fn bend_plot_height(scale: f32) -> f32 {
    theme::row_height(scale) * PLOT_ROWS
}

/// The part of the well the curves are drawn in: inset like a bar's track, so
/// both ends of the range are places the handle can stand rather than edges
/// it merges into.
fn plot_area(well: egui::Rect, scale: f32) -> egui::Rect {
    well.shrink(HANDLE_INSET * scale + 2.0 * scale)
}

/// Which of a gradient's three channels the plot has in hand.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Channel {
    #[default]
    Hue,
    Brightness,
    Saturation,
}

impl Channel {
    const ALL: [Channel; 3] = [Channel::Hue, Channel::Brightness, Channel::Saturation];

    fn label(self) -> &'static str {
        match self {
            Channel::Hue => "Hue",
            Channel::Brightness => "Brightness",
            Channel::Saturation => "Saturation",
        }
    }

    fn of(self, g: &Gradient) -> Bend {
        match self {
            Channel::Hue => g.hue_bend,
            Channel::Brightness => g.lightness_bend,
            Channel::Saturation => g.chroma_bend,
        }
    }

    fn of_mut(self, g: &mut Gradient) -> &mut Bend {
        match self {
            Channel::Hue => &mut g.hue_bend,
            Channel::Brightness => &mut g.lightness_bend,
            Channel::Saturation => &mut g.chroma_bend,
        }
    }
}

/// The bend of one channel at a time, picked on a row of buttons above a plot
/// of the range (across) against how far that channel has come (up). Drag
/// anywhere on the plot to put the bend there; double-click to straighten it.
///
/// **One channel at a time**, because the three start on the same point: every
/// gradient opens straight, so three handles would stand on one another at the
/// centre with nothing to say which a press takes. The other two curves are
/// still drawn, dimmed, since how the channels lean against each other is most
/// of what a bend is for.
///
/// A second picture of the range rather than a mark on the bars above, whose
/// tracks run along the channel's own VALUE (the hue circle, the `L*` axis)
/// and not along the range at all.
///
/// The readout says where the bend stands in the range's own units — a note
/// for the pitch gradient, a level for the analyzer's — which is what `axis`
/// is handed for.
pub struct BendPlot<'a> {
    id_salt: &'a str,
    gradient: &'a mut Gradient,
    home: Gradient,
    axis: &'a dyn Fn(f32) -> String,
}

impl<'a> BendPlot<'a> {
    /// `axis` names a position along the range, 0 at the bottom and 1 at the
    /// top. `id_salt` keeps apart which channel two plots on one page hold.
    pub fn new(
        id_salt: &'a str,
        gradient: &'a mut Gradient,
        axis: &'a dyn Fn(f32) -> String,
    ) -> Self {
        BendPlot { id_salt, gradient, home: Gradient::default(), axis }
    }

    /// The gradient a double-click takes the channel in hand back to.
    pub fn home(mut self, home: Gradient) -> Self {
        self.home = home;
        self
    }

    pub fn show(self, ui: &mut Ui) -> Response {
        let channel_id = ui.make_persistent_id(("bend_channel", self.id_salt));
        let mut channel = ui.data(|d| d.get_temp::<Channel>(channel_id)).unwrap_or_default();
        let options: Vec<_> = Channel::ALL.iter().map(|&c| (c, c.label(), "")).collect();
        super::choice_row(ui, "Curve", &mut channel, &options);
        ui.data_mut(|d| d.insert_temp(channel_id, channel));

        let scale = theme::ui_scale(ui.ctx());
        let size = Vec2::new(bar_width(ui), bend_plot_height(scale));
        let (rect, mut response) = ui.allocate_exact_size(size, Sense::click_and_drag());
        let plot = plot_area(rect, scale);
        let pos_of = |t: f32, w: f32| {
            egui::pos2(plot.left() + t * plot.width(), plot.bottom() - w * plot.height())
        };

        if response.double_clicked() {
            *channel.of_mut(self.gradient) = channel.of(&self.home.sanitized());
            response.mark_changed();
        } else if response.dragged() || response.clicked() {
            if let Some(p) = response.interact_pointer_pos() {
                // Whole percentages, so a bend reads out as the numbers it holds.
                let snap = |v: f32| (v * 100.0).round() / 100.0;
                let at = snap((p.x - plot.left()) / plot.width().max(1.0));
                let share = snap((plot.bottom() - p.y) / plot.height().max(1.0));
                let next = Bend { at, share }.sanitized();
                if next != channel.of(self.gradient) {
                    *channel.of_mut(self.gradient) = next;
                    response.mark_changed();
                }
            }
        }

        // ---- Paint ---- read back after the write, so the handle is under the
        // pointer on the frame it moved.
        let g = self.gradient.sanitized();
        let painter = ui.painter();
        painter.rect_filled(rect, CornerRadius::same(bar_radius(scale)), theme::well());
        let hairline = Stroke::new(scale, theme::hairline());
        painter.line_segment([pos_of(0.0, 0.0), pos_of(1.0, 1.0)], hairline);

        let live = response.hovered() || response.dragged();
        let curve = |bend: Bend, color: Color32, width: f32| {
            let points = (0..=CURVE_SEGMENTS)
                .map(|i| {
                    let t = i as f32 / CURVE_SEGMENTS as f32;
                    pos_of(t, bend.warp(f64::from(t)) as f32)
                })
                .collect();
            painter.add(egui::Shape::line(points, Stroke::new(width * scale, color)));
        };
        for other in Channel::ALL.into_iter().filter(|&c| c != channel) {
            curve(other.of(&g), theme::text_dim().gamma_multiply(0.45), 1.0);
        }
        let bend = channel.of(&g);
        curve(bend, theme::text(), 1.5);
        let dot = pos_of(bend.at, bend.share);
        let fill = if live { theme::text() } else { theme::text_dim() };
        painter.circle(dot, 4.0 * scale, fill, Stroke::new(scale, theme::panel()));

        let readout = format!("{:.0}% by {}", bend.share * 100.0, (self.axis)(bend.at));
        let text_color = if live { theme::text() } else { theme::text_dim() };
        let galley =
            painter.layout_no_wrap(readout, TextStyle::Monospace.resolve(ui.style()), text_color);
        // In the top-left corner, which a curve only reaches when it jumps at
        // the very bottom of the range — and the bottom-right is where the
        // curve every straight channel draws already runs.
        let pad = BAR_TEXT_PAD * scale * 0.5;
        painter.galley(rect.left_top() + Vec2::splat(pad), galley, text_color);

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
                    let axis = |t: f32| format!("{:.0}%", t * 100.0);
                    plot.set(BendPlot::new("test", g, &axis).show(ui).rect);
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
    fn a_click_bends_only_the_channel_in_hand_and_a_double_click_straightens_it() {
        let mut g = Gradient::default();
        click(&mut g, (0.9, 0.15), 1);
        let bend = g.hue_bend;
        assert_eq!(bend, Bend { at: 0.9, share: 0.15 });
        assert_eq!((g.lightness_bend, g.chroma_bend), (Bend::STRAIGHT, Bend::STRAIGHT));

        click(&mut g, (0.9, 0.15), 2);
        assert_eq!(g.hue_bend, Bend::STRAIGHT, "a double-click left the hue bent");
    }
}
