//! The Display tab: how everything on screen is drawn, one collapsible
//! section per subject — [`color`](super::color) and light, the lattice's
//! [`view`](super::view), [`nodes`](super::nodes), [`labels`](super::labels)
//! and [`grid`](super::grid), then the Analyzer's own settings
//! ([`spectral`](super::spectral)). One tab rather than one per subject,
//! because a settings tab is paid for in bar width at the editor's DEFAULT
//! window: per-pane tabs overflow it, and egui_dock answers overflow by
//! scrolling the bar, so every tab stays clickable and the one past the edge
//! is one a new user never learns exists (#287).
//!
//! **A setting lives in the section named for the widest thing it affects, and
//! the sections run widest scope to narrowest.** The colors and the light every
//! picture is painted with, then which of the lattice is framed, then a played
//! note's own layers, the text riding them, the lines between them, and last
//! the one pane with settings of its own. That makes a placement question
//! "what does this touch?" — answerable out of the render code — rather than
//! "what is this about?", which is argued afresh in every docstring and lands
//! the pitch gradient under Nodes while the Analyzer's ribbons read it too.
//! It is the rule the tree already follows between tabs (Tuning is what the
//! lattice IS, Display how it is drawn, System the machine around it) and
//! inside the Nodes section, where the whole-note group leads and the layers
//! follow; this is it applied to the section structure itself.
//!
//! The sections are FLAT — no super-section grouping the lattice's four —
//! because two-deep collapsing is clunky to work, and flat is what keeps the
//! bar's width independent of what is drawn: a future visualization adds one
//! section here and no tab, where a tab-per-picture bar overflows again a few
//! pictures in.
//!
//! Each header wears its pane's audited name (#286), "Analyzer" deliberately
//! shared with the display pane's title — see [`tab_title`](super::tab_title).
//! "Color & light" over "Style", which names nothing, and over "Color", which
//! Bloom is not one of.

use super::color::color_pane;
use super::grid::grid_pane;
use super::labels::labels_pane;
use super::nodes::nodes_pane;
use super::spectral::spectrum_settings_pane;
use super::view::view_pane;
use crate::params::ParamBackend;
use crate::SharedState;

/// Display's sections, in the order the pane stacks them: widest scope first —
/// what everything is painted with, then the frame around the lattice, a
/// played note's layers, the text on them, the lines between them, and the
/// analyzer last.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Section {
    Color,
    View,
    Nodes,
    Labels,
    Grid,
    Analyzer,
}

impl Section {
    /// Every section, in the stacking order named on the enum's own doc.
    ///
    /// Built from an exhaustive `match` rather than written out as a bare
    /// literal, so the list cannot fall behind the enum — the same guard
    /// `SpectralOrientation::ALL` in this crate uses, for the same reason.
    pub const ALL: [Section; 6] = {
        use Section::*;
        // Exhaustive, and the compiler checks it. The arm is `()` because
        // what is wanted is the coverage error, not the value.
        const fn covered(section: Section) {
            match section {
                Color | View | Nodes | Labels | Grid | Analyzer => (),
            }
        }
        covered(Color);
        [Color, View, Nodes, Labels, Grid, Analyzer]
    };

    /// The section's header text, and the name of the settings pane it holds.
    pub fn title(self) -> &'static str {
        match self {
            Section::Color => "Color & light",
            Section::View => "View",
            Section::Nodes => "Nodes",
            Section::Labels => "Labels",
            Section::Grid => "Grid",
            Section::Analyzer => "Analyzer",
        }
    }
}

/// Which of Display's sections stand open.
///
/// Lives in [`SharedState`] and persists through `UiPersist`, NOT in egui
/// `Context` memory, and the home is load-bearing: the plugin builds a brand
/// new `Context` every time the editor window opens (the trap
/// [`SharedState::release_context_resources`] sets out), so collapse state
/// kept in memory springs shut with every reopen. The pane forces each header
/// to its flag every frame and writes clicks straight back here, which leaves
/// memory only ever following this struct — see [`fold_out`].
///
/// Every section defaults COLLAPSED, so a fresh pane is six headers — a table
/// of contents — and the derived `Default` is the whole fallback for a blob
/// missing the key (`UiPersist`'s container-level `#[serde(default)]`
/// convention).
#[derive(Clone, Copy, Default, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(default)]
pub struct DisplaySections {
    pub color: bool,
    pub view: bool,
    pub nodes: bool,
    pub labels: bool,
    pub grid: bool,
    pub analyzer: bool,
}

impl DisplaySections {
    /// The flag that holds `section` open.
    pub fn open_mut(&mut self, section: Section) -> &mut bool {
        match section {
            Section::Color => &mut self.color,
            Section::View => &mut self.view,
            Section::Nodes => &mut self.nodes,
            Section::Labels => &mut self.labels,
            Section::Grid => &mut self.grid,
            Section::Analyzer => &mut self.analyzer,
        }
    }
}

/// The six settings panes the tab carries, each behind its fold-out header.
///
/// Every body draws straight into the tab and scrolls in the dock's own
/// `ScrollArea` (see `Viewer::scroll_bars`), rather than building a second one
/// per section. Both scroll the same list to the same wheel; what tells them
/// apart is where the bar goes. The dock's area is the pane, so its bar falls
/// in the pane's right margin — one built here starts at the content box, and
/// a floating bar draws over the content it has no room beside, which is the
/// right end of every bar in every section.
pub(super) fn display_pane(ui: &mut egui::Ui, state: &mut SharedState, params: &dyn ParamBackend) {
    // Copied out and written back, so a header can hold its flag while the
    // section body borrows the whole state.
    let mut sections = state.display_sections;
    for section in Section::ALL {
        fold_out(ui, sections.open_mut(section), section.title(), |ui| match section {
            Section::Color => color_pane(ui, state, params),
            Section::View => view_pane(ui, state),
            Section::Nodes => nodes_pane(ui, state, params),
            Section::Labels => labels_pane(ui, state),
            Section::Grid => grid_pane(ui, state),
            Section::Analyzer => spectrum_settings_pane(ui, state),
        });
    }
    state.display_sections = sections;
}

/// One collapsible section: a heading-face header that toggles `open`, and the
/// body under it while the flag holds it open.
///
/// The header is FORCED to `open` every frame (`.open(Some(..))`), which is
/// the mechanism that keeps [`DisplaySections`] the single source of truth:
/// with `open` supplied, egui's own click handling is bypassed and its memory
/// only follows the flag, so the click has to land here — in state that
/// persists — or nowhere.
///
/// The body is unindented, so a section's bars take the same column every
/// settings pane measures against (`widgets::bar_width`) rather than one an
/// indent has narrowed — the sweeps in `tests::settings` hold each section to
/// the pane's own width.
fn fold_out(ui: &mut egui::Ui, open: &mut bool, title: &str, body: impl FnOnce(&mut egui::Ui)) {
    // The rule between sections, as [`super::section`] draws it, decided by
    // the same cursor question — so the first header leads the pane bare.
    if ui.cursor().top() > ui.max_rect().top() + 0.5 {
        ui.add_space(4.0);
        ui.separator();
    }
    let header = egui::CollapsingHeader::new(egui::RichText::new(title).heading())
        .open(Some(*open))
        .show_unindented(ui, body);
    if header.header_response.clicked() {
        *open = !*open;
    }
}
