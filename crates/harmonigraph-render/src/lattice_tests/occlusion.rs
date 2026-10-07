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
    // `hide_behind` is the bar's strength. The field is packed for the
    // Darkness throughout, so only the strength moves.
    let mut shot = |scene: &Scene, hide_behind: f32| {
        // Keep the pane: ending with a single node must not follow old links
        // left in a storage allocation previously holding several nodes.
        shooter.draw_modified(scene, LatticeLabels::default(), |cb| {
            cb.uniforms.geometry_shadow.occlusion = hide_behind;
        })
    };

    for kernel in
        [harmonigraph_scene::ShadowKernel::Distance, harmonigraph_scene::ShadowKernel::Gaussian]
    {
        for lit in [false, true] {
            let mut scene = a_node_behind_another(lit);
            // Full Darkness, for the exposed glow it darkens below; the
            // hiding itself is the Hide behind bar's alone.
            scene.view.shadow = one_shadow(1.0, 1.0, kernel);
            scene.view.note_bloom = 0.0;
            scene.view.glow_reach = 0.0;

            // Actual rasterized receiver coverage, not an assumed disc: the
            // fixture needs rear ink inside the front shadow's exposed skirt.
            // Keep its shadow width: that also sizes the rasterized quad.
            let front = scene.nodes.pop().unwrap();
            let ink = shot(&scene, 0.0);
            scene.nodes.push(front);
            for glow in [0.0, 2.0] {
                scene.view.glow_reach = glow;
                let before = shot(&scene, 0.0);
                let after = shot(&scene, 1.0);
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
                // its linear coverage would. A distance spends its coverage
                // linearly, so there the hiding is lighter: it must still be
                // reached.
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
            // have no RGB to darken, and the ordinary shadow never darkens the
            // ink of a node behind its caster (`node_split`) — so with the
            // hiding at full strength, Darkness moves no rear ink at all. It
            // followed Darkness after #1288; Hide behind is its own bar now.
            // Light against deep rather than from 0, where the quads stop
            // growing for a shadow and clip a few codes of faint ink at
            // their edge whatever hides it; `a_node_at_no_darkness_still_hides_the_node_behind`
            // reads 0.
            scene.view.glow_reach = 0.0;
            scene.view.note_bloom = 0.0;
            let mut at = |depth: f32| {
                scene.view.shadow.lattice_geometry.depth = depth;
                shot(&scene, 1.0)
            };
            let (none, full) = (at(0.18), at(0.8));
            let moved = none
                .chunks_exact(4)
                .zip(full.chunks_exact(4))
                .zip(ink.chunks_exact(4))
                .filter(|((a, b), mask)| {
                    mask[..3] != [0, 0, 0] && (0..3).any(|k| a[k].abs_diff(b[k]) > 2)
                })
                .count();
            assert_eq!(
                moved, 0,
                "{kernel:?}, lit={lit}: Darkness moved {moved} rear-ink pixels, so the hiding \
                 still follows it"
            );
            // A front node at full Hide behind, beside a shadow as light as
            // the fresh look's, hides much more of the rear ink than the same
            // field at 0.28 — which is exactly what #1288's coupling left a
            // 0.28 Darkness hiding. Through bloom too, which is not linear:
            // more hides more and never less.
            scene.view.shadow.lattice_geometry.depth = 0.28;
            for bloom in [0.0, 1.0] {
                scene.view.note_bloom = bloom;
                let (coupled, whole) = (shot(&scene, 0.28), shot(&scene, 1.0));
                let pairs = || coupled.chunks_exact(4).zip(whole.chunks_exact(4));
                let hidden_more =
                    pairs().filter(|(c, w)| brightness(c) - brightness(w) > 6).count();
                let brighter = pairs().filter(|(c, w)| brightness(w) > brightness(c) + 1).count();
                assert!(
                    hidden_more > 50,
                    "{kernel:?}, lit={lit}, bloom={bloom}: full Hide behind hid only {hidden_more} \
                     more rear-ink pixels than 0.28"
                );
                assert_eq!(
                    brighter, 0,
                    "{kernel:?}, lit={lit}, bloom={bloom}: more Hide behind left node ink brighter"
                );
            }
            scene.view.shadow.lattice_geometry.depth = 0.8;
            // The same depth adjustment must still darken exposed glow,
            // including the cloud texture in the pooled light.
            scene.view.atmosphere.texture_depth = 0.85;
            scene.view.note_bloom = 0.0;
            scene.view.glow_reach = 2.0;
            let deep_glow = shot(&scene, 1.0);
            scene.view.shadow.lattice_geometry.depth = 0.18;
            let faint_glow = shot(&scene, 1.0);
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
            assert!(differing_pixels(&shot(&scene, 0.0), &shot(&scene, 1.0)) > 50);
            scene.nodes.remove(0);
            rows_per_node(&mut scene);
            assert_eq!(
                shot(&scene, 0.0),
                shot(&scene, 1.0),
                "a lone node must remain unchanged after a larger frame"
            );
        }
    }
}

