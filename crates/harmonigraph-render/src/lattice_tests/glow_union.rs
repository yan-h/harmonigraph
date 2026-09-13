//! Peak-normalized screen is bounded by the fixed full-strength glow, not by
//! either tail at the pixel where notes meet. Read the production glow target
//! directly so rings, shadows, bloom and the background cannot hide a failure.

use super::fixtures::*;
use crate::gpu_harness::{readback, render_to_texture};
use crate::*;

const SIZE: [u32; 2] = [256, 256];
const CENTRE: glam::Vec2 = glam::Vec2::new(128.5, 128.5);

fn scene(levels: &[f32], gain: f32, separated: bool) -> Scene {
    let mut scene = single_marked_node(0, 0);
    scene.glow_reach = 2.88;
    scene.glow_strength = gain;
    scene.glow_curve = harmonigraph_scene::GlowCurve { shape: 0.0 };
    scene.glow_blend = 1.0;
    scene.bloom_strength = 0.0;
    let original = scene.nodes[0];
    scene.nodes = levels
        .iter()
        .enumerate()
        .map(|(i, level)| {
            let mut node = original;
            node.glow.level = *level;
            node.lattice_pos = harmonigraph_core::LatticePos::new(i as i32, 0, 0);
            if separated {
                node.world_pos.x = (i as f32 * 2.0 - 1.0) * scene.node_radius * 1.8;
            }
            node.cents = if i % 2 == 0 { 0.0 } else { 1100.0 };
            node
        })
        .collect();
    rows_per_node(&mut scene);
    for _ in 0..12 {
        scene.camera.pan(CENTRE - on_screen(&scene, SIZE, glam::Vec3::ZERO));
    }
    assert!(on_screen(&scene, SIZE, glam::Vec3::ZERO).distance(CENTRE) < 0.001);
    scene
}

/// Copy the actual half-float glow to bytes with the production plain blit.
/// No scene ink or dither is involved; only the final byte quantization remains.
fn glow(shooter: &mut Shooter, scene: &Scene) -> Vec<u8> {
    shooter.shot(scene);
    read_glow(shooter)
}

fn read_glow(shooter: &Shooter) -> Vec<u8> {
    let resources = shooter.resources.get::<LatticeResources>().unwrap();
    let offscreen = resources.panes[&shooter.pane].offscreen.as_ref().unwrap();
    let Some(glow) = &offscreen.glow else {
        return vec![0; (shooter.size[0] * shooter.size[1] * 4) as usize];
    };
    let shader = blit_module(&shooter.device);
    let pipeline = create_post_pipeline(
        &shooter.device,
        &shader,
        "fs_blit",
        shooter.format,
        &resources.compiled.filter_layout,
        None,
    );
    let texture = render_to_texture(
        &shooter.device,
        &shooter.queue,
        shooter.size,
        shooter.format,
        wgpu::Color::TRANSPARENT,
        |pass| {
            pass.set_pipeline(&pipeline);
            pass.set_bind_group(0, &glow.bind_group, &[]);
            pass.draw(0..4, 0..1);
        },
    );
    readback(&shooter.device, &shooter.queue, &texture, shooter.size)
}

#[test]
fn a_held_nodes_light_breathes_without_advancing_its_ink_history() {
    let Some(mut shooter) = Shooter::new(SIZE) else { return };
    let mut scene = scene(&[1.0], 0.75, false);
    scene.glow_timing =
        Some(harmonigraph_scene::GlowTiming { now: 0.0, attack: 0.3, release: 2.5 });
    let first = glow(&mut shooter, &scene);
    scene.glow_timing.as_mut().unwrap().now = 4.0;
    shooter.shot_again(&scene);
    let later = read_glow(&shooter);
    assert!(first.iter().any(|&v| v > 20), "the held node must light the target");
    assert_ne!(first, later, "a constant held note's halo must breathe");
    // A fresh pane at the same time must agree with the carried pane: breathing
    // is a display modulation, with no accumulated effect on the ink colour.
    assert_eq!(later, glow(&mut shooter, &scene));
    scene.atmosphere.breath_amount = 0.0;
    let steady = glow(&mut shooter, &scene);
    scene.glow_timing.as_mut().unwrap().now = 0.0;
    assert_eq!(steady, glow(&mut shooter, &scene), "zero depth must stop breathing");
    scene.atmosphere.breath_amount = 1.0;
    scene.atmosphere.enabled = false;
    assert_eq!(steady, glow(&mut shooter, &scene), "the master switch includes breathing");
}

