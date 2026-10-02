//! Rear-node coverage fades independently of the shadow on the pooled light.

use super::fixtures::*;
use crate::*;

/// A receiver node with a second node in front of it, overlapping it on the
/// pane, so the front node's field covers part of the rear one's ink: the shape
/// `node_visibility` acts on. Painter order puts the front node LATER, which is
/// what makes it a caster for the rear one.
fn a_node_behind_another(lit: bool) -> Scene {
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
    scene
}

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
            let mut scene = a_node_behind_another(lit);
            // Full Darkness, where occlusion is at its whole strength: below
            // it the hiding is scaled down with the shadow (#1288).
            scene.view.shadow = one_shadow(1.0, 1.0, kernel);
            scene.view.note_bloom = 0.0;
            scene.view.glow_reach = 0.0;

            // Actual rasterized receiver coverage, not an assumed disc: the
            // fixture needs rear ink inside the front shadow's exposed skirt.
            // Keep its shadow width: that also sizes the rasterized quad.
            let front = scene.nodes.pop().unwrap();
            let ink = shot(&scene, false);
            scene.nodes.push(front);
            for glow in [0.0, 2.0] {
                scene.view.glow_reach = glow;
                let before = shot(&scene, false);
                let after = shot(&scene, true);
                let mut changed_ink = 0;
                let mut reached_ink = 0;
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
                    } else if old != new {
                        reached_ink += 1;
                    }
                    if mask[..3] != [0, 0, 0] && brightness(old) - brightness(new) > 6 {
                        changed_ink += 1;
                        if brightness(old) > 32 && brightness(new) * 2 < brightness(old) {
                            mostly_hidden_ink += 1;
                        }
                    }
                }
                // A Gaussian's occlusion spends its field through the
                // full-depth transmittance, which hides far more rear ink than
                // the ordinary coverage the reference path multiplies it by. A
                // distance spends the same field at the same Darkness as that
                // reference (#1288), so there the two paths only part in how
                // the faded ink composites: it must still be reached.
                if kernel == harmonigraph_scene::ShadowKernel::Gaussian {
                    assert!(
                        changed_ink > 50,
                        "{kernel:?}, lit={lit}, glow={glow}: only {changed_ink} rear pixels faded"
                    );
                }
                assert!(
                    reached_ink > 50,
                    "{kernel:?}, lit={lit}, glow={glow}: occlusion reached only {reached_ink} rear pixels"
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
            // have no RGB to darken, so whatever Darkness moves in node ink
            // here is occlusion alone. Occlusion follows it LINEARLY (#1288),
            // so the rear ink at 0.4 is the mean of 0 and 0.8: rear ink that
            // also took the ordinary shadow on top of its fading would bow
            // well under it.
            scene.view.glow_reach = 0.0;
            scene.view.note_bloom = 0.0;
            let mut at = |depth: f32| {
                scene.view.shadow.lattice_geometry.depth = depth;
                shot(&scene, true)
            };
            let (none, half, full) = (at(0.0), at(0.4), at(0.8));
            let (probed, worst) = off_the_midpoint(&none, &half, &full, 8);

            assert!(probed > 50, "{kernel:?}, lit={lit}: Darkness moved only {probed} pixels");
            assert!(
                worst <= 2.0,
                "{kernel:?}, lit={lit}: rear ink at 0.4 sits {worst} codes off the mean of 0 and \
                 0.8 over {probed} pixels, so something besides occlusion darkens it"
            );
            // And through bloom, which is not linear: a deeper shadow hides
            // more rear ink and never less.
            for bloom in [0.0, 1.0] {
                scene.view.note_bloom = bloom;
                scene.view.shadow.lattice_geometry.depth = 0.18;
                let faint = shot(&scene, true);
                scene.view.shadow.lattice_geometry.depth = 0.8;
                let deep = shot(&scene, true);
                let pairs = || faint.chunks_exact(4).zip(deep.chunks_exact(4));
                let hidden_more =
                    pairs().filter(|(f, d)| brightness(f) - brightness(d) > 6).count();
                let brighter = pairs().filter(|(f, d)| brightness(d) > brightness(f) + 1).count();
                assert!(
                    hidden_more > 50,
                    "{kernel:?}, lit={lit}, bloom={bloom}: a deeper shadow hid only {hidden_more} more rear-ink pixels"
                );
                assert_eq!(
                    brighter, 0,
                    "{kernel:?}, lit={lit}, bloom={bloom}: a deeper shadow left node ink brighter"
                );
            }
            // The same depth adjustment must still darken exposed glow,
            // including the cloud texture in the pooled light.
            scene.view.atmosphere.texture_depth = 0.85;
            scene.view.note_bloom = 0.0;
            scene.view.glow_reach = 2.0;
            let deep_glow = shot(&scene, true);
            scene.view.shadow.lattice_geometry.depth = 0.18;
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
            scene.view.note_bloom = 1.0;
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

/// A node's faintest shadow hides next to none of the node behind it.
///
/// Occlusion follows the Shadow darkness bar (#1288): it used to hide rear ink
/// at full strength at any Darkness above 0 and vanish all at once at 0. At 1%
/// it may take at most a hundredth of the rear ink, so no channel moves more
/// than a few codes from the frame with the group switched off. The fixture
/// has to reach `node_visibility` for that to mean anything, so full Darkness
/// must hide plenty of the same rear ink first.
#[test]
fn a_nodes_faintest_shadow_hides_next_to_none_of_the_node_behind() {
    const FAINTEST: f32 = 0.01;
    let Some(mut shooter) = Shooter::new([256, 256]) else {
        return;
    };
    for kernel in
        [harmonigraph_scene::ShadowKernel::Gaussian, harmonigraph_scene::ShadowKernel::Distance]
    {
        let mut shot = |depth: f32, occlusion: bool| {
            let mut scene = a_node_behind_another(true);
            scene.view.shadow = one_shadow(1.0, depth, kernel);
            scene.view.glow_reach = 0.0;
            scene.view.note_bloom = 0.0;
            shooter.draw_modified(&scene, LatticeLabels::default(), |cb| {
                cb.uniforms.geometry_shadow.occlusion = f32::from(occlusion);
            })
        };
        let reached = differing_pixels(&shot(1.0, true), &shot(1.0, false));
        assert!(reached > 50, "{kernel:?}: full occlusion moved only {reached} rear-ink pixels");
        let (faint, off) = (shot(FAINTEST, true), shot(0.0, true));
        let worst = faint.iter().zip(&off).map(|(a, b)| a.abs_diff(*b)).max().unwrap();
        assert!(
            worst <= 3,
            "{kernel:?}: a 1% node shadow moved a channel {worst} codes from no shadow at all",
        );
    }
}
