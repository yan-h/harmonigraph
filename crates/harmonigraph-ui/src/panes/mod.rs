//! Dock panes and their shared controls. The picture settings pages are composed in [`pages`].

use crate::params::{ParamBackend, ParamKey};
use crate::widgets::{RangeBar, ValueBar};
use crate::PictureState;

pub mod color;
pub mod console;
pub mod plus;
// Not a pane either — the node glow's own clock, run by the Lattice pane over
// the scene it has just derived.
pub mod glow_fade;
pub mod labels;
pub mod lattice;
mod lattice_atmosphere;
pub mod lighting;
pub(crate) mod node_motion;
pub mod nodes;
pub mod pages;
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

use color::color_pane;
use console::console_pane;
use lattice::lattice_pane;
use pages::{analyzer_settings_pane, lattice_settings_pane};
use render::render_pane;
use spiral::spiral_pane;
use system::system_pane;
use tuning::tuning_pane;

/// The surface every docked tab draws on. One id serves all of them because the
/// dock holds one tab per pane; the Video tab's preview takes 1, and the
/// offline renderer numbers its placements from 0 in a process of its own.
///
/// The Video preview has its own gesture mode, while an offline render has no
/// pointer at all; Analyzer gesture routing distinguishes all three.
pub(crate) const DOCKED_SURFACE: usize = 0;

/// The one target chrome both draggable pictures use in the Video preview.
/// A translucent accent fill keeps the destination legible over either a dark
/// spectrogram or a busy lattice, while the opaque edge keeps its bounds exact.
pub(crate) fn paint_preview_drop_target(painter: &egui::Painter, rect: egui::Rect) {
    painter.rect(
        rect.shrink(2.0),
        0.0,
        crate::theme::accent().gamma_multiply(0.18),
        egui::Stroke::new(2.0, crate::theme::accent_edge()),
        egui::StrokeKind::Inside,
    );
}

/// Wrap degrees into -180..=180 for display (orbit accumulates yaw
/// without bound).
pub(super) fn normalize_deg(deg: f32) -> f32 {
    (deg + 180.0).rem_euclid(360.0) - 180.0
}

/// 12-TET key spellings for MIDI-note readouts (the color range's ends in
/// [`color`], via [`pitch_readout`]). Octave numbers next to these use
/// Bitwig's convention (middle C = C3).
pub(super) const KEY_NAMES: [&str; 12] = [
    "C",
    "C\u{266F}",
    "D",
    "D\u{266F}",
    "E",
    "F",
    "F\u{266F}",
    "G",
    "G\u{266F}",
    "A",
    "A\u{266F}",
    "B",
];

/// A MIDI note as a key name and octave — "C1", "C8" — so a range's ends read
/// as pitches rather than bare numbers. Shared by the Octaves section's Center
/// and the Colors page's color range, which is why it is here rather than in
/// either.
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

/// Fixed workspace destinations. The selected analyzer and settings tabs are
/// persisted; retiring a variant still requires an audible parse refusal.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, serde::Serialize, serde::Deserialize)]
pub enum Tab {
    Lattice,
    /// Where the lattice's nodes sit in pitch: the prime bars, and the commas
    /// it tempers out.
    Tuning,
    /// Everything the Lattice picture draws and how it is lit. Titled
    /// "Lattice", after the picture it sets up.
    LatticeSettings,
    /// The Analyzer and Spiral pictures, the spectrogram in the Analyzer, and
    /// the analysis they share. Titled "Analyzer".
    AnalyzerSettings,
    /// Note and level colors, shared by every picture.
    Colors,
    /// Rendering cost and the editor's own interface.
    System,
    Console,
    /// The Spectral display: FFT curve, voices, and piano roll. Titled
    /// "Analyzer".
    Spectral,
    /// The same analyzer frame wound onto a chroma spiral, one turn per
    /// octave, with the sounding notes dotted on it and named around its rim.
    /// Titled "Spiral".
    Spiral,
    /// A live preview of the offline video frame, composed and adjusted here.
    /// Titled "Video".
    Video,
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
    ///
    /// Every tab outside the Settings column is a picture, so this is read off
    /// the section strips rather than listed again.
    pub(crate) fn is_picture(&self) -> bool {
        crate::workspace::Section::of(*self) != crate::workspace::Section::Settings
    }
}

