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

/// Bin power at or below this is treated as flat silence — skips the `log10`
/// in the intensity map for the many empty buckets, without changing the
/// look (they'd land at 0 anyway).
const NEAR_ZERO: f32 = 1e-9;

/// Draw the spectrogram across the roll's depth region (`split..1`), sharing
/// the roll's `depth_of` time mapping so its columns register with the notes.
///
/// It's drawn as ONE mesh over a shared grid of vertices — one per (column,
/// bin), colored by that cell's loudness — so egui interpolates the color
/// across the quads between them. That smooth field, rather than a wall of
/// flat cells, is what keeps it from reading as blocks and from shimmering as
/// the continuous scroll slides hard cell edges across the pixels.
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
    // A grid needs at least two columns to have a quad between them.
    if history.len() < 2 {
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

    // The visible buckets, one grid row each, placed at the bucket CENTER so
    // the color reads correctly at the vertex and interpolates to its
    // neighbours. Rows off the octave zoom are dropped.
    let bin_semis = 1.0 / BINS_PER_SEMITONE as f32;
    // One bucket of slack on each side (in axis-fraction units), so the grid
    // has a vertex just past each edge to interpolate the visible range to.
    let margin = (bin_semis / scale.span).min(0.5);
    let bins: Vec<(usize, f32, f32)> = (0..SPECTRUM_BINS)
        .filter_map(|idx| {
            let midi = SPECTRUM_MIN_MIDI + (idx as f32 + 0.5) * bin_semis;
            let t = scale.t_of(midi);
            (t > -margin && t < 1.0 + margin).then_some((idx, midi, t.clamp(0.0, 1.0)))
        })
        .collect();
    if bins.len() < 2 {
        return;
    }

    // The columns inside the window, oldest first, including the one just past
    // the far edge so the grid reaches depth 1 as columns scroll off. Cap the
    // grid to about the depth's pixel resolution — more rows than pixels only
    // costs vertices — striding a long window down to fit.
    let first = history.partition_point(|c| c.time < oldest).saturating_sub(1);
    let max_cols = (axes.depth_len().ceil() as usize).clamp(2, 600);
    let available = history.len() - first;
    let stride = available.div_ceil(max_cols).max(1);
    let cols: Vec<&crate::SpectrogramColumn> =
        history.iter().skip(first).step_by(stride).collect();
    if cols.len() < 2 {
        return;
    }

    let nb = bins.len();
    let mut mesh = egui::Mesh::default();
    mesh.reserve_vertices(cols.len() * nb);
    mesh.reserve_triangles((cols.len() - 1) * (nb - 1) * 2);

    // Vertices: column-major, so vertex (c, r) is at index `c * nb + r`.
    for col in &cols {
        let d = depth_of(col.time);
        for &(idx, midi, t) in &bins {
            let power = col.power[idx];
            let level = if power <= NEAR_ZERO { 0.0 } else { loudness(cfg, power, midi) };
            let color = cell_color(cfg.spectrogram_color, level, midi, &state.frame_params, opacity);
            // Axis space -> screen via `at`, so orientation and flips are free.
            mesh.colored_vertex(axes.at(t, d), color);
        }
    }
    // Stitch each 2x2 block of neighbouring vertices into a quad. egui blends
    // the four corner colors across it — the smoothing that removes the blocks.
    for c in 0..cols.len() - 1 {
        let (a, b) = (c * nb, (c + 1) * nb);
        for r in 0..nb - 1 {
            let (v00, v01, v10, v11) = (a + r, a + r + 1, b + r, b + r + 1);
            mesh.add_triangle(v00 as u32, v10 as u32, v11 as u32);
            mesh.add_triangle(v00 as u32, v11 as u32, v01 as u32);
        }
    }
    if !mesh.is_empty() {
        painter.add(egui::Shape::mesh(mesh));
    }
}

/// A cell's color: `level` (0..1 loudness) mapped through the chosen ramp.
/// The quiet end fades to transparent (rather than a hard cutoff) so silence
/// shows the well cleanly and there's no on/off edge to shimmer as it scrolls;
/// with the grid's interpolation this ramps smoothly across the heatmap.
fn cell_color(
    kind: SpectrogramColor,
    level: f32,
    midi: f32,
    frame: &FrameParams,
    opacity: f32,
) -> Color32 {
    let t = level.clamp(0.0, 1.0);
    // Reach full opacity by the low third, so only near-silence is see-through.
    let alpha = ((t * 3.0).min(1.0) * opacity * 255.0) as u8;
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
    fn quiet_fades_out_and_loud_is_opaque_and_bright() {
        let frame = FrameParams::default();
        let quiet = cell_color(SpectrogramColor::Heat, 0.0, 60.0, &frame, 1.0);
        let loud = cell_color(SpectrogramColor::Heat, 1.0, 60.0, &frame, 1.0);
        // Silence is transparent (shows the well), not a black block.
        assert_eq!(quiet.a(), 0, "silence should be see-through");
        // Loud is opaque and bright; the ramp runs dark -> light.
        assert_eq!(loud.a(), 255);
        let lum = |c: Color32| c.r() as u32 + c.g() as u32 + c.b() as u32;
        assert!(lum(loud) > 0, "loud should carry color, got {loud:?}");
    }

    #[test]
    fn opacity_scales_the_alpha_of_a_full_cell() {
        let frame = FrameParams::default();
        // A loud cell (past the fade-in third) takes the full opacity.
        let c = cell_color(SpectrogramColor::Mono, 1.0, 60.0, &frame, 0.5);
        assert_eq!(c.a(), 127);
    }
}
