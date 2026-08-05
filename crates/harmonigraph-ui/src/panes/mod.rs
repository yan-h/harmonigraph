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
pub mod nodes;
pub mod notes;
pub mod panel;
/// The offline video frame, composed live so you can preview and adjust it
/// before rendering. The "Video" tab.
pub mod render;
pub mod scene;
pub mod spectral;
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

/// The variant names are a persistence contract: a saved layout names its
/// tabs by these spellings, so renaming one orphans the dock in every project
/// that has it open. Retiring a tab is the same problem from the other side —
/// an unknown variant fails the whole `UiPersist` parse and takes the
/// dialed-in camera, view and analyzer settings down with it, not just the
/// arrangement. Either change needs a `UI_PERSIST_VERSION` bump behind it
/// (see [`load_persist`](crate::SharedState::load_persist)).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, serde::Serialize, serde::Deserialize)]
pub enum Tab {
    Lattice,
    /// The lattice itself: how it is tuned, and how it is framed. The framing
    /// half was a tab of its own until the two short panes were merged.
    Tuning,
    /// How a sounding note is drawn.
    Nodes,
    /// The structure and overlays around the notes.
    Scene,
    Console,
    /// The Spectral display: FFT curve, voices, and piano roll.
    Spectral,
    /// Settings for the Spectral display. Titled "Analyzer".
    Analyzer,
    Notes,
    /// A live preview of the offline video frame, composed and adjusted here.
    /// Titled "Video".
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
/// [`Viewer`]'s `ui` before the body draws anything and read by
/// `widgets::bar_width`.
///
/// A bar fills the pane, so it has to ask how wide the pane is, and neither
/// obvious answer survives contact with the dock. `max_rect` is no good by the
/// time a bar asks: egui's `Region::expand_to_include_rect` unions it when a
/// control overruns, so it may already have grown past the pane. Nor is the
/// clip rect, which is the tab BODY — egui_dock clips to the whole body and
/// only then insets it by [`crate::theme::pane_inner_margin`] via a `Frame`,
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


/// A lattice node's note name for display, spelled against whatever commas
/// are being tempered out: a tempered comma makes two positions one pitch, so
/// the name is the one its collapsed position carries (meantone names E- and
/// E alike; marvel names the harmonic seventh as the augmented sixth it has
/// become). Shared so every pane that labels a node agrees.
pub(super) fn display_note_name(
    pos: harmonigraph_core::LatticePos,
    tempered: harmonigraph_core::Tempered,
) -> harmonigraph_core::NoteName {
    pos.respell(tempered).note_name()
}

/// The visible lattice node whose pitch class most closely matches `pc`
/// under the current tuning (several can match when the tolerance is wide).
///
/// The question is "is this PLAYED pitch on the lattice, and where", and
/// every pane that asks it uses this, so they can't disagree: the Notes
/// pane's node column, and the analyzer's red band for a voice with no node
/// to light. `Tuning::tolerance` is load-bearing in both — a note off every
/// node is a note the lattice cannot show, and saying so is the point.
///
/// One neighbour is close enough to be reached for by mistake and does not
/// want the tolerance: [`names`](crate::panes::spectral::names)'s `naming_node`
/// takes the same played
/// pitch but asks what to CALL it, where a collapsed equal temperament makes
/// the choice AMONG matches the whole problem rather than an afterthought.
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
    // The rule separates a section from the one above it, so the FIRST section
    // in a pane has nothing to separate from and takes a bare heading. The
    // Nodes and Scene panes write that case out by hand — they are built from
    // section functions in a fixed order, so each knows whether it leads.
    //
    // A pane whose first section depends on the shell cannot know: the Video
    // pane leads with Record under a host and with Frame in the standalone,
    // which has no transport to record. The CURSOR is what answers it without
    // asking the caller — it starts at the top of the ui and only moves down
    // once something is laid out, so still being there means this heading is
    // the pane's first.
    //
    // `min_rect` is the tempting reading of "has anything been drawn" and it
    // is the wrong one here: egui_dock wraps every pane body in a `ScrollArea`
    // whose ui arrives with `min_rect` already equal to `max_rect`, so it is a
    // full-height rect before the pane draws a thing. A fixture that builds
    // the pane ui directly sees an empty `min_rect` instead and cannot tell
    // the two apart — which is why `the_video_pane_does_not_start_with_a_rule`
    // goes through the real dock.
    if ui.cursor().top() > ui.max_rect().top() + 0.5 {
        ui.add_space(4.0);
        ui.separator();
    }
    ui.heading(title);
}