pub struct Viewer<'a> {
    pub state: &'a mut PictureState,
    pub interaction: &'a mut crate::Interaction,
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
/// clip rect, which is the tab BODY — the workspace clips to the whole body and
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
/// a folded pane leaves behind (see [`crate::workspace`]).
pub fn tab_title(tab: &Tab) -> &'static str {
    match tab {
        Tab::Lattice => "Lattice",
        Tab::Tuning => "Tuning",
        // Deliberately the same names as the pictures they set up: a picture
        // and its settings are one feature on two surfaces, so each pair reads
        // as "the analyzer, and its knobs" rather than as two things to tell
        // apart.
        Tab::LatticeSettings => "Lattice",
        Tab::AnalyzerSettings => "Analyzer",
        Tab::Colors => "Colors",
        Tab::System => "System",
        Tab::Console => "Console",
        Tab::Spectral => "Analyzer",
        Tab::Spiral => "Spiral",
        Tab::Video => "Video",
    }
}

impl Viewer<'_> {
    pub(crate) fn id(&self, tab: &Tab) -> egui::Id {
        egui::Id::new(("pane-body", *tab))
    }

    pub(crate) fn scroll_bars(&self, tab: &Tab) -> [bool; 2] {
        [false, !tab.is_picture()]
    }

    pub(crate) fn ui(&mut self, ui: &mut egui::Ui, tab: &mut Tab) {
        // Before the body draws anything — see [`pane_content_right`].
        let right = ui.max_rect().right();
        ui.data_mut(|d| d.insert_temp(pane_content_right(), right));
        // A picture draws no sections, so it has no folds to hand down.
        if tab.is_picture() {
            return self.body(ui, tab);
        }
        // Before the body too — see [`SectionFolds`].
        let folds = SectionFolds {
            page: tab_title(tab),
            folded: std::mem::take(&mut self.interaction.folded_sections),
        };
        ui.data_mut(|d| d.insert_temp(SectionFolds::id(), folds));
        self.body(ui, tab);
        if let Some(folds) = ui.data_mut(|d| d.remove_temp::<SectionFolds>(SectionFolds::id())) {
            self.interaction.folded_sections = folds.folded;
        }
    }

    fn body(&mut self, ui: &mut egui::Ui, tab: &mut Tab) {
        match tab {
            Tab::Lattice => {
                self.state.runtime.lattice_maps = self.params.lattice_maps();
                lattice_pane(ui, self.state, self.now, DOCKED_SURFACE);
                if let Some(destination) = self.state.runtime.map_destination.take() {
                    self.params
                        .edit_lattice_map(crate::lattice_maps::MapEdit::Replace(destination));
                }
            }
            Tab::Tuning => tuning_pane(ui, self.state, self.params, self.now),
            Tab::LatticeSettings => {
                lattice_settings_pane(ui, self.state, self.interaction, self.params)
            }
            Tab::AnalyzerSettings => {
                analyzer_settings_pane(ui, self.state, self.interaction, self.params)
            }
            Tab::Colors => color_pane(ui, &mut self.state.appearance, self.params),
            Tab::System => system_pane(ui, &mut self.state.appearance, self.interaction),
            Tab::Console => console_pane(ui, &mut self.state.runtime),
            Tab::Spectral => {
                // The docked analyzer, and the reason the hold is applied HERE
                // rather than inside the pane: this is the one copy of it whose
                // size is a window's to change. The Video tab's preview draws
                // the same config in a box of its own, and the offline renderer
                // draws it at whatever the layout says — both reach
                // `spectral_pane` too, and either holding its own size would be
                // a second answer overwriting this one in the single fraction
                // all three compose from.
                self.interaction.analyzer_regions.draw(ui, self.state, self.now);
            }
            Tab::Spiral => spiral_pane(ui, self.state, self.now, DOCKED_SURFACE),
            Tab::Video => render_pane(ui, self.state, self.interaction, self.now),
        }
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

/// Whether `window` holds any node at all for `pc` under the current tuning:
/// the analyzer's red band for a voice the lattice has nowhere to light.
///
/// The question is "is this PLAYED pitch on the lattice", and
/// `Tuning::tolerance` is load-bearing in it — a note off every node is a note
/// the lattice cannot show, and saying so is the point.
///
/// `window` is [`PictureState::shown`](crate::PictureState::shown), the
/// picture's own window and not the view's reach. Taking a window rather than a
/// view is what makes that a choice a caller has to make rather than one it can
/// fall into.
///
/// It answers WHETHER and not WHICH, so it stops at the first match. That was
/// once one half of a pair: the Notes pane printed which node a voice sat on,
/// so it drained the same walk to pick a nearest. #975 retired that pane and
/// the nearest with it, which leaves the half worth having — the walk is not
/// small. The window is the camera's now, and a tilted one takes it to twenty
/// thousand positions against the reach's thousand, per voice, per frame.
/// Draining that to pick a winner measured 2.34ms on ten held voices where
/// stopping at the first match measured 40µs, against a whole `derive_scene`
/// priced at 1.2ms.
///
/// WHICH node is still a live question one caller away, and deliberately not
/// this one: [`names`](crate::panes::spectral::names)'s `naming_node` takes the
/// same played pitch and asks what to CALL it, where a collapsed equal
/// temperament makes the choice AMONG matches the whole problem rather than an
/// afterthought.
pub(super) fn window_shows_node(
    window: &harmonigraph_scene::DrawnWindow,
    tuning: &harmonigraph_core::Tuning,
    pc: harmonigraph_core::PitchClass,
) -> bool {
    window.positions().any(|pos| tuning.matches(pc, tuning.pitch_class(pos)))
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
/// Shift-wheel is the other spelling to preserve: egui deliberately turns its
/// vertical delta into a horizontal one for scroll areas, but on a picture
/// Shift selects a drag mode and must not disable the wheel's zoom.
pub(super) fn zoom_gesture(ui: &egui::Ui, response: &egui::Response) -> Option<(f32, f32)> {
    response.contains_pointer().then(|| {
        ui.input(|i| {
            let scroll = if i.modifiers.shift {
                i.smooth_scroll_delta.x + i.smooth_scroll_delta.y
            } else {
                i.smooth_scroll_delta.y
            };
            (scroll, i.zoom_delta())
        })
    })
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
    let (scale, suffix, decimals) = key.unit();
    let response = ValueBar::new(&mut value, key.range(), key.label())
        .eased(key.logarithmic())
        .unit(scale, suffix)
        .decimals(decimals)
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
/// range whose ends are both automatable params (the Colors page's color range).
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
/// how much of that reach it spends FADING out. Every setting shaped like that
/// pair takes this bar, so the two handles are one drag apart and the fade can
/// never outrun the edge it belongs to.
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
///
/// The pane cannot always say which section leads: the Video pane leads with
/// Record under a host and with Frame in the standalone, which has no transport
/// to record, and a page built from other panes' sections leads with whichever
/// it puts first. The CURSOR is what answers it without asking the caller — it
/// starts at the top of the ui and only moves down once something is laid out,
/// so still being there means this heading is the pane's first.
///
/// `min_rect` is the tempting reading of "has anything been drawn" and it
/// is the wrong one here: the workspace wraps settings bodies in a `ScrollArea`
/// whose ui arrives with `min_rect` already equal to `max_rect`, so it is a
/// full-height rect before the pane draws a thing. A fixture that builds
/// the pane ui directly sees an empty `min_rect` instead and cannot tell
/// the two apart — which is why `the_video_pane_does_not_start_with_a_rule`
/// goes through the real dock.
///
/// Called by [`section`] and by nothing else — a rule over a heading is what
/// a section is, so the two travel together.
///
/// The rule is the row gap plus a point either side of the line, and no more:
/// the heading row under it is already a row high around type that is not,
/// which is all the setting-off a section needs in a column kept tight. Any
/// room it does take is split evenly, because a folded heading sits between
/// two of these — space added above the rule alone lands under the heading
/// before it and nowhere over it, and a folded section reads as sitting high.
pub(super) fn section_separator(ui: &mut egui::Ui) {
    if ui.cursor().top() > ui.max_rect().top() + 0.5 {
        ui.add(egui::Separator::default().spacing(2.0 * crate::theme::ui_scale(ui.ctx())));
    }
}

/// Which settings sections are folded, handed from [`Viewer`]'s `ui` to every
/// [`section`] its body draws and back.
///
/// The set lives in [`Interaction`](crate::Interaction), persisted, because
/// the plugin builds a brand new egui `Context` every time the editor opens:
/// a fold kept in egui memory would spring open with every reopen. But a
/// section is drawn from deep inside functions that are handed a `Ui` and the
/// settings they draw and nothing else, so the set rides in the `Ui`'s data
/// for the length of one tab's body instead of down every signature.
///
/// Keys are "page/section" by title, so the same heading on two pages folds
/// separately and a blob names what it folds in words.
#[derive(Clone, Default)]
struct SectionFolds {
    page: &'static str,
    folded: std::collections::BTreeSet<String>,
}

impl SectionFolds {
    fn id() -> egui::Id {
        egui::Id::new("section-folds")
    }

    /// `f` over the set in place, or `None` outside a [`Viewer`] body. In place
    /// because every section of every drawn tab reads it every frame, and a
    /// `get_temp` would clone the whole set each time.
    fn with<R>(ui: &egui::Ui, f: impl FnOnce(&mut Self) -> R) -> Option<R> {
        let key = egui::util::id_type_map::RawKey::new::<Self>(Self::id());
        ui.data_mut(|d| d.get_temp_raw_mut(key)?.downcast_mut::<Self>().map(f))
    }
}

/// A section of a settings pane: a thin rule, then the group's name as a
/// [heading](section_header) that folds the section away, and `body` below it
/// while it is open.
///
/// The whole header row is the target, and its chevron leads the name where
/// a [`subsection`]'s does, so every fold in a pane is found in one column.
pub(super) fn section<R>(
    ui: &mut egui::Ui,
    title: &str,
    body: impl FnOnce(&mut egui::Ui) -> R,
) -> Option<R> {
    section_separator(ui);
    let (key, folded) = SectionFolds::with(ui, |folds| {
        let key = format!("{}/{title}", folds.page);
        let folded = folds.folded.contains(&key);
        (key, folded)
    })
    .unzip();
    let open = !folded.unwrap_or(false);
    let clicked = section_header(ui, title, open).clicked();
    crate::widgets::mark_spaced(ui);
    // Outside a [`Viewer`] body there is nowhere to keep a fold, so the header
    // stays put rather than hiding its body for the one frame of the click.
    let open = match key {
        Some(key) if clicked => {
            SectionFolds::with(ui, |folds| {
                if open {
                    folds.folded.insert(key);
                } else {
                    folds.folded.remove(&key);
                }
            });
            !open
        }
        _ => open,
    };
    open.then(|| body(ui))
}

/// Extra space between the letters of a [`section`] heading, at scale 1.
/// Capitals set solid read as a block; a little air makes them a label.
const HEADING_TRACKING: f32 = 1.2;

/// How far a [`section`] heading's capitals sit from the rule over them and
/// from the first thing under them, ink to ink: the gap every group in a
/// column keeps, so a heading is spaced as a line of text is.
const HEADING_GAP: f32 = crate::widgets::GROUP_GAP;

/// The heading row of a [`section`]: its name in small, spaced, dim capitals
/// — told from the rows under it by size, case and colour at once, where the
/// bold it replaced differed from them by weight alone.
fn section_header(ui: &mut egui::Ui, title: &str, open: bool) -> egui::Response {
    let scale = crate::theme::ui_scale(ui.ctx());
    let job = egui::text::LayoutJob::single_section(
        title.to_uppercase(),
        egui::TextFormat {
            font_id: egui::TextStyle::Small.resolve(ui.style()),
            extra_letter_spacing: HEADING_TRACKING * scale,
            color: crate::theme::text_dim(),
            ..Default::default()
        },
    );
    // The gap less the row gap, which lies between the heading and whatever
    // is over or under it. The rule's own point of room each side is not
    // counted: its line is drawn across the middle of it, so its ink is that
    // far from either edge anyway.
    let pad = HEADING_GAP * scale - ui.spacing().item_spacing.y;
    fold_header(ui, job, title, open, pad)
}

/// A fold's header row: a chevron that points at the name while folded and
/// down while open, centred in the first `indent`, then the name after it —
/// so a subsection's chevron and name sit exactly under its section's.
///
/// The row is the name trimmed to its capitals, as a
/// [`widgets::label`](crate::widgets::label) is, plus `pad` above and below.
/// Its target reaches half a row gap further each way, so a subsection with no
/// pad is still a comfortable click.
fn fold_header(
    ui: &mut egui::Ui,
    mut job: egui::text::LayoutJob,
    title: &str,
    open: bool,
    pad: f32,
) -> egui::Response {
    let indent = ui.spacing().indent;
    job.wrap =
        egui::text::TextWrapping::truncate_at_width((ui.available_width() - indent).max(0.0));
    let galley = ui.fonts_mut(|fonts| fonts.layout_job(job));
    let (top, bottom) = crate::widgets::cap_trim(ui, &galley);
    let (rect, row) = ui.allocate_exact_size(
        egui::vec2(ui.available_width(), galley.size().y - top - bottom + 2.0 * pad),
        egui::Sense::hover(),
    );
    let target = rect.expand2(egui::vec2(0.0, ui.spacing().item_spacing.y / 2.0));
    let response = ui.interact(target, row.id.with("fold"), egui::Sense::click());
    response.widget_info(|| {
        egui::WidgetInfo::selected(egui::WidgetType::CollapsingHeader, ui.is_enabled(), open, title)
    });
    let hot = response.hovered() || response.has_focus();
    let text = egui::pos2(rect.left() + indent, rect.top() + pad - top);
    ui.painter().galley(text, galley, crate::theme::text());
    crate::widgets::paint_chevron(
        ui.painter(),
        egui::pos2(rect.left() + indent / 2.0, rect.center().y),
        if open { egui::Vec2::Y } else { egui::Vec2::X },
        hot,
        crate::theme::ui_scale(ui.ctx()),
    );
    response
}

/// A fold inside a section, closed until opened: a [`fold_header`] in the
/// body face, and the body indented under it — the [`section`] header's
/// chevron and row, so the two levels read as one kind of control.
///
/// Its fold stays in egui memory rather than [`SectionFolds`] — a subsection
/// holds detail opened for the moment, and springs shut when the editor
/// reopens. Returns the header, for a hover.
///
/// It is a [group](crate::widgets::group_space), [`GROUP_GAP`](crate::widgets::GROUP_GAP)
/// from the rows over and under it, except directly under a section heading,
/// whose own gap already holds.
pub(super) fn subsection<R>(
    ui: &mut egui::Ui,
    title: &str,
    body: impl FnOnce(&mut egui::Ui) -> R,
) -> egui::Response {
    crate::widgets::group_space(ui);
    let id = ui.make_persistent_id(title);
    let mut fold =
        egui::collapsing_header::CollapsingState::load_with_default_open(ui.ctx(), id, false);
    let job = egui::text::LayoutJob::single_section(
        title.to_owned(),
        egui::TextFormat::simple(egui::TextStyle::Button.resolve(ui.style()), crate::theme::text()),
    );
    let header = fold_header(ui, job, title, fold.is_open(), 0.0);
    if header.clicked() {
        fold.toggle(ui);
    }
    fold.show_body_indented(&header, ui, body);
    // The fold, header and body both, is one group.
    crate::widgets::group_end(ui);
    header
}
