//! What one interactive frame costs the CPU on the egui side, measured
//! headlessly: building the UI, tessellating what it built, and the heap
//! churn of both.
//!
//! Every test here is `#[ignore]`d, because a measurement has no pass or
//! fail — it prints numbers and the reader decides. Run them with
//!
//! ```text
//! cargo test --release -p harmonigraph-ui -- --ignored --nocapture \
//!     --test-threads=1 profile
//! ```
//!
//! ONE THREAD, and it is not a nicety either: cargo runs tests in parallel by
//! default, so the five of these otherwise time each other's work and share
//! one pair of allocation counters. Left parallel they read three times high
//! and rank the variants in an order that makes no sense.
//!
//! RELEASE, because the frame's cost under `--release` and
//! under the test profile's `opt-level = 2` are different numbers, and only
//! the first is the one a DAW pays.
//!
//! What this can and cannot see. The dock, every pane's layout, the scene
//! derivation and the text batches all run here exactly as they do in the
//! plugin. What does NOT is anything behind a paint callback: the lattice's
//! `prepare` (the scene pass and the bloom chain) and the roll's instance
//! upload need a wgpu device, so a callback here is a shape that costs
//! nothing. Those are the renderer's cost, and the perf overlay's `prepare`
//! and `scene` rows are where they are read.

use super::harness::*;
use crate::*;
use harmonigraph_core::NoteEvent;

/// Counts what a frame takes off the heap. Two relaxed atomics per
/// allocation, which is small enough that the timings stay readable with it
/// installed — a counter that had to be switched off to time anything would
/// be measuring a build nobody runs.
struct Counting;

static ALLOCS: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);
static BYTES: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);

unsafe impl std::alloc::GlobalAlloc for Counting {
    unsafe fn alloc(&self, layout: std::alloc::Layout) -> *mut u8 {
        ALLOCS.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        BYTES.fetch_add(layout.size(), std::sync::atomic::Ordering::Relaxed);
        unsafe { std::alloc::System.alloc(layout) }
    }
    unsafe fn dealloc(&self, ptr: *mut u8, layout: std::alloc::Layout) {
        unsafe { std::alloc::System.dealloc(ptr, layout) }
    }
    unsafe fn realloc(&self, ptr: *mut u8, layout: std::alloc::Layout, new: usize) -> *mut u8 {
        // A growing `Vec` is counted at the size it grew TO, so a buffer that
        // doubles its way to N bytes reads as about 2N. That overstates the
        // bytes and is the honest reading of the churn: every one of those
        // sizes really was asked of the allocator.
        ALLOCS.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        BYTES.fetch_add(new, std::sync::atomic::Ordering::Relaxed);
        unsafe { std::alloc::System.realloc(ptr, layout, new) }
    }
}

#[global_allocator]
static ALLOCATOR: Counting = Counting;

const RATE: f32 = 48_000.0;
/// One 60 fps frame of audio at [`RATE`], which is about what a shell hands
/// the analyzer per frame.
const FRAME_SAMPLES: usize = 800;
/// Frames run before any are counted, so the spectrogram's ring is full and
/// the galley cache warm. A cold frame is a real cost, but it happens once
/// and the question here is what the steady state costs.
const WARMUP: usize = 60;
/// The window every dock measurement runs at, in points.
const WINDOW: egui::Vec2 = egui::vec2(1600.0, 1000.0);

/// A chord's worth of audio, so the analyzer has structure to draw rather
/// than a flat floor — an empty spectrum thins out the heatmap's work and
/// would flatter every reading below.
fn chord_samples(n: usize, phase: &mut f64) -> Vec<f32> {
    let mut out = Vec::with_capacity(n);
    for _ in 0..n {
        let mut v = 0.0f64;
        for f in [220.0, 277.2, 330.0, 440.0, 554.4, 660.0] {
            v += (*phase * f * std::f64::consts::TAU).sin() * 0.12;
        }
        out.push(v as f32);
        *phase += 1.0 / RATE as f64;
    }
    out
}

/// Six voices held down for the whole run: enough to light nodes on the
/// lattice and give the roll something to scroll.
fn held_chord(state: &mut SharedState, now: f64) {
    for note in [57u8, 61, 64, 69, 73, 76] {
        state.tracker.handle_event(NoteEvent::on(now, 0, note, 0.8));
    }
}

