//! What the spectrum's FFT costs, and what the variants of it cost:
//! `cargo run --release --example fft_bench -p harmonigraph-core`.
//!
//! Full CI compiles this and gates it on warnings (`ci.sh` runs clippy with
//! `--all-targets`), so it can fail the check — it is only the RUNNING of it
//! that is manual. It is also the one place outside `spectrum.rs` naming
//! `DEFAULT_FFT_SIZE`, so a rename has to come through here.
//!
//! `fft_in_place` is private, so the rows below are copies of it. Keeping them
//! honest is a matter of reading: what pins the shipped transform is
//! `reusing_a_stages_twiddles_does_not_move_a_single_bit` over in
//! `spectrum.rs`, not anything here. `pitch_spectrum` is called through the
//! public API, so that row alone is the real thing.
//!
//! The core rows run at HALF the window, which is the length a column
//! transforms: `pitch_spectrum` packs its real window into `n / 2` complex
//! points and untangles the bins afterwards. Timing them at the full window
//! would measure a transform this crate does not run.

use harmonigraph_core::spectrum::{SpectrumAnalyzer, DEFAULT_FFT_SIZE};
use std::hint::black_box;
use std::time::Instant;

/// The twiddle recomputed inside the butterfly loop — what `fft_in_place`
/// does NOT do, kept as the thing its cost is measured against.
fn fft_per_block(re: &mut [f32], im: &mut [f32]) {
    let n = re.len();
    let bits = n.trailing_zeros();
    for i in 0..n {
        let j = i.reverse_bits() >> (usize::BITS - bits);
        if j > i {
            re.swap(i, j);
            im.swap(i, j);
        }
    }
    let mut len = 2;
    while len <= n {
        let step = std::f32::consts::TAU / len as f32;
        let half = len / 2;
        for start in (0..n).step_by(len) {
            for k in 0..half {
                let angle = -step * k as f32;
                let (ws, wc) = angle.sin_cos();
                let (i, j) = (start + k, start + k + half);
                let (tr, ti) = (re[j] * wc - im[j] * ws, re[j] * ws + im[j] * wc);
                re[j] = re[i] - tr;
                im[j] = im[i] - ti;
                re[i] += tr;
                im[i] += ti;
            }
        }
        len *= 2;
    }
}

/// A copy of the shipped `fft_in_place`: the same butterflies on the same
/// values, with each twiddle computed once per stage rather than once per
/// block.
fn fft_per_stage(re: &mut [f32], im: &mut [f32]) {
    let n = re.len();
    let bits = n.trailing_zeros();
    for i in 0..n {
        let j = i.reverse_bits() >> (usize::BITS - bits);
        if j > i {
            re.swap(i, j);
            im.swap(i, j);
        }
    }
    let mut len = 2;
    while len <= n {
        let step = std::f32::consts::TAU / len as f32;
        let half = len / 2;
        for k in 0..half {
            let angle = -step * k as f32;
            let (ws, wc) = angle.sin_cos();
            for start in (0..n).step_by(len) {
                let (i, j) = (start + k, start + k + half);
                let (tr, ti) = (re[j] * wc - im[j] * ws, re[j] * ws + im[j] * wc);
                re[j] = re[i] - tr;
                im[j] = im[i] - ti;
                re[i] += tr;
                im[i] += ti;
            }
        }
        len *= 2;
    }
}

/// One twiddle table for the whole transform, built once and indexed per
/// stage. Same butterfly order as the baseline.
fn fft_table(re: &mut [f32], im: &mut [f32], tw: &[(f32, f32)]) {
    let n = re.len();
    let bits = n.trailing_zeros();
    for i in 0..n {
        let j = i.reverse_bits() >> (usize::BITS - bits);
        if j > i {
            re.swap(i, j);
            im.swap(i, j);
        }
    }
    let mut len = 2;
    while len <= n {
        let half = len / 2;
        let stride = n / len;
        for start in (0..n).step_by(len) {
            for k in 0..half {
                let (ws, wc) = tw[k * stride];
                let (i, j) = (start + k, start + k + half);
                let (tr, ti) = (re[j] * wc - im[j] * ws, re[j] * ws + im[j] * wc);
                re[j] = re[i] - tr;
                im[j] = im[i] - ti;
                re[i] += tr;
                im[i] += ti;
            }
        }
        len *= 2;
    }
}

