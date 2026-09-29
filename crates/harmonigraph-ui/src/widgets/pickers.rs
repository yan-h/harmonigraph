//! Compact color, integer-band and spatial edge selectors.
use super::plot::{number, Plot};
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
        egui::pos2(rect.left() + 8.0 * scale, rect.center().y - galley.size().y / 2.0),
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
        ui.set_width(220.0 * theme::ui_scale(ui.ctx()));
        let key = crate::panes::pane_content_right();
        let previous = ui.data(|d| d.get_temp::<f32>(key));
        ui.data_mut(|d| d.insert_temp(key, ui.max_rect().right()));
        ui.push_id(label, |ui| {
            let plot = Plot::new(ui, "Hue → / amount ↑");
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
            number(ui, hue, 0.0..=360.0, labels[0], 1.0, "°");
            number(ui, amount, 0.0..=1.0, labels[1], 100.0, "%");
            plot.dot(ui, *hue / 360.0, *amount);
        });
        ui.data_mut(|d| {
            if let Some(right) = previous {
                d.insert_temp(key, right);
            } else {
                d.remove::<f32>(key);
            }
        });
    });
}

pub(crate) fn contours(ui: &mut Ui, levels: &mut f32) {
    use harmonigraph_scene::{CONTOURS_MAX, CONTOURS_MIN};
    #[cfg(test)]
    super::range_probe::record("Contour levels", &[*levels], &(CONTOURS_MIN..=CONTOURS_MAX));
    super::label(ui, "Contour levels");
    let mut n = *levels as i32;
    super::button_row(ui, |ui| {
        if ui.add_enabled(n > CONTOURS_MIN as i32, egui::Button::new("−")).clicked() {
            n -= 1;
        }
        ui.add(
            egui::DragValue::new(&mut n)
                .range(CONTOURS_MIN as i32..=CONTOURS_MAX as i32)
                .speed(0.1),
        );
        if ui.add_enabled(n < CONTOURS_MAX as i32, egui::Button::new("+")).clicked() {
            n += 1;
        }
    });
    if n as f32 != *levels {
        *levels = n as f32;
    }
    let (rect, _) = ui.allocate_exact_size(
        egui::vec2(super::bar::bar_width(ui), theme::row_height(theme::ui_scale(ui.ctx()))),
        egui::Sense::hover(),
    );
    let mut mesh = egui::Mesh::default();
    for i in 0..n {
        let left = rect.left() + rect.width() * i as f32 / n as f32;
        let right = rect.left() + rect.width() * (i + 1) as f32 / n as f32;
        mesh.add_colored_rect(
            egui::Rect::from_min_max(
                egui::pos2(left, rect.top()),
                egui::pos2(right, rect.bottom()),
            ),
            theme::well().lerp_to_gamma(theme::text(), i as f32 / (n - 1) as f32),
        );
    }
    ui.painter().add(egui::Shape::mesh(mesh));
}

pub(crate) fn spectrum_edge(ui: &mut Ui, edge: &mut crate::SpectralOrientation) {
    use crate::SpectralOrientation::*;
    super::label(ui, "Spectrum edge");
    let plot = Plot::new(ui, "Click an edge · spectrum at the highlighted side");
    for value in crate::SpectralOrientation::ALL {
        let (label, a, b) = match value {
            Left => ("Left", (0.0, 0.15), (0.18, 0.85)),
            Right => ("Right", (0.82, 0.15), (1.0, 0.85)),
            Top => ("Top", (0.18, 0.7), (0.82, 1.0)),
            Bottom => ("Bottom", (0.18, 0.0), (0.82, 0.3)),
        };
        let rect = egui::Rect::from_two_pos(plot.point(a.0, a.1), plot.point(b.0, b.1));
        if ui.put(rect, egui::Button::new(label).selected(*edge == value)).clicked() {
            *edge = value;
        }
    }
}
