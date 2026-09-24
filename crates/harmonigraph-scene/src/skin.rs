//! The skin: one struct owning every color the CHROME draws, so a look is
//! defined in exactly one place. `harmonigraph-ui::theme` converts the bytes
//! into egui colors for the panel chrome.
//!
//! The PICTURE's own colors are deliberately not in it: the ground, rulings,
//! axis numbers, note names and the off-scale band are the `PICTURE*`
//! constants below. They land in every exported frame, and the offline
//! renderer is never told which skin the editor wears, so a skinnable picture
//! color would make a video disagree with the editor it was set up in.
//!
//! The selectable skins are [`skins`]: the original dark look first, then
//! base16 schemes run through [`Skin::from_base16`]. Which one is in force is
//! PER THREAD ([`set_active_skin`]) rather than process-wide, because a host
//! can load several plugin instances into one process and each editor wears
//! its own; each sets its skin at the top of its frame, and frames do not
//! interleave on a thread. It also keeps parallel tests from reskinning one
//! another.

use std::cell::Cell;
use std::sync::OnceLock;

use glam::Vec4;

#[derive(Clone, Debug)]
pub struct Skin {
    // ---- UI chrome (sRGB bytes; converted to egui colors in theme) ----
    // CHROME only. What the lattice is drawn AT REST in — its markers, both of
    // a node's rings where nothing is lit — is a view setting rather than a
    // skin color (`ViewConfig::lattice_ground`), because it is a thing to be
    // dialled while a picture is being read.
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
    /// Hover accent: a lit divider, a hovered switch track.
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
    /// resting markers brighten to match. See git history for the pre-pass
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

impl Skin {
    /// A skin from a base16 scheme's sixteen slots (`0xRRGGBB`, `base00`
    /// first), mapped the way the scheme's own roles suggest: `base00` the
    /// pane, `base01` the tracks and wells, `base02` buttons, rules and
    /// hovered rows, `base04`/`base05` secondary and primary text, `base0D`
    /// the accent, `base0A` learn, `base08` warnings. The accent fills are
    /// mixes of the accent into the surface they sit on, so they stay opaque
    /// (see `theme::accent_fill`).
    ///
    /// Where `base01` is all but the pane colour (under 1.06:1, as several
    /// schemes ship it) the track is lifted most of the way to `base02`,
    /// or the bars would have no visible track at all.
    pub fn from_base16(slots: [u32; 16]) -> Skin {
        let b = slots.map(|v| [(v >> 16) as u8, (v >> 8) as u8, v as u8]);
        let mut track = b[1];
        if contrast(track, b[0]) < 1.06 {
            track = mix(b[0], b[2], 0.6);
        }
        let widget = b[2];
        let accent = b[13];
        Skin {
            panel: b[0],
            well: track,
            surface_faint: b[2],
            hairline: b[2],
            widget,
            widget_hover: mix(b[2], b[3], 0.35),
            accent,
            text: b[5],
            text_dim: b[4],
            accent_fill: mix(track, accent, 0.42),
            accent_fill_hover: mix(track, accent, 0.58),
            accent_fill_drag: mix(track, accent, 0.78),
            accent_active: mix(widget, accent, 0.55),
            accent_edge: mix(accent, [255, 255, 255], 0.06),
            armed: b[10],
            warning_text: b[8],
            warning_bg: mix(b[0], b[8], 0.2),
        }
    }
}

/// `a` moved `t` of the way to `b`, per sRGB byte.
fn mix(a: [u8; 3], b: [u8; 3], t: f32) -> [u8; 3] {
    std::array::from_fn(|i| {
        (f32::from(a[i]) + (f32::from(b[i]) - f32::from(a[i])) * t).round() as u8
    })
}

/// WCAG relative luminance of an sRGB byte triple.
fn luminance(c: [u8; 3]) -> f32 {
    let lin = |v: u8| {
        let v = f32::from(v) / 255.0;
        if v <= 0.04045 {
            v / 12.92
        } else {
            ((v + 0.055) / 1.055).powf(2.4)
        }
    };
    0.2126 * lin(c[0]) + 0.7152 * lin(c[1]) + 0.0722 * lin(c[2])
}

/// WCAG contrast ratio between two sRGB byte triples, 1.0 to 21.0.
pub fn contrast(a: [u8; 3], b: [u8; 3]) -> f32 {
    let (x, y) = (luminance(a), luminance(b));
    (x.max(y) + 0.05) / (x.min(y) + 0.05)
}

/// One entry in the picker.
#[derive(Debug)]
pub struct SkinEntry {
    /// What a saved editor state names it by. Stable: renaming one sends
    /// every blob that chose it back to the default.
    pub id: &'static str,
    /// What the picker shows.
    pub name: &'static str,
    pub skin: Skin,
}

/// The id of [`Skin::default`], first in [`skins`].
pub const DEFAULT_SKIN: &str = "default";

/// Base16 schemes offered beside the default: id, name, `base00`..`base0F`.
/// Values from tinted-theming/schemes (`base16/<id>.yaml`); the eight were
/// picked from the whole dark set drawn on a mock of the settings pane.
const BASE16: [(&str, &str, [u32; 16]); 8] = [
    (
        "berlin",
        "Berlin",
        [
            0x000000, 0x0e0e0e, 0x181818, 0x333333, 0x707070, 0xcccccc, 0xd6d6d6, 0xffffff,
            0x999999, 0xbbbbbb, 0xdddddd, 0xbbbbbb, 0xcccccc, 0x888888, 0xaaaaaa, 0x7a7a7a,
        ],
    ),
    (
        "corduroy-dark",
        "Corduroy Dark",
        [
            0x141016, 0x1b151e, 0x221a26, 0x7d7082, 0x9a8d9e, 0xddd8df, 0xddd8df, 0x5a5160,
            0xe8758a, 0xf0a89b, 0xf0bd9c, 0x55a0a0, 0xf0a89b, 0xdc92a3, 0xcf98c4, 0x9a8d9e,
        ],
    ),
    (
        "kissa-macchiato",
        "Kissa Macchiato",
        [
            0x1f1c16, 0x35322d, 0x47443f, 0xb8a48c, 0xd4c4a8, 0xfaf0e6, 0xe8d5b7, 0xfef4e4,
            0xe87777, 0xda9050, 0xeac67a, 0x8cb870, 0x6ab8b0, 0x7fa8d4, 0xb094cc, 0xcc88aa,
        ],
    ),
    (
        "outrun-dark",
        "Outrun Dark",
        [
            0x00002a, 0x20204a, 0x30305a, 0x50507a, 0xb0b0da, 0xd0d0fa, 0xe0e0ff, 0xf5f5ff,
            0xff4242, 0xfc8d28, 0xf3e877, 0x59f176, 0x0ef0f0, 0x66b0ff, 0xf10596, 0xf003ef,
        ],
    ),
    (
        "soft-server",
        "Soft Server",
        [
            0x211e2a, 0x2c2737, 0x3f3951, 0x6e6780, 0x8a829e, 0xe4dee9, 0xf2e8f0, 0xffffff,
            0xe965a5, 0xf4b870, 0xebde76, 0xb1f2a7, 0xb3f4f3, 0x95a6f4, 0xff79c6, 0xbd93f9,
        ],
    ),
    (
        "spaceduck",
        "Spaceduck",
        [
            0x16172d, 0x1b1c36, 0x30365f, 0x686f9a, 0x818596, 0xecf0c1, 0xc1c3cc, 0xffffff,
            0xe33400, 0xe39400, 0xf2ce00, 0x5ccc96, 0x00a3cc, 0x7a5ccc, 0xb3a1e6, 0xce6f8f,
        ],
    ),
    (
        "tinta",
        "Tinta",
        [
            0x101012, 0x202023, 0x2c2c30, 0x62626a, 0x9d9c9d, 0xd8d6d0, 0xe3e1db, 0xeeece6,
            0xd0726a, 0xe8843a, 0xc8b86a, 0x9aa890, 0x80b8b4, 0x8a9ab0, 0xb0a0b8, 0x4a4a50,
        ],
    ),
    (
        "tokyo-city-dark",
        "Tokyo City Dark",
        [
            0x171d23, 0x1d252c, 0x28323a, 0x526270, 0xb7c5d3, 0xd8e2ec, 0xf6f6f8, 0xfbfbfd,
            0xf7768e, 0xff9e64, 0xb7c5d3, 0x9ece6a, 0x89ddff, 0x7aa2f7, 0xbb9af7, 0xbb9af7,
        ],
    ),
];

/// Every selectable skin, [`DEFAULT_SKIN`] first.
pub fn skins() -> &'static [SkinEntry] {
    static SKINS: OnceLock<Vec<SkinEntry>> = OnceLock::new();
    SKINS.get_or_init(|| {
        std::iter::once(SkinEntry { id: DEFAULT_SKIN, name: "Default", skin: Skin::default() })
            .chain(BASE16.iter().map(|&(id, name, slots)| SkinEntry {
                id,
                name,
                skin: Skin::from_base16(slots),
            }))
            .collect()
    })
}

