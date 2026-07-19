//! Node color: the LCh pitch ramp shared by the CPU and the shader LUT,
//! plus channel and idle colors.

use crate::view::ViewConfig;
use lattice_core::ChannelRole;
use crate::DOT_RAMP_N;
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
fn pitch_ramp_t(pitch: f32, darkest_pitch: f32, brightest_pitch: f32) -> f64 {
    f64::from(
        (pitch.clamp(darkest_pitch, brightest_pitch) - darkest_pitch)
            / (brightest_pitch - darkest_pitch).max(0.01),
    )
}

/// The pitch-gradient LCH ramp as a function of normalized height `t`
/// (0..1). Shared by the node disc color and the dots octave style's
/// per-dot tint, so a dot is the same color as the disc that pitch lights.
fn pitch_ramp_lch(t: f64) -> Vec4 {
    lch(t * 80.0, 85.0 - t * 60.0, (-100.0 + t * 190.0).rem_euclid(360.0))
}

/// The pitch ramp sampled into [`DOT_RAMP_N`] colors evenly spaced over the
/// full `t` range, for the shader's per-dot color lookup (the shader maps a
/// dot's pitch to a `t` and indexes this). Endpoints of the disc gradient
/// are applied shader-side, so this LUT itself is range-independent.
pub fn pitch_ramp_lut() -> [Vec4; DOT_RAMP_N] {
    // Constant (range-independent, per the doc above) but each entry costs
    // several transcendentals through the LCH->sRGB conversion, and it's read
    // once per animating frame (~66/s). Compute it once and copy out the
    // cached array — same value, none of the per-frame color math.
    static LUT: std::sync::OnceLock<[Vec4; DOT_RAMP_N]> = std::sync::OnceLock::new();
    *LUT.get_or_init(|| {
        std::array::from_fn(|k| pitch_ramp_lch(k as f64 / (DOT_RAMP_N - 1) as f64))
    })
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
        ChannelRole::PitchGradient => {
            pitch_ramp_lch(pitch_ramp_t(pitch, darkest_pitch, brightest_pitch))
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
