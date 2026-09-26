use harmonigraph_core::{NoteEvent, NoteTracker, SourceId, Tuning};
use harmonigraph_scene::{derive_scene, Camera, FrameParams, NodeMotion, RingFade, ViewConfig};
use std::{hint::black_box, time::Instant};
extern "C" {
    fn clock() -> std::os::raw::c_ulong;
}
fn main() {
    for (label, distance, depth) in [
        ("default", 12.0, 0),
        ("zoomed out", 24.0, 0),
        ("zoomed out nine sheets", 24.0, 4),
    ] {
        for (voices, events) in [(10, 0), (10, 1), (10, 8), (64, 0)] {
            let view = ViewConfig {
                min_sevens: -depth,
                max_sevens: depth,
                ..Default::default()
            };
            let camera = Camera {
                distance,
                cabinet_scale: 1.0,
                ..Default::default()
            };
            let frame = FrameParams::default();
            let env = view.envelope(&frame);
            let tuning = Tuning::default();
            let fade = RingFade::default();
            let mut tracker = NoteTracker::new();
            for i in 0..voices {
                tracker.handle_event(NoteEvent::on(0.0, SourceId::DIRECT, 0, 24 + i, 0.8));
            }
            let window = view.scrolled(&camera, 16.0 / 9.0);
            let mut scene = derive_scene(&tracker, &tuning, &view, &window, &frame, camera, None);
            let mut motion = NodeMotion::default();
            motion.step(&mut scene, &tracker, &tuning, &view, &env, &fade, 2.0);
            let mut times = vec![];
            for step in 1..=40 {
                let now = 2.0 + f64::from(step) / 60.0;
                for e in 0..events {
                    let at = now - (f64::from(events - e) - 0.5) / f64::from(events) / 60.0;
                    let key = 100 + e as u8;
                    tracker.handle_event(NoteEvent::on(at, SourceId::DIRECT, 1, key, 0.8));
                    tracker.handle_event(NoteEvent::off(at + 0.00001, SourceId::DIRECT, 1, key));
                }
                let started = Instant::now();
                let cpu = unsafe { clock() };
                motion.step(&mut scene, &tracker, &tuning, &view, &env, &fade, now);
                let _wall = started.elapsed().as_secs_f64() * 1000.0;
                let ms = (unsafe { clock() } - cpu) as f64 / 1000.0;
                black_box(&scene);
                if step > 10 {
                    times.push(ms);
                }
            }
            times.sort_by(f64::total_cmp);
            println!("{label} actual_nodes={} voices={voices} stabs_per_frame={events} motion_cpu_ms p50={:.3} p95={:.3}",scene.nodes.len(),times[15],times[28]);
        }
    }
}
