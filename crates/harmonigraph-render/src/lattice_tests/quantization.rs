//! Precision inside the lattice pass, and the one quantization left at its
//! boundary with the host surface.

use std::collections::BTreeMap;

use super::fixtures::*;
use crate::*;

#[test]
fn the_lattice_keeps_its_gradients_in_half_floats_until_the_final_composite() {
    assert_eq!(LATTICE_COLOR_FORMAT, wgpu::TextureFormat::Rgba16Float);
    let Some(mut shooter) = Shooter::new([64, 64]) else {
        return;
    };
    let mut scene = single_marked_node(0, 0);
    scene.glow_reach = 0.8;
    scene.bloom_strength = 1.0;
    shooter.shot(&scene);

    // Inspect the resources allocated by the production prepare path, not only
    // the constant intended for them. A matching pipeline/attachment pair can
    // validate and render after both have accidentally regressed to RGBA8.
    let resources: &LatticeResources = shooter.resources.get().expect("the shot made resources");
    let pane = resources.panes.get(&shooter.pane).expect("the shot made its pane");
    let offscreen = pane.offscreen.as_ref().expect("the shot made its scene target");
    assert_eq!(offscreen.format, LATTICE_COLOR_FORMAT);
    assert_eq!(offscreen.bloom.format, LATTICE_COLOR_FORMAT);
    assert_eq!(
        offscreen.glow.as_ref().expect("the positive reach made a glow target").format,
        LATTICE_COLOR_FORMAT,
    );
}

/// Byte-representable flat ink passes through the half-float scene before the
/// final dither. F16 rounding moves some byte values slightly off their exact
/// centres; the dither must leave enough margin that this never turns a flat
/// marker into two output codes.
#[test]
fn final_dither_keeps_byte_representable_flat_ink_flat() {
    const SIZE: [u32; 2] = [128, 128];
    let Some(mut shooter) = Shooter::new(SIZE) else {
        return;
    };
    for code in [143u8, 239] {
        let mut scene = single_marked_node(0, 0);
        scene.camera = harmonigraph_scene::Camera {
            projection: harmonigraph_scene::Projection::Orthographic,
            yaw: 0.0,
            pitch: 0.0,
            ..Default::default()
        };
        scene.nodes.clear();
        scene.glow_reach = 0.0;
        scene.bloom_strength = 0.0;
        for style in scene.shadow.groups_mut() {
            style.depth = 0.0;
        }
        scene.plus_half_width = 1.0;
        scene.plus_taper_start = 1.0;
        let channel = code as f32 / 255.0;
        scene.pluses = vec![one_marker(
            glam::Vec3::ZERO,
            1.2,
            glam::Vec4::new(channel, channel, channel, 1.0),
            1.0,
        )];

        let shot = shooter.shot(&scene);
        let centre = on_screen(&scene, SIZE, glam::Vec3::ZERO).round().as_ivec2();
        for y in centre.y - 5..=centre.y + 5 {
            for x in centre.x - 5..=centre.x + 5 {
                let i = (y as usize * SIZE[0] as usize + x as usize) * 4;
                assert_eq!(
                    &shot[i..i + 4],
                    &[code, code, code, 255],
                    "byte {code} acquired grain at ({x}, {y})",
                );
            }
        }
    }
}

