//! Node color: the LCh pitch ramp shared by the CPU and the shader LUT,
//! plus channel and idle colors.

use crate::view::ViewConfig;
use crate::style::PitchPalette;
use harmonigraph_core::ChannelRole;
use crate::PITCH_LUT_N;
use glam::Vec4;

fn lch(l: f64, c: f64, h: f64) -> Vec4 {
    // The conversion is unclamped and out-of-gamut LCH inputs yield values
    // outside 0..255 (v1's graphics stack clamped downstream; we must do it
    // ourselves before handing colors to the shader).
    let rgb = color_space::Rgb::from(color_space::Lch::new(l, c, h));
    Vec4::new(
        (rgb.r.clamp(0.0, 255.0) / 255.0) as f32,
        (rgb.g.clamp(0.0, 255.0) / 255.0) as f32,
        (rgb.b.clamp(0.0, 255.0) / 255.0) as f32,
        1.0,
    )
}

/// Normalized pitch height in 0..1 across the gradient range: 0 at
/// `darkest_pitch`, 1 at `brightest_pitch` (both MIDI note numbers).
///
/// The RATIO is what gets clamped, which is the shader's own form
/// (`pitch_lut_color` in `lattice.wgsl`) and the reason anything can be
/// colored to match what the shader draws. Clamping the PITCH into
/// `darkest..brightest` instead reads the same over every range the Nodes
/// pane can dial, and comes apart at the two the pane cannot: the ends are
/// independent params over 0..120, ordered only by the range bar's min span,
/// so a host reaches an inverted pair — where `f32::clamp` panics on
/// `min > max` — and a collapsed one, where clamping the pitch first pins
/// every note to the dark end while the shader takes the bright one.
///
/// Non-finite ends yield the darkest color rather than a NaN that would ride
/// into the instance buffer unnoticed.
fn pitch_ramp_t(pitch: f32, darkest_pitch: f32, brightest_pitch: f32) -> f64 {
    if !darkest_pitch.is_finite() || !brightest_pitch.is_finite() {
        return 0.0;
    }
    f64::from(
        ((pitch - darkest_pitch) / (brightest_pitch - darkest_pitch).max(0.01)).clamp(0.0, 1.0),
    )
}

/// How far [`PitchPalette::Ramp`] is nudged toward white. The octave
/// indicators always carried this lift (a 30%-toward-white mix in the
/// shader); baking it into the ramp itself puts the core disc and the piano
/// roll — which sample the same ramp — at the same lightness, so a note reads
/// as one color across all three instead of the indicators sitting a shade
/// lighter.
///
/// Only Ramp takes it. The lift is a correction for a curve that runs to
/// `L* = 0`, and the palettes that do not run there have no black end to
/// rescue — mixing toward white would only wash their hues out and, on the
/// fixed-`L*` ones, break the equal brightness that is their whole point.
/// Nothing downstream re-applies it either way: every shape reads one table
/// (see [`pitch_lut_color`]), so they agree whatever is baked in.
const NOTE_LIGHTEN: f32 = 0.30;

/// The hue arc the pitch gradient sweeps, in LCh degrees: blue-violet through
/// magenta and red to yellow-green. Shared by [`PitchPalette::Ramp`],
/// [`Even`](PitchPalette::Even) and [`Lift`](PitchPalette::Lift), which is
/// what makes those three an A/B on brightness alone — the same colors in the
/// same order, differing only in how much luminance separates the ends.
fn classic_hue(t: f64) -> f64 {
    (-100.0 + t * 190.0).rem_euclid(360.0)
}

