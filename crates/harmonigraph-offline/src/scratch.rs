//! SCRATCH: the peaks-over-cloud spectrogram prototype, applied to the
//! whole-song column set before it is drawn, so the real fold, shader and
//! palette draw it. Driven by env vars; verification only, not for merging.
//!
//! `HG_SCRATCH_MODE=peaks` turns it on. Parameters:
//! - `HG_CLOUD_T` (s, sigma along time, 0.25), `HG_CLOUD_P` (semitones, 1.0),
//!   `HG_CLOUD_DB` (dB offset added to the cloud, 0)
//! - `HG_PROM` (dB a peak must stand above the cloud, 6)
//! - `HG_STROKE` (cents, sigma across pitch, 12), `HG_STROKE_T` (s, sigma
//!   along time, 0.02), `HG_PEAK_W` (buckets either side a peak must beat, 6)
//! - `HG_SCRATCH_DIFFUSION` overrides the take's diffusion for either mode.
use harmonigraph_core::spectrogram::{DB_FLOOR, DB_STEP};
use harmonigraph_core::spectrum::{BINS_PER_SEMITONE, SPECTRUM_BINS};
use harmonigraph_ui::{SpectrumConfig, WholeSong};

pub fn env_f(name: &str, default: f32) -> f32 {
    std::env::var(name).ok().and_then(|v| v.parse().ok()).unwrap_or(default)
}

/// Gaussian along one axis of a row-major `rows x cols` image, renormalized
/// at the edges. `sigma` in samples of that axis. `along_rows` blurs down the
/// row index (time); otherwise along the column index (pitch).
fn blur_axis(img: &mut [f32], rows: usize, cols: usize, along_rows: bool, sigma: f32) {
    if sigma <= 0.0 {
        return;
    }
    let radius = (3.0 * sigma).ceil() as isize;
    let weights: Vec<f32> =
        (-radius..=radius).map(|i| (-(i * i) as f32 / (2.0 * sigma * sigma)).exp()).collect();
    let src = img.to_vec();
    let (outer, inner) = if along_rows { (cols, rows) } else { (rows, cols) };
    let at = |o: usize, i: usize| if along_rows { i * cols + o } else { o * cols + i };
    for o in 0..outer {
        for i in 0..inner {
            let mut sum = 0.0;
            let mut total = 0.0;
            for (k, w) in weights.iter().enumerate() {
                let j = i as isize + k as isize - radius;
                if j < 0 || j >= inner as isize {
                    continue;
                }
                sum += w * src[at(o, j as usize)];
                total += w;
            }
            img[at(o, i)] = sum / total;
        }
    }
}

pub fn apply(ws: &mut WholeSong, cfg: &mut SpectrumConfig) {
    if let Ok(v) = std::env::var("HG_SCRATCH_DIFFUSION") {
        cfg.atmosphere.diffusion = v.parse().expect("HG_SCRATCH_DIFFUSION is a number");
    }
    if std::env::var("HG_SCRATCH_MODE").as_deref() != Ok("peaks") {
        return;
    }
    let n = ws.columns.len();
    if n < 3 {
        return;
    }
    let b = SPECTRUM_BINS;
    let hop = ((ws.columns[n - 1].time - ws.columns[0].time) / (n as f64 - 1.0)) as f32;
    let cloud_t = env_f("HG_CLOUD_T", 0.25) / hop;
    let cloud_p = env_f("HG_CLOUD_P", 1.0) * BINS_PER_SEMITONE as f32;
    let cloud_db = env_f("HG_CLOUD_DB", 0.0);
    let prom = env_f("HG_PROM", 6.0);
    let stroke_p = env_f("HG_STROKE", 12.0) / 100.0 * BINS_PER_SEMITONE as f32;
    let stroke_t = env_f("HG_STROKE_T", 0.02) / hop;
    let peak_w = env_f("HG_PEAK_W", 6.0) as usize;
    // Black is the heatmap's own floor: everything under it draws the same,
    // so the cloud averages levels the picture can show rather than -120s.
    // The floor is per bucket because the display tilts dB by pitch about
    // `TILT_PIVOT_MIDI` before windowing: clamping at the untilted floor lit
    // every silent treble bucket dark blue.
    const TILT_PIVOT_MIDI: f32 = 83.213_1;
    let black: Vec<f32> = (0..b)
        .map(|j| {
            let midi = harmonigraph_core::spectrum::SPECTRUM_MIN_MIDI
                + (j as f32 + 0.5) / BINS_PER_SEMITONE as f32;
            cfg.volume_floor_db + cfg.tilt * (midi - TILT_PIVOT_MIDI) / 12.0
        })
        .collect();
    let floor = cfg.volume_floor_db;
    eprintln!(
        "scratch peaks: {n} columns, hop {hop:.4} s, cloud sigma {cloud_t:.1} col x {cloud_p:.0} bkt, \
         prom {prom} dB, stroke sigma {stroke_p:.1} bkt x {stroke_t:.1} col, floor {floor:.1} dB"
    );

    // Work in "dB above black at this pitch", so the tilt is already folded
    // in and silence is zero everywhere.
    let raw: Vec<f32> = ws
        .columns
        .iter()
        .flat_map(|c| {
            c.db.iter()
                .enumerate()
                .map(|(j, &v)| (DB_FLOOR + v as f32 * DB_STEP - black[j]).max(0.0))
        })
        .collect();

    let mut cloud = raw.clone();
    blur_axis(&mut cloud, n, b, true, cloud_t);
    blur_axis(&mut cloud, n, b, false, cloud_p);

    let mut stroke = vec![0.0f32; n * b];
    let reach = (3.0 * stroke_p).ceil() as usize;
    let mut peaks = 0usize;
    for t in 0..n {
        let col = &raw[t * b..(t + 1) * b];
        for i in peak_w..b - peak_w {
            let c = col[i];
            if c <= 0.5 || c - cloud[t * b + i] < prom {
                continue;
            }
            // A local maximum over +-peak_w; a flat top counts once (its left end).
            if (i - peak_w..=i + peak_w).any(|j| j != i && col[j] > c)
                || (i - peak_w..i).any(|j| col[j] == c)
            {
                continue;
            }
            // Power-weighted centroid over the peak's own neighbourhood: steadier
            // column to column than a three-point parabola on noisy dB.
            let (mut num, mut den) = (0.0f32, 0.0f32);
            for j in i - peak_w..=i + peak_w {
                let w = 10f32.powf((col[j] - c) / 10.0);
                num += w * j as f32;
                den += w;
            }
            let x = num / den;
            peaks += 1;
            let lo = i.saturating_sub(reach);
            let hi = (i + reach).min(b - 1);
            for j in lo..=hi {
                let d = j as f32 - x;
                let v = c * (-(d * d) / (2.0 * stroke_p * stroke_p)).exp();
                let cell = &mut stroke[t * b + j];
                *cell = cell.max(v);
            }
        }
    }
    blur_axis(&mut stroke, n, b, true, stroke_t);
    eprintln!("scratch peaks: {peaks} peaks ({:.1} per column)", peaks as f32 / n as f32);

    for (t, col) in ws.columns.iter_mut().enumerate() {
        for (j, out) in col.db.iter_mut().enumerate() {
            let v = (cloud[t * b + j] + cloud_db).max(stroke[t * b + j]).max(0.0) + black[j];
            *out = ((v - DB_FLOOR) / DB_STEP).round().clamp(0.0, 255.0) as u8;
        }
    }
}
