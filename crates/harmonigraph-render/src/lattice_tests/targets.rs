//! Optional bloom is a resource decision; release history is independent.

use super::fixtures::*;
use crate::*;

fn target(shooter: &Shooter) -> &Offscreen {
    shooter.resources.get::<LatticeResources>().unwrap().panes[&shooter.pane]
        .offscreen
        .as_ref()
        .unwrap()
}

fn history(shooter: &Shooter) -> &InkStrip {
    shooter.resources.get::<LatticeResources>().unwrap().panes[&shooter.pane]
        .ink_history
        .as_ref()
        .unwrap()
}

/// A release has no current ink to reconstruct. Assert pixels as well as GPU
/// identities, then cross the real row-capacity boundary separately: that
/// baseline deliberately replaces and reseeds the strip.
#[test]
fn bloom_toggles_preserve_release_history_and_only_replace_bloom() {
    let Some(mut shooter) = Shooter::new([256, 256]) else { return };
    let mut scene = single_marked_node(0, 0);
    scene.glow_reach = 0.8;
    scene.glow_strength = 1.5;
    scene.bloom_strength = 1.0;
    scene.pitch_lut = [glam::Vec4::new(1.0, 0.1, 0.0, 1.0); harmonigraph_scene::PITCH_LUT_N];
    scene.nodes[0].octaves = [1.0; harmonigraph_scene::OCTAVE_SLOTS];
    scene.nodes[0].activation = 1.0;
    scene.nodes[0].audio_ring = 0.0;
    shooter.shot(&scene);
    // Nothing but the held GPU colour remains.
    scene.nodes[0].activation = 0.0;
    scene.nodes[0].octaves.fill(0.0);
    scene.nodes[0].glow.mix = 0.0;
    let bloomed = shooter.shot_again(&scene);
    assert!(total_light(&bloomed) > 64, "release fixture must emit visible light");
    let color = target(&shooter).color_view.clone();
    let glow = target(&shooter).glow.as_ref().unwrap().view.clone();
    let raw = history(&shooter).raw_views.clone();
    let initial_bloom = target(&shooter).bloom.as_ref().unwrap().nodes_view.clone();
    let bytes = |view: &wgpu::TextureView| {
        let tex = view.texture();
        u64::from(tex.width()) * u64::from(tex.height()) * 8
    };
    let bloom = target(&shooter).bloom.as_ref().unwrap();
    let bloom_bytes = [
        &bloom.nodes_view,
        &bloom.chain.half_view,
        &bloom.chain.quarter_a_view,
        &bloom.chain.quarter_b_view,
    ]
    .into_iter()
    .map(bytes)
    .sum::<u64>();
    assert_eq!(bloom_bytes, 720_896, "256-square pane: nodes plus half and two quarters, RGBA16F");
    assert_eq!(bytes(&color), 524_288);

    // Changing strength while enabled must keep the same allocation.
    scene.bloom_strength = 0.5;
    shooter.shot_again(&scene);
    assert_eq!(target(&shooter).bloom.as_ref().unwrap().nodes_view, initial_bloom);
    scene.bloom_strength = 0.0;
    let plain = shooter.shot_again(&scene);
    assert!(target(&shooter).bloom.is_none());
    assert!(total_light(&plain) > 64 && total_light(&plain) < total_light(&bloomed));
    assert_eq!(plain, shooter.shot_again(&scene), "steady off frame");
    for _ in 0..3 {
        scene.bloom_strength = 1.0;
        assert_eq!(bloomed, shooter.shot_again(&scene), "on recovers identical pixels");
        assert_ne!(target(&shooter).bloom.as_ref().unwrap().nodes_view, initial_bloom);
        scene.bloom_strength = 0.0;
        assert_eq!(plain, shooter.shot_again(&scene), "off recovers identical pixels");
        let held = target(&shooter);
        assert!(held.bloom.is_none());
        assert_eq!(held.color_view, color);
        assert_eq!(held.glow.as_ref().unwrap().view, glow);
        assert_eq!(history(&shooter).raw_views, raw);
    }
    // Both a native resize and a render-scale change carry the same rows,
    // including when the same frame turns bloom back on.
    shooter.size = [256, 260];
    for scale in [1.0, 2.0] {
        scene.render_scale = scale;
        scene.bloom_strength = 1.0;
        let resized = shooter.shot_again(&scene);
        assert!(total_light(&resized) > total_light(&plain) / 2);
        assert_eq!(history(&shooter).raw_views, raw);
        assert_ne!(target(&shooter).color_view, color);
    }
    // Preserve the current growth/reseed contract, which does not carry an
    // inkless release through growth. This really crosses capacity 1 -> 2.
    scene.glow_rows = 2;
    scene.nodes[0].glow.mix = 1.0;
    let grown = shooter.shot_again(&scene);
    assert_eq!(history(&shooter).rows, 2);
    assert_ne!(history(&shooter).raw_views, raw);
    assert_eq!(total_light(&grown), 0, "growth reseeds from empty current ink");
    // Reusing row zero seeds its new green owner instead of stale red.
    scene.nodes[0].activation = 1.0;
    scene.nodes[0].octaves.fill(1.0);
    scene.pitch_lut.fill(glam::Vec4::new(0.0, 1.0, 0.0, 1.0));
    let reused = shooter.shot_again(&scene);
    let fresh = shooter.shot(&scene);
    assert_eq!(reused, fresh, "a reused row is a fresh owner's ink");
    // Glow-off drops its history under the existing feature contract; on
    // with current ink reseeds and draws exactly the fresh frame.
    scene.glow_reach = 0.0;
    shooter.shot_again(&scene);
    assert!(target(&shooter).glow.is_none());
    scene.glow_reach = 0.8;
    assert_eq!(fresh, shooter.shot_again(&scene));
    // Silence while enabled retains the same bloom allocation for the next
    // note. Empty geometry gates allocation, not retention.
    let held_bloom = target(&shooter).bloom.as_ref().unwrap().nodes_view.clone();
    let nodes = std::mem::take(&mut scene.nodes);
    let pluses = std::mem::take(&mut scene.pluses);
    shooter.shot_again(&scene);
    assert_eq!(target(&shooter).bloom.as_ref().unwrap().nodes_view, held_bloom);
    scene.nodes = nodes;
    scene.pluses = pluses;
    shooter.shot_again(&scene);
    assert_eq!(target(&shooter).bloom.as_ref().unwrap().nodes_view, held_bloom);
    scene.nodes.clear();
    scene.pluses.clear();
    // Switching off while empty still retires the allocation, and switching
    // back on waits for a scene to draw before allocating again.
    scene.bloom_strength = 0.0;
    shooter.shot_again(&scene);
    assert!(target(&shooter).bloom.is_none());
    scene.bloom_strength = 1.0;
    shooter.shot_again(&scene);
    assert!(target(&shooter).bloom.is_none(), "no allocation before a scene can write it");
}