/// The untangle from `spectrum.rs`, over the bin range `pitch_spectrum` reads.
/// Half of what the packing costs, and the half a bare half-length transform
/// row would leave out.
fn untangle(re: &[f32], im: &[f32], tw: &[(f32, f32)], power: &mut [f32]) {
    let half = re.len();
    for k in 2..half - 1 {
        let (ws, wc) = tw[k];
        let (ar, ai) = (re[k], im[k]);
        let (cr, ci) = (re[half - k], im[half - k]);
        let (sr, si) = (ar + cr, ai - ci);
        let (dr, di) = (ar - cr, ai + ci);
        let xr = sr + wc * di + ws * dr;
        let xi = si + ws * di - wc * dr;
        power[k] += 0.25 * (xr * xr + xi * xi);
    }
}

/// The twiddles the untangle reads, indexed by bin of the half transform:
/// `e^(-i * tau * k / n)` for the REAL length `n`.
fn untangle_twiddles(n: usize) -> Vec<(f32, f32)> {
    (0..n / 2)
        .map(|k| {
            let angle = -std::f32::consts::TAU * k as f32 / n as f32;
            angle.sin_cos()
        })
        .collect()
}

fn twiddles(n: usize) -> Vec<(f32, f32)> {
    (0..n / 2)
        .map(|k| {
            let angle = -std::f32::consts::TAU * k as f32 / n as f32;
            angle.sin_cos()
        })
        .collect()
}

/// The ring walk `pitch_spectrum` pays for: a modulo per sample.
fn unroll_modulo(ring: &[f32], window: &[f32], write: usize, re: &mut [f32], im: &mut [f32]) {
    let n = ring.len();
    for i in 0..n {
        let src = (write + i) % n;
        re[i] = ring[src] * window[i];
        im[i] = 0.0;
    }
}

/// The same walk as two contiguous runs, which is what the ring actually is.
fn unroll_split(ring: &[f32], window: &[f32], write: usize, re: &mut [f32], im: &mut [f32]) {
    let n = ring.len();
    let (tail, head) = ring.split_at(write);
    let (w_head, w_tail) = window.split_at(n - write);
    for ((dst, r), w) in re[..n - write].iter_mut().zip(head).zip(w_head) {
        *dst = r * w;
    }
    for ((dst, r), w) in re[n - write..].iter_mut().zip(tail).zip(w_tail) {
        *dst = r * w;
    }
    im.fill(0.0);
}

/// How many timed rounds each row runs, of which the FASTEST is reported.
///
/// A mean over one round is the wrong statistic on a machine running several
/// sessions at once, which this repo does by design: a round that lost the
/// core to someone else's build is not a slower FFT, it is a different
/// measurement. Interference only ever adds time, so the minimum is the round
/// that ran least disturbed. Left as a single mean, this probe printed
/// "swapping loops 0.41x faster" under load — the summary contradicting its
/// own rows.
const ROUNDS: u32 = 5;

fn bench<F: FnMut()>(name: &str, iters: u32, mut f: F) -> f64 {
    for _ in 0..iters.min(20) {
        f();
    }
    let mut best = f64::INFINITY;
    for _ in 0..ROUNDS {
        let t = Instant::now();
        for _ in 0..iters {
            f();
        }
        best = best.min(t.elapsed().as_secs_f64() * 1e3 / f64::from(iters));
    }
    println!("  {name:<34} {best:>8.4} ms");
    best
}

