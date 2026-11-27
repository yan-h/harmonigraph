//! Rendering the UI with no window: a headless wgpu device, an egui
//! context driven by synthesized input, and an offscreen texture read
//! back to bytes.
//!
//! There is nothing exotic here, which is the point — the plugin's own
//! surface is the only thing this route does *not* need, and that surface
//! is exactly what blocks live capture (its usage flags don't allow a
//! copy, and neither its device nor its queue is reachable from the
//! editor). Everything else — the fonts, the panes, the lattice's paint
//! callback and all its GPU resources — works the same off a plain
//! `egui_wgpu::Renderer` as it does in the DAW.

use harmonigraph_render::wgpu;

/// The output pixel format. Non-sRGB deliberately: `egui-wgpu` picks its
/// gamma-correcting fragment shader based on `is_srgb()`, so an sRGB
/// target would shift every color away from what the plugin shows. RGBA
/// rather than the plugin's BGRA only so the bytes go straight to ffmpeg
/// without a swizzle; the two render identically.
pub const FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Rgba8Unorm;

/// Full-frame composition for pixel fixtures and scratch look prototypes.
/// This is test-only; the public export interface composes both panes.
#[cfg(test)]
pub(crate) fn single_pane(pane: harmonigraph_ui::Pane) -> harmonigraph_ui::Layout {
    use harmonigraph_ui::{Layout, Placement};
    Layout { panes: vec![Placement { pane, rect: (0.0, 0.0, 1.0, 1.0) }] }
}

/// How many frames can be with the GPU at once: one drawing while the one
/// before it is copied out and handed to the sink.
///
/// Two is the whole of the overlap there is to have. The CPU's share of a
/// frame — building and submitting the next one, then emitting this one — runs
/// while the GPU draws, and a third buffer would only let the CPU run further
/// ahead of a GPU it is already waiting on.
pub const IN_FLIGHT: usize = 2;

/// Everything needed to render frames, set up once.
pub struct Renderer {
    device: wgpu::Device,
    queue: wgpu::Queue,
    egui: egui_wgpu::Renderer,
    /// One target for every frame, though two can be in flight: the queue
    /// runs in submission order, so a frame's copy out of it is done before
    /// the next frame's pass clears it.
    target: wgpu::Texture,
    view: wgpu::TextureView,
    /// One per frame [`IN_FLIGHT`], taken in turn.
    readback: [wgpu::Buffer; IN_FLIGHT],
    /// Submitted and not yet read, oldest first.
    in_flight: std::collections::VecDeque<InFlight>,
    /// The `readback` slot the next frame copies into.
    next: usize,
    size: [u32; 2],
    /// Padded row stride of `readback`; wgpu requires 256-byte alignment,
    /// which an arbitrary width does not give us.
    bytes_per_row: u32,
}

/// A frame the GPU has been handed and the export has not read back yet.
struct InFlight {
    slot: usize,
    submission: wgpu::SubmissionIndex,
    /// Where the map callback reports. A channel rather than a panic in the
    /// callback, because an export that stops early (a dead encoder) drops the
    /// renderer with this frame's map still pending.
    mapped: std::sync::mpsc::Receiver<Result<(), wgpu::BufferAsyncError>>,
}

/// Round `bytes` up to wgpu's copy alignment.
fn aligned(bytes: u32) -> u32 {
    let align = wgpu::COPY_BYTES_PER_ROW_ALIGNMENT;
    bytes.div_ceil(align) * align
}

impl Renderer {
    /// `size` is in physical pixels. Returns `None` if the machine has no
    /// usable GPU adapter — callers decide whether that is fatal. Tests use
    /// the shared GPU requirement, which makes unavailable GPUs fatal in CI.
    pub fn new(size: [u32; 2]) -> Option<Renderer> {
        #[cfg(test)]
        let (device, queue) = harmonigraph_render::test_gpu_device()?;
        #[cfg(not(test))]
        let (device, queue) = {
            let instance = wgpu::Instance::default();
            let adapter = pollster::block_on(
                instance.request_adapter(&wgpu::RequestAdapterOptions::default()),
            )
            .ok()?;
            pollster::block_on(
                adapter.request_device(&harmonigraph_render::device_descriptor(&adapter)),
            )
            .ok()?
        };

        // `predictable_texture_filtering` makes glyph sampling identical
        // across GPUs. It costs a little sharpness, but a render that
        // differs by machine would make the determinism test a lie and
        // make a re-render of one shot not match the others.
        let egui = egui_wgpu::Renderer::new(
            &device,
            FORMAT,
            egui_wgpu::RendererOptions {
                predictable_texture_filtering: true,
                ..Default::default()
            },
        );

        let (target, view, readback, bytes_per_row) = targets(&device, size);

        Some(Renderer {
            device,
            queue,
            egui,
            target,
            view,
            readback,
            in_flight: Default::default(),
            next: 0,
            size,
            bytes_per_row,
        })
    }

