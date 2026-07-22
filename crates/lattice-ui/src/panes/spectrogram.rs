//! The Spectral pane's spectrogram: a frequency-vs-time heatmap of the
//! analyzed audio, drawn in the roll's depth region on the roll's own time
//! axis. A column of spectral energy therefore lines up with the note
//! ribbons that made it — the same pitch axis across, the same `now`-anchored
//! time along, so what you hear and what you played read against each other.
//!
//! It's a layer under the roll, not a pane. The heatmap is built into a small
//! image — one pixel per (time slab, pitch bin) — uploaded as a texture and
//! sampled with bilinear filtering, so it reads as one smooth, filled image
//! rather than a mesh of flat cells (which looked blocky) or interpolated
//! triangles (which floated and creased). Geometry still comes from
//! [`Axes`](super::spectral::Axes), so it turns and flips with the pane, and
//! its dB intensity scale is shared with the spectrum curve via
//! [`loudness`](super::spectral::loudness) so "loud" means the same in both.

use egui::Color32;
use lattice_core::spectrum::{BINS_PER_SEMITONE, SPECTRUM_BINS, SPECTRUM_MIN_MIDI};
use lattice_scene::{channel_color, FrameParams};

use super::spectral::{loudness, Axes, PitchScale};
use crate::{SharedState, SpectrogramColor, SpectrumConfig};

/// The channel whose role borrows the lattice's low-to-high pitch ramp, for
/// the `Pitch` colormap (matches the roll's Pitch coloring).
const PITCH_RAMP_CHANNEL: u8 = 9;

/// Bin power at or below this is treated as flat silence — skips the `log10`
/// in the intensity map for the many empty buckets, without changing the look.
const NEAR_ZERO: f32 = 1e-9;

