//! Peak-normalized screen is bounded by the fixed full-strength glow, not by
//! either tail at the pixel where notes meet. Read the production glow field
//! with linear reconstruction, excluding rings, shadows, bloom and background.

use super::fixtures::*;
use crate::gpu_harness::{readback, render_to_texture};
use crate::*;

const SIZE: [u32; 2] = [256, 256];
const CENTRE: glam::Vec2 = glam::Vec2::new(128.5, 128.5);

fn scene(levels: &[f32], gain: f32, separated: bool) -> Scene {
    let mut scene = single_marked_node(0, 0);
    scene.view.glow_reach = 2.88;
    scene.view.glow_strength = gain;
    scene.view.glow_curve = harmonigraph_scene::GlowCurve { shape: 0.0 };
    scene.view.glow_blend = 1.0;
    scene.view.note_bloom = 0.0;
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

/// Reconstruct the half-float glow at the requested output size using the
/// production linear blit. No scene ink or dither hides the sampled field.
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
    read_glow_binding(shooter, glow.binding())
}

fn read_glow_binding(shooter: &Shooter, binding: &wgpu::BindGroup) -> Vec<u8> {
    let resources = shooter.resources.get::<LatticeResources>().unwrap();
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
            pass.set_bind_group(0, binding, &[]);
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
    scene.view.atmosphere.breath_amount = 0.0;
    let steady = glow(&mut shooter, &scene);
    scene.glow_timing.as_mut().unwrap().now = 0.0;
    assert_eq!(steady, glow(&mut shooter, &scene), "zero depth must stop breathing");
    scene.view.atmosphere.breath_amount = 1.0;
    scene.view.atmosphere.texture_depth = 0.0;
    scene.glow_timing.as_mut().unwrap().now = 4.0;
    assert_ne!(steady, glow(&mut shooter, &scene), "breathing works with both stages off");
}

#[test]
fn textures_shape_the_combined_light_without_creating_or_recoloring_it() {
    let Some(mut shooter) = Shooter::new(SIZE) else { return };
    for (levels, accumulation) in [(vec![1.0], 0.0), (vec![1.0, 1.0], 0.5), (vec![1.0; 32], 1.0)] {
        let mut scene = scene(&levels, 0.75, levels.len() == 2);
        scene.view.glow_accumulation = accumulation;
        scene.camera = harmonigraph_scene::Camera {
            projection: harmonigraph_scene::Projection::Orthographic,
            distance: 28.0,
            yaw: 0.0,
            pitch: 0.0,
            ..Default::default()
        };
        // The glow target is half-resolution: resolve the clouds rather
        // than testing their subpixel average at this small size.
        scene.view.atmosphere.texture_scale = 4.0;
        scene.view.atmosphere.breath_amount = 0.0;
        scene.glow_timing =
            Some(harmonigraph_scene::GlowTiming { now: 0.0, attack: 0.3, release: 2.5 });
        let smooth = glow(&mut shooter, &scene);
        scene.view.atmosphere.texture_depth = 1.0;
        let textured = glow(&mut shooter, &scene);
        let mut changed = 0;
        let mut empty = 0;
        let mut darkest_ratio = 1.0f32;
        let mut brightest_ratio = 0.0f32;
        for (before, after) in smooth.chunks_exact(4).zip(textured.chunks_exact(4)) {
            if before[3] == 0 {
                assert_eq!(after, before, "texture cannot create light outside a halo");
                empty += 1;
                continue;
            }
            assert!(after.iter().zip(before).all(|(a, b)| a <= b));
            if before[3] > 30 && after[3] > 5 {
                let ratio = f32::from(after[3]) / f32::from(before[3]);
                darkest_ratio = darkest_ratio.min(ratio);
                brightest_ratio = brightest_ratio.max(ratio);
                for c in 0..3 {
                    assert!(
                        (f32::from(after[c]) - f32::from(before[c]) * ratio).abs() < 2.0,
                        "the material mask must preserve the glow's hue"
                    );
                }
                changed += usize::from(before[3] - after[3] > 5);
            }
        }
        assert!(
            changed > 1000 && empty > 1000,
            "measure both a broad halo and unlit ground: changed={changed}, empty={empty}"
        );
        assert!(
                brightest_ratio - darkest_ratio > 0.25,
                "Clouds: texture must vary spatially, not just dim the halo ({darkest_ratio}..{brightest_ratio})"
            );
        scene.glow_timing.as_mut().unwrap().now = 8.0;
        shooter.shot_again(&scene);
        let later = read_glow(&shooter);
        assert_ne!(textured, later, "materials must drift inside a held glow");
        assert_eq!(later, glow(&mut shooter, &scene), "texture cannot depend on history");
        scene.view.atmosphere.texture_speed = 0.0;
        assert_eq!(textured, glow(&mut shooter, &scene), "zero speed freezes the material field");
        scene.view.atmosphere.texture_depth = 0.0;
        assert_eq!(smooth, glow(&mut shooter, &scene), "texture off restores the smooth glow");
    }
}

#[test]
fn material_shadow_pickup_darkens_light_without_adding_coverage() {
    use harmonigraph_scene::{LatticeMaterial, ShadowKernel};
    let Some(mut shooter) = Shooter::new(SIZE) else { return };
    let mut scene = scene(&[1.0, 1.0], 0.75, true);
    scene.view.atmosphere.breath_amount = 0.0;
    scene.view.atmosphere.material_speed = 0.0;
    scene.view.atmosphere.material_settings.wash_size = 2.0;
    scene.view.atmosphere.material_settings.wash_refract = 1.0;
    for kernel in [ShadowKernel::Distance, ShadowKernel::Gaussian] {
        scene.view.shadow = one_shadow(0.8, 0.7, kernel);
        for material in
            [LatticeMaterial::Watercolor, LatticeMaterial::VelvetScales, LatticeMaterial::Stars]
        {
            scene.view.atmosphere.material_style = material;
            scene.view.atmosphere.material_shadow_pickup = 0.0;
            let before = glow(&mut shooter, &scene);
            scene.view.atmosphere.material_shadow_pickup = 1.0;
            shooter.shot_again(&scene);
            let after = read_glow(&shooter);
            let mut changed = 0;
            for (a, b) in before.chunks_exact(4).zip(after.chunks_exact(4)) {
                // A star brighter than its light owns alpha to contain its
                // colour; pigment dims that star and hands back a rounding step.
                let slack = u8::from(material == LatticeMaterial::Stars);
                assert!(
                    a[3].abs_diff(b[3]) <= slack,
                    "shadow pigment must preserve source coverage"
                );
                assert!(b[..3].iter().zip(a).all(|(b, a)| *b <= a.saturating_add(1)));
                changed += usize::from(a[..3].iter().zip(b).any(|(a, b)| a.saturating_sub(*b) > 3));
            }
            assert!(
                changed > 100,
                "fixture must reach source shadows: {kernel:?} {material:?}: {changed}"
            );
            assert_eq!(after, glow(&mut shooter, &scene), "carried pane matches fresh rendering");
        }
    }
}

