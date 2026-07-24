//! The UI theme: how panel chrome looks. Every color comes from the
//! active `lattice_scene::skin::Skin`, which also feeds the 3D scene — a
//! look is defined in exactly one struct. This module owns the *shapes*:
//! rounding, spacing, strokes, hover behavior, and the egui/dock plumbing.

use egui::{Color32, CornerRadius, FontId, Stroke, TextStyle, Vec2};
use lattice_scene::skin::active_skin;

// ---- Palette accessors ----------------------------------------------------
// All colors come from the active Skin (lattice_scene::skin), the single
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
pub(crate) const CONTROL_RADIUS: u8 = 5;

const WIDGET_RADIUS: CornerRadius = CornerRadius::same(CONTROL_RADIUS);

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
    let mut style = (*ctx.style_of(egui::Theme::Dark)).clone();

    // Type roles (families come from install_fonts).
    style.text_styles = [
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
    .into();

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
    // fill = selection.bg_fill, text = selection.stroke color. The old
    // accent-blue-on-translucent-accent read as blue-on-blue (~3:1) and, now
    // that resting buttons are brighter, the 0.35 fill no longer separated
    // from an unselected one. A denser fill plus bright text makes the
    // selected state unmistakable while still reading fine as a text
    // highlight over glyphs.
    visuals.selection.bg_fill = accent().gamma_multiply(0.5);
    visuals.selection.stroke = Stroke::new(1.0, text());
    visuals.slider_trailing_fill = true;
    visuals.window_corner_radius = CornerRadius::same(8);
    visuals.menu_corner_radius = CornerRadius::same(6);

    // Widget states: flat fills, rounded corners, strokes only where they
    // carry information (hover/focus), slight hover growth.
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
    w.hovered.expansion = 1.0;

    w.active.bg_fill = accent_active();
    w.active.weak_bg_fill = accent_active();
    w.active.bg_stroke = Stroke::new(1.0, accent());
    w.active.fg_stroke = Stroke::new(1.2, Color32::WHITE);
    w.active.corner_radius = WIDGET_RADIUS;
    w.active.expansion = 1.0;

    w.open.bg_fill = widget();
    w.open.weak_bg_fill = widget();
    w.open.bg_stroke = Stroke::new(1.0, accent_edge());
    w.open.fg_stroke = Stroke::new(1.0, text());
    w.open.corner_radius = WIDGET_RADIUS;

    ctx.set_style_of(egui::Theme::Dark, style);
}

/// The dock chrome, derived from the egui style plus our own tweaks.
///
/// The goal is ONE surface: panes share the panel color, separated by thin
/// dark lines, with no outlined boxes around bodies or tabs. Buttons that
/// add noise (per-tab close, collapse arrows) are disabled on the DockArea
/// itself in `root_ui`.
pub fn dock_style(egui_style: &egui::Style) -> egui_dock::Style {
    let mut style = egui_dock::Style::from_egui(egui_style);

    // No gap between the dock and the window edge, and no outer border.
    style.dock_area_padding = None;
    style.main_surface_border_stroke = Stroke::NONE;

    // Separators: slim bands in the well color, accent when grabbed.
    style.separator.width = 4.0;
    style.separator.extra_interact_width = 6.0;
    style.separator.color_idle = well();
    style.separator.color_hovered = accent_edge();
    style.separator.color_dragged = accent();

    // Tab bar: a quiet strip of the same surface, divided from the body by
    // a hairline; the active tab fills seamlessly into the body below it.
    style.tab_bar.bg_fill = well();
    style.tab_bar.height = 26.0;
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
    tab.tab_body.inner_margin = egui::Margin::same(8);

    style
}