/// Draw the spectrogram across the roll's depth region (`split..1`), sharing
/// the roll's `depth_of` time mapping so its columns register with the notes.
///
/// Builds the heatmap into a `[time slab x pitch bin]` image, (re)uploads it
/// to `spectrum.spectrogram_tex`, then stretches it over the region as a
/// single bilinear-filtered quad — smooth in both axes, and opaque (silence is
/// the ramp's dark end, not transparent) so the plane is a filled image rather
/// than bright patches floating on the background.
pub(super) fn draw_spectrogram(
    painter: &egui::Painter,
    axes: &Axes,
    scale: &PitchScale,
    state: &mut SharedState,
    split: f32,
    now: f64,
) {
    // Small copies, so `state.spectrum` is then free to take mutably (its
    // texture handle) without fighting the config/frame reads.
    let cfg = state.spectrum_config;
    let frame = state.frame_params;
    let spectrum = &mut state.spectrum;
    let opacity = cfg.spectrogram_opacity.clamp(0.0, 1.0);
    if opacity <= 0.0 || spectrum.history().len() < 2 {
        return;
    }
    let window = cfg.roll_seconds.max(0.05) as f64;
    let depth_span = 1.0 - split;
    // `now` at the split, the window's far end at 1 (the roll's mapping).
    let depth_of = |t: f64| split + ((now - t) / window).clamp(0.0, 1.0) as f32 * depth_span;
    let oldest = now - window;

    // The visible buckets (idx, center MIDI, center pitch fraction), one image
    // row each, low pitch first. A bucket of slack on each side lets the
    // filtering carry the visible range cleanly to its edges.
    let bin_semis = 1.0 / BINS_PER_SEMITONE as f32;
    let margin = (bin_semis / scale.span).min(0.5);
    let bins: Vec<(usize, f32, f32)> = (0..SPECTRUM_BINS)
        .filter_map(|idx| {
            let midi = SPECTRUM_MIN_MIDI + (idx as f32 + 0.5) * bin_semis;
            let t = scale.t_of(midi);
            (t > -margin && t < 1.0 + margin).then_some((idx, midi, t))
        })
        .collect();
    if bins.len() < 2 {
        return;
    }

    // Aggregate the in-window columns into one image column per depth pixel by
    // a FIXED time grid, MAX within each slab (keeps a short note's peak and
    // pins it against the scroll — see `aggregate_rows`).
    let target_cols = (depth_span * axes.depth_len()).round().clamp(2.0, 512.0) as usize;
    let bucket = window / target_cols as f64;
    let first = spectrum.history().partition_point(|c| c.time < oldest).saturating_sub(1);
    let bin_idx: Vec<usize> = bins.iter().map(|&(idx, _, _)| idx).collect();
    let (centers, mut power) = aggregate_rows(spectrum.history().iter().skip(first), &bin_idx, bucket);
    let newest = spectrum.history().back().map_or(now, |c| c.time);
    let (w, h) = (centers.len(), bins.len());
    if w < 2 {
        return;
    }
    // Optional temporal smoothing: average out fast beating/chorus wobble.
    smooth_time(&mut power, w, h, cfg.spectrogram_smoothing);

    // The image covers absolute time `[t_origin, t_origin + w*bucket]` — the
    // oldest slab's start to the newest slab's end. Its texel centers sit at
    // the slab centers, so `u = (t - t_origin) / span` places time exactly.
    let t_origin = centers[0] - 0.5 * bucket;
    let span = w as f64 * bucket;
    let (t0, tn) = (bins[0].2, bins[h - 1].2);
    if span < 1e-9 || (tn - t0).abs() < 1e-6 {
        return;
    }

    // Build and upload the image (pixel (x = slab, y = bin), y = 0 low pitch).
    let pixels = fill_pixels(&cfg, &frame, w, &bins, &power);
    let image = egui::ColorImage::new([w, h], pixels);
    let opts = egui::TextureOptions::LINEAR; // bilinear + ClampToEdge
    match &mut spectrum.spectrogram_tex {
        Some(handle) => handle.set(image, opts),
        slot => *slot = Some(painter.ctx().load_texture("spectrogram", image, opts)),
    }
    let Some(tex) = &spectrum.spectrogram_tex else { return };

    // Map a screen depth to the texture's time axis CONTINUOUSLY, through the
    // roll's own `now`-anchored depth<->time relation (unclamped, the inverse
    // of `depth_of`). This is the fix for the image not tracking the notes and
    // for the per-slab stutter: `u` now slides with `now` frame to frame,
    // exactly as a note ribbon at the same depth does, instead of being pinned
    // to the slab endpoints (which jump a whole slab at a time). `v` maps pitch
    // to the bin rows. UVs run past 0..1 in the thin slivers with no data (the
    // sub-slab at `now`, the bit beyond the oldest slab); ClampToEdge fills
    // those from the edge column so the plane stays whole.
    let u_at = |d: f32| ((now - window * ((d - split) / depth_span) as f64 - t_origin) / span) as f32;
    let v_at = |p: f32| (p - t0) / (tn - t0);
    let tint = Color32::from_white_alpha((opacity * 255.0) as u8);
    let vert =
        |p: f32, d: f32| egui::epaint::Vertex { pos: axes.at(p, d), uv: egui::pos2(u_at(d), v_at(p)), color: tint };

    // The quad only spans the depths the data actually reaches, so the drawn
    // strip GROWS from the now-line as history accumulates rather than being
    // stretched to fill the whole region. Without the far cap, clearing the
    // spectrogram (or startup) left a handful of fresh columns ClampToEdge-
    // smeared across everything as trails.
    //
    // Near edge: to the split while fresh columns keep arriving, but stopping
    // at the newest data once it goes stale — most visibly when switching the
    // window algorithm, which empties the ring for a window's worth of samples
    // and would otherwise smear the last pre-switch slice over the growing gap.
    // The grace keeps the ordinary ~one-FFT lag from opening a flickering
    // sliver. Far edge: the oldest slab's depth, which is 1 once history spans
    // the window (depth_of clamps there) and nearer while it is still filling.
    const FRESH: f64 = 0.12;
    let d_near = if now - newest <= FRESH { split } else { depth_of(newest) };
    let d_far = depth_of(t_origin);

    // One quad over pitch [0,1] x depth [d_near, d_far]; GPU bilinear-samples it.
    let mut mesh = egui::Mesh::with_texture(tex.id());
    mesh.vertices.push(vert(0.0, d_far)); // far, low
    mesh.vertices.push(vert(1.0, d_far)); // far, high
    mesh.vertices.push(vert(1.0, d_near)); // near, high
    mesh.vertices.push(vert(0.0, d_near)); // near, low
    mesh.add_triangle(0, 1, 2);
    mesh.add_triangle(0, 2, 3);
    painter.add(egui::Shape::mesh(mesh));
}