/// A faint Hide behind hides next to none of the node behind it.
///
/// Occlusion used to hide rear ink at full strength at any Darkness above 0
/// and vanish all at once at 0 (#1288); it is linear in its own bar now, so at
/// 1% it may take at most a hundredth of the rear ink, and no channel moves
/// more than a few codes from the frame with the bar at 0. The fixture has to
/// reach `node_visibility` for that to mean anything, so the whole bar must
/// hide plenty of the same rear ink first.
#[test]
fn a_faint_hide_behind_hides_next_to_none_of_the_node_behind() {
    const FAINTEST: f32 = 0.01;
    let Some(mut shooter) = Shooter::new([256, 256]) else {
        return;
    };
    for kernel in
        [harmonigraph_scene::ShadowKernel::Gaussian, harmonigraph_scene::ShadowKernel::Distance]
    {
        let mut shot = |hide_behind: f32| {
            let mut scene = a_node_behind_another(true);
            scene.view.shadow = one_shadow(1.0, 1.0, kernel);
            scene.view.hide_behind = hide_behind;
            scene.view.glow_reach = 0.0;
            scene.view.note_bloom = 0.0;
            shooter.draw(&scene, LatticeLabels::default())
        };
        let off = shot(0.0);
        let reached = differing_pixels(&shot(1.0), &off);
        assert!(reached > 50, "{kernel:?}: the whole bar moved only {reached} rear-ink pixels");
        let worst = shot(FAINTEST).iter().zip(&off).map(|(a, b)| a.abs_diff(*b)).max().unwrap();
        assert!(worst <= 3, "{kernel:?}: a 1% Hide behind moved a channel {worst} codes from 0");
    }
}

/// A node with no visible shadow still hides the node behind it.
///
/// At Darkness 0 the group casts nothing, but Hide behind spends the same
/// field, so the CPU still packs it (`lattice_frame`). The visible shadow must
/// still multiply by exactly 1: away from the rear node's ink the frame is
/// the one with the group switched off altogether (Width 0, no field at all).
#[test]
fn a_node_at_no_darkness_still_hides_the_node_behind() {
    let Some(mut shooter) = Shooter::new([256, 256]) else {
        return;
    };
    for kernel in
        [harmonigraph_scene::ShadowKernel::Gaussian, harmonigraph_scene::ShadowKernel::Distance]
    {
        let mut shot = |width: f32, hide_behind: f32, front: bool| {
            let mut scene = a_node_behind_another(true);
            scene.view.shadow = one_shadow(width, 0.0, kernel);
            scene.view.hide_behind = hide_behind;
            scene.view.glow_reach = 0.0;
            scene.view.note_bloom = 0.0;
            if !front {
                scene.nodes.pop();
            }
            shooter.draw(&scene, LatticeLabels::default())
        };
        let ink = shot(1.0, 1.0, false);
        let (hidden, shown, unfielded) =
            (shot(1.0, 1.0, true), shot(1.0, 0.0, true), shot(0.0, 1.0, true));
        let mut faded = 0;
        for (((h, s), u), mask) in hidden
            .chunks_exact(4)
            .zip(shown.chunks_exact(4))
            .zip(unfielded.chunks_exact(4))
            .zip(ink.chunks_exact(4))
        {
            if mask[..3] == [0, 0, 0] {
                assert_eq!(
                    h, u,
                    "{kernel:?}: a shadow at Darkness 0 moved a pixel off the rear ink"
                );
            } else if brightness(s) - brightness(h) > 6 {
                faded += 1;
            }
        }
        assert!(
            faded > 50,
            "{kernel:?}: at Darkness 0 the front node hid only {faded} rear-ink pixels"
        );
    }
}

