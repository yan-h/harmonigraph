//! Scene-to-GPU frame assembly: culling, painter order, labels and casters.

use super::*;

/// Add node instance `at` to the run this list ends with, or start a new one.
///
/// A run is exactly a stretch of instances the walk did not interrupt: the
/// buffers are filled in draw order, so "the last draw ends where this one
/// starts" is the whole test.
fn push_node(draws: &mut Vec<Draw>, at: u32) {
    match draws.last_mut() {
        Some(Draw::Nodes(_, end)) if *end == at => *end = at + 1,
        _ => draws.push(Draw::Nodes(at, at + 1)),
    }
}

/// The same for one marker — see [`push_node`].
fn push_plus(draws: &mut Vec<Draw>, at: u32) {
    match draws.last_mut() {
        Some(Draw::Pluses(_, end)) if *end == at => *end = at + 1,
        _ => draws.push(Draw::Pluses(at, at + 1)),
    }
}

/// The markers no node stands at, drawn where the whole field used to be:
/// over the sheets behind the home one, under the home sheet itself.
///
/// See `from_scene` for what makes a marker loose and why it can only be one
/// the caller built by hand.
fn push_loose(draws: &mut Vec<Draw>, pluses: &mut Vec<GpuPlus>, loose: &[GpuPlus]) {
    for &plus in loose {
        push_plus(draws, pluses.len() as u32);
        pluses.push(plus);
    }
}