fn material_source(shooter: &Shooter) -> Vec<u8> {
    let resources = shooter.resources.get::<LatticeResources>().unwrap();
    let view = &resources.panes[&shooter.pane]
        .offscreen
        .as_ref()
        .unwrap()
        .glow
        .as_ref()
        .unwrap()
        .material_source
        .as_ref()
        .unwrap()
        .view;
    let binding = shooter.device.create_bind_group(&wgpu::BindGroupDescriptor {
        label: None,
        layout: &resources.compiled.filter_layout,
        entries: &[
            wgpu::BindGroupEntry { binding: 0, resource: wgpu::BindingResource::TextureView(view) },
            wgpu::BindGroupEntry {
                binding: 1,
                resource: wgpu::BindingResource::Sampler(&resources.compiled.sampler),
            },
        ],
    });
    read_glow_binding(shooter, &binding)
}

#[test]
fn segment_pickup_tracks_pitch_position_and_activation_without_changing_coverage() {
    use harmonigraph_scene::{octave_layout, LatticeMaterial};
    let Some(mut shooter) = Shooter::new(SIZE) else { return };
    let mut scene = scene(&[1.0], 0.75, false);
    scene.node_radius *= 2.0; // narrow pigment still spans multiple pixels
    scene.view.atmosphere.material_style = LatticeMaterial::Watercolor;
    scene.view.atmosphere.breath_amount = 0.0;
    // A 0.4 reach keeps the solid band about 0.2 radii wide, so the
    // sampled arc centers sit inside pigment even after pixel rounding.
    scene.view.atmosphere.pigment_reach = 0.4;
    // Center and end sectors at a detuned seam must agree with the drawn ring.
    for cents in [0.0, 1100.0] {
        scene.octave_layout = octave_layout(7, 60.0);
        scene.nodes[0].cents = cents;
        for target_slot in [5, scene.octave_layout.ring(cents).base] {
            for level in [0.0, 0.5, 1.0] {
                scene.nodes[0].octaves = [0.0; 11];
                scene.nodes[0].octaves[4] = 1.0; // keeps real light under silent sectors
                scene.nodes[0].octaves[target_slot as usize] = level;
                scene.view.atmosphere.material_shadow_pickup = 0.0;
                scene.view.atmosphere.material_color_pickup = 0.0;
                shooter.shot(&scene);
                let before = material_source(&shooter);
                scene.view.atmosphere.material_color_pickup = 1.0;
                shooter.shot_again(&scene);
                let colored = material_source(&shooter);
                if level > 0.0 {
                    assert!(
                    before
                        .chunks_exact(4)
                        .zip(colored.chunks_exact(4))
                        .filter(|(a, b)| a[..3].iter().zip(*b).any(|(a, b)| a.abs_diff(*b) > 3))
                        .count()
                        > 100,
                    "color pickup works independently of dark pickup: slot {target_slot}, level {level}, cents {cents}"
                );
                }
                scene.view.atmosphere.material_shadow_pickup = 1.0;
                shooter.shot_again(&scene);
                let after = material_source(&shooter);
                for (a, b) in before.chunks_exact(4).zip(after.chunks_exact(4)) {
                    assert_eq!(
                        a[3], b[3],
                        "colored pigment preserves alpha, including empty light"
                    );
                    assert!(
                        b[..3].iter().all(|v| *v <= b[3].saturating_add(1)),
                        "valid premultiplied color"
                    );
                }
                for (slot, activation) in [(target_slot, level), (6, 0.0)] {
                    let (start, end) = scene.octave_layout.sector(slot, cents);
                    let angle = (start + end) * 0.5;
                    let world = glam::Vec3::new(angle.cos(), angle.sin(), 0.0)
                        * scene.node_radius
                        * 1.8
                        * scene.rings_outer;
                    let at = on_screen(&scene, SIZE, world);
                    let offset = (at.y as usize * SIZE[0] as usize + at.x as usize) * 4;
                    let pixel = &after[offset..offset + 4];
                    if slot == 6 {
                        assert_eq!(
                            &before[offset..offset + 4],
                            &colored[offset..offset + 4],
                            "color pickup alone leaves an unlit sector unchanged"
                        );
                    }
                    assert!(pixel[3] > 20, "fixture has enough source light at slot {slot}");
                    let pitch = scene.octave_layout.slot_pitch(slot, cents);
                    let t = ((pitch - scene.darkest_pitch)
                        / (scene.brightest_pitch - scene.darkest_pitch))
                        .clamp(0.0, 1.0);
                    for (actual, color) in pixel[..3].iter().zip([t, 0.4, 1.0 - t]) {
                        let expected = color * activation * f32::from(pixel[3]);
                        assert!(
                        (f32::from(*actual) - expected).abs() < 4.0,
                        "slot {slot}, cents {cents}, level {level}: {pixel:?}, expected {expected}"
                    );
                    }
                }
            }
        }
    }
}

