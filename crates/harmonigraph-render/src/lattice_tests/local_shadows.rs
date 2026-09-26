//! Local notation shadows keep their footprint when bloom is enabled.

use super::fixtures::*;
use crate::*;
use harmonigraph_scene::{ShadowKernel, ShadowStyle};

const SIZE: [u32; 2] = [256, 256];

fn scene(kernel: ShadowKernel, depth: f32) -> Scene {
    let mut scene = lit_node_and_a_name(3.0, 0.7, depth);
    scene.camera.distance = 6.0;
    scene.nodes[0].octaves.fill(1.0);
    scene.pitch_lut.fill(glam::vec4(0.9, 0.8, 0.7, 1.0));
    scene.glow_strength = 4.0;
    scene.shadow.lattice_geometry.depth = 0.0;
    scene.shadow.lattice_text = ShadowStyle { kernel, width: 0.7, depth, falloff: 0.0 };
    scene
}

#[test]
fn notation_shadows_darken_saturated_light_without_changing_remote_pixels() {
    let Some(mut shooter) = Shooter::new(SIZE) else { return };
    for kernel in [ShadowKernel::Gaussian, ShadowKernel::Distance] {
        for marker in [false, true] {
            let mut scene = scene(kernel, 1.0);
            scene.bloom_strength = 1.0;
            let at = name_on_the_band(&scene);
            if marker {
                scene.pluses.push(one_marker(0, at, 0.35, glam::Vec4::ONE, 1.0));
            }
            let labels = |scene: &Scene| {
                if marker {
                    LatticeLabels::default()
                } else {
                    name_at(scene, SIZE, at)
                }
            };
            let cb = LatticeCallback::from_scene(
                &scene,
                labels(&scene),
                egui::vec2(256.0, 256.0),
                shooter.format,
                1,
                None,
            );
            // Label boxes are world-positioned; the shared marker cell is local
            // to a cross, so bound that cross around its projected centre instead.
            let caster = if marker { &cb.casters[0] } else { cb.casters.last().unwrap() };
            let pad = caster.sigma_points * kernel.reach_sigmas() + 3.0;
            let centre = on_screen(&scene, SIZE, at);
            let radius = if marker {
                let edge = on_screen(&scene, SIZE, at + glam::vec3(0.35, 0.0, 0.0));
                (edge.x - centre.x).abs() + pad
            } else {
                NAME_SIZE * 0.5 + pad
            };
            let deep = shooter.shot_with(&scene, labels(&scene));
            scene.shadow.lattice_text.depth = 0.0;
            let bare = shooter.shot_with(&scene, labels(&scene));
            let (mut saturated, mut remote) = (0, 0);
            for (i, (a, b)) in bare.chunks_exact(4).zip(deep.chunks_exact(4)).enumerate() {
                let pixel = glam::vec2((i % 256) as f32 + 0.5, (i / 256) as f32 + 0.5);
                if (pixel - centre).abs().max_element() > radius {
                    assert_eq!(a, b, "{kernel:?}, marker={marker}: remote pixel {pixel:?}");
                    remote += 1;
                }
                if a[0] == 255 && b[0] < 235 {
                    saturated += 1;
                }
            }
            assert!(remote > 20_000, "fixture must leave a large unshadowed area");
            assert!(
                saturated > 32,
                "{kernel:?}, marker={marker}: only {saturated} clipped pixels darkened"
            );
        }
    }
}