/// The DESIGNED pitch-gradient curve as a function of normalized height `t`
/// (0..1) and the palette in force. Nothing draws this directly: it is the
/// curve [`pitch_ramp_lut`] samples, and that table is what every consumer
/// reads (see [`pitch_lut_color`]). Keeping one caller means the table and
/// the curve can never drift into two different answers for one pitch.
///
/// Every chroma here is a measured number, not a taste: at a fixed `L*` the
/// sRGB gamut admits a different maximum chroma at every hue, so each of the
/// fixed-`L*` palettes takes the largest chroma that stays inside the gamut
/// across its WHOLE arc. Going past that would not raise the saturation, it
/// would clip a channel — which bends the hue and, worse, changes the
/// luminance, so the palette would stop being the equal-brightness thing it
/// claims to be. `every_flat_palette_is_in_gamut_and_isoluminant` is what
/// holds the numbers to that.
fn pitch_ramp_lch(t: f64, palette: PitchPalette) -> Vec4 {
    match palette {
        // Brightness carries the pitch: `L*` runs the full 0..80. The dark
        // end asks for a chroma the gamut cannot hold at that lightness and
        // clips, which is why the bottom fifth of the range is nearly one
        // flat blue — a known cost of this curve, and the reason the palettes
        // below exist.
        PitchPalette::Ramp => {
            let base = lch(t * 80.0, 85.0 - t * 60.0, classic_hue(t));
            base.lerp(Vec4::new(1.0, 1.0, 1.0, base.w), NOTE_LIGHTEN)
        }
        // Ramp's arc, flattened. `L*` is a function of luminance alone, so
        // one fixed `L*` is one fixed screen brightness exactly — pitch is
        // carried by hue and nothing else. 68 is high enough to sit near the
        // bright half of what Ramp spans (a note gets brighter far more often
        // than it gets dimmer) and still leave chroma to work with: the arc's
        // tightest hue holds 49.9 at that lightness, so 46 clears it.
        PitchPalette::Even => lch(68.0, 46.0, classic_hue(t)),
        // The middle reading of "more similar brightness": a tilt, not a
        // flat. 55..83 is a 2.7x luminance span against Ramp's 5.4x, so up is
        // still visibly up while the low register keeps its presence. Chroma
        // is fixed at 40 — the arc's tightest point under a MOVING lightness
        // is 41.7 — rather than Ramp's 85..25 fall, which is what leaves
        // Ramp's top end a washed cream.
        PitchPalette::Lift => lch(55.0 + t * 28.0, 40.0, classic_hue(t)),
        // Equal brightness spent on separation instead of restraint: a wider
        // arc (280 -> 120, magenta through the warms into green) at the
        // chroma the gamut holds all the way along it. `L* = 60` is where
        // that chroma is largest — 63.6 at the tightest hue, so 60 clears it
        // — and it also keeps the ramp dark enough to read as color rather
        // than as light against the dark lattice.
        PitchPalette::Neon => lch(60.0, 60.0, (-80.0 + t * 200.0).rem_euclid(360.0)),
        // Saturation carries the pitch: one hue, one lightness, chroma
        // 8..47. The bottom is a near-neutral rather than a true grey so a
        // low note still reads as a note and not as an idle marker, and the
        // hue is the skin's own accent blue, which puts the lattice and the
        // UI chrome in one family.
        PitchPalette::Ink => lch(70.0, 8.0 + t * 39.0, 275.0),
        // The other direction from Even, kept as the contrast: brightness
        // carries pitch HARDER than Ramp (a 42..96 `L*` span), on the
        // incandescence reading — a deep ember, through orange, to a
        // white-hot top. Chroma falls as the lightness climbs because that is
        // both what heat does and all the gamut has up there.
        PitchPalette::Ember => lch(42.0 + t * 54.0, 62.0 - t * 54.0, 30.0 + t * 60.0),
    }
}

/// The designed curve, for the test that pins how closely [`pitch_ramp_lut`]
/// tracks it. Nothing in the draw path may call this — going off the curve
/// direct is precisely the mismatch the shared table exists to prevent.
#[cfg(test)]
pub(crate) fn designed_pitch_ramp(t: f64, palette: PitchPalette) -> Vec4 {
    pitch_ramp_lch(t, palette)
}

/// One palette's ramp, sampled into [`PITCH_LUT_N`] colors evenly spaced over
/// the full `t` range. Both sides read it: the renderer uploads it for the
/// shader to index, and [`pitch_lut_color`] walks it on the CPU.
///
/// Each side maps a pitch to a `t` FIRST and indexes with that, so the
/// gradient's endpoints never reach the table and it stays range-independent.
/// The palette is the only thing it varies with — which is why the memo below
/// is keyed on the palette and NOT on the range. A change that folded
/// `darkest_pitch`/`brightest_pitch` into the entries would make this cache
/// wrong, not just stale.
pub fn pitch_ramp_lut(palette: PitchPalette) -> [Vec4; PITCH_LUT_N] {
    // Each entry costs several transcendentals through the LCH->sRGB
    // conversion, and a table is read once per animating frame (~66/s).
    // Compute them once and copy out the cached array — same value, none of
    // the per-frame color math.
    //
    // ALL the palettes at once, rather than one slot filled on demand:
    // the whole set is six tables of 64 vectors, the build is a few hundred
    // conversions, and a single `OnceLock` over the lot cannot be left half
    // initialized or fall out of step with the enum the way a per-variant
    // array of locks could.
    static LUTS: std::sync::OnceLock<[[Vec4; PITCH_LUT_N]; PitchPalette::ALL.len()]> =
        std::sync::OnceLock::new();
    LUTS.get_or_init(|| {
        PitchPalette::ALL.map(|palette| {
            std::array::from_fn(|k| pitch_ramp_lch(k as f64 / (PITCH_LUT_N - 1) as f64, palette))
        })
    })[palette.index()]
}

