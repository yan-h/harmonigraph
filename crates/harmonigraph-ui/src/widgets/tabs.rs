//! A row of destinations that gives up room from its right end: the tabs that
//! no longer fit move, in order, into one overflow menu after the last tab
//! that does.
//!
//! All or nothing was the old answer, and the wrong one for a strip that is
//! usually one tab short: a column a few points too narrow hid every
//! destination behind a dropdown to spare the one that did not fit.

use egui::{Atom, Rect, Ui, Vec2};

use super::rows::{option_label, option_width};
use crate::theme;

/// Draw `options` as selectable buttons, as many as fit from the left, and the
/// rest behind a trailing overflow button.
///
/// The overflow button shows three dots while the selected tab is on the strip.
/// When the selected tab is one of the hidden ones, the button carries its name
/// and the selected fill instead, so the current destination is always
/// labelled and nothing on the strip shifts to make room for it.
pub(crate) fn tab_strip<T: Copy + PartialEq>(
    ui: &mut Ui,
    id: &str,
    value: &mut T,
    options: &[(T, &str)],
) {
    let widths: Vec<f32> = options.iter().map(|(_, label)| option_width(ui, label)).collect();
    let selected = options.iter().position(|(choice, _)| *choice == *value);
    let shown = strip_fit(&widths, ui.spacing().item_spacing.x, ui.available_width(), |shown| {
        overflow_width(ui, selected.filter(|&at| at >= shown).map(|at| options[at].1))
    });
    if shown == 0 {
        // Not one tab fits beside the overflow button, so the button is all
        // there is: the current tab's name in a dropdown the width of the
        // room, truncated rather than overrunning the strip.
        let current = selected.map_or("", |at| options[at].1);
        super::selected_combo(
            ui,
            egui::ComboBox::from_id_salt(id)
                .selected_text(option_label(current))
                .width(ui.available_width())
                .truncate(),
            |ui| {
                for &(choice, label) in options {
                    ui.selectable_value(value, choice, option_label(label));
                }
            },
        );
        return;
    }
    ui.horizontal(|ui| {
        for &(choice, label) in &options[..shown] {
            ui.selectable_value(value, choice, option_label(label));
        }
        if shown < options.len() {
            overflow(ui, id, value, &options[shown..]);
        }
    });
}

/// How many leading tabs fit beside the overflow button they would need,
/// given each tab's width, the gap between neighbours, the room there is,
/// and the overflow button's width with that many tabs shown.
///
/// Every tab fits or at least one goes, never all but none: the strip always
/// shows as many tabs as it can, and at worst only the overflow button.
fn strip_fit(widths: &[f32], gap: f32, room: f32, overflow: impl Fn(usize) -> f32) -> usize {
    let run = |count: usize| widths[..count].iter().sum::<f32>() + gap * count as f32;
    if run(widths.len()) - gap <= room {
        return widths.len();
    }
    (0..widths.len()).rev().find(|&count| run(count) + overflow(count) <= room).unwrap_or(0)
}

/// The overflow button's natural width: the hidden selected tab's name when
/// there is one, then the icon.
fn overflow_width(ui: &Ui, label: Option<&str>) -> f32 {
    let icon = ui.spacing().icon_width;
    match label {
        Some(label) => option_width(ui, label) + ui.spacing().icon_spacing + icon,
        None => (icon + 2.0 * ui.spacing().button_padding.x).max(ui.spacing().interact_size.y),
    }
}

fn overflow<T: Copy + PartialEq>(ui: &mut Ui, id: &str, value: &mut T, hidden: &[(T, &str)]) {
    let current = hidden.iter().find(|(choice, _)| *choice == *value).map(|&(_, label)| label);
    let icon = ui.make_persistent_id((id, "overflow icon"));
    let size = Vec2::splat(ui.spacing().icon_width);
    let button = match current {
        Some(label) => {
            egui::Button::selectable(true, (option_label(label), Atom::custom(icon, size)))
        }
        None => egui::Button::selectable(false, Atom::custom(icon, size))
            .min_size(Vec2::splat(ui.spacing().interact_size.y)),
    };
    let response = button.atom_ui(ui);
    if let Some(rect) = response.rect(icon) {
        let hot = response.response.hovered() || response.response.has_focus();
        let color = if current.is_some() || hot { theme::text() } else { theme::text_dim() };
        if current.is_some() {
            paint_chevron(ui, rect, color);
        } else {
            paint_dots(ui, rect, color);
        }
    }
    let response = response.response.on_hover_text("More tabs");
    egui::Popup::menu(&response).id(ui.make_persistent_id((id, "overflow menu"))).show(|ui| {
        for &(choice, label) in hidden {
            if ui.selectable_label(*value == choice, option_label(label)).clicked() {
                *value = choice;
            }
        }
    });
}