/// The fastest and the median of a set of frame timings.
///
/// The FASTEST is the headline, which the perf overlay would never do — and
/// the difference is what the two are measuring. The overlay reads a plugin
/// inside a DAW, where the host's contention for the machine IS the frame
/// cost and averaging it away would hide the thing being watched. This runs
/// on a developer's machine beside a browser, three other builds and often
/// the DAW itself, where the same contention is noise about the machine
/// rather than about the code: a mean here moved by a factor of three
/// between two runs of the SAME build, and ranked variants in an order that
/// had "no roll" costing more than drawing one.
///
/// The minimum is the cost with nothing in the way, which is the only figure
/// that compares two builds on an unquiet machine. The median comes along to
/// say how far from it the run actually sat — the two close together means
/// the machine was quiet and both are the answer.
fn stats(mut v: Vec<f64>) -> (f64, f64) {
    v.sort_by(|a, b| a.partial_cmp(b).unwrap());
    (v[0], v[v.len() / 2])
}

/// One profiled run's fixture: the two things that separate a plugin sitting
/// idle in a project from one being played.
#[derive(Default, Clone, Copy)]
struct Load {
    /// Where the pointer sits every frame, which is what makes the lattice's
    /// pick and the spectral pane's gestures run at all.
    hover: Option<egui::Pos2>,
    /// Notes started per frame, each held a quarter second. Six is a busy
    /// passage — 360 notes a second through the roll.
    notes_per_frame: usize,
}

/// Drive the whole dock for a fixed run and time both halves of every frame.
fn profile(label: &str, ppp: f32, load: Load, tweak: impl Fn(&mut SharedState)) {
    const FRAMES: usize = 240;
    let mut state = fresh();
    tweak(&mut state);
    let mut h = DockHarness::at(WINDOW);
    // The DEVICE ratio, not the chrome scale: a Retina panel tessellates the
    // same shapes into four times the pixels.
    h.ctx.set_pixels_per_point(ppp);
    held_chord(&mut state, 0.0);

    let mut phase = 0.0f64;
    let (mut ui_ms, mut tess_ms) = (Vec::new(), Vec::new());
    let (mut shapes, mut verts, mut idx) = (0, 0, 0);

    for i in 0..(WARMUP + FRAMES) {
        // Everything fed in below is stamped at the clock the frame about to
        // run will read it at.
        let t = h.next_time();
        let audio = chord_samples(FRAME_SAMPLES, &mut phase);
        let cfg = state.spectrum_config;
        state.spectrum.push_samples(&audio, 1, RATE, t, &cfg);
        for k in 0..load.notes_per_frame {
            // Walking the keyboard rather than repeating one note: a name is
            // memoized per pitch class, so a stuck note would measure one
            // cache hit a frame instead of the naming work.
            let note = 36 + ((i * load.notes_per_frame + k) * 7 % 60) as u8;
            state.tracker.handle_event(NoteEvent::on(t, 0, note, 0.7));
            state.tracker.handle_event(NoteEvent::off(t + 0.25, 0, note));
        }

        let events = load.hover.map(|at| vec![egui::Event::PointerMoved(at)]).unwrap_or_default();
        let (out, ui) = h.frame_timed(&mut state, events);

        let n = out.shapes.len();
        let start = std::time::Instant::now();
        let prims = h.ctx.tessellate(out.shapes, out.pixels_per_point);
        let tess = start.elapsed().as_secs_f64() * 1000.0;

        if i >= WARMUP {
            ui_ms.push(ui);
            tess_ms.push(tess);
            shapes = n;
            verts = mesh_total(&prims, |m| m.vertices.len());
            idx = mesh_total(&prims, |m| m.indices.len());
        }
    }
    let (ui_min, ui_med) = stats(ui_ms);
    let (tess_min, tess_med) = stats(tess_ms);
    println!(
        "{label:26} ui {ui_min:6.3} (med {ui_med:6.3})  tess {tess_min:6.3} \
         (med {tess_med:6.3})  shapes {shapes:5}  verts {verts:6}  idx {idx:6}",
    );
}

fn mesh_total(
    prims: &[egui::ClippedPrimitive],
    of: impl Fn(&egui::epaint::Mesh) -> usize,
) -> usize {
    prims
        .iter()
        .map(|p| match &p.primitive {
            egui::epaint::Primitive::Mesh(mesh) => of(mesh),
            _ => 0,
        })
        .sum()
}