/// The rear ring of [`a_node_behind_another`] alone, lit, with nothing else in
/// the frame yet.
fn a_lone_ring() -> Scene {
    let mut scene = a_node_behind_another(true);
    scene.nodes.pop();
    rows_per_node(&mut scene);
    scene
}

/// The middle of node `i`'s band at the top of its ring, in world units: where
/// a cross or a name is put under that ring.
fn band_top(scene: &Scene, i: usize) -> glam::Vec3 {
    let n = scene.nodes[i];
    let mid = 0.5 * (scene.outer_inner + scene.outer_outer) * 1.8 * scene.node_radius * n.scale;
    n.world_pos + glam::vec3(0.0, mid, 0.0)
}

/// A position of its own at depth `z` that draws nothing but what is put on
/// it — an idle node, which ships nothing — returning its index.
fn a_bare_place(scene: &mut Scene, z: f32) -> usize {
    let node = scene.nodes.len();
    scene.nodes.push(harmonigraph_scene::NodeInstance {
        audio_ring: 0.0,
        ..harmonigraph_scene::NodeInstance::at(
            harmonigraph_core::LatticePos::new(node as i32 + 10, 0, 0),
            glam::vec3(0.0, 0.0, z),
            1.0,
            true,
            0.0,
            false,
            node as u32,
        )
    });
    rows_per_node(scene);
    node
}

/// A resting cross on position `place`, over `at`, half a world unit to the
/// tip.
fn a_cross_on(scene: &mut Scene, place: usize, at: glam::Vec3) {
    let ground = scene.lattice_ground;
    scene.pluses.push(one_marker(place, at, 0.5, ground, 1.0));
}

/// A square name of `side` points on position `place`, centred over `at`.
fn a_name_on(scene: &Scene, place: usize, at: glam::Vec3, side: f32) -> LatticeLabels {
    let c = on_screen(scene, [256, 256], at);
    let rect = [c.x - side / 2.0, c.y - side / 2.0, side, side];
    names(vec![(place as u32, vec![name_glyph(scene, rect)])])
}

/// The pixels a shot holds any ink on, out of a black frame.
fn inked(shot: &[u8]) -> Vec<bool> {
    shot.chunks_exact(4).map(|px| px[..3] != [0, 0, 0]).collect()
}