    /// Draw every later frame at `size`, as a window resized between frames
    /// would. The frames in flight are read first, being copies out of the
    /// target this replaces.
    #[cfg(all(test, target_vendor = "apple"))]
    pub fn resize(&mut self, size: [u32; 2]) {
        while self.read(&mut Vec::new()).is_some() {}
        (self.target, self.view, self.readback, self.bytes_per_row) = targets(&self.device, size);
        self.size = size;
    }

    /// Frames submitted and not yet [read](Self::read); at most [`IN_FLIGHT`].
    pub fn in_flight(&self) -> usize {
        self.in_flight.len()
    }

    /// The widest texture this device will take, on either side.
    ///
    /// The context has to be TOLD this — `RawInput::max_texture_side` left
    /// `None` makes egui report its own 2048 default, and egui grows its font
    /// atlas only up to whatever it is told: a render whose labels need more
    /// glyph area than that simply loses them, with nothing on screen or in
    /// stderr saying so (issue #368). The interactive editor accepts a 4096
    /// bound to cap its long-lived font atlas; an offline frame can legitimately
    /// exceed 4K and needs the device's full answer here.
    ///
    /// The DEVICE's number rather than a lower one chosen here. A second,
    /// quieter cap in this crate would make an offline frame differ from the
    /// editor's for a reason no pane could report.
    pub fn max_texture_side(&self) -> usize {
        self.device.limits().max_texture_dimension_2d as usize
    }

