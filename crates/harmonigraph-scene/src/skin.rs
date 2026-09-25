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
//! The selectable skins are [`skins`]: [`DEFAULT_SKIN`] first, then the
//! original dark look, then the other base16 schemes run through
//! [`Skin::from_base16`]. Which one is in force is
//! PER THREAD ([`set_active_skin`]) rather than process-wide, because a host
//! can load several plugin instances into one process and each editor wears
//! its own; each sets its skin at the top of its frame, and frames do not
//! interleave on a thread. It also keeps parallel tests from reskinning one
//! another.

use std::cell::Cell;
use std::ops::RangeInclusive;
use std::sync::OnceLock;

use glam::Vec4;

#[derive(Clone, Copy, Debug)]
pub struct Skin {
    // ---- UI chrome (sRGB bytes; converted to egui colors in theme) ----
    // CHROME only. What the lattice is drawn AT REST in — its markers, both of
    // a node's rings where nothing is lit — is a view setting rather than a
    // skin color (`ViewConfig::lattice_ground`), because it is a thing to be
    // dialled while a picture is being read.
    /// Window/panel background: the settings page, and every popup.
    pub panel: [u8; 3],
    /// Pane headers and folded rails, one step above `panel`.
    pub header: [u8; 3],
    /// Recessed areas: console scrollback, tracks, meters.
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

impl Skin {
    /// The original dark look, the default before Tinta. Backgrounds (`panel`/`well`) stay at the original deep
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
    pub fn original() -> Self {
        Skin {
            panel: [24, 25, 29],
            header: [24, 25, 29],
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
    /// The neutral slots are the scheme's own here; [`Skin::stepped`]
    /// replaces them, keeping only `base00`'s hue and tint.
    pub fn from_base16(slots: [u32; 16]) -> Skin {
        let b = slots.map(|v| [(v >> 16) as u8, (v >> 8) as u8, v as u8]);
        let track = b[1];
        let widget = b[2];
        let accent = b[13];
        Skin {
            panel: b[0],
            header: b[0],
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

/// The contrast dim text keeps against the page at every [`Chrome`].
pub const LABEL_FLOOR: f32 = 4.0;

/// How the chrome's neutral layers are laid out: the page's OKLab lightness,
/// and the lightness gap from each layer to the next. The two dials a user
/// turns; everything else about a skin comes from its scheme.
#[derive(Clone, Copy, Debug, PartialEq, serde::Serialize, serde::Deserialize)]
#[serde(default)]
pub struct Chrome {
    pub lightness: f32,
    pub step: f32,
}

impl Chrome {
    /// Both ranges end where the lightest corner, a track at 0.16 + 2 × 0.07,
    /// still leaves every skin's slider fill reading against its track. Below
    /// 0.08 the page is within a byte or two of black, where OKLab lightness
    /// moves in steps too coarse to lay a ladder on.
    pub const LIGHTNESS_RANGE: RangeInclusive<f32> = 0.08..=0.16;
    pub const STEP_RANGE: RangeInclusive<f32> = 0.02..=0.07;

    /// Inside both ranges, a non-finite value going back to its default, so
    /// a hand-edited blob cannot put the dials somewhere the chrome is not.
    pub fn sanitize(self) -> Self {
        let fit = |v: f32, range: RangeInclusive<f32>, fallback: f32| {
            if v.is_finite() {
                v.clamp(*range.start(), *range.end())
            } else {
                fallback
            }
        };
        let default = Self::default();
        Self {
            lightness: fit(self.lightness, Self::LIGHTNESS_RANGE, default.lightness),
            step: fit(self.step, Self::STEP_RANGE, default.step),
        }
    }
}

impl Default for Chrome {
    fn default() -> Self {
        Self { lightness: 0.15, step: 0.06 }
    }
}

impl Skin {
    /// This skin with its neutral layers laid out by `chrome`: each a whole
    /// number of steps above the page, in the page colour's hue and tint.
    ///
    /// | layer | steps |
    /// |---|---|
    /// | `panel` (the page) | 0 |
    /// | `header` | 1 |
    /// | `well` (tracks) | 2 |
    /// | `widget`, `hairline`, `surface_faint` | 3 |
    /// | `widget_hover` | 4 |
    ///
    /// Stepped in OKLab lightness, where equal steps look equal, rather than
    /// taken from the scheme's own backgrounds: base16 has no slot darker than
    /// its page, and the ones it has are ordered by convention only, so a
    /// scheme could put its tracks under its page or its header on a button.
    /// The ladder makes that order a property rather than a hope. The accent
    /// fills are remixed from the stepped surfaces they sit on.
    ///
    /// Dim text is the scheme's, lifted only as far as it takes to keep
    /// [`LABEL_FLOOR`] against the page: a lighter page would otherwise take a
    /// scheme's darker greys under it (Berlin's `base04` sits at 3.97:1 on
    /// the default page).
    pub fn stepped(&self, chrome: Chrome) -> Skin {
        let [_, a, b] = oklab(self.panel);
        let layer = |steps: f32| srgb(chrome.lightness + steps * chrome.step, a, b);
        let (panel, header, well, widget) = (layer(0.0), layer(1.0), layer(2.0), layer(3.0));
        let [mut lightness, dim_a, dim_b] = oklab(self.text_dim);
        let mut text_dim = self.text_dim;
        while contrast(text_dim, panel) < LABEL_FLOOR && lightness < 1.0 {
            lightness += 0.005;
            text_dim = srgb(lightness, dim_a, dim_b);
        }
        Skin {
            text_dim,
            panel,
            header,
            well,
            widget,
            hairline: widget,
            surface_faint: widget,
            widget_hover: layer(4.0),
            accent_fill: mix(well, self.accent, 0.42),
            accent_fill_hover: mix(well, self.accent, 0.58),
            accent_fill_drag: mix(well, self.accent, 0.78),
            accent_active: mix(widget, self.accent, 0.55),
            warning_bg: mix(panel, self.warning_text, 0.2),
            ..*self
        }
    }
}

/// An sRGB byte triple in OKLab, `[L, a, b]`.
pub(crate) fn oklab(c: [u8; 3]) -> [f32; 3] {
    let lin = |v: u8| {
        let v = f32::from(v) / 255.0;
        if v <= 0.04045 {
            v / 12.92
        } else {
            ((v + 0.055) / 1.055).powf(2.4)
        }
    };
    let [r, g, b] = c.map(lin);
    let l = (0.412_221_46 * r + 0.536_332_55 * g + 0.051_445_995 * b).cbrt();
    let m = (0.211_903_5 * r + 0.680_699_5 * g + 0.107_396_96 * b).cbrt();
    let s = (0.088_302_46 * r + 0.281_718_85 * g + 0.629_978_7 * b).cbrt();
    [
        0.210_454_26 * l + 0.793_617_8 * m - 0.004_072_047 * s,
        1.977_998_5 * l - 2.428_592_2 * m + 0.450_593_7 * s,
        0.025_904_037 * l + 0.782_771_77 * m - 0.808_675_77 * s,
    ]
}

/// An OKLab colour as sRGB bytes, its chroma pulled in until it fits sRGB so
/// the lightness and hue are the ones asked for.
fn srgb(lightness: f32, a: f32, b: f32) -> [u8; 3] {
    let linear = |chroma: f32| {
        let (a, b) = (a * chroma, b * chroma);
        let l = (lightness + 0.396_337_78 * a + 0.215_803_76 * b).powi(3);
        let m = (lightness - 0.105_561_346 * a - 0.063_854_17 * b).powi(3);
        let s = (lightness - 0.089_484_18 * a - 1.291_485_5 * b).powi(3);
        [
            4.076_741_7 * l - 3.307_711_6 * m + 0.230_969_94 * s,
            -1.268_438 * l + 2.609_757_4 * m - 0.341_319_38 * s,
            -0.004_196_086_3 * l - 0.703_418_6 * m + 1.707_614_7 * s,
        ]
    };
    let mut chroma = 1.0;
    let mut rgb = linear(chroma);
    while chroma > 0.01 && rgb.iter().any(|v| !(-1e-4..=1.0 + 1e-4).contains(v)) {
        chroma *= 0.9;
        rgb = linear(chroma);
    }
    rgb.map(|v| {
        let v = v.clamp(0.0, 1.0);
        let v = if v <= 0.003_130_8 { 12.92 * v } else { 1.055 * v.powf(1.0 / 2.4) - 0.055 };
        (v * 255.0).round() as u8
    })
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
    /// The scheme as mapped. Its neutral layers only lend the page colour's
    /// hue and tint: what the chrome wears is this [`Skin::stepped`].
    pub skin: Skin,
}

/// The id of the skin a fresh install opens in, first in [`skins`].
pub const DEFAULT_SKIN: &str = "tinta";

/// Base16 schemes, [`DEFAULT_SKIN`] among them: id, name, `base00`..`base0F`.
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
        let (default, rest): (Vec<_>, Vec<_>) = BASE16
            .iter()
            .map(|&(id, name, slots)| SkinEntry { id, name, skin: Skin::from_base16(slots) })
            .partition(|entry| entry.id == DEFAULT_SKIN);
        let original = SkinEntry { id: "original", name: "Original", skin: Skin::original() };
        default.into_iter().chain([original]).chain(rest).collect()
    })
}

/// Where the skin saved as `id` sits in [`skins`], if it is one.
pub fn skin_index(id: &str) -> Option<usize> {
    skins().iter().position(|entry| entry.id == id)
}

/// The skin in force: which of [`skins`], at which [`Chrome`], and the
/// stepped result, kept so a colour lookup is a copy rather than a remix.
#[derive(Clone, Copy)]
struct Active {
    index: usize,
    chrome: Chrome,
    skin: Skin,
}

thread_local! {
    static ACTIVE: Cell<Option<Active>> = const { Cell::new(None) };
}

fn active() -> Active {
    ACTIVE.get().unwrap_or_else(|| {
        let chrome = Chrome::default();
        Active { index: 0, chrome, skin: skins()[0].skin.stepped(chrome) }
    })
}

/// Put `skins()[index]` in force on this thread, stepped by `chrome`; an
/// index past the end is the default. See the module doc for why this is per
/// thread. Restepped only when the index or the chrome moved, the two things
/// the stepped skin is made of.
pub fn set_active_skin(index: usize, chrome: Chrome) {
    let index = if index < skins().len() { index } else { 0 };
    let current = active();
    if (current.index, current.chrome) == (index, chrome) {
        ACTIVE.set(Some(current));
        return;
    }
    ACTIVE.set(Some(Active { index, chrome, skin: skins()[index].skin.stepped(chrome) }));
}

/// The index and chrome [`active_skin`] was stepped from, for a context to
/// remember which skin its style was built in.
pub fn active_skin_key() -> (usize, Chrome) {
    let active = active();
    (active.index, active.chrome)
}

/// The skin in force on this thread; [`DEFAULT_SKIN`]'s at the default
/// [`Chrome`] until one is set.
pub fn active_skin() -> Skin {
    active().skin
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
// the original skin's chrome roles carried when these were split off, so the
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
