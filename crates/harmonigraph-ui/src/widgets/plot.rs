//! Shared geometry and numeric entry for the settings' direct-manipulation controls.
use crate::theme;
use egui::{Color32, Pos2, Rect, Response, Sense, Stroke, Ui, Vec2};
use std::ops::RangeInclusive;

pub(super) struct Plot {
    pub rect: Rect,
    pub response: Response,
    fields: Rect,
}
impl Plot {
    /// Keep the picture a stable size beside exact values. Very narrow panes
    /// stack instead of squeezing labels, numbers, and handles into one row.
    pub fn with_fields(ui: &mut Ui, label: &str, count: usize) -> Self {
        super::label(ui, label);
        let scale = theme::ui_scale(ui.ctx());
        let width = super::bar::bar_width(ui);
        let gap = ui.spacing().item_spacing.y;
        let fields_height = (count as f32 * (theme::row_height(scale) + gap) - gap).max(0.0);
        let beside = count > 0 && width >= 220.0 * scale;
        let total_height =
            height(scale) + if count > 0 && !beside { gap + fields_height } else { 0.0 };
        let (bounds, response) =
            ui.allocate_exact_size(Vec2::new(width, total_height), Sense::hover());
        let well = Rect::from_min_size(
            bounds.min,
            Vec2::new(if beside { 112.0 * scale } else { width }, height(scale)),
        );
        ui.painter().rect_filled(well, super::bar::bar_radius(scale), theme::well());
        let fields = if beside {
            Rect::from_min_max(
                egui::pos2(well.right() + 8.0 * scale, bounds.center().y - fields_height / 2.0),
                egui::pos2(bounds.right(), bounds.center().y + fields_height / 2.0),
            )
        } else {
            Rect::from_min_max(egui::pos2(bounds.left(), well.bottom() + gap), bounds.max)
        };
        Self { rect: well.shrink(10.0 * scale), response, fields }
    }
    pub fn fields<R>(&self, ui: &mut Ui, draw: impl FnOnce(&mut Ui) -> R) -> R {
        // The outer control already allocated the whole rect. A child prevents
        // the value column from advancing its parent's cursor a second time.
        let mut child = ui.new_child(
            egui::UiBuilder::new()
                .id_salt(self.response.id.with("values"))
                .max_rect(self.fields)
                .layout(egui::Layout::top_down(egui::Align::Min)),
        );
        child.set_clip_rect(self.fields.intersect(ui.clip_rect()));
        draw(&mut child)
    }
    pub fn square(&mut self) {
        self.rect = Rect::from_center_size(
            self.rect.center(),
            Vec2::splat(self.rect.width().min(self.rect.height())),
        );
    }
    pub fn point(&self, x: f32, y: f32) -> Pos2 {
        egui::pos2(
            self.rect.left() + x * self.rect.width(),
            self.rect.bottom() - y * self.rect.height(),
        )
    }
    pub fn line(&self, ui: &Ui, points: Vec<Pos2>, color: Color32) {
        ui.painter()
            .with_clip_rect(self.rect.expand(2.0).intersect(ui.clip_rect()))
            .add(egui::Shape::line(points, Stroke::new(1.5 * theme::ui_scale(ui.ctx()), color)));
    }
    /// Separate handle IDs hold their identity even when values or curves cross.
    /// Arrow keys offer one-axis editing; Shift makes the step finer.
    pub fn handle(&self, ui: &mut Ui, key: &str, x: f32, y: f32) -> (Response, Option<Vec2>) {
        let at = self.point(x, y);
        let scale = theme::ui_scale(ui.ctx());
        let mut response = ui.interact(
            Rect::from_center_size(at, Vec2::splat(16.0 * scale)),
            self.response.id.with(key),
            Sense::click_and_drag(),
        );
        if response.clicked() || response.drag_started() {
            response.request_focus();
        }
        let mut next = None;
        if response.dragged() {
            if let Some(p) = response.interact_pointer_pos() {
                next = Some(egui::vec2(
                    ((p.x - self.rect.left()) / self.rect.width().max(1.0)).clamp(0.0, 1.0),
                    ((self.rect.bottom() - p.y) / self.rect.height().max(1.0)).clamp(0.0, 1.0),
                ));
            }
        }
        if response.has_focus() {
            let delta = ui.input(|i| {
                let step = if i.modifiers.shift { 0.001 } else { 0.01 };
                egui::vec2(
                    (i.key_pressed(egui::Key::ArrowRight) as i32
                        - i.key_pressed(egui::Key::ArrowLeft) as i32) as f32,
                    (i.key_pressed(egui::Key::ArrowUp) as i32
                        - i.key_pressed(egui::Key::ArrowDown) as i32) as f32,
                ) * step
            });
            if delta != Vec2::ZERO {
                next = Some((egui::vec2(x, y) + delta).clamp(Vec2::ZERO, Vec2::splat(1.0)));
            }
        }
        if next.is_some() {
            response.mark_changed();
        }
        (response.on_hover_text(key), next)
    }
    /// Paint after every input has been applied, including numeric entry.
    pub fn dot(&self, ui: &Ui, x: f32, y: f32) {
        let scale = theme::ui_scale(ui.ctx());
        let painter =
            ui.painter().with_clip_rect(self.rect.expand(5.0 * scale).intersect(ui.clip_rect()));
        painter.circle_filled(self.point(x, y), 4.0 * scale, theme::accent());
        painter.circle_stroke(self.point(x, y), 4.0 * scale, Stroke::new(1.0, theme::text()));
    }
}

/// A familiar scalar slider beside the combined gesture in the picture.
pub(super) fn value_bar(
    ui: &mut Ui,
    value: &mut f32,
    range: RangeInclusive<f32>,
    labels: [&str; 2],
    scale: f32,
    suffix: &str,
) -> Response {
    let decimals = match suffix {
        " ms" => 0,
        "%" | " pp" | "°" | "¢" => 1,
        _ => 2,
    };
    super::ValueBar::new(value, range, labels[0])
        .caption(labels[1])
        .unit(scale, suffix)
        .decimals(decimals)
        .show(ui)
        .on_hover_text(format!("{} · drag to adjust, double-click to type", labels[0]))
}

pub(super) fn curve(plot: &Plot, ui: &Ui, sample: impl Fn(f32) -> (f32, f32), color: Color32) {
    plot.line(
        ui,
        (0..=64)
            .map(|i| {
                let (x, y) = sample(i as f32 / 64.0);
                plot.point(x, y)
            })
            .collect(),
        color,
    );
}

pub(crate) fn height(scale: f32) -> f32 {
    theme::row_height(scale) * 3.5
}