    /// Hand one frame's tessellated shapes to the GPU, to be collected by a
    /// later [`Self::read`], and report what that cost the CPU.
    ///
    /// The cost is applying texture deltas, running the paint callbacks'
    /// `prepare`, encoding the pass and the copy, and `queue.submit` — the
    /// only part of a frame's GPU side a cheaper scene would shorten. Waiting
    /// for the GPU is [`Self::read`]'s, which is why the two are apart.
    ///
    /// Panics if [`IN_FLIGHT`] frames are already waiting to be read.
    pub fn submit(
        &mut self,
        primitives: &[egui::ClippedPrimitive],
        mut textures: egui::TexturesDelta,
        pixels_per_point: f32,
        clear: egui::Color32,
    ) -> std::time::Duration {
        assert!(self.in_flight.len() < IN_FLIGHT, "read a frame back before submitting another");
        let handed_over = std::time::Instant::now();
        // Taken and drained, since egui asserts a dropped delta was applied:
        // textures in any order, each one's deltas in theirs.
        for (id, deltas) in textures.set.drain() {
            for delta in &deltas {
                self.egui.update_texture(&self.device, &self.queue, id, delta);
            }
        }
        // Paint callbacks need the texture after this frame's deltas, not the
        // image cloned out of the font store. Texture::clone shares the GPU
        // allocation; it does not copy atlas pixels.
        let font_texture =
            self.egui.texture(&egui::TextureId::default()).and_then(|entry| entry.texture.clone());
        if let Some(texture) = font_texture {
            self.egui.callback_resources.insert(texture);
        }

        let descriptor =
            egui_wgpu::ScreenDescriptor { size_in_pixels: self.size, pixels_per_point };
        let mut encoder = self.device.create_command_encoder(&Default::default());
        // This is also where the lattice's paint callback runs its scene
        // and bloom passes — `update_buffers` dispatches `prepare` on our
        // encoder, and the callback creates its own GPU resources on
        // first use, so nothing here has to know the lattice exists.
        // Any command buffers a callback produced on its own encoder must
        // be submitted BEFORE the pass that reads their results.
        let callback_commands = self.egui.update_buffers(
            &self.device,
            &self.queue,
            &mut encoder,
            primitives,
            &descriptor,
        );

        {
            let mut pass = encoder
                .begin_render_pass(&wgpu::RenderPassDescriptor {
                    label: Some("offline frame"),
                    color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                        view: &self.view,
                        depth_slice: None,
                        resolve_target: None,
                        ops: wgpu::Operations {
                            load: wgpu::LoadOp::Clear(clear_value(clear)),
                            store: wgpu::StoreOp::Store,
                        },
                    })],
                    depth_stencil_attachment: None,
                    timestamp_writes: None,
                    occlusion_query_set: None,
                    multiview_mask: None,
                })
                .forget_lifetime();
            self.egui.render(&mut pass, primitives, &descriptor);
        }

        let slot = self.next;
        let readback = &self.readback[slot];
        encoder.copy_texture_to_buffer(
            self.target.as_image_copy(),
            wgpu::TexelCopyBufferInfo {
                buffer: readback,
                layout: wgpu::TexelCopyBufferLayout {
                    offset: 0,
                    bytes_per_row: Some(self.bytes_per_row),
                    rows_per_image: None,
                },
            },
            wgpu::Extent3d { width: self.size[0], height: self.size[1], depth_or_array_layers: 1 },
        );
        let submission = self.queue.submit(callback_commands.into_iter().chain([encoder.finish()]));
        // Asked for now, after the submit that writes the buffer, so the map
        // lands as soon as this frame's work does.
        let (report, mapped) = std::sync::mpsc::channel();
        readback.slice(..).map_async(wgpu::MapMode::Read, move |result| {
            let _ = report.send(result);
        });
        self.in_flight.push_back(InFlight { slot, submission, mapped });
        self.next = (slot + 1) % IN_FLIGHT;
        let submit = handed_over.elapsed();

        // The GPU keeps a freed texture alive until the work using it is done.
        for id in textures.free.drain() {
            self.egui.free_texture(&id);
        }
        submit
    }

    /// Wait for the oldest frame still in flight and copy it into `frame` as
    /// tightly packed RGBA8 (row padding removed). `None`, touching nothing,
    /// when no frame is in flight.
    ///
    /// The time returned is the render thread blocked on the GPU plus the
    /// unpadding copy out of the mapped rows — the whole of it the export doing
    /// nothing else, which is what makes it worth its own number. With a newer
    /// frame submitted first, the wait is only whatever the GPU still had left
    /// once the CPU had built that frame.
    ///
    /// `frame` is filled rather than returned, and the caller is expected to
    /// hand back a buffer that has been round the sink: at 1440p a frame is
    /// 14.7 MB, so allocating one per frame is 880 MB a second of video through
    /// the allocator for no reason. Whatever is in it is overwritten; only its
    /// capacity is reused.
    pub fn read(&mut self, frame: &mut Vec<u8>) -> Option<std::time::Duration> {
        let InFlight { slot, submission, mapped } = self.in_flight.pop_front()?;
        let waited = std::time::Instant::now();
        // This frame's submission rather than the latest, which would wait out
        // the newer frame too and give back the overlap.
        self.device
            .poll(wgpu::PollType::Wait { submission_index: Some(submission), timeout: None })
            .expect("poll");
        // The poll that finished the submission has run its map callback.
        mapped
            .try_recv()
            .expect("the readback mapped once its submission finished")
            .expect("map readback buffer");
        let readback = &self.readback[slot];
        let row_bytes = (self.size[0] * 4) as usize;
        frame.clear();
        frame.reserve(row_bytes * self.size[1] as usize);
        {
            let mapped =
                readback.slice(..).get_mapped_range().expect("mapped range of the readback");
            for row in 0..self.size[1] as usize {
                let start = row * self.bytes_per_row as usize;
                frame.extend_from_slice(&mapped[start..start + row_bytes]);
            }
        }
        readback.unmap();
        Some(waited.elapsed())
    }

    /// One frame submitted and read straight back into a fresh buffer, for the
    /// probes below — they are about what a frame LOOKS like, and neither the
    /// overlap, the recycling nor the cost would say anything to them.
    #[cfg(test)]
    pub fn render_to_vec(
        &mut self,
        primitives: &[egui::ClippedPrimitive],
        textures: egui::TexturesDelta,
        pixels_per_point: f32,
        clear: egui::Color32,
    ) -> Vec<u8> {
        self.submit(primitives, textures, pixels_per_point, clear);
        let mut frame = Vec::new();
        self.read(&mut frame).expect("the frame just submitted");
        frame
    }
}

/// The frame target at `size`, its view, a readback buffer per frame in
/// flight, and their padded row stride.
fn targets(
    device: &wgpu::Device,
    size: [u32; 2],
) -> (wgpu::Texture, wgpu::TextureView, [wgpu::Buffer; IN_FLIGHT], u32) {
    let target = device.create_texture(&wgpu::TextureDescriptor {
        label: Some("offline frame"),
        size: wgpu::Extent3d { width: size[0], height: size[1], depth_or_array_layers: 1 },
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format: FORMAT,
        usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::COPY_SRC,
        view_formats: &[],
    });
    let view = target.create_view(&Default::default());
    let bytes_per_row = aligned(size[0] * 4);
    let readback = std::array::from_fn(|_| {
        device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("offline readback"),
            size: u64::from(bytes_per_row) * u64::from(size[1]),
            usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
            mapped_at_creation: false,
        })
    });
    (target, view, readback, bytes_per_row)
}

