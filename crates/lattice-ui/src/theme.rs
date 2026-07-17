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

const WIDGET_RADIUS: CornerRadius = CornerRadius::same(5);

// ---- UI font ---------------------------------------------------------------

/// Candidate UI faces, embedded for side-by-side comparison (switcher in
/// the View section). All OFL/UFL licensed; license texts ship next to the
/// TTFs in `fonts/`. Losers get deleted once one wins.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default, serde::Serialize, serde::Deserialize)]
pub enum UiFont {
    /// egui's bundled default.
    #[default]
    UbuntuLight,
    Inter,
    IbmPlexSans,
    FiraSans,
    SourceSans3,
    AtkinsonHyperlegible,
}

impl UiFont {
    pub const ALL: [UiFont; 6] = [
        UiFont::UbuntuLight,
        UiFont::Inter,
        UiFont::IbmPlexSans,
        UiFont::FiraSans,
        UiFont::SourceSans3,
        UiFont::AtkinsonHyperlegible,
    ];

    pub fn label(self) -> &'static str {
        match self {
            UiFont::UbuntuLight => "Ubuntu",
            UiFont::Inter => "Inter",
            UiFont::IbmPlexSans => "Plex",
            UiFont::FiraSans => "Fira",
            UiFont::SourceSans3 => "Source",
            UiFont::AtkinsonHyperlegible => "Atkinson",
        }
    }

    /// The font-data key registered with egui (None = egui's default).
    fn font_name(self) -> Option<&'static str> {
        match self {
            UiFont::UbuntuLight => None,
            UiFont::Inter => Some("Inter"),
            UiFont::IbmPlexSans => Some("IBMPlexSans"),
            UiFont::FiraSans => Some("FiraSans"),
            UiFont::SourceSans3 => Some("SourceSans3"),
            UiFont::AtkinsonHyperlegible => Some("AtkinsonHyperlegible"),
        }
    }
}

/// Install the chosen proportional face. All candidates are registered;
/// the chosen one is prepended to the Proportional family so every
/// proportional text role uses it (fallbacks, including emoji and the
/// default face, stay in the list). Monospace (the console) keeps Hack.
/// Cheap enough to call whenever the selection changes.
pub fn apply_font(ctx: &egui::Context, font: UiFont) {
    use egui::epaint::text::{FontData, FontDefinitions, FontFamily};

    let mut fonts = FontDefinitions::default();
    for (name, bytes) in [
        ("Inter", &include_bytes!("../fonts/inter/Inter-Regular.ttf")[..]),
        ("IBMPlexSans", &include_bytes!("../fonts/ibmplexsans/IBMPlexSans-Regular.ttf")[..]),
        ("FiraSans", &include_bytes!("../fonts/firasans/FiraSans-Regular.ttf")[..]),
        ("SourceSans3", &include_bytes!("../fonts/sourcesans3/SourceSans3-Regular.ttf")[..]),
        (
            "AtkinsonHyperlegible",
            &include_bytes!("../fonts/atkinsonhyperlegible/AtkinsonHyperlegible-Regular.ttf")[..],
        ),
    ] {
        fonts
            .font_data
            .insert(name.to_owned(), std::sync::Arc::new(FontData::from_static(bytes)));
    }
    if let Some(name) = font.font_name() {
        fonts
            .families
            .get_mut(&FontFamily::Proportional)
            .expect("proportional family exists")
            .insert(0, name.to_owned());
    }
    ctx.set_fonts(fonts);
}

// ---- Application ----------------------------------------------------------

/// Apply the theme to a context. Each shell calls this once at startup
/// (eframe's creation context; the plugin editor's build closure).
pub fn apply_theme(ctx: &egui::Context) {
    // The lattice is a dark-background instrument; pin the UI to dark
    // rather than following the host/system preference.
    ctx.set_theme(egui::ThemePreference::Dark);
    let mut style = (*ctx.style_of(egui::Theme::Dark)).clone();

    // Type roles. Default fonts for now; embedding a custom face via
    // ctx.set_fonts() slots in here later.
    style.text_styles = [
        (TextStyle::Heading, FontId::proportional(17.0)),
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
    visuals.faint_bg_color = Color32::from_rgb(30, 31, 36);

    // Text selection keeps alpha (it overlays glyphs); widget states below
    // are opaque.
    visuals.selection.bg_fill = accent().gamma_multiply(0.35);
    visuals.selection.stroke = Stroke::new(1.0, accent());
    visuals.slider_trailing_fill = true;
    visuals.window_corner_radius = CornerRadius::same(8);
    visuals.menu_corner_radius = CornerRadius::same(6);

    // Widget states: flat fills, rounded corners, strokes only where they
    // carry information (hover/focus), slight hover growth.
    let w = &mut visuals.widgets;

    w.noninteractive.bg_fill = panel();
    w.noninteractive.bg_stroke = Stroke::new(1.0, Color32::from_rgb(40, 42, 48));
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
    tab.hovered.bg_fill = Color32::from_rgb(30, 31, 36);
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
