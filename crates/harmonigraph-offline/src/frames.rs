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

/// Everything needed to render frames, set up once.
pub struct Renderer {
    device: wgpu::Device,
    queue: wgpu::Queue,
    egui: egui_wgpu::Renderer,
    target: wgpu::Texture,
    view: wgpu::TextureView,
    readback: wgpu::Buffer,
    size: [u32; 2],
    /// Padded row stride of `readback`; wgpu requires 256-byte alignment,
    /// which an arbitrary width does not give us.
    bytes_per_row: u32,
}

/// Round `bytes` up to wgpu's copy alignment.
fn aligned(bytes: u32) -> u32 {
    let align = wgpu::COPY_BYTES_PER_ROW_ALIGNMENT;
    bytes.div_ceil(align) * align
}

impl Renderer {
    /// `size` is in physical pixels. Returns `None` if the machine has no
    /// usable GPU adapter (CI containers, mostly) — callers decide
    /// whether that is fatal.
    pub fn new(size: [u32; 2]) -> Option<Renderer> {
        let instance = wgpu::Instance::default();
        let adapter =
            pollster::block_on(instance.request_adapter(&wgpu::RequestAdapterOptions::default()))
                .ok()?;
        let (device, queue) =
            pollster::block_on(adapter.request_device(&wgpu::DeviceDescriptor::default())).ok()?;

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

        let target = device.create_texture(&wgpu::TextureDescriptor {
            label: Some("offline frame"),
            size: wgpu::Extent3d {
                width: size[0],
                height: size[1],
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: FORMAT,
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::COPY_SRC,
            view_formats: &[],
        });
        let view = target.create_view(&Default::default());

        let bytes_per_row = aligned(size[0] * 4);
        let readback = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("offline readback"),
            size: u64::from(bytes_per_row) * u64::from(size[1]),
            usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
            mapped_at_creation: false,
        });