#[test]
fn tile_candidates_keep_the_untiled_picture_through_resize_and_reuse() {
    let Some(mut shooter) = Shooter::new([512, 512]) else { return };
    let mut scene = scene(&[1.0; 4], 0.75, true);
    scene.nodes[1].scale = 0.1;
    scene.nodes[2].scale = 0.15;
    // At 512px the large halos use the shared list, while the small pair
    // reaches only a few tiles. Both lists are interleaved in node order.
    for (size, accumulation) in [([512, 512], 0.0), ([576, 257], 0.5), ([256, 256], 1.0)] {
        shooter.size = size;
        scene.glow_accumulation = accumulation;
        shooter.shot_again(&scene);
        let tiled = read_glow(&shooter);
        let extent = egui::vec2(size[0] as f32, size[1] as f32);
        let cb = LatticeCallback::from_scene(
            &scene,
            LatticeLabels::default(),
            extent,
            shooter.format,
            shooter.pane,
            None,
        );
        let nodes = cb.glow_nodes(size);
        let tiles = crate::lattice_node_glow::tiles::pack(&nodes, size);
        if size == [512, 512] {
            let global_count = tiles[2] - tiles[1];
            assert!(global_count > 0 && global_count < nodes.len() as u32);
        }
        let tile_count = size[0].div_ceil(crate::lattice_node_glow::tiles::TILE)
            * size[1].div_ceil(crate::lattice_node_glow::tiles::TILE);
        let start = tile_count + 4;
        let mut reference = vec![start; start as usize];
        reference[0] = tiles[0];
        reference[1] = start;
        reference[2] = start + nodes.len() as u32;
        reference.extend(0..nodes.len() as u32);
        let resources = shooter.resources.get::<LatticeResources>().unwrap();
        let pane = &resources.panes[&shooter.pane];
        assert!(reference.len() <= pane.glow_tile_capacity);
        shooter.queue.write_buffer(&pane.glow_tile_buffer, 0, bytemuck::cast_slice(&reference));
        let glow = pane.offscreen.as_ref().unwrap().glow.as_ref().unwrap();
        let strip = pane.ink_history.as_ref().unwrap();
        let mut encoder = shooter.device.create_command_encoder(&Default::default());
        {
            let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: &glow.view,
                    depth_slice: None,
                    resolve_target: None,
                    ops: wgpu::Operations {
                        load: wgpu::LoadOp::Clear(wgpu::Color::TRANSPARENT),
                        store: wgpu::StoreOp::Store,
                    },
                })],
                ..Default::default()
            });
            pass.set_pipeline(&resources.compiled.glow_gather_pipeline);
            pass.set_bind_group(0, &pane.bind_group, &[]);
            pass.set_bind_group(1, &strip.blurred_bind_group, &[]);
            pass.set_bind_group(2, &pane.glow_node_bind_group, &[]);
            pass.set_bind_group(3, &pane.glow_tile_bind_group, &[]);
            pass.draw(0..4, 0..1);
        }
        shooter.queue.submit([encoder.finish()]);
        assert_eq!(tiled, read_glow(&shooter), "tile lists changed the picture at {size:?}");
    }
}

fn linear(gamma: f64) -> f64 {
    if gamma <= 0.04045 {
        gamma / 12.92
    } else {
        ((gamma + 0.055) / 1.055).powf(2.4)
    }
}

fn luminance(pixel: &[u8]) -> f64 {
    [0.2126, 0.7152, 0.0722]
        .iter()
        .zip(pixel)
        .map(|(weight, byte)| weight * linear(f64::from(*byte) / 255.0))
        .sum()
}

