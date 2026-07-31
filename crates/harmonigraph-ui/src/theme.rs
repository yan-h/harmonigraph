//! The UI theme: how panel chrome looks. Every color comes from the
//! active `harmonigraph_scene::skin::Skin`, which also feeds the 3D scene — a
//! look is defined in exactly one struct. This module owns the *shapes*:
//! rounding, spacing, strokes, hover behavior, and the egui/dock plumbing.

use egui::{Color32, CornerRadius, FontId, Stroke, TextStyle, Vec2};
use harmonigraph_scene::skin::active_skin;

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
/// Accent: selections, slider fill, hover strokes.
pub fn accent() -> Color32 {
    c(active_skin().accent)
}
/// Primary text.
pub fn text() -> Color32 {
    c(active_skin().text)
}
/// Secondary text (labels, weak()).
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
/// Hover/focus stroke color.
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

/// The one corner radius every framed control wears — buttons, checkboxes,
/// the value/range bars, the record button. Shared so they read as one family
/// instead of each rounding to its own taste. (Pill-shaped things — the toggle
/// switch, a bar's own handle — are their own shape and don't use it.)
///
/// The design size, at [scale](ui_scale) 1.0; anything drawing with it wants
/// [`control_radius`].
pub(crate) const CONTROL_RADIUS: u8 = 5;

const WIDGET_RADIUS: CornerRadius = CornerRadius::same(CONTROL_RADIUS);

/// Padding between a settings pane's controls and the edge of its tab body —
/// what stops the bars and labels running into the pane edge.
///
/// Named because the geometry around it is easy to get wrong: egui_dock clips
/// the tab body to the WHOLE body rect and only then insets it by this margin
/// (a `Frame`, which does not clip), so inside a pane the clip rect sits this
/// far OUTSIDE the content box. Anything asking "where does the pane end"
/// wants the content box, not the clip.
///
/// The design size, at [scale](ui_scale) 1.0; anything drawing with it wants
/// [`pane_inner_margin`].
pub(crate) const PANE_INNER_MARGIN: f32 = 8.0;

/// Height of a leaf's tab bar, which is also the thickness a folded pane is
/// squeezed to (see [`crate::fold`]) and the depth of dock chrome along the
/// top of the window: tab titles and the collapse arrow at the left of every
/// bar. Anything drawn OVER the dock keeps clear of it.
///
/// The design size, at [scale](ui_scale) 1.0; anything drawing with it wants
/// [`tab_bar_height`].
pub(crate) const TAB_BAR_HEIGHT: f32 = 26.0;

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

/// What the chrome scale may be dialled to. Down to 0.7, where the 13.5pt body
/// text lands near 9.5pt, which is about as small as Atkinson stays legible;
/// up to 1.5 for a large display seen from across a room.
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
/// Cheap to call every frame, and meant to be: the factor is stored the same
/// per-frame way as [`crate::panes::pane_content_right`], so a context that
/// stops being told falls back to the design size rather than keeping a stale
/// one, and one that loses the value gets it (and its style) back on the next
/// frame.
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

/// [`PANE_INNER_MARGIN`] at this scale.
pub(crate) fn pane_inner_margin(scale: f32) -> f32 {
    PANE_INNER_MARGIN * scale
}

/// [`TAB_BAR_HEIGHT`] at this scale.
///
/// No floor, and the collapse button is the reason to say so: egui_dock's
/// `TAB_COLLAPSE_BUTTON_SIZE` of 24 points is its WIDTH alone. The button's
/// rect runs `tabbar_outer_rect.left_top()` to `left_bottom() + (24, 0)`, so it
/// is as tall as whatever bar it sits in and only ever 24 wide, and the arrow
/// centred in it is a 10-point glyph. A bar shorter than 24 therefore clips
/// nothing — it takes the button's height down with it, which is the point.
///
/// What that leaves unscaled is the button's 24-point WIDTH and its 10-point
/// arrow, both private consts reachable only by forking egui_dock. So the
/// button grows squatter as the scale comes down rather than shrinking with
/// everything else. `fold`'s [`ARROW_BUTTON`](crate::fold) mirrors the same 24
/// deliberately, and has to keep mirroring it.
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

// ---- Fonts -----------------------------------------------------------------

/// The named family headings resolve to (Atkinson Bold first).
pub const HEADING_FAMILY: &str = "heading";

