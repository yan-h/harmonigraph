//! The individual panes. Adding a pane = add a `Tab` variant, a title, and
//! a body function in the matching submodule; it immediately participates
//! in docking, and gets the shared state (hover, console, tracker) for free.
//!
//! [`tuning`] is how the lattice is tuned, and [`display`] is how everything
//! is drawn, one collapsible section each and widest scope first (the rule is
//! written out in [`display`]): [`color`] (the table every pitch-colored shape
//! is painted through, and the bloom over it), [`view`] (which of the lattice
//! you are looking at and from where), [`nodes`] (a played note's own layers),
//! [`labels`] (the text on them), [`grid`] (the lines between them) and the
//! analyzer settings. [`system`] is the plugin's own render/layout knobs.
//! Alongside are the [`spectral`] display, the [`spiral`] one beside it (the
//! same analyzer wound onto a chroma circle), [`render`] (the Video tab), and
//! [`notes`] (Console + Notes). This file holds the `Tab` enum, the
//! `TabViewer` that dispatches to them, and the small helpers more than one
//! pane needs.
//!
//! Each tab's name covers everything in it, which is the whole job a tab bar
//! does: a subject its name omits is one nobody opens the tab to find.

use crate::params::{ParamBackend, ParamKey};
use crate::widgets::{RangeBar, ValueBar};
use crate::SharedState;

pub mod color;
pub mod display;
pub mod grid;
pub mod labels;
pub mod lattice;
pub mod nodes;
pub mod notes;
/// The offline video frame, composed live so you can preview and adjust it
/// before rendering. The "Video" tab.
pub mod render;
pub mod spectral;
// Not a pane — a post-pass the Lattice pane runs — but it is made of two of
// them, and its own docs say so.
pub mod spectral_fold;
pub mod spiral;
pub mod system;
pub mod tuning;
pub mod view;

use display::display_pane;
use lattice::lattice_pane;
use notes::{console_pane, notes_pane};
use render::render_pane;
use spectral::spectral_pane;
use spiral::spiral_pane;
use system::system_pane;
use tuning::tuning_pane;

/// Wrap degrees into -180..=180 for display (orbit accumulates yaw
/// without bound).
pub(super) fn normalize_deg(deg: f32) -> f32 {
    (deg + 180.0).rem_euclid(360.0) - 180.0
}

/// 12-TET key spellings for MIDI-note readouts (the Notes pane's rows, the
/// color range's ends in [`color`]). Octave numbers next to these use
/// Bitwig's convention (middle C = C3).
pub(super) const KEY_NAMES: [&str; 12] = [
    "C", "C\u{266F}", "D", "D\u{266F}", "E", "F",
    "F\u{266F}", "G", "G\u{266F}", "A", "A\u{266F}", "B",
];

/// A MIDI note as a key name and octave — "C1", "C8" — so a range's ends read
/// as pitches rather than bare numbers. Shared by the Nodes section's octave
/// Center and Color & light's color range, which is why it is here rather than
/// in either.
///
/// It ROUNDS, which is exact for the octave Center (its bar lands on whole
/// semitones) and a reading for the color range (whose ends are a continuous
/// gradient, where a tenth of a semitone changes nothing anyone can see). A
/// caller wanting finer steps than a semitone needs its own readout, not a
/// looser one here: this one would then name two visibly different settings
/// the same note.
pub(super) fn pitch_readout(midi: f32) -> String {
    let n = midi.round() as i32;
    let name = KEY_NAMES[n.rem_euclid(12) as usize];
    format!("{name}{}", harmonigraph_core::notes::display_octave_of(n))
}

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
    /// Where the lattice's nodes sit in pitch: the prime bars, and the commas
    /// it tempers out.
    Tuning,
    /// How everything on screen is drawn: the Color & light, View, Nodes,
    /// Labels, Grid and Analyzer settings, one collapsible section each — see
    /// [`display`] for why one tab carries all six, and for the rule that says
    /// which section a setting lands in.
    Display,
    Console,
    /// The Spectral display: FFT curve, voices, and piano roll. Titled
    /// "Analyzer".
    Spectral,
    /// The same analyzer frame wound onto a chroma spiral, one turn per
    /// octave, with the sounding notes marked on it. Titled "Spiral".
    Spiral,
    Notes,
    /// A live preview of the offline video frame, composed and adjusted here.
    /// Titled "Video".
    Video,
    /// The plugin's own render-quality and pane-layout knobs. Titled "System"
    /// rather than "Panel", which would name the thing being looked at rather
    /// than anything the tab changes — and sits one letter from "pane", which
    /// is what every tab in this dock is.
    System,
}

