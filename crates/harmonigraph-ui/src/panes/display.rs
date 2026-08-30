//! The Display tab: how everything on screen is drawn, one PAGE per picture —
//! the [`color`](super::color) tables and the light on them, the lattice
//! itself ([`view`](super::view), [`nodes`](super::nodes),
//! [`labels`](super::labels), [`plus`](super::plus)), the Analyzer's own
//! settings ([`spectral`](super::spectral)), and the machine around all of
//! them ([`system`](super::system)).
//!
//! **A setting lives on the page named for the PICTURE it changes.** Anything
//! about color lives on Colors, whichever picture it paints, because the two
//! color tables are the one subject a reader comes here holding rather than a
//! property of either picture; the Lattice page is the lattice and everything
//! drawn on it; the Analyzer page is the analyzer, and the Spiral reading the
//! same frame; System is the machine around the pictures rather than any of
//! them. That makes a placement question "which picture does this move?" —
//! answerable out of the render code — rather than "what is this about?",
//! which is argued afresh in every docstring.
//!
//! Pages inside ONE tab rather than a tab each, because a settings tab is paid
//! for in bar width at the editor's DEFAULT window: per-pane tabs overflow it,
//! and egui_dock answers overflow by scrolling the bar, so every tab stays
//! clickable and the one past the edge is one a new user never learns exists
//! (#287). Pages also SWITCH rather than nest — exactly one body under the
//! picker, never a body inside a body — and the picker's four names stay on
//! screen whichever of them is showing, so the row is a permanent table of
//! contents.
//!
//! Each page wears its pane's audited name (#286), "Analyzer" deliberately
//! shared with the display pane's title — see [`tab_title`](super::tab_title).

use super::color::color_pane;
use super::labels::labels_pane;
use super::nodes::nodes_pane;
use super::plus::plus_pane;
use super::spectral::spectrum_settings_pane;
use super::system::system_pane;
use super::view::view_pane;
use crate::params::ParamBackend;
use crate::theme;
use crate::SharedState;

/// Display's pages, in the order the picker lists them: the colors every
/// picture is painted with, then the lattice, then the analyzer beside it, and
/// last the machine around them.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub enum DisplayPage {
    #[default]
    Colors,
    Lattice,
    Analyzer,
    System,
}

impl DisplayPage {
    /// Every page, in the order named on the enum's own doc.
    ///
    /// Built from an exhaustive `match` rather than written out as a bare
    /// literal, so the list cannot fall behind the enum — the same guard
    /// `SpectralOrientation::ALL` in this crate uses, for the same reason.
    pub const ALL: [DisplayPage; 4] = {
        use DisplayPage::*;
        // Exhaustive, and the compiler checks it. The arm is `()` because
        // what is wanted is the coverage error, not the value.
        const fn covered(page: DisplayPage) {
            match page {
                Colors | Lattice | Analyzer | System => (),
            }
        }
        covered(Colors);
        [Colors, Lattice, Analyzer, System]
    };

    /// The page's name, on its picker label and nowhere else.
    pub fn title(self) -> &'static str {
        match self {
            DisplayPage::Colors => "Colors",
            DisplayPage::Lattice => "Lattice",
            DisplayPage::Analyzer => "Analyzer",
            DisplayPage::System => "System",
        }
    }
}

/// The picker, then the one page it selects.
///
/// Every body draws straight into the tab and scrolls in the dock's own
/// `ScrollArea` (see `Viewer::scroll_bars`), rather than building a second one
/// per page. Both scroll the same list to the same wheel; what tells them apart
/// is where the bar goes. The dock's area is the pane, so its bar falls in the
/// pane's right margin — one built here starts at the content box, and a
/// floating bar draws over the content it has no room beside, which is the
/// right end of every bar on every page.
pub(super) fn display_pane(ui: &mut egui::Ui, state: &mut SharedState, params: &dyn ParamBackend) {
    // Copied out and written back, so the picker can hold the choice while the
    // page body borrows the whole state.
    let mut page = state.display_page;
    page_picker(ui, &mut page);
    state.display_page = page;
    match page {
        DisplayPage::Colors => color_pane(ui, state, params),
        DisplayPage::Lattice => lattice_page(ui, state, params),
        DisplayPage::Analyzer => spectrum_settings_pane(ui, state),
        DisplayPage::System => system_pane(ui, state),
    }
}

