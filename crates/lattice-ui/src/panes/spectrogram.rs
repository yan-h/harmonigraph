//! The Spectral pane's spectrogram: a frequency-vs-time heatmap of the
//! analyzed audio, drawn in the roll's depth region on the roll's own time
//! axis. A column of spectral energy therefore lines up with the note
//! ribbons that made it — the same pitch axis across, the same `now`-anchored
//! time along, so what you hear and what you played read against each other.
//!
//! It's a layer under the roll, not a pane: geometry comes from
//! [`Axes`](super::spectral::Axes) (so it turns and flips with everything
//! else), and its dB intensity scale is shared with the spectrum curve via
//! [`loudness`](super::spectral::loudness), so "loud" means the same in both.

use egui::Color32;
use lattice_core::spectrum::{BINS_PER_SEMITONE, SPECTRUM_BINS, SPECTRUM_MIN_MIDI};
use lattice_scene::{channel_color, FrameParams};

use super::spectral::{loudness, Axes, PitchScale};
use crate::{SharedState, SpectrogramColor};

/// The channel whose role borrows the lattice's low-to-high pitch ramp, for
/// the `Pitch` colormap (matches the roll's Pitch coloring).
const PITCH_RAMP_CHANNEL: u8 = 9;

/// Cells quieter than this read as silence and are skipped — they'd paint the
/// dark end of the ramp over an already-dark background, so leaving them out
/// costs nothing and thins the mesh to only the bins carrying energy.
const SILENCE: f32 = 0.02;

/// Never emit more time columns than this, whatever the window and refresh
/// rate ask for; a long window is strided down to keep the mesh bounded.
const MAX_COLUMNS: usize = 360;

/// Draw the spectrogram across the roll's depth region (`split..1`), sharing
/// the roll's `depth_of` time mapping so its columns register with the notes.
pub(super) fn draw_spectrogram(
    painter: &egui::Painter,
    axes: &Axes,
    scale: &PitchScale,
    state: &SharedState,
    split: f32,
    now: f64,
) {
    let cfg = &state.spectrum_config;
    let history = state.spectrum.history();
    if history.is_empty() {
        return;
    }

    let window = cfg.roll_seconds.max(0.05) as f64;
    let depth_span = 1.0 - split;
    // Same mapping as the roll: `now` at the split, the window's far end at 1.
    let depth_of = |t: f64| split + ((now - t) / window).clamp(0.0, 1.0) as f32 * depth_span;
    let oldest = now - window;
    let opacity = cfg.spectrogram_opacity.clamp(0.0, 1.0);
    if opacity <= 0.0 {
        return;
    }

    // Precompute each bucket's pitch-axis span once; skip buckets off the
    // octave zoom. `t_lo`/`t_hi` are the cell's low/high pitch fractions.
    struct Bin {
        idx: usize,
        midi: f32,
        t_lo: f32,
        t_hi: f32,
    }
    let bin_semis = 1.0 / BINS_PER_SEMITONE as f32;
    let bins: Vec<Bin> = (0..SPECTRUM_BINS)
        .filter_map(|idx| {
            let midi_lo = SPECTRUM_MIN_MIDI + idx as f32 * bin_semis;
            let midi_hi = midi_lo + bin_semis;
            let (t_lo, t_hi) = (scale.t_of(midi_lo), scale.t_of(midi_hi));
            // Keep any cell that overlaps the visible axis at all.
            (t_hi > 0.0 && t_lo < 1.0).then_some(Bin {
                idx,
                midi: midi_lo + 0.5 * bin_semis,
                t_lo: t_lo.clamp(0.0, 1.0),
                t_hi: t_hi.clamp(0.0, 1.0),
            })
        })
        .collect();
    if bins.is_empty() {
        return;
    }

    // The columns inside the window, oldest kept first. Stride if a long
    // window would otherwise overflow MAX_COLUMNS (draw every k-th column).
    let first = history.partition_point(|c| c.time < oldest).saturating_sub(1);
    let in_window = history.len() - first;
    let stride = in_window.div_ceil(MAX_COLUMNS).max(1);

    let mut mesh = egui::Mesh::default();
    // A generous reservation; the mesh grows past it if the audio is dense.
    mesh.reserve_vertices(bins.len() * 8);
    mesh.reserve_triangles(bins.len() * 4);

    let cols: Vec<&crate::SpectrogramColumn> = history.iter().skip(first).step_by(stride).collect();
    for (c, col) in cols.iter().enumerate() {
        // This column paints the depth band from its own time up to the next
        // kept column (or to `now` for the newest) — piecewise-constant in
        // time, matching how the ribbons treat a held sample.
        let far = depth_of(col.time);
        let near = cols.get(c + 1).map_or(split, |next| depth_of(next.time));
        if (far - near).abs() * axes.depth_len() < 0.25 {
            continue; // sub-pixel row; nothing to see
        }
        for bin in &bins {
            let level = loudness(cfg, col.power[bin.idx], bin.midi);
            if level < SILENCE {
                continue;
            }
            let color = cell_color(cfg.spectrogram_color, level, bin.midi, &state.frame_params, opacity);
            // Quad corners in axis space -> screen via `at`, so every
            // orientation and flip is handled for free.
            let base = mesh.vertices.len() as u32;
            mesh.colored_vertex(axes.at(bin.t_lo, near), color);
            mesh.colored_vertex(axes.at(bin.t_hi, near), color);
            mesh.colored_vertex(axes.at(bin.t_hi, far), color);
            mesh.colored_vertex(axes.at(bin.t_lo, far), color);
            mesh.add_triangle(base, base + 1, base + 2);
            mesh.add_triangle(base, base + 2, base + 3);
        }
    }
    if !mesh.is_empty() {
        painter.add(egui::Shape::mesh(mesh));
    }
}