/// The clear color, as wgpu wants it for [`FORMAT`].
///
/// Straight `byte / 255`, and that is the whole subtlety: because the
/// target is UNORM rather than sRGB, wgpu stores a clear value verbatim,
/// and egui — which picks its gamma-correcting fragment shader for
/// exactly the same reason — writes painted shapes as their gamma-space
/// bytes too. So both paths agree here. Converting to linear "because the
/// color is sRGB" would make the background come out several times too
/// dark, and only where the clear shows through.
fn clear_value(color: egui::Color32) -> wgpu::Color {
    let channel = |byte: u8| f64::from(byte) / 255.0;
    wgpu::Color { r: channel(color.r()), g: channel(color.g()), b: channel(color.b()), a: 1.0 }
}

#[cfg(test)]
mod tests {
    use super::*;
    use harmonigraph_ui::PictureState;

    #[test]
    fn row_stride_is_padded_to_wgpus_alignment() {
        // 1920 and 3840 are already aligned; 1000 is not, and is exactly
        // the sort of width a hand-set --size would produce.
        assert_eq!(aligned(1920 * 4), 1920 * 4);
        assert_eq!(aligned(3840 * 4), 3840 * 4);
        assert_eq!(aligned(1000 * 4), 4096);
        assert_eq!(aligned(1) % wgpu::COPY_BYTES_PER_ROW_ALIGNMENT, 0);
    }

    /// Pins the no-conversion decision: a UNORM target stores the clear
    /// verbatim, so the byte that goes in is the byte that comes out —
    /// matching what egui paints for the same color.
    #[test]
    fn the_clear_color_is_the_gamma_space_byte_not_a_linear_one() {
        let value = clear_value(egui::Color32::from_rgb(128, 0, 255));
        assert!((value.r - 128.0 / 255.0).abs() < 1e-9);
        assert_eq!(value.g, 0.0);
        assert_eq!(value.b, 1.0);
        assert_eq!(value.a, 1.0);
    }

    /// The standing setup the frame tests below share: a headless renderer, a
    /// themed context at one scale, and the single-pane lattice layout.
    ///
    /// It exists for the `RawInput` rather than for the line count. Each test
    /// used to build one by hand, and the field that decides whether a frame is
    /// a picture of what the EXPORT draws is invisible by its absence —
    /// `max_texture_side`, issue #368. [`Self::frame`] therefore goes through
    /// [`crate::render::frame_input`], the render loop's own constructor, so a
    /// field added there arrives here rather than being forgotten in each.
    ///
    /// What stays with each test is its `PictureState`: the ground, the
    /// tuning, the clocks, the notes and the dial under test are the reading
    /// conditions, and those are the expensive part of a test rather than the
    /// plumbing this holds.
    struct ProbeSheet {
        renderer: Renderer,
        context: egui::Context,
        /// Where each pane stands, in points.
        placements: Vec<(harmonigraph_ui::Pane, egui::Rect)>,
        screen: egui::Rect,
        size: [u32; 2],
        pixels_per_point: f32,
    }

    impl ProbeSheet {
        /// `None` when the machine has no usable GPU adapter — every caller
        /// says so and returns, a probe having nothing to assert.
        fn new(size: [u32; 2], pixels_per_point: f32) -> Option<ProbeSheet> {
            let renderer = Renderer::new(size)?;
            let context = egui::Context::default();
            harmonigraph_ui::theme::apply_theme(&context);
            context.set_pixels_per_point(pixels_per_point);

            let layout = single_pane(harmonigraph_ui::Pane::Lattice);
            let points =
                egui::vec2(size[0] as f32 / pixels_per_point, size[1] as f32 / pixels_per_point);
            Some(ProbeSheet {
                renderer,
                context,
                placements: layout.resolve(points),
                screen: egui::Rect::from_min_size(egui::Pos2::ZERO, points),
                size,
                pixels_per_point,
            })
        }

