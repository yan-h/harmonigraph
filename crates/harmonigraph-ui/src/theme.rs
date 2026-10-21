//! The UI theme: how panel chrome looks. Every color comes from the
//! active `harmonigraph_scene::skin::Skin`, which also feeds the 3D scene — a
//! look is defined in exactly one struct. This module owns the *shapes*:
//! rounding, spacing, strokes, hover behavior, and the egui/dock plumbing.

use egui::{Color32, CornerRadius, FontId, Stroke, TextStyle, Vec2};
use harmonigraph_scene::skin::{self, active_skin};

// ---- Palette accessors ----------------------------------------------------
// All colors come from the active Skin (harmonigraph_scene::skin), the single
// place a look is defined; these helpers just convert to egui colors.

fn c(rgb: [u8; 3]) -> Color32 {
    Color32::from_rgb(rgb[0], rgb[1], rgb[2])
}

/// Window/panel background.
pub fn panel() -> Color32 {
    c(active_skin().panel)
}
/// Recessed areas: console scrollback, text edits, plot backgrounds.
pub fn well() -> Color32 {
    c(active_skin().well)
}
/// Pane headers and folded rails, one step above the page.
pub fn header() -> Color32 {
    c(active_skin().header)
}
/// Subtly raised surface between panel and widget (hovered tabs, faint
/// striping).
pub fn surface_faint() -> Color32 {
    c(active_skin().surface_faint)
}
/// Hairline strokes around noninteractive chrome.
pub fn hairline() -> Color32 {
    c(active_skin().hairline)
}
/// Resting widget fill (buttons, slider tracks).
pub fn widget() -> Color32 {
    c(active_skin().widget)
}
/// Hovered widget fill.
pub fn widget_hover() -> Color32 {
    c(active_skin().widget_hover)
}
/// Accent: selections, slider fill, a dragged divider.
pub fn accent() -> Color32 {
    c(active_skin().accent)
}
/// Primary text.
pub fn text() -> Color32 {
    c(active_skin().text)
}
/// Marks that are not text: unlit thumbs, chevrons, preview lines, and a
/// switched-off section's heading. Every text is [`text`].
pub fn text_dim() -> Color32 {
    c(active_skin().text_dim)
}
// Opaque accent mixes (accent blended into the surface colors). Alpha-based
// accents (gamma_multiply) made dragged widgets translucent, which read as
// a glitch; everything below is fully opaque.
/// ValueBar fill at rest.
pub fn accent_fill() -> Color32 {
    c(active_skin().accent_fill)
}
/// ValueBar fill, hovered.
pub fn accent_fill_hover() -> Color32 {
    c(active_skin().accent_fill_hover)
}
/// ValueBar fill while dragging.
pub fn accent_fill_drag() -> Color32 {
    c(active_skin().accent_fill_drag)
}
/// Pressed/active widget fill (buttons).
pub fn accent_active() -> Color32 {
    c(active_skin().accent_active)
}
/// Hover accent: a lit divider, a hovered switch track.
pub fn accent_edge() -> Color32 {
    c(active_skin().accent_edge)
}
/// Armed-mode indicator (learn mode).
pub fn armed() -> Color32 {
    c(active_skin().armed)
}
/// Warning text (e.g. notes not represented on the lattice).
pub fn warning_text() -> Color32 {
    c(active_skin().warning_text)
}
/// Background band behind warning rows.
pub fn warning_bg() -> Color32 {
    c(active_skin().warning_bg)
}

// ---- Picture colors -------------------------------------------------------
// What a picture pane draws INSIDE the picture, and so what an export shows.
// Fixed rather than skinned (see `harmonigraph_scene::skin`): reach for these,
// not the chrome roles above, for anything the offline renderer also draws.

/// The ground a picture pane is bedded on (see
/// `harmonigraph_scene::skin::picture_color`).
pub fn picture() -> Color32 {
    c(skin::PICTURE)
}
/// Rulings across a picture: the analyzer's grid, the spiral's seam and rays.
pub fn picture_ruling() -> Color32 {
    c(skin::PICTURE_RULING)
}
/// The line between two panes of an exported frame.
pub fn picture_divider() -> Color32 {
    c(skin::PICTURE_DIVIDER)
}
/// The analyzer's axis numbers.
pub fn picture_marking() -> Color32 {
    c(skin::PICTURE_MARKING)
}
/// Note names drawn in a picture.
pub fn picture_name() -> Color32 {
    c(skin::PICTURE_NAME)
}
/// The band behind a sounding note outside the scale.
pub fn picture_off_scale() -> Color32 {
    c(skin::PICTURE_OFF_SCALE)
}