impl LatticeCallback {
    pub(super) fn from_scene(
        scene: &Scene,
        labels: LatticeLabels,
        size_points: egui::Vec2,
        target_format: wgpu::TextureFormat,
        pane_id: u64,
        stats: Option<std::sync::Arc<LatticeStats>>,
    ) -> Self {
        let aspect = size_points.x / size_points.y.max(1.0);
        let render_scale = scene.render_scale.clamp(RENDER_SCALE_RANGE.0, RENDER_SCALE_RANGE.1);
        let camera = scene.camera;
        let view_proj = camera.view_proj(aspect);
        let (right, up) = camera.right_up();

        // Sort back-to-front along the view direction: alpha blending relies
        // on painter order, including every node, marker and label shadow.
        //
        // Sheets back to front FIRST, then painter's order within a sheet.
        // That is still just back-to-front — world z IS the sevens axis, and
        // the first key is only its depth — but it stays EXACT when the
        // camera is orbited, where two nodes on one sheet have different
        // depths and a plain depth sort interleaves the sheets. Interleaving
        // is not a cosmetic problem: it puts the markers in the wrong place in
        // the order, and every shadow in the frame is cast in that order — an
        // item multiplies what is already under it, so which of two overlapping
        // items darkens the other is exactly where each stands in this walk.
        //
        // Do not reorder the sheets on top of this. Forcing the home sheet
        // to the bottom (so off-sheet notes could never be hidden by it)
        // inverts the far half of the axis: the sheet BEHIND home then draws
        // last, over the home sheet in front of it. Grouping by distance from
        // home does the same thing more thoroughly. Depth is what the reader is
        // being shown; it is what the order has to follow.
        //
        // The `forward.z` factor is what keeps it honest if the view is
        // orbited right around past the sheets: which way along z is "away"
        // is the camera's business, not an assumption.
        let eye = camera.eye();
        let forward = (camera.target - eye).normalize_or_zero();
        let sheet_depth = |n: &harmonigraph_scene::NodeInstance| n.world_pos.z * forward.z;
        // Carrying the node's INDEX rather than the node itself, because a
        // label names one and has to be put back beside it after the sort.
        let mut order: Vec<(f32, f32, usize)> = scene
            .nodes
            .iter()
            .enumerate()
            .map(|(i, n)| (sheet_depth(n), (n.world_pos - eye).dot(forward), i))
            .collect();
        order.sort_by(|a, b| b.0.total_cmp(&a.0).then(b.1.total_cmp(&a.1)));

        let to_gpu = |n: &harmonigraph_scene::NodeInstance| GpuInstance {
            world_pos: n.world_pos.to_array(),
            color: n.color.to_array(),
            params: [n.activation, n.melody_level, n.bass_level],
            octaves: pack_octaves(&n.octaves),
            cents: n.cents,
            marks: [n.melody_slots, n.bass_slots],
            melody_color: n.melody_color.to_array(),
            bass_color: n.bass_color.to_array(),
            scale: n.scale,
            ring: n.audio_ring,
            glow: [n.glow.level, n.glow.row as f32, n.glow.mix, n.glow.marked],
        };

        let split = order.iter().position(|&(plane, _, _)| plane <= 0.0).unwrap_or(order.len());
        // A node that can paint nothing is not shipped at all. The shader
        // already discards it per fragment, but the billboard is deliberately
        // bigger than the node (QUAD_MARGIN and then some), so the discard is
        // paid a fragment at a time over a quad the disc never reaches — and
        // an unplayed lattice is ENTIRELY such nodes: an idle node draws no
        // light of its own and carries no trail mark, so a still lattice ships
        // its markers and nothing else.
        //
        // The gates are the ones `fs_main`'s idle branch reads, in the same
        // order, off the packed instance rather than the scene node — so this
        // asks the question the shader answers, not a restatement of it that
        // could drift. Reading the PACKED octave word is what makes that exact
        // rather than close: an octave level under half a byte quantizes to
        // zero on the way to the GPU, and a node dropped for that is a node
        // the shader would have discarded anyway.
        //
        // The audio RING is why this is not a property of the node alone: the
        // ring is a window onto the spectrum rather than a level a node
        // carries, so it takes BOTH the layer being on and this node wearing
        // some of the ring (`Scene::wear_audio_rings`) for the node to owe an
        // annulus. With the gate at its floor that is every node in the window,
        // silence included — the ungated picture, and what "the ring reads raw"
        // costs; dialled up, an idle node with nothing sounding at it goes back
        // to shipping nothing at all, ONCE ITS FADE HAS RUN OUT — the level
        // reaches exactly 0 rather than approaching it, so a ring on its way
        // out is shipped for exactly as long as it is drawn. The shader's idle
        // branch pays the rest per fragment, keeping an otherwise idle node to
        // the ring's own annulus and the hole that ring clears around it.
        //
        // A node's own LIGHT is the one thing here that is not a layer, and it
        // is why the cull is not simply "does this node draw ink": the light is
        // carried on a clock of its own (`panes::glow_fade`), so it outlives
        // every layer that lit it, and a node dropped the frame its last layer
        // went silent would take its whole halo off in one. It ships until the
        // light is over — exactly 0, again rather than nearly, so a shipped
        // instance is always one with something to draw.
        let ringing = scene.spectral.ring_draws();
        let lights = scene.glow_reach > 0.0 && scene.glow_strength > 0.0;
        let paints = |g: &GpuInstance| {
            (ringing && g.ring > 0.0)
                || (lights && g.glow[0] > 0.0)
                || g.params[0] > 0.0
                || g.params[1] > 0.0
                || g.params[2] > 0.0
                || (g.octaves[0] | g.octaves[1] | g.octaves[2]) != 0
        };
        // Which marker stands at each node's position, by index into
        // `scene.pluses`. The crosses are derived off the home nodes
        // (`derive_pluses`), so in the plugin every one of them has a node here
        // and the lattice position is what ties the two lists together. That
        // link is what gives a marker its DEPTH: it stands exactly where its
        // node stands, so the sorted order the nodes are walked in is the
        // order the markers want too.
        //
        // A marker at a position no home node holds is LOOSE, having no node to
        // take a depth from. It keeps the place the whole field used to have —
        // over the sheets behind home, under the home sheet — which is the one
        // place a marker of unknown depth cannot be wrong by more than a sheet.
        // Two home nodes claiming one lattice position is the other way in
        // here, the map keeping the last of them.
        let mut plus_of = vec![u32::MAX; scene.nodes.len()];
        let node_at: std::collections::HashMap<_, usize> = scene
            .nodes
            .iter()
            .enumerate()
            .filter(|(_, n)| n.on_home)
            .map(|(i, n)| (n.lattice_pos, i))
            .collect();
        for (p, plus) in scene.pluses.iter().enumerate() {
            if let Some(&i) = node_at.get(&plus.lattice_pos) {
                plus_of[i] = p as u32;
            }
        }
        // Read back off `plus_of` rather than written beside it: a position two
        // markers name has only the last of them in the map, and the first is
        // then claimed by nobody and has to stay loose or it is never drawn.
        let mut claimed = vec![false; scene.pluses.len()];
        for &p in &plus_of {
            if p != u32::MAX {
                claimed[p as usize] = true;
            }
        }
        let to_plus = |d: &harmonigraph_scene::PlusInstance| GpuPlus {
            pos_radius: [d.pos.x, d.pos.y, d.pos.z, d.radius],
            color: [d.color.x, d.color.y, d.color.z, d.strength],
        };
        let loose: Vec<GpuPlus> = scene
            .pluses
            .iter()
            .enumerate()
            .filter(|(p, _)| !claimed[*p])
            .map(|(_, d)| to_plus(d))
            .collect();

        // Where each name's glyphs sit in what the caller handed over, per
        // node, so the walk below can put a name at its own node's place in the
        // order rather than working that place out again afterwards. The cursor
        // advances over labels the scene has no node for as much as over the
        // rest: the run lengths are what say which glyphs are whose, and a
        // label naming a node that is not in the scene is dropped rather than
        // drawn somewhere arbitrary — the caller and the scene disagreeing
        // about how many nodes there are is a bug in the caller.
        let mut glyphs_of = vec![(0u32, 0u32); scene.nodes.len()];
        let mut taken = 0u32;
        for label in &labels.labels {
            let start = taken;
            taken = (taken + label.glyphs).min(labels.glyphs.len() as u32);
            if let Some(slot) = glyphs_of.get_mut(label.node as usize) {
                *slot = (start, taken - start);
            }
        }

        // The one walk: every draw the pass makes, emitted in the order it
        // makes them, filling the four buffers as it goes (see [`Draw`]).
        //
        // Inside ONE home node the spacing is the cross standing at its
        // position, then the node's ink over it: a node covers the cross it
        // stands on, and shadows it, exactly as it covers the sheets behind.
        //
        // The markers are in this walk at all because a cross in FRONT of a
        // node covers that node. Face-on the case barely arises — a node's disc
        // reaches about its own cell, so the only cross near it is its own —
        // but tilt the sheet and one disc spans a dozen positions while the
        // billboard does not foreshorten with it, so where each cross stands in
        // the order is what the reader is being told about depth.
        // One node's ink as a box on the pane, in points — the space a caster
        // is packed in (`shadow::pack`). The billboard faces the camera's own
        // right and up, so those two axes project onto the screen's and one
        // corner along each bounds the circle the node fits inside.
        //
        // A node the projection cannot place — behind the eye, or collapsed to
        // nothing — casts no shadow rather than a box of infinities for the
        // packer to size.
        let points = glam::vec2(size_points.x, size_points.y);
        let to_points =
            |p: glam::Vec3| project_onto(&view_proj, points, p).map(|(at, _)| at.to_array());
        let node_points = scene.node_radius * camera.points_per_world(size_points.y);
        // Each group's own σ in POINTS, read once: a caster carries it and the
        // packer needs no second conversion (`shadow::sigma_points`). A group
        // with either bar at the bottom hands over a σ of nothing, which is the
        // group's off switch all the way down — no cell, no atlas, no taps.
        let geometry = scene.shadow.lattice_geometry;
        let text = scene.shadow.lattice_text;
        let sigma_of = |style: harmonigraph_scene::ShadowStyle| {
            if style.casts() {
                shadow::sigma_points(style.width, node_points)
            } else {
                0.0
            }
        };
        let (geometry_sigma, text_sigma) = (sigma_of(geometry), sigma_of(text));
        // How far the GEOMETRY group's shadow reaches past its own ink, in
        // points — what a node's box is clipped to the pane by.
        let shadow_reach = geometry_sigma * geometry.kernel.reach_sigmas(geometry.falloff);
        let node_caster = |n: &harmonigraph_scene::NodeInstance, g: &GpuInstance| {
            // The circle the node's ink fits inside, in its own uv: `node_rim`
            // in lattice.wgsl, widened by the audio ring, which is dialled on
            // radii of its own and may stand outside the ring stack.
            let mut rim = scene.rings_outer.max(0.0);
            if (g.marks[0] | g.marks[1]) != 0 && scene.mark_thickness > 0.0 {
                rim = rim.max(scene.mark_inner + scene.mark_thickness);
            }
            if ringing && g.ring > 0.0 {
                rim = rim.max(scene.spectral.outer);
            }
            // uv 1 is 1.8 node radii of the node's own sheet (`node_vertex`),
            // which is the one conversion between the bars' unit and the world.
            let reach = rim * scene.node_radius * 1.8 * n.scale.max(0.05);
            let empty = shadow::Caster {
                rect: [0.0; 4],
                level: 0.0,
                sigma_points: geometry_sigma,
                kernel: geometry.kernel,
                falloff: geometry.falloff,
                direct_distance: false,
            };
            let (Some(c), Some(x), Some(y)) = (
                to_points(n.world_pos),
                to_points(n.world_pos + right * reach),
                to_points(n.world_pos + up * reach),
            ) else {
                return empty;
            };
            let half = |axis: usize| (x[axis] - c[axis]).abs().max((y[axis] - c[axis]).abs());
            let (hw, hh) = (half(0), half(1));
            if !(hw.is_finite() && hh.is_finite() && hw > 0.0 && hh > 0.0) {
                return empty;
            }
            // Clipped to the pane the shadow can land on, grown by the blur's
            // own reach. A perspective camera projects a node close to the eye
            // onto a box thousands of panes wide, and the packer sizes the
            // WHOLE atlas off the widest box it is handed: unclipped, one such
            // node takes the atlas to the device's limit and every cell packed
            // after it falls outside and casts nothing. Nothing is lost — a
            // caster's cell is a picture of what its shadow lands on, and past
            // this box it lands off the pane.
            let lo = glam::Vec2::splat(-shadow_reach);
            let hi =
                glam::Vec2::new(size_points.x, size_points.y) + glam::Vec2::splat(shadow_reach);
            let min = glam::Vec2::new(c[0] - hw, c[1] - hh).max(lo);
            let max = glam::Vec2::new(c[0] + hw, c[1] + hh).min(hi);
            if !(max.x > min.x && max.y > min.y) {
                return empty;
            }
            // LEVEL 1: the coverage the cell is filled with already carries
            // every layer's own envelope (`node_ink`), so a released node's
            // shadow fades with its ink and needs no second term here.
            shadow::Caster {
                rect: [min.x, min.y, max.x - min.x, max.y - min.y],
                level: 1.0,
                ..empty
            }
        };
        // One arm of a resting marker on this pane, in points. Every cross is
        // the same shape at the same size — they stand on the home sheet at one
        // radius (`derive_pluses`) — so one number answers for the whole field,
        // through the same conversion a node's own radius takes.
        let points_per_world = camera.points_per_world(size_points.y);
        let marker_arm_points =
            scene.pluses.first().map_or(0.0, |p| (p.radius * points_per_world).max(0.0));

        let mut instances = Vec::with_capacity(order.len());
        let mut glow_owners =
            Vec::with_capacity(if scene.glow_timing.is_some() { order.len() } else { 0 });
        let mut pluses = Vec::with_capacity(scene.pluses.len());
        let mut glyphs = Vec::with_capacity(labels.glyphs.len());
        let mut casters: Vec<shadow::Caster> = Vec::new();
        let mut node_cells: Vec<u32> = Vec::with_capacity(order.len());
        // The marker field's caster, ahead of everything the walk pushes. Its
        // style is the TEXT group's, which is what a marker turns into and out
        // of as a name appears. A Gaussian gives it one shared cell centred on
        // a crossing; a distance keeps only the profile metadata and evaluates
        // the exact field in `plus_paint`.
        if marker_arm_points > 0.0 {
            let a = marker_arm_points;
            casters.push(shadow::Caster {
                rect: [-a, -a, 2.0 * a, 2.0 * a],
                level: 1.0,
                sigma_points: text_sigma,
                kernel: text.kernel,
                falloff: text.falloff,
                direct_distance: true,
            });
        }
        let mut draws: Vec<Draw> = Vec::with_capacity(order.len());
        let mut loose_drawn = false;
        for (k, &(_, _, i)) in order.iter().enumerate() {
            if k == split {
                push_loose(&mut draws, &mut pluses, &loose);
                loose_drawn = true;
            }
            let instance = to_gpu(&scene.nodes[i]);
            let ships = paints(&instance);
            // The cross, whether or not the node it stands on draws anything:
            // an idle position is exactly where a marker does its work, and the
            // node it belongs to is still what says how far off it is.
            if plus_of[i] != u32::MAX {
                push_plus(&mut draws, pluses.len() as u32);
                pluses.push(to_plus(&scene.pluses[plus_of[i] as usize]));
            }
            if ships {
                push_node(&mut draws, instances.len() as u32);
                // Its own cell of the atlas, beside it: what this node's shadow
                // is a blur of, and what it multiplies the frame under it by.
                node_cells.push(casters.len() as u32);
                casters.push(node_caster(&scene.nodes[i], &instance));
                instances.push(instance);
                if scene.glow_timing.is_some() {
                    glow_owners.push(scene.nodes[i].glow.incarnation);
                }
            }
            // The name, immediately after the node it names — so what covers a
            // name is exactly what covers its node.
            let (start, count) = glyphs_of[i];
            if count > 0 {
                let at = glyphs.len() as u32;
                let run = &labels.glyphs[start as usize..(start + count) as usize];
                glyphs.extend_from_slice(run);
                draws.push(Draw::Label(at, at + count, casters.len() as u32));
                casters.push(shadow::caster_of(run, text_sigma, text.kernel, text.falloff));
            }
        }
        // The home run can be empty and can run to the end of the order, in
        // which case the walk never reached `split`.
        if !loose_drawn {
            push_loose(&mut draws, &mut pluses, &loose);
        }

        LatticeCallback {
            pipeline_cache: None,
            instances,
            glow_owners,
            glow_timing: scene.glow_timing,
            glyphs,
            casters,
            node_cells,
            shadow: scene.shadow,
            marker_arm_points,
            draws,
            atlas: labels.atlas,
            marks: labels.marks,
            sdf: labels.sdf,
            slide: labels.slide,
            pluses,
            uniforms: Uniforms {
                composite: CompositeParams {
                    darkest_pitch: scene.darkest_pitch,
                    brightest_pitch: scene.brightest_pitch,
                    render_scale,
                    bloom_strength: bloom_strength(scene.bloom_strength),
                },
                camera: CameraParams {
                    view_proj: Matrix4(view_proj.to_cols_array_2d().map(Float4)),
                    right: Float4(right.extend(0.0).to_array()),
                    up: Float4(up.extend(0.0).to_array()),
                },
                node: NodeParams {
                    radius: scene.node_radius,
                    band_inner: scene.outer_inner,
                    band_outer: scene.outer_outer,
                    rings_outer: scene.rings_outer,
                    mark_inner: scene.mark_inner,
                    angular_gap: scene.octave_gap,
                    mark_thickness: scene.mark_thickness,
                    padding: 0.0,
                },
                marker: MarkerParams {
                    half_width: scene.plus_half_width,
                    taper_start: scene.plus_taper_start,
                    world_unit: scene.marker_unit,
                    padding: 0.0,
                },
                octave: OctaveParams {
                    span: scene.octave_layout.span as f32,
                    center: scene.octave_layout.center,
                    padding: Float2([0.0; 2]),
                    bounds: std::array::from_fn(|row| {
                        Float4(std::array::from_fn(|col| scene.octave_layout.bounds[row * 4 + col]))
                    }),
                },
                spectral: SpectralParams {
                    inner: scene.spectral.inner,
                    outer: scene.spectral.outer,
                    range_cents: scene.spectral.range,
                    folded: f32::from(u8::from(scene.spectral.folded)),
                },
                glow: if lights {
                    GlowParams {
                        reach: scene.glow_reach,
                        strength: scene.glow_strength,
                        blend: scene.glow_blend,
                        curve: scene.glow_curve.shape(),
                        wash: scene.glow_wash,
                        row_capacity: scene.glow_rows.max(1) as f32,
                        // Settled by prepare, which is where the lit nodes are
                        // mapped onto the target's pixels.
                        lit: 0.0,
                        accumulation: scene.glow_accumulation,
                    }
                } else {
                    bytemuck::Zeroable::zeroed()
                },
                // Every shadow still casts with the glow disabled. Markers
                // inherit notation's style even though this pipeline draws them.
                geometry_shadow: ShadowParams {
                    width: geometry.width,
                    reach_sigmas: geometry.kernel.reach_sigmas(geometry.falloff),
                    depth: geometry.depth,
                    padding: 0.0,
                },
                marker_shadow: ShadowParams {
                    width: text.width,
                    reach_sigmas: text.kernel.reach_sigmas(text.falloff),
                    depth: text.depth,
                    padding: 0.0,
                },
                shadow_target: ShadowTargetParams {
                    pane_points: Float2([size_points.x, size_points.y]),
                    atlas_texels: Float2([0.0; 2]),
                },
                // Settled by prepare after packing casters[0].
                marker_cell: bytemuck::Zeroable::zeroed(),
                lattice_ground: Float4(scene.lattice_ground.to_array()),
                pitch_lut: std::array::from_fn(|k| Float4(scene.pitch_lut[k].to_array())),
                spectral_lut: std::array::from_fn(|k| Float4(scene.spectral.lut[k].to_array())),
                // No shader reader while the ring is off; skip the bucket pack.
                spectrum_color: if scene.spectral.ring_draws() {
                    pack_spectrum(&scene.spectral.color_levels).map(Uint4)
                } else {
                    [Uint4([0; 4]); SPECTRUM_WORDS]
                },
            },
            target_format,
            pane_id,
            size_points: [size_points.x, size_points.y],
            render_scale,
            stats,
        }
    }
}