/// The whole dock, one frame at a time, across the things that move its cost.
#[test]
#[ignore]
fn profile_frame() {
    let idle = Load::default();
    let busy = Load { notes_per_frame: 6, ..Load::default() };
    // In the picture panes rather than in the settings column: the lattice's
    // pick and the spectral pane's gestures only run under a pointer.
    let on_lattice = Load { hover: Some(egui::pos2(400.0, 500.0)), ..Load::default() };
    let on_spectral = Load { hover: Some(egui::pos2(1100.0, 500.0)), ..Load::default() };

    println!("\n-- the whole dock at 1600x1000, ms per frame --");
    profile("idle @1x", 1.0, idle, |_| {});
    profile("idle @2x (retina)", 2.0, idle, |_| {});
    profile("idle, perf overlay on", 2.0, idle, |s| {
        s.view.show_perf = true;
        s.view.show_perf_detail = true;
    });
    profile("idle, no spectrogram", 2.0, idle, |s| s.spectrum_config.show_spectrogram = false);
    profile("idle, no roll", 2.0, idle, |s| s.spectrum_config.show_roll = false);
    profile("idle, no lattice labels", 2.0, idle, |s| s.view.show_labels = false);
    profile("idle, hover lattice", 2.0, on_lattice, |_| {});
    profile("idle, hover spectral", 2.0, on_spectral, |_| {});

    println!("-- a bigger lattice: the scene derivation is per NODE --");
    profile("sevens open (819 nodes)", 2.0, idle, |s| s.view.extent_sevens = 1);
    profile("3075 nodes", 2.0, idle, |s| {
        s.view.extent_threes = 20;
        s.view.extent_fives = 12;
        s.view.extent_sevens = 1;
    });

    println!("-- a busy passage: 6 notes a frame, each held a quarter second --");
    profile("busy", 2.0, busy, |_| {});
    profile("busy, no note names", 2.0, busy, |s| s.spectrum_config.note_names = false);
    profile("busy, no roll", 2.0, busy, |s| s.spectrum_config.show_roll = false);
    profile("busy, no lattice labels", 2.0, busy, |s| s.view.show_labels = false);
    profile("busy, 2 notes a frame", 2.0, Load { notes_per_frame: 2, ..idle }, |_| {});
    profile("busy, 12 notes a frame", 2.0, Load { notes_per_frame: 12, ..idle }, |_| {});
}

/// What a frame takes off the heap, by phase — the churn behind the timings,
/// which is where a p95 well clear of the mean usually comes from.
///
/// The spectrogram's fallback counters print alongside, because a run that
/// was quietly re-aggregating its whole window every frame would read as an
/// expensive heatmap rather than as a broken ring.
#[test]
#[ignore]
fn profile_allocations() {
    const FRAMES: usize = 200;
    fn mark() -> (usize, usize) {
        (
            ALLOCS.load(std::sync::atomic::Ordering::Relaxed),
            BYTES.load(std::sync::atomic::Ordering::Relaxed),
        )
    }

    let mut state = fresh();
    let mut h = DockHarness::at(WINDOW);
    h.ctx.set_pixels_per_point(2.0);
    held_chord(&mut state, 0.0);

    let mut phase = 0.0f64;
    let mut phases = [(0usize, 0usize); 3];
    let mut at_warmup = (0, [0; crate::spectrogram::Restart::COUNT]);

    for i in 0..(WARMUP + FRAMES) {
        let t = h.next_time();
        if i == WARMUP {
            at_warmup = state.spectrum.spectrogram_fallbacks();
        }
        let counting = i >= WARMUP;
        let mut charge = |slot: usize, from: (usize, usize)| {
            let now = mark();
            if counting {
                phases[slot].0 += now.0 - from.0;
                phases[slot].1 += now.1 - from.1;
            }
        };

        let at = mark();
        let audio = chord_samples(FRAME_SAMPLES, &mut phase);
        let cfg = state.spectrum_config;
        state.spectrum.push_samples(&audio, 1, RATE, t, &cfg);
        charge(0, at);

        let at = mark();
        let out = h.frame(&mut state, vec![]);
        charge(1, at);

        let at = mark();
        h.ctx.tessellate(out.shapes, out.pixels_per_point);
        charge(2, at);
    }

    println!("\n-- heap per frame --");
    for (name, (allocs, bytes)) in ["audio + fft", "root_ui", "tessellate"].iter().zip(phases) {
        println!(
            "{name:>12}  {:6} allocations  {:9.1} KB",
            allocs / FRAMES,
            bytes as f64 / FRAMES as f64 / 1024.0,
        );
    }
    let (rebuilds, restarts) = state.spectrum.spectrogram_fallbacks();
    let restarts: Vec<u32> =
        restarts.iter().zip(at_warmup.1).map(|(end, start)| end - start).collect();
    println!(
        "spectrogram over {FRAMES} frames: {} re-aggregations, restarts {restarts:?}",
        rebuilds - at_warmup.0,
    );
}

/// Each settings pane on its own, through ONE shared context — a fresh
/// `egui::Context` per call rasterizes the font atlas, which is milliseconds
/// and is not what a frame pays. Measured that way the panes read as ~2 ms
/// each, which is the atlas and not the pane.
#[test]
#[ignore]
fn profile_settings_panes() {
    let mut state = fresh();
    // The Video pane's record button and progress bar only exist with a take
    // backend behind them.
    state.take.supported = true;
    state.take.last_ready = true;
    let ctx = super::probe::themed();

    println!("\n-- one settings pane at 300 points wide, ms per frame --");
    for pane in SETTINGS_PANES {
        let tab = pane.install(&mut state);
        let mut samples = Vec::new();
        let mut shapes = 0;
        for i in 0..200 {
            let now = i as f64 / 60.0;
            let start = std::time::Instant::now();
            let out = tab_body_on(&ctx, &mut state, tab, 300.0, 900.0, now);
            let ms = start.elapsed().as_secs_f64() * 1000.0;
            shapes = out.shapes.len();
            if i >= 40 {
                samples.push(ms);
            }
        }
        let (min, med) = stats(samples);
        println!("{pane:>24?}  {min:6.3} ms (med {med:6.3})  shapes {shapes}");
    }
}

