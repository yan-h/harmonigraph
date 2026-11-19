//! The atmosphere over the lattice glow: the material, its textures and
//! stars, and the breathing, each read off the production glow field as
//! [`glow_overlap`](super::glow_overlap) reads the light they shape.

use super::fixtures::*;
use super::glow_overlap::{glow, luminance, read_glow, read_glow_binding, scene, SIZE};
use crate::*;

#[test]
fn a_held_nodes_light_breathes_without_accumulating_anything() {
    let Some(mut shooter) = Shooter::new(SIZE) else { return };
    let mut scene = scene(&[1.0], 0.75, false);
    scene.glow_clock = Some(0.0);
    let first = glow(&mut shooter, &scene);
    scene.glow_clock = Some(4.0);
    shooter.shot_again(&scene);
    let later = read_glow(&shooter);
    assert!(first.iter().any(|&v| v > 20), "the held node must light the target");
    assert_ne!(first, later, "a constant held note's halo must breathe");
    // A fresh pane at the same time must agree with the carried pane: breathing
    // is a display modulation, with no accumulated effect on the ink colour.
    assert_eq!(later, glow(&mut shooter, &scene));
    scene.view.atmosphere.breath_amount = 0.0;
    let steady = glow(&mut shooter, &scene);
    scene.glow_clock = Some(0.0);
    assert_eq!(steady, glow(&mut shooter, &scene), "zero depth must stop breathing");
    scene.view.atmosphere.breath_amount = 1.0;
    scene.view.atmosphere.texture_depth = 0.0;
    scene.glow_clock = Some(4.0);
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
        // The glow target is quarter-resolution: resolve the clouds rather
        // than testing their subpixel average at this small size.
        scene.view.atmosphere.texture_scale = 4.0;
        scene.view.atmosphere.breath_amount = 0.0;
        scene.glow_clock = Some(0.0);
        let smooth = glow(&mut shooter, &scene);
        scene.view.atmosphere.texture_depth = 1.0;
        let textured = glow(&mut shooter, &scene);
        let mut changed = 0;
        let mut empty = 0;
        let mut darkest_ratio = 1.0f32;
        let mut brightest_ratio = 0.0f32;
        for (before, after) in smooth.as_chunks::<4>().0.iter().zip(textured.as_chunks::<4>().0) {
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
        scene.glow_clock = Some(8.0);
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
fn watercolor_motion_and_silence_use_the_production_light() {
    let Some(mut shooter) = Shooter::new(SIZE) else { return };
    let mut scene = scene(&[1.0, 1.0], 0.75, true);
    scene.view.atmosphere.material_style = harmonigraph_scene::LatticeMaterial::Watercolor;
    scene.view.atmosphere.breath_amount = 0.0;
    scene.view.atmosphere.material_amount = 0.0;
    scene.view.atmosphere.material_settings.wash_size = 2.0;
    scene.view.atmosphere.material_speed = 1.0;
    scene.glow_clock = Some(0.0);
    let baseline = glow(&mut shooter, &scene);
    scene.view.atmosphere.material_amount = 1.0;
    let smooth = glow(&mut shooter, &scene);
    let changed = |a: &[u8], b: &[u8]| {
        a.as_chunks::<4>()
            .0
            .iter()
            .zip(b.as_chunks::<4>().0)
            .filter(|(a, b)| a.iter().zip(*b).any(|(a, b)| a.abs_diff(*b) > 3))
            .count()
    };
    assert!(changed(&baseline, &smooth) > 500, "fixture must reach the displacement pass");
    for pixel in smooth.as_chunks::<4>().0 {
        assert!(pixel[..3].iter().all(|c| *c <= pixel[3] + 1), "premultiplied light");
        assert!(pixel.iter().all(|c| *c <= 154), "fixed gain ceiling");
    }
    scene.glow_clock = Some(8.0);
    shooter.shot_again(&scene);
    let moved = read_glow(&shooter);
    assert_ne!(smooth, moved, "held notes still show material motion");
    scene.view.atmosphere.material_speed = 0.0;
    scene.glow_clock = Some(16.0);
    shooter.shot_again(&scene);
    assert_eq!(moved, read_glow(&shooter), "zero speed freezes washes where they are");
    scene.view.atmosphere.material_amount = 0.0;
    shooter.shot_again(&scene);
    assert_eq!(baseline, read_glow(&shooter), "depth zero restores the production smooth source");
    scene.view.atmosphere.material_amount = 1.0;
    scene.view.atmosphere.material_style = harmonigraph_scene::LatticeMaterial::None;
    shooter.shot_again(&scene);
    assert_eq!(baseline, read_glow(&shooter), "disabled material is exact");
    scene.view.atmosphere.material_style = harmonigraph_scene::LatticeMaterial::Watercolor;
    shooter.shot_again(&scene);
    assert_eq!(moved, read_glow(&shooter), "a disabled material goes on from where it was");
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
        scene.glow_clock = Some(0.0);
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
            // Each direction's first crossing comes before the last one's, a
            // clock run backwards, so the drift starts over at `now` times
            // the rate there.
            let drift = |now| {
                harmonigraph_scene::MaterialSettings::DRIFT_START[axis]
                    + harmonigraph_scene::MaterialSettings::drift_rate(1.0, direction)[axis] * now
            };
            let cells = crate::material_cells::cells_per_unit(
                style == harmonigraph_scene::LatticeMaterial::VelvetScales,
                &scene.view.atmosphere.material_settings,
            );
            let origin = if style == harmonigraph_scene::LatticeMaterial::VelvetScales {
                0.0
            } else {
                drift(0.0) * cells
            };
            let velocity = (drift(1.0) - drift(0.0)) * cells;
            for boundary in [40.0, 200.0] {
                let crossing = (boundary * velocity.signum() - origin) / velocity;
                scene.glow_clock = Some(crossing - 0.0001);
                shooter.shot_again(&scene);
                let before = read_glow(&shooter);
                scene.glow_clock = Some(crossing + 0.0001);
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

/// A frame with no glow clock, as one with the glow switched off draws, holds
/// the textures where a drag left them: the glow comes back where it was
/// rather than where `now × speed` would put it.
#[test]
fn a_frame_without_a_glow_clock_holds_the_travel() {
    let Some(mut shooter) = Shooter::new(SIZE) else { return };
    let largest_step =
        |a: &[u8], b: &[u8]| a.iter().zip(b).map(|(a, b)| a.abs_diff(*b)).max().unwrap();
    let mut scene = scene(&[1.0, 1.0], 0.75, true);
    let a = &mut scene.view.atmosphere;
    a.material_amount = 0.0;
    a.breath_amount = 0.0;
    a.texture_depth = 1.0;
    a.texture_scale = 4.0;
    scene.glow_clock = Some(600.0);
    shooter.shot_again(&scene);
    scene.view.atmosphere.texture_speed *= 0.99;
    *scene.glow_clock.as_mut().unwrap() += 0.0001;
    shooter.shot_again(&scene);
    let dragged = read_glow(&shooter);
    let clock = scene.glow_clock.take();
    shooter.shot_again(&scene);
    scene.glow_clock = clock.map(|now| now + 0.0001);
    shooter.shot_again(&scene);
    let back = largest_step(&dragged, &read_glow(&shooter));
    assert!(back <= 2, "sudden channel step {back}");
}

#[test]
fn textures_feed_materials_before_sampling() {
    use harmonigraph_scene::LatticeMaterial;
    let Some(mut shooter) = Shooter::new(SIZE) else { return };
    let changed = |a: &[u8], b: &[u8]| {
        a.as_chunks::<4>()
            .0
            .iter()
            .zip(b.as_chunks::<4>().0)
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
        scene.glow_clock = Some(0.0);
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
        for pixel in combined.as_chunks::<4>().0 {
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
    scene.glow_clock = Some(0.0);
    let still = glow(&mut shooter, &scene);
    let geometry = |shooter: &Shooter| {
        shooter.resources.get::<LatticeResources>().unwrap().panes[&shooter.pane]
            .material_tile
            .as_ref()
            .map(|tile| tile.geometry_binding())
    };
    let original_tile = geometry(&shooter).unwrap();
    scene.glow_clock = Some(8.0);
    shooter.shot_again(&scene);
    assert_eq!(still, read_glow(&shooter), "both zero speeds freeze held light");
    // The material's drift is built up frame by frame, so it moves only as
    // the clock runs on at its speed.
    for (texture_speed, material_speed) in [(1.0, 0.0), (0.0, 1.0)] {
        scene.view.atmosphere.texture_speed = texture_speed;
        scene.view.atmosphere.material_speed = material_speed;
        *scene.glow_clock.as_mut().unwrap() += 8.0;
        shooter.shot_again(&scene);
        assert_ne!(still, read_glow(&shooter), "each stage moves independently");
        assert_eq!(
            Some(original_tile.clone()),
            geometry(&shooter),
            "motion does not rebake geometry"
        );
    }
    // Back to the clock's start, as a render started over: the carried pane's
    // drift starts over there too, and draws what a fresh pane does.
    scene.glow_clock = Some(0.0);
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
                before
                    .as_chunks::<4>()
                    .0
                    .iter()
                    .zip(after.as_chunks::<4>().0)
                    .all(|(a, b)| a[3] == b[3]),
                "brightness must preserve premultiplied coverage"
            );
        }
        assert_eq!(before_tile != tile(&shooter), rebake, "{style:?}/{label}: tile invalidation");
        let changed = before
            .as_chunks::<4>()
            .0
            .iter()
            .zip(after.as_chunks::<4>().0)
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
            .as_chunks::<4>()
            .0
            .iter()
            .zip(after.as_chunks::<4>().0)
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

#[test]
fn stars_sample_note_color_with_bounded_premultiplied_light_and_clear_silence() {
    use harmonigraph_scene::LatticeMaterial;
    let Some(mut shooter) = Shooter::new(SIZE) else { return };
    let mut scene = scene(&[1.0], 0.75, false);
    scene.view.atmosphere.texture_depth = 0.0;
    scene.view.atmosphere.breath_amount = 0.0;
    scene.view.atmosphere.material_style = LatticeMaterial::Stars;
    scene.glow_clock = Some(1.0);
    scene.view.atmosphere.material_amount = 0.0;
    let raw = glow(&mut shooter, &scene);
    let hue = raw.as_chunks::<4>().0.iter().max_by_key(|p| p[3]).unwrap();
    scene.view.atmosphere.material_amount = 1.0;
    for resolution in [1.0, 0.75, 0.25] {
        scene.view.atmosphere.stars.star_resolution = resolution;
        let painted = glow(&mut shooter, &scene);
        assert!(
            painted.as_chunks::<4>().0.iter().filter(|p| p[3] > 20).count() > 500,
            "fixture lights the starfield: {resolution}"
        );
        assert!(
            raw.iter().zip(&painted).filter(|(a, b)| a.abs_diff(**b) > 3).count() > 500,
            "Stars must change the note field: {resolution}"
        );
        for p in painted.as_chunks::<4>().0 {
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
        scene.glow_clock = Some(2.0);
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
        scene.glow_clock = Some(1.0);
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
    // The type's own sizes, small and dense enough to sum past coverage:
    // the lattice draws what it stores `LATTICE_STAR_SIZE_SCALE` times over.
    scene.view.atmosphere.stars = harmonigraph_scene::StarSettings::default()
        .scaled(1.0 / harmonigraph_scene::LATTICE_STAR_SIZE_SCALE);
    scene.glow_clock = Some(1.0);
    let mut mean = |material, depth| {
        scene.view.atmosphere.material_style = material;
        scene.view.atmosphere.texture_depth = depth;
        let px = glow(&mut shooter, &scene);
        px.as_chunks::<4>().0.iter().map(luminance).sum::<f64>() / (px.len() / 4) as f64
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
    scene.glow_clock = Some(3.0);
    for (style, amount, resolution, scale) in [
        (LatticeMaterial::Stars, 1.0, 1.0, 1.0),
        (LatticeMaterial::Stars, 1.0, 0.75, 1.0),
        (LatticeMaterial::Stars, 1.0, 0.25, 1.0),
        (LatticeMaterial::Watercolor, 1.0, 0.25, 1.0),
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
/// The glow target here is a quarter of `SIZE` (`GLOW_DIVISOR`), 64 texels
/// tall at render scale 1.0, and this Pattern size puts the detail at about
/// 1.2 texels a cell, two thirds of the way through the fade
/// (`NEBULA_DETAIL_FADE`), and the clouds at about three.
/// Read as the light's curvature between neighbouring glow texels, four pixels
/// apart, over the halo: 0.366 with the fade, 0.417 with it
/// switched off (measured by widening the fade past reach). The clouds' own
/// grain is most of either number, which is why the margin is narrow.
#[test]
fn the_clouds_detail_fades_where_it_is_finer_than_the_glow_texel() {
    // Twice the size the half-size target wanted (#1344): the footprint is the
    // Pattern size times the glow's height in texels, and that height halved
    // with #1461.
    const PATTERN: f32 = 0.22;
    const GLOW_TEXELS: u32 = SIZE[1].div_ceil(crate::lattice_targets::GLOW_DIVISOR);
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
    scene.glow_clock = Some(0.0);
    // Detail cells per glow texel, as `nebula_light` reckons them: the fixture
    // has to stand well inside the fade for this to measure it.
    let detail = 5.0 * 2.3 / (PATTERN * GLOW_TEXELS as f32);
    assert!((0.6..1.0).contains(&detail), "the detail spans {detail} cells a texel, off the fade");
    let smooth = glow(&mut shooter, &scene);
    scene.view.atmosphere.texture_depth = 1.0;
    let textured = glow(&mut shooter, &scene);
    // The texture's share of the light at a pixel, read every fourth pixel so
    // neighbours are neighbouring glow texels rather than the blit's blend.
    let w = SIZE[0] as usize;
    let ratio = |x: usize, y: usize| -> Option<f32> {
        let i = (y * w + x) * 4 + 3;
        (smooth[i] > 30).then(|| f32::from(textured[i]) / f32::from(smooth[i]))
    };
    let (mut curvature, mut count) = (0.0f32, 0);
    for y in (5..SIZE[1] as usize - 5).step_by(4) {
        for x in (5..w - 5).step_by(4) {
            let near =
                [ratio(x, y), ratio(x - 4, y), ratio(x + 4, y), ratio(x, y - 4), ratio(x, y + 4)];
            if let [Some(c), Some(l), Some(r), Some(u), Some(d)] = near {
                curvature += (l + r + u + d - 4.0 * c).abs();
                count += 1;
            }
        }
    }
    assert!(count > 500, "only {count} halo texels to read");
    let curvature = curvature / count as f32;
    assert!(curvature < 0.39, "the detail still grains the light: curvature {curvature}");
}
