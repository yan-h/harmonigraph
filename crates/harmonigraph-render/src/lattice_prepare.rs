//! GPU preparation and ordered pass encoding for one lattice callback.
//! Resource ownership and allocation live on the parent resource types; these
//! stages borrow them for one frame and never retain another owner.

use super::*;

struct PreparedFrame {
    offscreen_size: Option<[u32; 2]>,
    screen_size: [u32; 2],
    glow: bool,
    lit_nodes: Vec<GpuGlowNode>,
    tile_nodes: Vec<u32>,
    packed: shadow::Packed,
    shadow_wanted: Option<[u32; 2]>,
    blurs: bool,
}

struct FrameSheets {
    has_atlas: bool,
    sizes: [f32; 4],
}

impl CallbackTrait for LatticeCallback {
    fn prepare(
        &self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        screen_descriptor: &ScreenDescriptor,
        egui_encoder: &mut wgpu::CommandEncoder,
        callback_resources: &mut CallbackResources,
    ) -> Vec<wgpu::CommandBuffer> {
        // The shell publishes egui's current font texture under its concrete
        // wgpu type before callbacks prepare. Clone the handle before taking a
        // mutable borrow from the resource map; its pixels remain shared.
        let shared_atlas = callback_resources.get::<wgpu::Texture>().cloned();
        let shared_sdf =
            text::shared_sdf_texture(device, queue, callback_resources, self.sdf.as_ref());
        // Lazily (re)create shared resources. Recreate if the target format
        // changed (it can't today, but this keeps the invariant explicit).
        let recreate = callback_resources
            .get::<LatticeResources>()
            .is_none_or(|r| r.compiled.target_format != self.target_format);
        if recreate {
            // The plugin publishes the instance that owns this device. Other
            // shells keep their existing window-owned resource lifetime.
            let cached =
                self.pipeline_cache.as_ref().zip(callback_resources.get::<wgpu::Instance>());
            let resources = cached.map_or_else(
                || LatticeResources::new(device, queue, self.target_format),
                |(cache, instance)| cache.resources(instance, device, queue, self.target_format),
            );
            callback_resources.insert(resources);
        }
        let resources: &mut LatticeResources =
            callback_resources.get_mut().expect("inserted above when missing");

        let prepare_start = std::time::Instant::now();
        let poll_ms = self.poll_gpu_timer(device, resources);

        #[cfg(feature = "hot-reload")]
        Self::reload_shaders(device, resources);

        // The labels' sheets, before anything is encoded: the glyphs are drawn
        // inside the scene pass below, so the textures they read have to be the
        // current ones by the time that pass is recorded.
        let write_start = std::time::Instant::now();
        resources.bind_sheets(
            device,
            queue,
            shared_atlas.as_ref(),
            self.atlas.as_ref(),
            self.marks.as_ref(),
            shared_sdf.key,
        );
        // The first frames of a session can arrive before any pane has drawn a
        // glyph, and the labels wait for a font texture. Gated on the FONT
        // atlas alone: a mark is always drawn beside a letter, so a frame with
        // a mark in it is a frame with type in it.
        let has_atlas = !resources.atlas.is_empty();
        let sheet_sizes = resources.sheet_sizes();

        let frame = self.prepare_targets(device, screen_descriptor);
        let pane = resources.pane_buffers(
            device,
            self.pane_id,
            frame.offscreen_size,
            frame.screen_size,
            PaneTargets {
                bloom: self.uniforms.composite.bloom_strength > 0.0,
                glow: frame.glow,
                shadow: frame.shadow_wanted,
                blurs: frame.blurs,
                // The strip's height is the row map's CAPACITY, which the
                // light's own clock hands out and which has nothing to do with
                // how many nodes this frame draws (`Scene::glow_rows`).
                rows: self.uniforms.glow.row_capacity as u32,
                // At least one: an empty storage binding is a validation error,
                // and a frame that packed nothing still binds the array for the
                // pipeline's layout — one zeroed entry, which is a caster with
                // no cells and a multiply of 1.
                casters: frame.packed.casters.len().max(1),
                // And at least one lit node, on the same rule: a frame with the
                // light on and nothing lit still binds the list, and the pass
                // that would read it is skipped.
                glow_nodes: frame.lit_nodes.len().max(1),
                glow_tiles: frame.tile_nodes.len().max(1),
            },
            shared_sdf.texture.as_ref(),
        );

        self.upload_frame(
            device,
            queue,
            screen_descriptor,
            pane,
            &frame,
            FrameSheets { has_atlas, sizes: sheet_sizes },
        );
        let write_ms = write_start.elapsed().as_secs_f32() * 1000.0;

        let scene_start = std::time::Instant::now();
        self.encode_frame(resources, egui_encoder, &frame);

        let scene_ms = scene_start.elapsed().as_secs_f32() * 1000.0;

        if let Some(stats) = &self.stats {
            use std::sync::atomic::Ordering::Relaxed;
            let prepare_ms = prepare_start.elapsed().as_secs_f32() * 1000.0;
            stats.prepare_ms.store(prepare_ms.to_bits(), Relaxed);
            stats.poll_ms.store(poll_ms.to_bits(), Relaxed);
            stats.write_ms.store(write_ms.to_bits(), Relaxed);
            stats.scene_ms.store(scene_ms.to_bits(), Relaxed);
        }

        Vec::new()
    }