/// The one corner radius every framed control wears — buttons, checkboxes,
/// the value/range bars, the toggle switch, the record button. Shared so they
/// read as one family instead of each rounding to its own taste. (A bar's own
/// handle is its own shape and doesn't use it.)
///
/// Sized against the row rhythm, not on its own: at [`ROW_GAP`] apart, a radius
/// well past the gap cuts a notch between two stacked bars wider than the gap
/// itself, and the column reads as a stack of pills rather than a grid of rows
/// (5 did, at a 3pt gap). 3 read as hard-edged boxes. 4 is otonal's, at its
/// 4pt gap.
///
/// The design size, at [scale](ui_scale) 1.0; anything drawing with it wants
/// [`control_radius`].
pub(crate) const CONTROL_RADIUS: u8 = 4;

const WIDGET_RADIUS: CornerRadius = CornerRadius::same(CONTROL_RADIUS);

/// Padding between a settings pane's controls and the left, top and bottom
/// edges of its tab body — what stops the bars and labels running into the
/// pane edge. The right edge keeps the wider [`PANE_GUTTER`].
///
/// The gap every group in the column already keeps ([`GROUP_GAP`](crate::widgets::GROUP_GAP)),
/// so the pane edge reads as one more group boundary rather than a measure of
/// its own.
///
/// Named because the geometry around it is easy to get wrong: the workspace clips
/// the tab body to the WHOLE body rect and only then insets it by this margin
/// (a `Frame`, which does not clip), so inside a pane the clip rect sits this
/// far OUTSIDE the content box. Anything asking "where does the pane end"
/// wants the content box, not the clip.
///
/// The design size, at [scale](ui_scale) 1.0; the dock insets by
/// [`dock_pane_margin`].
pub(crate) const PANE_INSET: f32 = crate::widgets::GROUP_GAP;

/// The margin down the right of a settings pane, wider than [`PANE_INSET`]
/// because it has a second job: it is the width of every scroll bar in the
/// panel. A floating bar is drawn over the content, and this gutter is the
/// only air it has to sit in ([`style_at`]). Narrow the gutter and the bars
/// narrow with it; widen the bar past it on its own and it goes back to
/// standing on the controls.
///
/// The design size, at [scale](ui_scale) 1.0; anything drawing with it wants
/// [`pane_gutter`].
pub(crate) const PANE_GUTTER: f32 = 8.0;

/// The gap between two rows of a settings pane, egui's `item_spacing.y`.
///
/// A settings pane is a tall column of short rows, so this is spent once per
/// row. 3 kept two bars from reading as one control and read cramped beside
/// otonal's panes; 4 is otonal's chrome gap, under its 22-point controls.
///
/// The design size, at [scale](ui_scale) 1.0.
pub(crate) const ROW_GAP: f32 = 4.0;

/// The width of every line, chrome and picture alike: a divider, a widget's
/// stroke, a curve, the spectrum's outline. Otonal's mark width, and like
/// otonal's the floor: only a grid is ruled finer — the analyzer's rulings, the
/// Spiral's seam and rays — because a grid is a scale behind the data and must
/// not outweigh it.
///
/// The design size, at [scale](ui_scale) 1.0; chrome that scales multiplies it.
pub(crate) const LINE_WIDTH: f32 = 2.0;

/// The width a grid is ruled at, the one line finer than [`LINE_WIDTH`]: the
/// analyzer's rulings, the Spiral's seam and rays, and the Intensity bar's clip
/// hatching, whose 3-point pitch a full line would fill solid. Otonal's
/// `GRID_WIDTH`, half a line.
pub(crate) const GRID_WIDTH: f32 = 0.5 * LINE_WIDTH;

/// Height of one control row in a settings pane, and of every control that can
/// make one: a bar's track, a button's frame, a checkbox, a text field.
///
/// ONE number for all of them, because a settings pane is read as a column of
/// rows, and a row a few points taller than its neighbours reads as a
/// misalignment rather than as emphasis. They reach it by three different
/// routes, which is what makes a shared number worth naming rather than
/// assuming: egui's own controls are grown to it by `interact_size` (see
/// [`style_at`]), the bars allocate it as their exact size, and the three that
/// egui's floor does not reach — a text field, the toggle switch, the record
/// button — ask for it (see [`crate::widgets`]).
///
/// 22 points holds the 15-point text with room over and under it, as otonal's
/// chrome controls do; 19 held 13.5-point text, and read small and tight.
///
/// The design size, at [scale](ui_scale) 1.0; anything drawing with it wants
/// [`row_height`].
pub(crate) const ROW_HEIGHT: f32 = 22.0;

/// Height of a section's tab bar, which is also the thickness a folded section
/// is squeezed to (see [`crate::workspace`]) and the depth of the chrome along
/// the top of the window: tab titles and the collapse arrow at the left of
/// every bar. Anything drawn OVER the sections keeps clear of it.
///
/// The design size, at [scale](ui_scale) 1.0; anything drawing with it wants
/// [`tab_bar_height`].
pub(crate) const TAB_BAR_HEIGHT: f32 = 29.0;