fn history_scene() -> Scene {
    let mut scene = single_marked_node(0, 0);
    scene.glow_reach = 0.8;
    scene.glow_strength = 1.5;
    scene.bloom_strength = 0.0;
    scene.glow_rows = 64;
    scene.pitch_lut.fill(glam::Vec4::new(1.0, 0.0, 0.0, 1.0));
    scene.nodes[0].octaves.fill(1.0);
    scene.nodes[0].audio_ring = 0.0;
    scene
}

fn release(scene: &mut Scene) {
    for node in &mut scene.nodes {
        node.activation = 0.0;
        node.octaves.fill(0.0);
        node.glow.mix = 0.0;
    }
}

/// Each resized release is compared byte-exact with a separate pane seeded
/// at that viewport. A third pane carries a different colour and advances its
/// own parity. Constructor counts include any temporary strip before adoption.
#[test]
fn viewport_changes_keep_history_without_allocating_a_strip() {
    let Some(mut shooter) = Shooter::new([256, 256]) else { return };
    let mut scene = history_scene();
    shooter.pane = 10;
    shooter.shot_again(&scene);
    release(&mut scene);
    let red = shooter.shot_again(&scene);
    assert!(total_light(&red) > 64);
    let raw = history(&shooter).raw_views.clone();
    let bindings = history(&shooter).raw_bind_groups.clone();
    let blurred = history(&shooter).blurred_view.clone();
    let mut parity = history(&shooter).parity;
    let mut color = target(&shooter).color_view.clone();
    let mut light = target(&shooter).glow.as_ref().unwrap().view.clone();

    let mut preview = history_scene();
    preview.pitch_lut.fill(glam::Vec4::new(0.0, 1.0, 0.0, 1.0));
    shooter.pane = 20;
    shooter.shot_again(&preview);
    release(&mut preview);
    let green = shooter.shot_again(&preview);
    assert!(total_light(&green) > 64 && green != red);
    let preview_raw = history(&shooter).raw_views.clone();
    assert_ne!(preview_raw, raw);
    let mut preview_parity = history(&shooter).parity;

    for (i, (size, scale)) in
        [([256, 260], 1.0), ([256, 260], 2.0), ([256, 256], 2.0), ([256, 256], 1.0)]
            .into_iter()
            .enumerate()
    {
        shooter.pane = 10;
        shooter.size = size;
        scene.render_scale = scale;
        let creations = super::INK_STRIP_CREATIONS.get();
        let resized = shooter.shot_again(&scene);
        assert_eq!(super::INK_STRIP_CREATIONS.get(), creations, "no temporary strip on resize");
        assert_eq!(history(&shooter).raw_views, raw);
        assert_eq!(history(&shooter).raw_bind_groups, bindings);
        assert_eq!(history(&shooter).blurred_view, blurred);
        assert_eq!(history(&shooter).parity, parity ^ 1);
        parity ^= 1;
        assert_ne!(target(&shooter).color_view, color);
        assert_ne!(target(&shooter).glow.as_ref().unwrap().view, light);
        color = target(&shooter).color_view.clone();
        light = target(&shooter).glow.as_ref().unwrap().view.clone();
        assert!(total_light(&resized) > 64, "positive glow has no current ink");

        // Control: seed the same shader ink at the final viewport, then let
        // it release. No CPU reconstruction and no resize in this pane.
        shooter.pane = 100 + i as u64;
        let mut control = history_scene();
        control.render_scale = scale;
        shooter.shot_again(&control);
        release(&mut control);
        assert_eq!(resized, shooter.shot_again(&control));

        shooter.pane = 20;
        shooter.size = [256, 256];
        assert_eq!(green, shooter.shot_again(&preview));
        assert_eq!(history(&shooter).raw_views, preview_raw);
        assert_eq!(history(&shooter).parity, preview_parity ^ 1);
        preview_parity ^= 1;
        shooter.pane = 10;
        assert_eq!(history(&shooter).parity, parity, "another pane cannot advance this one");
    }
    eprintln!("history viewport fixture: 4 target recreations, 0 strip creations; 4 byte-exact controls, independent red/green panes");
}