    fn paint(
        &self,
        _info: egui::PaintCallbackInfo,
        render_pass: &mut wgpu::RenderPass<'static>,
        callback_resources: &CallbackResources,
    ) {
        let Some(resources) = callback_resources.get::<LatticeResources>() else {
            return;
        };
        let Some(pane) = resources.panes.get(&self.pane_id) else {
            return;
        };
        // Nothing was rendered into the offscreen target. The markers and the
        // labels count as much as the nodes here — see `prepare`, where the
        // same test decides whether the target exists at all.
        if pane.instance_count == 0 && pane.plus_count == 0 && pane.glyph_count == 0 {
            return;
        }
        let Some(offscreen) = &pane.offscreen else {
            return;
        };

        // The scene was rendered in prepare(); stretch it over the
        // viewport (egui-wgpu sets the viewport to the callback rect).
        render_pass.set_pipeline(&resources.compiled.composite_pipeline);
        render_pass.set_bind_group(0, &offscreen.composite_bind_group, &[]);
        render_pass.draw(0..4, 0..1);
    }
}

impl LatticeCallback {
    fn poll_gpu_timer(&self, device: &wgpu::Device, resources: &mut LatticeResources) -> f32 {
        // Advance the GPU timer's readback cycle first: a result that landed
        // is published now, and the cycle returns to Idle so this frame can be
        // the next one sampled.
        //
        // ONLY the callback carrying a stats sink touches the timer — see
        // `drives_timer`.
        let poll_start = std::time::Instant::now();
        match (resources.timer.as_mut(), &self.stats) {
            (Some(timer), Some(out)) => {
                if let Some(ms) = timer.poll(device) {
                    out.gpu_ms.store(ms.to_bits(), std::sync::atomic::Ordering::Relaxed);
                }
            }
            // No timer at all: the device refused the feature. Say so, rather
            // than leaving the readout on the same "nothing yet" it shows
            // while a first measurement is still in flight — those are very
            // different answers and the overlay could not tell them apart.
            (None, Some(out)) => {
                out.gpu_ms.store(GPU_TIME_UNSUPPORTED, std::sync::atomic::Ordering::Relaxed);
            }
            (_, None) => {}
        }
        poll_start.elapsed().as_secs_f32() * 1000.0
    }

    #[cfg(feature = "hot-reload")]
    fn reload_shaders(device: &wgpu::Device, resources: &mut LatticeResources) {
        // Dev builds: pick up edits to the .wgsl on disk. A broken edit is
        // rejected with a message; the pipelines in hand keep rendering.
        #[cfg(feature = "hot-reload")]
        if let Some(reloaded) = resources.watcher.poll() {
            // Both modules or neither. They are compiled against one
            // common.wgsl, so an edit in there is due to both, and committing
            // whichever half happened to compile would leave a name's shadow on
            // one build of `shadow_transmittance` and a node's on another —
            // which is the split this reload exists to close, not to make.
            let checked = validate_wgsl(
                "lattice.wgsl",
                &reloaded.lattice,
                reloaded.seam,
                LATTICE_ENTRY_POINTS,
            )
            .and_then(|()| {
                validate_wgsl("text.wgsl", &reloaded.text, reloaded.seam, text::TEXT_ENTRY_POINTS)
            })
            .and_then(|()| {
                validate_wgsl(
                    "roll.wgsl",
                    &module_source(&reloaded.common, roll::ROLL_SRC),
                    reloaded.seam,
                    roll::ROLL_ENTRY_POINTS,
                )
            })
            .and_then(|()| {
                validate_wgsl(
                    "dot_shadow.wgsl",
                    &module_source(&reloaded.common, dot_shadow::SRC),
                    reloaded.seam,
                    dot_shadow::ENTRY_POINTS,
                )
            });
            match checked {
                Ok(()) => {
                    let source = &reloaded.lattice;
                    let lattice_shader = lattice_module(device, source);
                    let blit_shader = blit_module(device);
                    // ...and the two draws that fill the atlas the pair above
                    // reads: a node's shadow is a blur of the same ink an edit
                    // just changed, so the cell has to be rasterized by the
                    // same build that draws the node.
                    let (node_cell_pipeline, plus_cell_pipeline) = create_cell_pipelines(
                        device,
                        &lattice_shader,
                        &resources.compiled.bind_group_layout,
                    );
                    // The glow off the same source, so an edit to a node's
                    // layers reaches the light around it in the same reload —
                    // they are one shader drawing one node, and reloading half
                    // of it is a halo of the previous build.
                    let glow_gather_pipeline = create_glow_gather_pipeline(
                        device,
                        &lattice_shader,
                        LATTICE_COLOR_FORMAT,
                        &resources.compiled.bind_group_layout,
                        &resources.compiled.strip_layout,
                        &resources.compiled.glow_node_layout,
                    );
                    // ...and the strip the light is coloured out of, on the
                    // same argument one step further back: an edit to what a
                    // layer paints is an edit to what the halo is made of.
                    let (ink_strip_pipeline, ink_blur_pipeline) = create_ink_strip_pipelines(
                        device,
                        &lattice_shader,
                        &resources.compiled.bind_group_layout,
                        &resources.compiled.strip_layout,
                    );
                    resources.compiled.node_cell_pipeline = node_cell_pipeline;
                    resources.compiled.plus_cell_pipeline = plus_cell_pipeline;
                    resources.compiled.glow_gather_pipeline = glow_gather_pipeline;
                    resources.compiled.ink_strip_pipeline = ink_strip_pipeline;
                    resources.compiled.ink_blur_pipeline = ink_blur_pipeline;

                    // The NAMES, off the other module the same edit changed.
                    // All three of their pipelines: the fill is the ink
                    // standing in the light, the cell is the ink its shadow is
                    // blurred from, and the box is where that shadow is spent —
                    // one shader drawing one name, on the same argument the
                    // glow's rebuild above is made on.
                    let glyph_shader = text::glyph_shader(device, &reloaded.text);
                    resources.compiled.scenes = create_scene_pipelines(
                        device,
                        &lattice_shader,
                        &blit_shader,
                        &glyph_shader,
                        SceneLayouts {
                            uniforms: &resources.compiled.bind_group_layout,
                            glow: &resources.compiled.filter_layout,
                            shadow: &resources.compiled.shadow_layout,
                            casters: &resources.compiled.caster_layout,
                        },
                        &resources.compiled.glyph_layout,
                    );
                    let (
                        glyph_coverage_cell_pipeline,
                        glyph_distance_cell_pipeline,
                        glyph_distance_pad_pipeline,
                    ) = text::create_glyph_cell_pipelines(
                        device,
                        &glyph_shader,
                        &resources.compiled.glyph_layout,
                    );
                    resources.compiled.glyph_coverage_cell_pipeline = glyph_coverage_cell_pipeline;
                    resources.compiled.glyph_distance_cell_pipeline = glyph_distance_cell_pipeline;
                    resources.compiled.glyph_distance_pad_pipeline = glyph_distance_pad_pipeline;

                    // And the text CALLBACK's own glyph pipelines, in an entry
                    // of the map this one cannot reach: publishing raises the
                    // count they watch, and the next prepare that reads it
                    // rebuilds them against this same source.
                    reload::publish(reloaded.text, reloaded.common);
                    eprintln!("[harmonigraph-render] shaders hot-reloaded");
                }
                Err(err) => {
                    eprintln!("[harmonigraph-render] shader reload REJECTED, keeping old pipelines:\n{err}");
                }
            }
        }
    }