/// Each picture pane on its own, drawn the way the offline renderer draws
/// them — no dock, no tab bar.
#[test]
#[ignore]
fn profile_picture_panes() {
    println!("\n-- one picture pane at 1000x900, ms per frame --");
    for pane in [Pane::Lattice, Pane::Spectral] {
        let mut state = fresh();
        let backend = RecordingBackend::default();
        let ctx = super::probe::themed_at(2.0);
        let body = egui::vec2(1000.0, 900.0);
        held_chord(&mut state, 0.0);
        let mut phase = 0.0;
        let mut samples = Vec::new();
        for i in 0..(WARMUP + 240) {
            let t = i as f64 / 60.0;
            let audio = chord_samples(FRAME_SAMPLES, &mut phase);
            let cfg = state.spectrum_config;
            state.spectrum.push_samples(&audio, 1, RATE, t, &cfg);
            let start = std::time::Instant::now();
            let _ = super::probe::frame_into(
                &ctx,
                body,
                egui::Rect::from_min_size(egui::Pos2::ZERO, body),
                |ui| {
                    crate::begin_frame(&mut state, &backend, t);
                    draw_pane(ui, pane, &mut state, t);
                },
            );
            let ms = start.elapsed().as_secs_f64() * 1000.0;
            if i >= WARMUP {
                samples.push(ms);
            }
        }
        let (min, med) = stats(samples);
        println!("{pane:>10?}  {min:6.3} ms (med {med:6.3})");
    }
}

/// What the frame's triangles are made of, by shape kind — so a vertex count
/// has somewhere to come from, and a run that suddenly costs more
/// tessellation has somewhere to look.
#[test]
#[ignore]
fn profile_shape_census() {
    let mut state = fresh();
    let mut h = DockHarness::at(WINDOW);
    h.ctx.set_pixels_per_point(2.0);
    held_chord(&mut state, 0.0);

    let mut phase = 0.0;
    let mut out = None;
    for _ in 0..(WARMUP + 30) {
        let t = h.next_time();
        let audio = chord_samples(FRAME_SAMPLES, &mut phase);
        let cfg = state.spectrum_config;
        state.spectrum.push_samples(&audio, 1, RATE, t, &cfg);
        out = Some(h.frame(&mut state, vec![]));
    }

    let mut census: std::collections::BTreeMap<&str, (usize, usize, usize)> = Default::default();
    for clipped in &out.expect("a frame").shapes {
        let row = census.entry(shape_name(&clipped.shape)).or_default();
        row.0 += 1;
        // A callback is a handle to a wgpu pass, and the tessellator panics on
        // one rather than producing geometry: what it draws is never egui's.
        if matches!(clipped.shape, egui::Shape::Callback(_)) {
            continue;
        }
        let mut tessellator =
            egui::epaint::tessellator::Tessellator::new(2.0, Default::default(), [16, 16], vec![]);
        let mut mesh = egui::epaint::Mesh::default();
        tessellator.tessellate_shape(clipped.shape.clone(), &mut mesh);
        row.1 += mesh.vertices.len();
        row.2 += mesh.indices.len();
    }

    println!("\n-- one frame's shapes at 2x, by kind --");
    let mut rows: Vec<_> = census.into_iter().collect();
    rows.sort_by_key(|(_, (_, verts, _))| std::cmp::Reverse(*verts));
    for (name, (n, verts, idx)) in rows {
        println!("{name:>12}  n {n:4}  verts {verts:6}  idx {idx:6}");
    }
}

fn shape_name(shape: &egui::Shape) -> &'static str {
    match shape {
        egui::Shape::Noop => "noop",
        egui::Shape::Vec(_) => "vec",
        egui::Shape::Circle(_) => "circle",
        egui::Shape::Ellipse(_) => "ellipse",
        egui::Shape::LineSegment { .. } => "line",
        egui::Shape::Path(_) => "path",
        egui::Shape::Rect(_) => "rect",
        egui::Shape::Text(_) => "text",
        egui::Shape::Mesh(_) => "mesh",
        egui::Shape::QuadraticBezier(_) => "quad-bezier",
        egui::Shape::CubicBezier(_) => "cubic-bezier",
        egui::Shape::Callback(_) => "callback",
    }
}