#[test]
fn pickup_spreads_around_arc_ends_and_bloom_brightens_its_source() {
    use harmonigraph_scene::LatticeMaterial;
    let Some(mut shooter) = Shooter::new(SIZE) else { return };
    let mut scene = scene(&[1.0], 0.75, false);
    scene.octave_layout = probe_octave_layout();
    scene.pitch_lut.fill(glam::Vec4::new(0.35, 0.4, 0.5, 1.0));
    scene.view.atmosphere.material_style = LatticeMaterial::Watercolor;
    scene.view.atmosphere.breath_amount = 0.0;
    scene.view.atmosphere.material_shadow_pickup = 1.0;
    scene.view.atmosphere.material_color_pickup = 1.0;
    scene.view.atmosphere.pigment_reach = 0.7;
    shooter.shot(&scene);
    let plain = material_source(&shooter);
    let sample = |pixels: &[u8], uv: glam::Vec2| {
        let world = uv.extend(0.0) * scene.node_radius * 1.8;
        let at = on_screen(&scene, SIZE, world);
        let offset = (at.y as usize * SIZE[0] as usize + at.x as usize) * 4;
        assert!(pixels[offset + 3] > 20, "fixture must contain source light beyond the arc end");
        f32::from(pixels[offset + 2]) / f32::from(pixels[offset + 3])
    };
    let (start, end) = scene.octave_layout.sector(5, 0.0);
    for (angle, sign) in [(start, 1.0), (end, -1.0)] {
        let endpoint = glam::Vec2::new(angle.cos(), angle.sin()) * scene.rings_outer;
        let tangent = glam::Vec2::new(-angle.sin(), angle.cos()) * sign;
        let near = sample(&plain, endpoint + tangent * 0.2);
        let far_angle = angle + sign * 0.9;
        let far =
            sample(&plain, glam::Vec2::new(far_angle.cos(), far_angle.sin()) * scene.rings_outer);
        assert!(near > 0.1, "pickup extends past both angular ends: {near}");
        assert!(far < 0.025, "pickup ends by distance, rather than extending a ray: {far}");
    }
    scene.view.note_bloom = 2.0;
    shooter.shot_again(&scene);
    let bloomed = material_source(&shooter);
    let mut brighter = 0;
    for (a, b) in plain.chunks_exact(4).zip(bloomed.chunks_exact(4)) {
        assert_eq!(a[3], b[3], "Bloom changes pigment brightness, never source alpha");
        assert!(b[..3].iter().zip(a).all(|(b, a)| *b >= a.saturating_sub(1)));
        assert!(b[..3].iter().all(|v| *v <= b[3].saturating_add(1)));
        brighter += usize::from(b[..3].iter().zip(a).any(|(b, a)| b.saturating_sub(*a) > 3));
    }
    assert!(brighter > 100, "fixture reaches Bloom-responsive pigment: {brighter}");
    scene.view.atmosphere.material_color_pickup = 0.0;
    shooter.shot_again(&scene);
    let dark = material_source(&shooter);
    scene.view.note_bloom = 0.0;
    shooter.shot_again(&scene);
    assert_eq!(dark, material_source(&shooter), "Bloom never changes dark pickup");
}

#[test]
fn shadow_pickup_bypasses_with_material_or_light_disabled() {
    use harmonigraph_scene::LatticeMaterial;
    let Some(mut shooter) = Shooter::new(SIZE) else { return };
    for turn_off in [
        (|s: &mut Scene| s.view.atmosphere.material_style = LatticeMaterial::None)
            as fn(&mut Scene),
        |s| s.view.atmosphere.material_amount = 0.0,
        |s| s.view.glow_reach = 0.0,
        |s| s.view.glow_strength = 0.0,
        |s| s.view.atmosphere.pigment_reach = 0.0,
    ] {
        let mut scene = scene(&[1.0, 1.0], 0.75, true);
        scene.view.note_bloom = 1.0;
        scene.view.atmosphere.material_style = LatticeMaterial::Watercolor;
        scene.view.atmosphere.breath_amount = 0.0;
        turn_off(&mut scene);
        scene.view.atmosphere.material_shadow_pickup = 0.0;
        let ordinary = shooter.shot(&scene);
        scene.view.atmosphere.material_shadow_pickup = 1.0;
        scene.view.atmosphere.material_color_pickup = 1.0;
        assert_eq!(ordinary, shooter.shot_again(&scene), "bypass preserves ordinary shadows");
    }
}

#[test]
fn pickup_and_ordinary_shadows_are_independent() {
    use harmonigraph_scene::{LatticeMaterial, ShadowKernel};
    let Some(mut shooter) = Shooter::new(SIZE) else { return };
    let mut scene = scene(&[1.0, 1.0], 0.75, true);
    scene.view.atmosphere.material_style = LatticeMaterial::Watercolor;
    scene.view.atmosphere.breath_amount = 0.0;
    scene.view.shadow = one_shadow(0.8, 0.7, ShadowKernel::Distance);
    let scene_alpha = |shooter: &Shooter| {
        let resources = shooter.resources.get::<LatticeResources>().unwrap();
        let target = resources.panes[&shooter.pane].offscreen.as_ref().unwrap();
        let binding = shooter.device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: None,
            layout: &resources.compiled.filter_layout,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: wgpu::BindingResource::TextureView(&target.color_view),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: wgpu::BindingResource::Sampler(&resources.compiled.sampler),
                },
            ],
        });
        read_glow_binding(shooter, &binding).chunks_exact(4).map(|p| p[3]).collect::<Vec<_>>()
    };
    let plain = glow(&mut shooter, &scene);
    let ordinary = scene_alpha(&shooter);
    scene.view.atmosphere.material_shadow_pickup = 0.5;
    scene.view.atmosphere.material_color_pickup = 0.7;
    scene.view.atmosphere.pigment_reach = 5.5;
    shooter.shot_again(&scene);
    let pigment = read_glow(&shooter);
    assert_ne!(plain, pigment, "fixture reaches the pigment source");
    assert_eq!(ordinary, scene_alpha(&shooter), "pickup must not suppress the actual shadow mask");
    for (width, depth, kernel) in [
        (0.0, 0.0, ShadowKernel::Distance),
        (0.2, 1.0, ShadowKernel::Distance),
        (1.0, 0.3, ShadowKernel::Gaussian),
    ] {
        scene.view.shadow = one_shadow(width, depth, kernel);
        shooter.shot_again(&scene);
        assert_eq!(pigment, read_glow(&shooter), "actual shadow settings cannot change pickup");
        if width == 0.0 {
            assert_ne!(ordinary, scene_alpha(&shooter), "fixture must contain actual shadows");
        }
    }
}

#[test]
fn pigment_reach_extends_beyond_the_ordinary_node_quad() {
    use harmonigraph_scene::{LatticeMaterial, ShadowKernel};
    let Some(mut shooter) = Shooter::new(SIZE) else { return };
    let mut scene = scene(&[1.0], 1.0, false);
    scene.view.atmosphere.material_style = LatticeMaterial::Watercolor;
    scene.view.atmosphere.material_settings.wash_refract = 0.0;
    scene.view.atmosphere.breath_amount = 0.0;
    scene.view.shadow = one_shadow(0.0, 0.0, ShadowKernel::Distance);
    scene.view.glow_reach = 6.0;
    let plain = glow(&mut shooter, &scene);
    let (right, _) = scene.camera.right_up();
    let radius = on_screen(&scene, SIZE, right * scene.node_radius).distance(CENTRE);
    let far_changed = |output: &[u8]| {
        plain
            .chunks_exact(4)
            .zip(output.chunks_exact(4))
            .enumerate()
            .filter(|(i, (a, b))| {
                let at = glam::vec2(
                    (i % SIZE[0] as usize) as f32 + 0.5,
                    (i / SIZE[0] as usize) as f32 + 0.5,
                );
                at.distance(CENTRE) > 4.0 * radius
                    && a[..3].iter().zip(*b).any(|(a, b)| a.saturating_sub(*b) > 3)
            })
            .count()
    };
    scene.view.atmosphere.material_shadow_pickup = 0.8;
    scene.view.atmosphere.pigment_reach = 0.25;
    assert_eq!(far_changed(&glow(&mut shooter, &scene)), 0, "narrow pigment stays near its ring");
    for reach in [6.0, 12.0] {
        scene.view.atmosphere.pigment_reach = reach;
        let count = far_changed(&glow(&mut shooter, &scene));
        assert!(count > 100, "broad pickup must extend beyond the ink quad: {reach}: {count}");
    }
}

