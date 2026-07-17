//! The UI theme: one place to tweak how panel chrome looks.
//!
//! Colors deliberately stay close to egui's dark defaults (the current
//! scheme); what this module changes is the *shape* of things — rounding,
//! spacing, strokes, hover behavior — plus named constants so palette
//! experiments are one-file edits.
//!
//! Keep in loose sync with the lattice's own colors in `lattice-scene`
//! (IDLE_COLOR, channel palette). TODO(skins): fold both into a single
//! `Skin` struct feeding egui visuals AND shader uniforms.

use egui::{Color32, CornerRadius, FontId, Stroke, TextStyle, Vec2};

// ---- Palette -------------------------------------------------------------

/// Window/panel background.
pub const PANEL: Color32 = Color32::from_rgb(24, 25, 29);
/// Recessed areas: console scrollback, text edits, plot backgrounds.
pub const WELL: Color32 = Color32::from_rgb(15, 16, 19);
/// Resting widget fill (buttons, slider tracks).
pub const WIDGET: Color32 = Color32::from_rgb(41, 43, 50);
/// Hovered widget fill.
pub const WIDGET_HOVER: Color32 = Color32::from_rgb(54, 57, 66);
/// Accent: selections, slider fill, hover strokes. A desaturated cousin of
/// the channel-0 lattice red would also work here; blue-gray for now.
pub const ACCENT: Color32 = Color32::from_rgb(110, 140, 200);
/// Primary text.
pub const TEXT: Color32 = Color32::from_rgb(222, 224, 228);
/// Secondary text (labels, weak()).
pub const TEXT_DIM: Color32 = Color32::from_rgb(140, 144, 152);

const WIDGET_RADIUS: CornerRadius = CornerRadius::same(5);

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

    // Air. egui's defaults are tuned for dense tooling UIs.
    style.spacing.item_spacing = Vec2::new(8.0, 7.0);
    style.spacing.button_padding = Vec2::new(10.0, 5.0);
    style.spacing.interact_size = Vec2::new(40.0, 22.0);
    style.spacing.slider_width = 160.0;

    let visuals = &mut style.visuals;
    visuals.panel_fill = PANEL;
    visuals.window_fill = PANEL;
    visuals.extreme_bg_color = WELL;
    visuals.faint_bg_color = Color32::from_rgb(30, 31, 36);

    visuals.selection.bg_fill = ACCENT.gamma_multiply(0.35);
    visuals.selection.stroke = Stroke::new(1.0, ACCENT);
    visuals.slider_trailing_fill = true;
    visuals.window_corner_radius = CornerRadius::same(8);
    visuals.menu_corner_radius = CornerRadius::same(6);

    // Widget states: flat fills, rounded corners, strokes only where they
    // carry information (hover/focus), slight hover growth.
    let w = &mut visuals.widgets;

    w.noninteractive.bg_fill = PANEL;
    w.noninteractive.bg_stroke = Stroke::new(1.0, Color32::from_rgb(40, 42, 48));
    w.noninteractive.fg_stroke = Stroke::new(1.0, TEXT_DIM);
    w.noninteractive.corner_radius = WIDGET_RADIUS;

    w.inactive.bg_fill = WIDGET;
    w.inactive.weak_bg_fill = WIDGET;
    w.inactive.bg_stroke = Stroke::NONE;
    w.inactive.fg_stroke = Stroke::new(1.0, TEXT);
    w.inactive.corner_radius = WIDGET_RADIUS;

    w.hovered.bg_fill = WIDGET_HOVER;
    w.hovered.weak_bg_fill = WIDGET_HOVER;
    w.hovered.bg_stroke = Stroke::new(1.0, ACCENT.gamma_multiply(0.6));
    w.hovered.fg_stroke = Stroke::new(1.2, TEXT);
    w.hovered.corner_radius = WIDGET_RADIUS;
    w.hovered.expansion = 1.0;

    w.active.bg_fill = ACCENT.gamma_multiply(0.5);
    w.active.weak_bg_fill = ACCENT.gamma_multiply(0.5);
    w.active.bg_stroke = Stroke::new(1.0, ACCENT);
    w.active.fg_stroke = Stroke::new(1.2, Color32::WHITE);
    w.active.corner_radius = WIDGET_RADIUS;
    w.active.expansion = 1.0;

    w.open.bg_fill = WIDGET;
    w.open.weak_bg_fill = WIDGET;
    w.open.bg_stroke = Stroke::new(1.0, ACCENT.gamma_multiply(0.4));
    w.open.fg_stroke = Stroke::new(1.0, TEXT);
    w.open.corner_radius = WIDGET_RADIUS;

    ctx.set_style_of(egui::Theme::Dark, style);
}

/// The dock chrome, derived from the egui style plus our own tweaks.
pub fn dock_style(egui_style: &egui::Style) -> egui_dock::Style {
    let mut style = egui_dock::Style::from_egui(egui_style);
    style.tab_bar.bg_fill = WELL;
    style.separator.width = 2.0;
    style.separator.color_idle = Color32::from_rgb(34, 36, 42);
    style.separator.color_hovered = ACCENT.gamma_multiply(0.5);
    style.separator.color_dragged = ACCENT;
    style
}
