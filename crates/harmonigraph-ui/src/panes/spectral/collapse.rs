//! Editor-only folds inside the Analyzer. Hidden regions keep their depth in
//! points; a virtual full picture keeps the surviving region's calibration.

use egui::{Pos2, Rect, Sense, Vec2};

use super::{axes::Axes, gestures, Navigation, RegionView};
use crate::{panes::DOCKED_SURFACE, theme, PictureState};

#[derive(Clone, Default, serde::Serialize, serde::Deserialize)]
#[serde(default)]
pub(crate) struct Regions {
    pub collapsed: [bool; 2],
    depths: [f32; 2],
    /// The appearance split these measured lengths belong to.
    dial: f32,
    vertical: bool,
    /// Restore ceiling for a project saved with only an internal fold.
    pub window: f32,
    #[serde(skip)]
    pub request: Option<Request>,
    #[serde(skip)]
    landing: Option<[bool; 2]>,
    #[serde(skip)]
    restore: bool,
}

#[derive(Clone, Copy)]
pub(crate) struct Request {
    pub collapsed: [bool; 2],
    pub width_change: f32,
}

impl Regions {
    pub fn sanitize(&mut self) {
        for depth in &mut self.depths {
            *depth = if depth.is_finite() { depth.clamp(0.0, 16_384.0) } else { 0.0 };
        }
        self.window = if self.window.is_finite() { self.window.clamp(0.0, 32_768.0) } else { 0.0 };
        self.restore = true;
    }

    /// Visibility lands with the resize, never on the click's old geometry.
    pub fn begin_frame(&mut self) {
        if let Some(collapsed) = self.landing.take() {
            self.collapsed = collapsed;
            self.restore = !collapsed.iter().any(|&c| c);
        }
    }

    pub fn land(&mut self) {
        if let Some(request) = self.request.take() {
            self.landing = Some(request.collapsed);
        }
    }

    pub fn reset_width(&self, area: f32) -> f32 {
        if !self.vertical && self.collapsed.iter().any(|&closed| closed) {
            (self.window - area).max(0.0)
        } else {
            0.0
        }
    }

    pub fn draw(&mut self, ui: &mut egui::Ui, state: &mut PictureState, now: f64) {
        let rect = ui.available_rect_before_wrap();
        let cfg = state.appearance.spectrum;
        let axes = Axes::new(rect, &cfg);
        let depth = axes.depth_len();
        if !rect.is_positive() || !depth.is_finite() {
            return;
        }
        let history = cfg.show_roll || cfg.show_spectrogram;
        // Turning off the history layers hides that region and its control;
        // it does not discard the user's independent layout folds.
        let folded = [self.collapsed[0], self.collapsed[1] || !history];
        let rail = if state.appearance.view.frameless {
            0.0
        } else {
            theme::tab_bar_height(theme::ui_scale(ui.ctx())).min(
                depth / (usize::from(folded[0]) + usize::from(folded[1] && history)).max(1) as f32,
            )
        };
        if self.vertical != cfg.orientation.is_time_vertical()
            || self.depths.iter().any(|d| !d.is_finite())
            || self.depths.iter().sum::<f32>() <= 0.0
        {
            self.vertical = cfg.orientation.is_time_vertical();
            self.dial = cfg.roll_fraction;
            let share = super::axes::spectrum_share(&cfg);
            self.depths = [depth * share, depth * (1.0 - share)];
            self.restore = false;
        }
        let rail_depths =
            [if folded[0] { rail } else { 0.0 }, if folded[1] && history { rail } else { 0.0 }];
        let available = (depth - rail_depths.iter().sum::<f32>()).max(0.0);
        let split;
        if !folded[0] && !folded[1] {
            if std::mem::take(&mut self.restore) && self.dial == cfg.roll_fraction {
                gestures::restore_spectrum(state, self.depths);
            }
            super::hold_spectrum(state, rect.size());
            super::spectral_pane(ui, state, now, DOCKED_SURFACE, 1.0, Navigation::Docked);
            split = gestures::spectrum_split(state, DOCKED_SURFACE);
            self.depths = [depth * split, depth * (1.0 - split)];
            self.dial = state.appearance.spectrum.roll_fraction;
        } else {
            ui.allocate_rect(rect, Sense::hover());
            if !folded[0] {
                self.depths[0] = available;
            }
            if !folded[1] {
                self.depths[1] = available;
            }
            // The picture extends behind the collapsed region. Only the open
            // region gets a paint clip, so peaks neither flip nor rescale and
            // note trails keep their alignment with the shared now-line.
            let visible = Rect::from_two_pos(
                axes.at(0.0, rail_depths[0] / depth),
                axes.at(1.0, 1.0 - rail_depths[1] / depth),
            );
            let mut virtual_rect = visible;
            let dir = axes.dir_depth();
            if folded[0] {
                extend(&mut virtual_rect, -dir * self.depths[0]);
            }
            if folded[1] && history {
                extend(&mut virtual_rect, dir * self.depths[1]);
            }
            let total = Axes::new(virtual_rect, &cfg).depth_len();
            split = (self.depths[0] / total.max(1.0)).clamp(0.0, 1.0);
            if !folded[0] || !folded[1] {
                let mut child = ui.new_child(egui::UiBuilder::new().max_rect(virtual_rect));
                child.set_clip_rect(visible.intersect(ui.clip_rect()));
                super::spectral_pane(
                    &mut child,
                    state,
                    now,
                    DOCKED_SURFACE,
                    1.0,
                    Navigation::Folded(RegionView { split, shown: [!folded[0], !folded[1]] }),
                );
            }
        }
        if rail <= 0.0 {
            return;
        }
        let divider = if folded[0] {
            rail / depth
        } else if folded[1] {
            1.0 - rail_depths[1] / depth
        } else {
            split
        };
        for (index, name) in
            ["Spectrum", if cfg.show_spectrogram { "Spectrogram" } else { "Piano roll" }]
                .into_iter()
                .enumerate()
        {
            if index == 1 && !history {
                continue;
            }
            let closed = self.collapsed[index];
            let band = if closed {
                let (a, b) =
                    if index == 0 { (0.0, rail / depth) } else { (1.0 - rail / depth, 1.0) };
                Rect::from_two_pos(axes.at(0.0, a), axes.at(1.0, b)).intersect(rect)
            } else {
                control_rect(&axes, divider, index, theme::row_height(theme::ui_scale(ui.ctx())))
                    .intersect(rect)
            };
            if !band.is_positive() {
                continue;
            }
            let direction = axes.dir_depth() * if (index == 0) == closed { 1.0 } else { -1.0 };
            if control(ui, band, direction, closed, self.vertical, name, index)
                && self.request.is_none()
            {
                let mut collapsed = self.collapsed;
                collapsed[index] = !closed;
                let delta = (self.depths[index] - rail).max(0.0) * if closed { 1.0 } else { -1.0 };
                self.request = Some(Request {
                    collapsed,
                    width_change: if self.vertical { 0.0 } else { delta },
                });
            }
        }
    }
}

