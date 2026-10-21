//! Bloom, and the order the sheets reach the frame in.

use super::fixtures::*;
use crate::*;

/// Fractional strength must change the halo independently of AA/render scale.
/// Read unsaturated pixels outside the original ink so clipping cannot hide a
/// swapped control, and compare both scales at the same screen resolution.
///
/// Outside the ink at EVERY scale: a supersampled node composites a pixel of
/// antialiased rim the scale-1 frame leaves black, right where the halo is
/// brightest. Masked per scale, that rim alone read 15% less halo at scale 2,
/// which a bright pass taking one tap of every 4x4 box hid by keeping more
/// light through the threshold's knee than the box average does.
#[test]
fn fractional_bloom_strength_is_independent_of_render_scale() {
    let Some(mut shooter) = Shooter::new([256, 256]) else { return };
    let mut scene = single_marked_node(0, 0);
    scene.view.glow_reach = 0.0;
    scene.pluses.clear();
    scene.nodes[0].octaves.fill(1.0);
    scene.pitch_lut.fill(glam::Vec4::new(0.7, 0.5, 0.3, 1.0));
    let scales = [1.0, 2.0];
    scene.view.note_bloom = 0.0;
    let plains: Vec<Vec<u8>> = scales
        .iter()
        .map(|&scale| {
            scene.view.render_scale = scale;
            shooter.shot(&scene)
        })
        .collect();
    let outside: Vec<bool> = (0..plains[0].len() / 4)
        .map(|i| plains.iter().all(|plain| plain[4 * i..4 * i + 3] == [0, 0, 0]))
        .collect();
    let mut scale_totals = Vec::new();
    for scale in scales {
        scene.view.render_scale = scale;
        let mut halos = Vec::new();
        for strength in [0.25, 0.75] {
            scene.view.note_bloom = strength;
            let frame = shooter.shot(&scene);
            let halo: u64 = outside
                .iter()
                .zip(frame.as_chunks::<4>().0)
                .filter(|(outside, _)| **outside)
                .map(|(_, b)| b[..3].iter().map(|v| u64::from(*v)).sum::<u64>())
                .sum();
            halos.push(halo);
        }
        assert!(halos[0] > 500, "fixture needs a measurable halo: {halos:?}");
        let ratio = halos[1] as f64 / halos[0] as f64;
        assert!((2.8..3.2).contains(&ratio), "scale {scale}: halo ratio {ratio}, {halos:?}");
        eprintln!("fractional bloom scale {scale}: halo sums {halos:?}");
        scale_totals.push(halos[1]);
    }
    let ratio = scale_totals[1] as f64 / scale_totals[0] as f64;
    assert!((0.85..1.15).contains(&ratio), "render scale changes halo strength: {scale_totals:?}");
}

/// The bright pass hands the blur the same light from a one-pixel line
/// wherever the line stands, when the scene is not exactly twice the half.
///
/// One bilinear tap per half texel is a 2x2 box only at exactly 2:1. A 254 px
/// pane (whose half is 128, #1495) or a render scale of 1.5 moves the taps off
/// the scene's texels, and a line between two of them reached the half at 2%
/// of its light where one under a tap reached it at 100%.
///
/// Each pair of rows is one case, the line one column further along than in
/// the pair above, so one frame sweeps the line across every phase of the
/// grid. The rows themselves stay exactly 2:1, so no case leaks into the next.
#[test]
fn a_thin_lines_light_reaches_the_half_whatever_the_scene_to_half_ratio() {
    let Some((device, queue)) = crate::gpu_harness::headless_device() else { return };
    let compiled = CompiledLatticeResources::new(&device, wgpu::TextureFormat::Rgba8Unorm);
    const HEIGHT: u32 = 256;
    // A pane 2 mod 4 wide at render scale 1, and an aligned one at 1.5.
    for (screen_width, scene_width) in [(254u32, 254u32), (256, 384)] {
        let screen = [screen_width, HEIGHT];
        let ([hw, hh], _) = bloom_sizes(screen);
        let ratio = scene_width as f32 / hw as f32;
        assert!(ratio != 2.0, "the fixture must leave 2:1 across: {ratio}");
        assert_eq!(HEIGHT, 2 * hh, "and hold it down, or the cases leak into each other");
        // 1024 in f16: bright enough that the threshold's knee keeps all of a
        // box holding even a sliver of the line.
        let (lit, opaque) = (0x6400u16, 0x3c00u16);
        let mut texels = vec![[0u16; 4]; (scene_width * HEIGHT) as usize];
        for row in 0..HEIGHT {
            texels[(row * scene_width + 8 + row / 2) as usize] = [lit, lit, lit, opaque];
        }
        let source = crate::gpu_memory::texture(
            &device,
            &wgpu::TextureDescriptor {
                label: Some("bright_pass_line"),
                size: wgpu::Extent3d {
                    width: scene_width,
                    height: HEIGHT,
                    depth_or_array_layers: 1,
                },
                mip_level_count: 1,
                sample_count: 1,
                dimension: wgpu::TextureDimension::D2,
                format: LATTICE_COLOR_FORMAT,
                usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
                view_formats: &[],
            },
        );
        queue.write_texture(
            source.as_image_copy(),
            bytemuck::cast_slice(&texels),
            wgpu::TexelCopyBufferLayout {
                offset: 0,
                bytes_per_row: Some(scene_width * 8),
                rows_per_image: None,
            },
            source.size(),
        );
        let chain = BloomChain::new(
            &device,
            "bright_pass_line",
            LATTICE_COLOR_FORMAT,
            &compiled.filter_layout,
            &compiled.sampler,
            &source.create_view(&Default::default()),
            screen,
        );
        let mut encoder = device.create_command_encoder(&Default::default());
        let pipelines = BloomPipelines {
            bright: &compiled.bright_pipeline,
            downsample: &compiled.downsample_pipeline,
            blur_h: &compiled.blur_h_pipeline,
            blur_v: &compiled.blur_v_pipeline,
        };
        chain.run(&mut encoder, pipelines, "bright_pass_line", None);
        queue.submit([encoder.finish()]);

        // The half as it stands, in f32 so nothing clips.
        let format = wgpu::TextureFormat::Rgba32Float;
        let blit = create_post_pipeline(
            &device,
            &blit_module(&device),
            "fs_blit",
            format,
            &compiled.filter_layout,
            None,
        );
        let half = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("bright_pass_half"),
            layout: &compiled.filter_layout,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: wgpu::BindingResource::TextureView(&chain.half_view),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: wgpu::BindingResource::Sampler(&compiled.sampler),
                },
            ],
        });
        let texture = crate::gpu_harness::render_to_texture(
            &device,
            &queue,
            [hw, hh],
            format,
            wgpu::Color::TRANSPARENT,
            |pass| {
                pass.set_pipeline(&blit);
                pass.set_bind_group(0, &half, &[]);
                pass.draw(0..4, 0..1);
            },
        );
        let bytes = crate::gpu_harness::readback(&device, &queue, &texture, [hw, hh]);
        let red: Vec<f32> = bytes
            .as_chunks::<16>()
            .0
            .iter()
            .map(|texel| f32::from_le_bytes(texel[..4].try_into().unwrap()))
            .collect();
        // A one-texel line is 1/ratio of one half texel's box, wherever it
        // falls among them.
        let expected = 1024.0 / ratio;
        let light: Vec<f32> =
            red.chunks_exact(hw as usize).map(|row| row.iter().sum::<f32>() / expected).collect();
        let (lo, hi) =
            light.iter().fold((f32::MAX, f32::MIN), |(lo, hi), &l| (lo.min(l), hi.max(l)));
        assert!(
            lo > 0.98 && hi < 1.02,
            "{screen_width} px pane, {scene_width} px scene: the line reached the half at \
             {lo:.3}..{hi:.3} of its light across the grid",
        );
    }
}