#[test]
fn watercolor_motion_and_silence_use_the_production_light() {
    let Some(mut shooter) = Shooter::new(SIZE) else { return };
    let mut scene = scene(&[1.0, 1.0], 0.75, true);
    scene.view.atmosphere.material_style = harmonigraph_scene::LatticeMaterial::Watercolor;
    scene.view.atmosphere.breath_amount = 0.0;
    scene.view.atmosphere.material_amount = 0.0;
    scene.view.atmosphere.material_settings.wash_size = 2.0;
    scene.view.atmosphere.material_speed = 1.0;
    scene.glow_timing =
        Some(harmonigraph_scene::GlowTiming { now: 0.0, attack: 0.0, release: 0.0 });
    let baseline = glow(&mut shooter, &scene);
    scene.view.atmosphere.material_amount = 1.0;
    let smooth = glow(&mut shooter, &scene);
    let changed = |a: &[u8], b: &[u8]| {
        a.chunks_exact(4)
            .zip(b.chunks_exact(4))
            .filter(|(a, b)| a.iter().zip(*b).any(|(a, b)| a.abs_diff(*b) > 3))
            .count()
    };
    assert!(changed(&baseline, &smooth) > 500, "fixture must reach the displacement pass");
    for pixel in smooth.chunks_exact(4) {
        assert!(pixel[..3].iter().all(|c| *c <= pixel[3] + 1), "premultiplied light");
        assert!(pixel.iter().all(|c| *c <= 154), "fixed gain ceiling");
    }
    scene.glow_timing.as_mut().unwrap().now = 8.0;
    shooter.shot_again(&scene);
    assert_ne!(smooth, read_glow(&shooter), "held notes still show material motion");
    scene.view.atmosphere.material_speed = 0.0;
    shooter.shot_again(&scene);
    assert_eq!(smooth, read_glow(&shooter), "zero speed freezes washes");
    scene.view.atmosphere.material_amount = 0.0;
    shooter.shot_again(&scene);
    assert_eq!(baseline, read_glow(&shooter), "depth zero restores the production smooth source");
    scene.view.atmosphere.material_amount = 1.0;
    scene.view.atmosphere.material_style = harmonigraph_scene::LatticeMaterial::None;
    shooter.shot_again(&scene);
    assert_eq!(baseline, read_glow(&shooter), "disabled material is exact");
    scene.view.atmosphere.material_style = harmonigraph_scene::LatticeMaterial::Watercolor;
    shooter.shot_again(&scene);
    assert_eq!(smooth, read_glow(&shooter));
    for node in &mut scene.nodes {
        node.glow.level = 0.0;
    }
    shooter.shot_again(&scene);
    assert!(
        read_glow(&shooter).iter().all(|b| *b == 0),
        "silence clears old source and displaced light"
    );
}

#[test]
fn materials_drift_stays_continuous_across_axis_wraps() {
    let Some(mut shooter) = Shooter::new(SIZE) else { return };
    for style in [
        harmonigraph_scene::LatticeMaterial::Watercolor,
        harmonigraph_scene::LatticeMaterial::VelvetScales,
    ] {
        let mut scene = scene(&[1.0, 1.0], 0.75, true);
        scene.view.atmosphere.material_style = style;
        scene.view.atmosphere.breath_amount = 0.0;
        scene.view.atmosphere.material_settings.wash_size = 2.0;
        scene.view.atmosphere.material_speed = 1.0;
        scene.glow_timing =
            Some(harmonigraph_scene::GlowTiming { now: 0.0, attack: 0.0, release: 0.0 });
        scene.view.atmosphere.material_amount = 0.0;
        let plain = glow(&mut shooter, &scene);
        scene.view.atmosphere.material_amount = 1.0;
        let painted = glow(&mut shooter, &scene);
        assert!(
            plain.iter().zip(&painted).filter(|(a, b)| a.abs_diff(**b) > 3).count() > 500,
            "fixture must reach the watercolor displacement pass"
        );
        // Cross both the former 40-cell boundary and the rotated tile's 200-cell
        // axis repeat, travelling in both directions along each screen axis.
        for (direction, axis) in [(0.0, 0), (90.0, 1), (180.0, 0), (270.0, 1)] {
            scene.view.atmosphere.material_direction = direction;
            let drift =
                |now| harmonigraph_scene::MaterialSettings::drift(1.0, direction, now)[axis];
            let cells = f64::from(if style == harmonigraph_scene::LatticeMaterial::VelvetScales {
                405.0 / 240.0 / scene.view.atmosphere.material_settings.velvet_size
            } else {
                5.25 / scene.view.atmosphere.material_settings.wash_size
            });
            let origin = if style == harmonigraph_scene::LatticeMaterial::VelvetScales {
                0.0
            } else {
                drift(0.0) * cells
            };
            let velocity = (drift(1.0) - drift(0.0)) * cells;
            for boundary in [40.0, 200.0] {
                let crossing = (boundary * velocity.signum() - origin) / velocity;
                scene.glow_timing.as_mut().unwrap().now = crossing - 0.0001;
                shooter.shot_again(&scene);
                let before = read_glow(&shooter);
                scene.glow_timing.as_mut().unwrap().now = crossing + 0.0001;
                shooter.shot_again(&scene);
                let after = read_glow(&shooter);
                let largest_step =
                    before.iter().zip(&after).map(|(a, b)| a.abs_diff(*b)).max().unwrap();
                assert!(
                largest_step <= 2,
                "direction {direction}, boundary {boundary}: sudden channel step {largest_step}"
            );
            }
        }
    }
}

