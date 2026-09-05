//! Optional bloom is a resource decision; release history is independent.

use super::fixtures::*;
use crate::*;

fn target(shooter: &Shooter) -> &Offscreen {
    shooter.resources.get::<LatticeResources>().unwrap().panes[&shooter.pane]
        .offscreen
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
    let raw = target(&shooter).glow.as_ref().unwrap().strip.raw_views.clone();
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
        assert_eq!(held.glow.as_ref().unwrap().strip.raw_views, raw);
    }
    // Both a native resize and a render-scale change carry the same rows,
    // including when the same frame turns bloom back on.
    shooter.size = [256, 260];
    for scale in [1.0, 2.0] {
        scene.render_scale = scale;
        scene.bloom_strength = 1.0;
        let resized = shooter.shot_again(&scene);
        assert!(total_light(&resized) > total_light(&plain) / 2);
        assert_eq!(target(&shooter).glow.as_ref().unwrap().strip.raw_views, raw);
        assert_ne!(target(&shooter).color_view, color);
    }
    // Preserve the current growth/reseed contract, which does not carry an
    // inkless release through growth. This really crosses capacity 1 -> 2.
    scene.glow_rows = 2;
    scene.nodes[0].glow.mix = 1.0;
    let grown = shooter.shot_again(&scene);
    assert_eq!(target(&shooter).glow.as_ref().unwrap().strip.rows, 2);
    assert_ne!(target(&shooter).glow.as_ref().unwrap().strip.raw_views, raw);
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
    // Empty geometry still retires the bloom allocation. It must not keep a
    // previous enabled frame's four images merely because sizing was skipped.
    scene.nodes.clear();
    scene.pluses.clear();
    scene.bloom_strength = 0.0;
    shooter.shot_again(&scene);
    assert!(target(&shooter).bloom.is_none());
    scene.bloom_strength = 1.0;
    shooter.shot_again(&scene);
    assert!(target(&shooter).bloom.is_none(), "no allocation before a scene can write it");
}
