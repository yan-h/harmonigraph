//! The Display tab: how everything on screen is drawn, one collapsible
//! section per subject — the lattice's [`view`](super::view),
//! [`nodes`](super::nodes) and [`scene`](super::scene), then the Analyzer's
//! own settings ([`spectral`](super::spectral)). One tab rather than one per
//! subject, because a settings tab is paid for in bar width at the editor's
//! DEFAULT window: per-pane tabs overflow it, and egui_dock answers overflow
//! by scrolling the bar, so every tab stays clickable and the one past the
//! edge is one a new user never learns exists (#287).
//!
//! The sections are FLAT — no super-section grouping the lattice's three —
//! because two-deep collapsing is clunky to work, and flat is what keeps the
//! bar's width independent of what is drawn: a future visualization adds one
//! section here and no tab, where a tab-per-picture bar overflows again a few
//! pictures in.
//!
//! Each header keeps the name the tab it stands for wore (#286's audited
//! names), "Analyzer" still deliberately shared with the display pane's title
//! — see [`tab_title`](super::tab_title).

use super::nodes::nodes_pane;
use super::scene::scene_pane;
use super::spectral::spectrum_settings_pane;
use super::view::view_pane;
use crate::params::ParamBackend;
use crate::SharedState;

/// Display's sections, in the order the pane stacks them: the lattice read
/// outward from the picture — which of it you see, how a note is drawn, the
/// scene around the notes — then the analyzer.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Section {
    View,
    Nodes,
    Scene,
    Analyzer,
}

impl Section {
    pub const ALL: [Section; 4] =
        [Section::View, Section::Nodes, Section::Scene, Section::Analyzer];

    /// The section's header text, and the name of the settings pane it holds.
    pub fn title(self) -> &'static str {
        match self {
            Section::View => "View",
            Section::Nodes => "Nodes",
            Section::Scene => "Scene",
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
/// Every section defaults COLLAPSED, so a fresh pane is four headers — a
/// table of contents — and the derived `Default` is the whole fallback for a
/// blob missing the key (`UiPersist`'s container-level `#[serde(default)]`
/// convention).
#[derive(Clone, Copy, Default, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(default)]
pub struct DisplaySections {
    pub view: bool,
    pub nodes: bool,
    pub scene: bool,
    pub analyzer: bool,
}

impl DisplaySections {
    /// The flag that holds `section` open.
    pub fn open_mut(&mut self, section: Section) -> &mut bool {
        match section {
            Section::View => &mut self.view,
            Section::Nodes => &mut self.nodes,
            Section::Scene => &mut self.scene,
            Section::Analyzer => &mut self.analyzer,
        }
    }
}

/// The four settings panes the tab carries, each behind its fold-out header.
pub(super) fn display_pane(ui: &mut egui::Ui, state: &mut SharedState, params: &dyn ParamBackend) {
    // Copied out and written back, so a header can hold its flag while the
    // section body borrows the whole state.
    let mut sections = state.display_sections;
    for section in Section::ALL {
        fold_out(ui, sections.open_mut(section), section.title(), |ui| match section {
            Section::View => view_pane(ui, state),
            Section::Nodes => nodes_pane(ui, state, params),
            Section::Scene => scene_pane(ui, state),
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