#[test]
fn textures_feed_materials_before_sampling() {
    use harmonigraph_scene::LatticeMaterial;
    let Some(mut shooter) = Shooter::new(SIZE) else { return };
    let changed = |a: &[u8], b: &[u8]| {
        a.chunks_exact(4)
            .zip(b.chunks_exact(4))
            .filter(|(a, b)| a.iter().zip(*b).any(|(a, b)| a.abs_diff(*b) > 3))
            .count()
    };
    for material in
        [LatticeMaterial::Watercolor, LatticeMaterial::VelvetScales, LatticeMaterial::Stars]
    {
        let mut scene = scene(&[1.0, 1.0], 0.75, true);
        scene.view.atmosphere.breath_amount = 0.0;
        scene.view.atmosphere.texture_depth = 0.85;
        scene.view.atmosphere.texture_scale = 4.0;
        scene.glow_timing =
            Some(harmonigraph_scene::GlowTiming { now: 0.0, attack: 0.0, release: 0.0 });
        let texture_only = glow(&mut shooter, &scene);
        scene.view.atmosphere.material_settings.wash_size = 1.0;
        scene.view.atmosphere.material_style = material;
        let combined = glow(&mut shooter, &scene);
        // Read the actual material input, proving order rather than just two visible effects.
        let resources = shooter.resources.get::<LatticeResources>().unwrap();
        let source = &resources.panes[&shooter.pane]
            .offscreen
            .as_ref()
            .unwrap()
            .glow
            .as_ref()
            .unwrap()
            .material_source
            .as_ref()
            .unwrap()
            .view;
        let source_binding = shooter.device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("test_material_input"),
            layout: &resources.compiled.filter_layout,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: wgpu::BindingResource::TextureView(source),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: wgpu::BindingResource::Sampler(&resources.compiled.sampler),
                },
            ],
        });
        assert_eq!(texture_only, read_glow_binding(&shooter, &source_binding));
        scene.view.atmosphere.texture_depth = 0.0;
        let material_only = glow(&mut shooter, &scene);
        assert!(
            changed(&combined, &texture_only) > 500,
            "Clouds/{material:?}: visible displacement"
        );
        assert!(changed(&combined, &material_only) > 500, "Clouds/{material:?}: visible texture");
        for pixel in combined.chunks_exact(4) {
            assert!(pixel[..3].iter().all(|c| *c <= pixel[3] + 1));
            assert!(pixel.iter().all(|c| *c <= 154), "fixed gain ceiling");
        }
        scene.view.atmosphere.texture_depth = 0.85;
        for node in &mut scene.nodes {
            node.glow.level = 0.0;
        }
        shooter.shot_again(&scene);
        assert!(
            read_glow(&shooter).iter().all(|c| *c == 0),
            "combined stages must clear in silence"
        );
    }
}

#[test]
fn stage_clocks_and_bypasses_are_independent_on_a_carried_pane() {
    use harmonigraph_scene::LatticeMaterial;
    let Some(mut shooter) = Shooter::new(SIZE) else { return };
    let mut scene = scene(&[1.0, 1.0], 0.75, true);
    scene.view.atmosphere.texture_depth = 0.85;
    scene.view.atmosphere.texture_scale = 2.0;
    scene.view.atmosphere.material_style = LatticeMaterial::Watercolor;
    scene.view.atmosphere.breath_amount = 0.0;
    scene.view.atmosphere.texture_speed = 0.0;
    scene.view.atmosphere.material_speed = 0.0;
    scene.glow_timing =
        Some(harmonigraph_scene::GlowTiming { now: 0.0, attack: 0.0, release: 0.0 });
    let still = glow(&mut shooter, &scene);
    let geometry = |shooter: &Shooter| {
        shooter.resources.get::<LatticeResources>().unwrap().panes[&shooter.pane]
            .material_tile
            .as_ref()
            .map(|tile| tile.geometry_binding())
    };
    let original_tile = geometry(&shooter).unwrap();
    scene.glow_timing.as_mut().unwrap().now = 8.0;
    shooter.shot_again(&scene);
    assert_eq!(still, read_glow(&shooter), "both zero speeds freeze held light");
    for (texture_speed, material_speed) in [(1.0, 0.0), (0.0, 1.0)] {
        scene.view.atmosphere.texture_speed = texture_speed;
        scene.view.atmosphere.material_speed = material_speed;
        shooter.shot_again(&scene);
        assert_ne!(still, read_glow(&shooter), "each stage moves independently");
        assert_eq!(
            Some(original_tile.clone()),
            geometry(&shooter),
            "motion does not rebake geometry"
        );
    }
    // Texture edits and positive material amount never invalidate active geometry.
    scene.view.atmosphere.texture_depth = 0.4;
    scene.view.atmosphere.texture_scale = 4.0;
    scene.view.atmosphere.material_amount = 0.6;
    shooter.shot_again(&scene);
    assert_eq!(Some(original_tile), geometry(&shooter));
    assert_eq!(read_glow(&shooter), glow(&mut shooter, &scene));
    // Crossing the bypass boundary deliberately releases resources; reactivation is fresh.
    for (contrast, material, amount) in [
        (0.0, LatticeMaterial::Watercolor, 0.6),
        (0.4, LatticeMaterial::None, 0.6),
        (0.4, LatticeMaterial::Watercolor, 0.0),
        (0.4, LatticeMaterial::Watercolor, 0.6),
    ] {
        scene.view.atmosphere.texture_depth = contrast;
        scene.view.atmosphere.material_style = material;
        scene.view.atmosphere.material_amount = amount;
        shooter.shot_again(&scene);
        assert_eq!(geometry(&shooter).is_some(), material != LatticeMaterial::None && amount > 0.0);
        assert_eq!(read_glow(&shooter), glow(&mut shooter, &scene));
    }
    scene.view.render_scale = 1.5;
    shooter.shot_again(&scene);
    assert_eq!(
        read_glow(&shooter),
        glow(&mut shooter, &scene),
        "resize replaces the material source"
    );
}