#[test]
fn a_lone_glow_keeps_its_colour_profile_and_fade() {
    let Some(mut shooter) = Shooter::new(SIZE) else { return };
    for (gain, level) in [(0.15, 1.0), (1.0, 0.25), (2.0, 1.0)] {
        let mut scene = scene(&[level], gain, false);
        let colour = [0.8, 0.4, 0.1];
        scene.pitch_lut = [glam::Vec4::new(colour[0], colour[1], colour[2], 1.0);
            harmonigraph_scene::PITCH_LUT_N];
        let pixels = glow(&mut shooter, &scene);
        for accumulation in [0.5, 1.0] {
            scene.glow_accumulation = accumulation;
            assert_eq!(glow(&mut shooter, &scene), pixels, "a lone glow cannot change");
        }
        let per_uv = on_screen(&scene, SIZE, glam::Vec3::X * scene.node_radius * 1.8).x - CENTRE.x;
        let mut visible = 0;
        for (index, pixel) in pixels.chunks_exact(4).enumerate() {
            let at = glam::vec2((index % 256) as f32 + 0.5, (index / 256) as f32 + 0.5);
            let d = at.distance(CENTRE) / per_uv;
            // This fixture has no marks, a rim at 0.795 UV and a linear falloff.
            let a = (0.8 * gain * level * (1.0 - d / (0.795 + 2.88)).max(0.0)).min(1.0);
            for (got, channel) in pixel.iter().zip([colour[0], colour[1], colour[2], 1.0]) {
                assert!(
                    (f32::from(*got) - a * channel * 255.0).abs() < 1.1,
                    "gain {gain}, level {level}, pixel {index}: {pixel:?}, expected coverage {a}"
                );
            }
            visible += usize::from(a > 0.02);
        }
        assert!(visible > 1000, "the fixture must read a visible halo");
    }
}

#[test]
fn different_hues_meet_at_screened_luminance_in_either_order() {
    let Some(mut shooter) = Shooter::new(SIZE) else { return };
    let at = |levels: &[f32]| {
        let mut scene = scene(levels, 0.75, true);
        scene.pitch_lut = std::array::from_fn(|i| {
            if i * 2 < harmonigraph_scene::PITCH_LUT_N {
                glam::Vec4::new(1.0, 0.15, 0.1, 1.0)
            } else {
                glam::Vec4::new(0.1, 1.0, 0.25, 1.0)
            }
        });
        scene
    };
    let left = glow(&mut shooter, &at(&[1.0, 0.0]));
    let right = glow(&mut shooter, &at(&[0.0, 1.0]));
    let mut pair = at(&[1.0, 1.0]);
    let both = glow(&mut shooter, &pair);
    let peak = linear(0.8 * 0.75);
    let mut overlap = 0;
    let mut lift = 0.0f64;
    for ((l, r), b) in left.chunks_exact(4).zip(right.chunks_exact(4)).zip(both.chunks_exact(4)) {
        let (a, c, got) = (luminance(l), luminance(r), luminance(b));
        if a > 0.01 && c > 0.01 {
            let expected = a + c - a * c / peak;
            // Three byte-quantized readings plus the intermediate half-float.
            assert!((got - expected).abs() < 0.006, "{got} != {expected}: {l:?}, {r:?}, {b:?}");
            overlap += 1;
            lift = lift.max(got - a.max(c));
        }
    }
    assert!(overlap > 500, "both coloured halos must reach the measured pixels");
    assert!(lift > 0.02, "the join must rise above the tails, not reproduce a near-max");
    pair.nodes.reverse();
    let reversed = glow(&mut shooter, &pair);
    assert!(both.iter().zip(reversed).all(|(a, b)| a.abs_diff(b) <= 1));
}

#[test]
fn a_dense_saturated_chord_stays_under_the_fixed_peak() {
    let Some(mut shooter) = Shooter::new(SIZE) else { return };
    for gain in [0.25, 1.0, 2.0] {
        let mut scene = scene(&[1.0; 32], gain, false);
        // All red forces the gamut repair: keeping its hue at the screened
        // luminance would push red past the ceiling. A grey fixture misses it.
        scene.pitch_lut = [glam::Vec4::new(1.0, 0.0, 0.0, 1.0); harmonigraph_scene::PITCH_LUT_N];
        let peak = (0.8 * f64::from(gain)).min(1.0);
        let ceiling = linear(peak);
        let pixels = glow(&mut shooter, &scene);
        let mut brightest = 0.0f64;
        for pixel in pixels.chunks_exact(4) {
            let light = luminance(pixel);
            assert!(light <= ceiling + 0.005, "gain {gain}: {pixel:?} exceeds peak {peak}");
            assert!(pixel[..3].iter().all(|channel| *channel <= pixel[3]));
            assert!(f64::from(pixel[3]) <= peak * 255.0 + 1.0);
            brightest = brightest.max(light);
        }
        assert!(brightest > ceiling * 0.95, "the chord must actually approach the ceiling");
        let centre = &pixels[(128 * 256 + 128) * 4..][..4];
        assert!(centre[1] > 20 && centre[2] > 20, "gamut repair must be exercised: {centre:?}");
    }
}