/// Where the skin saved as `id` sits in [`skins`], if it is one.
pub fn skin_index(id: &str) -> Option<usize> {
    skins().iter().position(|entry| entry.id == id)
}

thread_local! {
    static ACTIVE: Cell<usize> = const { Cell::new(0) };
}

/// Put `skins()[index]` in force on this thread; an index past the end is
/// the default. See the module doc for why this is per thread.
pub fn set_active_skin(index: usize) {
    ACTIVE.set(if index < skins().len() { index } else { 0 });
}

/// The index [`active_skin`] reads, for a context to remember which skin its
/// style was built in.
pub fn active_skin_index() -> usize {
    ACTIVE.get()
}

/// The skin in force on this thread; [`Skin::default`] until one is set.
pub fn active_skin() -> &'static Skin {
    &skins()[ACTIVE.get()].skin
}

/// An sRGB byte triple as the opaque RGBA vector the renderer wants.
///
/// A straight divide by 255, deliberately, not a gamma decode: the shader's
/// colors are sRGB-encoded 0..1 throughout (the offscreen target is a plain
/// `Unorm` format, so nothing converts on the way through), which is the
/// same arithmetic `harmonigraph-ui::theme` does to reach an egui `Color32`.
/// Shells use this to hand the scene the ground they composite it over.
pub fn ground_color(rgb: (u8, u8, u8)) -> Vec4 {
    Vec4::new(f32::from(rgb.0) / 255.0, f32::from(rgb.1) / 255.0, f32::from(rgb.2) / 255.0, 1.0)
}

