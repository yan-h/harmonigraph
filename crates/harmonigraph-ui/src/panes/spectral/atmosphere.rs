//! The analyzer's lit material. Only the surrounding light is smoothed;
//! the filled body's outer contour uses the original per-pixel FFT samples.

use egui::{Color32, Mesh, Painter};

use super::axes::{loudness_db, power_db, spectrogram_level_db, Axes};
use super::spectrogram::cell_color;
use crate::SpectrumConfig;

/// How far apart the halo's taps sit along the curve, as a fraction of the
/// curve's own length. Pane-relative and not sample-relative, so the smoothing
/// radius is the same slice of the picture however many samples the pane has
/// room for. It was `0.005 * spread` until #872 removed the Spread dial; this
/// is that dial at 1.
const HALO_TAP_SPACING: f32 = 0.005;

/// Taps either side of the sample being smoothed, so `2 * this + 1` in all.
///
/// A COMB rather than a low-pass, because the taps are [`HALO_TAP_SPACING`]
/// apart rather than adjacent. That is the reason decimating this is a change
/// to the picture and not a free saving (#995 item 2).
const HALO_TAPS: i32 = 4;

/// The along-curve kernel: `exp(-tap^2 / this)`, a Gaussian about 1.7 taps
/// wide. It leaves the outermost tap contributing ~6%, which is what makes the
/// truncation at [`HALO_TAPS`] invisible rather than a step.
const HALO_TAP_FALLOFF: f32 = 5.78;

/// The halo's half-width across the curve: this much of the pane's SHORTER
/// side, which the caller then divides into the depth axis. Off the shorter
/// side so the glow keeps its proportions at any aspect instead of stretching
/// with the pane. Also `* spread` before #872.
const HALO_REACH_FRACTION: f32 = 0.05;

/// Points of curve height over which the halo fades in. A curve lying on the
/// floor is silence and silence does not glow; two points is enough that the
/// fade is not itself an edge.
const HALO_FADE_IN_PT: f32 = 2.0;

/// Vertices across the halo, counting both rims. ODD, so one band sits exactly
/// on the curve and the two sides are mirrors.
const HALO_BANDS: usize = 9;

/// The across-curve profile: `exp(-x^2 * this)`, with `x` running ±1 from the
/// curve out to each rim. The band next to a rim keeps ~8%, and the rims
/// themselves are forced fully transparent, so the mesh ends in nothing rather
/// than in a visible border.
const HALO_BAND_FALLOFF: f32 = 4.5;

/// The halo's share of the one Analyzer softness dial.
///
/// #872 deleted a separate Glow dial and handed its job to Softness, which
/// also drives the body's flat-to-translucent blend. This is what Glow's own
/// setting became: at full Softness the halo sits where that dial used to put
/// it (`a5415bf7` replaced `settings.glow` with `0.65 * softness`).
const HALO_SOFTNESS_SHARE: f32 = 0.65;

/// The halo's peak opacity as a material, before the dial, the band profile
/// and the fade-in each take their cut — the prototype's own number
/// (`75d3be16`), unchanged since.
///
/// A second factor rather than folded into [`HALO_SOFTNESS_SHARE`] because the
/// two answer different questions (what the glow IS, versus how much of the
/// dial reaches it) and because their product is not the same `f32` as the
/// pair applied in order.
const HALO_PEAK_ALPHA: f32 = 0.38;

/// Points of curve height below which the body is a sliver rather than a
/// shape. The flat fill is off entirely below it and the soft fill fades in
/// across it, so a sub-pixel curve does not paint a hard line along the floor.
pub(super) const BODY_SLIVER_PT: f32 = 0.5;

/// The flat fill's alpha at zero Softness: the analyzer as it looked before
/// #872 gave it a material, and what Softness blends away from.
///
/// Held as the 8-bit value the picture actually carries rather than as a
/// fraction, because that is the number the pane's tests read back off a
/// vertex.
pub(super) const PLAIN_FILL_ALPHA_U8: u8 = 210;
const PLAIN_FILL_ALPHA: f32 = PLAIN_FILL_ALPHA_U8 as f32 / 255.0;

/// The body's stops from the floor out to the measured contour: `(fraction of
/// the curve's height, alpha at full Softness)`.
///
/// A dark translucent foot, a colored middle, then the exact measured edge.
/// The last one IS the contour, which is why it is the most opaque and why its
/// position is the one the halo's smoothing never touches.
pub(super) const BODY_STOPS: [(f32, f32); 3] = [(0.0, 0.28), (0.72, 0.59), (1.0, 0.86)];

/// Outline opacity below which the stroke is not worth a shape at all: one
/// 8-bit alpha level is 1/255, so anything under this rounds away.
const KEYLINE_VISIBLE_MIN: f32 = 0.004;