        /// Draw one frame at `now` and hand back its pixels, tightly packed
        /// RGBA8 — for the fixture that MEASURES a frame rather than saving it,
        /// and for the probe that steps a clock and keeps only some of what it
        /// draws.
        fn frame(&mut self, state: &mut PictureState, now: f64) -> Vec<u8> {
            let input =
                crate::render::frame_input(self.screen, now, self.renderer.max_texture_side());
            let placements = &self.placements;
            let output = self.context.run_ui(input, |ui| {
                harmonigraph_ui::draw_panes(ui, placements, state, now);
            });
            let primitives = self.context.tessellate(output.shapes, self.pixels_per_point);
            self.renderer.render_to_vec(
                &primitives,
                output.textures_delta,
                self.pixels_per_point,
                // The render pass's clear colour, the picture's ground — not
                // necessarily the ground a probe sets on its own state.
                harmonigraph_ui::theme::picture(),
            )
        }

        /// Write already-drawn pixels to `target/scratch/<name>.png` and print
        /// where they went, which is what makes the evidence readable under
        /// `--nocapture`.
        fn save(&self, bytes: &[u8], name: &str) {
            let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../target/scratch");
            std::fs::create_dir_all(&dir).expect("a scratch directory");
            let path = dir.join(format!("{name}.png"));
            image::save_buffer(
                &path,
                bytes,
                self.size[0],
                self.size[1],
                image::ExtendedColorType::Rgba8,
            )
            .expect("write the png");
            eprintln!("{}", path.canonicalize().unwrap_or(path.clone()).display());
        }
    }

    /// Hold `notes` down from t = 0 at full velocity.
    fn hold(state: &mut PictureState, notes: &[u8]) {
        for &note in notes {
            state.runtime.tracker.handle_event(harmonigraph_core::NoteEvent::on(
                0.0,
                harmonigraph_core::SourceId::DIRECT,
                0,
                note,
                1.0,
            ));
        }
    }

    #[test]
    fn roll_density_retains_pixel_gaps_and_a_continuous_textured_body() {
        use harmonigraph_core::{NoteEvent, NoteEventKind, SourceId};
        use harmonigraph_ui::{Pane, SpectralOrientation};
        for ppp in [1.0, 2.0] {
            let size = [(960.0 * ppp) as u32, (320.0 * ppp) as u32];
            let Some(mut sheet) = ProbeSheet::new(size, ppp) else { return };
            sheet.placements = single_pane(Pane::Spectral).resolve(egui::vec2(960.0, 320.0));
            for span in [4.0, 20.0, 80.0] {
                let mut state = PictureState::new(FORMAT);
                state.set_background((24, 25, 29));
                let cfg = &mut state.appearance.spectrum;
                cfg.orientation = SpectralOrientation::Left;
                cfg.show_roll = true;
                cfg.show_spectrogram = false;
                cfg.roll_fraction = 1.0;
                cfg.roll_seconds = span;
                cfg.roll_lead = 0.0;
                cfg.low_midi = 55.0;
                cfg.high_midi = 79.0;
                cfg.roll_thickness = 0.45;
                cfg.note_names = false;
                // Three performances ending at the same time: long notes with
                // tiny rests, tiny taps, and a much faster repeated-note run.
                for (note, period, duration) in
                    [(60, 0.12, 0.119), (64, 0.12, 0.006), (68, 0.025, 0.012)]
                {
                    let count = (12.0 / period) as usize;
                    for i in 0..count {
                        let start = 1.0 + i as f64 * period;
                        state.runtime.tracker.handle_event(NoteEvent::on(
                            start,
                            SourceId::DIRECT,
                            0,
                            note,
                            1.0,
                        ));
                        // Onset tuning must not leave a short line below the
                        // first padded note of the dense run.
                        if note == 68 {
                            state.runtime.tracker.handle_event(NoteEvent {
                                source: SourceId::DIRECT,
                                time: start,
                                channel: 0,
                                note,
                                kind: NoteEventKind::Tuning { semitones: 0.3 },
                            });
                        }
                        state.runtime.tracker.handle_event(NoteEvent::off(
                            start + duration,
                            SourceId::DIRECT,
                            0,
                            note,
                        ));
                    }
                }
                // A sustained note is the plain-fill reference.
                state.runtime.tracker.handle_event(NoteEvent::on(
                    1.0,
                    SourceId::DIRECT,
                    0,
                    71,
                    1.0,
                ));
                state.runtime.tracker.handle_event(NoteEvent::off(13.0, SourceId::DIRECT, 0, 71));
                let bytes = sheet.frame(&mut state, 13.2);
                if std::env::var_os("HARMONIGRAPH_ROLL_EVIDENCE").is_some() {
                    sheet.save(&bytes, &format!("roll-density-{span}-{ppp}x"));
                }
                let range = |note: u8| {
                    let tuning = if note == 68 { 0.3 } else { 0.0 };
                    let y =
                        ((320.0 - (f32::from(note - 55) + tuning) * 320.0 / 24.0) * ppp) as usize;
                    // Leave room for the bloom to fall off at either end.
                    let from = (20.0f32.max(0.2 / span * 960.0 + 12.0) * ppp) as usize;
                    let to = (900.0f32.min(12.2 / span * 960.0 - 12.0) * ppp) as usize;
                    let values: Vec<_> = (from..to)
                        .map(|x| {
                            let i = (y * size[0] as usize + x) * 4;
                            // Bloom can saturate one channel while the inset
                            // mark remains visible in the other two.
                            (bytes[i..i + 3].iter().map(|&v| u16::from(v)).sum::<u16>() / 3) as u8
                        })
                        .collect();
                    (*values.iter().min().unwrap(), *values.iter().max().unwrap())
                };
                let (plain_min, plain_max) = range(71);
                assert!(
                    plain_min >= plain_max.saturating_sub(2),
                    "sustain acquired texture at {span}s/{ppp}x: {plain_min}/{plain_max}"
                );
                if span <= 20.0 {
                    let (gap, body) = range(60);
                    // At the fresh Fill lightness and Tint fade a rest keeps the
                    // surround's tint, so a visible gap is about half the body
                    // (0.51..0.55 measured across these cases), not black.
                    assert!(
                        f32::from(gap) < f32::from(body) * 0.6,
                        "tiny rests lost their visible cuts at {span}s/{ppp}x: {gap}/{body}"
                    );
                }
                if span >= 20.0 {
                    let (dark, light) = range(68);
                    assert!(
                        f32::from(dark) > f32::from(light) * 0.3,
                        "tremolo marks erased the dense fill at {span}s/{ppp}x: {dark}/{light}"
                    );
                    assert!(
                        light.saturating_sub(dark) >= 60,
                        "dense ribbon lost its texture at {span}s/{ppp}x: {dark}/{light}"
                    );
                }
            }
        }
    }

