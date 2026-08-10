//! The render loop: a take in, frames out.
//!
//! One function, deliberately, because the whole claim of this crate is
//! that a frame depends on nothing but `now` and what has been fed in by
//! then. Keeping the loop in one place makes that auditable — there is
//! no hidden state between frames beyond the `SharedState` the plugin
//! also carries.

use harmonigraph_render::wgpu::TextureFormat;
use harmonigraph_ui::{begin_frame, draw_pane, Layout, SharedState};

use crate::wav::Audio;

use crate::frames::Renderer;
use crate::replay::Replay;

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
    /// Take time of the audio's first sample. Non-zero when recording
    /// was armed part-way into a song: without it the spectrum would
    /// read the wrong part of the bounce, by exactly however far in you
    /// started.
    pub audio_start: f64,
    /// Lay the whole take's spectrogram out at once and sweep a playhead
    /// across it, instead of the live scrolling window. Needs audio.
    pub whole_song_spectrogram: bool,
}

impl Settings {
    pub fn frame_count(&self) -> u64 {
        ((self.end - self.start).max(0.0) * self.fps).round() as u64
    }
}

/// Render every frame, handing each to `emit` as tightly packed RGBA8.
///
/// `emit` returning an error stops the render — that is how a dead encoder
/// gets reported rather than swallowed for another thousand frames. `emit`
/// returning `Ok(false)` also stops it, but cleanly: the encoder wants no more
/// frames (ffmpeg under `-shortest`, its soundtrack shorter than the visuals),
/// and the caller reads the true verdict from the encoder's exit status.
pub fn render(
    replay: &mut Replay,
    audio: Option<&Audio>,
    settings: &Settings,
    mut emit: impl FnMut(&[u8]) -> Result<bool, String>,
) -> Result<u64, String> {
    let mut renderer = Renderer::new(settings.size)
        .ok_or("no usable GPU adapter (this needs a real GPU, not a container)")?;

    let context = egui::Context::default();
    harmonigraph_ui::theme::apply_theme(&context);
    context.set_pixels_per_point(settings.pixels_per_point);

    let mut state = SharedState::new(TextureFormat::Rgba8Unorm);
    if let Some(blob) = replay.take().header.ui_state.clone() {
        // On stderr, because a refusal here is invisible otherwise: the
        // console the editor would log to is not drawn offline, and what a
        // refused blob produces is a render at wholly default camera, view
        // and frame — a plausible-looking mp4 that reproduces nothing about
        // the session it was recorded in. Better a line than a silent one.
        if !state.load_persist(&blob) {
            eprintln!(
                "warning: this take's ui_state was refused; rendering at defaults \
                 (camera, view, spectrum and frame)",
            );
        }
    }
    // Nothing offline is interactive, and both would draw over the
    // picture: no armed-mode pulse, no hover highlight.
    state.learn_active = false;
    state.hovered = None;
    // The comma auto-detects are interactive too, in the sense that matters
    // here: they answer a tuning EDIT, and a replay has no editor. Left on,
    // they judge the take's tuning afresh on frame 0 — and a session that
    // switched one off at a tuning that IS that temperament (12-TET, which is
    // both of them) would export with it on, respelling names and collapsing
    // comma-equivalent nodes the recorded session showed. The blob carries
    // what was on screen; the render's job is to reproduce it, not to
    // re-decide it.
    for comma in harmonigraph_core::Comma::ALL {
        *state.view.temper_auto_mut(comma) = false;
    }

    // Playhead mode: precompute the whole take's spectrogram from the full
    // audio, once, up front. It's a pure function of (audio, config, span), so
    // the per-frame draw just reads it and the render stays byte-identical
    // between runs. The live ring can't hold a whole song, hence the separate
    // precomputed set.
    // `--playhead` on the command line, or the take's own "Whole-song
    // playhead" render setting — either turns it on.
    if settings.whole_song_spectrogram || state.take.render_config.playhead {
        if let Some(audio) = audio {
            let span = (settings.end - settings.start).max(0.0);
            if span > 0.0 {
                state.whole_song = Some(harmonigraph_ui::WholeSong::precompute(
                    &audio.samples,
                    audio.channels,
                    audio.sample_rate,
                    settings.audio_start,
                    settings.start,
                    span,
                    &state.spectrum_config,
                ));
            }
        }
        // The whole take's notes, laid out from the start — the roll shows the
        // whole piece at once, not filling in as the playhead passes over it.
        if let Some(ws) = state.whole_song.as_mut() {
            ws.roll = replay.full_roll();
        }
    }

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
    // The same color as the scene's knockout ground. Offline clears to the
    // layout's background and paints the panes over it, rather than onto the
    // dock's panel — so without this the sevens gutter would clear to a
    // panel-colored disc several shades too light, and only in exported
    // video, which is the worst place to find out.
    state.set_background(settings.layout.background);

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
            // bounce actually is at `now`. The analyzer counts the samples it
            // is given and stamps its columns off `now`, both of which come
            // from the frame index — so the spectrum is as deterministic as
            // everything else, and its column rate does not depend on the
            // render's fps the way a frame-clock throttle would make it.
            let chunk = audio.slice_seconds(now - settings.audio_start, now + step - settings.audio_start);
            if !chunk.is_empty() {
                let config = state.spectrum_config;
                state.spectrum.push_samples(chunk, audio.channels, audio.sample_rate, now, &config);
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
                // Last, over the gaps the panes left: the seam that keeps the
                // lattice and the spectral pane from reading as one field.
                settings.layout.paint_dividers(ui.painter(), &placements);
            },
        );

        let primitives = context.tessellate(output.shapes, settings.pixels_per_point);
        let bytes = renderer.render(
            &primitives,
            &output.textures_delta,
            settings.pixels_per_point,
            background,
        );
        if !emit(&bytes)? {
            // The encoder wants no more frames (e.g. ffmpeg under -shortest,
            // the soundtrack ending before the visuals). Stop here; the caller
            // reads whether that was a clean finish from the exit status.
            return Ok(frame);
        }
    }
    Ok(frames)
}

