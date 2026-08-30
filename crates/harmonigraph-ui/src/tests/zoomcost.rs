//! TEMPORARY measurement: what a rapid zoom on the lattice costs per frame.

use super::harness::*;
use crate::*;

/// A frame loop of our own rather than [`DockHarness`], for the two inputs the
/// measurement is about: the device scale the plugin really runs at, and the
/// `max_texture_side` a shell reports off its wgpu device — which is what
/// decides how wide egui builds its font atlas, and so what one publication of
/// it copies.
struct Zoom {
    ctx: egui::Context,
    backend: RecordingBackend,
    screen: egui::Rect,
    side: usize,
    t: f64,
}

impl Zoom {
    fn new(ppp: f32, side: usize) -> Self {
        Zoom {
            ctx: super::probe::themed_at(ppp),
            backend: RecordingBackend::default(),
            screen: egui::Rect::from_min_size(egui::pos2(0.0, 0.0), egui::vec2(1512.0, 886.0)),
            side,
            t: 0.0,
        }
    }

    fn frame(&mut self, state: &mut SharedState, events: Vec<egui::Event>) -> f64 {
        self.t += 1.0 / 144.0;
        let raw = egui::RawInput {
            screen_rect: Some(self.screen),
            time: Some(self.t),
            max_texture_side: Some(self.side),
            events,
            ..Default::default()
        };
        let t = self.t;
        let backend = &self.backend;
        let mut ms = 0.0;
        // The `FullOutput` is dropped: what this measures is the UI's own
        // frame, and the shapes are a shell's half of it.
        let _ = self.ctx.run_ui(raw, |ui| {
            let start = std::time::Instant::now();
            root_ui(ui, state, backend, t);
            ms = start.elapsed().as_secs_f64() * 1000.0;
        });
        ms
    }

    fn atlas(&self) -> [usize; 2] {
        self.ctx.fonts(|f| f.font_image_size())
    }

    fn fill(&self) -> f32 {
        self.ctx.fonts(|f| f.font_atlas_fill_ratio())
    }
}

fn wheel(at: egui::Pos2, dy: f32) -> Vec<egui::Event> {
    vec![
        egui::Event::PointerMoved(at),
        egui::Event::MouseWheel {
            unit: egui::MouseWheelUnit::Line,
            delta: egui::vec2(0.0, dy),
            phase: egui::TouchPhase::Move,
            modifiers: egui::Modifiers::NONE,
        },
    ]
}

/// A lattice with names on it: a spread of held notes, which under the fresh
/// `NoteNames::Past` is what puts a name on a node at all.
fn played() -> SharedState {
    let mut state = fresh();
    state.view.show_labels = true;
    for key in 48..72 {
        state.tracker.handle_event(harmonigraph_core::NoteEvent::on(0.0, 0, key, 1.0));
    }
    state
}

fn stats() -> (usize, usize, usize, usize) {
    use std::sync::atomic::Ordering::Relaxed;
    (
        crate::text::probe::PUBLISHES.load(Relaxed),
        crate::text::probe::TEXELS.load(Relaxed),
        crate::text::probe::MARK_PUBLISHES.load(Relaxed),
        crate::text::probe::MARK_TEXELS.load(Relaxed),
    )
}