    fn upload_frame(
        &self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        screen_descriptor: &ScreenDescriptor,
        pane: &mut PaneBuffers,
        frame: &PreparedFrame,
        sheets: FrameSheets,
    ) {
        let FrameSheets { has_atlas, sizes: sheet_sizes } = sheets;
        let PreparedFrame { lit_nodes, tile_nodes, packed, .. } = frame;
        if self.instances.len() > pane.instance_capacity {
            pane.instance_capacity = self.instances.len().next_power_of_two();
            pane.instance_buffer = create_vertex_buffer::<GpuInstance>(
                device,
                "lattice_instances",
                pane.instance_capacity,
            );
        }
        pane.instance_count = self.instances.len() as u32;
        if !self.instances.is_empty() {
            // Only an encoded ink pass consumes its clock and row ownership.
            // Layout callbacks discarded by egui never reach this point.
            let encodes_ink = pane.offscreen.as_ref().is_some_and(|o| o.glow.is_some());
            let instances = if encodes_ink {
                let strip = pane.ink_history.as_mut().expect("glow target has pane history");
                strip.parity ^= 1;
                if let Some(timing) = self.glow_timing {
                    pane.ink_instances.clear();
                    pane.ink_instances.extend_from_slice(&self.instances);
                    strip.history.encode(timing, &mut pane.ink_instances, &self.glow_owners);
                    &pane.ink_instances
                } else {
                    strip.history.clear();
                    &self.instances
                }
            } else {
                &self.instances
            };
            queue.write_buffer(&pane.instance_buffer, 0, bytemuck::cast_slice(instances));
        }

        // This frame's lit nodes, into a buffer that may be larger than they
        // are. Nothing zeroes the tail: this frame's tile lists reference only
        // its live nodes, so entries a wider frame left behind are never walked.
        if !lit_nodes.is_empty() {
            queue.write_buffer(&pane.glow_node_buffer, 0, bytemuck::cast_slice(lit_nodes));
            queue.write_buffer(&pane.glow_tile_buffer, 0, bytemuck::cast_slice(tile_nodes));
        }

        if self.pluses.len() > pane.plus_capacity {
            pane.plus_capacity = self.pluses.len().next_power_of_two();
            pane.plus_buffer =
                create_vertex_buffer::<GpuPlus>(device, "lattice_pluses", pane.plus_capacity);
        }
        pane.plus_count = self.pluses.len() as u32;
        if !self.pluses.is_empty() {
            queue.write_buffer(&pane.plus_buffer, 0, bytemuck::cast_slice(&self.pluses));
        }

        // Every caster's whole kernel, for the SCENE draws: one entry a node, a
        // marker and a name all reach by the caster's own index. Written
        // whatever the packing says — a frame that packed nothing writes one
        // zeroed entry, which is a caster with no cells and a multiply of 1.
        let entries: &[shadow::ShadowCaster] =
            if packed.casters.is_empty() { &[shadow::NO_CASTER] } else { &packed.casters };
        queue.write_buffer(&pane.caster_buffer, 0, bytemuck::cast_slice(entries));
        pane.caster_count = packed.casters.len();

        // The casters' boxes, whether or not there is a font sheet to cut a
        // name out of: a node and a marker cast without one.
        if packed.boxes.len() > pane.box_capacity {
            pane.box_capacity = packed.boxes.len().next_power_of_two();
            pane.box_buffer = create_vertex_buffer::<shadow::ShadowBox>(
                device,
                "lattice_shadow_boxes",
                pane.box_capacity,
            );
        }
        pane.box_count = packed.boxes.len() as u32;
        if !packed.boxes.is_empty() {
            queue.write_buffer(&pane.box_buffer, 0, bytemuck::cast_slice(&packed.boxes));
        }
        // Each node instance's own box beside it, for the two draws that bind
        // the pair (`shadow::ShadowBox::BESIDE_NODES`). Written whatever the
        // packing says: a frame that packed nothing hands every instance a box
        // of zeros, which is a caster with no cell and a multiply of 1.
        if pane.instance_count as usize > pane.node_cell_capacity {
            pane.node_cell_capacity = (pane.instance_count as usize).next_power_of_two();
            pane.node_cell_buffer = create_vertex_buffer::<shadow::ShadowBox>(
                device,
                "lattice_node_cells",
                pane.node_cell_capacity,
            );
        }
        if pane.instance_count > 0 {
            let all = &packed.boxes;
            let boxes = self
                .node_cells
                .iter()
                .map(|&i| all.get(i as usize).copied().unwrap_or(shadow::NO_CELL));
            debug_assert_eq!(boxes.len(), self.instances.len(), "one box per node instance");
            write_shadow_boxes(queue, &pane.node_cell_buffer, self.instances.len(), boxes);
        }

        // The labels. With no atlas there is nothing to sample, so the pass
        // draws none of them rather than sampling a texture that isn't there.
        if has_atlas {
            // The glyphs are a VERTEX buffer, so growing it leaves the bind
            // group — uniforms, atlas, sampler — naming everything it named
            // before. Dropping it here would blank this pane's labels for the
            // frame the buffer grows on, since nothing rebuilds one until the
            // next `pane_buffers`.
            if self.glyphs.len() > pane.glyph_capacity {
                pane.glyph_capacity = self.glyphs.len().next_power_of_two();
                pane.glyph_buffer = create_vertex_buffer::<GlyphInstance>(
                    device,
                    "lattice_glyphs",
                    pane.glyph_capacity,
                );
                pane.cell_buffer = create_vertex_buffer::<shadow::ShadowBox>(
                    device,
                    "lattice_shadow_cells",
                    pane.glyph_capacity,
                );
            }
            pane.glyph_count = self.glyphs.len() as u32;
            if !packed.boxes.is_empty() {
                // Each glyph's own name's box beside it, for the cell draw. The
                // runs are contiguous in draw order, so this is the boxes
                // repeated by their runs' lengths.
                let cells = self
                    .draws
                    .iter()
                    .filter_map(|draw| match *draw {
                        Draw::Label(a, b, l) => Some((b - a, packed.boxes[l as usize])),
                        _ => None,
                    })
                    .flat_map(|(n, b)| std::iter::repeat_n(b, n as usize));
                write_shadow_boxes(queue, &pane.cell_buffer, self.glyphs.len(), cells);
            }
            if !self.glyphs.is_empty() {
                queue.write_buffer(&pane.glyph_buffer, 0, bytemuck::cast_slice(&self.glyphs));
                // The glyphs' own points, not the screen's: the rects arrive
                // in this pane's space because the pass they are drawn in is
                // this pane's. `pixels_per_point` stays the DEVICE's — it is
                // what the atlas was rasterized at, and so what turns the
                // rim's radius in points into a texel offset, whatever the
                // render scale does to the target's pixels.
                let atlas_size = pane
                    .offscreen
                    .as_ref()
                    .and_then(|o| o.shadow.as_ref())
                    .map_or([0.0; 2], |s| [s.size[0] as f32, s.size[1] as f32]);
                queue.write_buffer(
                    &pane.glyph_uniform_buffer,
                    0,
                    bytemuck::bytes_of(&text::TextUniforms {
                        screen_points: self.size_points,
                        atlas_size: [sheet_sizes[0], sheet_sizes[1]],
                        mark_atlas_size: [sheet_sizes[2], sheet_sizes[3]],
                        filter_axis: self.slide.unit(),
                        pixels_per_point: screen_descriptor.pixels_per_point.max(f32::EPSILON),
                        // The TEXT group's own depth, where `geometry_shadow` carries
                        // the geometry group's: a name's box is drawn by this
                        // pipeline and a ring's by the lattice's, so the two
                        // depths reach the two shaders through their own
                        // uniforms and never through each other.
                        //
                        // The group's WIDTH is not here: it is σ, and σ is
                        // spent on the CPU where the cells are packed.
                        shadow_depth: self.shadow.lattice_text.depth,
                        // The atlas the cells are drawn into, which may be
                        // larger than this frame's layout (`ensure_shadow`).
                        shadow_atlas_size: atlas_size,
                        node_occlusion: self.uniforms.geometry_shadow.occlusion,
                        _pad: [0.0; 3],
                    }),
                );
            }
        } else {
            pane.glyph_count = 0;
        }

        // The order, held to what actually reached the buffers: every index in
        // a `Draw` addresses one of four lists and the pass draws off them
        // without a second look, so a draw naming something they do not hold is
        // dropped here rather than trusted there. The names go the same way
        // when there is no atlas — nothing to sample.
        pane.draws.clear();
        pane.draws.extend(self.draws.iter().copied().filter(|draw| match *draw {
            Draw::Nodes(a, b) => a < b && b <= pane.instance_count,
            Draw::Pluses(a, b) => a < b && b <= pane.plus_count,
            Draw::Label(a, b, l) => {
                a < b && b <= pane.glyph_count && (l as usize) < self.casters.len()
            }
        }));

        // What the shader is told about the atlas, settled HERE because the
        // atlas is sized here: its texels, for the two draws that fill a cell
        // and so cannot read the texture they are writing, and the markers'
        // shared blur cell, which is `casters[0]` wherever that row casts.
        //
        // The arm in points is the one term `pack` has no way to know, and it
        // is what maps a fragment's place on a cross into a cell no cross
        // placed (`vs_plus`).
        let mut uniforms = self.uniforms;
        // How far into the lit-node list this frame's own entries run, which is
        // settled here for the same reason the atlas's texels are: the map that
        // fills it needs the target's pixels, and this is where they are known.
        uniforms.glow.lit = lit_nodes.len() as f32;
        if let Some(atlas) = pane.offscreen.as_ref().and_then(|o| o.shadow.as_ref()) {
            uniforms.shadow_target.atlas_texels = Float2(atlas.size.map(|v| v as f32));
        }
        if let Some(cell) = packed.boxes.first().filter(|_| self.marker_arm_points > 0.0) {
            uniforms.marker_cell = MarkerCellParams {
                rect: Float4(cell.rect),
                cell: Float4(cell.cell),
                points_to_texels: cell.cell_map[0],
                aa_scale: cell.cell_map[3],
                arm_points: self.marker_arm_points,
                padding: 0.0,
            };
        }
        queue.write_buffer(&pane.uniform_buffer, 0, bytemuck::bytes_of(&uniforms));
    }

