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
use super::plus::plus_pane;
use super::labels::labels_pane;
use super::nodes::nodes_pane;
use super::spectral::spectrum_settings_pane;
use super::system::system_pane;
use super::view::view_pane;
use crate::params::ParamBackend;
use crate::widgets::button_row;
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

    /// The page's name, on its picker button and nowhere else.
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

/// The row of page names at the top of the tab, and the whole of its
/// navigation.
///
/// No label in front of it, unlike the choice rows inside the pages: one of
/// those names a setting and then offers its values, where this row IS what the
/// tab holds — a word in front of it would read as a setting called Page.
///
/// A [`button_row`], so a narrow column wraps the names onto a second line
/// instead of running the last of them off the edge. A page is reached by
/// clicking its name and by nothing else, so a name past the pane edge is a
/// page with no way into it: horizontal scrolling is off in the dock (see
/// [`Viewer::scroll_bars`](super::Viewer)).
fn page_picker(ui: &mut egui::Ui, page: &mut DisplayPage) {
    button_row(ui, |ui| {
        for choice in DisplayPage::ALL {
            ui.selectable_value(page, choice, choice.title());
        }
    });
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