/// Install the product fonts: Atkinson Hyperlegible for proportional text
/// (with its Bold in a dedicated heading family), Iosevka Fixed for
/// monospace — chosen for its narrow numerals in ValueBars and readouts,
/// and used by the console. All OFL; license texts ship next to the TTFs
/// in `fonts/`. egui's bundled faces remain as fallbacks.
fn install_fonts(ctx: &egui::Context) {
    use egui::epaint::text::{FontData, FontDefinitions, FontFamily};

    let mut fonts = FontDefinitions::default();
    for (name, bytes) in [
        (
            "AtkinsonHyperlegible",
            &include_bytes!("../fonts/atkinsonhyperlegible/AtkinsonHyperlegible-Regular.ttf")[..],
        ),
        (
            "AtkinsonHyperlegibleBold",
            &include_bytes!("../fonts/atkinsonhyperlegible/AtkinsonHyperlegible-Bold.ttf")[..],
        ),
        (
            "IosevkaFixed",
            &include_bytes!("../fonts/iosevka/IosevkaFixed-Regular.ttf")[..],
        ),
    ] {
        fonts
            .font_data
            .insert(name.to_owned(), std::sync::Arc::new(FontData::from_static(bytes)));
    }

    // This runs inside the plugin's editor-open path, where a panic takes
    // the host down: if an egui upgrade ever drops the default families,
    // degrade to egui's bundled fonts instead of crashing the DAW.
    if let Some(proportional) = fonts.families.get_mut(&FontFamily::Proportional) {
        proportional.insert(0, "AtkinsonHyperlegible".to_owned());
        // Per-glyph fallback for symbols Atkinson lacks — notably the music
        // accidentals (Iosevka's subset keeps U+266D-266F).
        proportional.push("IosevkaFixed".to_owned());
        // Headings: the Bold face first, then the regular proportional stack
        // as fallback for any glyph Bold lacks.
        let mut heading = proportional.clone();
        heading.insert(0, "AtkinsonHyperlegibleBold".to_owned());
        fonts
            .families
            .insert(FontFamily::Name(HEADING_FAMILY.into()), heading);
    }
    if let Some(monospace) = fonts.families.get_mut(&FontFamily::Monospace) {
        monospace.insert(0, "IosevkaFixed".to_owned());
    }
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
    // Type roles (families come from install_fonts). In the initializer rather
    // than assigned after it because clippy reads the assign-after form as a
    // `Default` that meant to be a struct literal, and it is right.
    let mut style = egui::Style {
        text_styles: [
            // Headings differentiate by WEIGHT (Atkinson Bold), not size.
            (
                TextStyle::Heading,
                FontId::new(13.5, egui::FontFamily::Name(HEADING_FAMILY.into())),
            ),
            (TextStyle::Body, FontId::proportional(13.5)),
            (TextStyle::Button, FontId::proportional(13.5)),
            (TextStyle::Small, FontId::proportional(11.0)),
            (TextStyle::Monospace, FontId::monospace(12.0)),
        ]
        .into(),
        ..Default::default()
    };

    // Air between elements, but compact controls: interact_size drives
    // the height of sliders/DragValue boxes and was reading oversized next
    // to 13px text.
    style.spacing.item_spacing = Vec2::new(8.0, 7.0);
    style.spacing.button_padding = Vec2::new(9.0, 4.0);
    style.spacing.interact_size = Vec2::new(36.0, 17.0);
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
    visuals.selection.stroke = Stroke::new(1.0, text());
    visuals.slider_trailing_fill = true;
    visuals.window_corner_radius = CornerRadius::same(8);
    visuals.menu_corner_radius = CornerRadius::same(6);

    // Widget states: flat fills, rounded corners, strokes only where they
    // carry information (hover/focus). Every state is the same SIZE — hover
    // and press are read off the fill and the border, which cost no space.
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
    w.noninteractive.bg_stroke = Stroke::new(1.0, hairline());
    w.noninteractive.fg_stroke = Stroke::new(1.0, text_dim());
    w.noninteractive.corner_radius = WIDGET_RADIUS;

    w.inactive.bg_fill = widget();
    w.inactive.weak_bg_fill = widget();
    w.inactive.bg_stroke = Stroke::NONE;
    w.inactive.fg_stroke = Stroke::new(1.0, text());
    w.inactive.corner_radius = WIDGET_RADIUS;

    w.hovered.bg_fill = widget_hover();
    w.hovered.weak_bg_fill = widget_hover();
    w.hovered.bg_stroke = Stroke::new(1.0, accent_edge());
    w.hovered.fg_stroke = Stroke::new(1.2, text());
    w.hovered.corner_radius = WIDGET_RADIUS;

    w.active.bg_fill = accent_active();
    w.active.weak_bg_fill = accent_active();
    w.active.bg_stroke = Stroke::new(1.0, accent());
    w.active.fg_stroke = Stroke::new(1.2, Color32::WHITE);
    w.active.corner_radius = WIDGET_RADIUS;

    w.open.bg_fill = widget();
    w.open.weak_bg_fill = widget();
    w.open.bg_stroke = Stroke::new(1.0, accent_edge());
    w.open.fg_stroke = Stroke::new(1.0, text());
    w.open.corner_radius = WIDGET_RADIUS;

    scale_chrome(&mut style, scale);
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
    for v in [
        &mut s.item_spacing,
        &mut s.button_padding,
        &mut s.interact_size,
        &mut s.default_area_size,
    ] {
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
        // pane is the one piece that would give the game away.
        &mut s.scroll.bar_width,
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
    if value > 0 { scaled.max(1.0) as u8 } else { 0 }
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

/// The dock chrome, derived from the egui style plus our own tweaks.
///
/// The goal is ONE surface: panes share the panel color, separated by thin
/// dark lines, with no outlined boxes around bodies or tabs. Buttons that
/// add noise (per-tab close, collapse arrows) are disabled on the DockArea
/// itself in `root_ui`.
///
/// `scale` is the [chrome scale](ui_scale) the caller is drawing at. It has to
/// be passed rather than read off `egui_style`, which carries sizes already
/// multiplied by it but no record of the factor itself.
pub fn dock_style(egui_style: &egui::Style, scale: f32) -> egui_dock::Style {
    let mut style = egui_dock::Style::from_egui(egui_style);

    // No gap between the dock and the window edge, and no outer border.
    style.dock_area_padding = None;
    style.main_surface_border_stroke = Stroke::NONE;

    // Separators: slim bands in the well color, accent when grabbed.
    style.separator.width = 4.0 * scale;
    // Not scaled: how near a separator the pointer has to come to take hold of
    // it is a reach, not a drawn thing, and a narrower band is if anything the
    // case for keeping the reach where it was.
    style.separator.extra_interact_width = 6.0;
    // The narrowest a pane may be dragged, which is its own tab bar: the width
    // a fold leaves behind, so a pane can be dragged down to the rail it would
    // fold to and no further.
    //
    // egui_dock's own default is 175pt, and it applies this as a clamp on EVERY
    // separator's fraction on EVERY frame, dragged or not — so in a window
    // narrow enough for 175 to bite (the plugin's floor is 400) it does not
    // merely refuse a drag, it walks the layout toward 50/50 by itself, and it
    // mangles any fraction dialled for a window that has not arrived yet: the
    // fractions a fold hands back on the way out are exactly that. A tab bar
    // clears every fold's own fractions with a couple of points to spare, a
    // rail being one tab bar wide by construction.
    style.separator.extra = min_pane(scale);
    style.separator.color_idle = well();
    style.separator.color_hovered = accent_edge();
    style.separator.color_dragged = accent();

    // Tab bar: a quiet strip of the same surface, divided from the body by
    // a hairline; the active tab fills seamlessly into the body below it.
    style.tab_bar.bg_fill = well();
    style.tab_bar.height = tab_bar_height(scale);
    style.tab_bar.hline_color = well();
    style.tab_bar.corner_radius = CornerRadius::ZERO;

    // Tabs: no outlines anywhere; active = body color, inactive = recessed.
    let tab = &mut style.tab;
    for t in [
        &mut tab.active,
        &mut tab.focused,
        &mut tab.active_with_kb_focus,
        &mut tab.focused_with_kb_focus,
    ] {
        t.outline_color = Color32::TRANSPARENT;
        t.corner_radius = CornerRadius::ZERO;
        t.bg_fill = panel();
        t.text_color = text();
    }
    for t in [&mut tab.inactive, &mut tab.hovered, &mut tab.inactive_with_kb_focus] {
        t.outline_color = Color32::TRANSPARENT;
        t.corner_radius = CornerRadius::ZERO;
        t.bg_fill = well();
        t.text_color = text_dim();
    }
    tab.hovered.bg_fill = surface_faint();
    tab.hovered.text_color = text();
    tab.hline_below_active_tab_name = false;

    // Tab bodies: the boxes-within-boxes look came from here — a stroke
    // rectangle around every pane. Kill it; bodies are just the surface.
    tab.tab_body.stroke = Stroke::NONE;
    tab.tab_body.corner_radius = CornerRadius::ZERO;
    tab.tab_body.bg_fill = panel();
    tab.tab_body.inner_margin = egui::Margin::same(pane_inner_margin(scale) as i8);

    style
}
