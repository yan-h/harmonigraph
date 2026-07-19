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
    /// Idle nodes draw no disc; this colors their hover ghost.
    pub node_idle: Vec4,
    /// The faint background grid between node positions; alpha is the
    /// line opacity. Only the DEFAULT for `ViewConfig::grid_color`, which
    /// is what the scene actually reads.
    pub grid_line: Vec4,
    /// Melody mark: the highest held note (see
    /// `ViewConfig::highlight_extremes`). Warm, and deliberately far from
    /// `note_bass` in hue so the two ends never read as the same mark.
    pub note_melody: Vec4,
    /// Bass mark: the lowest held note. Cool, the counterpart to
    /// `note_melody`.
    pub note_bass: Vec4,

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
            node_idle: Vec4::new(0.27, 0.29, 0.34, 1.0),
            grid_line: Vec4::new(0.27, 0.29, 0.34, 0.62),
            note_melody: Vec4::new(1.0, 0.84, 0.36, 1.0),
            note_bass: Vec4::new(0.42, 0.82, 1.0, 1.0),
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