    fn prepare_targets(
        &self,
        device: &wgpu::Device,
        screen_descriptor: &ScreenDescriptor,
    ) -> PreparedFrame {
        // Offscreen pixel size: the callback rect at native resolution,
        // scaled by the render-scale view setting (clamped in from_scene).
        // The unscaled screen size drives the bloom chain.
        let max_dim = device.limits().max_texture_dimension_2d;
        let px_size = |scale: f32| {
            let px = screen_descriptor.pixels_per_point * scale;
            [
                ((self.size_points[0] * px).round() as u32).clamp(1, max_dim),
                ((self.size_points[1] * px).round() as u32).clamp(1, max_dim),
            ]
        };
        let size = px_size(self.render_scale);
        let screen_size = px_size(1.0);
        // Nothing to draw (matches paint()'s early-out): skip the offscreen
        // target and pass entirely. The EDGES count as much as the nodes —
        // `from_scene` drops nodes that can paint nothing, and an idle node
        // paints nothing, so a still lattice is exactly a frame of markers and
        // no instances — and keying this on the instances alone would take the
        // markers down with them. So do the LABELS, for the same reason from the
        // other end: a hovered idle node paints nothing and is named, so a
        // lattice can be a frame of one label and nothing else.
        let anything =
            !self.instances.is_empty() || !self.pluses.is_empty() || !self.glyphs.is_empty();
        let offscreen_size = anything.then_some(size);

        let glow = self.glow_draws();
        // Every lit node, mapped onto the pixels of the target the light is
        // gathered into. Here rather than in `from_scene` because that is the
        // one thing the map needs which the callback is not built with: the
        // render-scaled size, settled just above.
        let lit_nodes = if glow { self.glow_nodes(size) } else { Vec::new() };
        let tile_nodes = lattice_node_glow::tiles::pack(&lit_nodes, size);
        // Every caster's cell, packed for this frame (`shadow::pack`): the
        // Gaussian's one marker cross, one per node and one per name, each at
        // the resolution its own σ asks for. A caster whose group has either
        // bar at the bottom arrived with a σ of nothing and takes none, so a
        // frame with every group shut allocates no atlas and every cell reader
        // multiplies by exactly 1.
        //
        // The scale handed over is the TARGET's pixels per pane point — the
        // device's times the render scale, which is the term #496 found missing
        // from the field's reach.
        let ppp = screen_descriptor.pixels_per_point.max(f32::EPSILON);
        let mut packed = shadow::pack(&self.casters, ppp * self.render_scale, max_dim);
        // Every receiver, including a label with its own shadow disabled,
        // starts at the next node caster in painter order. Names immediately
        // follow their owner, so that owner can never occlude its own text.
        // Index + 1 leaves zero as the end, independent of buffer capacity.
        let mut next = 0;
        let mut nodes = self.node_cells.iter().rev().peekable();
        for (i, caster) in packed.casters.iter_mut().enumerate().rev() {
            caster.map[3] = next as f32;
            if nodes.peek().is_some_and(|&&node| node as usize == i) {
                nodes.next();
                if caster.shade[0] > 0.0 {
                    next = i as u32 + 1;
                }
            }
        }
        // A placeholder box preserves the caster index for a distance field
        // evaluated directly by its scene draw. Only a real cell asks for the
        // atlas; a markers-only Distance frame therefore allocates no atlas.
        let has_shadow_cells = packed.boxes.iter().any(|b| b.cell[2] > 0.0 && b.cell[3] > 0.0);
        let shadow_wanted = has_shadow_cells.then_some(packed.size);
        // The blur chain runs, and its intermediate is held, when some cell in
        // the atlas holds COVERAGE — which is what the chain convolves, a
        // distance cell collapsing its own quad inside both passes (`vs_cell`
        // in shadow.wgsl). Read off the boxes rather than off the settings, so
        // a Gaussian group with nothing in the frame to cast costs neither the
        // pass nor the plane.
        let blurs =
            packed.boxes.iter().any(|b| b.cell[2] > 0.0 && b.who[1] < 0.5 * shadow::DISTANCE_KIND);
        PreparedFrame {
            offscreen_size,
            screen_size,
            glow,
            lit_nodes,
            tile_nodes,
            packed,
            shadow_wanted,
            blurs,
        }
    }