    /// Mono `samples` analyzed into `state`'s spectrum as one run, its newest
    /// frame heard at `now`.
    #[cfg(target_vendor = "apple")]
    fn analyze(
        state: &mut PictureState,
        samples: &[f32],
        rate: f32,
        now: f64,
        cfg: &harmonigraph_ui::SpectrumConfig,
    ) {
        let origin = now - (samples.len() - 1) as f64 / f64::from(rate);
        let mut analysis = harmonigraph_ui::SpectrumAnalysis::default();
        let spectrum = &mut state.runtime.spectrum;
        analysis
            .feed_source(samples, 1, rate, origin, cfg, |delivery| spectrum.apply(delivery, cfg));
    }

    /// A sawtooth at MIDI `midi`, one second of it: every harmonic to Nyquist
    /// at amplitude 1/k, a sound with energy across the analyzer's axis.
    #[cfg(target_vendor = "apple")]
    fn sawtooth(midi: f32, rate: f32) -> Vec<f32> {
        let f = harmonigraph_core::spectrum::midi_to_hz(midi);
        (0..rate as usize)
            .map(|i| {
                let t = i as f32 / rate;
                let mut sum = 0.0;
                let mut k = 1.0f32;
                while k * f < rate * 0.45 {
                    sum += (std::f32::consts::TAU * k * f * t).sin() / k;
                    k += 1.0;
                }
                sum
            })
            .collect()
    }