// ---- Picture colors ----------------------------------------------------------
// Fixed rather than skinned (see the module doc). Tuned against the black
// ground they are drawn on, not against the chrome; the values are the ones
// the default skin's chrome roles carried when these were split off, so the
// split moved no pixel.

/// The ground every PICTURE pane is bedded on (see [`picture_color`]).
pub const PICTURE: [u8; 3] = [0, 0, 0];
/// Rulings drawn across a picture: the analyzer's frequency and level grid,
/// the spectrum/roll handover, the spiral's seam and pitch-class rays. Each
/// site fades it further.
pub const PICTURE_RULING: [u8; 3] = [64, 67, 77];
/// The analyzer's axis numbers.
pub const PICTURE_MARKING: [u8; 3] = [172, 177, 188];
/// Note names drawn in a picture, outlined in [`PICTURE`].
pub const PICTURE_NAME: [u8; 3] = [228, 230, 234];
/// The band behind a sounding note the scale does not contain.
pub const PICTURE_OFF_SCALE: [u8; 3] = [236, 142, 132];

/// [`PICTURE`] as the renderer wants it: the ground every PICTURE pane paints its own
/// rect with — the spectral pane, the spiral, the render preview, and the
/// lattice — and so the default ground a lattice pass is composited over.
///
/// BLACK, because the spectrogram's plane is black and has to be (a cell at
/// silence that is not black shows the plane's edge), and every other picture
/// stands beside it. On any other ground the analyzer's curve and the lattice
/// read as a grey card with the spectrogram cut out of it as a hole — faint in
/// the editor, where grey chrome surrounds it, and plain in a video, where the
/// player around the frame is black too. A picture is still recessed below
/// the chrome, only further: this is not the `well` the chrome's own tracks
/// and meters sit in.
///
/// `Layout`'s default `background` is this same colour, so a margin or a gap
/// in an export reads as the panes' own ground; an export stands the lattice
/// on the layout's colour rather than on this one, because a pane paints the
/// ground its shell hands it rather than this one
/// (`the_pane_paints_the_shells_ground_rather_than_the_skins`).
pub fn picture_color() -> Vec4 {
    let [r, g, b] = PICTURE;
    ground_color((r, g, b))
}
