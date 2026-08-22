//! The skin: one struct owning every color the CHROME draws, so a look is
//! defined in exactly one place. `harmonigraph-ui::theme` converts the bytes
//! into egui colors for the panel chrome; the scene reaches them through
//! [`well_color`], the ground it is composited over, and through
//! [`surface_faint_color`], the rung a fresh
//! [`ViewConfig::lattice_ground`](crate::ViewConfig) opens on.
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
    // ---- UI chrome (sRGB bytes; converted to egui colors in theme) ----
    // CHROME only. What the lattice is drawn AT REST in — its dots, both of
    // a node's rings where nothing is lit — is a view setting rather than a
    // skin color (`ViewConfig::lattice_ground`), because it is a thing to be
    // dialled while a picture is being read. The skin's part in it is
    // `surface_faint`, the rung that setting opens on.
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
    /// resting dots brighten to match. See git history for the pre-pass
    /// values.
    fn default() -> Self {
        Skin {
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

/// The active skin's `well`: the recessed ground every PICTURE pane paints
/// its own rect with — the spectral pane, the spiral, the render preview, and
/// the lattice — and so the default ground a lattice pass is composited over.
///
/// Not the `panel` the dock fills a tab body with, which is what a lattice
/// pane that paints no ground of its own shows through: a picture is recessed
/// below the chrome around it rather than flush with it, and the lattice being
/// the one picture at panel level made it read as a lighter card beside the
/// analyzer. The renderer's own default frame background is a shade BELOW this
/// one — `Layout`'s `background`, the window colour, at (14, 14, 18) against
/// this (15, 16, 19) — and an export stands the lattice on that, because a
/// pane paints the ground its shell hands it rather than the skin's
/// (`the_pane_paints_the_shells_ground_rather_than_the_skins`). The two are a
/// step apart on purpose: one is the colour a picture is recessed into, the
/// other the colour a frame is matted with.
pub fn well_color() -> Vec4 {
    let [r, g, b] = active_skin().well;
    ground_color((r, g, b))
}

/// The active skin's `surface_faint`: the grey a subtly raised surface of the
/// chrome sits at, and the rung [`ViewConfig::lattice_ground`](crate::ViewConfig)
/// opens on — what the whole lattice is drawn in at rest: its dots, and both of
/// a node's rings where nothing is lit.
///
/// Which rung of the ladder is the whole of what that default is, and the
/// ladder is short: [`well_color`] — the ground the lattice is composited
/// OVER — at `L*` 4.7, the chrome's panel at 8.8, and this at 20.0. The
/// lattice's ground has to sit well ABOVE the ground it stands on, so that a
/// quiet ring and a resting dot read as faintly raised structure rather
/// than as marks on the surface. The well IS the pane exactly, and structure
/// standing there vanishes into it — which is what a ring dialled to no width
/// looks like, the off switch
/// ([`ViewConfig::spectral_ring_width`](crate::ViewConfig)). The panel a single
/// rung up is barely clear of it, near enough that the structure reads as a
/// smudge on the ground rather than as a surface above it.
///
/// A step DOWN from the chrome's own [`hairline`](Skin::hairline), which is
/// the other grey the lattice could rule itself with — the argument for that
/// one being that the picture and the panel around it then cannot drift. What
/// it costs is the whole reason this is the rung instead: it holds the
/// lattice's structure to a CHROME decision, so the dots answer to the settings
/// pane's dividers rather than to the rings standing over them, and no bar can
/// move any of it. The rings are on the same sheet as the dots; the dividers
/// are not.
///
/// A DEFAULT and not the value itself: the bar reaches the whole `L*` axis, and
/// `the_fresh_ground_is_the_skins_faint_surface` is what keeps this rung and
/// that default the same number.
pub fn surface_faint_color() -> Vec4 {
    let [r, g, b] = active_skin().surface_faint;
    ground_color((r, g, b))
}
