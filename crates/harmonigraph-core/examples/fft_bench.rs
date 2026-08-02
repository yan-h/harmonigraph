//! Measurement scratch for the perf audit: what the spectrum's FFT costs, and
//! what the obvious variants of it cost. Not part of the build's contract —
//! `cargo run --release --example fft_bench -p harmonigraph-core`.
//!
//! `fft_in_place` is private, so the baseline here is a faithful copy of it.
//! `pitch_spectrum` is called through the public API, so the end-to-end row is
//! the real thing.

use harmonigraph_core::spectrum::{SpectrumAnalyzer, DEFAULT_FFT_SIZE};
use std::hint::black_box;
use std::time::Instant;

/// Verbatim copy of `harmonigraph_core::spectrum::fft_in_place`.
fn fft_baseline(re: &mut [f32], im: &mut [f32]) {
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

/// The same butterflies in the same order, with the k and start loops swapped
/// so each twiddle is computed once per stage instead of once per block.
fn fft_hoisted(re: &mut [f32], im: &mut [f32]) {
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

fn twiddles(n: usize) -> Vec<(f32, f32)> {
    (0..n / 2)
        .map(|k| {
            let angle = -std::f32::consts::TAU * k as f32 / n as f32;
            angle.sin_cos()
        })
        .collect()
}

/// The ring unroll from `pitch_spectrum`: a modulo per sample.
fn unroll_modulo(ring: &[f32], window: &[f32], write: usize, re: &mut [f32], im: &mut [f32]) {
    let n = ring.len();
    for i in 0..n {
        let src = (write + i) % n;
        re[i] = ring[src] * window[i];
        im[i] = 0.0;
    }
}

/// The same unroll as two contiguous runs, which is what the ring actually is.
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

fn bench<F: FnMut()>(name: &str, iters: u32, mut f: F) -> f64 {
    for _ in 0..iters.min(20) {
        f();
    }
    let t = Instant::now();
    for _ in 0..iters {
        f();
    }
    let per = t.elapsed().as_secs_f64() * 1e3 / f64::from(iters);
    println!("  {name:<34} {per:>8.4} ms");
    per
}

fn main() {
    let n = DEFAULT_FFT_SIZE;
    let iters = 400;
    let signal: Vec<f32> = (0..n)
        .map(|i| (i as f32 * 0.01).sin() * 0.5 + (i as f32 * 0.13).sin() * 0.2)
        .collect();
    let window: Vec<f32> = (0..n)
        .map(|i| {
            let phase = std::f32::consts::TAU * i as f32 / n as f32;
            0.5 - 0.5 * phase.cos()
        })
        .collect();
    let tw = twiddles(n);

    println!("\nFFT core, n = {n}, {iters} iterations each");
    let mut re = signal.clone();
    let mut im = vec![0.0f32; n];
    let base = bench("fft_in_place (current)", iters, || {
        re.copy_from_slice(&signal);
        im.fill(0.0);
        fft_baseline(black_box(&mut re), black_box(&mut im));
    });
    let hoisted = bench("fft, k/start loops swapped", iters, || {
        re.copy_from_slice(&signal);
        im.fill(0.0);
        fft_hoisted(black_box(&mut re), black_box(&mut im));
    });
    let table = bench("fft, precomputed twiddle table", iters, || {
        re.copy_from_slice(&signal);
        im.fill(0.0);
        fft_table(black_box(&mut re), black_box(&mut im), black_box(&tw));
    });

    println!("\nRing unroll (the loop feeding the FFT)");
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
    println!("  sin_cos calls per FFT (current)   {}", (n / 2) * n.trailing_zeros() as usize);
    println!("  distinct twiddles actually needed  {}", n - 1);
    println!("  fft: swapping loops               {:.2}x faster", base / hoisted);
    println!("  fft: twiddle table                {:.2}x faster", base / table);
    println!("  unroll: split vs modulo           {:.2}x faster", modulo / split);
    println!("  FFT as a share of pitch_spectrum  {:.0}%", base / whole * 100.0);
    let saved = base - table.min(hoisted);
    println!("  saved per FFT                     {saved:.4} ms");
    println!(
        "  saved per second, stereo @ 8 ms hop {:.2} ms/s ({:.1}% of a core)",
        saved * 125.0 * 2.0,
        saved * 125.0 * 2.0 / 10.0
    );
}
