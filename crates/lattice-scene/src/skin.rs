//! The skin: one struct owning every color the product draws, both the 3D
//! scene's and the UI chrome's, so a look is defined in exactly one place.
//! `lattice-ui::theme` converts the chrome bytes into egui colors; the
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
    /// Fill color of idle (non-sounding) lattice nodes, linear-ish RGBA.
    pub node_idle: Vec4,

    // ---- UI chrome (sRGB bytes; converted to egui colors in theme) ----
    /// Window/panel background.
    pub panel: [u8; 3],
    /// Recessed areas: console scrollback, tab bar, meters.
    pub well: [u8; 3],
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
    /// The original dark look; values identical to the constants the theme
    /// and scene used before the skin mechanism existed.
    fn default() -> Self {
        Skin {
            node_idle: Vec4::new(0.16, 0.17, 0.20, 1.0),
            panel: [24, 25, 29],
            well: [15, 16, 19],
            widget: [41, 43, 50],
            widget_hover: [54, 57, 66],
            accent: [110, 140, 200],
            text: [222, 224, 228],
            text_dim: [140, 144, 152],
            accent_fill: [58, 70, 94],
            accent_fill_hover: [70, 86, 118],
            accent_fill_drag: [88, 109, 150],
            accent_active: [76, 92, 125],
            accent_edge: [90, 111, 152],
            armed: [235, 171, 82],
            warning_text: [232, 130, 120],
            warning_bg: [58, 30, 28],
        }
    }
}

static ACTIVE: OnceLock<Skin> = OnceLock::new();

/// The active skin (currently always [`Skin::default`]).
pub fn active_skin() -> &'static Skin {
    ACTIVE.get_or_init(Skin::default)
}
