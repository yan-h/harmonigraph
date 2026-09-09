#![allow(dead_code)]
mod ballistics;
mod golden_audio;
mod spectrogram;
mod spectrum;
use spectrum::{ChannelBank, SPECTRUM_BINS};
use std::alloc::{GlobalAlloc, Layout, System};
use std::hint::black_box;
use std::io::Write;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering::Relaxed};
use std::time::Instant;

struct Counting;
static LIVE: AtomicUsize = AtomicUsize::new(0);
static PEAK: AtomicUsize = AtomicUsize::new(0);
static ALLOCS: AtomicUsize = AtomicUsize::new(0);
static COUNT: AtomicBool = AtomicBool::new(false);
fn added(n: usize) {
    let now = LIVE.fetch_add(n, Relaxed) + n;
    PEAK.fetch_max(now, Relaxed);
    ALLOCS.fetch_add(1, Relaxed);
}
unsafe impl GlobalAlloc for Counting {
    unsafe fn alloc(&self, l: Layout) -> *mut u8 {
        let p = System.alloc(l);
        if !p.is_null() && COUNT.load(Relaxed) {
            added(l.size());
        }
        p
    }
    unsafe fn dealloc(&self, p: *mut u8, l: Layout) {
        if COUNT.load(Relaxed) {
            LIVE.fetch_sub(l.size(), Relaxed);
        }
        System.dealloc(p, l);
    }
    unsafe fn realloc(&self, p: *mut u8, l: Layout, n: usize) -> *mut u8 {
        let q = System.realloc(p, l, n);
        if !q.is_null() && COUNT.load(Relaxed) {
            LIVE.fetch_sub(l.size(), Relaxed);
            added(n);
        }
        q
    }
}
#[global_allocator]
static ALLOCATOR: Counting = Counting;
const HOP: usize = 384; // 8 ms at 48 kHz.
const SIZES: [usize; 3] = [4096, 8192, 16384];
const TAPERS: [usize; 3] = [1, 3, 5];

