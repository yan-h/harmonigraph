//! Which instances reach the GPU at all, and the early-outs that drop them.

use crate::*;
use super::fixtures::*;
use crate::gpu_harness::{headless_device, readback, render_to_texture};

/// The fragment shader's early-outs — skipping the fragments outside
/// anything a node can paint, and the whole note path for an idle node —
/// must be exactly that: an optimization. Same scene through the real
/// shader and through one compiled with `EARLY_OUT` off, pixel for pixel.
///
/// Worth a GPU test rather than a reading of the code, because the bound in
/// `paint_reach` is a claim about EVERY layer's falloff at once: add a layer
/// that reaches further, or widen one's soft edge past its radius, and the
/// only symptom is a quietly clipped halo somewhere off the node.
#[test]
fn the_fragment_early_outs_do_not_change_a_pixel() {
    let Some((device, queue)) = headless_device() else {
        return;
    };
    const SIZE: [u32; 2] = [256, 256];
    let format = wgpu::TextureFormat::Rgba8Unorm;
    let reference_src = SHADER_SRC.replace(
        "const EARLY_OUT: bool = true;",
        "const EARLY_OUT: bool = false;",
    );
    assert_ne!(
        reference_src, SHADER_SRC,
        "the EARLY_OUT switch was renamed; this test is no longer comparing anything",
    );

    // The sheet running, at one fixed instant. `paint_reach` is where the
    // claim that shimmer keeps every bound exact has to be checked, and it can
    // only be checked here: the sheet relights the rings and the marked
    // slice inside their own coverage — `shimmer_terms` never touches
    // coverage — and a sheet that widened what a layer paints would push it
    // past the reach the early-out proved it could not cross, visible as a
    // ring clipped flat in the fast pipeline alone, which no other fixture
    // would catch because every other one leaves the pulse Off.
    let shimmering = || {
        let mut scene = parity_scene();
        scene.pulse_marks = harmonigraph_scene::Pulse::Bands;
        scene
    };
    // The audio ring on, which has two early-outs of its own: the annulus
    // skip inside `spectral_ring`, and the idle branch's radial exception,
    // which keeps an otherwise idle node's fragments where the ring is and out
    // to the edge of the hole that ring clears.
    // Both are answers about a layer NO other fixture here draws — the ring is
    // off in `parity_scene` — so without this the two switches would be
    // compiled and never compared.
    let ringing = || {
        let mut scene = parity_scene();
        let paint = &mut scene.spectral;
        paint.inner = 0.20;
        paint.outer = 0.38;
        paint.range = 300.0;
        paint.lut = std::array::from_fn(|k| {
            let t = k as f32 / (harmonigraph_scene::PITCH_LUT_N - 1) as f32;
            glam::Vec4::new(t, 0.6 * t, 1.0 - t, 1.0)
        });
        // A comb across the grid rather than a flat level, so a wedge carries
        // an EDGE: a fragment either side of one reads a different bucket, and
        // an early-out that shifted the sampled pitch by so much as a bucket
        // shows up as a moved edge rather than being averaged away.
        for (bucket, level) in paint.levels.iter_mut().enumerate() {
            *level = if (bucket / 7) % 3 == 0 { 220 } else { 20 };
        }
        // ...and a node with a ring and nothing else, which is the idle
        // branch's new case: no activation, no marks, and an annulus to draw.
        let mut silent = scene.nodes[0];
        silent.activation = 0.0;
        silent.octaves = [0.0; harmonigraph_scene::OCTAVE_SLOTS];
        silent.melody_slots = 0;
        silent.bass_slots = 0;
        silent.melody_level = 0.0;
        silent.bass_level = 0.0;
        // ...and a reach, which is the whole point of it: the idle branch keeps
        // an idle node's fragments out to the edge of the hole its ring clears,
        // and `parity_scene` hands its gutters to the off-sheet half — node 0,
        // which this is a copy of, carries none. Without this the branch is
        // compiled on both pipelines and never once decides anything, so the
        // one spelling of `clearing_edge` it shares with the coverage is
        // untested where a second spelling would show.
        silent.gutter = 0.12;
        silent.world_pos.x += 0.9;
        scene.nodes.push(silent);
        scene
    };
    // A clearing around a MARKED node, which is `node_clearing`'s own
    // skip: the clearing's shape is the rings' disc unioned with one wedge per
    // mark, and inside that disc the walk over the wedges is skipped as an
    // answer already arrived at. `parity_scene` hands its gutters and its marks
    // to different halves of its nodes — the off-sheet ones clear, the marked
    // ones are on the home sheet — so without this the two never meet on one
    // node and the skip is never taken.
    let clearing = || {
        let mut scene = parity_scene();
        for node in &mut scene.nodes {
            node.gutter = 0.16;
        }
        scene
    };
    // The ring's OTHER reading, which is the shader's second branch inside
    // `spectral_ring`: every fragment of a wedge reads the octave's own pitch
    // instead of walking a window across it, so the whole `wedge_fraction`
    // path is skipped and a wedge comes out flat.
    let folded = || {
        let mut scene = ringing();
        scene.spectral.folded = true;
        scene
    };
    // The STANDOFF around those same marked nodes, which is `glow_standoff`'s own
    // skip — the same one as `node_clearing`'s, over the same wedges, taken
    // once the band has held the pixel's light off in full. The REACH is what
    // puts it in the comparison at all, and no fixture above has one: the Gap
    // dials ride to the GPU only while the light does (`misc11` is zeroed at
    // reach 0), and there is no light draw to compare without a glow target
    // for it to write into. Both are the same switch, which is why one line
    // buys the fixture.
    let standing_off = || {
        let mut scene = clearing();
        scene.glow_reach = 0.8;
        // One node with INK and no light of its own, which is the other half of
        // `fs_glow`'s early-out: it weighs the standoff's answer as well as the
        // light's, and a fixture whose every node is lit decides it on the
        // light's alone — a relaxation that dropped the standoff's term would
        // then discard this node's band and nothing here would see it. Shipped
        // for its ink (`paints`), it draws its own layers and stands the light
        // off them while emitting none.
        let mut dark = scene.nodes[0];
        dark.glow.level = 0.0;
        dark.glow.row = scene.glow_rows;
        dark.world_pos.x -= 0.9;
        scene.glow_rows += 1;
        scene.nodes.push(dark);
        scene
    };
    // ...and that standoff with the ring CLOSED, which is `slice_gap_distance`'s
    // own skip: with the slices meeting edge to edge there is no gap to walk
    // the boundaries of, and the ring's plain radial distance has to be the
    // answer on both pipelines. Every fixture above carries a gap, so without
    // this the branch is compiled and never once decides anything.
    let closed_ring = || {
        let mut scene = standing_off();
        scene.octave_gap = 0.0;
        scene
    };
    // A marker's standoff reaching PAST its own pool, which is the only way
    // `fs_plus_glow`'s early-out decides anything: inside the pool the light
    // keeps every fragment on its own, and a fragment the standoff holds and
    // the light does not exists only out here. Every fixture above sizes the
    // marker's pool wider than its Gap, so without this the switch is compiled
    // on the markers and never once taken — a relaxation dropping the
    // standoff's term from it passes them all.
    let marker_standoff = || {
        let mut scene = standing_off();
        scene.glow_gap = 1.0;
        scene.glow_gap_soft = 1.0;
        scene.marker_light = 0.5;
        scene
    };
    // No all-idle fixture: an idle node paints nothing, so the cull ships
    // none of them and the comparison would be two empty images. What the
    // idle branch does is now pinned by
    // `a_silent_lattice_ships_no_nodes_and_still_draws_its_markers` instead,
    // on the CPU side where the decision actually lives.
    for (name, scene) in [
        ("lit", parity_scene()),
        ("shimmering", shimmering()),
        ("ringing", ringing()),
        ("folded", folded()),
        ("clearing", clearing()),
        ("standing off", standing_off()),
        ("standing off a closed ring", closed_ring()),
        ("a marker standing off past its pool", marker_standoff()),
    ] {
        let cb = LatticeCallback::from_scene(
            &scene,
            LatticeLabels::default(),
            egui::vec2(SIZE[0] as f32, SIZE[1] as f32),
            format,
            11,
            None,
        );
        let mut resources = CallbackResources::default();
        let screen = ScreenDescriptor { size_in_pixels: SIZE, pixels_per_point: 1.0 };
        let mut encoder = device.create_command_encoder(&Default::default());
        let bufs = cb.prepare(&device, &queue, &screen, &mut encoder, &mut resources);
        queue.submit(bufs.into_iter().chain([encoder.finish()]));

        let res: &LatticeResources = resources.get().expect("prepare created resources");
        let layouts =
            SceneLayouts { uniforms: &res.bind_group_layout, glow: &res.glow_layout };
        let build = |src: &str| create_pipelines(&device, src, format, layouts, false);
        let (fast, _) = build(SHADER_SRC);
        let (slow, _) = build(&reference_src);
        // The light at group 1: one colour over the whole frame, bound to both
        // pipelines, so a clearing reads the same thing back whichever is
        // drawing and what they differ by is the early-outs alone. A constant
        // rather than the 1x1 stand-in because `node_paint` reads its ground
        // and its Wash OUT of this, and a transparent nothing leaves both terms
        // at zero on either pipeline — which takes the whole of what a clearing
        // does with the light out of the comparison. Premultiplied, as the real
        // target is, and well under opaque so the ground still shows through it.
        let light = {
            let texture = device.create_texture(&wgpu::TextureDescriptor {
                label: Some("parity_light"),
                size: wgpu::Extent3d { width: SIZE[0], height: SIZE[1], depth_or_array_layers: 1 },
                mip_level_count: 1,
                sample_count: 1,
                dimension: wgpu::TextureDimension::D2,
                format,
                usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
                view_formats: &[],
            });
            let texel = [96u8, 64, 32, 128];
            queue.write_texture(
                wgpu::TexelCopyTextureInfo {
                    texture: &texture,
                    mip_level: 0,
                    origin: wgpu::Origin3d::ZERO,
                    aspect: wgpu::TextureAspect::All,
                },
                &texel.repeat((SIZE[0] * SIZE[1]) as usize),
                wgpu::TexelCopyBufferLayout {
                    offset: 0,
                    bytes_per_row: Some(SIZE[0] * 4),
                    rows_per_image: Some(SIZE[1]),
                },
                wgpu::Extent3d { width: SIZE[0], height: SIZE[1], depth_or_array_layers: 1 },
            );
            let view = texture.create_view(&Default::default());
            // Never written, and RENDER_ATTACHMENT is what lets wgpu zero it:
            // a coverage of zero is the light kept whole. Full size rather than
            // 1x1 because `node_paint` clamps its read against the LIGHT's
            // dimensions, so a smaller layer beside it would be read out of
            // bounds.
            let shade = device
                .create_texture(&wgpu::TextureDescriptor {
                    label: Some("parity_shade"),
                    size: wgpu::Extent3d {
                        width: SIZE[0],
                        height: SIZE[1],
                        depth_or_array_layers: 1,
                    },
                    mip_level_count: 1,
                    sample_count: 1,
                    dimension: wgpu::TextureDimension::D2,
                    format: GLOW_SHADE_FORMAT,
                    usage: wgpu::TextureUsages::TEXTURE_BINDING
                        | wgpu::TextureUsages::RENDER_ATTACHMENT,
                    view_formats: &[],
                })
                .create_view(&Default::default());
            device.create_bind_group(&wgpu::BindGroupDescriptor {
                label: Some("parity_light_bind_group"),
                layout: &res.glow_layout,
                entries: &[
                    wgpu::BindGroupEntry {
                        binding: 0,
                        resource: wgpu::BindingResource::TextureView(&view),
                    },
                    wgpu::BindGroupEntry {
                        binding: 1,
                        resource: wgpu::BindingResource::Sampler(&res.sampler),
                    },
                    // The one texture at both of the light's slots, so the
                    // Meld mixes a colour with itself and hands back that
                    // colour at every setting: what this fixture holds still
                    // is the light a clearing reads, and the bar is not what
                    // it is comparing.
                    wgpu::BindGroupEntry {
                        binding: 2,
                        resource: wgpu::BindingResource::TextureView(&view),
                    },
                    // A standoff of nothing, so the light above reaches these
                    // pipelines whole. What the standoff DOES is compared
                    // below, in the pass that now writes it; here it would only
                    // dim the constant this fixture is holding still.
                    wgpu::BindGroupEntry {
                        binding: 3,
                        resource: wgpu::BindingResource::TextureView(&shade),
                    },
                ],
            })
        };
        let light = &light;
        let pane = res.panes.get(&11).expect("prepare created the pane");

        let clear = wgpu::Color { r: 0.07, g: 0.08, b: 0.09, a: 1.0 };
        let draw = |pipeline: &wgpu::RenderPipeline| {
            let texture = render_to_texture(&device, &queue, SIZE, format, clear, |pass| {
                pass.set_pipeline(pipeline);
                pass.set_bind_group(0, &pane.bind_group, &[]);
                pass.set_bind_group(1, light, &[]);
                pass.set_vertex_buffer(0, pane.instance_buffer.slice(..));
                pass.draw(0..4, 0..pane.instance_count);
            });
            readback(&device, &queue, &texture, SIZE)
        };
        let with_early_out = draw(&fast);
        let without = draw(&slow);

        // Guard against a vacuous pass: the nodes must have drawn something
        // over the clear color.
        let bg = [18u8, 20, 23, 255];
        assert!(
            without.chunks(4).any(|px| px.iter().zip(bg).any(|(&c, b)| c.abs_diff(b) > 8)),
            "the {name} scene drew nothing; the comparison is vacuous",
        );

        let differing = with_early_out
            .iter()
            .zip(&without)
            .enumerate()
            .find(|(_, (&a, &b))| a != b);
        assert!(
            differing.is_none(),
            "the {name} scene changed when the early-outs were enabled: byte {:?}",
            differing.map(|(i, (a, b))| (i, *a, *b)),
        );

        // The LIGHT's own draw, compared the same way and for the same reason.
        // Three of its early-outs are only reachable here: `fs_glow`'s own,
        // which weighs the standoff's answer as well as the light's now that
        // one fragment carries both; `glow_standoff`'s skip inside it; and
        // `slice_gap_distance`'s. `node_paint` reaches none of them — it reads
        // the standoff back off a texture rather than computing it — so without
        // this the three would be compiled over that code and never once
        // compared.
        //
        // The SHADE attachment is what is read back, being the one the standoff
        // writes: a skip that dropped a band would leave the light beside it
        // identical and show only here.
        let Some(glow) = pane.offscreen.as_ref().and_then(|o| o.glow.as_ref()) else {
            continue;
        };
        let glow_draw = |src: &str,
                         entries: (&str, &str),
                         buffers: wgpu::VertexBufferLayout<'static>,
                         buffer: &wgpu::Buffer,
                         count: u32| {
            let pipeline = create_glow_pipeline(
                &device,
                src,
                format,
                &res.bind_group_layout,
                &res.strip_layout,
                entries,
                buffers,
            );
            let attachment = |label, format| {
                device.create_texture(&wgpu::TextureDescriptor {
                    label: Some(label),
                    size: wgpu::Extent3d {
                        width: SIZE[0],
                        height: SIZE[1],
                        depth_or_array_layers: 1,
                    },
                    mip_level_count: 1,
                    sample_count: 1,
                    dimension: wgpu::TextureDimension::D2,
                    format,
                    usage: wgpu::TextureUsages::RENDER_ATTACHMENT
                        | wgpu::TextureUsages::COPY_SRC,
                    view_formats: &[],
                })
            };
            let targets = [
                attachment("parity_glow", format),
                attachment("parity_glow_max", format),
                attachment("parity_glow_shade", GLOW_SHADE_FORMAT),
            ];
            let views: Vec<_> = targets.iter().map(|t| t.create_view(&Default::default())).collect();
            let mut encoder = device.create_command_encoder(&Default::default());
            {
                let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                    label: Some("parity_glow_pass"),
                    color_attachments: &views
                        .iter()
                        .map(|view| {
                            Some(wgpu::RenderPassColorAttachment {
                                view,
                                depth_slice: None,
                                resolve_target: None,
                                ops: wgpu::Operations {
                                    load: wgpu::LoadOp::Clear(wgpu::Color::TRANSPARENT),
                                    store: wgpu::StoreOp::Store,
                                },
                            })
                        })
                        .collect::<Vec<_>>(),
                    depth_stencil_attachment: None,
                    timestamp_writes: None,
                    occlusion_query_set: None,
                    multiview_mask: None,
                });
                pass.set_pipeline(&pipeline);
                pass.set_bind_group(0, &pane.bind_group, &[]);
                pass.set_bind_group(1, &glow.strip.blurred_bind_group, &[]);
                pass.set_vertex_buffer(0, buffer.slice(..));
                pass.draw(0..4, 0..count);
            }
            queue.submit([encoder.finish()]);
            // BOTH quantities the pass writes, because a skip can drop either
            // one alone: an early-out that went on the light's answer where it
            // should go on both leaves the standoff's layer untouched, and one
            // that went on the standoff's leaves the LIGHT's. The `max`-blended
            // light is left out of the readback and not out of the comparison —
            // one fragment emits it and the screened attachment together, so a
            // dropped fragment shows in this one.
            //
            // `readback`'s copy with its one assumption widened: a 256-wide row
            // is 1024 bytes of light or 512 of coverage, and both are aligned.
            let read = |target: &wgpu::Texture, bytes: u32| {
                let bytes_per_row = SIZE[0] * bytes;
                let buffer = device.create_buffer(&wgpu::BufferDescriptor {
                    label: Some("parity_glow_readback"),
                    size: (bytes_per_row * SIZE[1]) as u64,
                    usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
                    mapped_at_creation: false,
                });
                let mut encoder = device.create_command_encoder(&Default::default());
                encoder.copy_texture_to_buffer(
                    target.as_image_copy(),
                    wgpu::TexelCopyBufferInfo {
                        buffer: &buffer,
                        layout: wgpu::TexelCopyBufferLayout {
                            offset: 0,
                            bytes_per_row: Some(bytes_per_row),
                            rows_per_image: None,
                        },
                    },
                    wgpu::Extent3d { width: SIZE[0], height: SIZE[1], depth_or_array_layers: 1 },
                );
                queue.submit([encoder.finish()]);
                let slice = buffer.slice(..);
                slice.map_async(wgpu::MapMode::Read, |r| r.expect("map readback buffer"));
                device.poll(wgpu::PollType::wait_indefinitely()).expect("poll");
                slice.get_mapped_range().to_vec()
            };
            (read(&targets[0], 4), read(&targets[2], 2))
        };
        // BOTH draws that write the glow's three attachments. The marker's is a
        // pipeline of its own off the same shader, with an early-out of its own
        // (`fs_plus_glow`) weighing a standoff and a pool that a node's never
        // sees — so without this the switch is compiled on the markers and
        // never once compared, which is the whole claim this test makes.
        for (pass_name, entries, buffers, buffer, count) in [
            (
                "node",
                ("vs_glow", "fs_glow"),
                GpuInstance::LAYOUT,
                &pane.instance_buffer,
                pane.instance_count,
            ),
            (
                "marker",
                ("vs_plus_glow", "fs_plus_glow"),
                GpuPlus::LAYOUT,
                &pane.plus_buffer,
                pane.plus_count,
            ),
        ] {
            assert!(count > 0, "the {name} scene ships no {pass_name} to compare");
            let (light_fast, shade_fast) =
                glow_draw(SHADER_SRC, entries, buffers.clone(), buffer, count);
            let (light_slow, shade_slow) =
                glow_draw(&reference_src, entries, buffers, buffer, count);

            // Vacuous unless the pass actually wrote each of them: every fixture
            // that reaches here carries a reach and a depth, so a layer of zeroes
            // means the dials stopped arriving rather than that the skips are sound.
            assert!(
                shade_slow.iter().any(|&b| b != 0),
                "the {name} scene's {pass_name} held no light off; \
                 the standoff comparison is vacuous",
            );
            assert!(
                light_slow.iter().any(|&b| b != 0),
                "the {name} scene's {pass_name} lit nothing; the light comparison is vacuous",
            );

            for (layer, fast, slow) in
                [("light", &light_fast, &light_slow), ("standoff", &shade_fast, &shade_slow)]
            {
                let differing =
                    fast.iter().zip(slow.iter()).enumerate().find(|(_, (a, b))| a != b);
                assert!(
                    differing.is_none(),
                    "the {name} scene's {pass_name} {layer} changed when the early-outs \
                     were enabled: byte {:?}",
                    differing.map(|(i, (a, b))| (i, *a, *b)),
                );
            }
        }
    }
}