    fn encode_shadows(
        &self,
        compiled: &CompiledLatticeResources,
        pane: &PaneBuffers,
        offscreen: &Offscreen,
        egui_encoder: &mut wgpu::CommandEncoder,
        packed: &shadow::Packed,
        blurs: bool,
    ) {
        // The shadow atlas, ahead of the scene pass that samples it: every
        // caster's own ink into its own cell, then the blur over the cells.
        // Present exactly while something casts this frame
        // (`PaneTargets::shadow`), so a frame with the Shadow shut pays
        // none of it and its draws multiply by 1.
        //
        // THREE draws into one cleared target, in the order the buffers sit
        // in rather than the picture's. Their cells are disjoint, so this
        // order decides nothing.
        let atlas = offscreen.shadow.as_ref().filter(|_| pane.box_count > 0);
        if let Some(atlas) = atlas {
            let mut pass = atlas.ink_pass(egui_encoder);
            if let Some(glyphs) = pane.glyph_bind_group.as_ref().filter(|_| pane.glyph_count > 0) {
                pass.set_pipeline(&compiled.glyph_distance_pad_pipeline);
                pass.set_bind_group(0, glyphs, &[]);
                pass.set_vertex_buffer(0, pane.box_buffer.slice(..));
                pass.draw(0..4, 0..pane.box_count);
            }
            if pane.instance_count > 0 {
                pass.set_pipeline(&compiled.node_cell_pipeline);
                pass.set_bind_group(0, &pane.bind_group, &[]);
                pass.set_vertex_buffer(0, pane.instance_buffer.slice(..));
                pass.set_vertex_buffer(1, pane.node_cell_buffer.slice(..));
                pass.draw(0..4, 0..pane.instance_count);
            }
            // ONE cross for the whole field, at the home sheet's size and
            // at level 1: every marker reads this same cell and spends its
            // own opacity where it reads it.
            let marker_has_cells =
                packed.boxes.first().is_some_and(|b| b.cell[2] > 0.0 && b.cell[3] > 0.0);
            if pane.plus_count > 0 && self.marker_arm_points > 0.0 && marker_has_cells {
                pass.set_pipeline(&compiled.plus_cell_pipeline);
                pass.set_bind_group(0, &pane.bind_group, &[]);
                pass.draw(0..4, 0..1);
            }
            if let Some(glyphs) = pane.glyph_bind_group.as_ref().filter(|_| pane.glyph_count > 0) {
                pass.set_bind_group(0, glyphs, &[]);
                pass.set_vertex_buffer(0, pane.glyph_buffer.slice(..));
                pass.set_vertex_buffer(1, pane.cell_buffer.slice(..));
                // Every name in a frame is one group, so the fill is one
                // pipeline for the whole run — where a NODE's fill branches
                // per box, its own cell draw serving both kinds.
                pass.set_pipeline(if self.shadow.lattice_text.kernel.is_distance() {
                    &compiled.glyph_distance_cell_pipeline
                } else {
                    &compiled.glyph_coverage_cell_pipeline
                });
                pass.draw(0..4, 0..pane.glyph_count);
            }
            drop(pass);
            // The same answer the atlas was allocated on, so the chain and
            // the plane it ping-pongs through can never disagree.
            if blurs {
                let cells = &compiled.shadow_cell_pipelines;
                atlas.blur(
                    egui_encoder,
                    (&cells.blur_x, &cells.blur_y),
                    &pane.box_buffer,
                    pane.box_count,
                );
            }
        }
    }

