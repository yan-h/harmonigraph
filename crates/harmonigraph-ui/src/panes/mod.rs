//! The individual panes. Adding a pane = add a `Tab` variant, a title, and
//! a body function in the matching submodule; it immediately participates
//! in docking, and gets the shared state (hover, console, tracker) for free.
//!
//! The lattice's settings read outward from the picture: [`tuning`] is how it
//! is tuned and — via [`frame`], which draws the second half of that same tab
//! — how it is framed; [`nodes`] is how a played note is drawn, [`scene`] is
//! everything around the notes, and [`panel`] is the plugin's own render/
//! layout knobs. Alongside are the [`spectral`] display and its analyzer
//! settings, [`render`] (the Video tab), and [`notes`] (Console + Notes).
//! This file holds the `Tab` enum, the `TabViewer` that dispatches to them,
//! and the small helpers more than one pane needs.

use crate::params::{ParamBackend, ParamKey};
use crate::widgets::{RangeBar, ValueBar};
use crate::SharedState;

pub mod frame;
pub mod lattice;
/// The names written over the Spectral pane's note ribbons. Like [`roll`] and
/// [`spectrogram`], a layer of [`spectral`]'s far depth region rather than a
/// pane of its own.
pub mod names;
pub mod nodes;
pub mod notes;
pub mod panel;
/// The offline video frame, composed live so you can preview and adjust it
/// before rendering. The "Video" tab.
pub mod render;
/// The Spectral pane's piano roll. Not a pane of its own — it draws into
/// the far share of [`spectral`]'s depth axis, and is only split out
/// because it is the biggest single thing that pane draws.
pub mod roll;
pub mod scene;
pub mod spectral;
/// The Spectral pane's spectrogram heatmap. Like [`roll`], a layer of
/// [`spectral`]'s far depth region rather than a pane of its own.
pub mod spectrogram;
pub mod tuning;

use lattice::lattice_pane;
use nodes::nodes_pane;
use notes::{console_pane, notes_pane};
use panel::panel_pane;
use render::render_pane;
use scene::scene_pane;
use spectral::{spectral_pane, spectrum_settings_pane};
use tuning::tuning_pane;

/// Wrap degrees into -180..=180 for display (orbit accumulates yaw
/// without bound).
pub(super) fn normalize_deg(deg: f32) -> f32 {
    (deg + 180.0).rem_euclid(360.0) - 180.0
}

/// 12-TET key spellings for MIDI-note readouts (the Notes pane's rows, the
/// color range's ends in `nodes`). Octave numbers next to these use
/// Bitwig's convention (middle C = C3).
pub(super) const KEY_NAMES: [&str; 12] = [
    "C", "C\u{266F}", "D", "D\u{266F}", "E", "F",
    "F\u{266F}", "G", "G\u{266F}", "A", "A\u{266F}", "B",
];

/// The serde `alias`es carry pre-reorg persisted layouts across the rename:
/// an old blob names `View`/`Appearance`/`Spectrum`/`Render`, and without the
/// aliases that unknown variant would fail the whole `UiPersist` parse and
/// take the dialed-in camera/view/spectrum/render settings down with it. The
/// dock arrangement itself is refreshed separately (see `UiPersist` version).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, serde::Serialize, serde::Deserialize)]
pub enum Tab {
    Lattice,
    /// The lattice itself: how it is tuned, and how it is framed. The framing
    /// half was a tab of its own — "Frame", and "View" before that — until the
    /// two short panes were merged; both spellings alias here so an older
    /// layout still parses (the dock arrangement itself is refreshed by the
    /// `UiPersist` version bump that came with the merge).
    #[serde(alias = "Frame", alias = "View")]
    Tuning,
    /// How a sounding note is drawn (was the first half of "Appearance").
    #[serde(alias = "Appearance")]
    Nodes,
    /// The structure and overlays around the notes (second half of the old
    /// "Appearance").
    Scene,
    Console,
    /// The Spectral display: FFT curve, voices, and piano roll.
    Spectral,
    /// Settings for the Spectral display. Titled "Analyzer"; was "Spectrum".
    #[serde(alias = "Spectrum")]
    Analyzer,
    Notes,
    /// A live preview of the offline video frame, composed and adjusted here.
    /// Titled "Video"; was "Render".
    #[serde(alias = "Render")]
    Video,
    /// The plugin's own render-quality and pane-layout knobs.
    Panel,
}

pub struct Viewer<'a> {
    pub state: &'a mut SharedState,
    pub params: &'a dyn ParamBackend,
    pub now: f64,
}

