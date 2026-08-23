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

    /// The widest texture this device will take, on either side.
    ///
    /// The context has to be TOLD this — `RawInput::max_texture_side` left
    /// `None` makes egui report its own 2048 default, and the spectrogram
    /// reads that limit for both axes of the heatmap
    /// (`harmonigraph_ui::spectrogram::Plan`): rows are clamped to it directly
    /// and slabs through `slab_ceiling`. Left unfilled, a 4K export plans
    /// against a quarter of the area this device takes, and nothing on screen
    /// or in stderr says so — issue #368. The editor has no such gap: the
    /// vendored egui-baseview passes its renderer's limit in, off this same
    /// wgpu call.
    ///
    /// The DEVICE's number rather than a lower one chosen here, because the
    /// caps that decide what a spectrogram should spend already exist a layer
    /// up (`LIVE_SLAB_CAP`, `WHOLE_SONG_SLAB_CAP`, and the pane's own pixel
    /// count) and are the ones meant to bind. A second, quieter cap in this
    /// crate would make an offline frame differ from the editor's for a reason
    /// no pane could report.
    pub fn max_texture_side(&self) -> usize {
        self.device.limits().max_texture_dimension_2d as usize
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

    /// Which of the ring's readings a shot is of: none of them (the MIDI
    /// picture alone), the raw spectrum at a given Range, the fold, or the fold
    /// at a stated Gate — how loud a node's loudest wedge must read for that
    /// node to wear a ring at all.
    #[derive(Clone, Copy)]
    enum Shot {
        Midi,
        Spectrum(f32),
        Fold,
        Gate(f32),
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

    /// The node glow's picture, written to `target/scratch/` — a sweep of the
    /// Reach and the Feather together, which is the pair that decides whether
    /// the light is an accent on each node or a field the lattice sits in.
    ///
    /// A probe: it asserts nothing, the verdict being a look rather than a
    /// number. Kept and `#[ignore]`d for the same reason the ring's is: the
    /// expensive part is the reading conditions rather than the plumbing. The
    /// chord is read under the DEFAULT tuning, `Tuning::just()` putting a whole
    /// chord on one node and so leaving nothing for a halo to overlap; the
    /// camera is at the far distance, several nodes on screen, because a field
    /// is a claim about what light does BETWEEN nodes; the note Fade and the
    /// light's own clock are off, so one frame is the whole picture rather than
    /// a shot of an envelope part way through; and the ground is the skin's
    /// panel rather than the preset's near-black, which is what a wash is
    /// actually laid over in the DAW.
    ///
    /// The Strength comes down as the Reach goes up, deliberately: the light is
    /// SCREEN-blended, so a wide flat halo on every node of a chord saturates
    /// to white at a strength that was right for an accent, and the shots would
    /// then be a picture of the clamp.
    ///
    /// ```text
    /// cargo test -p harmonigraph-offline -- --ignored --nocapture node_glow
    /// ```
    #[test]
    #[ignore = "a probe: writes PNGs and asserts nothing"]
    fn the_node_glow_draws_a_picture() {
        use harmonigraph_ui::{draw_pane, Layout, SharedState};

        const SIZE: [u32; 2] = [1200, 1000];
        const PPP: f32 = 2.0;
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
        // The DAW's own lattice ground rather than the preset's near-black, so
        // what the light lands on here is what it lands on there.
        state.set_background((24, 25, 29));
        state.frame_params.fade_time = 0.0;
        // The light's own clock off: one frame is the whole picture, and a
        // halo part way through its attack is a shot of the ballistics.
        state.view.glow_attack = 0.0;
        state.view.glow_release = 0.0;
        for note in [55u8, 60, 64, 67, 71] {
            state.tracker.handle_event(harmonigraph_core::NoteEvent::on(0.0, 0, note, 1.0));
        }

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

        let fresh = harmonigraph_scene::ViewConfig::default();
        let shots: Vec<(f32, f32, f32)> = vec![
            (fresh.glow_reach, 0.0, fresh.glow_strength),
            (2.0, 0.0, 1.0),
            (2.0, 1.0, 1.0),
            (4.0, 0.0, 1.0),
            (4.0, 1.0, 0.6),
            (8.0, 1.0, 0.4),
        ];
        let home = state.camera;
        for (reach, feather, strength) in shots {
            state.camera = home;
            state.camera.zoom_by(2.5);
            state.view.glow_reach = reach;
            state.view.glow_feather = feather;
            state.view.glow_strength = strength;
            let output = context.run_ui(
                egui::RawInput {
                    screen_rect: Some(screen),
                    time: Some(NOW),
                    // The device's own limit, as the render loop reports it —
                    // a probe drawn against a different ceiling from the export
                    // is a probe of a picture nothing ships.
                    max_texture_side: Some(renderer.max_texture_side()),
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
            let path = dir.join(format!(
                "node-glow-reach{:.0}-feather{:.0}.png",
                reach * 100.0,
                feather * 100.0,
            ));
            image::save_buffer(&path, &bytes, SIZE[0], SIZE[1], image::ExtendedColorType::Rgba8)
                .expect("write the png");
            eprintln!("{}", path.canonicalize().unwrap_or(path.clone()).display());
        }
    }

    /// The standoff against the OCTAVE GAP, written to `target/scratch/`: what
    /// a node's light does between its slices as the angular padding widens.
    ///
    /// A probe: it asserts nothing, the verdict being a look. The share of its
    /// light a gap keeps is measured instead, and exactly, by
    /// harmonigraph-render's `the_standoff_follows_the_gaps_between_the_slices`.
    /// What that number cannot say is the thing this is for — whether a node
    /// with the field coming through it still reads as one object rather than
    /// as a ring of unrelated marks.
    ///
    /// The reading conditions, which are the expensive part: the Reach up at a
    /// Strength that does NOT climb with it, since the light is screen-blended
    /// and a wide halo on every node of a chord saturates to white at a strength
    /// that was right for an accent; the light's own clock off, so one frame is
    /// the whole picture rather than a shot of the ballistics; the DAW's ground
    /// rather than the preset's near-black; and a zoom that puts one node's
    /// slices across a good part of the frame.
    ///
    /// `PROBE_TAG` names the shots, which is what makes a BEFORE and an AFTER of
    /// one look: sabotage `slice_gap_distance` in the shader to return `-d`,
    /// which is the picture with no angular term in it, shoot under one tag,
    /// restore and shoot under another.
    ///
    /// ```text
    /// cargo test -p harmonigraph-offline -- --ignored --nocapture the_standoff_against
    /// ```
    #[test]
    #[ignore = "a probe: writes PNGs and asserts nothing"]
    fn the_standoff_against_the_octave_gap() {
        use harmonigraph_ui::{draw_pane, Layout, SharedState};

        const SIZE: [u32; 2] = [1200, 1000];
        const PPP: f32 = 2.0;
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
        state.set_background((24, 25, 29));
        state.frame_params.fade_time = 0.0;
        state.view.glow_attack = 0.0;
        state.view.glow_release = 0.0;
        state.view.glow_reach = 2.0;
        state.view.glow_strength = 1.0;
        for note in [55u8, 60, 64, 67, 71] {
            state.tracker.handle_event(harmonigraph_core::NoteEvent::on(0.0, 0, note, 1.0));
        }

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
        let tag = std::env::var("PROBE_TAG").unwrap_or_else(|_| "after".to_string());

        let home = state.camera;
        for gap in [0.05f32, 0.2, 0.4] {
            state.camera = home;
            state.camera.zoom_by(3.5);
            state.view.octave_gap = gap;
            let output = context.run_ui(
                egui::RawInput {
                    screen_rect: Some(screen),
                    time: Some(NOW),
                    max_texture_side: Some(renderer.max_texture_side()),
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
            let path = dir.join(format!("gap{:.0}-{tag}.png", gap * 100.0));
            image::save_buffer(&path, &bytes, SIZE[0], SIZE[1], image::ExtendedColorType::Rgba8)
                .expect("write the png");
            eprintln!("{}", path.canonicalize().unwrap_or(path.clone()).display());
        }
    }

    /// The resting marker field's picture, written to `target/scratch/` — a
    /// sweep of the three bars that shape a cross: how far its arms reach, how
    /// thick they are, and how much of each end fades out. Together they decide
    /// whether the lattice at rest is a field of marks or a field of objects.
    ///
    /// A probe: it asserts nothing, the verdict being a look rather than a
    /// number, and it is kept and `#[ignore]`d for the reason the two below it
    /// are — the reading conditions are the expensive part.
    ///
    /// Those conditions: NOTHING sounding, because the subject is what an
    /// unplayed lattice draws and a chord over it is exactly the thing that
    /// hides it; the camera pulled back so several rows are on screen, a field
    /// being a claim about regularity rather than about one marker; and the
    /// skin's panel as the ground rather than the preset's near-black, because
    /// the markers are the ground's own grey a step above it and the whole
    /// judgement is how far above.
    ///
    /// One shot holds a held chord, and it is the one that answers the
    /// question the arm bar is really for: a node arriving has to COVER its own
    /// marker rather than grow out of one, which is a comparison between the
    /// arm's reach and where the node's rings start.
    ///
    /// Two shots are the NAMES, which take a marker off the position they
    /// stand on (`NodeInstance::name_level`): one under Past with a memory
    /// behind it, where names and markers share the field, and one under All,
    /// where the names take the whole of it. That pair is the reading the rule
    /// is for, and neither can be judged from a shot with no type in it.
    ///
    /// ```text
    /// cargo test -p harmonigraph-offline -- --ignored --nocapture resting_markers
    /// ```
    #[test]
    #[ignore = "a probe: writes PNGs and asserts nothing"]
    fn the_resting_markers_draw_a_picture() {
        use harmonigraph_ui::{draw_pane, Layout, SharedState};

        const SIZE: [u32; 2] = [1200, 1000];
        const PPP: f32 = 2.0;
        const NOW: f64 = 1.0;

        let Some(mut renderer) = Renderer::new(SIZE) else {
            eprintln!("no usable GPU adapter; nothing rendered");
            return;
        };
        let context = egui::Context::default();
        harmonigraph_ui::theme::apply_theme(&context);
        context.set_pixels_per_point(PPP);

        let layout = Layout::preset("lattice").expect("the lattice preset");
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

        let fresh = harmonigraph_scene::ViewConfig::default();
        use harmonigraph_scene::NoteNames;
        // Arm length, arm width, taper, whether a chord is held over it, and
        // which names show. The smallest arms earn their shots: a marker's
        // edge is the rings' band, which is a fixed number of PIXELS, so it is
        // at the bottom of the arm bar that the band is most of the marker and
        // the shape has the least room to be the shape it claims.
        let shots: Vec<(f32, f32, f32, bool, NoteNames)> = vec![
            // The fresh marker, then the ends of the arm bar, then the two
            // pictures the naming rule makes of the field.
            (fresh.plus_arm, fresh.plus_width, fresh.plus_taper, false, NoteNames::Played),
            (0.05, fresh.plus_width, fresh.plus_taper, false, NoteNames::Played),
            (0.5, fresh.plus_width, fresh.plus_taper, false, NoteNames::Played),
            (fresh.plus_arm, fresh.plus_width, fresh.plus_taper, true, NoteNames::Past),
            (fresh.plus_arm, fresh.plus_width, fresh.plus_taper, false, NoteNames::All),
            // The width, across its whole span at one arm: a hairline, the
            // fresh proportion, a heavy cross, and the square at the top.
            (fresh.plus_arm, 0.0, fresh.plus_taper, false, NoteNames::Played),
            (fresh.plus_arm, 0.25, fresh.plus_taper, false, NoteNames::Played),
            (fresh.plus_arm, 0.5, fresh.plus_taper, false, NoteNames::Played),
            // The taper, across its whole span: a square end, half the arm,
            // and an arm that fades the whole way from the crossing.
            (fresh.plus_arm, fresh.plus_width, 0.0, false, NoteNames::Played),
            (fresh.plus_arm, fresh.plus_width, 0.5 * fresh.plus_arm, false, NoteNames::Played),
            (fresh.plus_arm, fresh.plus_width, fresh.plus_arm, false, NoteNames::Played),
        ];
        for (size, width, taper, chord, names) in shots {
            let mut state = SharedState::new(FORMAT);
            state.view.show_labels = true;
            state.view.note_names = names;
            // The DAW's own lattice ground rather than the preset's near-black:
            // the markers are a step above the panel and nothing else here says
            // how big a step that reads as.
            state.set_background((24, 25, 29));
            state.frame_params.fade_time = 0.0;
            if chord {
                for note in [55u8, 60, 64, 67, 71] {
                    state.tracker.handle_event(harmonigraph_core::NoteEvent::on(0.0, 0, note, 1.0));
                }
            }
            state.camera.zoom_by(2.5);
            state.view.plus_arm = size;
            state.view.plus_width = width;
            state.view.plus_taper = taper;
            let output = context.run_ui(
                egui::RawInput {
                    screen_rect: Some(screen),
                    time: Some(NOW),
                    // The device's own limit, as the render loop reports it —
                    // a probe drawn against a different ceiling from the export
                    // is a probe of a picture nothing ships.
                    max_texture_side: Some(renderer.max_texture_side()),
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
            let path = dir.join(format!(
                "plus-arm{:.0}-width{:.0}-taper{:.0}{}-{names:?}.png",
                size * 100.0,
                width * 100.0,
                taper * 100.0,
                if chord { "-chord" } else { "" },
            ));
            image::save_buffer(&path, &bytes, SIZE[0], SIZE[1], image::ExtendedColorType::Rgba8)
                .expect("write the png");
            eprintln!("{}", path.canonicalize().unwrap_or(path.clone()).display());
        }
    }

    /// A node is a LAMP with a sheet behind it, not a hole: its own middle is
    /// at least as bright as the ground a few pixels away, at one sheet and at
    /// two alike.
    ///
    /// The pair is the reading, not either number alone. A node's middle holds
    /// its own light plus every neighbour's, and so does the ground between the
    /// nodes; what says the light is UNDER the node is that the middle is not
    /// the darker of the two. Adding a sevens sheet must not change which way
    /// that reads, and it is `extent_sevens` alone that moves between the two
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
        use harmonigraph_ui::{draw_pane, Layout, SharedState};

        const SIZE: [u32; 2] = [1200, 1000];
        const PPP: f32 = 2.0;
        const NOW: f64 = 1.0;

        let Some(mut renderer) = Renderer::new(SIZE) else {
            eprintln!("no usable GPU adapter; nothing rendered");
            return;
        };
        let context = egui::Context::default();
        harmonigraph_ui::theme::apply_theme(&context);
        context.set_pixels_per_point(PPP);

        let layout = Layout::preset("lattice").expect("the lattice preset");
        let points = egui::vec2(SIZE[0] as f32 / PPP, SIZE[1] as f32 / PPP);
        let screen = egui::Rect::from_min_size(egui::Pos2::ZERO, points);
        let placements = layout.resolve(points);
        let background = egui::Color32::from_rgb(
            layout.background.0,
            layout.background.1,
            layout.background.2,
        );

        // One luma out of a pixel, the same weighting for both points.
        let at = |b: &[u8], x: u32, y: u32| {
            let i = ((y * SIZE[0] + x) * 4) as usize;
            0.2126 * b[i] as f64 + 0.7152 * b[i + 1] as f64 + 0.0722 * b[i + 2] as f64
        };
        let mut read = |extent: i32| -> (f64, f64) {
            let mut state = SharedState::new(FORMAT);
            state.set_background((24, 25, 29));
            state.frame_params.fade_time = 0.0;
            // Settled: the light's own clock would otherwise leave a one-frame
            // shot part way up its attack, which is a reading of the ramp.
            state.view.glow_attack = 0.0;
            state.view.glow_release = 0.0;
            state.view.extent_sevens = extent;
            // The off-sheet nodes at the home sheet's own size, so what differs
            // between the two shots is the sheet COUNT and not how big anything
            // on it is drawn.
            state.view.sevens_size = 1.0;
            state.view.glow_reach = 4.0;
            for note in [60u8, 64, 67, 70] {
                state.tracker.handle_event(harmonigraph_core::NoteEvent::on(0.0, 0, note, 1.0));
            }
            state.camera.zoom_by(2.0);
            let output = context.run_ui(
                egui::RawInput {
                    screen_rect: Some(screen),
                    time: Some(NOW),
                    max_texture_side: Some(renderer.max_texture_side()),
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
    /// Same note held as sounding, so both pictures are on screen at once: the
    /// held C3 lights its own node's band, and the ring inside it reads the
    /// saw's own spectrum around each of that node's octaves. That holds under
    /// either reading — the ring never replaces the MIDI picture.
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
        // "pull the extents in" is a ZOOM here: the drawn window is whatever
        // the camera is looking at, and the extents set the naming
        // reach rather than the picture's edge — so the way to have fewer
        // nodes on screen is to look at fewer of them. The floor is the other
        // half, and it is unchanged: at −60 dB the comma neighbours haze over
        // the constellation.
        state.view.extent_threes = 4;
        state.view.extent_fives = 3;
        state.spectrum_config.floor_db = -35.0;
        // The Analyzer's own Attack and Release. Both off here, so the picture
        // is the spectrum of the second that was pushed rather than a function
        // of how many hops this fixture happened to feed.
        state.spectrum_config.attack = 0.0;
        state.spectrum_config.release = 0.0;
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
        // octave band, and what one wedge is actually showing, is one about a
        // single node — and at reading distance a wedge is a dozen pixels.
        //
        // The Range sweep runs at both, because the setting trades exactly
        // between them: narrow is legible close up and a smear at distance,
        // and the octave end is one picture repeated on every node, which only
        // shows on a screen full of them.
        //
        // The FOLD rides along at both distances, because it is the other half
        // of the selector and the pair only means anything side by side: the
        // same wedges in the same annulus, one flat per octave and one a
        // window across it. Which of the two reads better is the judgement
        // this probe exists to put in front of a person.
        //
        // The GATE sweep runs at the far distance alone, and that is its
        // subject rather than a saving: what it decides is which of a screenful
        // of nodes is worth a ring, and one node up close has nothing to say
        // about that. Its low end is the gate off — a ring on every node in
        // view, silence included — which is what the sweep is read against.
        let fresh_range = state.view.spectral_ring_range;
        let mut shots: Vec<(f32, &str, Shot)> = Vec::new();
        for (zoom, at) in [(2.5f32, ""), (9.0, "-close")] {
            shots.push((zoom, at, Shot::Midi));
            for range in [50.0f32, fresh_range, 600.0, 1200.0] {
                shots.push((zoom, at, Shot::Spectrum(range)));
            }
            shots.push((zoom, at, Shot::Fold));
        }
        for gate in [0.0f32, 0.25, 0.4, 0.6] {
            shots.push((2.5, "", Shot::Gate(gate)));
        }

        let home = state.camera;
        for (zoom, at, shot) in shots {
            let source = match shot {
                Shot::Midi => "audio-ring-off".to_string(),
                Shot::Fold => "audio-ring-fold".to_string(),
                Shot::Spectrum(range) => {
                    state.view.spectral_ring_range = range;
                    format!("audio-ring-{range:.0}c")
                }
                Shot::Gate(gate) => format!("audio-ring-gate-{:.0}", gate * 100.0),
            };
            // Every other shot is of the ring's own reading, so they are taken
            // at the gate OFF: a node held back would read as a reading that
            // says nothing there, which is the one thing those shots are for.
            state.view.spectral_ring_gate = match shot {
                Shot::Gate(gate) => gate,
                _ => 0.0,
            };
            // The ring's WIDTH is what turns it off, so the MIDI shot dials it
            // to nothing rather than picking a reading that says "none" — and
            // the octave band closes in over the space it leaves, which is the
            // MIDI picture the stack draws.
            let fresh_width = harmonigraph_scene::ViewConfig::default().spectral_ring_width;
            state.view.spectral_ring_width = match shot {
                Shot::Midi => 0.0,
                _ => fresh_width,
            };
            state.view.spectral_reading = match shot {
                Shot::Fold | Shot::Midi | Shot::Gate(_) => {
                    harmonigraph_scene::SpectralReading::Fold
                }
                Shot::Spectrum(_) => harmonigraph_scene::SpectralReading::Spectrum,
            };
            // From the fresh camera each time: the pane pans the view's center
            // with the camera, so a zoom applied on top of the last one would
            // compound.
            state.camera = home;
            // And from a fresh ring, for the same reason one step further on:
            // every shot here is taken at ONE clock, so a ring carried over
            // from the shot before would still be standing where that shot's
            // Gate put it (both halves step against the clock, so a second
            // frame at the same moment moves nothing). These are pictures of
            // settings rather than frames of an animation, and a fresh ring is
            // what the first frame of each of them draws.
            //
            // BOTH halves, which is why it is one call: the carried levels are
            // the grid the wedges read and the fade is whether the annulus is
            // there at all. Clearing the fade alone leaves the Fold shots and
            // the whole Gate sweep drawn over the RAW spectrum's grid — the
            // one measured first — which reads as the gate admitting far more
            // than it does, and by more the higher the gate is set.
            state.reset_ring();
            state.camera.zoom_by(zoom);
            let output = context.run_ui(
                egui::RawInput {
                    screen_rect: Some(screen),
                    time: Some(NOW),
                    // The device's own limit, as the render loop reports it —
                    // a probe drawn against a different ceiling from the export
                    // is a probe of a picture nothing ships.
                    max_texture_side: Some(renderer.max_texture_side()),
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