/// The strip of page names at the top of the tab, and the whole of its
/// navigation.
///
/// No label in front of it, unlike the choice rows inside the pages: one of
/// those names a setting and then offers its values, where this row IS what the
/// tab holds — a word in front of it would read as a setting called Page.
///
/// Text and an underline rather than selectable buttons: this row NAVIGATES to
/// another body, while a filled selectable button inside that body CHANGES a
/// setting. Giving both jobs one resting shape makes the page names read as
/// another enum setting. The hairline under the whole strip ties its names
/// together, and the accent stroke ties the active name to that boundary.
///
/// The row still wraps when the column is too narrow to hold every name. A
/// page is reached by clicking its name and by nothing else, so a name past the
/// pane edge is a page with no way into it: horizontal scrolling is off in the
/// dock (see [`Viewer::scroll_bars`](super::Viewer)).
fn page_picker(ui: &mut egui::Ui, page: &mut DisplayPage) {
    let selected = *page;
    let scale = theme::ui_scale(ui.ctx());
    let row = ui.horizontal_wrapped(|ui| {
        let mut tabs = Vec::with_capacity(DisplayPage::ALL.len());
        for choice in DisplayPage::ALL {
            let title = choice.title();
            let font = egui::TextStyle::Button.resolve(ui.style());
            let width = ui.painter().layout_no_wrap(title.to_owned(), font, theme::text()).size().x;
            let size = egui::vec2(width + 12.0 * scale, theme::row_height(scale));
            let (rect, response) = ui.allocate_exact_size(size, egui::Sense::click());
            response.widget_info(|| {
                egui::WidgetInfo::selected(
                    egui::WidgetType::SelectableLabel,
                    ui.is_enabled(),
                    choice == selected,
                    title,
                )
            });
            tabs.push((choice, response, rect));
        }
        tabs
    });

    for (choice, response, _) in &row.inner {
        if response.clicked() {
            *page = *choice;
        }
    }

    // Match the dock tabs' hover without giving an inactive destination the
    // persistent fill that makes option controls read as selected.
    for (choice, response, rect) in &row.inner {
        if *choice != *page && response.hovered() {
            ui.painter().rect_filled(*rect, egui::CornerRadius::ZERO, theme::surface_faint());
        }
    }

    // Each wrapped line is a strip of its own. One boundary under the whole
    // block leaves an active page on any earlier line floating between rows.
    let mut boundaries: Vec<(egui::Rangef, f32)> = Vec::new();
    for (_, _, rect) in &row.inner {
        match boundaries.last_mut() {
            Some((range, bottom)) if (*bottom - rect.bottom()).abs() < 0.5 => {
                range.max = rect.right();
            }
            _ => boundaries.push((rect.x_range(), rect.bottom())),
        }
    }
    // Preserve the full strip boundary when the picker fits on one line.
    if boundaries.len() == 1 {
        boundaries[0].0 = row.response.rect.x_range();
    }
    for (range, bottom) in boundaries {
        ui.painter().hline(range, bottom, egui::Stroke::new(1.0, theme::hairline()));
    }

    for (choice, response, rect) in row.inner {
        let active = choice == *page;
        let font = egui::TextStyle::Button.resolve(ui.style());
        let highlighted = active || response.hovered() || response.has_focus();
        let color = if highlighted { theme::text() } else { theme::text_dim() };
        ui.painter().text(rect.center(), egui::Align2::CENTER_CENTER, choice.title(), font, color);
        if choice == *page {
            ui.painter().hline(
                rect.x_range(),
                rect.bottom(),
                egui::Stroke::new(2.0 * scale, theme::accent()),
            );
        }
    }
}