    /// A node is a LAMP with a sheet behind it, not a hole: its own middle is
    /// at least as bright as the ground a few pixels away, at one sheet and at
    /// two alike.
    ///
    /// The pair is the reading, not either number alone. A node's middle holds
    /// its own light plus every neighbour's, and so does the ground between the
    /// nodes; what says the light is UNDER the node is that the middle is not
    /// the darker of the two. Adding a sevens sheet must not change which way
    /// that reads, and it is the sevens stack alone that moves between the two
    /// shots here.
    ///
    /// Read at a wide Reach because that is the regime where it matters most:
    /// the light a node stands in is then mostly its NEIGHBOURS', which is
    /// exactly what a pass taking the light of the sheets behind off a node's
    /// body would take away. This is #435's measurement, made an assertion —
    /// it reported one sheet 73.7 against a ground of 54.6 and two sheets 101.3
    /// against 128.1, the relation inverting on the sheet count alone.
    ///
    /// The two sample points are fixed pixels: the C node sits at the middle of
    /// the pane under this camera in both shots, and (600, 690) is ground clear
    /// of every node's ink. A camera change moves both, which is why the
    /// fixture sets its own rather than taking the layout's.
    #[test]
    fn a_node_with_a_sheet_behind_it_is_still_a_lamp() {
        const SIZE: [u32; 2] = [1200, 1000];
        const PPP: f32 = 2.0;
        const NOW: f64 = 1.0;

        let Some(mut sheet) = ProbeSheet::new(SIZE, PPP) else {
            eprintln!("no usable GPU adapter; nothing rendered");
            return;
        };

        // One luma out of a pixel, the same weighting for both points.
        let at = |b: &[u8], x: u32, y: u32| {
            let i = ((y * SIZE[0] + x) * 4) as usize;
            0.2126 * b[i] as f64 + 0.7152 * b[i + 1] as f64 + 0.0722 * b[i + 2] as f64
        };
        let mut read = |extent: i32| -> (f64, f64) {
            let mut state = PictureState::new(FORMAT);
            state.set_background((24, 25, 29));
            state.appearance.view.fade_time = 0.0;
            // Settle the light in one frame, then release it onto a long tail
            // for the measured frame. The note names are consequently gone
            // while the light remains, so the centre pixel isolates the glow
            // instead of reading a forced-white glyph and its shadow.
            state.appearance.view.glow_attack = 0.0;
            state.appearance.view.glow_release = harmonigraph_scene::GLOW_BALLISTICS_MAX;
            state.appearance.view.min_sevens = -extent;
            state.appearance.view.max_sevens = extent;
            // The off-sheet nodes at the home sheet's own size, so what differs
            // between the two shots is the sheet COUNT and not how big anything
            // on it is drawn.
            state.appearance.view.sevens_size = 1.0;
            state.appearance.view.glow_reach = 4.0;
            // A compact falloff keeps the ground below saturation, leaving the
            // missing light under the node measurable at this wide reach.
            state.appearance.view.glow_curve.shape = 2.75;
            const NOTES: [u8; 4] = [60, 64, 67, 70];
            hold(&mut state, &NOTES);
            state.appearance.camera.zoom_by(2.0);
            let _ = sheet.frame(&mut state, 0.0);
            for note in NOTES {
                state.runtime.tracker.handle_event(harmonigraph_core::NoteEvent::off(
                    0.0,
                    harmonigraph_core::SourceId::DIRECT,
                    0,
                    note,
                ));
            }
            let bytes = sheet.frame(&mut state, NOW);
            (at(&bytes, 600, 500), at(&bytes, 600, 690))
        };

        let (flat_centre, flat_ground) = read(0);
        let (sheets_centre, sheets_ground) = read(1);
        eprintln!("one sheet:  centre {flat_centre:.1}  ground {flat_ground:.1}");
        eprintln!("two sheets: centre {sheets_centre:.1}  ground {sheets_ground:.1}");
        // Non-vacuous: a frame with no light in it reads 0 at both points and
        // satisfies every comparison below.
        assert!(
            flat_centre > 1.0 && sheets_centre > 1.0,
            "the fixture drew no light at the node's middle",
        );
        assert!(
            flat_centre >= flat_ground,
            "on a flat lattice the node's middle ({flat_centre:.1}) is darker than the ground \
             beside it ({flat_ground:.1})",
        );
        assert!(
            sheets_centre >= sheets_ground,
            "with a sevens sheet the node's middle ({sheets_centre:.1}) is darker than the \
             ground beside it ({sheets_ground:.1}): its own light is being taken off its body",
        );
    }