        Some(Renderer { device, queue, egui, target, view, readback, size, bytes_per_row })
    }

    /// Paint one frame's tessellated shapes and read the result back as
    /// tightly packed RGBA8 (row padding removed).
    pub fn render(
        &mut self,
        primitives: &[egui::ClippedPrimitive],
        textures: &egui::TexturesDelta,
        pixels_per_point: f32,
        clear: egui::Color32,
    ) -> Vec<u8> {
        for (id, delta) in &textures.set {
            self.egui.update_texture(&self.device, &self.queue, *id, delta);
        }

        let descriptor = egui_wgpu::ScreenDescriptor {
            size_in_pixels: self.size,
            pixels_per_point,
        };
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

        encoder.copy_texture_to_buffer(
            self.target.as_image_copy(),
            wgpu::TexelCopyBufferInfo {
                buffer: &self.readback,
                layout: wgpu::TexelCopyBufferLayout {
                    offset: 0,
                    bytes_per_row: Some(self.bytes_per_row),
                    rows_per_image: None,
                },
            },
            wgpu::Extent3d {
                width: self.size[0],
                height: self.size[1],
                depth_or_array_layers: 1,
            },
        );
        self.queue.submit(callback_commands.into_iter().chain([encoder.finish()]));

        let slice = self.readback.slice(..);
        slice.map_async(wgpu::MapMode::Read, |r| r.expect("map readback buffer"));
        self.device.poll(wgpu::PollType::wait_indefinitely()).expect("poll");
        let row_bytes = (self.size[0] * 4) as usize;
        let frame = {
            let mapped = slice.get_mapped_range();
            let mut frame = Vec::with_capacity(row_bytes * self.size[1] as usize);
            for row in 0..self.size[1] as usize {
                let start = row * self.bytes_per_row as usize;
                frame.extend_from_slice(&mapped[start..start + row_bytes]);
            }
            frame
        };
        self.readback.unmap();

        for id in &textures.free {
            self.egui.free_texture(id);
        }
        frame
    }
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
    wgpu::Color {
        r: channel(color.r()),
        g: channel(color.g()),
        b: channel(color.b()),
        a: 1.0,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

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

    /// Which of the lattice's three readings a shot is of: the MIDI picture
    /// alone, the audio ring over it at a given Range, or the nodes themselves
    /// lit from the analyzer.
    #[derive(Clone, Copy)]
    enum Shot {
        Midi,
        Ring(f32),
        Lit,
    }

    /// A sawtooth at MIDI `midi`, one second of it: every harmonic to Nyquist
    /// at amplitude 1/k, which is the signal the whole spectral-lattice family
    /// is judged on — its constellation is PREDICTED by the harmonic series
    /// rather than measured (see `panes::spectral_fold`'s own tests).
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

    /// The audio ring's picture, written to `target/scratch/` — the only way
    /// to LOOK at this change without the DAW.
    ///
    /// A probe: it asserts nothing, because what it produces is a judgement
    /// (#381's verdict is Yan's, at the plugin). It is kept, and kept
    /// `#[ignore]`d, because the reading conditions are the expensive part
    /// rather than the plumbing — #351 measured that the fresh extents draw a
    /// C3 saw as a haze of comma neighbours, so a picture taken at them says
    /// nothing about the ring, and the two settings below (extents 4 × 3, the
    /// Analyzer's floor at −35 dB) are what make the constellation legible.
    /// Rebuilding those from the issue costs more than the rest of this put
    /// together.
    ///
    /// ```text
    /// cargo test -p harmonigraph-offline -- --ignored --nocapture audio_ring
    /// ```
    ///
    /// Same note held as sounding, so both readings are on screen at once: the
    /// held C3 lights its own node's band, and the ring inside it reads the
    /// saw's own spectrum around each of that node's octaves.
    ///
    /// The RANGE sweep is what the fresh value was chosen against, and it is
    /// the reason this writes more than two shots: the setting decides whether
    /// a wedge is a zoom on the node's own pitch or a copy of the whole wheel,
    /// and the two ends look nothing alike.
    #[test]
    #[ignore = "a probe: writes PNGs and asserts nothing"]
    fn the_audio_ring_draws_a_picture() {
        use harmonigraph_ui::{draw_pane, Layout, SharedState};

        const SIZE: [u32; 2] = [1200, 1000];
        // Retina-ish, so the wedges and the note names are resolved rather
        // than aliased — this is a picture to be looked at, not a fixture to
        // be measured.
        const PPP: f32 = 2.0;
        const RATE: f32 = 48_000.0;
        // A second in: long enough that the analyzer's window is full of the
        // steady spectrum rather than of its own attack.
        const NOW: f64 = 1.0;

        let Some(mut renderer) = Renderer::new(SIZE) else {
            eprintln!("no usable GPU adapter; nothing rendered");
            return;
        };
        let context = egui::Context::default();
        harmonigraph_ui::theme::apply_theme(&context);
        context.set_pixels_per_point(PPP);

        let layout = Layout::preset("lattice").expect("the lattice preset");
        let mut state = SharedState::new(FORMAT);
        state.set_background(layout.background);
        // Just intonation, which is what the panel is aimed at: a partial of a
        // just-tuned note lands ON its node rather than near it.
        state.tuning = harmonigraph_core::Tuning::just();
        // #351's reading conditions, and the reason this probe exists. Its
        // "pull the extents in" is a ZOOM here: since #357 the drawn window is
        // whatever the camera is looking at, and the extents set the naming
        // reach rather than the picture's edge — so the way to have fewer
        // nodes on screen is to look at fewer of them. The floor is the other
        // half, and it is unchanged: at −60 dB the comma neighbours haze over
        // the constellation.
        state.view.extent_threes = 4;
        state.view.extent_fives = 3;
        state.spectrum_config.floor_db = -35.0;
        // The display's EMA is the Analyzer's Smoothing control. Off here so
        // the picture is the spectrum of the second that was pushed rather
        // than a function of how many hops this fixture happened to feed.
        state.spectrum_config.smoothing = 0.0;
        // Fully lit at once: an envelope would put the MIDI half of the
        // picture part way through its arrival.
        state.frame_params.fade_time = 0.0;
        state.tracker.handle_event(harmonigraph_core::NoteEvent::on(0.0, 0, 48, 1.0));
        let cfg = state.spectrum_config;
        state.spectrum.push_samples(&sawtooth(48.0, RATE), 1, RATE, NOW, &cfg);

        let points = egui::vec2(SIZE[0] as f32 / PPP, SIZE[1] as f32 / PPP);
        let screen = egui::Rect::from_min_size(egui::Pos2::ZERO, points);
        let placements = layout.resolve(points);
        let background = egui::Color32::from_rgb(
            layout.background.0,
            layout.background.1,
            layout.background.2,
        );
        let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../target/scratch");
        std::fs::create_dir_all(&dir).expect("a scratch directory");

        // Two distances, because the two questions are asked at different
        // ones. Whether the constellation READS is a question about a screen
        // full of nodes; whether the ring sits clear of the core and the
        // melody ring, and what one wedge is actually showing, is one about a
        // single node — and at reading distance a wedge is a dozen pixels.
        //
        // The Range sweep runs at both, because the setting trades exactly
        // between them: narrow is legible close up and a smear at distance,
        // and the octave end is one picture repeated on every node, which only
        // shows on a screen full of them.
        //
        // `Light from audio` rides along at both distances, because it is the
        // OTHER half of the frequency colour scheme: the band and the bodies
        // painted by level off the analyzer's ramp rather than by pitch off
        // the lattice's, which is a thing to look at rather than to assert.
        let fresh_range = state.view.spectral_ring_range;
        let mut shots: Vec<(f32, &str, Shot)> = Vec::new();
        for (zoom, at) in [(2.5f32, ""), (9.0, "-close")] {
            shots.push((zoom, at, Shot::Midi));
            for range in [50.0f32, fresh_range, 600.0, 1200.0] {
                shots.push((zoom, at, Shot::Ring(range)));
            }
            shots.push((zoom, at, Shot::Lit));
        }

        let home = state.camera;
        for (zoom, at, shot) in shots {
            let source = match shot {
                Shot::Midi => "audio-ring-off".to_string(),
                Shot::Lit => "light-from-audio".to_string(),
                Shot::Ring(range) => {
                    state.view.spectral_ring_range = range;
                    format!("audio-ring-{range:.0}c")
                }
            };
            state.view.spectral_ring = matches!(shot, Shot::Ring(_));
            state.view.spectral_light = matches!(shot, Shot::Lit);
            // From the fresh camera each time: the pane pans the view's center
            // with the camera, so a zoom applied on top of the last one would
            // compound.
            state.camera = home;
            state.camera.zoom_by(zoom);
            let output = context.run_ui(
                egui::RawInput {
                    screen_rect: Some(screen),
                    time: Some(NOW),
                    ..Default::default()
                },
                |ui| {
                    for (pane, rect) in &placements {
                        let mut child = ui.new_child(egui::UiBuilder::new().max_rect(*rect));
                        draw_pane(&mut child, *pane, &mut state, NOW);
                    }
                },
            );
            let primitives = context.tessellate(output.shapes, PPP);
            let bytes = renderer.render(&primitives, &output.textures_delta, PPP, background);
            let path = dir.join(format!("{source}{at}.png"));
            image::save_buffer(&path, &bytes, SIZE[0], SIZE[1], image::ExtendedColorType::Rgba8)
                .expect("write the png");
            eprintln!("{}", path.canonicalize().unwrap_or(path.clone()).display());
        }
    }
}