/// The Lattice page: the whole lattice picture, read from the camera in front
/// of it inward. What is framed ([`view_pane`]), how a sounding note draws
/// ([`nodes_pane`]), the text riding it ([`labels_pane`]), and last what is
/// there when nothing sounds at all ([`plus_pane`]).
///
/// [`view_pane`] leads, so its own first heading is the page's and stays plain
/// — see [`section`](super::section) for the rule that separates a section from
/// the one above it.
fn lattice_page(ui: &mut egui::Ui, state: &mut SharedState, params: &dyn ParamBackend) {
    view_pane(ui, state);
    nodes_pane(ui, state, params);
    labels_pane(ui, state);
    plus_pane(ui, state);
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Page navigation is text on one shared boundary, while an enum setting
    /// is a row of filled controls. The distinction is the whole reason this
    /// picker has its own widget instead of `selectable_value`.
    #[test]
    fn the_page_picker_does_not_read_as_an_option_row() {
        let mut page = DisplayPage::Analyzer;
        let shapes = crate::tests::probe::painted_full(egui::vec2(400.0, 100.0), |ui| {
            page_picker(ui, &mut page)
        })
        .shapes;

        let fills: Vec<_> = shapes
            .iter()
            .filter_map(|shape| match &shape.shape {
                egui::Shape::Rect(rect) if rect.fill != egui::Color32::TRANSPARENT => {
                    Some(rect.fill)
                }
                _ => None,
            })
            .collect();
        assert!(fills.is_empty(), "the page picker drew filled option buttons: {fills:?}");

        let lines: Vec<_> = shapes
            .iter()
            .filter_map(|shape| match &shape.shape {
                egui::Shape::LineSegment { points, stroke } => Some((points, stroke)),
                _ => None,
            })
            .collect();
        let boundary = lines.iter().find(|(_, stroke)| stroke.color == theme::hairline());
        let active = lines.iter().find(|(_, stroke)| stroke.color == theme::accent());
        let (boundary, active) = (
            boundary.expect("the page names have no common boundary"),
            active.expect("the active page has no accent underline"),
        );
        let width = |points: &[egui::Pos2; 2]| (points[1].x - points[0].x).abs();
        assert!(
            width(active.0) < width(boundary.0),
            "the active stroke does not identify one page",
        );
    }

    /// The underline and color identify the current page without changing its
    /// letterforms. The Display pages use the same weight behavior as tabs.
    #[test]
    fn selecting_a_page_does_not_bold_its_label() {
        let mut page = DisplayPage::Analyzer;
        let shapes = crate::tests::probe::painted_full(egui::vec2(400.0, 100.0), |ui| {
            page_picker(ui, &mut page)
        })
        .shapes;
        let font = |title: &str| {
            shapes
                .iter()
                .find_map(|shape| match &shape.shape {
                    egui::Shape::Text(text) if text.galley.text() == title => {
                        Some(text.galley.job.sections[0].format.font_id.clone())
                    }
                    _ => None,
                })
                .unwrap_or_else(|| panic!("the picker drew no {title:?}"))
        };

        assert_eq!(font("Analyzer"), font("Colors"));
    }

    /// Every wrapped line is a complete navigation strip, so its selected
    /// page replaces the boundary directly beneath that line.
    #[test]
    fn a_wrapped_picker_keeps_the_active_stroke_on_its_row_boundary() {
        let mut page = DisplayPage::Colors;
        let shapes = crate::tests::probe::painted_full(egui::vec2(120.0, 160.0), |ui| {
            page_picker(ui, &mut page)
        })
        .shapes;
        let lines: Vec<_> = shapes
            .iter()
            .filter_map(|shape| match &shape.shape {
                egui::Shape::LineSegment { points, stroke } => Some((*points, *stroke)),
                _ => None,
            })
            .collect();
        let active = lines
            .iter()
            .find(|(_, stroke)| stroke.color == theme::accent())
            .expect("the active page has no accent underline");
        let boundaries: Vec<_> =
            lines.iter().filter(|(_, stroke)| stroke.color == theme::hairline()).collect();
        let labels: Vec<_> = shapes
            .iter()
            .filter_map(|shape| match &shape.shape {
                egui::Shape::Text(text) => Some((text.galley.text(), text.pos.y)),
                _ => None,
            })
            .collect();
        let selected_label_y = labels
            .iter()
            .find_map(|(title, y)| (*title == "Colors").then_some(*y))
            .expect("the picker drew no Colors label");
        let last_label_y = labels.iter().map(|(_, y)| *y).reduce(f32::max).unwrap();
        assert!(selected_label_y < last_label_y, "the fixture selected a page on the final row");
        let active_y = active.0[0].y;
        assert!(
            boundaries.iter().any(|(points, _)| {
                (points[0].y - active_y).abs() < 0.01
                    && points[0].x <= active.0[0].x
                    && points[1].x >= active.0[1].x
            }),
            "the active stroke is detached from its row boundary",
        );
    }

    /// Hover borrows the dock tabs' highlight without changing weight. A face
    /// change under the pointer makes the letterforms twitch.
    #[test]
    fn hovering_a_page_highlights_it_without_bolding_it() {
        let ctx = crate::tests::probe::themed();
        let size = egui::vec2(400.0, 100.0);
        let rect = egui::Rect::from_min_size(egui::Pos2::ZERO, size);
        let mut page = DisplayPage::Analyzer;
        let mut draw = |events| {
            crate::tests::probe::events_into(&ctx, size, rect, events, |ui| {
                page_picker(ui, &mut page)
            })
        };
        let style = |out: &egui::FullOutput, title: &str| {
            out.shapes
                .iter()
                .find_map(|shape| match &shape.shape {
                    egui::Shape::Text(text) if text.galley.text() == title => {
                        let format = &text.galley.job.sections[0].format;
                        let rect = egui::Rect::from_min_size(text.pos, text.galley.size());
                        Some((format.font_id.clone(), format.color, rect.center()))
                    }
                    _ => None,
                })
                .unwrap_or_else(|| panic!("the picker drew no {title:?}"))
        };

        let resting = draw(vec![]);
        let (resting_font, resting_color, target) = style(&resting, "Colors");
        let hovered = draw(vec![egui::Event::PointerMoved(target)]);
        let (hovered_font, hovered_color, _) = style(&hovered, "Colors");
        assert_eq!(resting_color, theme::text_dim());
        assert_eq!(hovered_color, theme::text());
        assert_eq!(hovered_font, resting_font, "hover changed the label's weight");
        assert!(
            hovered.shapes.iter().any(|shape| matches!(
                &shape.shape,
                egui::Shape::Rect(rect) if rect.fill == theme::surface_faint()
            )),
            "hover drew no tab highlight",
        );
    }
}