#[test]
fn a_fading_neighbour_returns_to_the_lone_glow_without_moving_the_ceiling() {
    let Some(mut shooter) = Shooter::new(SIZE) else { return };
    let at = |level| {
        let mut scene = scene(&[1.0, level], 0.6, true);
        scene.pitch_lut = [glam::Vec4::ONE; harmonigraph_scene::PITCH_LUT_N];
        scene
    };
    let lone = glow(&mut shooter, &at(0.0));
    let mut previous = glow(&mut shooter, &at(1.0));
    for level in [0.5, 0.1, 0.01, 0.0001, 0.0] {
        let current = glow(&mut shooter, &at(level));
        for ((before, after), single) in
            previous.chunks_exact(4).zip(current.chunks_exact(4)).zip(lone.chunks_exact(4))
        {
            assert!(after[0] <= before[0] + 1);
            assert!(after[0] + 1 >= single[0]);
        }
        if level == 0.0 {
            assert_eq!(current, lone);
        }
        previous = current;
    }
    let mut distant = at(0.0);
    distant.nodes[1].glow.level = 1.0;
    distant.nodes[1].world_pos.x = 10000.0;
    assert_eq!(glow(&mut shooter, &distant), lone, "an unrelated note cannot reset the ceiling");
    distant.glow_strength = 0.0;
    assert!(glow(&mut shooter, &distant).iter().all(|byte| *byte == 0));
}

#[test]
fn accumulation_sweeps_to_the_original_screen_of_each_colour_channel() {
    let Some(mut shooter) = Shooter::new(SIZE) else { return };
    let at = |levels: &[f32], accumulation| {
        let mut scene = scene(levels, 0.35, false);
        scene.glow_accumulation = accumulation;
        scene.pitch_lut = std::array::from_fn(|i| {
            if i * 2 < harmonigraph_scene::PITCH_LUT_N {
                glam::Vec4::new(0.8, 0.5, 0.2, 1.0)
            } else {
                glam::Vec4::new(0.2, 1.0, 0.4, 1.0)
            }
        });
        scene
    };
    let levels = [1.0, 0.65, 0.85];
    let singles: Vec<_> = (0..3)
        .map(|i| {
            let mut alone = [0.0; 3];
            alone[i] = levels[i];
            glow(&mut shooter, &at(&alone, 0.0))
        })
        .collect();
    let bounded = glow(&mut shooter, &at(&levels, 0.0));
    let original = glow(&mut shooter, &at(&levels, 1.0));
    for (i, actual) in original.iter().enumerate() {
        // The old gather screened gamma-encoded premultiplied RGBA, including
        // alpha. Three quantized singles incur at most 1.5 bytes of input error.
        let expected =
            255.0 * (1.0 - singles.iter().map(|s| 1.0 - f64::from(s[i]) / 255.0).product::<f64>());
        assert!((f64::from(*actual) - expected).abs() < 2.1, "byte {i}: {actual} != {expected}");
    }
    let peak = linear(0.8 * 0.35);
    let brightest = original.chunks_exact(4).map(luminance).fold(0.0f64, f64::max);
    assert!(brightest > peak + 0.02, "the old endpoint must actually exceed the fixed ceiling");
    for accumulation in [0.25, 0.5, 0.75] {
        let mut scene = at(&levels, accumulation);
        let mixed = glow(&mut shooter, &scene);
        for ((a, b), actual) in bounded.iter().zip(&original).zip(&mixed) {
            let expected = f32::from(*a) * (1.0 - accumulation) + f32::from(*b) * accumulation;
            assert!(
                (f32::from(*actual) - expected).abs() < 1.5,
                "the sweep must blend the endpoints"
            );
        }
        assert!(mixed.chunks_exact(4).all(|p| p[..3].iter().all(|c| *c <= p[3])));
        scene.nodes.reverse();
        assert!(mixed.iter().zip(glow(&mut shooter, &scene)).all(|(a, b)| a.abs_diff(b) <= 1));
    }
}