    /// A resize holds one size's targets at a time (#1508). Every target keyed
    /// on a pane's size — the lattice's offscreen set, the roll's and the
    /// spectral cloud's — is dropped before its replacement
    /// is built, and so is each spectral pane's shadow atlas, which grows in
    /// place (the lattice's is rebuilt with its offscreen set). A rebuild that built first would hold two of the
    /// target it rebuilds, so no texture's label may ever have had more copies
    /// alive at once than it holds at rest (#1518).
    ///
    /// Counted per label rather than read off the byte peak, a doubled owner
    /// shows whatever its place in the frame: in the peak it showed only where
    /// its old set outweighed what the owners preparing after it had yet to
    /// grow, which hid a doubled shadow atlas.
    ///
    /// The default look in both panes. Each owner's target is checked on the
    /// record at both sizes, since a target never built would pass this for
    /// the wrong reason (#450). Built new before old, each owner's rebuild
    /// fails this, one at a time.
    #[cfg(target_vendor = "apple")]
    #[test]
    fn a_resize_holds_one_sizes_targets_at_a_time() {
        use harmonigraph_render::gpu_memory::{most_copies_of, snapshot_of, GpuMemory, Shape};
        use std::collections::{BTreeMap, BTreeSet};

        // One target from each owner that rebuilds on a pane's size, each
        // of which must be seen rebuilt for the counts below to reach it.
        const KEYED: [&str; 6] = [
            "lattice_offscreen_color",
            "roll_holdout",
            "spectral_cloud_source",
            "lattice_shadow_atlas",
            "spectral_shadow_atlas",
            "lattice_material_tile_a",
        ];
        // B is picked so that every target above crosses a step from A: the
        // Watercolor tile pinned below goes from 384 to 512 texels, which a
        // few hundred pixels more height does not reach, and the lattice's
        // shadow atlas grows too. The first loop below checks each did.
        const A: [u32; 2] = [1600, 1000];
        const B: [u32; 2] = [2600, 1600];
        const RATE: f32 = 48_000.0;

        let Some(mut sheet) = ProbeSheet::new(A, 1.0) else { return };
        let layout = harmonigraph_ui::Layout::split(harmonigraph_ui::LatticeSide::Left, 0.68);
        let mut state = PictureState::new(FORMAT);
        // Notes for the roll and a second of sound for the cloud, neither of
        // which builds a target without.
        hold(&mut state, &[55, 60, 64, 67, 71]);
        // A lit Watercolor glow, whose tile is sized on the pane, at the
        // texture size and render scale the sizes above were picked for: at
        // a render scale of 0.5, a valid setting, the shadow atlas does not
        // grow between them.
        let view = &mut state.appearance.view;
        (view.glow_reach, view.glow_strength) = (1.0, 1.0);
        view.render_scale = 1.0;
        view.atmosphere.material_style = harmonigraph_scene::LatticeMaterial::Watercolor;
        view.atmosphere.material_amount = 1.0;
        view.atmosphere.material_settings.wash_size = 1.459_047_9;
        let mut now = 1.0;
        let cfg = state.appearance.spectrum;
        analyze(&mut state, &sawtooth(55.0, RATE), RATE, now, &cfg);
        let params = crate::replay::ReplayParams::default();

        // A few frames at `size`, after reading back every frame in flight,
        // as a surface reconfigure waits for the GPU.
        let mut settle = |size: [u32; 2]| -> (GpuMemory, BTreeMap<String, usize>) {
            sheet.renderer.resize(size);
            let points = egui::vec2(size[0] as f32, size[1] as f32);
            let screen = egui::Rect::from_min_size(egui::Pos2::ZERO, points);
            for _ in 0..3 {
                now += 1.0 / 30.0;
                harmonigraph_ui::begin_frame(&mut state, &params, now);
                let input =
                    crate::render::frame_input(screen, now, sheet.renderer.max_texture_side());
                let output = sheet.context.run_ui(input, |ui| {
                    harmonigraph_ui::draw_panes(ui, &layout.resolve(points), &mut state, now);
                });
                let primitives = sheet.context.tessellate(output.shapes, 1.0);
                let clear = egui::Color32::BLACK;
                sheet.renderer.render_to_vec(&primitives, output.textures_delta, 1.0, clear);
            }
            let device = &sheet.renderer.device;
            (snapshot_of(device), most_copies_of(device))
        };
        let (at_a, built) = settle(A);
        let (at_b, grown) = settle(B);
        let (back, shrunk) = settle(A);

        let shapes = |record: &GpuMemory, label: &str| -> Vec<_> {
            record.resources.iter().filter(|r| r.label == label).map(|r| r.shape.clone()).collect()
        };
        for label in KEYED {
            let (a, b) = (shapes(&at_a, label), shapes(&at_b, label));
            assert!(
                !a.is_empty() && b.iter().any(|s| !a.contains(s)),
                "{label} must be built at both sizes: {a:?}, then {b:?}"
            );
        }
        // Every texture, not only the owners named above: one rebuilt on a
        // size this list has not met is held twice just the same. Buffers
        // are left out, the per-frame ones growing to demand beside their
        // old copy, which is a size but not a pane's.
        let steady = [&at_a, &at_b, &back];
        let textures = steady.iter().flat_map(|r| &r.resources);
        let textures = textures.filter(|r| matches!(r.shape, Shape::Texture { .. }));
        let labels: BTreeSet<&str> = textures.map(|r| r.label.as_str()).collect();
        // The counts are the most since the device was made, so each step's
        // takes in those before it.
        for (step, most) in [("building", &built), ("growing", &grown), ("shrinking", &shrunk)] {
            for &label in &labels {
                let held = steady.map(|r| shapes(r, label).len()).into_iter().max();
                let most = most.get(label).copied();
                assert!(
                    most <= held,
                    "{step}: {label} had {most:?} alive at once, against {held:?} at rest\n{}",
                    back.table()
                );
            }
        }
    }
}