pub(super) fn draw_profile(
    painter: &Painter,
    axes: &Axes,
    cfg: &SpectrumConfig,
    visible: &[(f32, f32, f32)],
    budget: f32,
    split: f32,
) {
    if visible.len() < 2 {
        return;
    }
    let softness = cfg.atmosphere.sanitized().analyzer_softness;
    let silence = power_db(0.0);
    let sd = |d| if split < 1.0 { split - d } else { d };
    let samples: Vec<_> = visible
        .iter()
        .map(|&(midi, t, level)| {
            (
                t,
                // Tilt can lift the stored zero-power floor above the plot's
                // floor at high pitches. It must not give silence a body.
                if level > silence { loudness_db(cfg, level, midi) * budget } else { 0.0 },
                cell_color(cfg.spectrogram_gradient, spectrogram_level_db(cfg, level, midi)),
            )
        })
        .collect();
    let tint = |color: Color32, alpha: f32| {
        Color32::from_rgba_unmultiplied(
            color.r(),
            color.g(),
            color.b(),
            (alpha.clamp(0.0, 1.0) * 255.0).round() as u8,
        )
    };
    let vertex = |mesh: &mut Mesh, t, d, color| {
        mesh.colored_vertex(axes.at(t, sd(d)), color);
    };
    let connect = |mesh: &mut Mesh, rows: usize, bands: usize| {
        for row in 1..rows {
            for band in 0..bands - 1 {
                let a = ((row - 1) * bands + band) as u32;
                let b = (row * bands + band) as u32;
                mesh.add_triangle(a, b, a + 1);
                mesh.add_triangle(a + 1, b, b + 1);
            }
        }
    };

    if softness > 0.0 {
        let mut halo = Mesh::default();
        let stride = ((samples.len() as f32 * HALO_TAP_SPACING).round() as usize).max(1);
        let reach = axes.pitch_len().min(axes.depth_len()) * HALO_REACH_FRACTION
            / axes.depth_len().max(1.0);
        // The bands run from one rim to the other, so `x` reaches ±1 at this
        // many bands out from the middle one.
        let half_bands = (HALO_BANDS - 1) as f32 / 2.0;
        for (i, &(t, _, _)) in samples.iter().enumerate() {
            let (mut depth, mut rgb, mut total) = (0.0, [0.0; 3], 0.0);
            for tap in -HALO_TAPS..=HALO_TAPS {
                let index = (i as i64 + i64::from(tap) * stride as i64)
                    .clamp(0, samples.len() as i64 - 1) as usize;
                let (_, d, c) = samples[index];
                let weight = (-(tap * tap) as f32 / HALO_TAP_FALLOFF).exp();
                depth += d * weight;
                for (channel, value) in rgb.iter_mut().zip([c.r(), c.g(), c.b()]) {
                    *channel += f32::from(value) * weight;
                }
                total += weight;
            }
            let color = Color32::from_rgb(
                (rgb[0] / total) as u8,
                (rgb[1] / total) as u8,
                (rgb[2] / total) as u8,
            );
            let active = (depth / total * axes.depth_len() / HALO_FADE_IN_PT).clamp(0.0, 1.0);
            for band in 0..HALO_BANDS {
                let x = band as f32 / half_bands - 1.0;
                let weight = if band == 0 || band == HALO_BANDS - 1 {
                    0.0
                } else {
                    (-x * x * HALO_BAND_FALLOFF).exp()
                };
                vertex(
                    &mut halo,
                    t,
                    depth / total + x * reach,
                    tint(color, weight * HALO_SOFTNESS_SHARE * softness * active * HALO_PEAK_ALPHA),
                );
            }
        }
        connect(&mut halo, samples.len(), HALO_BANDS);
        painter.add(halo);
    }

    let mut body = Mesh::default();
    for &(t, d, color) in &samples {
        // A dark translucent foot, a colored body, then the exact measured
        // edge. Peak positions and the volume-color lookup stay unchanged.
        let active = (d * axes.depth_len() / BODY_SLIVER_PT).clamp(0.0, 1.0);
        let plain_alpha =
            if d * axes.depth_len() > BODY_SLIVER_PT { PLAIN_FILL_ALPHA } else { 0.0 };
        for (fraction, alpha) in BODY_STOPS {
            let alpha = egui::lerp(plain_alpha..=alpha * active, softness);
            vertex(&mut body, t, d * fraction, tint(color, alpha));
        }
    }
    connect(&mut body, samples.len(), BODY_STOPS.len());
    painter.add(body);

    // The white contour supplies contrast at the palette's dark end.
    // Its color, width and opacity never depend on the material's softness.
    let opacity = cfg.keyline.clamp(0.0, 1.0);
    if opacity > KEYLINE_VISIBLE_MIN && samples.iter().any(|&(_, d, _)| d > 0.0) {
        let top = samples.iter().map(|&(t, d, _)| axes.at(t, sd(d))).collect();
        painter.add(egui::Shape::line(
            top,
            egui::Stroke::new(1.0, Color32::WHITE.gamma_multiply(opacity)),
        ));
    }
}