fn extend(rect: &mut Rect, delta: Vec2) {
    rect.min += delta.min(Vec2::ZERO);
    rect.max += delta.max(Vec2::ZERO);
}

/// The high-pitch end for vertical time, the top edge otherwise. Each button
/// sits wholly on its own side of the divider, leaving its drag band clear.
/// The button uses a control row's height, not a whole tab bar's: two full-size
/// tabs over the picture obscure the very boundary they are meant to fold.
fn control_rect(axes: &Axes, split: f32, index: usize, size: f32) -> Rect {
    let offset = size * 0.5 + gestures::SPLIT_GRAB_HALF;
    let pitch = 1.0 - (size * 0.5 + 4.0) / axes.pitch_len();
    let d = split + if index == 0 { -offset } else { offset } / axes.depth_len();
    Rect::from_center_size(axes.at(pitch, d), Vec2::splat(size))
}

fn control(
    ui: &egui::Ui,
    rect: Rect,
    direction: Vec2,
    rail: bool,
    vertical: bool,
    name: &str,
    index: usize,
) -> bool {
    let response =
        ui.interact(rect, egui::Id::new(("analyzer region fold", index)), Sense::click());
    response.widget_info(|| {
        egui::WidgetInfo::labeled(
            egui::WidgetType::Button,
            true,
            format!("{} {name}", if rail { "Expand" } else { "Collapse" }),
        )
    });
    let painter = ui.painter_at(rect);
    let scale = theme::ui_scale(ui.ctx());
    let style = theme::dock_style(ui.style(), scale);
    let size = theme::tab_bar_height(scale);
    let center = if rail {
        rect.left_top()
            + if vertical {
                egui::vec2(size * 0.5, rect.height() * 0.5)
            } else {
                egui::vec2(rect.width() * 0.5, size * 0.5)
            }
    } else {
        rect.center()
    };
    // A restore rail is tab chrome; only its arrow cell takes the dock's
    // button fill. Over the picture, the small open controls use the ordinary
    // widget fill so they remain visible against black without a new accent.
    if rail {
        painter.rect_filled(rect, egui::CornerRadius::ZERO, style.tab.active.bg_fill);
    }
    let button = if rail { Rect::from_center_size(center, Vec2::splat(size)) } else { rect };
    let hovered = response.hovered() || response.has_focus();
    painter.rect_filled(
        button,
        if rail {
            egui::CornerRadius::ZERO
        } else {
            egui::CornerRadius::same(theme::control_radius(scale))
        },
        if hovered {
            style.buttons.collapse_tabs_bg_fill
        } else if rail {
            style.tab_bar.bg_fill
        } else {
            theme::widget()
        },
    );
    let cross = egui::vec2(-direction.y, direction.x);
    painter.add(egui::Shape::convex_polygon(
        vec![
            center - direction * 4.0 + cross * 4.0,
            center + direction * 4.0,
            center - direction * 4.0 - cross * 4.0,
        ],
        if hovered {
            style.buttons.collapse_tabs_active_color
        } else {
            style.buttons.collapse_tabs_color
        },
        egui::Stroke::NONE,
    ));
    if rail {
        let galley = painter.layout_no_wrap(
            name.to_owned(),
            egui::TextStyle::Button.resolve(ui.style()),
            style.tab.active.text_color,
        );
        let available = if vertical { rect.width() } else { rect.height() };
        if galley.size().x + size + 12.0 <= available {
            let (anchor, angle) = if vertical {
                (Pos2::new(rect.left() + size + 6.0, rect.center().y - galley.size().y * 0.5), 0.0)
            } else {
                (
                    Pos2::new(
                        rect.center().x - galley.size().y * 0.5,
                        rect.top() + size + 6.0 + galley.size().x,
                    ),
                    -std::f32::consts::FRAC_PI_2,
                )
            };
            painter.add(
                egui::epaint::TextShape::new(anchor, galley, style.tab.active.text_color)
                    .with_angle(angle),
            );
        }
    }
    response.on_hover_text(format!("{} {name}", if rail { "Expand" } else { "Collapse" })).clicked()
}