#[test]
#[ignore = "a measurement, not an assertion: cargo test -- --ignored --nocapture measure_zoom_cost"]
fn measure_zoom_cost() {
    let mut state = played();
    let mut h = Zoom::new(2.0, 8192);
    let at = egui::pos2(200.0, 200.0);
    for _ in 0..8 {
        h.frame(&mut state, vec![egui::Event::PointerMoved(at)]);
    }
    println!("warm: atlas {:?} fill {:.3}", h.atlas(), h.fill());

    // Baselines: the camera standing still, at each end of the range. What
    // these separate is the two things a zoom changes at once — how MANY nodes
    // are on screen, and the churn of walking between sizes. A still camera
    // zoomed out has the node count without any of the churn.
    for distance in [2.0, 6.0, 17.9] {
        state.camera.distance = distance;
        crate::text::probe::reset();
        for _ in 0..20 {
            h.frame(&mut state, vec![egui::Event::PointerMoved(at)]);
        }
        let mut still = 0.0;
        for _ in 0..120 {
            still += h.frame(&mut state, vec![egui::Event::PointerMoved(at)]);
        }
        let (p, tx, mp, _) = stats();
        println!(
            "STILL at {distance:5.2}: {:.2} ms/frame, {p} font + {mp} mark publishes, \
             {:.1} MB, atlas {:?} fill {:.3}",
            still / 120.0,
            tx as f64 * 4.0 / 1e6,
            h.atlas(),
            h.fill()
        );
    }

    // A rapid zoom over the WHOLE range, in and out. The clamp is at
    // distance 2 and the far end near 15, and a gesture that crosses it in
    // half a second is what the complaint is about — so the sweep has to
    // cross it, and the per-frame step has to stay a walk through the sizes
    // rather than a jump over them.
    crate::text::probe::reset();
    let mut per_frame: Vec<(f64, usize)> = Vec::new();
    for sweep in 0..8 {
        let dy = if sweep % 2 == 0 { 0.45 } else { -0.45 };
        for _ in 0..72 {
            let before = stats().0;
            let ms = h.frame(&mut state, wheel(at, dy));
            per_frame.push((ms, stats().0 - before));
        }
        let (p, tx, mp, mtx) = stats();
        println!(
            "  sweep {sweep}: distance {:5.2}, atlas {:?} fill {:.3}, font {p} publishes \
             {:.1} MB, marks {mp} publishes {:.1} MB",
            state.camera.distance,
            h.atlas(),
            h.fill(),
            tx as f64 * 4.0 / 1e6,
            mtx as f64 * 4.0 / 1e6
        );
    }
    let (p, tx, mp, mtx) = stats();
    let frames = per_frame.len();
    let total: f64 = per_frame.iter().map(|(ms, _)| ms).sum();
    let quiet: Vec<f64> = per_frame.iter().filter(|(_, n)| *n == 0).map(|(ms, _)| *ms).collect();
    let busy: Vec<f64> = per_frame.iter().filter(|(_, n)| *n > 0).map(|(ms, _)| *ms).collect();
    let mean = |v: &[f64]| if v.is_empty() { 0.0 } else { v.iter().sum::<f64>() / v.len() as f64 };
    let mut sorted: Vec<f64> = per_frame.iter().map(|(ms, _)| *ms).collect();
    sorted.sort_by(|a, b| a.partial_cmp(b).unwrap());
    println!(
        "ZOOMING {:.2} ms/frame over {frames} frames (p50 {:.2}, p99 {:.2}, max {:.2})\n\
         \x20 {} frames published an atlas at {:.2} ms mean; {} did not, at {:.2} ms\n\
         \x20 font atlas: {p} publishes, {:.1} MB copied ({:.2} MB/frame), atlas {:?} fill {:.3}\n\
         \x20 mark sheet: {mp} publishes, {:.1} MB copied, {} repacks, \
            {} marks rasterized, sheet {} tall",
        total / frames as f64,
        sorted[frames / 2],
        sorted[frames * 99 / 100],
        sorted[frames - 1],
        busy.len(),
        mean(&busy),
        quiet.len(),
        mean(&quiet),
        tx as f64 * 4.0 / 1e6,
        tx as f64 * 4.0 / 1e6 / frames as f64,
        h.atlas(),
        h.fill(),
        mtx as f64 * 4.0 / 1e6,
        crate::text::probe::REPACKS.load(std::sync::atomic::Ordering::Relaxed),
        crate::text::probe::PACKED.load(std::sync::atomic::Ordering::Relaxed),
        crate::text::probe::SHEET_H.load(std::sync::atomic::Ordering::Relaxed),
    );
}

/// What happens as the set of sizes keeps growing — which is what a session
/// does, since the label size is a function of the camera, the pane and the
/// Name size bar, and all three move.
#[test]
#[ignore = "a measurement, not an assertion: cargo test -- --ignored --nocapture measure_atlas_growth"]
fn measure_atlas_growth() {
    let mut state = played();
    let mut h = Zoom::new(2.0, 8192);
    let at = egui::pos2(200.0, 200.0);
    for _ in 0..8 {
        h.frame(&mut state, vec![egui::Event::PointerMoved(at)]);
    }

    let mut worst: f64 = 0.0;
    for round in 0..24 {
        // A different Name size each round, so each zoom walks a set of rungs
        // the atlas has not seen.
        state.view.label_scale = 0.7 + round as f32 * 0.05;
        crate::text::probe::reset();
        let mut ms = 0.0;
        let mut peak: f64 = 0.0;
        for sweep in 0..2 {
            let dy = if sweep % 2 == 0 { 0.45 } else { -0.45 };
            for _ in 0..72 {
                let f = h.frame(&mut state, wheel(at, dy));
                ms += f;
                peak = peak.max(f);
                worst = worst.max(f);
            }
        }
        let (p, tx, ..) = stats();
        let clone_ms =
            crate::text::probe::CLONE_NS.load(std::sync::atomic::Ordering::Relaxed) as f64 / 1e6;
        println!(
            "round {round:2} scale {:.2}: {:.2} ms/frame peak {peak:6.2}, {p} publishes \
             {:6.1} MB, clone {clone_ms:6.1} ms of {:6.1} ms total, atlas {:?} fill {:.3}",
            state.view.label_scale,
            ms / 144.0,
            tx as f64 * 4.0 / 1e6,
            ms,
            h.atlas(),
            h.fill()
        );
    }
    println!("worst frame over the whole run: {worst:.2} ms");
}
