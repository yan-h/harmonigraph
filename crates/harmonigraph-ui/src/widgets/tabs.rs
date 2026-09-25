//! A row of destinations that gives up room from its right end: the tabs that
//! no longer fit move, in order, into one overflow menu after the last tab
//! that does.
//!
//! All or nothing was the old answer, and the wrong one for a strip that is
//! usually one tab short: a column a few points too narrow hid every
//! destination behind a dropdown to spare the one that did not fit.

use egui::{Atom, Rect, Response, Ui, Vec2};

use super::rows::{option_label, option_width};
use crate::theme;

/// Draw `options` as tabs, as many as fit from the left, and the rest behind a
/// trailing overflow button. The selected tab is underlined at `baseline`, the
/// bottom edge of the header the strip stands in.
///
/// Tabs are text on the header rather than buttons: dim at rest, bright when
/// hovered or selected, and the selected one underlined in the accent. Filled
/// they were the same widget as every choice row in the page below, and read
/// as one more of them.
///
/// The overflow button shows three dots while the selected tab is on the strip.
/// When the selected tab is one of the hidden ones, the button carries its name
/// and the underline instead, so the current destination is always labelled
/// and nothing on the strip shifts to make room for it. Where not even one tab
/// fits beside it, that button is the whole strip, its name truncated to the
/// room: one dropdown, the same one as at every other width.
pub(crate) fn tab_strip<T: Copy + PartialEq>(
    ui: &mut Ui,
    id: &str,
    value: &mut T,
    options: &[(T, &str)],
    baseline: f32,
) {
    let widths: Vec<f32> = options.iter().map(|(_, label)| option_width(ui, label)).collect();
    let selected = options.iter().position(|(choice, _)| *choice == *value);
    let shown = strip_fit(&widths, ui.spacing().item_spacing.x, ui.available_width(), |shown| {
        overflow_width(ui, selected.filter(|&at| at >= shown).map(|at| options[at].1))
    });
    ui.horizontal(|ui| {
        for &(choice, label) in &options[..shown] {
            tab(ui, value, choice, label, baseline);
        }
        if shown < options.len() {
            overflow(ui, id, value, &options[shown..], baseline);
        }
    });
}

/// One tab: its label, as wide as [`option_width`] measures it so the strip's
/// fit is the strip drawn, and its underline when selected.
fn tab<T: Copy + PartialEq>(ui: &mut Ui, value: &mut T, choice: T, label: &str, baseline: f32) {
    let selected = *value == choice;
    let galley = egui::WidgetText::from(option_label(label)).into_galley(
        ui,
        Some(egui::TextWrapMode::Extend),
        f32::INFINITY,
        egui::TextStyle::Button,
    );
    let size = Vec2::new(option_width(ui, label), ui.spacing().interact_size.y);
    let (rect, response) = ui.allocate_exact_size(size, egui::Sense::click());
    response.widget_info(|| {
        egui::WidgetInfo::selected(
            egui::WidgetType::SelectableLabel,
            ui.is_enabled(),
            selected,
            galley.text(),
        )
    });
    if response.clicked() {
        *value = choice;
    }
    let hot = response.hovered() || response.has_focus();
    let color = if selected || hot { theme::text() } else { theme::text_dim() };
    ui.painter().galley(rect.center() - galley.size() * 0.5, galley, color);
    if selected {
        underline(ui, &response, baseline);
    }
}

