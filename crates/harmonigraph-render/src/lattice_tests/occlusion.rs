//! Rear-node coverage fades independently of the shadow on the pooled light.

use super::fixtures::*;
use crate::*;

#[test]
fn partial_occlusion_fades_rear_ink_and_preserves_the_background() {
    let Some(mut shooter) = Shooter::new([256, 256]) else {
        return;
    };
    let mut shot = |scene: &Scene, enabled: bool| {
        // Keep the pane: ending with a single node must not follow old links
        // left in a storage allocation previously holding several nodes.
        shooter.draw_modified(scene, LatticeLabels::default(), |cb| {
            cb.uniforms.geometry_shadow.occlusion = f32::from(enabled);
        })
    };

    for kernel in
        [harmonigraph_scene::ShadowKernel::Distance, harmonigraph_scene::ShadowKernel::Gaussian]
    {
        for lit in [false, true] {
            let mut scene = single_marked_node(0, 0);
            scene.camera = harmonigraph_scene::Camera {
                projection: harmonigraph_scene::Projection::Orthographic,
                yaw: 0.0,
                pitch: 0.0,
                distance: 9.0,
                ..Default::default()
            };
            scene.octave_layout = probe_octave_layout();
            // A thin ring against a wide blur: direct Gaussian coverage is
            // too diluted here to hide most of the ink behind it.
            scene.outer_inner = scene.outer_outer - 0.08;
            scene.lattice_ground = glam::vec4(0.35, 0.35, 0.35, 1.0);
            scene.pitch_lut.fill(glam::vec4(0.9, 0.6, 0.3, 1.0));
            scene.nodes[0].octaves.fill(f32::from(lit));
            scene.nodes[0].world_pos = glam::vec3(-0.6, 0.0, -1.0);
            let mut front = scene.nodes[0];
            // A real light source even when the receiver is unlit.
            front.octaves.fill(1.0);
            front.world_pos = glam::vec3(0.9, 0.0, 1.0);
            scene.nodes.push(front);
            rows_per_node(&mut scene);
            scene.shadow = one_shadow(1.0, 0.18, kernel);
            scene.bloom_strength = 0.0;
            scene.glow_reach = 0.0;

            // Actual rasterized receiver coverage, not an assumed disc: the
            // fixture needs rear ink inside the front shadow's exposed skirt.
            // Keep its shadow width: that also sizes the rasterized quad.
            let front = scene.nodes.pop().unwrap();
            let ink = shot(&scene, false);
            scene.nodes.push(front);
            for glow in [0.0, 2.0] {
                scene.glow_reach = glow;
                let before = shot(&scene, false);
                let after = shot(&scene, true);
                let mut changed_ink = 0;
                let mut mostly_hidden_ink = 0;
                let mut unchanged_ground = 0;
                let mut illuminated_ground = 0;
                for ((old, new), mask) in
                    before.chunks_exact(4).zip(after.chunks_exact(4)).zip(ink.chunks_exact(4))
                {
                    if mask[..3] == [0, 0, 0] {
                        assert_eq!(old, new, "{kernel:?}, lit={lit}, glow={glow}: background or foreground ink moved");
                        unchanged_ground += 1;
                        if brightness(old) > 48 {
                            illuminated_ground += 1;
                        }
                    } else if brightness(old) - brightness(new) > 6 {
                        changed_ink += 1;
                        if brightness(old) > 32 && brightness(new) * 2 < brightness(old) {
                            mostly_hidden_ink += 1;
                        }
                    }
                }
                assert!(
                    changed_ink > 50,
                    "{kernel:?}, lit={lit}, glow={glow}: only {changed_ink} rear pixels faded"
                );
                if kernel == harmonigraph_scene::ShadowKernel::Gaussian && lit && glow == 0.0 {
                    assert!(
                        mostly_hidden_ink > 50,
                        "a wide Gaussian hid most of only {mostly_hidden_ink} rear-ink pixels"
                    );
                }
                assert!(unchanged_ground > 10_000, "the fixture must contain exposed background");
                if glow > 0.0 {
                    assert!(
                        illuminated_ground > 1_000,
                        "the unchanged background must include actual glow"
                    );
                }
            }
            // With no glow and a black clear, ordinary background shadows
            // have no RGB to darken. Changing their depth must leave ALL node
            // ink identical, both directly and through bloom. The old path
            // must differ here, proving that this fixture reaches a rear node
            // under a foreground shadow rather than merely missing it.
            scene.glow_reach = 0.0;
            for bloom in [0.0, 1.0] {
                scene.bloom_strength = bloom;
                scene.shadow.lattice_geometry.depth = 0.18;
                let faint = shot(&scene, true);
                let old_faint = shot(&scene, false);
                scene.shadow.lattice_geometry.depth = 0.8;
                let deep = shot(&scene, true);
                assert_eq!(
                    differing_pixels(&faint, &deep),
                    0,
                    "{kernel:?}, lit={lit}, bloom={bloom}: node ink still receives shadow darkness"
                );
                if bloom == 0.0 {
                    assert!(
                        differing_pixels(&old_faint, &shot(&scene, false)) > 50,
                        "the control must actually shadow rear ink"
                    );
                }
            }
            // The same depth adjustment must still darken exposed glow.
            scene.bloom_strength = 0.0;
            scene.glow_reach = 2.0;
            let deep_glow = shot(&scene, true);
            scene.shadow.lattice_geometry.depth = 0.18;
            let faint_glow = shot(&scene, true);
            let changed_glow = faint_glow
                .chunks_exact(4)
                .zip(deep_glow.chunks_exact(4))
                .zip(ink.chunks_exact(4))
                .filter(|((faint, deep), mask)| {
                    mask[..3] == [0, 0, 0] && brightness(faint) - brightness(deep) > 6
                })
                .count();
            assert!(
                changed_glow > 50,
                "{kernel:?}, lit={lit}: shadow darkness reached only {changed_glow} glow pixels"
            );

            // The bloom path must also use the reduced coverage, and the
            // final surviving foreground node must never occlude itself.
            scene.bloom_strength = 1.0;
            assert!(differing_pixels(&shot(&scene, false), &shot(&scene, true)) > 50);
            scene.nodes.remove(0);
            rows_per_node(&mut scene);
            assert_eq!(
                shot(&scene, false),
                shot(&scene, true),
                "a lone node must remain unchanged after a larger frame"
            );
        }
    }
}
