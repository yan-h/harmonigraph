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
//! A skin is made from five [`SkinDials`] rather than chosen from a list:
//! every background and both text colours are one grey with one slight tint,
//! and every highlight is one accent colour mixed into those greys. Status
//! colours (learn mode, warnings) are fixed, so no accent can be mistaken
//! for them. Which dials are in force is PER THREAD ([`set_active_skin`])
//! rather than process-wide, because a host can load several plugin
//! instances into one process and each editor wears its own; each sets its
//! skin at the top of its frame, and frames do not interleave on a thread.
//! It also keeps parallel tests from reskinning one another.

use std::cell::Cell;
use std::ops::RangeInclusive;

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

/// What a skin is made of. Persisted in the editor's workspace, never in the
/// picture's appearance: it colours the panel and nothing an export draws.
///
/// Hues are degrees on the OKLab hue circle; `tint` and `accent_saturation`
/// are fractions of [`TINT_MAX`] and [`ACCENT_MAX`], so a bar can read them as
/// a percentage.
#[derive(Clone, Copy, Debug, PartialEq, serde::Serialize, serde::Deserialize)]
#[serde(default)]
pub struct SkinDials {
    /// The OKLab lightness of the page, which every other background layer
    /// stands a whole number of [`STEP`]s above.
    pub lightness: f32,
    /// The hue the greys lean toward.
    pub tint_hue: f32,
    /// How far they lean: 0 is neutral grey.
    pub tint: f32,
    /// The hue of every highlight.
    pub accent_hue: f32,
    /// How colourful the highlights are: 0 is a grey accent.
    pub accent_saturation: f32,
}

/// The contrast dim text keeps against the page at every page lightness.
pub const LABEL_FLOOR: f32 = 4.0;

/// The OKLab lightness between one neutral layer and the next. Fixed rather
/// than a dial: 0.07 is where it was tuned by eye, and moving it would move
/// the range [`LIGHTNESS_RANGE`] was fitted to.
pub const STEP: f32 = 0.07;

/// Where the page's lightness can be dialled. The top is where the lightest
/// track, 0.16 + 2 × [`STEP`], still leaves the slider fill reading against
/// it; below 0.08 the page is within a byte or two of black, where OKLab
/// lightness moves in steps too coarse to lay a ladder on.
pub const LIGHTNESS_RANGE: RangeInclusive<f32> = 0.08..=0.16;

/// Either hue dial: once round the circle.
pub const HUE_RANGE: RangeInclusive<f32> = 0.0..=360.0;

/// The OKLab chroma a full `tint` gives the greys: enough for dim text to
/// read plainly as a warm or cool grey, not enough to read as a colour.
pub const TINT_MAX: f32 = 0.06;

/// The OKLab chroma a full `accent_saturation` asks for, about the most any
/// hue holds at [`ACCENT_LIGHTNESS`]; hues that hold less are pulled in to
/// the gamut by [`srgb`].
pub const ACCENT_MAX: f32 = 0.16;

/// Fixed lightnesses, so no dial can take text or a highlight into the page.
/// Primary text, and where dim text starts before [`LABEL_FLOOR`] lifts it.
const TEXT_LIGHTNESS: f32 = 0.90;
const TEXT_DIM_LIGHTNESS: f32 = 0.72;
/// Where the base16 schemes' accents sat, 0.68 to 0.74.
pub const ACCENT_LIGHTNESS: f32 = 0.70;

/// Status colours, the same under every skin.
const ARMED: [u8; 3] = [238, 178, 92];
const WARNING: [u8; 3] = [236, 142, 132];

impl Default for SkinDials {
    /// Tinta's page and accent as measured, the default before the dials: a
    /// barely cool grey and a muted slate blue.
    fn default() -> Self {
        SkinDials {
            lightness: 0.15,
            tint_hue: 285.0,
            tint: 0.08,
            accent_hue: 255.0,
            accent_saturation: 0.25,
        }
    }
}

impl SkinDials {
    /// Every dial inside the range its bar offers, a non-finite one going
    /// back to its default, so a hand-edited blob cannot put a bar somewhere
    /// the chrome is not.
    pub fn sanitize(&mut self) {
        let fresh = SkinDials::default();
        let sane = |v: f32, range: RangeInclusive<f32>, fresh: f32| {
            if v.is_finite() {
                v.clamp(*range.start(), *range.end())
            } else {
                fresh
            }
        };
        self.lightness = sane(self.lightness, LIGHTNESS_RANGE, fresh.lightness);
        self.tint_hue = sane(self.tint_hue, HUE_RANGE, fresh.tint_hue);
        self.tint = sane(self.tint, 0.0..=1.0, fresh.tint);
        self.accent_hue = sane(self.accent_hue, HUE_RANGE, fresh.accent_hue);
        self.accent_saturation = sane(self.accent_saturation, 0.0..=1.0, fresh.accent_saturation);
    }
}