/// A node that can paint nothing is not shipped at all — and the marker standing
/// at its position still is.
///
/// The billboard is deliberately bigger than the node, so a node the shader
/// discards every fragment of still costs a quad's worth of rasterizing; on
/// an unplayed lattice that is EVERY node, an idle one drawing nothing of its
/// own and carrying no trail mark. So the frame drops to the marker field and
/// nothing else, and the callback has to keep drawing that field — which is
/// why neither `prepare` nor `paint` may read "no instances" as "nothing to
/// draw": that test takes the markers down with the nodes.
#[test]
fn a_silent_lattice_ships_no_nodes_and_still_draws_its_markers() {
    let scene = idle_scene();
    assert!(!scene.pluses.is_empty(), "the fixture has to carry a marker field");
    let cb = LatticeCallback::from_scene(
        &scene,
        LatticeLabels::default(),
        egui::vec2(256.0, 256.0),
        wgpu::TextureFormat::Rgba8Unorm,
        31,
        None,
    );
    assert!(
        cb.instances.is_empty(),
        "every node is idle with nothing to draw at one, so none should ship",
    );
    assert!(!cb.pluses.is_empty(), "a marker is not a node and must survive");

    let Some((device, queue)) = headless_device() else {
        return;
    };
    const SIZE: [u32; 2] = [256, 256];
    let mut resources = CallbackResources::default();
    let screen = ScreenDescriptor { size_in_pixels: SIZE, pixels_per_point: 1.0 };
    let mut encoder = device.create_command_encoder(&Default::default());
    let bufs = cb.prepare(&device, &queue, &screen, &mut encoder, &mut resources);
    queue.submit(bufs.into_iter().chain([encoder.finish()]));
    let rect = egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(256.0, 256.0));
    let clear = wgpu::Color { r: 0.07, g: 0.08, b: 0.09, a: 1.0 };
    let texture = render_to_texture(&device, &queue, SIZE, format_of(&cb), clear, |pass| {
        cb.paint(
            egui::PaintCallbackInfo {
                viewport: rect,
                clip_rect: rect,
                pixels_per_point: 1.0,
                screen_size_px: SIZE,
            },
            pass,
            &resources,
        );
    });
    let px = readback(&device, &queue, &texture, SIZE);
    let bg = [18u8, 20, 23, 255];
    assert!(
        px.chunks(4).any(|p| p.iter().zip(bg).any(|(&c, b)| c.abs_diff(b) > 4)),
        "the markers vanished with the nodes",
    );
}