// ---- Chrome scale ----------------------------------------------------------
// One factor over every SIZE in the panel chrome — type, spacing, control
// heights, tab bars — so a small screen can be told to spend fewer of its
// pixels on knobs and more on the picture.
//
// Deliberately not egui's `zoom_factor`, which is the obvious tool and the
// wrong one here. Zoom works by moving `pixels_per_point`, and the plugin's
// shell (vendored egui-baseview) builds its `screen_rect`, its pointer
// coordinates, and its render descriptor from the NATIVE scale alone — so a
// zoom would lay the UI out for a window a different size than the one it has,
// and the fix is four edits deep in the input path of a plugin that runs
// inside somebody else's process. Scaling the style reaches the same chrome
// through code this crate owns.
//
// It also leaves the picture alone, which zoom would not: the lattice, the
// roll and the spectrogram size themselves off their pane rect, so nothing
// here moves a single node. That is the whole request — the panel gets out of
// the way, the picture is untouched.

/// What the chrome scale may be dialled to. Down to 0.7, where the 15pt body
/// text lands near 10.5pt; up to 1.5 for a large display seen from across a
/// room.
pub const UI_SCALE_RANGE: std::ops::RangeInclusive<f32> = 0.7..=1.5;

/// Where [`set_ui_scale`] leaves the factor for the chrome to find.
fn ui_scale_id() -> egui::Id {
    egui::Id::new("ui-scale")
}

/// A scale a context can actually be drawn at: in range, and not a NaN or an
/// infinity out of a hand-edited persisted blob.
///
/// Applied on the way IN as well as here, so the control cannot read out a
/// number the chrome is not actually drawn at.
pub(crate) fn sane_ui_scale(scale: f32) -> f32 {
    if scale.is_finite() {
        scale.clamp(*UI_SCALE_RANGE.start(), *UI_SCALE_RANGE.end())
    } else {
        1.0
    }
}

/// The chrome scale in force on this context — the design size, 1.0, unless a
/// shell has set one.
///
/// The offline renderer deliberately never sets one. It draws only the picture
/// panes, which size themselves off their rect rather than off this, so a
/// recorded frame cannot depend on where this control is left and the
/// determinism test stays honest.
pub(crate) fn ui_scale(ctx: &egui::Context) -> f32 {
    ctx.data(|d| d.get_temp::<f32>(ui_scale_id())).unwrap_or(1.0)
}

/// Put `scale` in force on this context: rebuild the style at that size, and
/// leave the factor where [`ui_scale`] can find it. Reports whether anything
/// moved, which is a caller's cue that a `Ui` built from the old style is now
/// a frame behind.
///
/// Cheap to call every frame, and meant to be: the factor is stored in the
/// context's per-frame temp data, so a context that stops being told falls back
/// to the design size rather than keeping a stale one, and one that loses the
/// value gets it (and its style) back on the next frame.
pub fn set_ui_scale(ctx: &egui::Context, scale: f32) -> bool {
    let scale = sane_ui_scale(scale);
    let previous = ctx.data(|d| d.get_temp::<f32>(ui_scale_id()));
    if previous == Some(scale) {
        return false;
    }
    ctx.data_mut(|d| d.insert_temp(ui_scale_id(), scale));
    // A context that has only ever been at the design size keeps the style it
    // came with, untouched. [`apply_theme`] has already put it there, so the
    // rebuild would be a no-op on a shell — and on a context that never had
    // the theme applied at all it would be worse than one: it would install
    // this crate's type roles, which name a font FAMILY that only
    // [`install_fonts`] binds, and every piece of text laid out afterwards
    // would panic on the missing family.
    if scale == 1.0 && previous.is_none() {
        return false;
    }
    ctx.set_style_of(egui::Theme::Dark, style_at(scale));
    true
}

/// [`CONTROL_RADIUS`] at this scale.
pub(crate) fn control_radius(scale: f32) -> u8 {
    scaled_points(CONTROL_RADIUS, scale)
}

/// [`PANE_GUTTER`] at this scale.
pub(crate) fn pane_gutter(scale: f32) -> f32 {
    PANE_GUTTER * scale
}

/// The gap between two buttons side by side, at this scale: the tab bar's
/// margin around a row-high control, so a pane header's tabs and a settings
/// row's buttons stand the same distance apart. Narrower than
/// `item_spacing.x`, which also separates a label from the controls it names.
pub(crate) fn button_gap(scale: f32) -> f32 {
    (TAB_BAR_HEIGHT - ROW_HEIGHT) * 0.5 * scale
}

