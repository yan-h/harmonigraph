//! The analyzer's lit material. Only the surrounding light is smoothed;
//! the filled body's outer contour uses the original per-pixel FFT samples.

use egui::{Color32, Mesh, Painter};

use super::axes::{loudness_db, power_db, spectrogram_level_db, Axes};
use super::spectrogram::cell_color;
use crate::SpectrumConfig;

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
        let stride = ((samples.len() as f32 * 0.005).round() as usize).max(1);
        let reach = axes.pitch_len().min(axes.depth_len()) * 0.05 / axes.depth_len().max(1.0);
        for (i, &(t, _, _)) in samples.iter().enumerate() {
            let (mut depth, mut rgb, mut total) = (0.0, [0.0; 3], 0.0);
            for tap in -4i32..=4 {
                let index = (i as i64 + i64::from(tap) * stride as i64)
                    .clamp(0, samples.len() as i64 - 1) as usize;
                let (_, d, c) = samples[index];
                let weight = (-(tap * tap) as f32 / 5.78).exp();
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
            let active = (depth / total * axes.depth_len() / 2.0).clamp(0.0, 1.0);
            for band in 0..9 {
                let x = band as f32 / 4.0 - 1.0;
                let weight = if band == 0 || band == 8 { 0.0 } else { (-x * x * 4.5).exp() };
                vertex(
                    &mut halo,
                    t,
                    depth / total + x * reach,
                    tint(color, weight * 0.65 * softness * active * 0.38),
                );
            }
        }
        connect(&mut halo, samples.len(), 9);
        painter.add(halo);
    }

    let mut body = Mesh::default();
    for &(t, d, color) in &samples {
        // A dark translucent foot, a colored body, then the exact measured
        // edge. Peak positions and the volume-color lookup stay unchanged.
        let active = (d * axes.depth_len() / 0.5).clamp(0.0, 1.0);
        let plain_alpha = if d * axes.depth_len() > 0.5 { 210.0 / 255.0 } else { 0.0 };
        for (fraction, alpha) in [(0.0, 0.28), (0.72, 0.59), (1.0, 0.86)] {
            let alpha = egui::lerp(plain_alpha..=alpha * active, softness);
            vertex(&mut body, t, d * fraction, tint(color, alpha));
        }
    }
    connect(&mut body, samples.len(), 3);
    painter.add(body);

    // The white contour supplies contrast at the palette's dark end.
    // Its color, width and opacity never depend on the material's softness.
    let opacity = cfg.keyline.clamp(0.0, 1.0);
    if opacity > 0.004 && samples.iter().any(|&(_, d, _)| d > 0.0) {
        let top = samples.iter().map(|&(t, d, _)| axes.at(t, sd(d))).collect();
        painter.add(egui::Shape::line(
            top,
            egui::Stroke::new(1.0, Color32::WHITE.gamma_multiply(opacity)),
        ));
    }
}