/// A cell's color: `level` (0..1 loudness) mapped through the chosen ramp,
/// at the overall `opacity`. The dark end of each ramp matches the pane's
/// well, so quiet cells melt into the background.
fn cell_color(
    kind: SpectrogramColor,
    level: f32,
    midi: f32,
    frame: &FrameParams,
    opacity: f32,
) -> Color32 {
    let t = level.clamp(0.0, 1.0);
    let alpha = (opacity * 255.0) as u8;
    let rgb = match kind {
        SpectrogramColor::Mono => ramp(t, &[[0, 0, 0], [255, 255, 255]]),
        SpectrogramColor::Heat => ramp(
            t,
            &[[0, 0, 0], [90, 0, 20], [200, 40, 20], [240, 150, 30], [255, 240, 180]],
        ),
        SpectrogramColor::Ice => ramp(
            t,
            &[[0, 0, 0], [10, 20, 70], [20, 70, 180], [60, 180, 220], [220, 250, 255]],
        ),
        SpectrogramColor::Aurora => ramp(
            t,
            &[[0, 0, 0], [50, 10, 90], [30, 90, 120], [40, 170, 100], [230, 230, 60]],
        ),
        SpectrogramColor::Pitch => {
            // The lattice's own pitch color, scaled down toward black by the
            // loudness so quiet cells stay dark while keeping their hue.
            let c = channel_color(PITCH_RAMP_CHANNEL, midi, frame.darkest_pitch, frame.brightest_pitch);
            let s = t.sqrt(); // lift the low end a touch so faint pitches still show
            [
                (c.x.clamp(0.0, 1.0) * s * 255.0) as u8,
                (c.y.clamp(0.0, 1.0) * s * 255.0) as u8,
                (c.z.clamp(0.0, 1.0) * s * 255.0) as u8,
            ]
        }
    };
    Color32::from_rgba_unmultiplied(rgb[0], rgb[1], rgb[2], alpha)
}

/// Sample an evenly-spaced list of RGB stops at `t` in `0..1`, linearly
/// interpolating between the two nearest.
fn ramp(t: f32, stops: &[[u8; 3]]) -> [u8; 3] {
    let n = stops.len();
    if n == 1 {
        return stops[0];
    }
    let x = t.clamp(0.0, 1.0) * (n - 1) as f32;
    let i = (x.floor() as usize).min(n - 2);
    let f = x - i as f32;
    let lerp = |a: u8, b: u8| (a as f32 + (b as f32 - a as f32) * f) as u8;
    [
        lerp(stops[i][0], stops[i + 1][0]),
        lerp(stops[i][1], stops[i + 1][1]),
        lerp(stops[i][2], stops[i + 1][2]),
    ]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ramp_hits_its_endpoints_and_midpoint() {
        let stops = [[0, 0, 0], [100, 100, 100], [200, 200, 200]];
        assert_eq!(ramp(0.0, &stops), [0, 0, 0]);
        assert_eq!(ramp(1.0, &stops), [200, 200, 200]);
        // Halfway lands on the middle stop.
        assert_eq!(ramp(0.5, &stops), [100, 100, 100]);
        // A quarter of the way is halfway into the first segment.
        assert_eq!(ramp(0.25, &stops), [50, 50, 50]);
    }

    #[test]
    fn quiet_is_dark_and_loud_is_bright() {
        let frame = FrameParams::default();
        let quiet = cell_color(SpectrogramColor::Heat, 0.0, 60.0, &frame, 1.0);
        let loud = cell_color(SpectrogramColor::Heat, 1.0, 60.0, &frame, 1.0);
        // The ramp runs dark -> light, so loud is strictly brighter.
        let lum = |c: Color32| c.r() as u32 + c.g() as u32 + c.b() as u32;
        assert!(lum(loud) > lum(quiet), "loud {loud:?} vs quiet {quiet:?}");
        assert_eq!(quiet, Color32::from_rgba_unmultiplied(0, 0, 0, 255));
    }

    #[test]
    fn opacity_reaches_the_alpha_channel() {
        let frame = FrameParams::default();
        let c = cell_color(SpectrogramColor::Mono, 1.0, 60.0, &frame, 0.5);
        assert_eq!(c.a(), 127);
    }
}