/// The margin the dock insets a settings pane's content by, in the whole
/// points egui stores a `Margin` in: [`PANE_INSET`] left, top and bottom, and
/// [`PANE_GUTTER`] on the right, which the floating scroll bar is sized to.
pub(crate) fn dock_pane_margin(scale: f32) -> egui::Margin {
    let inset = (PANE_INSET * scale) as i8;
    egui::Margin { left: inset, right: pane_gutter(scale) as i8, top: inset, bottom: inset }
}

/// [`ROW_HEIGHT`] at this scale.
pub(crate) fn row_height(scale: f32) -> f32 {
    ROW_HEIGHT * scale
}

/// [`TAB_BAR_HEIGHT`] at this scale, including the fold button.
pub(crate) fn tab_bar_height(scale: f32) -> f32 {
    TAB_BAR_HEIGHT * scale
}

/// The narrowest (or shortest) a pane may be dragged: four tab bars, which is
/// about where a settings pane still has room for a label and a value beside it,
/// and where a picture is still a picture.
///
/// Not the width a FOLD leaves behind (that is one tab bar): a pane dragged this
/// small is still a pane, drawing its body, with none of a rail's chrome. Folding
/// is the way to get it out of the way, and it is a click on the arrow.
pub(crate) fn min_pane(scale: f32) -> f32 {
    4.0 * tab_bar_height(scale)
}

/// The narrowest a settings pane's content box is ever laid out at, in design
/// points: where a plot still has room for its exact values beside the picture
/// (a 112-point well, the gap, and bars wide enough for a name and a readout),
/// and the Tuning pane's sources still fit their three-column grid.
///
/// Above it a narrower column only wraps a row of buttons or turns one into a
/// dropdown, and each row keeps its place. Below it those two blocks would have
/// to stack, which moves everything under them by hundreds of points, so the
/// column stops here instead and neither has a stacked layout to fall back to.
pub(crate) const SETTINGS_MIN_CONTENT: f32 = 220.0;

/// The narrowest the settings column is dragged or fitted to, at this scale:
/// [`SETTINGS_MIN_CONTENT`] plus the pane margins either side of it. Only a
/// window too narrow to hold it at all draws the column any narrower.
pub(crate) fn min_settings(scale: f32) -> f32 {
    SETTINGS_MIN_CONTENT * scale + dock_pane_margin(scale).sum().x
}

/// Give a scroll area built INSIDE a pane a lane of its own to draw its bar in.
///
/// The bar is [as wide as the gutter](style_at) a pane leaves down its right
/// side, which is what keeps it clear of the controls in the panes the dock
/// scrolls for: that area's edge is the pane's edge, and the gutter is the air
/// between them. An area built inside a pane starts at the CONTENT box, where
/// there is no such air, so a floating bar there sits on the last few points of
/// whatever it scrolls — the run of a log line, the right end of a row.
///
/// Reserving the lane costs the area the bar's width, and only while a bar is
/// showing: egui takes `floating_allocated_width` out of a floating area's size
/// per axis that actually overflows. It rides on the UI's spacing, which is the
/// only place egui reads a scroll bar's size from — there is no per-area way to
/// set one — so it holds for every area built in `ui` from here on. A pane that
/// wants the lane for one of its areas wants it for all of them.
pub(crate) fn reserve_scroll_gutter(ui: &mut egui::Ui) {
    let scroll = &mut ui.spacing_mut().scroll;
    scroll.floating_allocated_width = scroll.bar_width;
}

// ---- Fonts -----------------------------------------------------------------

/// How far the face's glyphs are drawn below where egui lays them out, as a
/// fraction of the font size: what centres its CAPITALS in its line box.
///
/// Iosevka stands its capitals high in the box its metrics give it, 0.26 em
/// over them against 0.32 em under the baseline, so egui centring a box — a
/// button's text, a bar's name — stood the caps about a point above the middle
/// of a row. Visual only: egui lays the text out exactly as before, so no row,
/// width or wrap moves, and every box-centred run comes out cap-centred.
///
/// The chrome's alone, in the proportional family, which every chrome text is
/// set in (its `TextStyle::Monospace` included). The picture sets its text in
/// [`picture_family`], untweaked, because it places every glyph by the ink
/// egui measures for it, and a shift egui does not lay out would stand the ink
/// off where the placement put it.
pub(crate) const CAP_CENTRING: f32 = 0.031;

/// The family every text the picture draws is set in — lattice and analyzer
/// names, markings, the badge over the lattice — and so what the offline
/// renderer draws too: the face without [`CAP_CENTRING`]. Picture code asks for
/// it by this name, so text that is picture rather than chrome says so where it
/// is laid out; chrome code never asks for egui's monospace family directly.
///
/// egui's monospace family rather than a named one of its own: egui panics
/// laying text out in a named family a context was never given, and a context
/// without the theme, which only the stock families survive, is one a picture
/// must still draw in rather than take the host down from.
pub(crate) fn picture_family() -> egui::FontFamily {
    egui::FontFamily::Monospace
}

