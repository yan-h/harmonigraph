//! The individual panes. Adding a pane = add a `Tab` variant, a title, and
//! a body function in the matching submodule; it immediately participates
//! in docking, and gets the shared state (hover, console, tracker) for free.
//!
//! One file per pane — [`lattice`], [`tuning`], [`view`], [`appearance`],
//! [`spectral`], [`notes`] (Console + Notes). This file holds the `Tab`
//! enum, the `TabViewer` that dispatches to them, and the small helpers
//! more than one pane needs.

use crate::params::{ParamBackend, ParamKey};
use crate::widgets::ValueBar;
use crate::SharedState;

pub mod appearance;
pub mod lattice;
pub mod notes;
/// The Spectral pane's piano roll. Not a pane of its own — it draws into
/// the far share of [`spectral`]'s depth axis, and is only split out
/// because it is the biggest single thing that pane draws.
pub mod roll;
pub mod spectral;
/// The Spectral pane's spectrogram heatmap. Like [`roll`], a layer of
/// [`spectral`]'s far depth region rather than a pane of its own.
pub mod spectrogram;
pub mod tuning;
pub mod view;

use appearance::appearance_pane;
use lattice::lattice_pane;
use notes::{console_pane, notes_pane};
use spectral::{spectral_pane, spectrum_settings_pane};
use tuning::tuning_pane;
use view::view_pane;

/// Wrap degrees into -180..=180 for display (orbit accumulates yaw
/// without bound).
pub(super) fn normalize_deg(deg: f32) -> f32 {
    (deg + 180.0).rem_euclid(360.0) - 180.0
}

/// 12-TET key spellings for MIDI-note readouts (Spectral hover, Notes
/// pane). Octave numbers next to these use Bitwig's convention
/// (middle C = C3).
pub(super) const KEY_NAMES: [&str; 12] = [
    "C", "C\u{266F}", "D", "D\u{266F}", "E", "F",
    "F\u{266F}", "G", "G\u{266F}", "A", "A\u{266F}", "B",
];

#[derive(Clone, Copy, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub enum Tab {
    Lattice,
    Tuning,
    View,
    Appearance,
    Console,
    Spectral,
    /// Settings for the Spectral pane's display and analyzer.
    Spectrum,
    Notes,
}

pub struct Viewer<'a> {
    pub state: &'a mut SharedState,
    pub params: &'a dyn ParamBackend,
    pub now: f64,
}

impl egui_dock::TabViewer for Viewer<'_> {
    type Tab = Tab;

    fn title(&mut self, tab: &mut Tab) -> egui::WidgetText {
        match tab {
            Tab::Lattice => "Lattice".into(),
            Tab::Tuning => "Tuning".into(),
            Tab::View => "View".into(),
            Tab::Appearance => "Appearance".into(),
            Tab::Console => "Console".into(),
            Tab::Spectral => "Spectral".into(),
            Tab::Spectrum => "Spectrum".into(),
            Tab::Notes => "Notes".into(),
        }
    }

    fn ui(&mut self, ui: &mut egui::Ui, tab: &mut Tab) {
        match tab {
            Tab::Lattice => lattice_pane(ui, self.state, self.now),
            Tab::Tuning => tuning_pane(ui, self.state, self.params, self.now),
            Tab::View => view_pane(ui, self.state),
            Tab::Appearance => appearance_pane(ui, self.state, self.params),
            Tab::Console => console_pane(ui, self.state),
            Tab::Spectral => spectral_pane(ui, self.state, self.now),
            Tab::Spectrum => spectrum_settings_pane(ui, self.state),
            Tab::Notes => notes_pane(ui, self.state),
        }
    }

    /// The Spectral display paints its own well-colored plot surface, so
    /// the default 8px body margin reads as a pointless border around it:
    /// drop the margin and let the plot fill the whole tab.
    fn tab_style_override(
        &self,
        tab: &Tab,
        global_style: &egui_dock::TabStyle,
    ) -> Option<egui_dock::TabStyle> {
        matches!(tab, Tab::Spectral).then(|| {
            let mut style = global_style.clone();
            style.tab_body.inner_margin = egui::Margin::ZERO;
            style
        })
    }
}

/// A scene color (linear-ish RGBA in `0..1`, as `lattice_scene` hands
/// them out) as an egui color. Alpha comes from `alpha` rather than the
/// vector's own: scene colors are opaque, and every 2D use of them wants
/// its own transparency.
pub(super) fn scene_color(c: glam::Vec4, alpha: f32) -> egui::Color32 {
    let byte = |v: f32| (v.clamp(0.0, 1.0) * 255.0) as u8;
    egui::Color32::from_rgba_unmultiplied(byte(c.x), byte(c.y), byte(c.z), byte(alpha))
}

/// The dimmest-visible convention shared with the shader (level_floor in
/// lattice.wgsl): quiet elements sit at 35% and scale up to full.
pub(super) fn visibility_floor(level: f32) -> f32 {
    0.35 + 0.65 * level
}

/// A channel in the [`PitchGradient`](lattice_core::ChannelRole::PitchGradient)
/// role, used to borrow the lattice's low-to-high color ramp for the `Pitch`
/// colormap of the roll and spectrogram — the ramp has no entry point of its
/// own that takes the display's darkest/brightest bounds. Shared so the two
/// layers color their Pitch mode identically.
pub(super) const PITCH_RAMP_CHANNEL: u8 = 9;

/// A lattice node's note name for display, honoring meantone mode: meantone
/// tempers out the syntonic comma, so the comma marks are dropped (E- and E
/// name the same pitch). Shared so every pane that labels a node agrees.
pub(super) fn display_note_name(
    pos: lattice_core::LatticePos,
    meantone: bool,
) -> lattice_core::NoteName {
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
    view: &lattice_scene::ViewConfig,
    tuning: &lattice_core::Tuning,
    pc: lattice_core::PitchClass,
) -> Option<lattice_core::LatticePos> {
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

/// A section header inside a settings pane: a little breathing room, a thin
/// rule, then the group's name in the heading (bold) face — so each block
/// of related controls is easy to pick out at a glance.
pub(super) fn section(ui: &mut egui::Ui, title: &str) {
    ui.add_space(4.0);
    ui.separator();
    ui.heading(title);
}
