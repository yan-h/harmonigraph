//! Editor-only folds inside the Analyzer. Hidden regions keep their depth in
//! points; a virtual full picture keeps the surviving region's calibration.

use egui::{Rect, Sense, Vec2};

use super::{axes::Axes, gestures, Navigation, RegionView};
use crate::{
    panes::DOCKED_SURFACE,
    theme,
    widgets::{Rail, RailLook},
    PictureState,
};

#[derive(Clone, Default, PartialEq, serde::Serialize, serde::Deserialize)]
#[serde(default)]
pub(crate) struct Regions {
    pub collapsed: [bool; 2],
    /// Last fully open composition. Temporary expansion into a folded region
    /// never changes this snapshot or the appearance dial.
    geometry: Option<Geometry>,
}

#[derive(Clone, Copy, Default, PartialEq, serde::Serialize, serde::Deserialize)]
#[serde(default)]
struct Geometry {
    depths: [f32; 2],
    dial: f32,
    vertical: bool,
}

impl Geometry {
    fn matches(&self, cfg: &crate::SpectrumConfig) -> bool {
        self.dial == cfg.roll_fraction && self.vertical == cfg.orientation.is_time_vertical()
    }
}

impl Regions {
    /// Take on what the pane writes by itself as it lays the Analyzer out:
    /// the depths it draws, every frame, from the pane's size, the Tape tab and
    /// the spectrum hold. The pane writes
    /// the depths every frame it draws whole, so a copy settled at each write
    /// holds `live`'s. An edit that moves them moves the dial or the layout on
    /// the way, which the key sees, a divider dragged back to where it began
    /// included (`SharedState::report_edit` reports a gesture that moved).
    pub(crate) fn copy_layout(&mut self, live: &Regions) {
        self.geometry = live.geometry;
    }

    pub fn sanitize(&mut self) {
        self.geometry = self.geometry.filter(|geometry| {
            geometry.depths.iter().all(|d| d.is_finite() && (0.0..=16_384.0).contains(d))
                && geometry.depths.iter().sum::<f32>() > 0.0
                && geometry.dial.is_finite()
                && (0.0..=1.0).contains(&geometry.dial)
        });
    }

    /// The docked Analyzer: in Tape mode, the audition's controls along the
    /// bottom, and with the far region open the tape's overview under them,
    /// and the picture over the rest.
    pub fn draw(&mut self, ui: &mut egui::Ui, state: &mut PictureState, now: f64, frameless: bool) {
        let full = ui.available_rect_before_wrap();
        let (row, strip) = super::tape::rows(theme::ui_scale(ui.ctx()));
        let view = &state.runtime.tape;
        if !(view.present && view.showing) || full.height() <= 3.0 * row {
            return self.draw_picture(ui, state, now, frameless);
        }
        // Only over a far region with depth to show the tape in, and the
        // first to go on a short pane: the controls hold Stop.
        let overview = super::tape::active(state, Navigation::Docked)
            && !self.collapsed[1]
            && gestures::spectrum_split(state, DOCKED_SURFACE) < 1.0
            && full.height() > 3.0 * (row + strip);
        let below = row + if overview { strip } else { 0.0 };
        let (picture, bottom) = full.split_top_bottom_at_y(full.bottom() - below);
        let (controls, strip) = bottom.split_top_bottom_at_y(bottom.top() + row);
        // First, so a stretch moved on it is the one the picture draws.
        if overview {
            super::tape::overview(ui, strip, state, DOCKED_SURFACE);
        }
        super::tape::controls(ui, controls, state);
        ui.scope_builder(egui::UiBuilder::new().max_rect(picture), |ui| {
            self.draw_picture(ui, state, now, frameless);
        });
        ui.allocate_rect(bottom, Sense::hover());
    }