#[test]
fn material_controls_change_light_and_only_geometry_controls_rebake() {
    use harmonigraph_scene::{LatticeMaterial, MaterialSettings};
    let Some(mut shooter) = Shooter::new(SIZE) else { return };
    let mut base = scene(&[1.0, 1.0], 0.75, true);
    base.view.atmosphere.breath_amount = 0.0;
    base.view.atmosphere.texture_depth = 0.8;
    base.view.atmosphere.material_speed = 0.0;
    base.view.atmosphere.material_settings = MaterialSettings {
        wash_size: 2.0,
        wash_fuzz: 0.5,
        wash_lobe: 0.5,
        wash_refract: 1.0,
        ..Default::default()
    };
    let tile = |shooter: &Shooter| {
        shooter.resources.get::<LatticeResources>().unwrap().panes[&shooter.pane]
            .material_tile
            .as_ref()
            .unwrap()
            .geometry_binding()
    };
    type Turn = fn(&mut MaterialSettings);
    for (style, label, rebake, turn) in [
        (
            LatticeMaterial::Watercolor,
            "feathering",
            true,
            (|s: &mut MaterialSettings| s.wash_fuzz = 0.0) as Turn,
        ),
        (LatticeMaterial::Watercolor, "warp", true, |s| s.wash_lobe = 0.0),
        (LatticeMaterial::Watercolor, "refraction", false, |s| s.wash_refract = 0.0),
        (LatticeMaterial::Watercolor, "fine layer", false, |s| s.wash_layers = 0.0),
        (LatticeMaterial::Watercolor, "random brightness", false, |s| s.wash_randomness = 1.0),
    ] {
        let mut scene = scene(&[1.0, 1.0], 0.75, true);
        scene.view.atmosphere = base.view.atmosphere;
        scene.view.atmosphere.material_style = style;
        let before = glow(&mut shooter, &scene);
        let before_tile = tile(&shooter);
        turn(&mut scene.view.atmosphere.material_settings);
        shooter.shot_again(&scene);
        let after = read_glow(&shooter);
        if label == "random brightness" {
            assert!(
                before.chunks_exact(4).zip(after.chunks_exact(4)).all(|(a, b)| a[3] == b[3]),
                "brightness must preserve premultiplied coverage"
            );
        }
        assert_eq!(before_tile != tile(&shooter), rebake, "{style:?}/{label}: tile invalidation");
        let changed = before
            .chunks_exact(4)
            .zip(after.chunks_exact(4))
            .filter(|(a, b)| a.iter().zip(*b).any(|(a, b)| a.abs_diff(*b) > 3))
            .count();
        assert!(
            changed > 100,
            "{style:?}/{label}: fixture must reach the effect ({changed} pixels)"
        );
        assert_eq!(
            after,
            glow(&mut shooter, &scene),
            "{style:?}/{label}: carried and fresh panes agree"
        );
    }
    // Scales bake no tile, so their dials only need to reach the light. Scale
    // shape 0, or its scallop taper hides much of the corner; Tilt is turned
    // over the squares, since a round body has no rotation to show.
    let mut scene = scene(&[1.0, 1.0], 0.75, true);
    scene.view.atmosphere = base.view.atmosphere;
    scene.view.atmosphere.material_style = LatticeMaterial::VelvetScales;
    scene.view.atmosphere.material_settings.velvet_shape = 0.0;
    for (label, turn) in [
        ("squareness", (|s: &mut MaterialSettings| s.velvet_square = 1.0) as Turn),
        ("tilt", |s| s.velvet_tilt = 0.0),
    ] {
        let before = glow(&mut shooter, &scene);
        turn(&mut scene.view.atmosphere.material_settings);
        shooter.shot_again(&scene);
        let after = read_glow(&shooter);
        let changed = before
            .chunks_exact(4)
            .zip(after.chunks_exact(4))
            .filter(|(a, b)| a.iter().zip(*b).any(|(a, b)| a.abs_diff(*b) > 3))
            .count();
        assert!(changed > 100, "Scales/{label}: fixture must reach the effect ({changed} pixels)");
    }
    // The inactive style must neither change the picture nor rebake the tile.
    base.view.atmosphere.material_style = LatticeMaterial::Watercolor;
    let before = glow(&mut shooter, &base);
    let before_tile = tile(&shooter);
    base.view.atmosphere.material_settings.velvet_variety = 0.0;
    shooter.shot_again(&base);
    assert_eq!(before, read_glow(&shooter));
    assert_eq!(before_tile, tile(&shooter));
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
        // Read a 256-square glow directly: the scene stays at twice that
        // resolution. This checks the analytic profile before reconstruction.
        scene.view.render_scale = 2.0;
        let colour = [0.8, 0.4, 0.1];
        scene.pitch_lut = [glam::Vec4::new(colour[0], colour[1], colour[2], 1.0);
            harmonigraph_scene::PITCH_LUT_N];
        let pixels = glow(&mut shooter, &scene);
        for accumulation in [0.5, 1.0] {
            scene.view.glow_accumulation = accumulation;
            assert_eq!(glow(&mut shooter, &scene), pixels, "a lone glow cannot change");
        }
        let per_uv = on_screen(&scene, SIZE, glam::Vec3::X * scene.node_radius * 1.8).x - CENTRE.x;
        let mut visible = 0;
        for (index, pixel) in pixels.chunks_exact(4).enumerate() {
            let at = glam::vec2((index % 256) as f32 + 0.5, (index / 256) as f32 + 0.5);
            let d = at.distance(CENTRE) / per_uv;
            // Even unmarked nodes use the maximum configured mark rim.
            let rim = scene.rings_outer.max(scene.mark_inner + scene.view.mark_thickness);
            let a = (0.8 * gain * level * (1.0 - d / (rim + 2.88)).max(0.0)).min(1.0);
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
    distant.view.glow_strength = 0.0;
    assert!(glow(&mut shooter, &distant).iter().all(|byte| *byte == 0));
}

#[test]
fn accumulation_sweeps_to_the_original_screen_of_each_colour_channel() {
    let Some(mut shooter) = Shooter::new(SIZE) else { return };
    let at = |levels: &[f32], accumulation| {
        let mut scene = scene(levels, 0.35, false);
        scene.view.glow_accumulation = accumulation;
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

/// Marks can change direction and hue, but no longer resize a held halo.
#[test]
fn marks_do_not_change_the_halo_footprint() {
    let Some(mut shooter) = Shooter::new(SIZE) else { return };
    let mut scene = scene(&[1.0], 0.75, false);
    assert!(
        scene.mark_inner + scene.view.mark_thickness > scene.rings_outer + 0.1,
        "the mark must extend beyond the rings to exercise the old size change"
    );
    scene.nodes[0].octaves.fill(1.0);
    let alpha = |pixels: Vec<u8>| pixels.chunks_exact(4).map(|p| p[3]).collect::<Vec<_>>();
    let bare = alpha(glow(&mut shooter, &scene));
    assert!(bare.iter().filter(|&&v| v > 20).count() > 1000);
    for level in [0.25, 1.0, 0.0] {
        scene.nodes[0].melody_slots = 1 << harmonigraph_scene::MIDDLE_C_SLOT;
        scene.nodes[0].melody_level = level;
        assert_eq!(bare, alpha(glow(&mut shooter, &scene)), "mark level {level}");
    }
}

/// Inactive mark geometry may widen the footprint, but must not wash the
/// directional colors toward their mean outside the ordinary ring edge.
#[test]
fn the_color_transition_ends_at_the_ring_even_with_a_wider_halo() {
    let Some(mut shooter) = Shooter::new(SIZE) else { return };
    let mut scene = scene(&[1.0], 0.75, false);
    scene.view.glow_blend = 0.0;
    scene.nodes[0].octaves.fill(1.0);
    scene.pitch_lut = std::array::from_fn(|i| {
        let t = i as f32 / (harmonigraph_scene::PITCH_LUT_N - 1) as f32;
        glam::vec4(1.0 - t, 0.05, t, 1.0)
    });
    scene.view.mark_thickness = 0.0;
    let ordinary = glow(&mut shooter, &scene);
    scene.mark_inner = scene.rings_outer;
    scene.view.mark_thickness = 0.6;
    let wide = glow(&mut shooter, &scene);
    let per_uv = on_screen(&scene, SIZE, glam::Vec3::X * scene.node_radius * 1.8).x - CENTRE.x;
    let hue = |p: &[u8]| {
        let sum = p[..3].iter().map(|&c| f32::from(c)).sum::<f32>();
        glam::vec3(f32::from(p[0]), f32::from(p[1]), f32::from(p[2])) / sum
    };
    let mut count = 0;
    let mut low = glam::Vec3::ONE;
    let mut high = glam::Vec3::ZERO;
    let mut worst = 0.0f32;
    for (i, (a, b)) in ordinary.chunks_exact(4).zip(wide.chunks_exact(4)).enumerate() {
        let at = glam::vec2((i % 256) as f32 + 0.5, (i / 256) as f32 + 0.5);
        let d = at.distance(CENTRE) / per_uv;
        if d <= scene.rings_outer + 0.02 || d >= scene.rings_outer + 0.3 {
            continue;
        }
        assert!(a[3] > 40 && b[3] > 40, "sample a visible halo, not quantization noise");
        let (a, b) = (hue(a), hue(b));
        low = low.min(a);
        high = high.max(a);
        worst = worst.max((a - b).abs().max_element());
        count += 1;
    }
    assert!(
        count > 100 && (high - low).max_element() > 0.2,
        "probe multiple directional colors in the affected annulus: count={count}, spread={:?}",
        high - low
    );
    assert!(worst < 0.025, "a wider footprint averaged directional colors: hue error {worst}");
}

#[test]
#[ignore = "manual MRT precision stress measurement"]
fn dense_faint_overlap_order_precision() {
    let Some(mut shooter) = Shooter::new(SIZE) else { return };
    // Follow-up: https://github.com/yan-h/harmonigraph/issues/878
    // Coincident halos intentionally exceed ordinary lattice overlap. This
    // probe measures f16 blend drift rather than asserting exact order parity.
    for count in [128, 1024, 4096] {
        let levels: Vec<f32> = (0..count)
            .map(|i| if i < count * 500 / 4096 { 0.000382 / 0.8 } else { 0.004165 / 0.8 })
            .collect();
        let mut scene = scene(&levels, 1.0, false);
        scene.view.atmosphere.texture_depth = 0.0;
        scene.view.atmosphere.breath_amount = 0.0;
        scene.pitch_lut = std::array::from_fn(|i| {
            if i * 2 < harmonigraph_scene::PITCH_LUT_N {
                glam::Vec4::new(1.0, 0.15, 0.1, 1.0)
            } else {
                glam::Vec4::new(0.1, 1.0, 0.25, 1.0)
            }
        });
        for (i, node) in scene.nodes.iter_mut().enumerate() {
            node.cents = if i < count * 500 / 4096 { 0.0 } else { 1100.0 };
        }
        for accumulation in [0.0, 0.5, 1.0] {
            scene.view.glow_accumulation = accumulation;
            let forward = glow(&mut shooter, &scene);
            assert!(
                forward.chunks_exact(4).any(|pixel| pixel[..3].iter().any(|v| *v > 20)),
                "the stress fixture must produce visible overlapping light"
            );
            scene.nodes.reverse();
            let backward = glow(&mut shooter, &scene);
            scene.nodes.reverse();
            let largest = forward.iter().zip(&backward).map(|(a, b)| a.abs_diff(*b)).max().unwrap();
            eprintln!("precision count={count} accumulation={accumulation}: maximum order difference {largest}/255");
        }
    }
}

#[test]
fn stars_sample_note_color_with_bounded_premultiplied_light_and_clear_silence() {
    use harmonigraph_scene::LatticeMaterial;
    let Some(mut shooter) = Shooter::new(SIZE) else { return };
    let mut scene = scene(&[1.0], 0.75, false);
    scene.view.atmosphere.texture_depth = 0.0;
    scene.view.atmosphere.breath_amount = 0.0;
    scene.view.atmosphere.material_style = LatticeMaterial::Stars;
    scene.glow_timing =
        Some(harmonigraph_scene::GlowTiming { now: 1.0, attack: 0.0, release: 0.0 });
    scene.view.atmosphere.material_amount = 0.0;
    let raw = glow(&mut shooter, &scene);
    let hue = raw.chunks_exact(4).max_by_key(|p| p[3]).unwrap();
    scene.view.atmosphere.material_amount = 1.0;
    for resolution in [1.0, 0.75, 1.0 / 3.0] {
        scene.view.atmosphere.stars.star_resolution = resolution;
        let painted = glow(&mut shooter, &scene);
        assert!(
            painted.chunks_exact(4).filter(|p| p[3] > 20).count() > 500,
            "fixture lights the starfield: {resolution}"
        );
        assert!(
            raw.iter().zip(&painted).filter(|(a, b)| a.abs_diff(**b) > 3).count() > 500,
            "Stars must change the note field: {resolution}"
        );
        for p in painted.chunks_exact(4) {
            assert!(
                p[..3].iter().all(|c| *c <= p[3].saturating_add(1)),
                "premultiplied {resolution}: {p:?}"
            );
            // A star is its source's hue at the star's own level, so channels
            // keep the source's proportions while alpha follows coverage.
            let brightest = *p[..3].iter().max().unwrap();
            if brightest > 30 {
                let source = *hue[..3].iter().max().unwrap();
                for channel in 0..3 {
                    let expected =
                        f32::from(hue[channel]) / f32::from(source) * f32::from(brightest);
                    assert!(
                        (f32::from(p[channel]) - expected).abs() <= 3.0,
                        "source hue {resolution}: {p:?}, source {hue:?}"
                    );
                }
            }
        }
        scene.glow_timing.as_mut().unwrap().now = 2.0;
        shooter.shot_again(&scene);
        assert_ne!(painted, read_glow(&shooter), "star motion must advance: {resolution}");
        for node in &mut scene.nodes {
            node.glow.level = 0.0;
        }
        shooter.shot_again(&scene);
        assert!(
            read_glow(&shooter).iter().all(|b| *b == 0),
            "silent stars retain no independent light: {resolution}"
        );
        scene.nodes[0].glow.level = 1.0;
        scene.glow_timing.as_mut().unwrap().now = 1.0;
    }
}

/// Stars carry the light's level in their colour, so dense stars with wide
/// fringes, which sum past full coverage, still follow the light's darkness.
/// Carried as opacity, the level clipped away: at these defaults full Pattern
/// contrast left 57% of the field where it leaves 33% of the plain glow.
#[test]
fn dense_stars_follow_the_pattern_darkness_of_their_light() {
    use harmonigraph_scene::LatticeMaterial;
    let Some(mut shooter) = Shooter::new(SIZE) else { return };
    let mut scene = scene(&[1.0, 1.0, 0.6], 0.75, true);
    scene.view.atmosphere.breath_amount = 0.0;
    scene.view.atmosphere.material_amount = 1.0;
    // The spectrogram's sizes, small and dense enough to sum past coverage:
    // the lattice draws what it stores `LATTICE_STAR_SIZE_SCALE` times over.
    scene.view.atmosphere.stars = harmonigraph_scene::StarSettings::default()
        .scaled(1.0 / harmonigraph_scene::LATTICE_STAR_SIZE_SCALE);
    scene.glow_timing =
        Some(harmonigraph_scene::GlowTiming { now: 1.0, attack: 0.0, release: 0.0 });
    let mut mean = |material, depth| {
        scene.view.atmosphere.material_style = material;
        scene.view.atmosphere.texture_depth = depth;
        let px = glow(&mut shooter, &scene);
        px.chunks_exact(4).map(luminance).sum::<f64>() / (px.len() / 4) as f64
    };
    let plain = [mean(LatticeMaterial::None, 0.0), mean(LatticeMaterial::None, 1.0)];
    let stars = [mean(LatticeMaterial::Stars, 0.0), mean(LatticeMaterial::Stars, 1.0)];
    let (kept, plain_kept) = (stars[1] / stars[0], plain[1] / plain[0]);
    assert!(kept < plain_kept * 1.25, "contrast darkens stars: {kept} vs {plain_kept}");
}

#[test]
fn stars_carried_material_and_resolution_transitions_match_fresh_panes() {
    use harmonigraph_scene::LatticeMaterial;
    let Some(mut shooter) = Shooter::new(SIZE) else { return };
    let mut scene = scene(&[1.0, 1.0], 0.75, true);
    scene.view.atmosphere.breath_amount = 0.0;
    scene.glow_timing =
        Some(harmonigraph_scene::GlowTiming { now: 3.0, attack: 0.0, release: 0.0 });
    for (style, amount, resolution, scale) in [
        (LatticeMaterial::Stars, 1.0, 1.0, 1.0),
        (LatticeMaterial::Stars, 1.0, 0.75, 1.0),
        (LatticeMaterial::Stars, 1.0, 1.0 / 3.0, 1.0),
        (LatticeMaterial::Watercolor, 1.0, 1.0 / 3.0, 1.0),
        (LatticeMaterial::Stars, 1.0, 1.0, 1.0),
        (LatticeMaterial::Stars, 0.0, 1.0, 1.0),
        (LatticeMaterial::Stars, 1.0, 1.0, 1.0),
        (LatticeMaterial::Stars, 1.0, 0.75, 1.5),
        (LatticeMaterial::Stars, 1.0, 0.5, 1.5),
    ] {
        scene.view.atmosphere.material_style = style;
        scene.view.atmosphere.material_amount = amount;
        scene.view.atmosphere.stars.star_resolution = resolution;
        scene.view.render_scale = scale;
        shooter.shot_again(&scene);
        let carried = read_glow(&shooter);
        assert_eq!(
            carried,
            glow(&mut shooter, &scene),
            "{style:?}, {amount}, {resolution}, {scale}"
        );
        if amount == 0.0 {
            scene.view.atmosphere.material_style = LatticeMaterial::None;
            assert_eq!(carried, glow(&mut shooter, &scene), "amount zero is exact bypass");
        }
    }
}

/// The Clouds pattern's detail layer fades toward its mean where its cells are
/// finer than the glow resolve texel, so the smallest Pattern sizes do not
/// alias it into texel-scale grain (#1319).
///
/// The glow target here is half of `SIZE`, 128 texels tall, and this Pattern
/// size puts the detail at about 1.2 texels a cell, two thirds of the way
/// through the fade (`NEBULA_DETAIL_FADE`), and the clouds at about three.
/// Read as the light's curvature between neighbouring glow texels over the
/// halo: 0.279 with the fade, 0.315 with it switched off (measured by
/// widening the fade past reach). The clouds' own grain is most of either
/// number, which is why the margin is narrow.
#[test]
fn the_clouds_detail_fades_where_it_is_finer_than_the_glow_texel() {
    const PATTERN: f32 = 0.11;
    let Some(mut shooter) = Shooter::new(SIZE) else { return };
    let mut scene = scene(&[1.0; 32], 0.75, false);
    scene.camera = harmonigraph_scene::Camera {
        projection: harmonigraph_scene::Projection::Orthographic,
        distance: 28.0,
        yaw: 0.0,
        pitch: 0.0,
        ..Default::default()
    };
    scene.view.atmosphere.texture_scale = PATTERN;
    scene.view.atmosphere.breath_amount = 0.0;
    scene.glow_timing =
        Some(harmonigraph_scene::GlowTiming { now: 0.0, attack: 0.3, release: 2.5 });
    // Detail cells per glow texel, as `nebula_light` reckons them: the fixture
    // has to stand well inside the fade for this to measure it.
    let detail = 5.0 * 2.3 / (PATTERN * (SIZE[1] / 2) as f32);
    assert!(detail > 0.6, "the detail spans {detail} cells a texel, short of the fade");
    let smooth = glow(&mut shooter, &scene);
    scene.view.atmosphere.texture_depth = 1.0;
    let textured = glow(&mut shooter, &scene);
    // The texture's share of the light at a pixel, read every other pixel so
    // neighbours are neighbouring glow texels rather than the blit's blend.
    let w = SIZE[0] as usize;
    let ratio = |x: usize, y: usize| -> Option<f32> {
        let i = (y * w + x) * 4 + 3;
        (smooth[i] > 30).then(|| f32::from(textured[i]) / f32::from(smooth[i]))
    };
    let (mut curvature, mut count) = (0.0f32, 0);
    for y in (3..SIZE[1] as usize - 3).step_by(2) {
        for x in (3..w - 3).step_by(2) {
            let near =
                [ratio(x, y), ratio(x - 2, y), ratio(x + 2, y), ratio(x, y - 2), ratio(x, y + 2)];
            if let [Some(c), Some(l), Some(r), Some(u), Some(d)] = near {
                curvature += (l + r + u + d - 4.0 * c).abs();
                count += 1;
            }
        }
    }
    assert!(count > 2000, "only {count} halo texels to read");
    let curvature = curvature / count as f32;
    assert!(curvature < 0.3, "the detail still grains the light: curvature {curvature}");
}