/// The pitch gradient, evaluated: [`pitch_ramp_lut`] sampled at `pitch` and
/// interpolated between entries exactly as `pitch_lut_color` in `lattice.wgsl`
/// does. Every pitch-colored shape reaches the ramp through this one walk —
/// the disc, the trail, the piano roll and the melody/bass rings on the CPU,
/// the lit octave glyphs on the GPU.
///
/// It is a LIT pitch that this draws: a sounding glyph stands for a position
/// on the pitch axis rather than for the voice that lit it, and so do the
/// glow's lobes once two octaves sound. The band's ghosts and a solo voice's
/// glow keep the node's own color instead, deliberately — a lone voice keeps
/// its exact color, fixed channel hues included, which the ramp could not
/// reproduce. Do not simplify `octave_glow_color`'s `count < 2u` fallback away
/// on the strength of this function's name.
///
/// One table for all of them is what puts a note's disc and its own lit octave
/// indicator on the same color EXACTLY, rather than to within a tolerance, for
/// a given pitch. (Which pitch each is fed is a separate question: a voice
/// outside the wheel's Range lights the outermost slot on its side, so the
/// disc takes the voice's pitch while the glyph takes the clamped slot's — see
/// `derive`. They differ there because they are naming different pitches, not
/// because two definitions of one pitch's color disagree.)
///
/// The shader can only afford a lookup — an LCh->sRGB conversion per fragment
/// is out of reach, and the glow loops call this several times over — so the
/// choice is not "table vs. exact curve" but "one table vs. a table and a
/// curve that disagree". Two shapes sharing an edge is the harshest test of a
/// color match there is, and structural agreement passes it at any table size.
///
/// What the table's size buys is therefore fidelity to the DESIGNED curve
/// ([`pitch_ramp_lch`]), never agreement between shapes — see [`PITCH_LUT_N`]
/// for why that fidelity is worth far less per entry than it looks.
pub fn pitch_lut_color(
    pitch: f32,
    darkest_pitch: f32,
    brightest_pitch: f32,
    palette: PitchPalette,
) -> Vec4 {
    let lut = pitch_ramp_lut(palette);
    let f = pitch_ramp_t(pitch, darkest_pitch, brightest_pitch) as f32 * (PITCH_LUT_N - 1) as f32;
    // `pitch_ramp_t` clamps into 0..1, so the floor lands inside the table and
    // the last entry pairs with itself at a lerp weight of 0.
    let i0 = f.floor() as usize;
    lut[i0].lerp(lut[(i0 + 1).min(PITCH_LUT_N - 1)], f - f.floor())
}

/// Ported verbatim from v1 (`editor/color.rs`); the channel policy itself
/// lives in [`ChannelRole`]. Gradient channels are colored by pitch height on
/// the `palette`'s LCh ramp, spread between `darkest_pitch` and
/// `brightest_pitch` (MIDI note numbers). The fixed per-channel colors below
/// are NOT on that ramp and no palette touches them: a channel color names a
/// voice, and it has to keep meaning the same voice whatever the pitch
/// gradient is set to.
pub fn channel_color(
    channel: u8,
    pitch: f32,
    darkest_pitch: f32,
    brightest_pitch: f32,
    palette: PitchPalette,
) -> Vec4 {
    match ChannelRole::of(channel) {
        ChannelRole::FixedColor => match channel {
            0 => lch(48.0, 45.0, 32.0),  // red
            1 => lch(65.0, 60.0, 68.0),  // orange
            2 => lch(80.0, 42.0, 83.0),  // yellow
            3 => lch(65.0, 50.0, 120.0), // green
            4 => lch(60.0, 40.0, 280.0), // blue
            5 => lch(50.0, 55.0, 305.0), // purple
            6 => lch(70.0, 30.0, 340.0), // pink
            7 => lch(80.0, 0.0, 0.0),    // white
            _ => lch(0.0, 0.0, 0.0),     // 8: black
        },
        // Through the table, not off the curve direct: the shader has only the
        // table, so this is what puts a disc and the octave glyph drawn on top
        // of it in the same color exactly (see [`pitch_lut_color`]).
        ChannelRole::PitchGradient => {
            pitch_lut_color(pitch, darkest_pitch, brightest_pitch, palette)
        }
        // Outline voices get a bright neutral (the ring shape is the
        // signal). Ignored never reaches here — the tracker drops it.
        ChannelRole::Outline | ChannelRole::Ignored => Vec4::new(0.85, 0.85, 0.88, 1.0),
    }
}

/// The idle layer's color: the grid color's RGB at full alpha. The grid's
/// alpha is the LINE opacity and doesn't belong to the markers, which have
/// their own presence — so it's dropped here rather than dimming them
/// along with the lines.
pub(crate) fn idle_color(view: &ViewConfig) -> Vec4 {
    let c = view.grid_color;
    Vec4::new(c[0], c[1], c[2], 1.0)
}