fn main() {
    let n = DEFAULT_FFT_SIZE;
    // The transform a column runs, which is half the window it analyzes.
    let m = n / 2;
    let iters = 400;
    let signal: Vec<f32> =
        (0..n).map(|i| (i as f32 * 0.01).sin() * 0.5 + (i as f32 * 0.13).sin() * 0.2).collect();
    let window: Vec<f32> = (0..n)
        .map(|i| {
            let phase = std::f32::consts::TAU * i as f32 / n as f32;
            0.5 - 0.5 * phase.cos()
        })
        .collect();
    let tw = twiddles(m);
    let untw = untangle_twiddles(n);

    println!("\nFFT core, n = {m} (half of the {n}-sample window), {iters} iterations each");
    // Packed as a column packs: even samples real, odd samples imaginary.
    let packed_re: Vec<f32> = signal.iter().step_by(2).copied().collect();
    let packed_im: Vec<f32> = signal.iter().skip(1).step_by(2).copied().collect();
    let (mut re, mut im) = (packed_re.clone(), packed_im.clone());
    let per_block = bench("fft, twiddle per butterfly", iters, || {
        re.copy_from_slice(&packed_re);
        im.copy_from_slice(&packed_im);
        fft_per_block(black_box(&mut re), black_box(&mut im));
    });
    let shipped = bench("fft_in_place (current)", iters, || {
        re.copy_from_slice(&packed_re);
        im.copy_from_slice(&packed_im);
        fft_per_stage(black_box(&mut re), black_box(&mut im));
    });
    let table = bench("fft, precomputed twiddle table", iters, || {
        re.copy_from_slice(&packed_re);
        im.copy_from_slice(&packed_im);
        fft_table(black_box(&mut re), black_box(&mut im), black_box(&tw));
    });
    let mut power = vec![0.0f32; m];
    let packing = bench("fft_in_place + untangle (a column's)", iters, || {
        re.copy_from_slice(&packed_re);
        im.copy_from_slice(&packed_im);
        fft_per_stage(black_box(&mut re), black_box(&mut im));
        untangle(black_box(&re), black_box(&im), black_box(&untw), black_box(&mut power));
    });

    println!("\nRing walk (the loop feeding the FFT)");
    let mut re2 = vec![0.0f32; n];
    let mut im2 = vec![0.0f32; n];
    let modulo = bench("unroll with % per sample (current)", iters * 4, || {
        unroll_modulo(black_box(&signal), black_box(&window), 3457, &mut re2, &mut im2);
    });
    let split = bench("unroll as two contiguous runs", iters * 4, || {
        unroll_split(black_box(&signal), black_box(&window), 3457, &mut re2, &mut im2);
    });

    println!("\nEnd to end, through the public API");
    let mut analyzer = SpectrumAnalyzer::new(48_000.0);
    analyzer.push_samples(&signal);
    analyzer.push_samples(&signal);
    let whole = bench("pitch_spectrum (one channel)", iters, || {
        black_box(analyzer.pitch_spectrum());
    });

    println!("\nSummary");
    let stages = m.trailing_zeros() as usize;
    // Per stage the shipped order computes `half` twiddles and the per-block
    // order computes them once per block, i.e. `half` times `m / len`.
    println!("  sin_cos per FFT, current          {}", m - 1);
    println!("  sin_cos per FFT, per butterfly    {}", (m / 2) * stages);
    // What a shared table would have to hold: one per k of the LAST stage,
    // every earlier stage's angles being a subsampling of those.
    println!("  distinct twiddles in the transform {}", m / 2);
    println!("  current vs per butterfly          {:.2}x faster", per_block / shipped);
    println!("  a twiddle table would be          {:.2}x faster", per_block / table);
    println!("  walk: split vs modulo             {:.2}x faster", modulo / split);
    // The PACKING, not the bare transform: what a column spends on this stage
    // is the half-length FFT and the untangle that unpacks it, and crediting
    // only the first understates the stage it is a share of.
    println!("  transform as a share of a column  {:.0}%", packing / whole * 100.0);
    // What the loop order actually merged saves. Crediting the table here —
    // the faster of the two, and the one NOT implemented — overstated it 11%.
    let saved = per_block - shipped;
    println!("  saved per FFT                     {saved:.4} ms");
    // A hop is 8 ms (`AudioSpectrum::FFT_INTERVAL`), so 125 columns a second,
    // and one FFT per column PER CHANNEL. A core is 1000 ms of the same second.
    let per_second = saved * 125.0 * 2.0;
    println!(
        "  saved per second, stereo @ 8 ms hop {per_second:.2} ms/s ({:.1}% of a core)",
        per_second / 10.0
    );
}
