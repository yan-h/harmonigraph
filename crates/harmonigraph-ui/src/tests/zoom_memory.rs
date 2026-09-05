//! The font-atlas capacity a full-range lattice zoom leaves behind.

use super::harness::*;
use crate::*;

struct Zoom {
    ctx: egui::Context,
    backend: RecordingBackend,
    screen: egui::Rect,
    t: f64,
}

#[derive(Clone, Copy, Debug)]
struct AtlasFrame {
    size: [usize; 2],
    fill: f32,
    rebuilt: bool,
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

    fn frame(&mut self, state: &mut SharedState) -> AtlasFrame {
        self.t += 1.0 / 144.0;
        let mut raw = egui::RawInput {
            screen_rect: Some(self.screen),
            time: Some(self.t),
            max_texture_side: Some(8192),
            ..Default::default()
        };
        crate::shell::limit_font_atlas(&mut raw);
        let t = self.t;
        let backend = &self.backend;
        let output = self.ctx.run_ui(raw, |ui| root_ui(ui, state, backend, t));
        AtlasFrame {
            size: self.atlas(),
            fill: self.fill(),
            rebuilt: output
                .textures_delta
                .set
                .iter()
                .any(|(id, delta)| *id == egui::TextureId::default() && delta.pos.is_none()),
        }
    }

    fn atlas(&self) -> [usize; 2] {
        self.ctx.fonts(|fonts| fonts.font_image_size())
    }

    fn fill(&self) -> f32 {
        self.ctx.fonts(|fonts| fonts.font_atlas_fill_ratio())
    }
}

fn zoom_distances() -> Vec<f32> {
    let min = harmonigraph_scene::Camera::MIN_DISTANCE;
    let max = harmonigraph_scene::Camera::MAX_DISTANCE;
    let steps = ((max / min).ln() / crate::text::SIZE_STEP.ln()).ceil() as usize + 1;
    (0..=steps).map(|step| min * (max / min).powf(step as f32 / steps as f32)).collect()
}

fn played() -> SharedState {
    let mut state = fresh();
    state.view.show_labels = true;
    for key in 48..72 {
        state.tracker.handle_event(harmonigraph_core::NoteEvent::on(
            0.0,
            harmonigraph_core::SourceId::DIRECT,
            0,
            key,
            1.0,
        ));
    }
    state
}

/// A zoom walks every font-size rung once, then reuses it in either direction.
///
/// The fixture names two octaves of played notes and crosses the camera's
/// entire range eight times. That is enough to grow the atlas well past its
/// 32-row seed; repeating the gesture is therefore testing retained glyph
/// variants rather than an idle pane that never reached the growing path.
fn assert_repeated_zooms_settle(label_scale: f32, expected_maximum: [usize; 2]) {
    let mut state = played();
    state.view.label_scale = label_scale;
    let mut zoom = Zoom::new();
    for _ in 0..8 {
        let _ = zoom.frame(&mut state);
    }
    let distances = zoom_distances();
    let mut maximum = [0, 0];
    let mut settled = None;
    for sweep in 0..8 {
        let walk: Box<dyn Iterator<Item = &f32>> = if sweep % 2 == 0 {
            Box::new(distances.iter())
        } else {
            Box::new(distances.iter().rev())
        };
        let mut previous_fill = zoom.fill();
        for &distance in walk {
            state.camera.distance = distance;
            let frame = zoom.frame(&mut state);
            if frame.size[0] * frame.size[1] > maximum[0] * maximum[1] {
                maximum = frame.size;
            }
            if sweep > 0 {
                assert!(
                    !frame.rebuilt,
                    "Name size {label_scale}, sweep {sweep} rebuilt the font atlas at {distance}: \
                     {frame:?}"
                );
                assert!(
                    frame.fill + f32::EPSILON >= previous_fill,
                    "Name size {label_scale}, sweep {sweep} cleared the font atlas at {distance}: \
                     {previous_fill:.3} -> {frame:?}",
                );
            }
            previous_fill = frame.fill;
        }
        settled.get_or_insert(zoom.atlas());
        assert!(
            zoom.fill() < 0.8,
            "Name size {label_scale}, sweep {sweep} left the atlas at {:.3}, due for an egui rebuild",
            zoom.fill(),
        );
    }
    let settled = settled.unwrap();
    assert!(settled[1] >= 512, "the first sweep never reached atlas growth: {settled:?}");
    assert_eq!(maximum, expected_maximum, "Name size {label_scale} reached an unexpected atlas");
    assert_eq!(zoom.atlas(), settled, "the return trips kept growing the atlas");
}

#[test]
fn repeated_lattice_zooms_reuse_the_bounded_atlas() {
    assert_repeated_zooms_settle(
        harmonigraph_scene::ViewConfig::default().label_scale,
        [4096, 2048],
    );
    assert_repeated_zooms_settle(3.0, [4096, 4096]);
}
