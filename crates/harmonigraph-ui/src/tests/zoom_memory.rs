//! The font-atlas capacity a full-range lattice zoom leaves behind.

use super::harness::*;
use crate::*;

struct Zoom {
    ctx: egui::Context,
    backend: RecordingBackend,
    screen: egui::Rect,
    t: f64,
}

impl Zoom {
    fn new() -> Self {
        Zoom {
            ctx: super::probe::themed_at(2.0),
            backend: RecordingBackend::default(),
            screen: egui::Rect::from_min_size(egui::pos2(0.0, 0.0), egui::vec2(1512.0, 886.0)),
            t: 0.0,
        }
    }

    fn frame(&mut self, state: &mut SharedState) {
        self.t += 1.0 / 144.0;
        let raw = egui::RawInput {
            screen_rect: Some(self.screen),
            time: Some(self.t),
            max_texture_side: Some(4096),
            ..Default::default()
        };
        let t = self.t;
        let backend = &self.backend;
        let _ = self.ctx.run_ui(raw, |ui| root_ui(ui, state, backend, t));
    }

    fn atlas(&self) -> [usize; 2] {
        self.ctx.fonts(|fonts| fonts.font_image_size())
    }

    fn fill(&self) -> f32 {
        self.ctx.fonts(|fonts| fonts.font_atlas_fill_ratio())
    }
}

fn played() -> SharedState {
    let mut state = fresh();
    state.view.show_labels = true;
    for key in 48..72 {
        state.tracker.handle_event(harmonigraph_core::NoteEvent::on(0.0, 0, key, 1.0));
    }
    state
}

/// A zoom walks the font-size ladder once, then reuses it in either direction.
///
/// The fixture names two octaves of played notes and crosses the camera's
/// entire range eight times. That is enough to grow the atlas well past its
/// 32-row seed; repeating the gesture is therefore testing retained glyph
/// variants rather than an idle pane that never reached the growing path.
#[test]
fn repeated_lattice_zooms_settle_at_sixteen_mib() {
    let mut state = played();
    let mut zoom = Zoom::new();
    for _ in 0..8 {
        zoom.frame(&mut state);
    }
    let mut after = Vec::new();
    for sweep in 0..8 {
        for frame in 0..72 {
            let t = frame as f32 / 71.0;
            state.camera.distance = if sweep % 2 == 0 {
                harmonigraph_scene::Camera::MIN_DISTANCE
                    + t * (harmonigraph_scene::Camera::MAX_DISTANCE
                        - harmonigraph_scene::Camera::MIN_DISTANCE)
            } else {
                harmonigraph_scene::Camera::MAX_DISTANCE
                    - t * (harmonigraph_scene::Camera::MAX_DISTANCE
                        - harmonigraph_scene::Camera::MIN_DISTANCE)
            };
            zoom.frame(&mut state);
        }
        after.push(zoom.atlas());
        assert!(
            zoom.fill() < 0.8,
            "sweep {sweep} left the atlas at {:.3}, due for an egui rebuild",
            zoom.fill(),
        );
    }
    assert!(after[0][1] >= 512, "the first sweep never reached atlas growth: {after:?}");
    assert_eq!(
        after.iter().copied().max(),
        Some([4096, 1024]),
        "a repeated zoom retained more than 16 MiB of font pixels: {after:?}",
    );
    assert_eq!(after[1..], [after[0]; 7], "the return trips kept growing the atlas");
}