/// The selected tab's mark: an accent bar under its label, standing on the
/// header's bottom edge.
fn underline(ui: &Ui, response: &Response, baseline: f32) {
    let scale = theme::ui_scale(ui.ctx());
    let inset = ui.spacing().button_padding.x * 0.5;
    let bar = Rect::from_x_y_ranges(
        (response.rect.left() + inset)..=(response.rect.right() - inset),
        (baseline - 2.0 * scale)..=baseline,
    );
    ui.painter().rect_filled(bar, egui::CornerRadius::same(1), theme::accent());
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

fn overflow<T: Copy + PartialEq>(
    ui: &mut Ui,
    id: &str,
    value: &mut T,
    hidden: &[(T, &str)],
    baseline: f32,
) {
    let current = hidden.iter().find(|(choice, _)| *choice == *value).map(|&(_, label)| label);
    let icon = ui.make_persistent_id((id, "overflow icon"));
    let size = Vec2::splat(ui.spacing().icon_width);
    // Frameless, as the tabs are: hover brightens the glyph rather than
    // filling a button.
    let button = match current {
        Some(label) => {
            egui::Button::new((option_label(label).color(theme::text()), Atom::custom(icon, size)))
                .truncate()
        }
        None => egui::Button::new(Atom::custom(icon, size))
            .min_size(Vec2::splat(ui.spacing().interact_size.y)),
    }
    .frame(false);
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
    if current.is_some() {
        underline(ui, &response.response, baseline);
    }
    let response = response.response.on_hover_text("More tabs");
    let width = response.rect.width();
    egui::Popup::menu(&response)
        .id(ui.make_persistent_id((id, "overflow menu")))
        .style(super::menu_style(ui.ctx()))
        .show(|ui| {
            // Never narrower than the button that opened it, as a dropdown's
            // list is never narrower than the dropdown.
            ui.set_min_width(width);
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
    use super::{option_width, overflow_width, strip_fit, tab_strip};

    const TABS: [(u8, &str); 4] = [(0, "Tuning"), (1, "Lattice"), (2, "Analyzer"), (3, "Colors")];

    fn drawn(width: f32, selected: u8) -> (Vec<String>, usize) {
        let mut value = selected;
        let shapes = crate::tests::probe::painted_full(egui::vec2(width, 60.0), |ui| {
            tab_strip(ui, "tabs", &mut value, &TABS, 60.0)
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

    /// The selected tab is marked by an accent bar standing on the baseline
    /// under it, and no tab is filled: a filled tab is the choice-row pill.
    #[test]
    fn the_selected_tab_is_underlined_rather_than_filled() {
        let mut value = 1;
        let shapes = crate::tests::probe::painted_full(egui::vec2(600.0, 60.0), |ui| {
            tab_strip(ui, "tabs", &mut value, &TABS, 40.0)
        })
        .shapes;
        let label = |name: &str| {
            shapes.iter().find_map(|cs| match &cs.shape {
                egui::Shape::Text(text) if text.galley.text() == name => {
                    Some(text.visual_bounding_rect())
                }
                _ => None,
            })
        };
        let rects: Vec<_> = shapes
            .iter()
            .filter_map(|cs| match &cs.shape {
                egui::Shape::Rect(rect) => Some(rect.clone()),
                _ => None,
            })
            .collect();
        let [bar] = rects.as_slice() else { panic!("expected one underline: {rects:?}") };
        assert_eq!(bar.fill, crate::theme::accent());
        assert_eq!(bar.rect.bottom(), 40.0, "the underline is off the baseline");
        let lattice = label("Lattice").expect("the selected tab's label");
        assert!(
            bar.rect.left() < lattice.center().x && lattice.center().x < bar.rect.right(),
            "the underline is not under the selected tab: {:?} vs {lattice:?}",
            bar.rect,
        );
    }

    /// A selected tab that has overflowed names the overflow button, so the
    /// current destination is always on the strip.
    #[test]
    fn an_overflowed_selection_names_the_overflow_button() {
        let (labels, dots) = drawn(150.0, 3);
        assert_eq!(labels.last().map(String::as_str), Some("Colors"), "{labels:?}");
        assert_eq!(dots, 0, "the named overflow button still drew dots");
    }

    /// Where not one tab fits, the strip is the named overflow button alone,
    /// truncated inside the room rather than overrunning it.
    #[test]
    fn a_strip_with_room_for_no_tab_is_the_named_overflow_button() {
        let width = 40.0;
        let mut value = 2;
        let shapes = crate::tests::probe::painted_full(egui::vec2(width, 60.0), |ui| {
            tab_strip(ui, "tabs", &mut value, &TABS, 60.0)
        })
        .shapes;
        let texts: Vec<_> = shapes
            .iter()
            .filter_map(|cs| match &cs.shape {
                egui::Shape::Text(text) => {
                    Some((text.galley.text().to_owned(), text.visual_bounding_rect()))
                }
                _ => None,
            })
            .collect();
        assert_eq!(texts.len(), 1, "{texts:?}");
        assert_eq!(texts[0].0, "Analyzer");
        assert!(texts[0].1.right() <= width, "the name overran the strip: {texts:?}");
        assert!(!shapes.iter().any(|cs| matches!(cs.shape, egui::Shape::Circle(_))));
    }

    /// A menu first opened over a short hidden tab still fits a longer one
    /// that joins it later. egui keeps a popup's size from one opening to the
    /// next, so a label free to wrap would stay inside the first width.
    #[test]
    fn the_overflow_menu_grows_to_fit_a_longer_hidden_tab() {
        const TABS: [(u8, &str); 4] = [(0, "Tuning"), (1, "Lattice"), (2, "Analyzer"), (3, "Ab")];
        let ctx = crate::tests::probe::themed();
        let mut value = 0;
        let mut frame = |width: f32, events: Vec<egui::Event>| {
            let rect = egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(width, 60.0));
            let screen = egui::vec2(600.0, 300.0);
            crate::tests::probe::events_into(&ctx, screen, rect, events, |ui| {
                tab_strip(ui, "tabs", &mut value, &TABS, 60.0)
            })
            .shapes
        };
        // Room for all but the last tab, then for all but the last two.
        let mut room = [0.0; 2];
        crate::tests::probe::frame_full(&ctx, egui::vec2(600.0, 300.0), |ui| {
            let widths: Vec<f32> = TABS.iter().map(|(_, l)| option_width(ui, l)).collect();
            let gap = ui.spacing().item_spacing.x;
            let dots = overflow_width(ui, None);
            let run = |count: usize| widths[..count].iter().sum::<f32>() + gap * count as f32;
            room = [run(3) + dots + 1.0, run(2) + dots + 1.0];
        });
        let mut open = |width: f32| {
            egui::Popup::close_all(&ctx);
            let shapes = frame(width, vec![]);
            let dots: Vec<_> = shapes
                .iter()
                .filter_map(|cs| match &cs.shape {
                    egui::Shape::Circle(circle) => Some(circle.center),
                    _ => None,
                })
                .collect();
            assert_eq!(dots.len(), 3, "no overflow button at {width}");
            let at = dots[1];
            frame(width, vec![egui::Event::PointerMoved(at)]);
            frame(width, vec![crate::tests::probe::press(at, true)]);
            frame(width, vec![crate::tests::probe::press(at, false)]);
            frame(width, vec![])
        };
        open(room[0]);
        let rows: Vec<_> = open(room[1])
            .iter()
            .filter_map(|cs| match &cs.shape {
                egui::Shape::Text(text) if text.galley.text() == "Analyzer" => {
                    Some(text.galley.rows.len())
                }
                _ => None,
            })
            .collect();
        assert_eq!(rows, [1], "the reopened menu wrapped its longer entry");
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