/// The ring's floor colour is a picture on every node, so the ring being on
/// is itself a reason to ship an instance: the cull that drops idle nodes
/// keeps all of them the moment the annulus is real. Nothing else reaches
/// that term — the pixel tests above light their nodes, which passes the
/// cull on activation instead.
#[test]
fn an_open_ring_ships_every_idle_node() {
    let fresh = harmonigraph_scene::ViewConfig::default();
    let mut scene = idle_scene();
    assert!(!scene.nodes.is_empty(), "the fixture has to carry idle nodes");
    (scene.spectral.inner, scene.spectral.outer) = fresh.rings().audio;
    let cb = LatticeCallback::from_scene(
        &scene,
        LatticeLabels::default(),
        egui::vec2(256.0, 256.0),
        wgpu::TextureFormat::Rgba8Unorm,
        32,
        None,
    );
    assert_eq!(
        cb.instances.len(),
        scene.nodes.len(),
        "with the ring on, a node with nothing else to draw still wears the floor colour",
    );
}

/// ...and the other end of that term: a node whose ring has faded out owes no
/// annulus, so the ring layer being ON is not on its own a reason to ship it.
///
/// The whole cost argument for the gate is here rather than in the picture —
/// a gated-off idle node ships nothing at all, where an ungated ring forces
/// every node in the window onto the bus. Nothing else can see it: a shipped
/// node with no ring and nothing else to draw is transparent at every
/// fragment, so no pixel test can tell it from one that was never sent, and
/// [`an_open_ring_ships_every_idle_node`] only ever drives the term's true
/// side.
///
/// Part way through the fade is the case that says it is a LEVEL and not the
/// gate's own bit: a ring on its way out is drawn, so it is shipped for
/// exactly as long as it is drawn.
#[test]
fn a_faded_out_ring_ships_no_idle_node() {
    let fresh = harmonigraph_scene::ViewConfig::default();
    let mut scene = idle_scene();
    assert!(!scene.nodes.is_empty(), "the fixture has to carry idle nodes");
    (scene.spectral.inner, scene.spectral.outer) = fresh.rings().audio;
    let ships = |scene: &Scene| {
        LatticeCallback::from_scene(
            scene,
            LatticeLabels::default(),
            egui::vec2(256.0, 256.0),
            wgpu::TextureFormat::Rgba8Unorm,
            32,
            None,
        )
        .instances
        .len()
    };
    for node in &mut scene.nodes {
        node.audio_ring = 0.0;
    }
    assert_eq!(ships(&scene), 0, "an idle node with no ring left was still shipped");
    for node in &mut scene.nodes {
        node.audio_ring = 0.5;
    }
    assert_eq!(
        ships(&scene),
        scene.nodes.len(),
        "an idle node part way through its fade draws a ring and has to be shipped",
    );
}