/// A smooth radial glow gives every fragment at one exact squared radius the
/// same ideal value. An eight-bit intermediate makes that entire ring one byte;
/// the final screen-fixed dither instead distributes adjacent byte values
/// around it, so a changing glow cannot present one moving contour at a time.
///
/// The two other parts of the boundary contract live in the same shot: the
/// pattern is stable between frames, and a transparent part of the callback
/// leaves the pane's existing colour byte-exact rather than laying grain over
/// the whole viewport.
#[test]
fn the_final_dither_breaks_radial_quantization_rings_without_temporal_noise() {
    const SIZE: [u32; 2] = [256, 256];
    const CLEAR: [u8; 4] = [64, 96, 128, 255];
    let Some(mut shooter) = Shooter::new(SIZE) else {
        return;
    };
    shooter.clear = wgpu::Color {
        r: CLEAR[0] as f64 / 255.0,
        g: CLEAR[1] as f64 / 255.0,
        b: CLEAR[2] as f64 / 255.0,
        a: 1.0,
    };

    let mut scene = single_marked_node(0, 0);
    scene.camera = harmonigraph_scene::Camera {
        projection: harmonigraph_scene::Projection::Orthographic,
        yaw: 0.0,
        pitch: 0.0,
        ..Default::default()
    };
    scene.background = glam::Vec4::new(
        CLEAR[0] as f32 / 255.0,
        CLEAR[1] as f32 / 255.0,
        CLEAR[2] as f32 / 255.0,
        1.0,
    );
    let grey = glam::Vec4::new(0.72, 0.72, 0.72, 1.0);
    scene.nodes[0].color = grey;
    scene.nodes[0].melody_color = grey;
    scene.nodes[0].bass_color = grey;
    scene.nodes[0].audio_ring = 0.0;
    scene.pitch_lut.fill(grey);
    scene.glow_reach = 1.4;
    scene.glow_strength = 0.35;
    scene.glow_blend = 1.0;
    scene.bloom_strength = 0.0;
    for style in scene.shadow.groups_mut() {
        style.depth = 0.0;
    }

    // Put the radial field on one fragment centre. Then pixels with the same
    // integer dx² + dy² are samples of the same ideal glow value rather than
    // near neighbours being treated as though they were equal.
    let wanted = glam::Vec2::new(SIZE[0] as f32 * 0.5 + 0.5, SIZE[1] as f32 * 0.5 + 0.5);
    for _ in 0..12 {
        scene.camera.pan(wanted - on_screen(&scene, SIZE, glam::Vec3::ZERO));
    }
    let centre = on_screen(&scene, SIZE, glam::Vec3::ZERO);
    assert!(centre.distance(wanted) < 0.001, "the radial fixture landed at {centre:?}");

    let first = shooter.shot(&scene);
    let second = shooter.shot(&scene);
    assert_eq!(first, second, "the final dither changed between identical frames");
    assert_eq!(
        &first[0..4],
        &CLEAR,
        "a transparent corner changed the pane colour under the callback",
    );

    let points_per_world = on_screen(&scene, SIZE, glam::Vec3::X).distance(centre);
    let rim = scene.rings_outer * scene.marker_unit * points_per_world;
    let edge = (scene.rings_outer + scene.glow_reach) * scene.marker_unit * points_per_world;
    let inner2 = (rim + 3.0).powi(2);
    let outer2 = (edge - 3.0).powi(2);
    assert!(outer2 > inner2, "the fixture left no halo-only annulus to measure");

    let centre_px = (wanted.x as i32, wanted.y as i32);
    let mut rings: BTreeMap<i32, Vec<u8>> = BTreeMap::new();
    for y in 0..SIZE[1] as i32 {
        for x in 0..SIZE[0] as i32 {
            let dx = x - centre_px.0;
            let dy = y - centre_px.1;
            let radius2 = dx * dx + dy * dy;
            if (radius2 as f32) < inner2 || (radius2 as f32) > outer2 {
                continue;
            }
            let p = (y as usize * SIZE[0] as usize + x as usize) * 4;
            rings.entry(radius2).or_default().push(first[p]);
        }
    }

    let split_rings = rings
        .values()
        .filter(|samples| samples.len() >= 4)
        .filter(|samples| {
            let min = *samples.iter().min().unwrap();
            let max = *samples.iter().max().unwrap();
            max - min == 1 && max > CLEAR[0]
        })
        .count();
    assert!(
        split_rings >= 16,
        "only {split_rings} equal-radius halo rings were distributed across adjacent byte values",
    );
}
