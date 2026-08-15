//! Scratch measurement for the audit: the CPU cost of building a lattice
//! paint callback (the back-to-front sort, cull and instance packing in
//! `from_scene`) at several scene sizes. Run with
//! `cargo run --release -p harmonigraph-render --example callback_bench`.

use std::time::Instant;

use harmonigraph_core::{NoteEvent, NoteTracker, Tuning};
use harmonigraph_render::{lattice_paint_callback, LatticeLabels};
use harmonigraph_scene::{derive_scene, Camera, FrameParams, ViewConfig};

fn time_callback(label: &str, view: &ViewConfig, tracker: &NoteTracker) {
    let tuning = Tuning::default();
    let frame = FrameParams::default();
    let now = 10_000.0;
    let scene = derive_scene(tracker, &tuning, view, &frame, Camera::default(), None, now);
    let rect = egui::Rect::from_min_size(egui::pos2(0.0, 0.0), egui::vec2(1200.0, 800.0));
    let format = harmonigraph_render::wgpu::TextureFormat::Bgra8Unorm;
    for _ in 0..3 {
        let _ = lattice_paint_callback(rect, &scene, LatticeLabels::default(), format, 0, None);
    }
    let reps = 20;
    let start = Instant::now();
    for _ in 0..reps {
        let _ = lattice_paint_callback(rect, &scene, LatticeLabels::default(), format, 0, None);
    }
    let ms = start.elapsed().as_secs_f64() * 1000.0 / f64::from(reps);
    println!("{label:<40} {:>6} nodes  {ms:>7.3} ms/frame", scene.nodes.len());
}

fn main() {
    let default_view = ViewConfig::default();
    let huge = ViewConfig {
        extent_threes: 40,
        extent_fives: 20,
        extent_sevens: 2,
        ..ViewConfig::default()
    };

    let idle = NoteTracker::new();
    let mut chord = NoteTracker::new();
    for note in [48u8, 52, 55, 59, 62, 65, 69, 72, 76, 79] {
        chord.handle_event(NoteEvent::on(9_999.5, 0, note, 0.9));
    }

    time_callback("idle, default window", &default_view, &idle);
    time_callback("idle, huge window", &huge, &idle);
    time_callback("10-note chord, default window", &default_view, &chord);
    time_callback("10-note chord, huge window", &huge, &chord);
}