/// [`picture_family`] at `size` points.
pub(crate) fn picture_font(size: f32) -> FontId {
    FontId::new(size, picture_family())
}

/// The family of the chrome text that marks where things are — section
/// headings, tab labels, block titles — in Medium, where every other chrome
/// text is Regular. Reached through `TextStyle::Heading`, the role all three
/// take, rather than asked for by name at each.
///
/// A named family, which egui panics laying text out in where a context was
/// never given the theme; the chrome is only drawn in one that was, unlike
/// the picture (see [`picture_family`]).
fn heading_family() -> egui::FontFamily {
    egui::FontFamily::Name("ChromeMedium".into())
}

/// Install the product face, Iosevka Fixed Curly, ahead of egui's bundled
/// faces: words, numbers, headings and the picture's own labels are one face,
/// as otonal sets them. The TTFs are the same OFL subset of it in two weights
/// (Latin-1, punctuation, arrows, comparison and the music accidentals), with
/// the license beside them in `fonts/`; egui's faces stay as per-glyph
/// fallbacks.
///
/// - The chrome, egui's proportional family, is Regular: Medium over every row
///   of a pane was too heavy.
/// - The chrome's headings ([`heading_family`]) are Medium, the weight otonal
///   chose because egui draws text thinner than macOS draws its own, kept for
///   the few runs one reads to find one's place.
/// - The picture ([`picture_family`]) is Medium too: the drawn marks' stroke
///   is fitted to its letters ([`crate::marks::MARK_WEIGHT`]).
///
/// The two weights share every advance and vertical metric, so which weight a
/// text takes moves no row, width or elision. Both chrome families carry
/// [`CAP_CENTRING`]; the picture's is the face without it.
pub(crate) fn install_fonts(ctx: &egui::Context) {
    use egui::epaint::text::{FontData, FontDefinitions, FontFamily};

    const REGULAR: &[u8] = include_bytes!("../fonts/iosevka/IosevkaFixedCurly-Regular.ttf");
    const MEDIUM: &[u8] = include_bytes!("../fonts/iosevka/IosevkaFixedCurly-Medium.ttf");
    let centred = |face| {
        FontData::from_static(face)
            .tweak(egui::FontTweak { y_offset_factor: CAP_CENTRING, ..Default::default() })
    };
    let mut fonts = FontDefinitions::default();
    for (name, data) in [
        ("IosevkaChrome", centred(REGULAR)),
        ("IosevkaHeading", centred(MEDIUM)),
        ("IosevkaPicture", FontData::from_static(MEDIUM)),
    ] {
        fonts.font_data.insert(name.to_owned(), std::sync::Arc::new(data));
    }

    // This runs inside the plugin's editor-open path, where a panic takes
    // the host down: if an egui upgrade ever drops the default families,
    // degrade to egui's bundled fonts instead of crashing the DAW.
    for (family, face) in
        [(FontFamily::Proportional, "IosevkaChrome"), (picture_family(), "IosevkaPicture")]
    {
        if let Some(faces) = fonts.families.get_mut(&family) {
            faces.insert(0, face.to_owned());
        }
    }
    // The headings fall back as the chrome does, after their own face.
    let fallbacks = fonts.families.get(&FontFamily::Proportional).cloned().unwrap_or_default();
    let headings = std::iter::once("IosevkaHeading".to_owned())
        .chain(fallbacks.into_iter().filter(|face| face != "IosevkaChrome"));
    fonts.families.insert(heading_family(), headings.collect());
    ctx.set_fonts(fonts);
}

// ---- Application ----------------------------------------------------------

/// Apply the theme to a context. Each shell calls this once at startup
/// (eframe's creation context; the plugin editor's build closure).
pub fn apply_theme(ctx: &egui::Context) {
    install_fonts(ctx);
    // The lattice is a dark-background instrument; pin the UI to dark
    // rather than following the host/system preference.
    ctx.set_theme(egui::ThemePreference::Dark);
    ctx.set_style_of(egui::Theme::Dark, style_at(ui_scale(ctx)));
    // Recorded so [`set_skin`] rebuilds only when the skin it is told differs
    // from the one this style was built in, which on a thread shared with
    // another editor need not be the default.
    ctx.data_mut(|d| d.insert_temp(skin_id(), skin::active_skin_key()));
}

/// Where [`apply_theme`] and [`set_skin`] leave the dials the context's style
/// was built in.
fn skin_id() -> egui::Id {
    egui::Id::new("skin")
}