/// Where the settings pane being drawn ends on the right, written by
/// [`Viewer::ui`] before the body draws anything and read by
/// [`crate::widgets::bar_width`].
///
/// A bar fills the pane, so it has to ask how wide the pane is, and neither
/// obvious answer survives contact with the dock. `max_rect` is no good by the
/// time a bar asks: egui's `Region::expand_to_include_rect` unions it when a
/// control overruns, so it may already have grown past the pane. Nor is the
/// clip rect, which is the tab BODY — egui_dock clips to the whole body and
/// only then insets it by [`crate::theme::PANE_INNER_MARGIN`] via a `Frame`,
/// which does not clip — so a bar clamped to it comes out a margin longer than
/// its neighbours and sits flush on the pane border.
///
/// What is wanted is `max_rect` as it stood on the way in, before anything in
/// the pane could widen it, which is a thing only the caller can know. Hence
/// the hand-off. It is written every frame, immediately before the body, so no
/// bar can read a value from another pane or another size.
pub(crate) fn pane_content_right() -> egui::Id {
    egui::Id::new("pane-content-right")
}

/// What a tab is called, wherever its name is drawn: its own tab, and the rail
/// a folded pane leaves behind (see [`crate::fold`]).
pub fn tab_title(tab: &Tab) -> &'static str {
    match tab {
        Tab::Lattice => "Lattice",
        Tab::Tuning => "Tuning",
        Tab::Nodes => "Nodes",
        Tab::Scene => "Scene",
        Tab::Console => "Console",
        // Deliberately the same name as the settings tab below: the display
        // and the settings for it are one feature, and they sit in different
        // docks, so the pair reads as "the analyzer, and its knobs" rather
        // than as two things to tell apart.
        Tab::Spectral => "Analyzer",
        Tab::Analyzer => "Analyzer",
        Tab::Notes => "Notes",
        Tab::Video => "Video",
        Tab::Panel => "Panel",
    }
}

impl egui_dock::TabViewer for Viewer<'_> {
    type Tab = Tab;

    fn title(&mut self, tab: &mut Tab) -> egui::WidgetText {
        tab_title(tab).into()
    }

    /// Identify a tab by its VARIANT, never by its title.
    ///
    /// egui_dock's default is `Id::new(title)`, and this dock deliberately has
    /// two tabs titled "Analyzer" — the display and the settings for it. That
    /// made them one id, and the id keys the tab BODY's `Ui`
    /// (`tab_body_id` mixes in the surface but not the node), so the two
    /// bodies shared their state: egui_dock wraps every body in a
    /// `ScrollArea`, and scrolling the settings scrolled the display instead.
    fn id(&mut self, tab: &mut Tab) -> egui::Id {
        egui::Id::new(("lattice-pane", *tab))
    }

    /// The picture panes never scroll. Both fill their body exactly — the
    /// lattice with a wgpu callback, the analyzer with a painter over the
    /// whole rect — so there is nothing under the edge to reach, and a scroll
    /// area around them can only shift a picture that is meant to sit still.
    /// Settings panes keep the VERTICAL bar only: they are lists, and a short
    /// dock column has to be able to reach the end of one.
    ///
    /// Horizontal scrolling is off everywhere. egui_dock wraps the body in
    /// `ScrollArea::new(self.scroll_bars(tab))`, and a both-axes area offers
    /// its content an unbounded width; the settings panes that size to the
    /// space they're given (`available_size`) then fill it and never report
    /// vertical overflow, so the wheel had nothing to grab and they wouldn't
    /// scroll at all. Vertical-only matches the panes that build their own
    /// `ScrollArea::vertical()` — the ones that always scrolled fine.
    fn scroll_bars(&self, tab: &Tab) -> [bool; 2] {
        let picture = matches!(tab, Tab::Lattice | Tab::Spectral);
        [false, !picture]
    }

    fn ui(&mut self, ui: &mut egui::Ui, tab: &mut Tab) {
        // Before the body draws anything — see [`pane_content_right`].
        let right = ui.max_rect().right();
        ui.data_mut(|d| d.insert_temp(pane_content_right(), right));
        match tab {
            Tab::Lattice => lattice_pane(ui, self.state, self.now),
            Tab::Tuning => tuning_pane(ui, self.state, self.params, self.now),
            Tab::Nodes => nodes_pane(ui, self.state, self.params),
            Tab::Scene => scene_pane(ui, self.state),
            Tab::Console => console_pane(ui, self.state),
            Tab::Spectral => spectral_pane(ui, self.state, self.now, 0),
            Tab::Analyzer => spectrum_settings_pane(ui, self.state),
            Tab::Notes => notes_pane(ui, self.state),
            Tab::Video => render_pane(ui, self.state, self.now),
            Tab::Panel => panel_pane(ui, self.state),
        }
    }

    /// The two picture panes paint their own surface edge to edge — the
    /// Spectral display its plot well, the Lattice its 3D view — so the
    /// default 8px body margin reads as a pointless border around a picture
    /// rather than as breathing room between controls. Drop it and let them
    /// fill the whole tab.
    ///
    /// Settings panes keep the margin: there the padding is what stops the
    /// bars and labels from running into the pane edge.
    fn tab_style_override(
        &self,
        tab: &Tab,
        global_style: &egui_dock::TabStyle,
    ) -> Option<egui_dock::TabStyle> {
        matches!(tab, Tab::Spectral | Tab::Lattice).then(|| {
            let mut style = global_style.clone();
            style.tab_body.inner_margin = egui::Margin::ZERO;
            style
        })
    }
}