/// A ring wedge takes its colour from the analyzer's ramp at ITS OWN level —
/// not from the node's pitch colour scaled by that level, which is the octave
/// band's MIDI logic and the exact scheme confusion the two tables exist to
/// prevent. The ramp here switches hue at half: a level on one side paints
/// blue, on the other red, and no colour-times-level path can flip a hue, so
/// the flip is the shader indexing the ramp.
///
/// Read on the FOLDED branch, where a wedge is one flat level: the raw one
/// spreads a window of the grid across the wedge, and a shot of it would be
/// two hues at once by design.
#[test]
fn a_ring_wedge_wears_its_own_levels_ramp_entry() {
    const SIZE: [u32; 2] = [256, 256];
    let Some(mut gpu) = Shooter::new(SIZE) else {
        return;
    };

    let shot_at = |gpu: &mut Shooter, level: u8| -> Vec<u8> {
        let fresh = harmonigraph_scene::ViewConfig::default();
        let mut scene = single_marked_node(0, 0);
        let node = &mut scene.nodes[0];
        // Nothing held, so what is on screen is the ring alone: the band's own
        // wedges are the MIDI picture and would sum into the channels below.
        node.octaves = [0.0; harmonigraph_scene::OCTAVE_SLOTS];
        node.activation = 1.0;
        let rings = fresh.rings();
        scene.outer_inner = rings.band.0;
        scene.outer_outer = rings.band.1;
        scene.rings_outer = rings.outer;
        scene.octave_gap = fresh.octave_gap_width();
        let mut paint = harmonigraph_scene::SpectralPaint::silent();
        (paint.inner, paint.outer) = rings.audio;
        paint.folded = true;
        // The whole grid at one level, so every wedge whose octave the
        // analyzer's axis reaches reads the same entry. Off the axis
        // `spectrum_at` answers 0 whatever the grid holds, which this wheel
        // stays clear of.
        paint.levels = Box::new([level; harmonigraph_scene::SPECTRAL_BUCKETS]);
        paint.lut = std::array::from_fn(|k| {
            let t = k as f32 / (harmonigraph_scene::PITCH_LUT_N - 1) as f32;
            if t < 0.5 {
                glam::Vec4::new(0.0, 0.0, 1.0, 1.0)
            } else {
                glam::Vec4::new(1.0, 0.0, 0.0, 1.0)
            }
        });
        scene.spectral = paint;
        gpu.shot(&scene)
    };

    let low = shot_at(&mut gpu, 90);
    let high = shot_at(&mut gpu, 217);
    let sum = |px: &[u8], ch: usize| -> i64 { px.chunks(4).map(|p| p[ch] as i64).sum() };
    let (blue_low, blue_high) = (sum(&low, 2), sum(&high, 2));
    let (red_low, red_high) = (sum(&low, 0), sum(&high, 0));
    eprintln!("low: red {red_low} blue {blue_low}; high: red {red_high} blue {blue_high}");
    // The margin is a wedge's worth of one channel against antialiasing
    // fringes; the ring itself sums in the tens of thousands.
    const HUE_FLIP: i64 = 5_000;
    assert!(
        blue_low > blue_high + HUE_FLIP && red_high > red_low + HUE_FLIP,
        "crossing the ramp's half did not flip the wedge's hue: the ring is not \
         indexing the analyzer's ramp at the wedge's own level",
    );
}

