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

    // Aggregate the in-window columns into one grid row per depth pixel,
    // grouping by a FIXED time grid (each row a `bucket`-second slab) and
    // taking the element-wise MAX of the columns in it. Two things this buys:
    // a short, bright note keeps its peak (max, not a dropped sample) and it
    // stays put as the scroll advances (the slab is a function of absolute
    // time, not of position in the ring), so it no longer flickers on and off.
    // With fewer columns than rows each column simply lands in its own slab.
    let nb = bins.len();
    let target_rows = (depth_span * axes.depth_len()).round().clamp(2.0, 512.0) as usize;
    let bucket = window / target_rows as f64;
    let first = history.partition_point(|c| c.time < oldest).saturating_sub(1);
    let bin_idx: Vec<usize> = bins.iter().map(|&(idx, _, _)| idx).collect();
    let (centers, power) = aggregate_rows(history.iter().skip(first), &bin_idx, bucket);
    if centers.len() < 2 {
        return;
    }

    let mut mesh = egui::Mesh::default();
    mesh.reserve_vertices(centers.len() * nb);
    mesh.reserve_triangles((centers.len() - 1) * (nb - 1) * 2);

    // Vertices: row-major, so vertex (row, k) is at index `row * nb + k`.
    for (row, &center) in centers.iter().enumerate() {
        let d = depth_of(center);
        let base = row * nb;
        for (k, &(_, midi, t)) in bins.iter().enumerate() {
            let p = power[base + k];
            let level = if p <= NEAR_ZERO { 0.0 } else { loudness(cfg, p, midi) };
            let color = cell_color(cfg.spectrogram_color, level, midi, &state.frame_params, opacity);
            // Axis space -> screen via `at`, so orientation and flips are free.
            mesh.colored_vertex(axes.at(t, d), color);
        }
    }
    // Stitch each 2x2 block of neighbouring vertices into a quad. egui blends
    // the four corner colors across it — the smoothing that removes the blocks.
    for row in 0..centers.len() - 1 {
        let (a, b) = (row * nb, (row + 1) * nb);
        for k in 0..nb - 1 {
            let (v00, v01, v10, v11) = (a + k, a + k + 1, b + k, b + k + 1);
            mesh.add_triangle(v00 as u32, v10 as u32, v11 as u32);
            mesh.add_triangle(v00 as u32, v11 as u32, v01 as u32);
        }
    }
    if !mesh.is_empty() {
        painter.add(egui::Shape::mesh(mesh));
    }
}

/// Group `columns` (oldest first) into time-slabs of `bucket` seconds, taking
/// the element-wise MAX over the bins listed in `bin_idx` within each slab.
/// Returns each slab's center time and a flat row-major power grid
/// (`rows * bin_idx.len()`).
///
/// The slab a column lands in is `floor(time / bucket)` — a function of
/// absolute time alone, so it doesn't move as columns scroll off the far end
/// of the ring. That, plus MAX (rather than dropping samples), is what stops a
/// short, bright note from flickering: its peak is kept and stays in one
/// slowly-scrolling slab instead of blinking in and out with the sampling.
fn aggregate_rows<'a>(
    columns: impl Iterator<Item = &'a crate::SpectrogramColumn>,
    bin_idx: &[usize],
    bucket: f64,
) -> (Vec<f64>, Vec<f32>) {
    let nb = bin_idx.len();
    let mut centers: Vec<f64> = Vec::new();
    let mut power: Vec<f32> = Vec::new();
    let mut cur_key: Option<i64> = None;
    for col in columns {
        let key = (col.time / bucket).floor() as i64;
        if Some(key) != cur_key {
            cur_key = Some(key);
            centers.push((key as f64 + 0.5) * bucket);
            power.resize(power.len() + nb, 0.0);
        }
        let base = power.len() - nb;
        for (k, &idx) in bin_idx.iter().enumerate() {
            let p = col.power[idx];
            if p > power[base + k] {
                power[base + k] = p;
            }
        }
    }
    (centers, power)
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
    use lattice_core::spectrum::SPECTRUM_BINS;

    /// A column at `time` with the given (bin, power) energy, rest silent.
    fn col(time: f64, energy: &[(usize, f32)]) -> crate::SpectrogramColumn {
        let mut power = Box::new([0.0f32; SPECTRUM_BINS]);
        for &(i, p) in energy {
            power[i] = p;
        }
        crate::SpectrogramColumn { time, power }
    }

    #[test]
    fn a_short_loud_column_keeps_its_peak_through_aggregation() {
        // A brief loud note (a short note) between two quiet columns, all in
        // one slab. MAX must keep the peak — the flicker came from dropping
        // exactly this kind of thin, bright sample.
        let cols = [
            col(0.00, &[(5, 0.001)]),
            col(0.02, &[(5, 1.0)]), // the short note
            col(0.04, &[(5, 0.002)]),
        ];
        let (centers, power) = aggregate_rows(cols.iter(), &[5], 1.0);
        assert_eq!(centers.len(), 1, "one slab of width 1.0 s holds all three");
        assert_eq!(power[0], 1.0, "the short note's peak survives");
    }

    #[test]
    fn a_slab_is_anchored_to_absolute_time_not_ring_position() {
        // The same note must land in the same slab whether or not older
        // columns are still present — otherwise scrolling would shift it and
        // it would shimmer. A note at t=2.6 sits in slab floor(2.6)=2.
        let with_old = [col(0.1, &[(0, 0.1)]), col(2.6, &[(0, 0.5)])];
        let (c_full, _) = aggregate_rows(with_old.iter(), &[0], 1.0);
        let just_note = [col(2.6, &[(0, 0.5)])];
        let (c_scrolled, _) = aggregate_rows(just_note.iter(), &[0], 1.0);
        assert!(c_full.contains(&2.5), "slab center is 2.5 with old columns");
        assert!(c_scrolled.contains(&2.5), "and still 2.5 after they scroll off");
    }

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