#[cfg(test)]
mod tests {
    use super::*;
    use harmonigraph_take::{Header, NoteKind, NoteRecord, Take};

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
            audio_start: 0.0,
            whole_song_spectrogram: false,
        }
    }

    fn render_frames(settings: &Settings) -> Option<Vec<Vec<u8>>> {
        let mut replay = Replay::new(take());
        let mut frames = Vec::new();
        match render(&mut replay, None, settings, |bytes| {
            frames.push(bytes.to_vec());
            Ok(true)
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

    /// Whole-song playhead mode: the spectrogram is precomputed from the whole
    /// audio up front, so it must stay as reproducible as the scrolling view,
    /// and the playhead must actually sweep.
    #[test]
    fn whole_song_playhead_render_is_deterministic_and_moves() {
        // A synthetic tone, so the precomputed spectrogram has content to lay
        // out across the frame.
        let sr = 48_000.0f32;
        let n = (sr as f64) as usize; // one second
        let samples: Vec<f32> =
            (0..n).map(|i| 0.6 * (std::f32::consts::TAU * 440.0 * i as f32 / sr).sin()).collect();
        let audio = Audio { sample_rate: sr, samples, channels: 1 };

        let mut settings = settings();
        settings.whole_song_spectrogram = true;
        settings.layout = Layout::preset("spectral").unwrap();

        let run = || -> Option<Vec<Vec<u8>>> {
            let mut replay = Replay::new(take());
            let mut frames = Vec::new();
            match render(&mut replay, Some(&audio), &settings, |bytes| {
                frames.push(bytes.to_vec());
                Ok(true)
            }) {
                Ok(_) => Some(frames),
                Err(e) if e.contains("no usable GPU adapter") => {
                    eprintln!("skipping: {e}");
                    None
                }
                Err(e) => panic!("{e}"),
            }
        };

        let Some(first) = run() else { return };
        let second = run().expect("second run also has a GPU");
        assert_eq!(first.len(), second.len());
        for (i, (a, b)) in first.iter().zip(&second).enumerate() {
            assert!(a == b, "whole-song frame {i} differs between runs");
        }
        assert!(first[0] != first[first.len() / 2], "the playhead should sweep across the frame");
    }
}
