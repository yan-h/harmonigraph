//! The render loop: a take in, frames out.
//!
//! One function, deliberately, because the whole claim of this crate is
//! that a frame depends on nothing but `now` and what has been fed in by
//! then. Keeping the loop in one place makes that auditable — there is
//! no hidden state between frames beyond the `PictureState` the plugin
//! also carries.

use harmonigraph_render::wgpu::TextureFormat;
use harmonigraph_ui::{
    begin_frame, draw_pane, AppearanceDocument, Layout, PictureState, SpectrogramRender,
};

use crate::wav::Audio;

use crate::frames::Renderer;
use crate::replay::Replay;

use std::time::{Duration, Instant};

/// Where an export's wall clock went.
///
/// Four stages, chosen so each one names something that could be done about
/// it, and the whole loop accounted for between them:
///
/// - `ui` — the replay's advance, the analyzer feed, egui's own pass and
///   tessellation. Everything on the CPU before the GPU hears about the frame.
/// - `submit` — building the frame's command buffers and handing them over
///   ([`FrameCost::submit`](crate::frames::FrameCost::submit)).
/// - `readback` — waiting for the GPU and unpadding the result
///   ([`FrameCost::readback`](crate::frames::FrameCost::readback)).
/// - `emit` — handing the finished bytes to the sink.
///
/// `wall` is the loop's own elapsed time, so the four shares are of something
/// they can add up to; the export's TOTAL is longer by whatever setup came
/// before the loop and by the encoder's backlog after it, and the gap between
/// the two is the part of a render that is ffmpeg rather than this crate.
#[derive(Clone, Copy, Default)]
pub struct Stages {
    /// Frames DRAWN, which is what the stage costs are per. Short of
    /// `frame_count()` when the encoder stopped early, and one more than the
    /// video holds in that case: the frame that found the pipe shut was drawn
    /// and paid for like any other.
    pub frames: u64,
    pub wall: Duration,
    pub ui: Duration,
    pub submit: Duration,
    pub readback: Duration,
    pub emit: Duration,
}

impl Stages {
    /// One line for the end of an export: the wall clock, the rate, and each
    /// stage as milliseconds per frame and a share of the loop.
    ///
    /// It must not contain the substring `" frames"`, and that is a CONTRACT
    /// rather than a style choice. The plugin's Video pane drives its progress
    /// bar off this same stderr, and `harmonigraph_record::parse_report` reads
    /// a count out of any segment carrying that substring — so a summary line
    /// spelling the word out would land in the bar as a frame count and
    /// retarget it, after the render is over and nothing is left to correct
    /// it. `the_timing_summary_avoids_the_progress_bars_substring` holds this.
    pub fn summary(&self, total: Duration) -> String {
        let Some(per) = (self.frames > 0).then_some(self.frames as f64) else {
            return format!("timing: nothing drawn, {:.1} s spent", total.as_secs_f64());
        };
        let share = |stage: Duration| {
            let of = self.wall.as_secs_f64();
            if of > 0.0 {
                100.0 * stage.as_secs_f64() / of
            } else {
                0.0
            }
        };
        let stage = |name: &str, took: Duration| {
            format!(
                "{name} {:.2} ms/frame ({:.0}%)",
                1000.0 * took.as_secs_f64() / per,
                share(took)
            )
        };
        format!(
            "timing: a {}-frame export in {:.1} s, {:.1} s of it drawing at {:.1} fps — {}, {}, {}, {}",
            self.frames,
            total.as_secs_f64(),
            self.wall.as_secs_f64(),
            per / self.wall.as_secs_f64().max(f64::MIN_POSITIVE),
            stage("ui+tess", self.ui),
            stage("submit", self.submit),
            stage("readback", self.readback),
            stage("emit", self.emit),
        )
    }
}

/// Select and normalize once before output setup. Explicit re-render settings
/// replace the recorded document in full. Refusal retains the existing default
/// rendering policy and reports it where an offline user can see it.
pub fn appearance_for(
    take: &harmonigraph_take::Take,
    replacement: Option<&str>,
) -> AppearanceDocument {
    let Some(blob) = replacement.or(take.header.appearance.as_deref()) else {
        return AppearanceDocument::default();
    };
    AppearanceDocument::parse(blob).unwrap_or_else(|err| {
        eprintln!("warning: {err}; rendering at defaults (camera, view, spectrum and frame)");
        AppearanceDocument::default()
    })
}

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
}

impl Settings {
    pub fn frame_count(&self) -> u64 {
        ((self.end - self.start).max(0.0) * self.fps).round() as u64
    }
}