/// Three dots across the middle: the "more" glyph, painted rather than typed
/// because the interface font has no ellipsis of its own weight.
fn paint_dots(ui: &Ui, rect: Rect, color: egui::Color32) {
    let scale = theme::ui_scale(ui.ctx());
    let step = rect.width() * 0.3;
    for offset in [-step, 0.0, step] {
        ui.painter().circle_filled(rect.center() + egui::vec2(offset, 0.0), 1.3 * scale, color);
    }
}

/// A downward chevron in the fold buttons' stroke: this button opens a menu.
fn paint_chevron(ui: &Ui, rect: Rect, color: egui::Color32) {
    let scale = theme::ui_scale(ui.ctx());
    let c = rect.center();
    ui.painter().add(egui::Shape::line(
        vec![
            c + egui::vec2(-4.0, -2.0) * scale,
            c + egui::vec2(0.0, 2.0) * scale,
            c + egui::vec2(4.0, -2.0) * scale,
        ],
        egui::Stroke::new(1.5 * scale, color),
    ));
}

#[cfg(test)]
mod tests {
    use super::{strip_fit, tab_strip};

    const TABS: [(u8, &str); 4] = [(0, "Tuning"), (1, "Lattice"), (2, "Analyzer"), (3, "Colors")];

    fn drawn(width: f32, selected: u8) -> (Vec<String>, usize) {
        let mut value = selected;
        let shapes = crate::tests::probe::painted_full(egui::vec2(width, 60.0), |ui| {
            tab_strip(ui, "tabs", &mut value, &TABS)
        })
        .shapes;
        let labels = shapes
            .iter()
            .filter_map(|cs| match &cs.shape {
                egui::Shape::Text(text) => Some(text.galley.text().to_owned()),
                _ => None,
            })
            .collect();
        let dots = shapes.iter().filter(|cs| matches!(cs.shape, egui::Shape::Circle(_))).count();
        (labels, dots)
    }

    /// Room for every tab draws every tab and no overflow button; less room
    /// keeps the leading tabs and puts three dots after them.
    #[test]
    fn a_narrowing_strip_keeps_its_leading_tabs_beside_an_overflow_button() {
        assert_eq!(drawn(600.0, 0), (TABS.map(|(_, label)| label.to_owned()).to_vec(), 0));
        let (labels, dots) = drawn(150.0, 0);
        assert!(labels.len() < TABS.len() && !labels.is_empty(), "{labels:?}");
        let leading: Vec<_> = TABS[..labels.len()].iter().map(|(_, l)| l.to_string()).collect();
        assert_eq!(labels, leading);
        assert_eq!(dots, 3, "the hidden tabs have no overflow button");
    }

    /// A selected tab that has overflowed names the overflow button, so the
    /// current destination is always on the strip.
    #[test]
    fn an_overflowed_selection_names_the_overflow_button() {
        let (labels, dots) = drawn(150.0, 3);
        assert_eq!(labels.last().map(String::as_str), Some("Colors"), "{labels:?}");
        assert_eq!(dots, 0, "the named overflow button still drew dots");
    }

    #[test]
    fn a_strip_with_room_shows_every_tab() {
        assert_eq!(strip_fit(&[50.0, 50.0, 50.0], 4.0, 158.0, |_| 30.0), 3);
    }

    /// One tab short of room hides one tab, not all of them.
    #[test]
    fn a_strip_short_of_room_gives_up_tabs_from_the_right() {
        // 50 + 4 + 50 + 4 = 108 before a 30 point overflow button: 138.
        assert_eq!(strip_fit(&[50.0, 50.0, 50.0], 4.0, 150.0, |_| 30.0), 2);
        assert_eq!(strip_fit(&[50.0, 50.0, 50.0], 4.0, 100.0, |_| 30.0), 1);
        assert_eq!(strip_fit(&[50.0, 50.0, 50.0], 4.0, 40.0, |_| 30.0), 0);
    }

    /// A wider overflow button — one carrying a hidden selected tab's name —
    /// takes its room from the tabs.
    #[test]
    fn a_named_overflow_button_takes_its_room_from_the_tabs() {
        let named = |shown: usize| if shown < 2 { 30.0 } else { 80.0 };
        assert_eq!(strip_fit(&[50.0, 50.0, 50.0], 4.0, 150.0, named), 1);
    }
}