/// Group `columns` (oldest first) into time-slabs of `bucket` seconds, taking
/// the element-wise MAX over the bins in `bin_idx` within each slab. Returns
/// each slab's center time and a flat row-major power grid (`rows * nb`).
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

/// Smooth `power` (flat `rows * nb`, row-major over time slabs) along time,
/// in place. `smoothing` in `0..1` sets the strength: 0 leaves it untouched,
/// toward 1 blends ever more of each column into its neighbors. Runs an EMA
/// forward then backward, so the smoothing is symmetric (zero-phase) and a
/// peak doesn't drift in either time direction.
fn smooth_time(power: &mut [f32], rows: usize, nb: usize, smoothing: f32) {
    let a = 1.0 - smoothing.clamp(0.0, 0.95);
    if a >= 1.0 || rows < 2 {
        return;
    }
    for row in 1..rows {
        let (i, prev) = (row * nb, (row - 1) * nb);
        for k in 0..nb {
            power[i + k] += (1.0 - a) * (power[prev + k] - power[i + k]);
        }
    }
    for row in (0..rows - 1).rev() {
        let (i, next) = (row * nb, (row + 1) * nb);
        for k in 0..nb {
            power[i + k] += (1.0 - a) * (power[next + k] - power[i + k]);
        }
    }
}

/// The heatmap image, row-major `pixel(x = slab, y = bin)` at `[y * w + x]`,
/// with `y = 0` the lowest bin. `power` is the flat `w * bins.len()` grid from
/// [`aggregate_rows`]. Opaque throughout — silence is the ramp's dark end, so
/// the plane is filled rather than see-through.
fn fill_pixels(
    cfg: &SpectrumConfig,
    frame: &FrameParams,
    w: usize,
    bins: &[(usize, f32, f32)],
    power: &[f32],
) -> Vec<Color32> {
    let h = bins.len();
    let mut pixels = vec![Color32::BLACK; w * h];
    for x in 0..w {
        let base = x * h;
        for (y, &(_, midi, _)) in bins.iter().enumerate() {
            let p = power[base + y];
            let level = if p <= NEAR_ZERO { 0.0 } else { loudness(cfg, p, midi) };
            pixels[y * w + x] = cell_color(cfg.spectrogram_color, level, midi, frame);
        }
    }
    pixels
}