/// The target format the callback was built for, so the test above renders
/// into the one its composite pipeline expects.
fn format_of(cb: &LatticeCallback) -> wgpu::TextureFormat {
    cb.target_format
}

/// One way of making a node sound, for the table in the test below.
type LightUp = fn(&mut harmonigraph_scene::NodeInstance);

/// Each of the four things that make a node sounding keeps it on its own.
///
/// The cull's first question is whether anything is lit — an envelope, a
/// melody or bass ring's level, or a lit octave — and it is a disjunction
/// over four terms that ordinarily move together: `derive_scene` rides all
/// of them on one envelope, so a scene built by playing notes has either
/// none of them or several, and no single term ever decides whether a node
/// ships. That is not a hypothetical gap. Inverting any one of the four,
/// or turning any `||` between them into `&&`, passed the whole suite.
///
/// They come apart in practice, which is why each gets a node here. An
/// octave's level is `envelope * attack(...)` and is packed to a byte, so
/// for the first frames after a note-on a node has a full activation and an
/// octave word of exactly zero — `params[0]` alone is holding it in the
/// buffer, and a cull that stopped reading it would drop the first frame of
/// a note. The mark levels ride their own ease-in (`melody_attack`) rather
/// than the node's, so they part company the same way.
///
/// Built by hand rather than played in: the point is one term at a time,
/// and a tracker cannot be asked for that.
#[test]
fn each_thing_that_makes_a_node_sounding_keeps_it_alone() {
    let bare = || {
        let mut scene = idle_scene();
        // An idle node draws nothing whatever its memory, so the only reason
        // to keep one is the term under test.
        for node in &mut scene.nodes {
            node.trail = 0.0;
        }
        scene
    };
    let ships = |scene: &Scene| {
        LatticeCallback::from_scene(
            scene,
            LatticeLabels::default(),
            egui::vec2(256.0, 256.0),
            wgpu::TextureFormat::Rgba8Unorm,
            33,
            None,
        )
        .instances
        .len()
    };
    assert_eq!(ships(&bare()), 0, "the fixture has to start with nothing drawn");

    // One node per term, each the ONLY lit thing about it.
    let cases: [(&str, LightUp); 4] = [
        ("activation", |n| n.activation = 1.0),
        ("melody level", |n| n.melody_level = 1.0),
        ("bass level", |n| n.bass_level = 1.0),
        ("a lit octave", |n| n.octaves[harmonigraph_scene::MIDDLE_C_SLOT] = 1.0),
    ];
    for (what, set) in cases {
        let mut scene = bare();
        set(&mut scene.nodes[0]);
        assert_eq!(ships(&scene), 1, "{what} alone has to keep its node");
    }

    // The octave levels pack into three u32s, four slots to a word, and the
    // three are OR'd. Two octaves of one pitch class held at the same level
    // is an ordinary voicing, and it puts the SAME byte in two words — where
    // anything but an OR cancels them against each other and reads the node
    // as unlit. Every pairing of words is needed, because which two cancel
    // depends on where the operator lands and how it then binds: `^` binds
    // tighter than `|`, so swapping either one regroups the whole expression
    // as well as changing it.
    for (a, b) in [(0usize, 4usize), (0, 8), (4, 8)] {
        let mut spread = bare();
        spread.nodes[0].octaves[a] = 1.0;
        spread.nodes[0].octaves[b] = 1.0;
        assert_eq!(
            ships(&spread),
            1,
            "octaves {a} and {b} held at one level keep their node",
        );
    }
}