#[test]
fn text_depth_scales_a_fixed_profile_on_the_pane_fill() {
    let Some(mut shooter) = Shooter::new(SIZE) else { return };
    shooter.clear = wgpu::Color { r: 0.8, g: 0.8, b: 0.8, a: 1.0 };
    for kernel in [ShadowKernel::Gaussian, ShadowKernel::Distance] {
        let mut scene = scene(kernel, 0.0);
        scene.background = glam::vec4(0.8, 0.8, 0.8, 1.0);
        scene.glow_reach = 0.0;
        scene.bloom_strength = 0.0;
        // Put the label on bare ground well outside the node, so this test
        // actually reads the host pane fill rather than the node's ink.
        let at = glam::vec3(-1.5, 1.5, 0.0);
        let mut shot = |depth| {
            scene.shadow.lattice_text.depth = depth;
            shooter.shot_with(&scene, name_at(&scene, SIZE, at))
        };
        let bare = shot(0.0);
        let half = shot(0.5);
        let full = shot(1.0);
        let mut measured = 0;
        for ((a, b), c) in bare.chunks_exact(4).zip(half.chunks_exact(4)).zip(full.chunks_exact(4))
        {
            let removed = i32::from(a[0]) - i32::from(c[0]);
            if a[0] == 204 && removed > 8 {
                let half_removed = i32::from(a[0]) - i32::from(b[0]);
                assert!(
                    (2 * half_removed - removed).abs() <= 3,
                    "{kernel:?}: {removed} vs {half_removed}"
                );
                measured += 1;
            }
        }
        assert!(measured > 100, "{kernel:?}: only {measured} ground pixels reached the shadow");
    }
}

#[test]
fn foreground_ink_restores_local_shadow_transmittance() {
    let Some(mut shooter) = Shooter::new(SIZE) else { return };
    for kernel in [ShadowKernel::Gaussian, ShadowKernel::Distance] {
        for bloom in [0.0, 1.0] {
            let mut scene = scene(kernel, 1.0);
            scene.glow_strength = 0.0;
            scene.bloom_strength = bloom;
            let mut near = scene.nodes[0];
            near.world_pos.z = 1.0;
            // The nearer owner supplies only a glyph, so its node cannot
            // cover the rear shadow before the glyph-restoration path does.
            near.scale = 0.0;
            scene.nodes.push(near);
            rows_per_node(&mut scene);
            let at = name_on_the_band(&scene);
            let centre = on_screen(&scene, SIZE, at);
            let foreground = [centre.x - 20.0, centre.y - 6.0, 40.0, 12.0];
            let labels = |scene: &Scene, cover: bool| {
                let mut labels = vec![(
                    0,
                    vec![name_glyph(scene, [centre.x - 6.0, centre.y - 6.0, 12.0, 12.0])],
                )];
                if cover {
                    labels.push((
                        1,
                        vec![GlyphInstance { fill: [255; 4], ..name_glyph(scene, foreground) }],
                    ));
                }
                names(labels)
            };
            // Identify opaque glyph pixels without bloom saturation disguising
            // partially covered reconstruction edges as white.
            scene.bloom_strength = 0.0;
            scene.shadow.lattice_text.depth = 0.0;
            let opaque = shooter.shot_with(&scene, labels(&scene, true));
            scene.bloom_strength = bloom;
            let mut shot = |depth, cover| {
                scene.shadow.lattice_text.depth = depth;
                shooter.shot_with(&scene, labels(&scene, cover))
            };
            let deep = shot(1.0, true);
            let flat = shot(0.0, true);
            let uncovered = shot(1.0, false);
            let uncovered_flat = shot(0.0, false);
            let mut restored = 0;
            for (i, ((a, b), c)) in deep
                .chunks_exact(4)
                .zip(flat.chunks_exact(4))
                .zip(uncovered.chunks_exact(4))
                .enumerate()
            {
                // Exclude the stretched atlas's reconstruction fringe;
                // near-opaque coverage can also round to 255 in readback.
                let x = (i % 256) as f32 + 0.5;
                let y = (i / 256) as f32 + 0.5;
                let inside = x > foreground[0] + 4.0
                    && x < foreground[0] + foreground[2] - 4.0
                    && y > foreground[1] + 2.0
                    && y < foreground[1] + foreground[3] - 2.0;
                if inside
                    && opaque[i * 4..i * 4 + 3] == [255; 3]
                    && i16::from(uncovered_flat[i * 4]) - i16::from(c[0]) > 8
                {
                    assert_eq!(a, b, "{kernel:?}, bloom={bloom}: foreground glyph darkened");
                    restored += 1;
                }
            }
            assert!(
                restored > 50,
                "{kernel:?}, bloom={bloom}: foreground only covered {restored} dark pixels"
            );
            assert!(differing_pixels(&deep, &flat) > 100, "shadow skirt must remain exposed");
        }
    }
}