/// A grey at OKLab `lightness`, leaning `tint` (a fraction of [`TINT_MAX`])
/// toward `hue` degrees: every background and text colour a skin has.
pub fn tinted(lightness: f32, hue: f32, tint: f32) -> [u8; 3] {
    let (sin, cos) = hue.to_radians().sin_cos();
    let chroma = tint * TINT_MAX;
    srgb(lightness, chroma * cos, chroma * sin)
}

/// Dim text on `panel`: the tinted grey at its fixed lightness, lifted only
/// as far as it takes to keep [`LABEL_FLOOR`] against the page.
fn lifted_dim(panel: [u8; 3], hue: f32, tint: f32) -> [u8; 3] {
    let mut dim = TEXT_DIM_LIGHTNESS;
    let mut text_dim = tinted(dim, hue, tint);
    while contrast(text_dim, panel) < LABEL_FLOOR && dim < 1.0 {
        dim += 0.005;
        text_dim = tinted(dim, hue, tint);
    }
    text_dim
}

/// The dim text `dials` make — every label in the panel — without the rest
/// of the skin, for a track previewing it once per point along a bar.
pub fn text_dim_color(dials: SkinDials) -> [u8; 3] {
    let panel = tinted(dials.lightness, dials.tint_hue, dials.tint);
    lifted_dim(panel, dials.tint_hue, dials.tint)
}

/// The accent `hue` degrees makes at `saturation` (a fraction of
/// [`ACCENT_MAX`]).
pub fn accent_color(hue: f32, saturation: f32) -> [u8; 3] {
    let (sin, cos) = hue.to_radians().sin_cos();
    let chroma = saturation * ACCENT_MAX;
    srgb(ACCENT_LIGHTNESS, chroma * cos, chroma * sin)
}

impl Skin {
    /// The skin the dials make. The neutral layers stand a whole number of
    /// [`STEP`]s above the page, in OKLab lightness, where equal steps look
    /// equal:
    ///
    /// | layer | steps |
    /// |---|---|
    /// | `panel` (the page) | 0 |
    /// | `header` | 1 |
    /// | `well` (tracks) | 2 |
    /// | `widget`, `hairline`, `surface_faint` | 3 |
    /// | `widget_hover` | 4 |
    ///
    /// Text is the same tinted grey at a fixed lightness; dim text is lifted
    /// only as far as it takes to keep [`LABEL_FLOOR`] against the page. The
    /// accent fills are mixes of the accent into the surface they sit on, so
    /// they stay opaque (see `theme::accent_fill`).
    pub fn from_dials(dials: SkinDials) -> Skin {
        let grey = |lightness: f32| tinted(lightness, dials.tint_hue, dials.tint);
        let layer = |steps: f32| grey(dials.lightness + steps * STEP);
        let (panel, header, well, widget) = (layer(0.0), layer(1.0), layer(2.0), layer(3.0));
        let accent = accent_color(dials.accent_hue, dials.accent_saturation);
        let text_dim = lifted_dim(panel, dials.tint_hue, dials.tint);
        Skin {
            panel,
            header,
            well,
            surface_faint: widget,
            hairline: widget,
            widget,
            widget_hover: layer(4.0),
            accent,
            text: grey(TEXT_LIGHTNESS),
            text_dim,
            accent_fill: mix(well, accent, 0.42),
            accent_fill_hover: mix(well, accent, 0.58),
            accent_fill_drag: mix(well, accent, 0.78),
            accent_active: mix(widget, accent, 0.55),
            accent_edge: mix(accent, [255, 255, 255], 0.06),
            armed: ARMED,
            warning_text: WARNING,
            warning_bg: mix(panel, WARNING, 0.2),
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

/// `c` with its OKLab lightness turned over — light to dark and back — and
/// its hue and chroma kept (as far as the gamut allows): the same colour seen
/// the other way up, for a handle that shows what it stands on inverted.
pub fn inverted_lightness(c: [u8; 3]) -> [u8; 3] {
    let [lightness, a, b] = oklab(c);
    srgb(1.0 - lightness, a, b)
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

/// The skin in force: the dials, and the skin they make, kept so a colour
/// lookup is a copy rather than a remix.
#[derive(Clone, Copy)]
struct Active {
    dials: SkinDials,
    skin: Skin,
}

thread_local! {
    static ACTIVE: Cell<Option<Active>> = const { Cell::new(None) };
}

fn active() -> Active {
    ACTIVE.get().unwrap_or_else(|| {
        let dials = SkinDials::default();
        Active { dials, skin: Skin::from_dials(dials) }
    })
}

/// Put the skin `dials` make in force on this thread. See the module doc for
/// why this is per thread. Remade only when a dial moved, the dials being
/// everything the skin is made of.
pub fn set_active_skin(dials: SkinDials) {
    let current = active();
    let next = if current.dials == dials {
        current
    } else {
        Active { dials, skin: Skin::from_dials(dials) }
    };
    ACTIVE.set(Some(next));
}

/// The dials [`active_skin`] was made from, for a context to remember which
/// skin its style was built in.
pub fn active_skin_key() -> SkinDials {
    active().dials
}

/// The skin in force on this thread; the default dials' until one is set.
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