/// The input every offline frame is drawn with.
///
/// One constructor rather than a literal at each call site, because the field
/// that matters here is invisible by its ABSENCE: `max_texture_side` left unset
/// makes egui report its own 2048 default and size its font atlas against that
/// (issue #368, and see [`Renderer::max_texture_side`](crate::frames::Renderer::max_texture_side)).
/// A second `RawInput` built by hand is a second place for that field to go
/// missing, and nothing downstream reports the loss — the frames still render,
/// with labels the atlas had no room for.
fn frame_input(screen: egui::Rect, now: f64, max_texture_side: usize) -> egui::RawInput {
    egui::RawInput {
        screen_rect: Some(screen),
        time: Some(now),
        max_texture_side: Some(max_texture_side),
        ..Default::default()
    }
}

/// Render every frame, handing each to `emit` as tightly packed RGBA8, and
/// report where the time went ([`Stages`]).
///
/// `emit` takes the frame's buffer and returns one to draw the next frame
/// into — its own, once it has finished with it, or one that has been round a
/// writer thread. A 1440p frame is 14.7 MB, so a buffer per frame would be the
/// allocator doing 880 MB a second of video for nothing.
///
/// `emit` returning an error stops the render — that is how a dead encoder
/// gets reported rather than swallowed for another thousand frames. `emit`
/// returning `Ok(None)` also stops it, but cleanly: the encoder wants no more
/// frames (ffmpeg under `-shortest`, its soundtrack shorter than the visuals),
/// and the caller reads the true verdict from the encoder's exit status.
pub fn render(
    replay: &mut Replay,
    mut audio: Option<&mut Audio>,
    settings: &Settings,
    appearance: AppearanceDocument,
    mut emit: impl FnMut(Vec<u8>) -> Result<Option<Vec<u8>>, String>,
) -> Result<Stages, String> {
    let mut renderer = Renderer::new(settings.size)
        .ok_or("no usable GPU adapter (this needs a real GPU, not a container)")?;

    let context = egui::Context::default();
    // `frames::Renderer` installs the current font texture after applying
    // each frame's texture deltas and before paint callbacks prepare.
    harmonigraph_ui::use_renderer_font_texture(&context);
    harmonigraph_ui::theme::apply_theme(&context);
    context.set_pixels_per_point(settings.pixels_per_point);
    // What the GPU under this render will actually take. Read once: it is a
    // property of the device, and the device outlives every frame.
    let max_texture_side = renderer.max_texture_side();

    let mut state = PictureState::new(TextureFormat::Rgba8Unorm);
    state.install_appearance(appearance);
    // Nothing offline is interactive, and both would draw over the
    // picture: no armed-mode pulse, no hover highlight.
    state.runtime.learn_active = false;
    state.surfaces.hovered = None;
    // The comma auto-detects are interactive too, in the sense that matters
    // here: they answer a tuning EDIT, and a replay has no editor. Left on,
    // they judge the take's tuning afresh on frame 0 — and a session that
    // switched one off at a tuning that IS that temperament (12-TET, which is
    // both of them) would export with it on, respelling names and collapsing
    // comma-equivalent nodes the recorded session showed. The blob carries
    // what was on screen; the render's job is to reproduce it, not to
    // re-decide it.
    for comma in harmonigraph_core::Comma::ALL {
        *state.appearance.view.temper_auto_mut(comma) = false;
    }
    // "Spectrogram: Whole video" — the one setting only a render can answer,
    // since the render window is its length. Set once before the first frame,
    // so no cache keyed on the analyzer config sees it move.
    let spectrogram = state.appearance.render.spectrogram;
    if spectrogram == SpectrogramRender::WholeVideo {
        state.appearance.spectrum.span_history(settings.end - settings.start);
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
    // The same colour the lattice pane stands on. Offline clears to the
    // layout's background and paints the panes over it, rather than onto the
    // dock's panel — so without this the pane would paint a panel-coloured
    // rectangle several shades too light, and only in exported video, which is
    // the worst place to find out.
    state.set_background(settings.layout.background);

    let frames = settings.frame_count();
    // Timers only: nothing between here and the end of the loop reads a clock
    // to decide anything, so the picture is the same with them as without, and
    // the determinism tests still hold.
    let mut stages = Stages::default();
    let loop_began = Instant::now();
    // The frame buffer, going round: drawn into here, handed to the sink, and
    // back from `emit` for the next frame. Empty to start with — the first
    // render sizes it, and after the first few the sink is handing back
    // buffers that are already the right size.
    let mut buffer = Vec::new();
    for frame in 0..frames {
        let drawing = Instant::now();
        let now = prepare_frame(replay, &mut state, audio.as_deref_mut(), settings, frame)?;

        // No panels and no dock: the layout owns the frame, and the
        // background is the render pass's clear color rather than a
        // painted rect.
        let output = context.run_ui(frame_input(screen, now, max_texture_side), |ui| {
            for (surface, (pane, rect)) in placements.iter().enumerate() {
                let mut child = ui.new_child(egui::UiBuilder::new().max_rect(*rect));
                draw_pane(&mut child, *pane, &mut state, now, surface);
            }
            // Last, over the gaps the panes left: the seam that keeps the
            // lattice and the spectral pane from reading as one field.
            settings.layout.paint_dividers(ui.painter(), &placements);
        });

        let primitives = context.tessellate(output.shapes, settings.pixels_per_point);
        stages.ui += drawing.elapsed();

        let cost = renderer.render(
            &mut buffer,
            &primitives,
            &output.textures_delta,
            settings.pixels_per_point,
            background,
        );
        stages.submit += cost.submit;
        stages.readback += cost.readback;

        let handing_over = Instant::now();
        let returned = emit(std::mem::take(&mut buffer))?;
        stages.emit += handing_over.elapsed();
        stages.frames = frame + 1;
        stages.wall = loop_began.elapsed();
        let Some(next) = returned else {
            // The encoder wants no more frames (e.g. ffmpeg under -shortest,
            // the soundtrack ending before the visuals). Stop here; the caller
            // reads whether that was a clean finish from the exit status.
            //
            // This frame was drawn and handed over and the sink dropped it, so
            // it counts toward the stage costs but not toward the frames the
            // file holds — which is the encoder's own count, printed separately.
            return Ok(stages);
        };
        buffer = next;
    }
    Ok(stages)
}

/// Advance one export frame through the same replay/audio path the renderer
/// draws. Feed through the frame clock and preserve the analyzer's half-window
/// lag. Future columns would advance live retention past the picture's far edge
/// at low frame rates; the first frame therefore feeds an empty slice.
fn prepare_frame(
    replay: &mut Replay,
    state: &mut PictureState,
    audio: Option<&mut Audio>,
    settings: &Settings,
    frame: u64,
) -> Result<f64, String> {
    // Frame-index time avoids accumulated floating-point drift.
    let step = 1.0 / settings.fps;
    let now = settings.start + frame as f64 * step;
    replay.advance_to(&mut state.runtime, now);
    if let Some(audio) = audio {
        let from = settings.start + frame.saturating_sub(1) as f64 * step;
        let range = audio.range_seconds(from - settings.audio_start, now - settings.audio_start);
        if !range.is_empty() {
            let end = range.end;
            let newest = settings.audio_start + (end - 1) as f64 / f64::from(audio.sample_rate);
            let config = state.appearance.spectrum;
            state.runtime.spectrum.push_sample_chunks(
                range.len(),
                audio.channels,
                audio.sample_rate,
                newest,
                &config,
                |feed| audio.for_frames(range, feed),
            )?;
        }
    }
    begin_frame(state, &replay.params, now);
    Ok(now)
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
                    source: 0,
                    t,
                    channel: 0,
                    note,
                    kind: NoteKind::On { velocity: 0.8 },
                });
            }
            notes.push(NoteRecord {
                source: 0,
                t: t + 0.2,
                channel: 0,
                note: chord[0],
                kind: NoteKind::Tuning { semitones: 0.25 },
            });
            for &note in chord {
                notes.push(NoteRecord {
                    source: 0,
                    t: t + 0.6,
                    channel: 0,
                    note,
                    kind: NoteKind::Off,
                });
            }
        }
        Take {
            header: Header::default(),
            events: notes.into_iter().map(harmonigraph_take::CanonicalRecord::Note).collect(),
            params: Vec::new(),
            configurations: Vec::new(),
            truncated: false,
            incomplete: None,
        }
    }

    fn transient_audio() -> Audio {
        let mut samples = vec![0.0; 62_271 * 2];
        samples[38_400 * 2] = 1.0;
        samples[38_400 * 2 + 1] = -1.0;
        Audio::from_samples(48_000.0, samples, 2)
    }

    /// Two seconds of broadband noise: a pane with light in every row of it.
    ///
    /// [`transient_audio`] is one sample of ±1 in an otherwise empty buffer,
    /// which draws a pane that is 97% digital silence. That is the right
    /// fixture for the sample-grid tests that read it, and the wrong one for
    /// any test about what the cloud layer DRAWS — over silence there is
    /// nothing for a texture to bend.
    ///
    /// A plain LCG rather than `rand`, for the reason the golden set gives:
    /// a stream this crate does not own could re-baseline a picture on a
    /// version bump.
    fn lit_audio() -> Audio {
        let mut step = 0x5eed_4321u32;
        let samples = (0..96_000 * 2)
            .map(|_| {
                step = step.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
                ((step >> 8) as f32 / (1 << 23) as f32 - 1.0) * 0.25
            })
            .collect();
        Audio::from_samples(48_000.0, samples, 2)
    }

    fn transient_take(origin: f64) -> Take {
        let onset = origin + 0.8;
        Take {
            events: vec![
                harmonigraph_take::CanonicalRecord::Note(NoteRecord {
                    source: 0,
                    t: onset,
                    channel: 0,
                    note: 69,
                    kind: NoteKind::On { velocity: 0.8 },
                }),
                harmonigraph_take::CanonicalRecord::Note(NoteRecord {
                    source: 0,
                    t: onset + 0.05,
                    channel: 0,
                    note: 69,
                    kind: NoteKind::Off,
                }),
            ],
            ..Default::default()
        }
    }

    /// The timing line must not carry the substring the plugin's progress bar
    /// is parsed by, and its shares must be of the loop it can account for.
    ///
    /// `harmonigraph_record::parse_report` pulls the token before `" frames"`
    /// out of any segment of the renderer's stderr, which is how the Video
    /// pane follows a render. This line is printed AFTER `done: N frames`, so
    /// a `" frames"` in it would be the last word on the subject and would
    /// leave the bar retargeted at whatever number happened to precede it.
    #[test]
    fn the_timing_summary_avoids_the_progress_bars_substring() {
        // A plausible export rather than a blank one: every field is nonzero,
        // so every number in the line is actually formatted, and the total is
        // longer than the loop the way a real one is.
        let stages = Stages {
            frames: 5320,
            wall: Duration::from_secs_f64(78.125),
            ui: Duration::from_secs_f64(33.0),
            submit: Duration::from_secs_f64(9.5),
            readback: Duration::from_secs_f64(31.0),
            emit: Duration::from_secs_f64(4.4),
        };
        let line = stages.summary(Duration::from_secs_f64(92.4));
        assert!(!line.contains(" frames"), "{line}");
        assert!(line.contains("5320-frame"), "{line}");
        for stage in ["ui+tess", "submit", "readback", "emit"] {
            assert!(line.contains(stage), "{stage} is missing from {line}");
        }
        // Shares are of the LOOP, so the four account for it and the gap up to
        // the total is the setup and the encoder's backlog, not an unnamed
        // stage: 33.0 s of a 78.125 s loop is 42%, not the 36% it is of 92.4.
        assert!(line.contains("(42%)"), "{line}");
        assert!(line.contains("68.1 fps"), "5320 frames in 78.125 s: {line}");

        // A render that drew nothing says so rather than dividing by no frames.
        let nothing = Stages::default().summary(Duration::from_secs_f64(0.2));
        assert!(nothing.contains("nothing drawn"), "{nothing}");
    }

    #[test]
    fn scrolling_audio_uses_the_supplied_sample_grid() {
        const SR: f64 = 48_000.0;
        const FRAMES: usize = 62_271;
        const HOP: usize = 384;
        let mut audio = transient_audio();
        for (origin, offset, first) in
            [(0.0, 0.0, 0usize), (7.125, 0.0, 0), (7.125, 0.41731, 20_031), (7.125, -0.00713, 0)]
        {
            let mut previous_bins: Option<Vec<Vec<u8>>> = None;
            for fps in [1.0, 4.0, 30.0, 60.0, 120.0, 30_000.0 / 1001.0] {
                let settings = Settings {
                    fps,
                    start: origin + offset,
                    end: origin + 3.5,
                    audio_start: origin,
                    ..settings()
                };
                let mut replay = Replay::new(transient_take(origin));
                let mut state = PictureState::new(TextureFormat::Rgba8Unorm);
                state.runtime.learn_active = false;
                let window = state.appearance.spectrum.window.samples();
                let lag = window as f64 / (2.0 * SR);
                let audio_end = origin + FRAMES as f64 / SR;
                let mut partial_tail = false;
                for frame in 0..settings.frame_count() {
                    let before = state.runtime.spectrum.history().len();
                    let now =
                        prepare_frame(&mut replay, &mut state, Some(&mut audio), &settings, frame)
                            .unwrap();
                    let from = settings.start + frame.saturating_sub(1) as f64 / fps;
                    if from < audio_end && now > audio_end {
                        partial_tail = true;
                        assert!(
                            state.runtime.spectrum.history().len() > before,
                            "partial tail must emit a column: {fps}, {offset}"
                        );
                    }
                    if from >= audio_end {
                        assert_eq!(
                            state.runtime.spectrum.history().len(),
                            before,
                            "empty tail added data"
                        );
                    }
                    if frame == 0 {
                        assert_eq!(
                            state.runtime.spectrum.history().len(),
                            0,
                            "no pre-roll on the first frame"
                        );
                    }
                    assert!(
                        state.runtime.spectrum.history().iter().all(|c| c.time <= now),
                        "future columns would evict visible history at {fps} fps"
                    );
                }
                assert!(partial_tail);
                // Enumerate source sample indices, independent of slicing and
                // the analyzer's anchor. Late starts retain their hop phase.
                let first_ready = window.div_ceil(HOP) * HOP;
                let expected: Vec<_> = (first_ready..=FRAMES - first)
                    .step_by(HOP)
                    .map(|fed| origin + (first + fed - 1) as f64 / SR - lag)
                    .collect();
                let history = state.runtime.spectrum.history();
                assert_eq!(history.len(), expected.len());
                assert!(!expected.is_empty());
                for (column, expected) in history.iter().zip(expected) {
                    assert!(
                        (column.time - expected).abs() < 1e-9,
                        "{fps} fps, start {offset}: {} instead of {expected}",
                        column.time
                    );
                }
                assert!((state.runtime.spectrum.column_lag() - lag).abs() < 1e-12);
                let bins: Vec<_> = history.iter().map(|c| c.db().to_vec()).collect();
                if let Some(previous) = &previous_bins {
                    assert_eq!(&bins, previous, "batching changed spectrum bytes at {fps} fps");
                }
                previous_bins = Some(bins);
                let note = state
                    .roll()
                    .notes()
                    .find(|n| n.source == harmonigraph_core::SourceId(0) && n.note == 69)
                    .expect("the MIDI event reached the replayed roll");
                assert_eq!(note.start, origin + 0.8);
                let energy = |c: &harmonigraph_ui::SpectrogramColumn| {
                    c.db().iter().map(|&v| u64::from(v)).sum::<u64>()
                };
                let peak = history.iter().map(energy).max().unwrap();
                assert!(peak > 0, "anti-phase transient reached the analyzer");
                // Quantized dB can make a short plateau around a symmetric
                // impulse. Its midpoint dates the peak without favoring a side.
                let peaks: Vec<_> =
                    history.iter().filter(|c| energy(c) == peak).map(|c| c.time).collect();
                let peak_time = (peaks[0] + peaks[peaks.len() - 1]) * 0.5;
                assert!(
                    (peak_time - note.start).abs() <= (HOP + 1) as f64 / SR,
                    "transient {peak_time} versus replayed MIDI {} at {fps} fps",
                    note.start
                );
            }
        }
    }

    #[test]
    fn rendering_sliced_audio_twice_is_byte_identical() {
        let mut audio = transient_audio();
        let mut take = transient_take(7.125);
        let mut state = PictureState::new(TextureFormat::Rgba8Unorm);
        state.appearance.spectrum.roll_seconds = 1.0;
        state.appearance.spectrum.roll_fraction = 1.0;
        state.appearance.spectrum.show_roll = false;
        take.header.appearance = Some(state.appearance.serialize());
        let settings = Settings {
            layout: Layout::preset("spectral").unwrap(),
            fps: 30_000.0 / 1001.0,
            start: 7.125 + 0.41731,
            end: 7.125 + 1.4,
            audio_start: 7.125,
            ..settings()
        };
        let run = |audio: Option<&mut Audio>| {
            let mut frames = Vec::new();
            let result = render(
                &mut Replay::new(take.clone()),
                audio,
                &settings,
                appearance_for(&take, None),
                |bytes| {
                    frames.push(bytes);
                    Ok(Some(Vec::new()))
                },
            );
            match result {
                Ok(_) => Some(frames),
                Err(e) if e.contains("no usable GPU adapter") => {
                    eprintln!("skipping: {e}");
                    None
                }
                Err(e) => panic!("{e}"),
            }
        };
        let Some(first) = run(Some(&mut audio)) else { return };
        assert_eq!(first, run(Some(&mut audio)).unwrap());
        assert_ne!(first, run(None).unwrap(), "the audio must change the rendered picture");
    }

    /// "Spectrogram: Whole video" draws exactly what the History duration dialled to
    /// the render's own length draws — and Scrolling at the default span draws
    /// something else, or the equality would hold for a render that ignored the
    /// setting.
    #[test]
    fn a_whole_video_spectrogram_draws_the_render_window_as_its_span() {
        let mut audio = transient_audio();
        let settings = Settings {
            layout: Layout::preset("spectral").unwrap(),
            start: 7.125,
            end: 7.125 + 1.4,
            audio_start: 7.125,
            ..settings()
        };
        let mut run = |configure: fn(&mut AppearanceDocument, f32)| {
            let mut appearance = AppearanceDocument::default();
            configure(&mut appearance, (settings.end - settings.start) as f32);
            let mut frames = Vec::new();
            let result = render(
                &mut Replay::new(transient_take(7.125)),
                Some(&mut audio),
                &settings,
                appearance,
                |bytes| {
                    frames.push(bytes);
                    Ok(Some(Vec::new()))
                },
            );
            match result {
                Ok(_) => Some(frames),
                Err(e) if e.contains("no usable GPU adapter") => {
                    eprintln!("skipping: {e}");
                    None
                }
                Err(e) => panic!("{e}"),
            }
        };
        let Some(spanned) = run(|a, _| a.render.spectrogram = SpectrogramRender::WholeVideo) else {
            return;
        };
        // Scrolling named on both of these, Whole video being the default: left
        // to it, the dialled run would be Whole video too and equal by itself.
        let dialled = run(|a, window| {
            a.render.spectrogram = SpectrogramRender::Scrolling;
            a.spectrum.roll_seconds = window;
        })
        .unwrap();
        assert_eq!(spanned, dialled, "Whole video must span exactly the render's window");
        let scrolling = run(|a, _| a.render.spectrogram = SpectrogramRender::Scrolling).unwrap();
        assert_ne!(spanned, scrolling, "the default span drew the same picture");
    }

    #[test]
    fn export_selects_one_complete_appearance_and_defaults_a_refused_replacement() {
        let mut recorded = AppearanceDocument::default();
        recorded.camera.yaw = 1.23;
        recorded.view.max_sevens = 3;
        recorded.spectrum.low_midi = 40.5;
        recorded.spiral.zoom = 2.75;
        recorded.render.short_edge = 2160;
        let mut take = take();
        take.header.appearance = Some(recorded.serialize());
        let selected = appearance_for(&take, None);
        assert_eq!(selected.serialize(), recorded.serialize());
        let mut replacement = AppearanceDocument::default();
        replacement.camera.yaw = -0.5;
        replacement.view.max_sevens = 2;
        replacement.spectrum.low_midi = 45.0;
        replacement.spiral.zoom = 1.5;
        replacement.render.short_edge = 720;
        let selected = appearance_for(&take, Some(&replacement.serialize()));
        assert_eq!(selected.serialize(), replacement.serialize());
        let expected_size = replacement.render.frame.pixels(720);
        assert_eq!(crate::output_size(None, &selected.render), expected_size);
        assert_eq!(crate::output_size(Some([640, 480]), &selected.render), [640, 480]);
        let mut state = PictureState::new(TextureFormat::Rgba8Unorm);
        state.install_appearance(selected);
        assert_eq!(state.appearance.serialize(), replacement.serialize());
        for refused in
            ["broken".to_string(), replacement.serialize().replacen("version:1", "version:0", 1)]
        {
            assert_eq!(
                appearance_for(&take, Some(&refused)).serialize(),
                AppearanceDocument::default().serialize()
            );
        }
        take.header.appearance = None;
        assert_eq!(
            appearance_for(&take, None).serialize(),
            AppearanceDocument::default().serialize()
        );
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
        }
    }

    /// The same take with the node glow dialled on, through the one channel a
    /// take has for a look: the blob the header carries.
    ///
    /// Its own fixture rather than a setting on the one above, because the
    /// light is the only thing in the draw path that is CARRIED between frames
    /// — a node's row of the ink strip holds the colour it had last frame — and
    /// what the tests above measure is the frame count and the picture, neither
    /// of which wants a halo over it.
    fn lit_take() -> Take {
        let mut state = PictureState::new(TextureFormat::Rgba8Unorm);
        state.appearance.view.glow_reach = 0.8;
        state.appearance.view.glow_strength = 1.5;
        // Long against the tenth of a second a frame is here, so several frames
        // of every note's light are a mix of the frames before it rather than a
        // settle.
        state.appearance.view.glow_attack = 0.3;
        state.appearance.view.glow_release = 2.5;
        let mut take = take();
        take.header.appearance = Some(state.appearance.serialize());
        take
    }

    fn spectral_shadow_take(enabled: bool) -> Take {
        let mut state = PictureState::new(TextureFormat::Rgba8Unorm);
        state.appearance.spectrum.show_roll = true;
        state.appearance.spectrum.roll_fraction = 0.7;
        state.appearance.view.shadow.spectral_geometry = harmonigraph_scene::ShadowStyle {
            kernel: harmonigraph_scene::ShadowKernel::Gaussian,
            width: 0.75,
            depth: f32::from(enabled),
            ..Default::default()
        };
        state.appearance.view.shadow.spectral_text = harmonigraph_scene::ShadowStyle {
            kernel: harmonigraph_scene::ShadowKernel::Distance,
            width: 0.75,
            depth: f32::from(enabled),
            ..Default::default()
        };
        let mut take = take();
        take.header.appearance = Some(state.appearance.serialize());
        take
    }

    fn render_frames(settings: &Settings) -> Option<Vec<Vec<u8>>> {
        render_take(take(), settings)
    }

    fn render_take(take: Take, settings: &Settings) -> Option<Vec<Vec<u8>>> {
        let mut replay = Replay::new(take);
        let mut frames = Vec::new();
        let appearance = appearance_for(replay.take(), None);
        match render(&mut replay, None, settings, appearance, |bytes| {
            frames.push(bytes);
            Ok(Some(Vec::new()))
        }) {
            Ok(_) => Some(frames),
            // Optional local GPU tests may skip. The shared device setup
            // fails before returning this error when CI requires a GPU.
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

    /// The same take renders to the same bytes with a CARRIED light in it.
    ///
    /// The determinism above is a claim about a draw path with no state in it,
    /// and the node glow is the one thing that has some: a node's light is
    /// filtered on the CPU against the frame clock, and its colour is mixed
    /// into a texture the last frame left behind. Both are deterministic — the
    /// clock comes off the frame index and the strip is built afresh with the
    /// renderer — but neither is deterministic by CONSTRUCTION the way a pure
    /// function of `now` is, so it is measured rather than reasoned about.
    #[test]
    fn rendering_a_take_with_a_carried_light_twice_is_byte_identical() {
        let settings = settings();
        let Some(first) = render_take(lit_take(), &settings) else { return };
        let second = render_take(lit_take(), &settings).expect("second run also has a GPU");
        assert_eq!(first.len(), second.len());
        for (i, (a, b)) in first.iter().zip(&second).enumerate() {
            assert!(a == b, "frame {i} differs between two runs of the same lit take");
        }
        // Non-vacuous: the light has to be in the picture, or this is the test
        // above with more steps. Against the same take drawn without it, which
        // differs in nothing else.
        let dark = render_frames(&settings).expect("a third run also has a GPU");
        let mid = first.len() / 2;
        assert!(first[mid] != dark[mid], "the glow changed no pixel of frame {mid}");
    }

    /// A DRIFTING watercolour wash renders to the same bytes twice.
    ///
    /// The spectrogram's cloud layer is the one place in the draw path with a
    /// clock of its own, and all that keeps it reproducible is that the drift
    /// comes off `atmosphere.now * cloud_speed` rather than a wall clock. The
    /// shader text reads the same either way, so this is measured.
    ///
    /// Both halves are non-vacuous, and either could be silently absent. The
    /// wash has to be REACHED — the texture sits behind an enum whose fresh
    /// value is the other one, so the frame is held against the same take drawn
    /// with the scales — and the clock has to MOVE something, or this is the
    /// test above with one more uniform in it. The second is held against the
    /// same run with `Cloud speed` alone at 0 and NOT against another frame of
    /// the run itself: over a fixture with light in it the spectrogram scrolls,
    /// so any two frames differ whatever the cloud clock does, and a
    /// frame-to-frame comparison would pass with the whole layer frozen.
    ///
    /// Both of those rest on the pane HAVING LIGHT IN IT, which is the third
    /// assert and the one this test shipped without. Its first fixture was one
    /// sample of ±1 read a second past the end of its own buffer, a pane 97%
    /// digital silence; the two textures drew it apart only because the wash
    /// lifted black to mid-tone, so the moment that lift got a floor under it
    /// they agreed pixel for pixel. A guard that a change of LOOK can turn
    /// vacuous is worth no more than the fixture behind it.
    #[test]
    fn rendering_a_drifting_wash_twice_is_byte_identical() {
        let clouded = |wash: bool, speed: f32| {
            let mut state = PictureState::new(TextureFormat::Rgba8Unorm);
            let a = &mut state.appearance.spectrum.atmosphere;
            if wash {
                a.cloud_style = harmonigraph_scene::CloudStyle::Watercolor;
            }
            // Fast enough that a second of render carries the field a visible
            // way: at the fresh 1x the whole run is a fraction of one glob.
            a.cloud_speed = speed;
            let mut take = transient_take(0.0);
            take.header.appearance = Some(state.appearance.serialize());
            take
        };
        let settings = Settings {
            // The full-pane preset, because the cloud layer is drawn over the
            // spectrogram and nothing else — a layout that gives it a third of
            // a 320-point frame measures the other two thirds.
            layout: Layout::preset("spectral").unwrap(),
            ..settings()
        };
        let run = |take: &Take| {
            let mut audio = lit_audio();
            let mut frames = Vec::new();
            let result = render(
                &mut Replay::new(take.clone()),
                Some(&mut audio),
                &settings,
                appearance_for(take, None),
                |bytes| {
                    frames.push(bytes);
                    Ok(Some(Vec::new()))
                },
            );
            match result {
                Ok(_) => Some(frames),
                Err(e) if e.contains("no usable GPU adapter") => {
                    eprintln!("skipping: {e}");
                    None
                }
                Err(e) => panic!("{e}"),
            }
        };
        let Some(first) = run(&clouded(true, 8.0)) else { return };
        assert_eq!(first, run(&clouded(true, 8.0)).expect("a second GPU run"));
        let scales = run(&clouded(false, 8.0)).expect("a third GPU run");
        let still = run(&clouded(true, 0.0)).expect("a fourth GPU run");
        let mid = first.len() / 2;
        // The pane has light in it, which is the assumption under BOTH asserts
        // below and the one that quietly stopped holding: a silent pane sits on
        // the palette's floor for either texture, so a dark fixture makes them
        // agree and makes a stirred frame identical to a still one. The old
        // fixture was one sample of ±1 read a second past the end of its own
        // buffer — 97% digital silence, which both asserts passed over only
        // because the wash then painted it with a lift nothing held back.
        let lit = first[mid].chunks_exact(4).filter(|px| px[..3] != [0, 0, 0]).count();
        let pane = first[mid].len() / 4;
        assert!(
            lit * 2 > pane,
            "the fixture draws a mostly black pane, so neither assert below is about the \
             wash: {lit} of {pane} pixels lit",
        );
        assert!(first[mid] != scales[mid], "the wash drew the scales' frame {mid}");
        assert!(first[mid] != still[mid], "the cloud clock moved nothing in frame {mid}");
    }

    #[test]
    fn lattice_atmosphere_requires_note_light_in_export() {
        let settings = Settings {
            layout: Layout::preset("lattice").unwrap(),
            fps: 2.0,
            end: 3.0,
            ..settings()
        };
        let mut silent = take();
        silent.events.clear();
        let Some(first) = render_take(silent.clone(), &settings) else { return };
        assert_eq!(first.len(), 6);
        assert_eq!(first[1], first[5], "no independent background motion in silence");
        let mut appearance = AppearanceDocument::default();
        appearance.view.atmosphere.enabled = false;
        silent.header.appearance = Some(appearance.serialize());
        let off = render_take(silent, &settings).expect("the same GPU is available");
        assert_eq!(first, off, "atmosphere must not create light without notes");

        let lit = lit_take();
        let textured = render_take(lit.clone(), &settings).expect("the same GPU is available");
        let mut smooth = lit;
        let mut appearance = AppearanceDocument::parse(smooth.header.appearance.as_ref().unwrap())
            .expect("the recorded appearance is valid");
        appearance.view.atmosphere.nebula_depth = 0.0;
        smooth.header.appearance = Some(appearance.serialize());
        let smooth = render_take(smooth, &settings).expect("the same GPU is available");
        assert_ne!(textured[2], smooth[2], "the recorded nebula setting must reach export");
    }

    /// Offline export calls the same pane and paint callbacks as the editor;
    /// exercise that shared route at every UI/export scale promised by #556.
    /// The mixed frame is compared with its two depths shut so a passing render
    /// must actually reach both spectral shadow groups, not merely produce a
    /// plausible roll and axis at each scale.
    #[test]
    fn spectral_shadows_agree_with_the_shared_editor_path_at_every_export_scale() {
        for ppp in [1.0f32, 1.5, 2.0, 4.0] {
            let settings = Settings {
                layout: Layout::preset("spectral").expect("the spectral preset exists"),
                size: [(320.0 * ppp).round() as u32, (200.0 * ppp).round() as u32],
                pixels_per_point: ppp,
                fps: 2.0,
                start: 0.0,
                end: 0.5,
                audio_start: 0.0,
            };
            let Some(shadowed) = render_take(spectral_shadow_take(true), &settings) else {
                return;
            };
            let bare = render_take(spectral_shadow_take(false), &settings)
                .expect("the second run sees the same GPU");
            assert_eq!(shadowed.len(), 1, "the fixture is one frame at {ppp} ppp");
            assert_ne!(
                shadowed[0], bare[0],
                "the mixed spectral shadow changed no export pixel at {ppp} ppp",
            );
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