fn bank(n: usize, k: usize) -> ChannelBank {
    let mut b = ChannelBank::new(48_000.0, 2);
    b.set_fft_size(n);
    b.set_tapers(k);
    b
}
fn signal(kind: &str, frames: usize) -> Vec<f32> {
    if kind == "golden" {
        return golden_audio::probe_audio().into_iter().take(frames).flat_map(|x| [x, x]).collect();
    }
    let mut rng = 0x1234abcd_u32;
    (0..frames)
        .flat_map(|i| {
            rng = rng.wrapping_mul(1664525).wrapping_add(1013904223);
            let noise = (rng as f64 / u32::MAX as f64 - 0.5) as f32;
            let t = i as f64 / 48_000.0;
            let sine = |hz: f64| (std::f64::consts::TAU * hz * t).sin() as f32;
            let l = match kind {
                "silence" => 0.0,
                "tone" | "antiphase" => 0.7 * sine(440.0),
                "quiet" => 1e-5 * sine(440.0),
                "noise" => noise * 0.3,
                "impulse" => {
                    if i % 997 == 0 {
                        0.8
                    } else {
                        0.0
                    }
                }
                _ => {
                    (0.3 * sine(220.0)
                        + 0.2 * sine(440.37)
                        + 0.12 * sine(660.0)
                        + 0.06 * sine(4031.0))
                        * (0.6 + 0.4 * sine(1.7))
                        + 0.02 * noise
                }
            };
            let r = match kind {
                "antiphase" => -l,
                "mixed" => 0.23 * sine(330.0) + 0.1 * sine(1101.3) - 0.03 * noise,
                _ => l,
            };
            [l, r]
        })
        .collect()
}
fn feed(b: &mut ChannelBank, input: &[f32]) {
    for chunk in input.chunks(2 * HOP) {
        b.push_frames(chunk);
    }
}
fn timing() {
    // Force process-global bucket frequencies before per-bank measurements.
    drop(bank(8192, 1));
    println!("n,tapers,input,ns_per_column,steady_allocs,retained_bytes,construct_peak_bytes,init_ns,reconfigure_ns");
    for n in SIZES {
        for k in TAPERS {
            for kind in ["mixed", "silence"] {
                let input = signal(kind, n + 64 * HOP);
                COUNT.store(true, Relaxed);
                LIVE.store(0, Relaxed);
                PEAK.store(0, Relaxed);
                let mut b = bank(n, k);
                let peak = PEAK.load(Relaxed);
                feed(&mut b, &input[..2 * n]);
                let retained = LIVE.load(Relaxed);
                // Allocation verification is separate from every timed region.
                let allocs = ALLOCS.load(Relaxed);
                b.push_frames(&input[2 * n..2 * n + 2 * HOP]);
                black_box(b.power_sum().unwrap());
                let allocations = ALLOCS.load(Relaxed) - allocs;
                drop(b);
                assert_eq!(LIVE.load(Relaxed), 0);
                COUNT.store(false, Relaxed);
                let mut b = bank(n, k);
                feed(&mut b, &input[..2 * n]);
                for i in 0..32 {
                    b.push_frames(&input[2 * n + i * 2 * HOP..2 * n + (i + 1) * 2 * HOP]);
                    black_box(b.power_sum().unwrap());
                }
                let iters = if kind == "silence" { 2000 } else { 500 };
                let start = Instant::now();
                for i in 0..iters {
                    let offset = 2 * n + (i % 64) * 2 * HOP;
                    b.push_frames(black_box(&input[offset..offset + 2 * HOP]));
                    black_box(b.power_sum().unwrap());
                }
                let ns = start.elapsed().as_nanos() as f64 / iters as f64;
                let start = Instant::now();
                for _ in 0..25 {
                    black_box(bank(n, k));
                }
                let init = start.elapsed().as_nanos() / 25;
                let start = Instant::now();
                for _ in 0..25 {
                    b.set_tapers(if k == 1 { 3 } else { 1 });
                    b.set_tapers(k);
                }
                let reconf = start.elapsed().as_nanos() / 50;
                println!("{n},{k},{kind},{ns:.1},{allocations},{retained},{peak},{init},{reconf}");
            }
        }
    }
}
fn dump(path: &str) {
    let mut f = std::io::BufWriter::new(std::fs::File::create(path).unwrap());
    let mut q = std::io::BufWriter::new(std::fs::File::create(format!("{path}.db")).unwrap());
    let mut smooth =
        std::io::BufWriter::new(std::fs::File::create(format!("{path}.smooth")).unwrap());
    println!("n,tapers,input,dft_peak_scaled_error,dft_normalized_absolute_error");
    for n in SIZES {
        for k in TAPERS {
            for kind in
                ["mixed", "silence", "tone", "antiphase", "quiet", "noise", "impulse", "golden"]
            {
                let columns = if kind == "golden" { 192 } else { 16 };
                let input = signal(kind, n + columns * HOP);
                let mut b = bank(n, k);
                feed(&mut b, &input[..2 * n]);
                let mut shown = [0.0f32; SPECTRUM_BINS];
                for i in 0..columns {
                    b.push_frames(&input[2 * n + i * 2 * HOP..2 * n + (i + 1) * 2 * HOP]);
                    let buckets = b.power_sum().unwrap();
                    assert!(buckets.iter().all(|p| p.is_finite() && *p >= 0.0));
                    if kind == "silence" {
                        assert_eq!(buckets, [0.0; SPECTRUM_BINS]);
                    }
                    for (p, shown) in buckets.into_iter().zip(&mut shown) {
                        f.write_all(&p.to_le_bytes()).unwrap();
                        q.write_all(&[spectrogram::quantize(p)]).unwrap();
                        // UI's power recurrence with default attack/release; this
                        // observes numerical propagation, not a rendered UI frame.
                        let tau = if p > *shown { 0.010f32 } else { 0.150 };
                        let alpha = ballistics::hop_alpha(tau, 0.008);
                        *shown += (p - *shown) * alpha;
                        smooth.write_all(&shown.to_le_bytes()).unwrap();
                    }
                }
                let (scaled, abs) = spectrum::reference_error(&b);
                println!("{n},{k},{kind},{scaled:.9e},{abs:.9e}");
            }
        }
    }
}
fn main() {
    let args: Vec<_> = std::env::args().collect();
    match args.get(1).map(String::as_str) {
        Some("dump") => dump(&args[2]),
        _ => timing(),
    }
}
