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

/// A light lighting `slot` alone, at a level of 0.8 and `thickness` times the
/// band's width.
fn slice(slot: usize, thickness: f32) -> GlowLight {
    let mut light = GlowLight::default();
    light.l[slot] = 0.8;
    light.p[slot] = 0.8 * thickness;
    light
}

/// The light a row carries in all, summed round the turn.
fn weight(texels: &[[f32; 4]]) -> f32 {
    texels.iter().map(|texel| texel[3]).sum()
}

/// The thickness at which a slice on the [`fixture`]'s rings reaches the
/// swell cap, as the shader draws it: a slice far past any cap, read against
/// one at rest, rather than a copy of `swell_cap` and its constants.
fn cap_thickness(shooter: &mut Shooter, slot: usize) -> f32 {
    let (mut probe, _, cents) = fixture();
    probe.nodes =
        [1.0, 1000.0].iter().map(|&t| lit_node(&probe, t * 1e-3, slice(slot, t))).collect();
    let rows = strip_rows(shooter, &probe);
    weight(row_of(&rows, cents, slice(slot, 1000.0)))
        / weight(row_of(&rows, cents, slice(slot, 1.0)))
}

/// One node lit by `light`, at `x`, on the band-and-marks fixture.
fn lit_node(scene: &Scene, x: f32, light: GlowLight) -> harmonigraph_scene::NodeInstance {
    let mut node = scene.nodes[0];
    node.world_pos.x = x;
    node.lattice_pos = harmonigraph_core::LatticePos::new((x * 10.0) as i32, 0, 0);
    node.glow = GlowStep { level: 1.0, light: Some(light) };
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
/// weights. Random lights, the band thinned and swelled below the cap and the
/// marks on slots no two of which are neighbours round the wheel (whose seam
/// joins its two ends) — the two places it is not linear, the swell cap and
/// the per-angle max over marks, are the listed exceptions — each stepped by
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
    let below_cap = 0.9 * cap_thickness(&mut shooter, low);
    assert!(below_cap > 1.2, "the fixture's cap leaves no room to swell: {below_cap}");
    let mut seed = 0x9e37_79b9_u32;
    let mut rand = move || {
        seed ^= seed << 13;
        seed ^= seed >> 17;
        seed ^= seed << 5;
        seed as f32 / u32::MAX as f32
    };
    let mut light = |marks: &[usize]| {
        let l: [f32; OCTAVE_SLOTS] = std::array::from_fn(|_| rand());
        let p = std::array::from_fn(|i| l[i] * below_cap * rand());
        let mut m = [0.0; OCTAVE_SLOTS];
        for &slot in marks {
            m[slot] = rand();
        }
        GlowLight { l, p, m }
    };
    let mut trials = Vec::new();
    for mix in [0.003, 0.05, 0.5, 0.003, 0.05, 0.5] {
        let (a, b) = (light(&[low, low + 2]), light(&[low + 4]));
        let mut stepped = a;
        stepped.step(&b, mix);
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

/// A slice's light grows with its thickness up to the swell cap and no
/// further — where a swelled slice stops being drawn wider (`reach_at`, which
/// `ink_at` asks for the reach).
///
/// The cap is read off the shader (`cap_thickness`), so what is pinned is the
/// shape: proportional below it, right up to it, and flat past it.
#[test]
fn a_slices_light_grows_with_its_thickness_up_to_the_cap() {
    let Some(mut shooter) = Shooter::new([64, 64]) else {
        return;
    };
    let (mut scene, slots, cents) = fixture();
    let slot = (*slots.start() + *slots.end()) / 2;
    let cap = cap_thickness(&mut shooter, slot);
    assert!(cap > 1.2, "the fixture's cap leaves no room to swell: {cap}");
    let at = |thickness: f32| slice(slot, thickness);
    let thicknesses = [0.5, 1.0, cap * 0.9, cap * 1.5, cap * 4.0];
    scene.nodes =
        thicknesses.iter().enumerate().map(|(i, &t)| lit_node(&scene, i as f32, at(t))).collect();
    let rows = strip_rows(&mut shooter, &scene);
    let weight = |t: f32| weight(row_of(&rows, cents, at(t)));
    let one = weight(1.0);
    assert!(one > 0.5, "the slice lit nothing: {one}");
    for t in [0.5, cap * 0.9] {
        let ratio = weight(t) / one;
        assert!((ratio - t).abs() < 1e-4, "at {t}x the slice's light is {ratio}x its rest");
    }
    for t in [cap * 1.5, cap * 4.0] {
        let ratio = weight(t) / one;
        assert!(
            (ratio - cap).abs() < 1e-4,
            "at {t}x, past the cap of {cap}x, the light is {ratio}x"
        );
    }
}

/// A mark's light wears the colour the CPU gives the mark it draws: its own
/// slot's pitch through the gradient, read in WGSL (`ink_at`) where the mark's
/// ink takes `NodeInstance::melody_color` and `bass_color` off the CPU.
///
/// A real chord through `derive_scene` and `NodeMotion`, so the colours are
/// the ones the app hands the marks, on its melody's and its bass's nodes. Each
/// marked node is lit by its mark alone, on the mark's own slot.
#[test]
fn the_halos_mark_colour_is_its_slots() {
    let Some(mut shooter) = Shooter::new([64, 64]) else {
        return;
    };
    use harmonigraph_core::{NoteEvent, NoteTracker, SourceId, Tuning};
    use harmonigraph_scene::{Camera, FrameParams, ViewConfig};
    let mut tracker = NoteTracker::new();
    for note in [60u8, 64, 67] {
        tracker.handle_event(NoteEvent::on(0.0, SourceId::DIRECT, 0, note, 1.0));
    }
    let view = ViewConfig {
        extent_threes: 2,
        extent_fives: 2,
        min_sevens: 0,
        max_sevens: 0,
        glow_reach: 0.8,
        ..ViewConfig::default()
    };
    let frame = FrameParams { fade_time: 0.0, ..FrameParams::default() };
    let mut scene = animated_snapshot(
        &tracker,
        &Tuning::default(),
        &view,
        &view.reach(),
        &frame,
        Camera::default(),
        None,
        0.5,
    );
    let mut probes = Vec::new();
    for node in &mut scene.nodes {
        let (slots, colour) = if node.melody_slots != 0 {
            (node.melody_slots, node.melody_color)
        } else {
            (node.bass_slots, node.bass_color)
        };
        if slots == 0 {
            node.glow = GlowStep::default();
            continue;
        }
        let mut light = GlowLight::default();
        light.m[slots.trailing_zeros() as usize] = 1.0;
        node.glow = GlowStep { level: 1.0, light: Some(light) };
        probes.push((node.cents, light, colour));
    }
    let classes: std::collections::BTreeSet<u32> = probes.iter().map(|p| p.0.to_bits()).collect();
    assert!(classes.len() >= 2, "the chord marked {} pitch classes", classes.len());
    let rows = strip_rows(&mut shooter, &scene);
    for (cents, light, colour) in probes {
        let texels = row_of(&rows, cents, light);
        let lit: Vec<_> = texels.iter().filter(|t| t[3] > 1e-4).collect();
        assert!(!lit.is_empty(), "the mark at {cents} cents lit nothing");
        for texel in lit {
            let read = glam::Vec3::new(texel[0], texel[1], texel[2]) / texel[3];
            assert!(
                (read - colour.truncate()).abs().max_element() < 1e-3,
                "the halo reads the mark at {cents} cents as {read}, which the mark draws in {colour}",
            );
        }
    }
}