/// The markers' place in the draw order is counted over the nodes actually
/// shipped, not over the ones the scene held.
///
/// `pluses_at` is the seam between the sheets BEHIND the home sheet and the
/// home sheet itself, and the whole argument for it is in `from_scene`: put
/// the markers under everything and a node on a sheet behind the home one
/// punches its clearing through the home markers. Culling breaks the old
/// expression silently, because `split` indexes the list before the cull —
/// with the sheets behind home mostly idle, `split` runs past the end of the
/// kept run, `prepare`'s `.min(instance_count)` pins the markers to the very
/// end, and it draws over every node instead of under the home sheet.
///
/// One lit node behind home and one on it, with an idle node behind home
/// between them: the seam has to land at 1, and it is `split` (2) that says
/// otherwise.
#[test]
fn the_marker_seam_counts_the_nodes_that_ship() {
    let mut scene = idle_scene();
    scene.nodes.truncate(3);
    for node in &mut scene.nodes {
        node.trail = 0.0;
        node.activation = 0.0;
        node.octaves = [0.0; harmonigraph_scene::OCTAVE_SLOTS];
    }
    // Which world z is "behind" is the camera's to say — the sort keys on
    // `world_pos.z * forward.z` — and the default camera's eye sits at +z, so
    // the sheets behind the home one are the negative side.
    scene.nodes[0].world_pos.z = -2.0;
    scene.nodes[0].activation = 1.0; // behind home, lit: ships, before the markers
    scene.nodes[1].world_pos.z = -1.0; // behind home, idle: culled
    scene.nodes[2].world_pos.z = 0.0;
    scene.nodes[2].activation = 1.0; // the home sheet: ships, after the markers
    scene.nodes[2].on_home = true;

    let call = LatticeCallback::from_scene(
        &scene,
        LatticeLabels::default(),
        egui::vec2(256.0, 256.0),
        wgpu::TextureFormat::Rgba8Unorm,
        34,
        None,
    );
    assert_eq!(call.instances.len(), 2, "the idle node behind home is culled");
    assert_eq!(
        call.pluses_at, 1,
        "the markers draw after the one sheet-behind node that ships, not after \
         the two the scene held",
    );
}