/// A cell's opaque color: `level` (0..1 loudness) mapped through the chosen
/// ramp. The ramp's dark end matches the pane's well, so silence recedes while
/// energy stands out — the overall opacity is applied once, as the quad's tint.
fn cell_color(kind: SpectrogramColor, level: f32, midi: f32, frame: &FrameParams) -> Color32 {
    let t = level.clamp(0.0, 1.0);
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
            // The lattice's own pitch color, scaled toward black by loudness so
            // quiet cells stay dark while keeping their hue.
            let c = channel_color(PITCH_RAMP_CHANNEL, midi, frame.darkest_pitch, frame.brightest_pitch);
            let s = t.sqrt(); // lift the low end a touch so faint pitches still show
            [
                (c.x.clamp(0.0, 1.0) * s * 255.0) as u8,
                (c.y.clamp(0.0, 1.0) * s * 255.0) as u8,
                (c.z.clamp(0.0, 1.0) * s * 255.0) as u8,
            ]
        }
    };
    Color32::from_rgb(rgb[0], rgb[1], rgb[2])
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
        // A brief loud note between two quiet columns, all in one slab. MAX
        // must keep the peak — the flicker came from dropping this thin, bright
        // sample.
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
        // The same note must land in the same slab whether or not older columns
        // are present — otherwise scrolling would shift it and it would shimmer.
        // A note at t=2.6 sits in slab floor(2.6)=2.
        let with_old = [col(0.1, &[(0, 0.1)]), col(2.6, &[(0, 0.5)])];
        let (c_full, _) = aggregate_rows(with_old.iter(), &[0], 1.0);
        let just_note = [col(2.6, &[(0, 0.5)])];
        let (c_scrolled, _) = aggregate_rows(just_note.iter(), &[0], 1.0);
        assert!(c_full.contains(&2.5), "slab center is 2.5 with old columns");
        assert!(c_scrolled.contains(&2.5), "and still 2.5 after they scroll off");
    }

    #[test]
    fn fill_pixels_places_energy_at_the_right_slab_and_bin() {
        // Two slabs, three bins. Put a loud value in slab x=1, bin y=2 and
        // check it lands at pixel [y*w + x] and nowhere else is bright.
        let cfg = SpectrumConfig::default();
        let frame = FrameParams::default();
        let w = 2;
        let bins = [(10usize, 40.0f32, 0.1f32), (11, 41.0, 0.2), (12, 42.0, 0.3)];
        let mut power = vec![0.0f32; w * bins.len()]; // row-major [slab][bin]
        power[bins.len() + 2] = 1.0; // slab 1, bin 2 loud
        let px = fill_pixels(&cfg, &frame, w, &bins, &power);
        let lum = |c: Color32| c.r() as u32 + c.g() as u32 + c.b() as u32;
        let loud = px[2 * w + 1]; // y=2, x=1
        assert!(lum(loud) > 0, "the loud cell should carry color");
        // Every other pixel is silence -> the ramp's dark end.
        for (i, &c) in px.iter().enumerate() {
            if i != 2 * w + 1 {
                assert_eq!(lum(c), 0, "pixel {i} should be dark, got {c:?}");
            }
        }
    }

    #[test]
    fn cells_are_opaque_and_run_dark_to_bright() {
        let frame = FrameParams::default();
        let quiet = cell_color(SpectrogramColor::Heat, 0.0, 60.0, &frame);
        let loud = cell_color(SpectrogramColor::Heat, 1.0, 60.0, &frame);
        // Opaque throughout (opacity is applied as the quad tint, not here).
        assert_eq!(quiet.a(), 255);
        assert_eq!(loud.a(), 255);
        // Silence is the dark end; loud is brighter.
        let lum = |c: Color32| c.r() as u32 + c.g() as u32 + c.b() as u32;
        assert_eq!(lum(quiet), 0, "silence is the ramp's black end");
        assert!(lum(loud) > lum(quiet));
    }

    #[test]
    fn smoothing_off_is_a_no_op_and_on_spreads_a_spike_without_drift() {
        // One bin (nb=1), a spike well inside a long run of slabs so the
        // forward+backward passes reach a symmetric interior (edge rows aside).
        let n = 15;
        let center = 7;
        let mut base = vec![0.0f32; n];
        base[center] = 1.0;

        // Off: untouched.
        let mut p = base.clone();
        smooth_time(&mut p, n, 1, 0.0);
        assert_eq!(p, base);

        // On: the spike spreads into both neighbors, the peak drops, and the
        // two sides match — the forward+backward passes leave no time drift.
        let mut p = base.clone();
        smooth_time(&mut p, n, 1, 0.7);
        assert!(p[center] < 1.0, "peak drops as it spreads");
        assert!(p[center - 1] > 0.0 && p[center + 1] > 0.0, "reaches both neighbors");
        assert!((p[center - 1] - p[center + 1]).abs() < 1e-3, "no time-direction drift");
        assert!(p[center - 1] > p[center - 2], "monotonic falloff away from the peak");
    }

    #[test]
    fn ramp_hits_its_endpoints_and_midpoint() {
        let stops = [[0, 0, 0], [100, 100, 100], [200, 200, 200]];
        assert_eq!(ramp(0.0, &stops), [0, 0, 0]);
        assert_eq!(ramp(1.0, &stops), [200, 200, 200]);
        assert_eq!(ramp(0.5, &stops), [100, 100, 100]);
        assert_eq!(ramp(0.25, &stops), [50, 50, 50]);
    }
}
