//! Compact color, integer-band and spatial edge selectors.
use super::plot::{value_bar, Plot};
use crate::theme;
use egui::{Color32, Ui};

pub(crate) fn skin_color(
    ui: &mut Ui,
    label: &str,
    hue: &mut f32,
    amount: &mut f32,
    labels: [&str; 2],
    color: impl Fn(f32, f32) -> Color32,
) {
    // Closed popovers still participate in the loaded-value census.
    #[cfg(test)]
    {
        super::range_probe::record(labels[0], &[*hue], &(0.0..=360.0));
        super::range_probe::record(labels[1], &[*amount], &(0.0..=1.0));
    }
    let scale = theme::ui_scale(ui.ctx());
    let (rect, response) = ui.allocate_exact_size(
        egui::vec2(super::bar::bar_width(ui), theme::row_height(scale)),
        egui::Sense::click(),
    );
    let visuals = ui.style().interact(&response);
    ui.painter().rect_filled(rect, super::bar::bar_radius(scale), visuals.bg_fill);
    let job = egui::text::LayoutJob::simple(
        label.to_owned(),
        egui::TextStyle::Button.resolve(ui.style()),
        theme::text(),
        f32::INFINITY,
    );
    let galley = super::bar::elided_name(ui.painter(), job, rect.width(), scale, 20.0 * scale);
    ui.painter().galley(
        super::text_at(ui, rect, &galley, rect.left() + super::bar::BAR_TEXT_PAD * scale),
        galley,
        theme::text(),
    );
    let swatch = egui::Rect::from_center_size(
        egui::pos2(rect.right() - 12.0 * scale, rect.center().y),
        egui::vec2(14.0, 12.0) * scale,
    );
    ui.painter().rect_filled(swatch, 2, color(*hue, *amount));
    let response = response.on_hover_text("Choose hue and color amount");
    egui::Popup::menu(&response).style(super::menu_style(ui.ctx())).show(|ui| {
        ui.set_width(280.0 * theme::ui_scale(ui.ctx()));
        ui.push_id(label, |ui| {
            let plot = Plot::with_fields(ui, "Hue → / amount ↑", 2);
            // Use the skin's own OKLab conversion and permitted chroma.
            for x in 0..48 {
                for y in 0..16 {
                    let lo = plot.point(x as f32 / 48.0, (y + 1) as f32 / 16.0);
                    let hi = plot.point((x + 1) as f32 / 48.0, y as f32 / 16.0);
                    ui.painter().rect_filled(
                        egui::Rect::from_min_max(lo, hi),
                        0,
                        color(x as f32 / 48.0 * 360.0, y as f32 / 15.0),
                    );
                }
            }
            let (_, next) = plot.handle(ui, "Hue and amount", *hue / 360.0, *amount);
            if let Some(p) = next {
                *hue = p.x * 360.0;
                *amount = p.y;
            }
            // Test the real entries when the popup is opened, as well as its closed state.
            plot.fields(ui, |ui| {
                value_bar(ui, hue, 0.0..=360.0, [labels[0], "Hue"], 1.0, "°");
                value_bar(ui, amount, 0.0..=1.0, [labels[1], "Amount"], 100.0, "%");
            });
            plot.dot(ui, *hue / 360.0, *amount);
        });
    });
}

pub(crate) fn spectrum_edge(ui: &mut Ui, edge: &mut crate::SpectralOrientation) {
    use crate::SpectralOrientation::*;
    super::label(ui, "Spectrum edge");
    let scale = theme::ui_scale(ui.ctx());
    let row = theme::row_height(scale);
    let gap = theme::button_gap(scale);
    let width = super::bar::bar_width(ui).min(176.0 * scale);
    let (bounds, _) =
        ui.allocate_exact_size(egui::vec2(width, 2.0 * row + gap), egui::Sense::hover());
    let side = (width - 2.0 * gap).max(0.0) * 0.27;
    let middle_left = bounds.left() + side + gap;
    let middle_right = bounds.right() - side - gap;
    for value in crate::SpectralOrientation::ALL {
        let (label, min, max) = match value {
            Left => ("Left", bounds.min, egui::pos2(bounds.left() + side, bounds.bottom())),
            Right => ("Right", egui::pos2(bounds.right() - side, bounds.top()), bounds.max),
            Top => (
                "Top",
                egui::pos2(middle_left, bounds.top()),
                egui::pos2(middle_right, bounds.top() + row),
            ),
            Bottom => (
                "Bottom",
                egui::pos2(middle_left, bounds.bottom() - row),
                egui::pos2(middle_right, bounds.bottom()),
            ),
        };
        let rect = egui::Rect::from_min_max(min, max);
        if ui
            .put(rect, egui::Button::new(label).truncate().selected(*edge == value))
            .on_hover_text(label)
            .clicked()
        {
            *edge = value;
        }
    }
}