/// This directly supplies renderer inputs, not an execution of the CPU row
/// allocator. Capacity really changes 64 -> 128 and a held node writes row 64,
/// beyond the old texture. Both rows receive the allocator's reseed signal.
#[test]
fn capacity_growth_reseeds_current_ink_and_row_reuse_keeps_identity() {
    let Some(mut shooter) = Shooter::new([256, 256]) else { return };
    let mut scene = history_scene();
    scene.node_radius = 0.6;
    scene.nodes[0].world_pos.x = -1.2;
    shooter.shot_again(&scene);
    release(&mut scene);
    assert!(total_light(&shooter.shot_again(&scene)) > 64);
    let raw = history(&shooter).raw_views.clone();
    let color = target(&shooter).color_view.clone();
    let light = target(&shooter).glow.as_ref().unwrap().view.clone();
    let mut held = scene.nodes[0];
    held.world_pos.x = 1.2;
    held.lattice_pos = harmonigraph_core::LatticePos::new(1, 0, 0);
    held.glow.row = 64;
    held.activation = 1.0;
    held.octaves.fill(1.0);
    scene.nodes.push(held);
    scene.glow_rows = 128;
    scene.pitch_lut.fill(glam::Vec4::new(0.0, 1.0, 0.0, 1.0));
    for node in &mut scene.nodes {
        node.glow.mix = 1.0;
    }
    let creations = super::INK_STRIP_CREATIONS.get();
    let grown = shooter.shot_again(&scene);
    assert_eq!(super::INK_STRIP_CREATIONS.get(), creations + 1);
    assert_eq!(history(&shooter).rows, 128);
    assert_eq!(history(&shooter).raw_views[0].texture().height(), 128);
    assert_ne!(history(&shooter).raw_views, raw);
    assert_eq!(history(&shooter).parity, 1, "new history starts at parity zero, then advances");
    assert_eq!(target(&shooter).color_view, color);
    assert_eq!(target(&shooter).glow.as_ref().unwrap().view, light);
    assert_eq!(
        shooter.resources.get::<LatticeResources>().unwrap().panes[&shooter.pane].instance_count,
        2,
        "the old release and the newly writable row must both reach the GPU"
    );
    assert_eq!(grown.chunks_exact(4).map(|p| u64::from(p[0])).sum::<u64>(), 0);
    assert!(grown.chunks_exact(4).map(|p| u64::from(p[1])).sum::<u64>() > 64);
    let pane = shooter.pane;
    assert_eq!(grown, shooter.shot(&scene), "growth is exactly fresh current ink");
    shooter.pane = pane;
    let grown_raw = history(&shooter).raw_views.clone();
    // Dropping row zero's old owner and changing draw order cannot reassign
    // row 64. It now releases its green while zero seeds blue, never red.
    scene.nodes.remove(0);
    let mut reused = scene.nodes[0];
    release(&mut scene);
    reused.glow.row = 0;
    reused.world_pos.x = -1.2;
    reused.lattice_pos = harmonigraph_core::LatticePos::new(-1, 0, 0);
    scene.nodes.push(reused);
    scene.pitch_lut.fill(glam::Vec4::new(0.0, 0.0, 1.0, 1.0));
    let reused = shooter.shot_again(&scene);
    assert_eq!(history(&shooter).raw_views, grown_raw);
    assert_eq!(reused.chunks_exact(4).map(|p| u64::from(p[0])).sum::<u64>(), 0);
    assert!(reused.chunks_exact(4).map(|p| u64::from(p[1])).sum::<u64>() > 64);
    assert!(reused.chunks_exact(4).map(|p| u64::from(p[2])).sum::<u64>() > 64);
    scene.nodes.swap(0, 1);
    assert_eq!(reused, shooter.shot_again(&scene), "row identity survives instance reordering");
    eprintln!("history growth fixture: 64 -> 128, 2 GPU instances including row 64, 1 strip creation, 0 viewport recreations; release red lost, held green present, reuse byte-exact");
}