    fn encode_node_glow(
        &self,
        compiled: &CompiledLatticeResources,
        pane: &PaneBuffers,
        offscreen: &Offscreen,
        egui_encoder: &mut wgpu::CommandEncoder,
        has_lit_nodes: bool,
    ) {
        // The node glow, into a target of its own and BEFORE the scene
        // pass, which composites it at its bottom and samples it per node.
        //
        // One draw over the whole instance buffer, every sheet at once:
        // the blend the pass writes is commutative and never subtracts, so
        // there is nothing for a per-sheet walk to decide —
        // what hides a node's halo is the scene pass drawing a nearer node
        // over it.
        //
        // Encoded whenever the target exists, nodes or none: the pass
        // CLEARS it, and a frame that skipped it would composite whatever
        // the last frame left there — light around nodes that are no longer
        // on screen. A lattice can be a frame of markers and labels with
        // every node culled, which is exactly that frame.
        if let Some(glow) = offscreen.glow.as_ref() {
            let strip = pane.ink_history.as_ref().expect("glow target has pane history");
            // What colour that light is, before any of it is laid down: the
            // ink read round every node, then blurred (see [`InkStrip`]).
            // Both are skipped with no instances to read — the light's own
            // draws are too, so nothing samples what they would leave.
            if pane.instance_count > 0 {
                let mut pass = egui_encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                    label: Some("lattice_ink_strip_pass"),
                    color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                        view: strip.writing(),
                        depth_slice: None,
                        resolve_target: None,
                        ops: wgpu::Operations {
                            // Cleared, though what the pass leaves behind
                            // is not this frame's ink alone: every row a
                            // node holds is written whole, and the rows in
                            // between are ones no node has been handed. It
                            // is what a row that has just been handed BACK
                            // needs — the next node to take it seeds off
                            // its own reading rather than off a stranger's
                            // ink two frames old.
                            load: wgpu::LoadOp::Clear(wgpu::Color::TRANSPARENT),
                            store: wgpu::StoreOp::Store,
                        },
                    })],
                    depth_stencil_attachment: None,
                    timestamp_writes: None,
                    occlusion_query_set: None,
                    multiview_mask: None,
                });
                pass.set_bind_group(0, &pane.bind_group, &[]);
                // The strip this same pass wrote last frame: what a node's
                // light is carried FROM (see [`InkStrip`]).
                pass.set_bind_group(1, strip.carried(), &[]);
                pass.set_vertex_buffer(0, pane.instance_buffer.slice(..));
                pass.set_pipeline(&compiled.ink_strip_pipeline);
                pass.draw(0..4, 0..pane.instance_count);
                drop(pass);

                let mut pass = egui_encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                    label: Some("lattice_ink_blur_pass"),
                    color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                        view: &strip.blurred_view,
                        depth_slice: None,
                        resolve_target: None,
                        ops: wgpu::Operations {
                            load: wgpu::LoadOp::Clear(wgpu::Color::TRANSPARENT),
                            store: wgpu::StoreOp::Store,
                        },
                    })],
                    depth_stencil_attachment: None,
                    timestamp_writes: None,
                    occlusion_query_set: None,
                    multiview_mask: None,
                });
                pass.set_bind_group(0, &pane.bind_group, &[]);
                pass.set_bind_group(1, strip.written(), &[]);
                pass.set_vertex_buffer(0, pane.instance_buffer.slice(..));
                pass.set_pipeline(&compiled.ink_blur_pipeline);
                pass.draw(0..4, 0..pane.instance_count);
            }

            // Cleared to transparent, which is what the gather writes where
            // no node reaches — so the clear is the answer for the frames
            // that skip the draw below, and agrees with it everywhere else.
            let mut pass = egui_encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("lattice_glow_pass"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: &glow.view,
                    depth_slice: None,
                    resolve_target: None,
                    ops: wgpu::Operations {
                        load: wgpu::LoadOp::Clear(wgpu::Color::TRANSPARENT),
                        store: wgpu::StoreOp::Store,
                    },
                })],
                depth_stencil_attachment: None,
                timestamp_writes: None,
                occlusion_query_set: None,
                multiview_mask: None,
            });
            // ONE quad over the target, each pixel folding its tile's
            // candidates from group 2. The fold is commutative,
            // so this target is one field of light with no depth in it
            // at all — which is what makes it safe to lay under
            // every sheet as a single layer.
            //
            // Skipped with nothing lit, where the clear above has already
            // written what the pass would: the light is over, or the frame
            // ships only markers and names.
            if has_lit_nodes {
                pass.set_bind_group(0, &pane.bind_group, &[]);
                pass.set_bind_group(1, &strip.blurred_bind_group, &[]);
                pass.set_bind_group(2, &pane.glow_node_bind_group, &[]);
                pass.set_bind_group(3, &pane.glow_tile_bind_group, &[]);
                pass.set_pipeline(&compiled.glow_gather_pipeline);
                pass.draw(0..4, 0..1);
            }
        }
    }

    fn encode_scene(
        &self,
        compiled: &CompiledLatticeResources,
        pane: &PaneBuffers,
        offscreen: &Offscreen,
        egui_encoder: &mut wgpu::CommandEncoder,
    ) {
        let atlas = offscreen.shadow.as_ref().filter(|_| pane.box_count > 0);
        let scene = &compiled.scenes[usize::from(offscreen.bloom.is_some())];
        let attachment = |view| {
            Some(wgpu::RenderPassColorAttachment {
                view,
                depth_slice: None,
                resolve_target: None,
                ops: wgpu::Operations {
                    load: wgpu::LoadOp::Clear(wgpu::Color::TRANSPARENT),
                    store: wgpu::StoreOp::Store,
                },
            })
        };
        let attachments = [
            attachment(&offscreen.color_view),
            attachment(&offscreen.ink_view),
            offscreen.bloom.as_ref().and_then(|b| attachment(&b.nodes_view)),
            offscreen.bloom.as_ref().and_then(|b| attachment(&b.ink_view)),
        ];
        let mut pass = egui_encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
            label: Some("lattice_scene_pass"),
            color_attachments: &attachments[..if offscreen.bloom.is_some() { 4 } else { 2 }],
            depth_stencil_attachment: None,
            timestamp_writes: None,
            occlusion_query_set: None,
            multiview_mask: None,
        });

        // The light, FIRST — under every node, marker and label in the
        // pass, which is what makes a node a lamp rather than a hole and
        // what puts the light under every SHADOW: each item multiplies
        // whatever is already in the frame under it by its own blurred ink
        // (`node_paint`, `plus_paint`, text.wgsl's `fs_shadow_box`), and
        // the light is in the frame first.
        //
        // With bloom on it writes both attachments, so the bloom's
        // bright pass reads the light exactly as it reads the nodes: it is
        // light the nodes emit, and it blooms with the rest of them.
        if let Some(glow) = offscreen.glow.as_ref() {
            pass.set_pipeline(&scene.glow_over);
            pass.set_bind_group(0, &glow.bind_group, &[]);
            pass.draw(0..4, 0..1);
        }

        // That same target at group 1 of every node and marker draw, for
        // the wash to read back. The dummy where the light does not exist
        // at all, which is the Reach bar at 0 — a transparent read is the
        // plain ground, so nothing branches (see `glow_dummy_bind_group`).
        let light =
            offscreen.glow.as_ref().map_or(&compiled.glow_dummy_bind_group, |g| &g.bind_group);
        // The finished atlas at group 2 of every node and marker draw, for
        // each to read its own cell. The 1x1 stand-in where this frame
        // packed none: every box is then zeros, and a caster with no cell
        // multiplies by exactly 1 with nothing sampled (`shadow_through`).
        let cells = atlas.map_or(&compiled.shadow_dummy_bind_group, |a| a.read());

        // The order, as `from_scene` laid it down (see [`Draw`]). One walk
        // forward, back to front, with nothing here deciding what goes
        // where — a name is covered by exactly what covers its node, and a
        // cross covers the nodes behind it and no others, because that is
        // where each of them was emitted.
        for draw in &pane.draws {
            match *draw {
                Draw::Nodes(a, b) => {
                    pass.set_pipeline(&scene.nodes);
                    pass.set_bind_group(0, &pane.bind_group, &[]);
                    pass.set_bind_group(1, light, &[]);
                    pass.set_bind_group(2, cells, &[]);
                    pass.set_bind_group(3, &pane.caster_bind_group, &[]);
                    pass.set_vertex_buffer(0, pane.instance_buffer.slice(..));
                    // One box per node instance: what this draw takes off
                    // a box is the caster's INDEX, which is the only row of
                    // it the scene pass reads.
                    pass.set_vertex_buffer(1, pane.node_cell_buffer.slice(..));
                    pass.draw(0..4, a..b);
                }
                Draw::Pluses(a, b) => {
                    pass.set_pipeline(&scene.pluses);
                    pass.set_bind_group(0, &pane.bind_group, &[]);
                    pass.set_bind_group(1, light, &[]);
                    pass.set_bind_group(2, cells, &[]);
                    pass.set_bind_group(3, &pane.caster_bind_group, &[]);
                    pass.set_vertex_buffer(0, pane.plus_buffer.slice(..));
                    pass.draw(0..4, a..b);
                }
                // Two draws per name: its shadow over everything already
                // in the frame under its box, then the glyphs themselves,
                // washed by the light they stand in. A name paints no rim
                // — what keeps a halo off it is the shadow, which is a
                // multiply on the frame and so lands on the light along
                // with everything else under it (`fs_shadow_box`).
                //
                // The SHADOW FIRST, which is what keeps a name's own ink
                // out of its own shadow: the blend's ink term is not
                // multiplied, so a name's letters are the one thing in the
                // frame its shadow never darkens.
                //
                // The light at group 1 for the fill, the same bind group
                // the nodes and markers above it took; the atlas at group
                // 2 for the box, which reads its own cell and nothing else.
                Draw::Label(a, b, l) => {
                    let Some(bind_group) = pane.glyph_bind_group.as_ref() else {
                        continue;
                    };
                    pass.set_bind_group(0, bind_group, &[]);
                    pass.set_bind_group(1, light, &[]);
                    // One instance at the caster's own index, which is how
                    // the draw finds its shadow: `vs_shadow_box` reads the
                    // quad and the level out of the array at group 3 and
                    // binds no vertex buffer at all.
                    if let Some(atlas) = atlas.filter(|_| (l as usize) < pane.caster_count) {
                        pass.set_bind_group(2, atlas.read(), &[]);
                        pass.set_bind_group(3, &pane.caster_bind_group, &[]);
                        pass.set_pipeline(&scene.shadow_box);
                        pass.draw(0..4, l..l + 1);
                    }
                    pass.set_bind_group(2, cells, &[]);
                    pass.set_bind_group(3, &pane.caster_bind_group, &[]);
                    pass.set_vertex_buffer(0, pane.glyph_buffer.slice(..));
                    pass.set_vertex_buffer(1, pane.cell_buffer.slice(..));
                    pass.set_pipeline(&scene.glyph_fill);
                    pass.draw(0..4, a..b);
                }
            }
        }
        drop(pass);
    }

    fn encode_frame(
        &self,
        resources: &mut LatticeResources,
        egui_encoder: &mut wgpu::CommandEncoder,
        frame: &PreparedFrame,
    ) {
        let PreparedFrame { lit_nodes, packed, blurs, .. } = frame;
        let blurs = *blurs;
        // The scene pass: draw into the pane's offscreen target, on the
        // encoder egui-wgpu executes before its own render pass. paint()
        // then just composites the finished texture.
        let pane = resources.panes.get(&self.pane_id).expect("created by pane_buffers above");
        let draws = pane.instance_count > 0 || pane.plus_count > 0 || pane.glyph_count > 0;
        if let Some(offscreen) = pane.offscreen.as_ref().filter(|_| draws) {
            // Bracket all lattice preparation, starting before the first
            // optional shadow/ink/glow pass and ending after optional bloom.
            // The final egui composite belongs to paint's host pass.
            // Skipped while a readback is still in flight, so the query set is
            // never overwritten mid-cycle.
            let timing =
                self.drives_timer() && resources.timer.as_ref().is_some_and(GpuTimer::arming);
            if timing {
                resources.timer.as_ref().expect("armed timer").opening(egui_encoder);
            }

            self.encode_shadows(&resources.compiled, pane, offscreen, egui_encoder, packed, blurs);

            self.encode_node_glow(
                &resources.compiled,
                pane,
                offscreen,
                egui_encoder,
                !lit_nodes.is_empty(),
            );

            self.encode_scene(&resources.compiled, pane, offscreen, egui_encoder);

            if let Some(bloom) = &offscreen.bloom {
                bloom.chain.run(egui_encoder, Self::bloom_pipelines(resources), "lattice");
            }

            if timing {
                if let Some(timer) = resources.timer.as_mut() {
                    timer.close(egui_encoder);
                }
            }
        } else if let Some(out) = &self.stats {
            // No pass was encoded, so no reading can land and the timer's
            // cycle does not turn over. Say "nothing measured" rather than
            // leaving the last real figure sitting there: `poll` returns None
            // from Idle forever, and the overlay would keep re-averaging a
            // number from whenever the lattice last drew.
            //
            // The distinction is why GPU_TIME_PENDING exists at all — a frozen
            // reading and a live one are the same bits otherwise, and a pane
            // that encodes no pass can sit here indefinitely rather than for a
            // frame. A silent lattice already ships no NODE, every idle one
            // being culled; what it takes to ship no edge either is a window
            // holding no adjacent pair, which the extent bars do not reach
            // (they stop at 1). So the state is currently out of a user's
            // reach and stays guarded on purpose: it is one lattice-sizing
            // change away, and the symptom would be a stale figure rather
            // than a crash — the kind nobody reports.
            out.gpu_ms.store(GPU_TIME_PENDING, std::sync::atomic::Ordering::Relaxed);
        }
    }
}