    fn draw_picture(
        &mut self,
        ui: &mut egui::Ui,
        state: &mut PictureState,
        now: f64,
        frameless: bool,
    ) {
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
        let rail = if frameless {
            0.0
        } else {
            theme::tab_bar_height(theme::ui_scale(ui.ctx())).min(
                depth / (usize::from(folded[0]) + usize::from(folded[1] && history)).max(1) as f32,
            )
        };
        let vertical = cfg.orientation.is_time_vertical();
        let share = super::axes::spectrum_share(&cfg);
        let geometry = self.geometry.filter(|geometry| geometry.matches(&cfg));
        let mut drawn_depths =
            geometry.map_or([depth * share, depth * (1.0 - share)], |g| g.depths);
        let rail_depths =
            [if folded[0] { rail } else { 0.0 }, if folded[1] && history { rail } else { 0.0 }];
        let available = (depth - rail_depths.iter().sum::<f32>()).max(0.0);
        if !folded[0] && !folded[1] {
            if std::mem::take(&mut state.surfaces.spectrum_hold.relayout) {
                if let Some(geometry) = geometry {
                    gestures::restore_spectrum(state, geometry.depths);
                }
            }
            super::hold_spectrum(state, rect.size());
            let (shown, solo) = (state.shown(), state.surfaces.star_solo.spectrogram);
            let navigation = Navigation::Docked;
            super::spectral_pane(ui, state, now, DOCKED_SURFACE, 1.0, navigation, shown, solo);
            let split = gestures::spectrum_split(state, DOCKED_SURFACE);
            drawn_depths = [depth * split, depth * (1.0 - split)];
            self.geometry = Some(Geometry {
                depths: drawn_depths,
                dial: state.appearance.spectrum.roll_fraction,
                vertical,
            });
        } else {
            ui.allocate_rect(rect, Sense::hover());
            if !folded[0] {
                drawn_depths[0] = available;
            }
            if !folded[1] {
                drawn_depths[1] = available;
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
                extend(&mut virtual_rect, -dir * drawn_depths[0]);
            }
            if folded[1] && history {
                extend(&mut virtual_rect, dir * drawn_depths[1]);
            }
            let total = Axes::new(virtual_rect, &cfg).depth_len();
            let split = (drawn_depths[0] / total.max(1.0)).clamp(0.0, 1.0);
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
                    state.shown(),
                    state.surfaces.star_solo.spectrogram,
                );
            }
        }
        if rail <= 0.0 {
            return;
        }
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
                control_rect(&axes, index, theme::tab_bar_height(theme::ui_scale(ui.ctx())))
                    .intersect(rect)
            };
            if !band.is_positive() {
                continue;
            }
            let direction = axes.dir_depth() * if (index == 0) == closed { 1.0 } else { -1.0 };
            // Toggled at the click: this frame has drawn the old geometry
            // already, and the next draws the new one.
            if control(ui, band, direction, closed, vertical, name, index) {
                self.collapsed[index] = !closed;
                ui.ctx().request_repaint();
            }
        }
    }
}

fn extend(rect: &mut Rect, delta: Vec2) {
    rect.min += delta.min(Vec2::ZERO);
    rect.max += delta.max(Vec2::ZERO);
}

/// Put each button at the far end of the region it folds, away from the busy
/// now-line and its drag band. `Axes` turns the same two depth endpoints into
/// left/right or top/bottom for every orientation. Flush with the high-pitch
/// edge and the region's outer end, each square reads as pane chrome rather
/// than a floating widget over the picture.
///
/// The cell is a rail's width, not a row's, so the visible button lands on the
/// spot its rail's Expand button takes once the region folds (a fold never
/// moves the pane's edges), rather than a few points off it.
fn control_rect(axes: &Axes, index: usize, size: f32) -> Rect {
    let inset = size * 0.5;
    let pitch = 1.0 - inset / axes.pitch_len();
    let d = if index == 0 { inset / axes.depth_len() } else { 1.0 - inset / axes.depth_len() };
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
    let id = egui::Id::new(("analyzer region fold", index));
    if rail {
        // Time down the pane stacks the regions, so a folded one is a row
        // across it; time across stands them side by side, each a column.
        let run = if vertical { Rail::Row } else { Rail::Column };
        // A region's own look: a tab bar's button even on a rail squeezed
        // thinner, the title 6 points from it at every scale, named.
        let button = theme::tab_bar_height(theme::ui_scale(ui.ctx()));
        let look = RailLook { button, gap: 6.0, named: true };
        return crate::widgets::paint_rail(ui, rect, id, run, direction, name, look);
    }
    let response = ui.interact(rect, id, Sense::click());
    let collapse = format!("Collapse {name}");
    response.widget_info(|| {
        egui::WidgetInfo::labeled(egui::WidgetType::Button, true, collapse.clone())
    });
    crate::widgets::paint_fold(ui, &response, rect, direction);
    response.on_hover_text(collapse).clicked()
}