/// Empty geometry keeps the existing maintenance/parity path. Once drawable
/// geometry returns, glow-off must discard both objects so an inkless release
/// cannot resurrect a colour retained by the new history owner.
#[test]
fn glow_off_discards_history_when_target_maintenance_runs() {
    let Some(mut shooter) = Shooter::new([256, 256]) else { return };
    let mut scene = history_scene();
    shooter.shot_again(&scene);
    release(&mut scene);
    let red = shooter.shot_again(&scene);
    assert!(total_light(&red) > 64);
    let raw = history(&shooter).raw_views.clone();
    let color = target(&shooter).color_view.clone();
    let mut parity = history(&shooter).parity;
    let nodes = std::mem::take(&mut scene.nodes);
    scene.glow_reach = 0.0;
    shooter.size = [256, 260];
    let creations = super::INK_STRIP_CREATIONS.get();
    for _ in 0..2 {
        shooter.shot_again(&scene);
        assert_eq!(history(&shooter).raw_views, raw);
        assert_eq!(target(&shooter).color_view, color);
        assert!(target(&shooter).glow.is_some());
        assert_eq!(history(&shooter).parity, parity ^ 1);
        parity ^= 1;
    }
    assert_eq!(super::INK_STRIP_CREATIONS.get(), creations);
    scene.nodes = nodes;
    shooter.shot_again(&scene);
    assert!(target(&shooter).glow.is_none());
    assert!(shooter.resources.get::<LatticeResources>().unwrap().panes[&shooter.pane]
        .ink_history
        .is_none());
    scene.glow_reach = 0.8;
    let reset = shooter.shot_again(&scene);
    assert_eq!(super::INK_STRIP_CREATIONS.get(), creations + 1);
    assert_ne!(history(&shooter).raw_views, raw);
    assert_eq!(total_light(&reset), 0, "glow on cannot resurrect the discarded red history");
}
