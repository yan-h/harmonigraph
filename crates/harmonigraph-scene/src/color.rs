//! Node color: the LCh pitch ramp shared by the CPU and the shader LUT,
//! plus channel and idle colors.

use crate::view::ViewConfig;
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

/// How far the pitch gradient is nudged toward white. The octave indicators
/// always carried this lift (a 30%-toward-white mix in the shader); baking it
/// into the ramp itself puts the core disc and the piano roll — which sample
/// the same ramp — at the same lightness, so a note reads as one color across
/// all three instead of the indicators sitting a shade lighter.
const NOTE_LIGHTEN: f32 = 0.30;

/// The DESIGNED pitch-gradient curve as a function of normalized height `t`
/// (0..1). Nothing draws this directly: it is the curve [`pitch_ramp_lut`]
/// samples, and that table is what every consumer reads (see
/// [`pitch_lut_color`]). Keeping one caller means the table and the curve can
/// never drift into two different answers for one pitch.
///
/// The ramp is defined already lightened (see [`NOTE_LIGHTEN`]); everything
/// that samples it inherits the lift, so nothing downstream has to add its own.
fn pitch_ramp_lch(t: f64) -> Vec4 {
    let base = lch(t * 80.0, 85.0 - t * 60.0, (-100.0 + t * 190.0).rem_euclid(360.0));
    base.lerp(Vec4::new(1.0, 1.0, 1.0, base.w), NOTE_LIGHTEN)
}

/// The designed curve, for the test that pins how closely [`pitch_ramp_lut`]
/// tracks it. Nothing in the draw path may call this — going off the curve
/// direct is precisely the mismatch the shared table exists to prevent.
#[cfg(test)]
pub(crate) fn designed_pitch_ramp(t: f64) -> Vec4 {
    pitch_ramp_lch(t)
}

/// The pitch ramp sampled into [`PITCH_LUT_N`] colors evenly spaced over the
/// full `t` range. Both sides read it: the renderer uploads it for the shader
/// to index, and [`pitch_lut_color`] walks it on the CPU.
///
/// Each side maps a pitch to a `t` FIRST and indexes with that, so the
/// gradient's endpoints never reach the table and it stays range-independent —
/// which is what lets the memo below hold one array for the whole session
/// without a key. A change that folded `darkest_pitch`/`brightest_pitch` into
/// the entries would make this cache wrong, not just stale.
pub fn pitch_ramp_lut() -> [Vec4; PITCH_LUT_N] {
    // Constant (range-independent, per the doc above) but each entry costs
    // several transcendentals through the LCH->sRGB conversion, and it's read
    // once per animating frame (~66/s). Compute it once and copy out the
    // cached array — same value, none of the per-frame color math.
    static LUT: std::sync::OnceLock<[Vec4; PITCH_LUT_N]> = std::sync::OnceLock::new();
    *LUT.get_or_init(|| {
        std::array::from_fn(|k| pitch_ramp_lch(k as f64 / (PITCH_LUT_N - 1) as f64))
    })
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
pub fn pitch_lut_color(pitch: f32, darkest_pitch: f32, brightest_pitch: f32) -> Vec4 {
    let lut = pitch_ramp_lut();
    let f = pitch_ramp_t(pitch, darkest_pitch, brightest_pitch) as f32 * (PITCH_LUT_N - 1) as f32;
    // `pitch_ramp_t` clamps into 0..1, so the floor lands inside the table and
    // the last entry pairs with itself at a lerp weight of 0.
    let i0 = f.floor() as usize;
    lut[i0].lerp(lut[(i0 + 1).min(PITCH_LUT_N - 1)], f - f.floor())
}

/// Ported verbatim from v1 (`editor/color.rs`); the channel policy itself
/// lives in [`ChannelRole`]. Gradient channels are colored by pitch height
/// on an LCH ramp between `darkest_pitch` and `brightest_pitch` (MIDI note
/// numbers).
pub fn channel_color(channel: u8, pitch: f32, darkest_pitch: f32, brightest_pitch: f32) -> Vec4 {
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
        ChannelRole::PitchGradient => pitch_lut_color(pitch, darkest_pitch, brightest_pitch),
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