impl Tab {
    /// Whether this tab is a PICTURE — a pane that paints its own surface edge
    /// to edge — rather than a list of controls.
    ///
    /// A method rather than a `matches!` at each site, because more than one
    /// site asks (the scroll bars and the body margin below), and a new picture
    /// pane that slips past one of them draws with a scroll area around it or a
    /// border of chrome inside it, neither of which fails anything: it just
    /// looks wrong, in a way nobody thinks to attribute to a missing arm.
    pub(crate) fn is_picture(&self) -> bool {
        matches!(self, Tab::Lattice | Tab::Spectral | Tab::Spiral)
    }
}

pub struct Viewer<'a> {
    pub state: &'a mut SharedState,
    pub params: &'a dyn ParamBackend,
    pub now: f64,
}

/// Where the settings pane being drawn ends on the right, written by
/// [`Viewer`]'s `ui` before the body draws anything and read by
/// `widgets::bar::bar_width`.
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
        Tab::Display => "Display",
        Tab::Console => "Console",
        // Deliberately the same name as the Display tab's Analyzer section:
        // the display and the settings for it are one feature, and they sit
        // on different surfaces, so the pair reads as "the analyzer, and its
        // knobs" rather than as two things to tell apart.
        Tab::Spectral => "Analyzer",
        Tab::Spiral => "Spiral",
        Tab::Notes => "Notes",
        Tab::Video => "Video",
        Tab::System => "System",
    }
}

