//! What a light is made of, read round its node: the ink strip's reading of a
//! light table row (`fs_ink_strip`), which has to be linear in the weights a
//! light carries for a carried light's colour to ease as its weights do.

use super::fixtures::*;
use crate::gpu_harness::readback;
use crate::*;
use harmonigraph_scene::{GlowLight, GlowStep, OCTAVE_SLOTS};

/// Every lit node's row of the light table `scene` uploads, with that row of
/// the raw ink strip it was read into: per angle, the colour premultiplied by
/// the weight and the weight (`ink_at`).
fn strip_rows(shooter: &mut Shooter, scene: &Scene) -> Vec<(LightRow, Vec<[f32; 4]>)> {
    shooter.pane += 1;
    let size = shooter.size;
    let screen = ScreenDescriptor { size_in_pixels: size, pixels_per_point: 1.0 };
    let points = egui::vec2(size[0] as f32, size[1] as f32);
    let cb = LatticeCallback::from_scene(
        scene,
        LatticeLabels::default(),
        points,
        shooter.format,
        shooter.pane,
        None,
    );
    let mut encoder = shooter.device.create_command_encoder(&Default::default());
    let bufs =
        cb.prepare(&shooter.device, &shooter.queue, &screen, &mut encoder, &mut shooter.resources);
    shooter.queue.submit(bufs.into_iter().chain([encoder.finish()]));
    let resources = shooter.resources.get::<LatticeResources>().expect("prepare built them");
    let strip = resources.panes[&shooter.pane].ink_strip.as_ref().expect("the scene draws a glow");
    let size = [INK_STRIP_N, strip.rows];
    let bytes = readback(&shooter.device, &shooter.queue, strip.raw_view.texture(), size);
    let texels: Vec<[f32; 4]> = bytemuck::pod_collect_to_vec(&bytes);
    let n = INK_STRIP_N as usize;
    cb.lights
        .iter()
        .enumerate()
        .map(|(row, &light)| (light, texels[row * n..][..n].to_vec()))
        .collect()
}

/// The texels of the row whose light is `light` on a node at `cents`.
fn row_of(rows: &[(LightRow, Vec<[f32; 4]>)], cents: f32, light: GlowLight) -> &[[f32; 4]] {
    let want = LightRow::new(cents, light);
    let bytes = |row: &LightRow| bytemuck::bytes_of(row).to_vec();
    let (_, texels) =
        rows.iter().find(|(row, _)| bytes(row) == bytes(&want)).expect("a row for every light");
    texels
}

/// One node lit by `light`, at `x`, on the band-and-marks fixture.
fn lit_node(scene: &Scene, x: f32, light: GlowLight) -> harmonigraph_scene::NodeInstance {
    let mut node = scene.nodes[0];
    node.world_pos.x = x;
    node.lattice_pos = harmonigraph_core::LatticePos::new((x * 10.0) as i32, 0, 0);
    node.glow = GlowStep { level: 1.0, light };
    node
}

/// The fixture: one marked node with the glow on, its slots and its cents.
fn fixture() -> (Scene, std::ops::RangeInclusive<usize>, f32) {
    let mut scene = single_marked_node(0, 0);
    scene.view.glow_reach = 0.8;
    scene.view.glow_strength = 1.5;
    let cents = scene.nodes[0].cents;
    let (low, high) = scene.octave_layout.slots(cents);
    (scene, low.max(0) as usize..=(high as usize).min(OCTAVE_SLOTS - 1), cents)
}

/// A light stepped part of the way between two others reads as their two
/// readings mixed by the same amount (#1759).
///
/// The guarantee a carried light's colour rests on: `GlowFade` steps every
/// weight toward its target with the level's own coefficient and nothing
/// steps the colour, so the halo eases only because `ink_at` is linear in the
/// weights. Random lights, with the marks on slots no two of which are
/// neighbours round the wheel (whose seam joins its two ends) — the one place
/// it is not linear, the per-angle max over marks, is the listed exception —
/// each stepped by
/// `GlowLight::step` itself, at a coefficient a 144 Hz frame takes, one a
/// 60 Hz frame takes, and half.
#[test]
fn a_stepped_light_reads_as_its_two_readings_mixed() {
    let Some(mut shooter) = Shooter::new([64, 64]) else {
        return;
    };
    let (mut scene, slots, cents) = fixture();
    let low = *slots.start();
    assert!(slots.end() - low >= 6, "the wheel shows too few slots for three marks: {slots:?}");
    let mut seed = 0x9e37_79b9_u32;
    let mut rand = move || {
        seed ^= seed << 13;
        seed ^= seed >> 17;
        seed ^= seed << 5;
        seed as f32 / u32::MAX as f32
    };
    let mut light = |marks: &[usize]| {
        let l: [f32; OCTAVE_SLOTS] = std::array::from_fn(|_| rand());
        let mut m = [0.0; OCTAVE_SLOTS];
        for &slot in marks {
            m[slot] = rand();
        }
        GlowLight { l, m }
    };
    let mut trials = Vec::new();
    for mix in [0.003, 0.05, 0.5, 0.003, 0.05, 0.5] {
        let (a, b) = (light(&[low, low + 2]), light(&[low + 4]));
        let mut stepped = a;
        stepped.step(&b, mix, mix);
        trials.push((a, b, stepped, mix));
    }
    let template = scene.nodes[0];
    scene.nodes = trials
        .iter()
        .enumerate()
        .flat_map(|(i, (a, b, stepped, _))| {
            let x = i as f32 * 0.1;
            [
                lit_node(&scene, x, *a),
                lit_node(&scene, x + 0.03, *b),
                lit_node(&scene, x + 0.06, *stepped),
            ]
        })
        .map(|node| harmonigraph_scene::NodeInstance { cents: template.cents, ..node })
        .collect();
    let rows = strip_rows(&mut shooter, &scene);
    assert_eq!(rows.len(), trials.len() * 3, "every lit node has a row");
    for (trial, (a, b, stepped, mix)) in trials.iter().enumerate() {
        let (a, b, stepped) =
            (row_of(&rows, cents, *a), row_of(&rows, cents, *b), row_of(&rows, cents, *stepped));
        let weight: f32 = stepped.iter().map(|t| t[3]).sum();
        assert!(weight > 1.0, "trial {trial}: the stepped light lit nothing: {weight}");
        for (col, ((a, b), got)) in a.iter().zip(b).zip(stepped).enumerate() {
            for c in 0..4 {
                let want = a[c] + (b[c] - a[c]) * mix;
                assert!(
                    (got[c] - want).abs() <= 2e-5 * want.abs().max(1.0),
                    "trial {trial} at {mix}, column {col} channel {c}: read {} where the two \
                     readings mixed are {want}",
                    got[c],
                );
            }
        }
    }
}