/// Put the skin `dials` make in force for this frame: make it this thread's
/// active skin (what every color accessor reads), and rebuild the context's
/// style if it was built in another. Reports whether the style moved, the
/// same cue [`set_ui_scale`] gives.
///
/// Called every frame, before [`set_ui_scale`], which builds its style from
/// whatever skin is active. Cheap when nothing changed: one comparison, and
/// no remix. Keyed on the dials, everything the colours are made of, so a
/// dragged bar restyles every frame it moves and no other.
pub fn set_skin(ctx: &egui::Context, dials: skin::SkinDials) -> bool {
    skin::set_active_skin(dials);
    // Nothing built means the theme never reached this context, which keeps
    // its stock style for the reason [`set_ui_scale`] gives.
    let Some(built) = ctx.data(|d| d.get_temp::<skin::SkinDials>(skin_id())) else {
        return false;
    };
    if built == dials {
        return false;
    }
    ctx.data_mut(|d| d.insert_temp(skin_id(), dials));
    ctx.set_style_of(egui::Theme::Dark, style_at(ui_scale(ctx)));
    true
}

/// The theme's style at a given [chrome scale](ui_scale).
///
/// Built from egui's defaults rather than from whatever the context is wearing,
/// because a scale change rebuilds this from scratch: starting from the style
/// already in force would multiply the last scale by the new one and walk the
/// chrome off in whichever direction it was dialled. `Style::default` is
/// exactly egui's dark style (its `Visuals::default` IS `Visuals::dark`), so
/// this is the same starting point either way.
///
/// Everything below is written at the DESIGN size, scale 1.0, and
/// [`scale_chrome`] multiplies the lot at the end — so this reads as the one
/// specification of how the panel looks rather than as a spec with a factor
/// threaded through every number.
fn style_at(scale: f32) -> egui::Style {
    // Type roles (every family is the one face; see install_fonts). Text side
    // by side is one size, a label, a button and a bar's readout alike, as
    // otonal sets its chrome; Small is for the few runs laid over something
    // else. In the initializer rather than assigned after it because clippy
    // reads the assign-after form as a `Default` that meant to be a struct
    // literal, and it is right.
    let mut style = egui::Style {
        text_styles: [
            // The one Medium role: egui's own titles (a window's), settings
            // section headings in spaced capitals (`panes::section_header`),
            // tab labels and block titles (`panes::block`).
            (TextStyle::Heading, FontId::new(15.0, heading_family())),
            (TextStyle::Body, FontId::proportional(15.0)),
            (TextStyle::Button, FontId::proportional(15.0)),
            (TextStyle::Small, FontId::proportional(12.0)),
            // egui's name for the role a readout and the console take, in the
            // chrome's family like every other chrome text (see `install_fonts`).
            (TextStyle::Monospace, FontId::proportional(15.0)),
        ]
        .into(),
        ..Default::default()
    };

    // The gap ACROSS a row is a different question from [`ROW_GAP`] — that one
    // separates a label from the buttons it names — and stays wider.
    style.spacing.item_spacing = Vec2::new(8.0, ROW_GAP);
    // `interact_size` is the FLOOR under every control's height — the size egui
    // gives one with no opinion of its own (a checkbox, a text field), and the
    // minimum it grows a button to. That floor is what puts them all on
    // [`ROW_HEIGHT`] exactly, at every scale, and it is worth knowing it is the
    // floor doing it: egui stores a button frame's margin as WHOLE POINTS, so a
    // button sized by its padding alone can only land on its text plus an even
    // number of points — never on a round 20, and a point out from the bar
    // beside it. Sized by the floor there is no rounding to land wrong.
    //
    // Which leaves the vertical padding one job: stay out of the way. A button
    // is as tall as its text plus this, or the floor, whichever is more, so
    // any value that fits under the floor draws the same button and any value
    // over it is a button standing taller than its row.
    //
    // So it is none at all: the frame margin rounds to whole points where the
    // type does not, so any padding is a button that can stand a point taller
    // than its row at some scale. With none the floor holds at every scale in
    // [`UI_SCALE_RANGE`]; a 2x display rounds the line to half points, which
    // moves it by a quarter point at most. This is what a session retuning
    // the type or the row is spending, and `every_settings_row_is_one_row_high`
    // sweeps the range and says so after a change to either.
    style.spacing.button_padding = Vec2::new(5.0, 0.0);
    style.spacing.interact_size = Vec2::new(36.0, ROW_HEIGHT);
    style.spacing.slider_width = 160.0;
    style.spacing.slider_rail_height = 4.0;

    let visuals = &mut style.visuals;
    visuals.panel_fill = panel();
    visuals.window_fill = panel();
    visuals.extreme_bg_color = well();
    visuals.faint_bg_color = surface_faint();

    // Selection keeps alpha (it overlays glyphs). egui reuses this pair for
    // the *selected* button look (Meantone/Learn, every selectable_value):
    // fill = selection.bg_fill, text = selection.stroke color.
    // Accent-blue-on-translucent-accent reads as blue-on-blue (~3:1), and
    // against the brightness of a resting button a 0.35 fill does not
    // separate from an unselected one. A denser fill plus bright text makes the
    // selected state unmistakable while still reading fine as a text
    // highlight over glyphs.
    visuals.selection.bg_fill = accent().gamma_multiply(0.5);
    visuals.selection.stroke = Stroke::new(LINE_WIDTH, text());
    visuals.slider_trailing_fill = true;
    visuals.window_corner_radius = CornerRadius::same(8);
    visuals.menu_corner_radius = CornerRadius::same(6);

    // Widget states: flat fills, rounded corners, no outlines. Every state is
    // the same SIZE — hover and press are read off the fill, which costs no
    // space, and keyboard focus draws as press does.
    //
    // egui's `expansion` is the alternative, a hover swell of a point or two,
    // and it is not free here: it is paid back through a negative outer margin
    // on the button's frame, and egui stores frame margins as whole points. At
    // any [chrome scale](ui_scale) that leaves the expansion fractional the two
    // round apart, so the swell lands in the widget's ALLOCATED size — the row
    // after it slides sideways under the pointer, and a `button_row` wide
    // enough to wrap re-wraps. (Measured at scale 1.25: a hovered button 2pt
    // wider and everything right of it 2pt over; at 1.5, 2pt the other way.)
    let w = &mut visuals.widgets;

    w.noninteractive.bg_fill = panel();
    w.noninteractive.bg_stroke = Stroke::new(LINE_WIDTH, hairline());
    w.noninteractive.fg_stroke = Stroke::new(LINE_WIDTH, text());
    w.noninteractive.corner_radius = WIDGET_RADIUS;

    w.inactive.bg_fill = widget();
    w.inactive.weak_bg_fill = widget();
    w.inactive.bg_stroke = Stroke::NONE;
    w.inactive.fg_stroke = Stroke::new(LINE_WIDTH, text());
    w.inactive.corner_radius = WIDGET_RADIUS;

    w.hovered.bg_fill = widget_hover();
    w.hovered.weak_bg_fill = widget_hover();
    w.hovered.bg_stroke = Stroke::NONE;
    w.hovered.fg_stroke = Stroke::new(LINE_WIDTH, text());
    w.hovered.corner_radius = WIDGET_RADIUS;

    w.active.bg_fill = accent_active();
    w.active.weak_bg_fill = accent_active();
    w.active.bg_stroke = Stroke::NONE;
    w.active.fg_stroke = Stroke::new(LINE_WIDTH, Color32::WHITE);
    w.active.corner_radius = WIDGET_RADIUS;

    w.open.bg_fill = widget();
    w.open.weak_bg_fill = widget();
    w.open.bg_stroke = Stroke::new(LINE_WIDTH, accent_edge());
    w.open.fg_stroke = Stroke::new(LINE_WIDTH, text());
    w.open.corner_radius = WIDGET_RADIUS;

    scale_chrome(&mut style, scale);

    // A floating scroll bar is drawn OVER the content rather than beside it, so
    // its width is only free where the pane already leaves air: the
    // [`PANE_GUTTER`] down the right of every settings pane. egui's
    // default bar is 10pt against that 8pt gutter, and the 2pt of overhang lands
    // on the right end of every bar in the column, which is where the value
    // readouts are. Sized to the gutter it fills the air that is there and
    // touches nothing, expanded or dormant.
    //
    // Set HERE, after the scale, and this is the one number in the block above
    // that cannot be written at the design size: the gutter is a `Margin`, egui
    // stores those as whole points, and truncation does not survive being
    // multiplied. A bar scaled as a float overhangs the truncated gutter by the
    // fraction — 0.8pt of bar over every control at scale 1.1 — so it is taken
    // from [`dock_pane_margin`], which is what the dock actually insets by.
    //
    // The alternative is `floating_allocated_width`, which reserves the bar a
    // lane of its own and takes it out of the content — right for a scroll area
    // that has no gutter to sit in (see [`reserve_scroll_gutter`]), wrong here,
    // where it would inset the column a second time and reflow the whole pane
    // the moment its content grew past the bottom.
    style.spacing.scroll.bar_width = f32::from(dock_pane_margin(scale).right);
    style
}