/// A scene color (linear-ish RGBA in `0..1`, as `harmonigraph_scene` hands
/// them out) as an egui color. Alpha comes from `alpha` rather than the
/// vector's own: scene colors are opaque, and every 2D use of them wants
/// its own transparency.
pub(super) fn scene_color(c: glam::Vec4, alpha: f32) -> egui::Color32 {
    let byte = |v: f32| (v.clamp(0.0, 1.0) * 255.0) as u8;
    egui::Color32::from_rgba_unmultiplied(byte(c.x), byte(c.y), byte(c.z), byte(alpha))
}


/// A lattice node's note name for display, honoring meantone mode: meantone
/// tempers out the syntonic comma, so the comma marks are dropped (E- and E
/// name the same pitch). Shared so every pane that labels a node agrees.
pub(super) fn display_note_name(
    pos: harmonigraph_core::LatticePos,
    meantone: bool,
) -> harmonigraph_core::NoteName {
    let name = pos.note_name();
    if meantone {
        name.without_syntonic_commas()
    } else {
        name
    }
}

/// The visible lattice node whose pitch class most closely matches `pc`
/// under the current tuning (several can match when the tolerance is
/// wide). Every pane that answers "which node is this pitch class" uses
/// this, so they can't disagree.
pub(super) fn nearest_visible_node(
    view: &harmonigraph_scene::ViewConfig,
    tuning: &harmonigraph_core::Tuning,
    pc: harmonigraph_core::PitchClass,
) -> Option<harmonigraph_core::LatticePos> {
    view.visible_positions()
        .filter(|&pos| tuning.matches(pc, tuning.pitch_class(pos)))
        .min_by_key(|&pos| pc.distance_to(tuning.pitch_class(pos)))
}

/// Attention pulse for armed-mode indicators: a slow, shallow breathe
/// (calm "armed", not "alarmed").
pub(super) fn learn_pulse(now: f64) -> f32 {
    0.78 + 0.22 * (now * 2.0 * std::f64::consts::PI * 0.6).sin() as f32
}

/// One editable ValueBar for a parameter, with automation-gesture
/// bracketing so a drag records as a single host gesture.
pub(super) fn param_bar(
    ui: &mut egui::Ui,
    params: &dyn ParamBackend,
    key: ParamKey,
) -> egui::Response {
    let mut value = params.get(key);
    let response = ValueBar::new(&mut value, key.range(), key.label())
        .eased(key.logarithmic())
        .decimals(2)
        .show(ui);
    // Bracket drags so the host records one automation gesture per drag;
    // one-shot changes (typed values) go through set() alone.
    if response.drag_started() {
        params.begin_set(key);
    }
    if response.changed() {
        params.set(key, value);
    }
    if response.drag_stopped() {
        params.end_set(key);
    }
    response
}

/// A two-handle [`RangeBar`] over a PAIR of parameters — one control for a
/// range whose ends are both automatable params (the Nodes pane's color
/// range). `display` formats each end's readout.
///
/// Both params are bracketed for the whole drag and written every changed
/// frame, so a drag on either handle records as one gesture on each. A
/// double-click reset arrives as `changed` with no drag, and goes through the
/// same `set` without a gesture — matching [`param_bar`]'s one-shot path.
pub(super) fn param_range_bar(
    ui: &mut egui::Ui,
    params: &dyn ParamBackend,
    low_key: ParamKey,
    high_key: ParamKey,
    range: std::ops::RangeInclusive<f32>,
    min_span: f32,
    display: fn(f32) -> String,
) -> egui::Response {
    let (mut low, mut high) = (params.get(low_key), params.get(high_key));
    let response = RangeBar::new(&mut low, &mut high, range)
        .min_span(min_span)
        .display(display)
        .show(ui);
    if response.drag_started() {
        params.begin_set(low_key);
        params.begin_set(high_key);
    }
    if response.changed() {
        params.set(low_key, low);
        params.set(high_key, high);
    }
    if response.drag_stopped() {
        params.end_set(low_key);
        params.end_set(high_key);
    }
    response
}

/// A section header inside a settings pane: a little breathing room, a thin
/// rule, then the group's name in the heading (bold) face — so each block
/// of related controls is easy to pick out at a glance.
pub(super) fn section(ui: &mut egui::Ui, title: &str) {
    ui.add_space(4.0);
    ui.separator();
    ui.heading(title);
}
