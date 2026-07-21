//! The render loop: a take in, frames out.
//!
//! One function, deliberately, because the whole claim of this crate is
//! that a frame depends on nothing but `now` and what has been fed in by
//! then. Keeping the loop in one place makes that auditable — there is
//! no hidden state between frames beyond the `SharedState` the plugin
//! also carries.

use lattice_render::wgpu::TextureFormat;
use lattice_ui::{begin_frame, draw_pane, SharedState};

use crate::frames::Renderer;
use crate::layout::Layout;
use crate::replay::Replay;
use crate::wav::Audio;

/// Everything the loop needs that isn't the take itself.
pub struct Settings {
    pub layout: Layout,
    /// Output size in physical pixels.
    pub size: [u32; 2],
    /// Pixels per point — the UI's "zoom". Font sizes and paddings are
    /// in points, so this decides how big the text is *relative to the
    /// frame*, not just how sharp it is.
    pub pixels_per_point: f32,
    pub fps: f64,
    pub start: f64,
    pub end: f64,
}

impl Settings {
    pub fn frame_count(&self) -> u64 {
        ((self.end - self.start).max(0.0) * self.fps).round() as u64
    }
}

/// Render every frame, handing each to `emit` as tightly packed RGBA8.
///
/// `emit` returning an error stops the render — that is how a dead
/// encoder gets reported rather than swallowed for another thousand
/// frames.
pub fn render(
    replay: &mut Replay,
    audio: Option<&Audio>,
    settings: &Settings,
    mut emit: impl FnMut(&[u8]) -> Result<(), String>,
) -> Result<u64, String> {
    let mut renderer = Renderer::new(settings.size)
        .ok_or("no usable GPU adapter (this needs a real GPU, not a container)")?;

    let context = egui::Context::default();
    lattice_ui::theme::apply_theme(&context);
    context.set_pixels_per_point(settings.pixels_per_point);

    let mut state = SharedState::new(TextureFormat::Rgba8Unorm);
    if let Some(blob) = replay.take().header.ui_state.clone() {
        state.load_persist(&blob);
    }
    // Nothing offline is interactive, and both would draw over the
    // picture: no armed-mode pulse, no hover highlight.
    state.learn_active = false;
    state.hovered = None;

    let points = egui::vec2(
        settings.size[0] as f32 / settings.pixels_per_point,
        settings.size[1] as f32 / settings.pixels_per_point,
    );
    let screen = egui::Rect::from_min_size(egui::Pos2::ZERO, points);
    let placements = settings.layout.resolve(points);
    if placements.is_empty() {
        return Err("the layout resolved to no panes".into());
    }
    let background = egui::Color32::from_rgb(
        settings.layout.background.0,
        settings.layout.background.1,
        settings.layout.background.2,
    );

    let frames = settings.frame_count();
    let step = 1.0 / settings.fps;
    for frame in 0..frames {
        // Time is computed from the frame index rather than accumulated,
        // so a long render can't drift off the audio by accumulated
        // floating-point error.
        let now = settings.start + frame as f64 * step;

        replay.advance_to(&mut state, now);
        if let Some(audio) = audio {
            // Exactly one frame's worth of samples, taken from where the
            // bounce actually is at `now`. The analyzer's own throttling
            // and smoothing key off the `now` we hand it, so this makes
            // the spectrum as deterministic as everything else.
            let chunk = audio.slice_seconds(now, now + step);
            if !chunk.is_empty() {
                state.spectrum.push_samples(chunk, audio.sample_rate, now);
            }
        }
        begin_frame(&mut state, &replay.params, now);

        // No panels and no dock: the layout owns the frame, and the
        // background is the render pass's clear color rather than a
        // painted rect.
        let output = context.run_ui(
            egui::RawInput {
                screen_rect: Some(screen),
                time: Some(now),
                ..Default::default()
            },
            |ui| {
                for (pane, rect) in &placements {
                    let mut child = ui.new_child(egui::UiBuilder::new().max_rect(*rect));
                    draw_pane(&mut child, *pane, &mut state, now);
                }
            },
        );

        let primitives = context.tessellate(output.shapes, settings.pixels_per_point);
        let bytes = renderer.render(
            &primitives,
            &output.textures_delta,
            settings.pixels_per_point,
            background,
        );
        emit(&bytes)?;
    }
    Ok(frames)
}

