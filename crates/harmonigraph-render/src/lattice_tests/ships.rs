//! Which instances reach the GPU at all, and the early-outs that drop them.

use super::fixtures::*;
use crate::gpu_harness::{headless_device, readback, readback_r16, render_to_texture};
use crate::*;

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
    let reference_src =
        SHADER_SRC.replace("const EARLY_OUT: bool = true;", "const EARLY_OUT: bool = false;");
    assert_ne!(
        reference_src, SHADER_SRC,
        "the EARLY_OUT switch was renamed; this test is no longer comparing anything",
    );

    // The audio ring on, which has two early-outs of its own: the annulus
    // skip inside `spectral_ring`, and the idle branch's radial exception,
    // which keeps an otherwise idle node's fragments where the ring is.
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
        paint.color_levels.clone_from(&paint.levels);
        // ...and a node with a ring and nothing else, which is the idle
        // branch's new case: no activation, no marks, and an annulus to draw.
        let mut silent = scene.nodes[0];
        silent.activation = 0.0;
        silent.octaves = [0.0; harmonigraph_scene::OCTAVE_SLOTS];
        silent.melody_slots = 0;
        silent.bass_slots = 0;
        silent.melody_level = 0.0;
        silent.bass_level = 0.0;
        silent.world_pos.x += 0.9;
        scene.nodes.push(silent);
        scene
    };
    // A WIDER Shadow than the fixture's own, which is what puts `paint_reach`'s
    // skip under load: the quad grows with the blur's reach, so most of a
    // fragment's neighbours are outside every layer the node paints and are
    // carrying the shadow alone. At the Shadow `parity_scene` ships that ring of
    // fragments is thin; a Shadow this wide makes it most of the quad.
    let wide_shadow = || {
        let mut scene = parity_scene();
        for style in scene.shadow.groups_mut() {
            style.width = 0.6;
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
    // The one fixture whose Glow block reaches the shader with anything in it.
    // `from_scene` zeroes the whole block with the light off, so on every
    // fixture above `glow_wash` reads 0 and a LIT slice takes none of the field
    // — which is one arm of the `mix` in `node_paint`, on the far side of the
    // early-out that decides whether a fragment gets there at all.
    //
    // The light's OWN pass is not compared here, and no longer has anything to
    // compare: the gather has no `EARLY_OUT` branch at all where the billboard
    // had one. A node with no light is not in `glow_nodes` to begin with, and
    // past a halo's span `glow_layer` returns zero by the arithmetic either
    // path would run.
    let lit_field = || {
        let mut scene = wide_shadow();
        scene.glow_reach = 0.8;
        // One node with INK and no light of its own, which is what the cull
        // ships for its ink alone (`paints`): it draws its layers, takes the
        // wash under them, and emits nothing.
        let mut dark = scene.nodes[0];
        dark.glow.level = 0.0;
        dark.glow.row = scene.glow_rows;
        dark.world_pos.x -= 0.9;
        scene.glow_rows += 1;
        scene.nodes.push(dark);
        scene
    };
    // No all-idle fixture: an idle node paints nothing, so the cull ships
    // none of them and the comparison would be two empty images. What the
    // idle branch does is now pinned by
    // `a_silent_lattice_ships_no_nodes_and_still_draws_its_markers` instead,
    // on the CPU side where the decision actually lives.
    // The wide Shadow again on the DISTANCE row, which is the only way the
    // kind branch reaches this comparison at all. `shadow_kernel` walks its
    // terms inside the same fragment the early-outs guard, so a skip sized off
    // a blur's reach and a row that reaches further is a shadow clipped flat in
    // the fast pipeline alone — and #508's finding 3 is that `fs_node_cell` has
    // an early-out of its own, which every fixture above compiles on one row.
    let distance = || {
        let mut scene = wide_shadow();
        for style in scene.shadow.groups_mut() {
            style.kernel = harmonigraph_scene::ShadowKernel::Distance;
        }
        scene
    };
    for (name, scene) in [
        ("lit", parity_scene()),
        ("ringing", ringing()),
        ("folded", folded()),
        ("a wide shadow", wide_shadow()),
        ("a lit field", lit_field()),
        ("a distance row", distance()),
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
        let layouts = SceneLayouts {
            uniforms: &res.compiled.bind_group_layout,
            glow: &res.compiled.filter_layout,
            shadow: &res.compiled.shadow_layout,
            casters: &res.compiled.caster_layout,
        };
        let build = |src: &str| {
            let shader = lattice_module(&device, &with_common(src));
            create_pipelines(&device, &shader, format, layouts, false)
        };
        let (fast, _) = build(SHADER_SRC);
        let (slow, _) = build(&reference_src);
        // The light at group 1: one colour over the whole frame, bound to both
        // pipelines, so the wash reads the same thing back whichever is drawing
        // and what they differ by is the early-outs alone. A constant rather
        // than the 1x1 stand-in because `node_paint` reads its Wash OUT of
        // this, and a transparent nothing leaves that term at zero on either
        // pipeline — which takes the whole of what the ink does with the light
        // out of the comparison. Premultiplied, as the real target is, and well
        // under opaque so the ground still shows through it.
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
            device.create_bind_group(&wgpu::BindGroupDescriptor {
                label: Some("parity_light_bind_group"),
                layout: &res.compiled.filter_layout,
                entries: &[
                    wgpu::BindGroupEntry {
                        binding: 0,
                        resource: wgpu::BindingResource::TextureView(&view),
                    },
                    wgpu::BindGroupEntry {
                        binding: 1,
                        resource: wgpu::BindingResource::Sampler(&res.compiled.sampler),
                    },
                ],
            })
        };
        let light = &light;
        let pane = res.panes.get(&11).expect("prepare created the pane");
        let cells = pane
            .offscreen
            .as_ref()
            .and_then(|o| o.shadow.as_ref())
            .map_or(&res.compiled.shadow_dummy_bind_group, |a| a.read());

        let clear = wgpu::Color { r: 0.07, g: 0.08, b: 0.09, a: 1.0 };
        let draw = |pipeline: &wgpu::RenderPipeline| {
            let texture = render_to_texture(&device, &queue, SIZE, format, clear, |pass| {
                pass.set_pipeline(pipeline);
                pass.set_bind_group(0, &pane.bind_group, &[]);
                pass.set_bind_group(1, light, &[]);
                // The atlas `prepare` filled above, so the shadow each node
                // multiplies the frame by is in the comparison: the fragments
                // outside a node's ink are exactly the ones its early-out
                // decides, and they are the ones the shadow lives on.
                pass.set_bind_group(2, cells, &[]);
                // And every caster's kernel at group 3, which is where the
                // shadow this comparison is taken over is read from.
                pass.set_bind_group(3, &pane.caster_bind_group, &[]);
                pass.set_vertex_buffer(0, pane.instance_buffer.slice(..));
                pass.set_vertex_buffer(1, pane.node_cell_buffer.slice(..));
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

        let differing =
            with_early_out.iter().zip(&without).enumerate().find(|(_, (&a, &b))| a != b);
        assert!(
            differing.is_none(),
            "the {name} scene changed when the early-outs were enabled: byte {:?}",
            differing.map(|(i, (a, b))| (i, *a, *b)),
        );

        // #508's third finding lives in the CELL shader, not either scene
        // attachment. This fixture reaches its branch with a real angular gap,
        // a marked sector, and diagonal sector edges; rendering its analytic
        // distance atlas through both compiled switches makes every texel of
        // that geometry part of the parity claim.
        if name == "a distance row" {
            let atlas_size = pane
                .offscreen
                .as_ref()
                .and_then(|o| o.shadow.as_ref())
                .map(|a| a.size)
                .expect("the distance fixture packed an atlas");
            assert!(scene.octave_gap > 0.0, "the fixture has no angular gap");
            assert!(
                scene.nodes.iter().any(|n| n.melody_slots | n.bass_slots != 0),
                "the fixture has no marked sector",
            );
            let draw_cells = |src: &str| {
                let shader = lattice_module(&device, &with_common(src));
                let (pipeline, _) =
                    create_cell_pipelines(&device, &shader, &res.compiled.bind_group_layout);
                let target = render_to_texture(
                    &device,
                    &queue,
                    atlas_size,
                    shadow::ATLAS_FORMAT,
                    wgpu::Color::TRANSPARENT,
                    |pass| {
                        pass.set_pipeline(&pipeline);
                        pass.set_bind_group(0, &pane.bind_group, &[]);
                        pass.set_vertex_buffer(0, pane.instance_buffer.slice(..));
                        pass.set_vertex_buffer(1, pane.node_cell_buffer.slice(..));
                        pass.draw(0..4, 0..pane.instance_count);
                    },
                );
                readback_r16(&device, &queue, &target, atlas_size)
            };
            let cell_fast = draw_cells(SHADER_SRC);
            let cell_slow = draw_cells(&reference_src);
            assert!(
                cell_slow.chunks_exact(2).any(|p| {
                    let bits = u16::from_le_bytes([p[0], p[1]]);
                    bits & 0x8000 != 0 && bits != 0x8000
                }),
                "the distance fixture wrote no negative node interior",
            );
            let differing = cell_fast
                .chunks_exact(2)
                .zip(cell_slow.chunks_exact(2))
                .enumerate()
                .find(|(_, (a, b))| a != b)
                .map(|(i, (a, b))| {
                    (i, u16::from_le_bytes([a[0], a[1]]), u16::from_le_bytes([b[0], b[1]]))
                });
            assert!(
                differing.is_none(),
                "the node cell changed when the early-outs were enabled: texel {differing:?}",
            );
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

fn ringing_view() -> harmonigraph_scene::ViewConfig {
    harmonigraph_scene::ViewConfig {
        // These tests exercise the analyzer ring's shipping and paint paths,
        // so the fixture gives that layer a radial slot explicitly.
        spectral_ring_width: 0.1,
        ..Default::default()
    }
}

/// The ring's floor colour is a picture on every node, so the ring being on
/// is itself a reason to ship an instance: the cull that drops idle nodes
/// keeps all of them the moment the annulus is real. Nothing else reaches
/// that term — the pixel tests above light their nodes, which passes the
/// cull on activation instead.
#[test]
fn an_open_ring_ships_every_idle_node() {
    let view = ringing_view();
    let mut scene = idle_scene();
    assert!(!scene.nodes.is_empty(), "the fixture has to carry idle nodes");
    (scene.spectral.inner, scene.spectral.outer) = view.rings().audio;
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
    let view = ringing_view();
    let mut scene = idle_scene();
    assert!(!scene.nodes.is_empty(), "the fixture has to carry idle nodes");
    (scene.spectral.inner, scene.spectral.outer) = view.rings().audio;
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
        let view = ringing_view();
        let mut scene = single_marked_node(0, 0);
        let node = &mut scene.nodes[0];
        // Nothing held, so what is on screen is the ring alone: the band's own
        // wedges are the MIDI picture and would sum into the channels below.
        node.octaves = [0.0; harmonigraph_scene::OCTAVE_SLOTS];
        node.activation = 1.0;
        let rings = view.rings();
        scene.outer_inner = rings.band.0;
        scene.outer_outer = rings.band.1;
        scene.rings_outer = rings.outer;
        scene.octave_gap = view.octave_gap_width();
        let mut paint = harmonigraph_scene::SpectralPaint::silent();
        (paint.inner, paint.outer) = rings.audio;
        paint.folded = true;
        // Keep the analyzer gate and volume-color ramp at one level, so every
        // wedge whose octave the axis reaches reads the same entry. Off the
        // axis `spectrum_color_at` answers 0 whatever the grid holds, which this
        // wheel stays clear of.
        paint.levels = Box::new([level; harmonigraph_scene::SPECTRAL_BUCKETS]);
        paint.color_levels = Box::new([level; harmonigraph_scene::SPECTRAL_BUCKETS]);
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
         indexing the volume-color ramp at the wedge's own level",
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
        assert_eq!(ships(&spread), 1, "octaves {a} and {b} held at one level keep their node",);
    }
}

/// A node culled behind the home sheet moves no marker: the markers still go
/// over the sheets behind home and under the home sheet itself.
///
/// The argument for that placement is in `from_scene`: put the markers under
/// everything and a node on a sheet behind the home one is drawn over them,
/// which is the layer they are supposed to be hidden BY standing in front of
/// them. What makes it worth a test is the CULL — a node that paints nothing
/// ships no instance, so any expression of the placement that counts nodes has
/// to count the ones that ship rather than the ones the scene held, and the two
/// part company at the first idle node.
///
/// One lit node behind home and one on it, with an idle node behind home
/// between them.
#[test]
fn a_culled_node_behind_home_moves_no_marker() {
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
        call.draws.first(),
        Some(&Draw::Nodes(0, 1)),
        "the one sheet-behind node that ships draws first: {:?}",
        call.draws,
    );
    assert!(
        matches!(call.draws.get(1), Some(Draw::Pluses(..))),
        "and the markers next — the idle node between them shipped nothing to move: {:?}",
        call.draws,
    );
}

/// A marker standing NEARER the eye than a node covers that node, and one
/// standing behind it does not.
///
/// This is the whole of what putting the markers in the depth walk buys, and it
/// is invisible under every camera anyone checks first. A node and a marker are
/// both camera-facing billboards at a fixed world size while the sheet they
/// stand on foreshortens, so face-on a node's disc reaches about its own cell
/// and the only cross under it is its own — every other cross is clear of it,
/// and drawing the whole field under the whole sheet is a picture no pixel can
/// tell from this one. Tilt the sheet and one disc spans a dozen positions
/// while the billboard does not shrink with them, so a batched field puts every
/// cross it covers behind a node that is in front of half of them.
///
/// Two home nodes, one at each end of the tilted sheet, each with its own cross
/// and only the FAR one lit — so the near node ships no instance of its own and
/// the only thing that can order its cross against the far node's ink is the
/// walk. Read off the order rather than off pixels: what the picture does with
/// a marker over a disc is the marker shader's business and is measured with
/// the rest of it, while what this is about is which of the two goes down last.
#[test]
fn a_marker_nearer_the_eye_than_a_node_draws_after_it() {
    let mut scene = idle_scene();
    scene.nodes.truncate(2);
    // Nearly edge-on, which is the regime the order shows in at all.
    scene.camera = harmonigraph_scene::Camera {
        projection: harmonigraph_scene::Projection::Perspective,
        yaw: 0.0,
        pitch: 1.4,
        ..Default::default()
    };
    // Both on the home sheet and both marked, at their own lattice positions so
    // each claims its own cross. Apart along Y, which the tilt turns into
    // depth: the pitch is positive, so the eye is above the sheet looking down
    // and +Y is the NEAR end of it. The lit node goes at the far end.
    let far = harmonigraph_core::LatticePos::new(0, -1, 0);
    let near = harmonigraph_core::LatticePos::new(0, 1, 0);
    for (node, (pos, y, activation)) in
        scene.nodes.iter_mut().zip([(far, -3.0, 1.0), (near, 3.0, 0.0)])
    {
        node.world_pos = glam::Vec3::new(0.0, y, 0.0);
        node.lattice_pos = pos;
        node.on_home = true;
        node.activation = activation;
        node.trail = 0.0;
    }
    scene.pluses = [far, near]
        .into_iter()
        .zip([-3.0f32, 3.0])
        .map(|(lattice_pos, y)| harmonigraph_scene::PlusInstance {
            lattice_pos,
            ..one_marker(glam::Vec3::new(0.0, y, 0.0), 0.2, scene.lattice_ground, 1.0)
        })
        .collect();

    let call = LatticeCallback::from_scene(
        &scene,
        LatticeLabels::default(),
        egui::vec2(256.0, 256.0),
        wgpu::TextureFormat::Rgba8Unorm,
        41,
        None,
    );

    // Non-vacuous three ways: both crosses reached the buffer, the lit node
    // shipped the one instance there is to cover, and neither cross is loose —
    // a loose one keeps the field's old place and would prove nothing.
    assert_eq!(call.pluses.len(), 2, "both crosses ship: {:?}", call.draws);
    assert_eq!(call.instances.len(), 1, "only the lit node ships an instance: {:?}", call.draws);
    assert_eq!(
        call.draws,
        vec![
            // The far node: its own cross, then its ink over it.
            Draw::Pluses(0, 1),
            Draw::Nodes(0, 1),
            // Then the near position, which draws nothing but its cross — over
            // the node behind it, which is the point.
            Draw::Pluses(1, 2),
        ],
        "the near cross draws after the far node's ink, and the far cross before it",
    );
}

/// A resting lattice is still ONE marker draw, however many crosses it holds.
///
/// What the depth walk costs is a break in the marker run at every home node
/// that ships something to break it with, and an idle position ships nothing —
/// so the field a still lattice draws coalesces exactly as the single batch it
/// replaced did. The cost is bounded by the SOUNDING nodes, which is the same
/// number the pass already pays an ink draw each for.
#[test]
fn a_resting_lattice_ships_one_marker_draw() {
    let mut scene = idle_scene();
    // Nothing sounding and no trail either: every node culled, and every one of
    // them marked at its own lattice position.
    for (i, node) in scene.nodes.iter_mut().enumerate() {
        node.trail = 0.0;
        node.on_home = true;
        node.lattice_pos = harmonigraph_core::LatticePos::new(i as i32, 0, 0);
        node.world_pos = glam::Vec3::new(i as f32, 0.0, 0.0);
    }
    scene.pluses = scene
        .nodes
        .iter()
        .map(|n| harmonigraph_scene::PlusInstance {
            lattice_pos: n.lattice_pos,
            ..one_marker(n.world_pos, 0.2, scene.lattice_ground, 1.0)
        })
        .collect();

    let call = LatticeCallback::from_scene(
        &scene,
        LatticeLabels::default(),
        egui::vec2(256.0, 256.0),
        wgpu::TextureFormat::Rgba8Unorm,
        42,
        None,
    );

    assert!(call.pluses.len() > 4, "the fixture needs a field to coalesce: {:?}", call.draws);
    assert_eq!(call.instances.len(), 0, "an idle lattice ships no node");
    assert_eq!(
        call.draws,
        vec![Draw::Pluses(0, call.pluses.len() as u32)],
        "every cross is one run, the same batch the field used to be",
    );
}

/// The light's own list is the billboard pass's set less the two drops
/// `glow_nodes` makes on the CPU, and the one under test here is the DEPTH
/// clip: a node the frustum excludes in depth had its whole quad clipped, every
/// corner sharing the one depth, so it lit nothing. The steeply pitched
/// perspective fixture is the one that holds such a node, a lattice corner in
/// front of the near plane, so it is the fixture that reaches the drop at all.
///
/// And the map on every node that does ship is checked against the billboard
/// it stands in for: the pixel `node_vertex` put uv (1, 0) at has to invert
/// back to (1, 0), and (0, 1) likewise. That is the check a transposed or
/// mis-signed adjugate fails, which the goldens cannot see — the halo stays
/// round either way and only its colour turns.
#[test]
fn the_lit_node_list_is_the_billboard_set_under_the_billboard_map() {
    const SIZE: [u32; 2] = [256, 256];
    let shot = super::golden::names_overlapping_on_one_sheet();
    let call = LatticeCallback::from_scene(
        &shot.scene,
        LatticeLabels::default(),
        egui::vec2(SIZE[0] as f32, SIZE[1] as f32),
        wgpu::TextureFormat::Rgba8Unorm,
        7,
        None,
    );
    let view_proj = glam::Mat4::from_cols_array_2d(&call.uniforms.camera.view_proj.0.map(|c| c.0));
    let pixels = glam::vec2(SIZE[0] as f32, SIZE[1] as f32);
    let axis = |v: Float4| glam::Vec3::new(v.0[0], v.0[1], v.0[2]);
    let (right, up) = (axis(call.uniforms.camera.right), axis(call.uniforms.camera.up));

    let lit: Vec<&GpuInstance> = call.instances.iter().filter(|i| i.glow[0] > 0.0).collect();
    assert!(lit.len() > 1, "the fixture lights nothing; the list is vacuous");
    let placed = |inst: &GpuInstance| {
        project_onto(&view_proj, pixels, glam::Vec3::from(inst.world_pos))
            .filter(|(_, depth)| (0.0..=1.0).contains(depth))
    };
    let dropped = lit.iter().filter(|inst| placed(inst).is_none()).count();
    assert!(
        dropped > 0,
        "the fixture lost its node in front of the near plane; the drop is unreached"
    );

    // The gather's OTHER drop, which this fixture reaches in bulk: a lattice
    // this wide runs far off a 256px pane, and a halo that cannot touch the
    // target is dropped on the CPU rather than guarded at every pixel
    // (`a_lit_node_whose_halo_misses_the_pane_is_not_shipped` is that claim's
    // own test). Asked of the production bound rather than re-derived, because
    // what this test is pinning is the near-plane drop above: a node 0.08 in
    // front of the eye has an enormous frame and would sail through the cull,
    // so the equality below is what fails if the depth filter goes.
    let reaches = |inst: &GpuInstance| -> bool {
        let Some((centre, _)) = placed(inst) else {
            return false;
        };
        let at = glam::Vec3::from(inst.world_pos);
        let uv_world = call.uniforms.node.radius * 1.8 * inst.scale.max(0.05);
        let axis_px = |a: glam::Vec3| {
            project_onto(&view_proj, pixels, at + a * uv_world).map(|p| p.0 - centre)
        };
        let (Some(r), Some(u)) = (axis_px(right), axis_px(up)) else {
            return false;
        };
        // Squared on both sides, as `glow_nodes` compares them: a node landing
        // on the bound would otherwise be kept here and dropped there, or the
        // other way about, on a rounding neither side is asking about.
        centre.clamp(glam::Vec2::ZERO, pixels).distance_squared(centre)
            <= halo_pixels(&call.uniforms, r, u).powi(2)
    };
    let kept: Vec<&&GpuInstance> = lit.iter().filter(|inst| reaches(inst)).collect();
    assert!(
        kept.len() < lit.len() - dropped,
        "every lit node's halo reaches this pane; the off-pane cull is unreached",
    );

    let nodes = call.glow_nodes(SIZE);
    assert_eq!(
        nodes.len(),
        kept.len(),
        "the list is the lit set less the clipped and the off-pane",
    );

    // Instance order on both sides, so the two zip.
    for (inst, node) in kept.iter().zip(&nodes) {
        let at = glam::Vec3::from(inst.world_pos);
        let uv_world = call.uniforms.node.radius * 1.8 * inst.scale.max(0.05);
        let centre = glam::Vec2::from(node.centre);
        let uv_of = |corner: glam::Vec3| {
            let (px, _) = project_onto(&view_proj, pixels, corner).expect("a placed corner");
            let d = px - centre;
            glam::vec2(glam::Vec2::from(node.inv_x).dot(d), glam::Vec2::from(node.inv_y).dot(d))
        };
        let (r, u) = (uv_of(at + right * uv_world), uv_of(at + up * uv_world));
        assert!(r.abs_diff_eq(glam::vec2(1.0, 0.0), 1e-3), "right corner inverts to {r}");
        assert!(u.abs_diff_eq(glam::vec2(0.0, 1.0), 1e-3), "up corner inverts to {u}");
    }
}

/// A lit node whose halo cannot touch the pane is not shipped to the gather,
/// and one whose halo can is — however far off the pane its own centre sits.
///
/// The billboard pass paid NOTHING for a node the rasterizer never covered a
/// pixel of; the gather pays its per-node guard at every pixel of the frame for
/// every entry in the list, and the reachable length of that list is
/// `glow_fade::MAX_ROWS`. Without the cull, zooming IN — which carries nodes off
/// the pane and leaves fewer of them on it — makes the light more expensive,
/// which is backwards. `halo_pixels` is the bound the cull is taken against.
///
/// One roaming node beside a node at the frame's centre, at three placements,
/// all of them clear of the pane by more than the node's own ink:
///
/// - close enough for the halo to cross the pane's edge,
/// - past the bound,
/// - and further still.
///
/// The first ships and the other two do not, which is the claim. What keeps
/// that from being a claim about `halo_pixels` talking to itself is the pair of
/// SHOTS: the near placement has to change the picture, since its light is on
/// the pane and dropping it would be a hole, and the two far placements have to
/// draw the same frame as each other, which is what says the light past the
/// bound is nothing rather than merely dim.
///
/// The Shadow is off in the fixture. It is the one other layer a node this far
/// out could reach the pane with, and it would answer for the light in both
/// shots.
#[test]
fn a_lit_node_whose_halo_misses_the_pane_is_not_shipped() {
    const SIZE: [u32; 2] = [256, 256];
    let pane = egui::vec2(SIZE[0] as f32, SIZE[1] as f32);
    // The pane's own scale, read off the projection rather than restated. World
    // x runs along the screen's x alone under this fixture's camera, so a
    // placement below is a pixel column and the reader can size it against the
    // 256 the pane is wide.
    let scale = {
        let scene = single_marked_node(0, 0);
        let origin = on_screen(&scene, SIZE, glam::Vec3::ZERO);
        (origin.x, on_screen(&scene, SIZE, glam::Vec3::X).x - origin.x)
    };
    let at = |centre_x: f32| -> Scene {
        let mut scene = single_marked_node(0, 0);
        scene.glow_reach = 0.8;
        scene.glow_strength = 1.5;
        scene.shadow = one_shadow(0.0, 0.0, harmonigraph_scene::ShadowKernel::Gaussian);
        let mut roamer = scene.nodes[0];
        roamer.world_pos = glam::Vec3::new((centre_x - scale.0) / scale.1, 0.0, 0.0);
        roamer.lattice_pos = harmonigraph_core::LatticePos::new(1, 0, 0);
        scene.nodes.push(roamer);
        rows_per_node(&mut scene);
        scene
    };
    // The node's ink stops 41px out and its halo 81px, against a bound of 92:
    // 328 is off the pane with light on it — 464 pixels of it, columns 247 to
    // 255 — and 380 and 420 are past every one of those numbers. The two shots
    // below are what hold the fixture to that reading rather than this comment.
    const NEAR: f32 = 328.0;
    const FAR: f32 = 380.0;
    const FURTHER: f32 = 420.0;
    let shipped = |centre_x: f32| -> usize {
        let call = LatticeCallback::from_scene(
            &at(centre_x),
            LatticeLabels::default(),
            pane,
            wgpu::TextureFormat::Rgba8Unorm,
            11,
            None,
        );
        assert_eq!(
            call.instances.iter().filter(|i| i.glow[0] > 0.0).count(),
            2,
            "the fixture must light both nodes at {centre_x}, or there is nothing to cull",
        );
        call.glow_nodes(SIZE).len()
    };
    assert_eq!(shipped(NEAR), 2, "a halo that crosses the pane's edge must be gathered");
    assert_eq!(shipped(FAR), 1, "a halo that stops short of the pane must not be");
    assert_eq!(shipped(FURTHER), 1, "nor one further out still");

    let Some(mut shooter) = Shooter::new(SIZE) else {
        return;
    };
    let (near, far, further) =
        (shooter.shot(&at(NEAR)), shooter.shot(&at(FAR)), shooter.shot(&at(FURTHER)));
    assert_eq!(
        differing_pixels(&far, &further),
        0,
        "the light past the bound is not nothing; the cull drops a node that draws",
    );
    assert!(
        differing_pixels(&near, &far) > 0,
        "the near placement lights no pixel either; it is not the branch it claims",
    );
}