impl egui_dock::TabViewer for Viewer<'_> {
    type Tab = Tab;

    fn title(&mut self, tab: &mut Tab) -> egui::WidgetText {
        tab_title(tab).into()
    }

    /// No pane leaves the editor for a window of its own.
    ///
    /// egui_dock offers it as "Eject" in a tab's right-click menu, and taking it
    /// away is what lets the fold pass be one tree rather than a loop over
    /// surfaces (see [`crate::fold`]). A dock window is laid out in a window
    /// whose size is not the editor's to know or to ask for, so everything the
    /// fold does with the plugin window — price a resize, hold a flag until it
    /// arrives, tell a refusal from a floor — has no answer out there, and the
    /// pass carried a second set of them for a surface that could only re-fit to
    /// whatever it found itself in.
    ///
    /// A layout saved with a window in it still opens: the dock draws it as it
    /// always did, and folding sideways there is simply the no-op it always
    /// effectively was. Dragging the tab back into the editor is unaffected.
    fn allowed_in_windows(&self, _tab: &mut Tab) -> bool {
        false
    }

    /// Identify a tab by its VARIANT, never by its title.
    ///
    /// egui_dock's default is `Id::new(title)`, and the id keys the tab
    /// BODY's `Ui` (`tab_body_id` mixes in the surface but not the node) — so
    /// under the default, two tabs given one title share their body state:
    /// egui_dock wraps every body in a `ScrollArea`, and scrolling one pane
    /// scrolls the other. Keying on the variant is what leaves titles free to
    /// repeat a name, which the dock still trades on — the Spectral pane
    /// wears "Analyzer", the same word as the Display section holding its
    /// settings.
    fn id(&mut self, tab: &mut Tab) -> egui::Id {
        egui::Id::new(("lattice-pane", *tab))
    }

    /// The picture panes never scroll. Each fills its body exactly — the
    /// lattice with a wgpu callback, the analyzer and the spiral with a
    /// painter over the whole rect — so there is nothing under the edge to
    /// reach, and a scroll area around them can only shift a picture that is
    /// meant to sit still.
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
        [false, !tab.is_picture()]
    }

    fn ui(&mut self, ui: &mut egui::Ui, tab: &mut Tab) {
        // Before the body draws anything — see [`pane_content_right`].
        let right = ui.max_rect().right();
        ui.data_mut(|d| d.insert_temp(pane_content_right(), right));
        match tab {
            Tab::Lattice => lattice_pane(ui, self.state, self.now),
            Tab::Tuning => tuning_pane(ui, self.state, self.params, self.now),
            Tab::Display => display_pane(ui, self.state, self.params),
            Tab::Console => console_pane(ui, self.state),
            Tab::Spectral => spectral_pane(ui, self.state, self.now, 0),
            Tab::Spiral => spiral_pane(ui, self.state, self.now),
            Tab::Notes => notes_pane(ui, self.state),
            Tab::Video => render_pane(ui, self.state, self.now),
            Tab::System => system_pane(ui, self.state),
        }
    }

    /// The picture panes paint their own surface edge to edge — the Spectral
    /// display its plot well, the Lattice its 3D view, the Spiral its disc — so
    /// the default 8px body margin reads as a pointless border around a picture
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
        tab.is_picture().then(|| {
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
    // ROUND and not truncate: `as u8` floors, which drops up to a whole level
    // of every non-integral channel — a systematic darkening.
    let byte = |v: f32| (v.clamp(0.0, 1.0) * 255.0).round() as u8;
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

/// The wheel/pinch zoom under the pointer — `(scroll, zoom)` — for a caller
/// to turn into a zoom in whatever unit its scene uses. `None` when the
/// pointer is elsewhere.
///
/// Gated on `contains_pointer` rather than `hovered()`: `hovered()` can be
/// suppressed by whatever draws over the gesture surface — a wgpu paint
/// callback under the lattice, or the spectral pane's split divider sitting
/// on top of it — or by a transient focus/interaction elsewhere, silently
/// killing the gesture. `contains_pointer` is pure geometry and answers
/// regardless.
///
/// Both the scroll delta (mouse wheel) and egui's `zoom_delta` are read and
/// handed back raw: a trackpad pinch or a ctrl+wheel arrives as a zoom
/// factor, not a scroll delta (egui zeroes the scroll for those), so a
/// caller that read only one would miss whichever gesture didn't use it.
pub(super) fn zoom_gesture(ui: &egui::Ui, response: &egui::Response) -> Option<(f32, f32)> {
    response.contains_pointer().then(|| ui.input(|i| (i.smooth_scroll_delta.y, i.zoom_delta())))
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
    let mut bar = ValueBar::new(&mut value, key.range(), key.label())
        .eased(key.logarithmic())
        .decimals(2);
    if let Some(display) = key.display() {
        bar = bar.display(display);
    }
    let response = bar.show(ui);
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
/// range whose ends are both automatable params (Color & light's color range).
/// `label` names the bar and `display` formats each end's readout.
///
/// Both params are bracketed for the whole drag and written every changed
/// frame, so a drag on either handle records as one gesture on each. A
/// double-click reset arrives as `changed` with no drag, and goes through the
/// same `set` without a gesture — matching [`param_bar`]'s one-shot path.
pub(super) fn param_range_bar(
    ui: &mut egui::Ui,
    params: &dyn ParamBackend,
    (low_key, high_key): (ParamKey, ParamKey),
    range: std::ops::RangeInclusive<f32>,
    min_span: f32,
    label: &str,
    display: fn(f32) -> String,
) -> egui::Response {
    let (mut low, mut high) = (params.get(low_key), params.get(high_key));
    let response = RangeBar::new(&mut low, &mut high, range, label)
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

/// One bar for a soft edge: how far it REACHES past whatever it surrounds, and
/// how much of that reach it spends FADING out. Three settings are this shape —
/// the lattice's knockout gutter, the piano roll's note outline, and the lead a
/// sounding note carries over the now-line.
///
/// The lead is the one whose axis is not in points, and it is worth saying that
/// this bar does not care: it is handed two numbers and a far end, and what
/// they measure is the caller's. See
/// [`SpectrumConfig::roll_lead`](crate::SpectrumConfig) for why that one is a
/// share of the analyzer where the outline beside it is a length.
///
/// The pair is stored as a reach and a fade WIDTH, and dragged as the two
/// points those describe — solid out to `reach - fade`, gone by `reach` — which
/// is what a [`RangeBar`] already is. So the two ends read out as the places
/// they are, the fade is the distance between them, and
/// [`fade_span`](RangeBar::fade_span) paints the fill to match.
///
/// The gestures are everything two bars of their own would give, plus one:
/// sliding the span moves the reach at a fixed fade, the low end moves the fade
/// at a fixed reach, and the high end pins where softening STARTS and moves
/// where it ends. Nothing here ties the fade to the reach — that would make a
/// wider edge always a blurrier one, which is the whole reason these are two
/// numbers.
///
/// A hard edge (fade 0) closes the span, which is why this bar takes no
/// `min_span` and why [`Grab::at`](crate::widgets) has a rule for a closed one.
///
/// Both numbers are distances from the same place, so the axis floors at 0 and
/// the caller passes only its far end.
///
/// `fresh` is where a double-click lands. A range bar's own reset opens to the
/// whole axis, which is the useful place to land for a window onto something
/// and the worst one here: both ends of this axis together are the widest edge
/// there is at the softest it goes, so the gesture would trade a dialled edge
/// for the most extreme one. The fresh pair is the neutral answer, and it is
/// what a double-click reaches for on a bar whose number is captured out of a
/// project rather than dragged to.
pub(super) fn edge_bar(
    ui: &mut egui::Ui,
    (reach, fade): (&mut f32, &mut f32),
    max: f32,
    label: &str,
    fresh: (f32, f32),
    display: fn(f32) -> String,
) -> egui::Response {
    // Clamped rather than trusted: a fade wider than its reach draws the same
    // as one exactly as wide (both shaders floor the fade at the edge it
    // surrounds), but it has no pair of points to put on the axis, and the
    // low end would read out somewhere the value does not say.
    // `ViewConfig::sanitize` and `SpectrumConfig::sanitize` hold the stored
    // pair to the same bound, so this only ever catches a live one.
    let (mut low, mut high) = ((*reach - *fade).max(0.0), *reach);
    let response =
        RangeBar::new(&mut low, &mut high, 0.0..=max, label).fade_span().display(display).show(ui);
    if response.double_clicked() {
        (*reach, *fade) = fresh;
    } else if response.changed() {
        (*reach, *fade) = (high, high - low);
    }
    response
}

/// The rule that separates a section from the one above it, so the FIRST
/// section in a pane has nothing to separate from and takes a bare heading.
/// The Nodes and View bodies write that case out by hand — they are built
/// from section functions in a fixed order, so each knows whether it leads
/// (and the Display sections that are one group apiece take no heading at
/// all, the fold-out header above them being the name it would carry).
///
/// A pane whose first section depends on the shell cannot know: the Video
/// pane leads with Record under a host and with Frame in the standalone,
/// which has no transport to record. The CURSOR is what answers it without
/// asking the caller — it starts at the top of the ui and only moves down
/// once something is laid out, so still being there means this heading is
/// the pane's first.
///
/// `min_rect` is the tempting reading of "has anything been drawn" and it
/// is the wrong one here: egui_dock wraps every pane body in a `ScrollArea`
/// whose ui arrives with `min_rect` already equal to `max_rect`, so it is a
/// full-height rect before the pane draws a thing. A fixture that builds
/// the pane ui directly sees an empty `min_rect` instead and cannot tell
/// the two apart — which is why `the_video_pane_does_not_start_with_a_rule`
/// goes through the real dock.
///
/// Shared by `section`'s plain heading and `display::fold_out`'s
/// collapsible one — a settings pane's header and Display's fold-out draw
/// the same rule the same way.
pub(super) fn section_separator(ui: &mut egui::Ui) {
    if ui.cursor().top() > ui.max_rect().top() + 0.5 {
        ui.add_space(4.0);
        ui.separator();
    }
}

/// A section header inside a settings pane: a little breathing room, a thin
/// rule, then the group's name in the heading (bold) face — so each block
/// of related controls is easy to pick out at a glance.
pub(super) fn section(ui: &mut egui::Ui, title: &str) {
    section_separator(ui);
    ui.heading(title);
}