/// Multiply every SIZE in `style` by `scale` — type, spacing, control heights,
/// the radii and margins that go with them.
///
/// Colors and STROKE WIDTHS are the deliberate exceptions. A hairline is a
/// hairline at any size, and the widths here are all 1-1.2 points: taken down
/// to 0.7 they land under a single pixel on a display with only one per point,
/// where the rasterizer pays for the shortfall in alpha. The border would then
/// read as having faded rather than as having shrunk, which is a different
/// control, not a smaller one.
///
/// Written field by field rather than swept generically because egui's `Style`
/// mixes sizes with counts, ratios and opacities in the same structs; the ones
/// listed here are the ones that are lengths.
fn scale_chrome(style: &mut egui::Style, scale: f32) {
    if scale == 1.0 {
        return;
    }
    for font in style.text_styles.values_mut() {
        font.size *= scale;
    }

    let s = &mut style.spacing;
    for v in
        [&mut s.item_spacing, &mut s.button_padding, &mut s.interact_size, &mut s.default_area_size]
    {
        *v *= scale;
    }
    for f in [
        &mut s.indent,
        &mut s.slider_width,
        &mut s.slider_rail_height,
        &mut s.combo_width,
        &mut s.text_edit_width,
        &mut s.icon_width,
        &mut s.icon_width_inner,
        &mut s.icon_spacing,
        &mut s.tooltip_width,
        &mut s.menu_width,
        &mut s.menu_spacing,
        &mut s.combo_height,
        // The scroll bars a settings pane overflows into, which are chrome
        // like everything else — a full-size bar down the side of a shrunken
        // pane is the one piece that would give the game away. `bar_width` is
        // not here: it has a whole-point gutter to fit inside rather than a
        // factor to follow, so [`style_at`] sets it after this runs.
        &mut s.scroll.handle_min_length,
        &mut s.scroll.bar_inner_margin,
        &mut s.scroll.bar_outer_margin,
        &mut s.scroll.floating_width,
        &mut s.scroll.floating_allocated_width,
    ] {
        *f *= scale;
    }
    for m in [&mut s.window_margin, &mut s.menu_margin, &mut s.scroll.content_margin] {
        *m = scale_margin(*m, scale);
    }

    let v = &mut style.visuals;
    v.window_corner_radius = scale_corner_radius(v.window_corner_radius, scale);
    v.menu_corner_radius = scale_corner_radius(v.menu_corner_radius, scale);
    v.resize_corner_size *= scale;
    v.clip_rect_margin *= scale;
    for shadow in [&mut v.window_shadow, &mut v.popup_shadow] {
        shadow.offset = [scale_i8(shadow.offset[0], scale), scale_i8(shadow.offset[1], scale)];
        shadow.blur = scaled_points(shadow.blur, scale);
        shadow.spread = scaled_points(shadow.spread, scale);
    }
    for w in [
        &mut v.widgets.noninteractive,
        &mut v.widgets.inactive,
        &mut v.widgets.hovered,
        &mut v.widgets.active,
        &mut v.widgets.open,
    ] {
        w.corner_radius = scale_corner_radius(w.corner_radius, scale);
        // `expansion` is deliberately not here: no widget state has any (see
        // `style_at`), and a scaled one is exactly the fractional expansion
        // that moves a hovered widget's neighbours.
    }
}

