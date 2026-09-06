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
            wgpu::Extent3d { width: self.size[0], height: self.size[1], depth_or_array_layers: 1 },
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
    wgpu::Color { r: channel(color.r()), g: channel(color.g()), b: channel(color.b()), a: 1.0 }
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
    /// Reach against the Strength, which is what decides whether the light is
    /// an accent on each node or a field the lattice sits in.
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
        let background =
            egui::Color32::from_rgb(layout.background.0, layout.background.1, layout.background.2);
        let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../target/scratch");
        std::fs::create_dir_all(&dir).expect("a scratch directory");

        let fresh = harmonigraph_scene::ViewConfig::default();
        let shots: Vec<(f32, f32)> = vec![
            (fresh.glow_reach, fresh.glow_strength),
            (2.0, 1.0),
            (4.0, 1.0),
            (4.0, 0.6),
            (8.0, 0.4),
        ];
        let home = state.camera;
        for (reach, strength) in shots {
            state.camera = home;
            state.camera.zoom_by(2.5);
            state.view.glow_reach = reach;
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
                    for (surface, (pane, rect)) in placements.iter().enumerate() {
                        let mut child = ui.new_child(egui::UiBuilder::new().max_rect(*rect));
                        draw_pane(&mut child, *pane, &mut state, NOW, surface);
                    }
                },
            );
            let primitives = context.tessellate(output.shapes, PPP);
            let bytes = renderer.render(&primitives, &output.textures_delta, PPP, background);
            let path = dir.join(format!(
                "node-glow-reach{:.0}-strength{:.0}.png",
                reach * 100.0,
                strength * 100.0,
            ));
            image::save_buffer(&path, &bytes, SIZE[0], SIZE[1], image::ExtendedColorType::Rgba8)
                .expect("write the png");
            eprintln!("{}", path.canonicalize().unwrap_or(path.clone()).display());
        }
    }

    /// What the Overlap bar does BETWEEN two notes, written to
    /// `target/scratch/` as a table and one PNG a position.
    ///
    /// A probe: it asserts nothing about the picture, the verdict being a look
    /// at a shape a number cannot settle. The bar's two ends are pinned by
    /// tests (`glow_union.rs` in harmonigraph-render): one note is the same
    /// frame at every position, and two equal notes meeting read `2^(1/p)`
    /// times one. What no ratio at one pixel can say is whether the light
    /// BETWEEN them still reads as one field — the union's ridge runs along the
    /// bisector, and as the exponent climbs that ridge narrows toward the
    /// crease a plain max leaves, which is the operator #443 and #520 ruled
    /// out. This is the measurement that removed Feather and Meld in #520,
    /// taken again on the operator that replaced them.
    ///
    /// The three readings per position, all off the same three frames — the
    /// pair lit, and each of the two lit alone:
    ///
    /// - the ridge HEIGHT at the midpoint, the light there over the larger of
    ///   the two singles at the same place, which is `2^(1/p)` if the operator
    ///   is doing what it says;
    /// - the ridge WIDTH, how far either side of the bisector the excess over
    ///   those singles keeps half its midpoint value — the number the bar is
    ///   actually judged on, since a height that survives on a band a few
    ///   pixels wide is a crease with a highlight on it;
    /// - the whole PROFILE from one node through the midpoint to the other, as
    ///   numbers, so the shape between the two can be read rather than inferred
    ///   from two of its points.
    ///
    /// The reading conditions. Yan's live Reach of 2.88 with the fresh Strength
    /// and falloff, which is the light the bar will be dialled against; the
    /// light's own clock and the note Fade off, so one frame is the whole
    /// picture; and three things taken away that would answer for light on a
    /// bare pixel — the Shadow, the resting markers and the labels. The ground
    /// is BLACK for the same reason, so a pixel is the light and nothing else,
    /// and the PNGs are therefore a picture of the light alone.
    ///
    /// Each sample is the mean over a short run ALONG the bisector's direction
    /// (`BAND` either side). The excess being measured is a couple of percent
    /// of a light that is itself dim, and an 8-bit channel is worth half a
    /// level; the run costs almost nothing because the two coverages are equal
    /// all along the bisector, so the quantity being averaged is nearly
    /// constant over it.
    ///
    /// ```text
    /// cargo test -p harmonigraph-offline -- --ignored --nocapture overlap_bar
    /// ```
    #[test]
    #[ignore = "a probe: writes PNGs and prints a table"]
    fn what_the_overlap_bar_does_between_two_notes() {
        use harmonigraph_ui::{draw_pane, Layout, SharedState};

        const SIZE: [u32; 2] = [1200, 1000];
        const PPP: f32 = 2.0;
        const NOW: f64 = 1.0;
        // Yan's live Reach. The Strength and the falloff come from the fresh
        // view, which is what the `the_live_view` golden carries for both.
        const REACH: f32 = 2.88;
        // Pixels either side of a sample, along the bisector, that one reading
        // is the mean of.
        const BAND: i32 = 4;
        // A fifth: one step along the lattice's first axis, so the two nodes
        // are neighbours.
        const NOTES: [u8; 2] = [60, 67];
        // How many points the profile from one node to the other is read at.
        const PROFILE: usize = 41;
        // How far along the bisector, in lattice steps, the width is read — far
        // enough that the walk across the ridge never touches a node's rings.
        const CLEAR: f32 = 0.75;

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
        let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../target/scratch");
        std::fs::create_dir_all(&dir).expect("a scratch directory");

        let fresh = harmonigraph_scene::ViewConfig::default();
        let state_at = |notes: &[u8], p: f32| -> SharedState {
            let mut state = SharedState::new(FORMAT);
            // Black, so a pixel IS the light: every reading below is one frame
            // against another and a ground would stand in both of them.
            state.set_background((0, 0, 0));
            state.frame_params.fade_time = 0.0;
            state.view.glow_attack = 0.0;
            state.view.glow_release = 0.0;
            state.view.glow_reach = REACH;
            state.view.glow_strength = fresh.glow_strength;
            state.view.glow_curve = fresh.glow_curve;
            state.view.glow_union = p;
            // The three things that would put something other than light on a
            // bare pixel between two nodes.
            state.view.plus_arm = 0.0;
            state.view.show_labels = false;
            for style in state.view.shadow.groups_mut() {
                style.depth = 0.0;
            }
            state.camera.zoom_by(2.0);
            for note in notes {
                state.tracker.handle_event(harmonigraph_core::NoteEvent::on(0.0, 0, *note, 1.0));
            }
            state
        };
        let mut shoot = |state: &mut SharedState| -> Vec<u8> {
            let output = context.run_ui(
                egui::RawInput {
                    screen_rect: Some(screen),
                    time: Some(NOW),
                    max_texture_side: Some(renderer.max_texture_side()),
                    ..Default::default()
                },
                |ui| {
                    for (surface, (pane, rect)) in placements.iter().enumerate() {
                        let mut child = ui.new_child(egui::UiBuilder::new().max_rect(*rect));
                        draw_pane(&mut child, *pane, state, NOW, surface);
                    }
                },
            );
            let primitives = context.tessellate(output.shapes, PPP);
            renderer.render(&primitives, &output.textures_delta, PPP, egui::Color32::BLACK)
        };

        // Where the two nodes land, off the scene the pane derives for itself:
        // the preset is the lattice alone, so the pane's viewport is the whole
        // frame and a projection into it is a projection into these pixels.
        let state = state_at(&NOTES, 8.0);
        let window = state.view.scrolled(&state.camera, points.x / points.y);
        let scene = harmonigraph_scene::derive_scene(
            &state.tracker,
            &state.tuning,
            &state.view,
            &window,
            &state.frame_params,
            state.camera,
            None,
            NOW,
        );
        let viewport = glam::vec2(SIZE[0] as f32, SIZE[1] as f32);
        let lit: Vec<_> = scene
            .nodes
            .iter()
            .filter(|node| node.glow.level > 0.0)
            .filter_map(|node| {
                scene.project(viewport, node.world_pos).map(|at| (node.lattice_pos, at))
            })
            .collect();
        // The PAIR by name, out of the many nodes the two notes light: 12-TET
        // spells one pitch class at every cell that reduces to it, so a note is
        // a whole grid of lit nodes (period 3 along the fives, 4 along the
        // threes) rather than one. The two wanted are the home cell and its
        // neighbour one step up the threes — a fifth, which is what the notes
        // are.
        let at_pos = |pos: harmonigraph_core::LatticePos| -> glam::Vec2 {
            lit.iter()
                .find(|(p, _)| *p == pos)
                .unwrap_or_else(|| panic!("{pos:?} is not lit; the notes or the window moved"))
                .1
        };
        let home = harmonigraph_core::LatticePos::new(0, 0, 0);
        let up = harmonigraph_core::LatticePos::new(1, 0, 0);
        let (a, b) = (at_pos(home), at_pos(up));
        let axis = (b - a).normalize();
        let across = glam::vec2(-axis.y, axis.x);
        let mid = (a + b) * 0.5;
        let step = a.distance(b);
        // How far a halo carries, in these pixels: the node's outermost edge
        // plus the whole Reach, in node uv, through the uv-to-pixel scale this
        // camera gives (`marker_unit` is one node uv in world).
        let per_uv = scene
            .project(viewport, glam::Vec3::new(scene.marker_unit, 0.0, 0.0))
            .expect("the uv scale projects")
            .distance(scene.project(viewport, glam::Vec3::ZERO).expect("the origin projects"));
        let carries = (0.795 + REACH) * per_uv;
        eprintln!(
            "node A {a:?}, node B {b:?}, one lattice step {step:.1} px, one node uv {per_uv:.1} \
             px, a halo carries {carries:.1} px"
        );
        // Nothing ELSE lights the ground being read. Every sample below is
        // within one step of the midpoint, so a third node farther off than its
        // own halo can carry, plus that step, cannot reach any of them — and
        // the union is not a sum, so a third contributor would not simply add.
        for (pos, at) in &lit {
            if *pos == home || *pos == up {
                continue;
            }
            assert!(
                at.distance(mid) > carries + step,
                "{pos:?} is {:.1} px from the pair's midpoint and its halo carries {carries:.1}; \
                 the reading would be of three nodes",
                at.distance(mid),
            );
        }

        // One reading: the mean of `2 * BAND + 1` pixels along the bisector,
        // summed over rgb so a level of quantisation is worth a third of what
        // it is on one channel.
        let read = |shot: &[u8], at: glam::Vec2| -> f64 {
            let mut sum = 0.0;
            for k in -BAND..=BAND {
                let p = at + across * k as f32;
                let (x, y) = (p.x.round() as i64, p.y.round() as i64);
                if x < 0 || y < 0 || x >= SIZE[0] as i64 || y >= SIZE[1] as i64 {
                    continue;
                }
                let px = ((y as usize) * SIZE[0] as usize + x as usize) * 4;
                sum += f64::from(shot[px]) + f64::from(shot[px + 1]) + f64::from(shot[px + 2]);
            }
            sum / f64::from(2 * BAND + 1)
        };

        // Where the ridge's WIDTH is read: on the bisector still, but clear of
        // both nodes' ink. The line from one node to the other crosses two
        // rings, and a ring is far brighter than this light — the excess
        // collapses to nothing across it, so a walk outward from the midpoint
        // finds its half-fall at the ring rather than in the light. This point
        // is `CLEAR` steps along the bisector, where the nearest ink is 0.75 of
        // a step away and the light is the only thing in the frame.
        let clear = mid + across * (step * CLEAR);
        eprintln!(
            "\n    t      p   2^(1/p)   ridge at mid   ridge out here   half width px   \
             of one step"
        );
        // Each position's profile, laid out as it will be printed: the table
        // above is a table only if nothing else is written between its rows.
        let mut profiles: Vec<String> = Vec::new();
        for i in 0..5 {
            let t = i as f32 / 4.0;
            // The bar's own travel: geometric in the exponent, 2 at the bottom
            // and 32 at the top (`ValueBar::eased` on a range above zero).
            let p = 2f32.powf(1.0 + 4.0 * t);
            let mut both = state_at(&NOTES, p);
            let mut only_a = state_at(&NOTES[..1], p);
            let mut only_b = state_at(&NOTES[1..], p);
            let (both, only_a, only_b) = (shoot(&mut both), shoot(&mut only_a), shoot(&mut only_b));

            // The profile from one node to the other, and with it the excess
            // the ridge is made of: what the pair puts down over what the
            // brighter of the two singles does at the same pixel.
            let profile: Vec<(f32, f64, f64)> = (0..PROFILE)
                .map(|k| {
                    let u = k as f32 / (PROFILE - 1) as f32;
                    let at = a + (b - a) * u;
                    let single = read(&only_a, at).max(read(&only_b, at));
                    (u, read(&both, at), single)
                })
                .collect();
            let (_, peak, peak_single) = profile[PROFILE / 2];
            let height = peak / peak_single;
            // The same gain read again where the width is measured. Every point
            // of the bisector is equally far from both nodes, so the two
            // coverages are equal all along it and the union's gain is the same
            // `2^(1/p)` at either — two readings of one claim, at two distances
            // from the pair.
            let out = read(&both, clear) / read(&only_a, clear).max(read(&only_b, clear));
            // How far off the midpoint the excess keeps half of what it has
            // there, walked out along the axis a pixel at a time and
            // interpolated where it crosses. Either side, and reported as the
            // mean of the two: the pair is symmetric, so a gap between the
            // halves is the fixture being off-centre rather than the operator.
            let excess_at = |offset: f32| -> f64 {
                let at = clear + axis * offset;
                read(&both, at) - read(&only_a, at).max(read(&only_b, at))
            };
            let top = excess_at(0.0);
            let half = |sign: f32| -> f32 {
                let mut last = top;
                let mut d = 0.0;
                while d < step * 0.5 {
                    let next = excess_at(sign * (d + 1.0));
                    if next < top * 0.5 {
                        let span = (last - next).max(1e-9);
                        return d + ((last - top * 0.5) / span) as f32;
                    }
                    last = next;
                    d += 1.0;
                }
                f32::NAN
            };
            let width = (half(1.0) + half(-1.0)) * 0.5;
            eprintln!(
                "{t:5.2} {p:6.1}    {:.4}         {height:.4}           {out:.4}        \
                 {width:8.1}        {:.3}",
                2f32.powf(1.0 / p),
                width / step,
            );
            let mut block =
                format!("\nprofile at t {t:.2}, p {p:.1}:   u    both   single   excess");
            for (u, both, single) in profile {
                block += &format!(
                    "\n                            {u:.3} {both:7.1} {single:7.1} {:7.1}",
                    both - single
                );
            }
            profiles.push(block);

            let path = dir.join(format!("overlap-t{:.0}-p{p:.0}.png", t * 100.0));
            image::save_buffer(&path, &both, SIZE[0], SIZE[1], image::ExtendedColorType::Rgba8)
                .expect("write the png");
            eprintln!("{}", path.canonicalize().unwrap_or(path.clone()).display());
        }

        // The profiles last, so the table above reads as a table. `u` walks
        // from node A at 0 to node B at 1, and both columns are sums over rgb,
        // so 765 is a white pixel. The two spikes in every one of them are the
        // rings the line crosses on its way between the nodes, which is why the
        // width is read off `clear` and not off here.
        for block in profiles {
            eprintln!("{block}");
        }
    }

    /// The SHADOW against the OCTAVE GAP, written to `target/scratch/`: what a
    /// node casts between its slices as the angular padding widens.
    ///
    /// A probe: it asserts nothing, the verdict being a look. A node's shadow
    /// is a blur of its own ink, so a wide gap is a gap in the shadow too, and
    /// what a number cannot say is whether a node with the field coming through
    /// it still reads as one object rather than as a ring of unrelated marks.
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
    /// one look: shoot under one tag, change what the node casts from, shoot
    /// under another.
    ///
    /// ```text
    /// cargo test -p harmonigraph-offline -- --ignored --nocapture the_shadow_against
    /// ```
    #[test]
    #[ignore = "a probe: writes PNGs and asserts nothing"]
    fn the_shadow_against_the_octave_gap() {
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
        let background =
            egui::Color32::from_rgb(layout.background.0, layout.background.1, layout.background.2);
        let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../target/scratch");
        std::fs::create_dir_all(&dir).expect("a scratch directory");
        let tag = std::env::var("PROBE_TAG").unwrap_or_else(|_| "after".to_string());

        let home = state.camera;
        for gap in [0.05f32, 0.1, 0.2] {
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
                    for (surface, (pane, rect)) in placements.iter().enumerate() {
                        let mut child = ui.new_child(egui::UiBuilder::new().max_rect(*rect));
                        draw_pane(&mut child, *pane, &mut state, NOW, surface);
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
        let background =
            egui::Color32::from_rgb(layout.background.0, layout.background.1, layout.background.2);
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
                    for (surface, (pane, rect)) in placements.iter().enumerate() {
                        let mut child = ui.new_child(egui::UiBuilder::new().max_rect(*rect));
                        draw_pane(&mut child, *pane, &mut state, NOW, surface);
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

    /// The three shadows in one picture, written to `target/scratch/`: a
    /// node's rings, a marker's cross and a name's type, all casting into the
    /// same frame off the same Shadow bar.
    ///
    /// A probe: it asserts nothing, the verdict being whether the three read as
    /// one shadow, and it is kept and `#[ignore]`d for the reason the field's
    /// own probe above is — the reading conditions are the expensive part.
    ///
    /// Those conditions: a chord HELD, because the light is what a shadow is
    /// most visible on and an unlit lattice has none; `NoteNames::Played`, which is
    /// the one setting that puts names and crosses in the same frame, the
    /// played positions taking type and the rest keeping their marks; and the
    /// camera pulled back far enough that a node's shadow reaches its
    /// neighbours, which is where two shadows can be compared at all.
    ///
    /// The sweep is the Shadow WIDTH, since that is the bar the three are meant
    /// to share: off, fresh, and wide enough that the pools meet.
    ///
    /// `PROBE_TAG` names the shots, so a before and an after can be shot from
    /// two builds and laid side by side.
    ///
    /// ```text
    /// cargo test -p harmonigraph-offline -- --ignored --nocapture lattice_shadows
    /// ```
    #[test]
    #[ignore = "a probe: writes PNGs and asserts nothing"]
    fn the_lattice_shadows_draw_a_picture() {
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
        let background =
            egui::Color32::from_rgb(layout.background.0, layout.background.1, layout.background.2);
        let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../target/scratch");
        std::fs::create_dir_all(&dir).expect("a scratch directory");
        let tag = std::env::var("PROBE_TAG").unwrap_or_else(|_| "after".to_string());

        let fresh = harmonigraph_scene::ShadowStyle::default();
        for shadow in [0.0f32, fresh.width, 0.45, harmonigraph_scene::GLOW_SHADOW_MAX] {
            let mut state = SharedState::new(FORMAT);
            state.view.show_labels = true;
            state.view.note_names = harmonigraph_scene::NoteNames::Played;
            state.set_background((24, 25, 29));
            state.frame_params.fade_time = 0.0;
            for note in [55u8, 60, 64, 67, 71] {
                state.tracker.handle_event(harmonigraph_core::NoteEvent::on(0.0, 0, note, 1.0));
            }
            state.camera.zoom_by(2.0);
            for style in state.view.shadow.groups_mut() {
                style.width = shadow;
            }
            let output = context.run_ui(
                egui::RawInput {
                    screen_rect: Some(screen),
                    time: Some(NOW),
                    max_texture_side: Some(renderer.max_texture_side()),
                    ..Default::default()
                },
                |ui| {
                    for (surface, (pane, rect)) in placements.iter().enumerate() {
                        let mut child = ui.new_child(egui::UiBuilder::new().max_rect(*rect));
                        draw_pane(&mut child, *pane, &mut state, NOW, surface);
                    }
                },
            );
            let primitives = context.tessellate(output.shapes, PPP);
            let bytes = renderer.render(&primitives, &output.textures_delta, PPP, background);
            let path = dir.join(format!("shadows-{:.0}-{tag}.png", shadow * 100.0));
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
        let background =
            egui::Color32::from_rgb(layout.background.0, layout.background.1, layout.background.2);

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
            // A compact falloff keeps the ground below saturation, leaving the
            // missing light under the node measurable at this wide reach.
            state.view.glow_curve.shape = 2.75;
            // No names: a played node's name stands on its middle and casts
            // its own shadow there (`fs_shadow_box` in harmonigraph-render),
            // which is a claim of its own and not the light under the body.
            state.view.show_labels = false;
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
                    for (surface, (pane, rect)) in placements.iter().enumerate() {
                        let mut child = ui.new_child(egui::UiBuilder::new().max_rect(*rect));
                        draw_pane(&mut child, *pane, &mut state, NOW, surface);
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
        let background =
            egui::Color32::from_rgb(layout.background.0, layout.background.1, layout.background.2);
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
                    for (surface, (pane, rect)) in placements.iter().enumerate() {
                        let mut child = ui.new_child(egui::UiBuilder::new().max_rect(*rect));
                        draw_pane(&mut child, *pane, &mut state, NOW, surface);
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

    /// **A released note letting go of its light**, written to
    /// `target/scratch/`: one frame per moment of one note's release, across
    /// the Shadow depth.
    ///
    /// A probe: it asserts nothing, the verdict being a look. What it is for is
    /// the one thing the stills beside it cannot show — a picture that is right
    /// while a note is held and wrong on the way out. The two clocks are why
    /// there is such a picture at all: a node's layers run on the note Fade and
    /// its light runs on the Glow release (`panes::glow_fade` in
    /// harmonigraph-ui), so every release has a stretch of full halo over
    /// fading ink, and whatever the two do to each other happens there and
    /// nowhere else.
    ///
    /// The DEPTH is what is swept, because it is what the stretch is worst at:
    /// it decides how much of the frame a caster's ink is still worth
    /// (`shadow_through` in lattice.wgsl), and the ink is the half that is
    /// leaving. Shot at the fresh setting first as the reference, then at its
    /// top.
    ///
    /// Reading conditions as [`the_node_glow_draws_a_picture`] sets them, with
    /// the two clocks left RUNNING rather than switched off, which is the whole
    /// subject here.
    ///
    /// ```text
    /// cargo test -p harmonigraph-offline -- --ignored --nocapture released_note
    /// ```
    #[test]
    #[ignore = "a probe: writes PNGs and asserts nothing"]
    fn a_released_note_lets_go_of_its_light() {
        use harmonigraph_ui::{draw_pane, Layout, SharedState};

        const SIZE: [u32; 2] = [900, 900];
        const PPP: f32 = 2.0;
        const STEP: f64 = 1.0 / 30.0;
        // Long enough that the note is settled at full before it is let go: the
        // Fade's attack runs the same second its release does, and a shot of a
        // note still arriving says nothing about one leaving.
        const OFF: f64 = 1.5;

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
        let background =
            egui::Color32::from_rgb(layout.background.0, layout.background.1, layout.background.2);
        let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../target/scratch");
        std::fs::create_dir_all(&dir).expect("a scratch directory");

        // (tag, depth). The first is the fresh setting, as the reference.
        let shots: Vec<(&str, f32)> = vec![("fresh", 0.85), ("deep", 1.0)];
        // Moments of the release, and one of the hold ahead of it as the
        // reference.
        let want: Vec<f64> =
            [-0.05, 0.1, 0.25, 0.4, 0.6, 0.9, 1.5, 2.5].iter().map(|t| OFF + t).collect();

        for (tag, depth) in shots {
            let mut state = SharedState::new(FORMAT);
            state.set_background((24, 25, 29));
            state.view.glow_reach = 1.5;
            state.view.glow_strength = 1.0;
            for style in state.view.shadow.groups_mut() {
                style.width = 0.16;
                style.depth = depth;
            }
            state.camera.zoom_by(2.0);
            state.tracker.handle_event(harmonigraph_core::NoteEvent::on(0.0, 0, 60, 1.0));
            let mut released = false;
            let mut now = 0.0f64;
            let mut shot = 0usize;
            while shot < want.len() {
                if !released && now >= OFF {
                    state.tracker.handle_event(harmonigraph_core::NoteEvent::off(now, 0, 60));
                    released = true;
                }
                let output = context.run_ui(
                    egui::RawInput {
                        screen_rect: Some(screen),
                        time: Some(now),
                        max_texture_side: Some(renderer.max_texture_side()),
                        ..Default::default()
                    },
                    |ui| {
                        for (surface, (pane, rect)) in placements.iter().enumerate() {
                            let mut child = ui.new_child(egui::UiBuilder::new().max_rect(*rect));
                            draw_pane(&mut child, *pane, &mut state, now, surface);
                        }
                    },
                );
                let primitives = context.tessellate(output.shapes, PPP);
                let bytes = renderer.render(&primitives, &output.textures_delta, PPP, background);
                if now + STEP * 0.5 >= want[shot] {
                    let path = dir.join(format!("release-{tag}-t{:03.0}.png", (now - OFF) * 100.0));
                    image::save_buffer(
                        &path,
                        &bytes,
                        SIZE[0],
                        SIZE[1],
                        image::ExtendedColorType::Rgba8,
                    )
                    .expect("write the png");
                    eprintln!("{}", path.canonicalize().unwrap_or(path.clone()).display());
                    shot += 1;
                }
                now += STEP;
            }
        }
    }

    /// The marker field under a tilted camera, written to `target/scratch/` —
    /// the picture the draw order is for.
    ///
    /// A probe: it asserts nothing, the verdict being a look. What it is a look
    /// AT is which crosses a node's disc covers. Nodes and markers are both
    /// camera-facing billboards at a fixed world size while the sheet they
    /// stand on foreshortens, so face-on a node's disc reaches about its own
    /// cell and the only cross under it is its own — tilt the sheet and one
    /// disc spans a dozen positions, most of them NEARER the eye than the node.
    ///
    /// Five cameras, and the first two are the control: cabinet and a gentle
    /// perspective, where the whole field could be drawn under the whole sheet
    /// and no pixel would know. The other three are where the order shows.
    ///
    /// The names are OFF, a marker being what a name replaces
    /// (`derive_pluses`): with them on, every position this is about is named
    /// and ships no cross at all. The ground is the skin's panel rather than
    /// the preset's near-black, which is what the field is actually read
    /// against.
    ///
    /// ```text
    /// cargo test -p harmonigraph-offline -- --ignored --nocapture marker_depth
    /// ```
    #[test]
    #[ignore = "a probe: writes PNGs and asserts nothing"]
    fn the_marker_depth_order_draws_a_picture() {
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
        let background =
            egui::Color32::from_rgb(layout.background.0, layout.background.1, layout.background.2);
        let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../target/scratch");
        std::fs::create_dir_all(&dir).expect("a scratch directory");

        for (tag, pitch, yaw) in [
            ("cabinet", 0.3f32, 0.4f32),
            ("flat", 0.3, 0.4),
            ("tilt", 1.15, 0.4),
            ("edge", 1.42, 0.4),
            ("yaw", 0.3, 1.42),
        ] {
            let mut state = SharedState::new(FORMAT);
            state.view.show_labels = false;
            state.set_background((24, 25, 29));
            state.frame_params.fade_time = 0.0;
            for note in [55u8, 60, 64, 67, 71] {
                state.tracker.handle_event(harmonigraph_core::NoteEvent::on(0.0, 0, note, 1.0));
            }
            state.camera.projection = if tag == "cabinet" {
                harmonigraph_scene::Projection::Cabinet
            } else {
                harmonigraph_scene::Projection::Perspective
            };
            state.camera.pitch = pitch;
            state.camera.yaw = yaw;
            state.camera.zoom_by(1.6);
            let output = context.run_ui(
                egui::RawInput {
                    screen_rect: Some(screen),
                    time: Some(NOW),
                    max_texture_side: Some(renderer.max_texture_side()),
                    ..Default::default()
                },
                |ui| {
                    for (surface, (pane, rect)) in placements.iter().enumerate() {
                        let mut child = ui.new_child(egui::UiBuilder::new().max_rect(*rect));
                        draw_pane(&mut child, *pane, &mut state, NOW, surface);
                    }
                },
            );
            let primitives = context.tessellate(output.shapes, PPP);
            let bytes = renderer.render(&primitives, &output.textures_delta, PPP, background);
            let path = dir.join(format!("marker-depth-{tag}.png"));
            image::save_buffer(&path, &bytes, SIZE[0], SIZE[1], image::ExtendedColorType::Rgba8)
                .expect("write the png");
            eprintln!("{}", path.canonicalize().unwrap_or(path.clone()).display());
        }
    }
}