/// A sheet in FRONT of the home sheet is drawn over it; a sheet BEHIND it
/// is drawn under. Both directions matter, and only one of them is obvious:
/// forcing the home sheet to the bottom (so an off-sheet note could never be
/// hidden by it) inverts the far half of the axis, and since a node is drawn
/// over — and casts over — whatever came before it, the sheet behind then
/// stands on top of the home sheet in front of it.
#[test]
fn sheets_draw_back_to_front_along_the_sevens_axis() {
    use harmonigraph_scene::{Camera, FrameParams, Projection, ViewConfig};

    let view = ViewConfig {
        extent_threes: 1,
        extent_fives: 1,
        min_sevens: -2,
        max_sevens: 2,
        ..ViewConfig::default()
    };
    for projection in [Projection::Cabinet, Projection::Perspective, Projection::Orthographic] {
        let mut scene = animated_snapshot(
            &harmonigraph_core::NoteTracker::new(),
            &harmonigraph_core::Tuning::default(),
            &view,
            &view.reach(),
            &FrameParams::default(),
            // Orbited, deliberately: this is the case a plain depth sort
            // gets wrong, because two nodes on one sheet then sit at
            // different depths and the sheets interleave.
            Camera { projection, ..Camera::default() },
            None,
            0.0,
        );
        // Every position SOUNDING. Set here rather than played in, because
        // what this test needs is one node per position on every sheet, and
        // which nodes a tracker lights is a question about tuning.
        //
        // Sounding is the only way to get one: an idle node paints nothing at
        // all now, so the cull drops it, and a scene of idle nodes would leave
        // the order below comparing an empty list with itself.
        for node in &mut scene.nodes {
            node.activation = 1.0;
        }
        let call = LatticeCallback::from_scene(
            &scene,
            LatticeLabels::default(),
            egui::vec2(800.0, 600.0),
            wgpu::TextureFormat::Bgra8Unorm,
            0,
            // No stats slot: this is about draw ORDER, not about timing.
            None,
        );
        // World z IS the sevens axis (see lattice_to_world), so the draw
        // order must run from the most negative sheet to the most positive
        // — and it has to hold under EVERY projection, not only the face-on
        // one. When it doesn't, the sheets interleave and every shadow in the
        // frame is cast in the wrong order, an item multiplying only what the
        // walk happened to put under it.
        let depths: Vec<f32> = call.instances.iter().map(|i| i.world_pos[2]).collect();
        // Several SHEETS, not several nodes. A node count passes on one
        // sheet's worth of identical depths, where every pair below holds
        // whatever the sort did — which is what culling the off-sheet nodes
        // reduced this to, silently, while it went on reading as coverage.
        let (lo, hi) =
            depths.iter().fold((f32::MAX, f32::MIN), |(lo, hi), &d| (lo.min(d), hi.max(d)));
        assert!(
            hi - lo > 1e-6,
            "{projection:?}: every node drawn is at one depth ({lo}), so the order \
             below compares a sheet with itself: {depths:?}",
        );
        for pair in depths.windows(2) {
            assert!(
                pair[1] >= pair[0] - 1e-6,
                "{projection:?}: a sheet behind is drawn after one in front: {pair:?}"
            );
        }
    }
}