/// A ring in front hides a resting cross behind it, as it hides a ring or a
/// name, and continuously in the bar: 1% hides next to nothing, and at 0 the
/// cross off the ring's own ink is the cross with no ring in the frame.
///
/// The cross stands over the front ring's band on a position further back, so
/// the ring's field covers the arms either side of the band. Darkness 0, so
/// the ring casts no visible shadow and a cross pixel that moves is hidden.
#[test]
fn a_ring_in_front_hides_a_cross_behind_it() {
    let Some(mut shooter) = Shooter::new([256, 256]) else {
        return;
    };
    for kernel in
        [harmonigraph_scene::ShadowKernel::Gaussian, harmonigraph_scene::ShadowKernel::Distance]
    {
        let mut shot = |ring: bool, hide_behind: f32| {
            let mut scene = a_lone_ring();
            scene.nodes[0].world_pos.z = 1.0;
            let place = a_bare_place(&mut scene, -1.0);
            let top = band_top(&scene, 0);
            a_cross_on(&mut scene, place, top);
            if !ring {
                scene.nodes[0].activation = 0.0;
                scene.nodes[0].octaves.fill(0.0);
            }
            scene.view.shadow = one_shadow(1.0, 0.0, kernel);
            scene.view.hide_behind = hide_behind;
            scene.view.glow_reach = 0.0;
            scene.view.note_bloom = 0.0;
            shooter.draw(&scene, LatticeLabels::default())
        };
        let alone = shot(false, 1.0);
        let cross = inked(&alone);
        let (shown, hidden) = (shot(true, 0.0), shot(true, 1.0));
        let pairs = || shown.chunks_exact(4).zip(hidden.chunks_exact(4)).enumerate();
        let faded =
            pairs().filter(|(i, (s, h))| cross[*i] && brightness(s) - brightness(h) > 6).count();
        let moved = pairs().filter(|(i, (s, h))| !cross[*i] && s != h).count();
        assert!(faded > 30, "{kernel:?}: the ring hid only {faded} pixels of the cross behind it");
        assert_eq!(moved, 0, "{kernel:?}: hiding the cross moved {moved} pixels off it");

        let worst = shot(true, 0.01).iter().zip(&shown).map(|(a, b)| a.abs_diff(*b)).max();
        assert!(worst <= Some(3), "{kernel:?}: a 1% Hide behind moved the cross {worst:?} codes");
        let ring = inked(&{
            let mut scene = a_lone_ring();
            scene.nodes[0].world_pos.z = 1.0;
            scene.view.shadow = one_shadow(1.0, 0.0, kernel);
            scene.view.glow_reach = 0.0;
            scene.view.note_bloom = 0.0;
            shooter.draw(&scene, LatticeLabels::default())
        });
        let kept = shown
            .chunks_exact(4)
            .zip(alone.chunks_exact(4))
            .enumerate()
            .filter(|(i, _)| cross[*i] && !ring[*i])
            .flat_map(|(_, (a, b))| (0..3).map(move |k| a[k].abs_diff(b[k])))
            .max();
        assert!(kept <= Some(1), "{kernel:?}: at 0 the ring hid the cross by {kept:?} codes");
    }
}

/// A node never hides the cross it stands on or its own name: a lit node with
/// both over its own band draws the same at every Hide behind. The control
/// moves the same cross and name to a position behind the node, where the same
/// ring has to hide plenty of them — so the fixture reaches the path the
/// exclusion guards.
#[test]
fn a_node_never_hides_its_own_cross_or_name() {
    let Some(mut shooter) = Shooter::new([256, 256]) else {
        return;
    };
    for kernel in
        [harmonigraph_scene::ShadowKernel::Gaussian, harmonigraph_scene::ShadowKernel::Distance]
    {
        let mut moved = |own: bool| {
            let mut shot = |hide_behind: f32| {
                let mut scene = a_lone_ring();
                let behind = a_bare_place(&mut scene, -2.0);
                let place = if own { 0 } else { behind };
                let top = band_top(&scene, 0);
                a_cross_on(&mut scene, place, top);
                scene.view.shadow = one_shadow(1.0, 0.5, kernel);
                scene.view.hide_behind = hide_behind;
                scene.view.glow_reach = 0.0;
                scene.view.note_bloom = 0.0;
                // Clear of the crossing, still over the band.
                let name = a_name_on(&scene, place, top + glam::vec3(0.6, 0.0, 0.0), 6.0);
                shooter.draw(&scene, name)
            };
            differing_pixels(&shot(0.0), &shot(1.0))
        };
        let reached = moved(false);
        assert!(reached > 50, "{kernel:?}: from behind they were hidden by only {reached} pixels");
        assert_eq!(moved(true), 0, "{kernel:?}: a node hid part of its own cross or name");
    }
}