/// Scale a rounding, which egui stores per corner as whole points.
fn scale_corner_radius(radius: CornerRadius, scale: f32) -> CornerRadius {
    CornerRadius {
        nw: scaled_points(radius.nw, scale),
        ne: scaled_points(radius.ne, scale),
        sw: scaled_points(radius.sw, scale),
        se: scaled_points(radius.se, scale),
    }
}

/// Scale a margin, which egui stores per side as whole points.
fn scale_margin(margin: egui::Margin, scale: f32) -> egui::Margin {
    egui::Margin {
        left: scale_i8(margin.left, scale),
        right: scale_i8(margin.right, scale),
        top: scale_i8(margin.top, scale),
        bottom: scale_i8(margin.bottom, scale),
    }
}

/// Scale a whole-point length, keeping a nonzero one nonzero: rounding a 1pt
/// margin or radius to nothing at 0.7 does not shrink it, it removes it, and
/// what comes back at 1.0 then looks like a different control rather than the
/// same one bigger.
pub(crate) fn scaled_points(value: u8, scale: f32) -> u8 {
    let scaled = (f32::from(value) * scale).round();
    if value > 0 {
        scaled.max(1.0) as u8
    } else {
        0
    }
}

/// [`scaled_points`] for a signed length (a shadow offset, a margin side),
/// where the floor applies in whichever direction the value already points.
fn scale_i8(value: i8, scale: f32) -> i8 {
    let scaled = (f32::from(value) * scale).round();
    match value {
        0 => 0,
        v if v > 0 => scaled.max(1.0) as i8,
        _ => scaled.min(-1.0) as i8,
    }
}