#[cfg(test)]
mod tests {
    use super::*;
    use lattice_take::{Header, NoteKind, NoteRecord, Take};

    fn take() -> Take {
        let mut notes = Vec::new();
        // A little chord progression with a bend, so the render exercises
        // held voices, releases, the roll and the pitch axis.
        for (i, chord) in [[60u8, 64, 67], [62, 65, 69], [59, 62, 67]].iter().enumerate() {
            let t = i as f64 * 0.7;
            for &note in chord {
                notes.push(NoteRecord {
                    t,
                    channel: 0,
                    note,
                    kind: NoteKind::On { velocity: 0.8 },
                });
            }
            notes.push(NoteRecord {
                t: t + 0.2,
                channel: 0,
                note: chord[0],
                kind: NoteKind::Tuning { semitones: 0.25 },
            });
            for &note in chord {
                notes.push(NoteRecord { t: t + 0.6, channel: 0, note, kind: NoteKind::Off });
            }
        }
        Take { header: Header::default(), notes, params: Vec::new(), truncated: false }
    }

    fn settings() -> Settings {
        Settings {
            layout: Layout::preset("side-by-side").unwrap(),
            // Small and 256-aligned-friendly; the point is the pipeline,
            // not the resolution.
            size: [320, 200],
            pixels_per_point: 1.0,
            fps: 10.0,
            start: 0.0,
            end: 1.0,
        }
    }

    fn render_frames(settings: &Settings) -> Option<Vec<Vec<u8>>> {
        let mut replay = Replay::new(take());
        let mut frames = Vec::new();
        match render(&mut replay, None, settings, |bytes| {
            frames.push(bytes.to_vec());
            Ok(())
        }) {
            Ok(_) => Some(frames),
            // CI without a GPU: the pipeline can't be exercised at all,
            // and a hard failure there would be noise, not signal.
            Err(e) if e.contains("no usable GPU adapter") => {
                eprintln!("skipping: {e}");
                None
            }
            Err(e) => panic!("{e}"),
        }
    }

    #[test]
    fn a_take_renders_the_expected_number_of_frames_and_they_are_not_blank() {
        let settings = settings();
        let Some(frames) = render_frames(&settings) else { return };
        assert_eq!(frames.len() as u64, settings.frame_count());
        assert_eq!(frames[0].len(), 320 * 200 * 4);
        // A frame mid-progression must have drawn something: more than a
        // handful of distinct pixel values means real content, not a
        // flat clear.
        let mid = &frames[frames.len() / 2];
        let distinct: std::collections::HashSet<&[u8]> = mid.chunks(4).collect();
        assert!(distinct.len() > 32, "frame looks blank ({} distinct pixels)", distinct.len());
    }

    /// The claim the whole approach rests on: the same take renders to
    /// the same bytes. If this ever fails, something time- or
    /// machine-dependent has crept into the draw path, and every render
    /// after it silently stops being reproducible.
    #[test]
    fn rendering_the_same_take_twice_is_byte_identical() {
        let settings = settings();
        let Some(first) = render_frames(&settings) else { return };
        let second = render_frames(&settings).expect("second run also has a GPU");
        assert_eq!(first.len(), second.len());
        for (i, (a, b)) in first.iter().zip(&second).enumerate() {
            assert!(a == b, "frame {i} differs between two runs of the same take");
        }
    }

    /// Frames must actually change over time — a determinism test alone
    /// would pass just as happily on a stuck picture.
    #[test]
    fn the_picture_moves_as_the_take_plays() {
        let settings = settings();
        let Some(frames) = render_frames(&settings) else { return };
        assert!(frames[0] != frames[frames.len() / 2], "nothing changed as notes arrived");
    }
}
