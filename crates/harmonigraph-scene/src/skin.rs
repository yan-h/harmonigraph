//! The skin: one struct owning every color the product draws, both the 3D
//! scene's and the UI chrome's, so a look is defined in exactly one place.
//! `harmonigraph-ui::theme` converts the chrome bytes into egui colors; the
//! scene reads its colors directly.
//!
//! Only one built-in skin exists so far (the original dark look). Adding a
//! skin = another `Skin` value plus a way to select it (a `set_skin`
//! existed briefly and was removed as unused — see git history; live
//! re-skinning needs the theme re-applied and is future work, as are
//! shader-side skin uniforms like glow strength).

use std::sync::OnceLock;

use glam::Vec4;

#[derive(Clone, Debug)]
pub struct Skin {
    // ---- 3D scene ----
    /// The faint background grid between node positions; alpha is the
    /// line opacity. Only the DEFAULT for `ViewConfig::grid_color`, which
    /// is what the scene actually reads.
    pub grid_line: Vec4,

    // ---- UI chrome (sRGB bytes; converted to egui colors in theme) ----
    /// Window/panel background.
    pub panel: [u8; 3],
    /// Recessed areas: console scrollback, tab bar, meters.
    pub well: [u8; 3],
    /// Subtly raised surface between panel and widget: hovered tabs,
    /// faint striping.
    pub surface_faint: [u8; 3],
    /// Hairline strokes around noninteractive chrome.
    pub hairline: [u8; 3],
    /// Resting widget fill.
    pub widget: [u8; 3],
    /// Hovered widget fill.
    pub widget_hover: [u8; 3],
    /// Accent hue for selections and focus.
    pub accent: [u8; 3],
    /// Primary text.
    pub text: [u8; 3],
    /// Secondary text.
    pub text_dim: [u8; 3],
    /// ValueBar fill at rest / hovered / dragging (opaque accent mixes).
    pub accent_fill: [u8; 3],
    pub accent_fill_hover: [u8; 3],
    pub accent_fill_drag: [u8; 3],
    /// Pressed/active widget fill.
    pub accent_active: [u8; 3],
    /// Hover/focus stroke color.
    pub accent_edge: [u8; 3],
    /// Armed-mode indicator (learn mode): amber, distinct from both the
    /// accent and the warning red.
    pub armed: [u8; 3],
    /// Warning text (e.g. notes not represented on the lattice).
    pub warning_text: [u8; 3],
    /// Background band behind warning rows.
    pub warning_bg: [u8; 3],
}

impl Default for Skin {
    /// The dark look. Backgrounds (`panel`/`well`) stay at the original deep
    /// values — the instrument reads as dark on purpose — but the whole
    /// foreground/structure band above them was collapsed into a near-
    /// invisible cluster (widget vs panel 1.24, hairline 1.22, slider fill
    /// vs track 2.01, surface_faint 1.07). This pass lifts that band so the
    /// chrome is legible: dividers, resting buttons, hovered surfaces, and
    /// slider fills now separate clearly from the background, and secondary
    /// text (labels, inactive tabs, console, disclosure arrows — all routed
    /// through `text_dim`) rises from ~5.5:1 to ~8:1. Idle nodes and the
    /// grid brighten to match. See git history for the pre-pass values.
    fn default() -> Self {
        Skin {
            grid_line: Vec4::new(0.27, 0.29, 0.34, 0.62),
            panel: [24, 25, 29],
            well: [15, 16, 19],
            surface_faint: [46, 48, 57],
            hairline: [64, 67, 77],
            widget: [62, 66, 77],
            widget_hover: [84, 88, 102],
            accent: [124, 156, 216],
            text: [228, 230, 234],
            text_dim: [172, 177, 188],
            accent_fill: [76, 95, 132],
            accent_fill_hover: [98, 122, 168],
            accent_fill_drag: [120, 150, 206],
            accent_active: [100, 124, 172],
            accent_edge: [130, 160, 216],
            armed: [238, 178, 92],
            warning_text: [236, 142, 132],
            warning_bg: [64, 33, 31],
        }
    }
}

static ACTIVE: OnceLock<Skin> = OnceLock::new();

/// The active skin (currently always [`Skin::default`]).
pub fn active_skin() -> &'static Skin {
    ACTIVE.get_or_init(Skin::default)
}

/// An sRGB byte triple as the opaque RGBA vector the renderer wants.
///
/// A straight divide by 255, deliberately, not a gamma decode: the shader's
/// colors are sRGB-encoded 0..1 throughout (the offscreen target is a plain
/// `Unorm` format, so nothing converts on the way through), which is the
/// same arithmetic `harmonigraph-ui::theme` does to reach an egui `Color32`.
/// Shells use this to hand the scene the ground they composite it over.
pub fn ground_color(rgb: (u8, u8, u8)) -> Vec4 {
    Vec4::new(
        f32::from(rgb.0) / 255.0,
        f32::from(rgb.1) / 255.0,
        f32::from(rgb.2) / 255.0,
        1.0,
    )
}

/// The active skin's `panel`: the ground `egui_dock` paints under every tab
/// body, and so the default ground for a pane drawn inside the dock.
pub fn panel_color() -> Vec4 {
    let [r, g, b] = active_skin().panel;
    ground_color((r, g, b))
}
