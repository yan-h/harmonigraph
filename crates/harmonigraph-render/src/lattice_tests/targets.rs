//! Optional bloom is a resource decision, and the pane's other targets and its
//! ink strip are independent of it and of the pane's size.

use super::fixtures::*;
use crate::*;

fn target(shooter: &Shooter) -> &Offscreen {
    shooter.resources.get::<LatticeResources>().unwrap().panes[&shooter.pane]
        .offscreen
        .as_ref()
        .unwrap()
}

fn strip(shooter: &Shooter) -> &InkStrip {
    shooter.resources.get::<LatticeResources>().unwrap().panes[&shooter.pane]
        .ink_strip
        .as_ref()
        .unwrap()
}

/// Bloom toggles replace the bloom chain and nothing else; a resize or a
/// render-scale change replaces the scene targets and leaves the ink strip;
/// silence keeps what is allocated, and nothing is allocated before a scene
/// can write it.
///
/// Drawn on an inkless node lit by its carried light alone, so every frame's
/// light is the light the strip reads from the table that frame.
#[test]
fn bloom_and_size_changes_replace_only_their_own_targets() {
    let Some(mut shooter) = Shooter::new([256, 256]) else { return };
    let mut scene = single_marked_node(0, 0);
    scene.view.glow_reach = 0.8;
    scene.view.glow_strength = 1.5;
    scene.view.note_bloom = 1.0;
    scene.pitch_lut = [glam::Vec4::new(1.0, 0.1, 0.0, 1.0); harmonigraph_scene::PITCH_LUT_N];
    let node = &mut scene.nodes[0];
    node.octaves = [1.0; harmonigraph_scene::OCTAVE_SLOTS];
    node.audio_ring = 0.0;
    let light = node.glow_light();
    node.octaves.fill(0.0);
    node.presence_with_opacity = 0.0;
    node.melody_slots = 0;
    node.melody_level = 0.0;
    node.glow = harmonigraph_scene::GlowStep { level: 0.8, light: Some(light) };
    let bloomed = shooter.shot(&scene);
    assert!(total_light(&bloomed) > 64, "the carried light must be visible");
    let color = target(&shooter).color_view.clone();
    let ink = target(&shooter).ink_view.clone();
    let glow = target(&shooter).glow.as_ref().unwrap().view.clone();
    let raw = strip(&shooter).raw_view.clone();
    let initial_bloom = target(&shooter).bloom.as_ref().unwrap().input_view.clone();
    let bytes = |view: &wgpu::TextureView, texel: u64| {
        let tex = view.texture();
        u64::from(tex.width()) * u64::from(tex.height()) * texel
    };
    let bloom = target(&shooter).bloom.as_ref().unwrap();
    let bloom_bytes = [
        &bloom.input_view,
        &bloom.chain.half_view,
        &bloom.chain.quarter_a_view,
        &bloom.chain.quarter_b_view,
    ]
    .into_iter()
    .map(|view| bytes(view, 8))
    .sum::<u64>();
    assert_eq!(
        bloom_bytes, 720_896,
        "256-square pane: bloom input plus half and two quarters, RGBA16F"
    );
    assert_eq!(bytes(&color, 8), 524_288);
    assert_eq!(bytes(&ink, 8), 524_288);
    // The strip at its floor of 64 rows: the raw reading and its blur, one
    // column wider, at RGBA32F, and the light table beside them.
    let held = strip(&shooter);
    assert_eq!(bytes(&held.raw_view, 16) + bytes(&held.blurred_view, 16), 132_096);
    assert_eq!(held.lights.size(), 64 * 136);

    // Changing strength while enabled must keep the same allocation.
    scene.view.note_bloom = 0.5;
    shooter.shot_again(&scene);
    assert_eq!(target(&shooter).bloom.as_ref().unwrap().input_view, initial_bloom);
    scene.view.note_bloom = 0.0;
    let plain = shooter.shot_again(&scene);
    assert!(target(&shooter).bloom.is_none());
    assert!(total_light(&plain) > 64 && total_light(&plain) < total_light(&bloomed));
    assert_eq!(plain, shooter.shot_again(&scene), "steady off frame");
    for _ in 0..3 {
        scene.view.note_bloom = 1.0;
        assert_eq!(bloomed, shooter.shot_again(&scene), "on recovers identical pixels");
        assert_ne!(target(&shooter).bloom.as_ref().unwrap().input_view, initial_bloom);
        scene.view.note_bloom = 0.0;
        assert_eq!(plain, shooter.shot_again(&scene), "off recovers identical pixels");
        let held = target(&shooter);
        assert!(held.bloom.is_none());
        assert_eq!(held.color_view, color);
        assert_eq!(held.ink_view, ink);
        assert_eq!(held.glow.as_ref().unwrap().view, glow);
        assert_eq!(strip(&shooter).raw_view, raw);
    }
    // A native resize and a render-scale change replace the scene targets and
    // keep the strip, including when the same frame turns bloom back on.
    shooter.size = [256, 260];
    for scale in [1.0, 2.0] {
        scene.view.render_scale = scale;
        scene.view.note_bloom = 1.0;
        let resized = shooter.shot_again(&scene);
        assert!(total_light(&resized) > total_light(&plain) / 2);
        assert_eq!(strip(&shooter).raw_view, raw);
        assert_ne!(target(&shooter).color_view, color);
        assert_ne!(target(&shooter).ink_view, ink);
    }
    // Glow off drops the glow target, and on draws exactly the frame it drew
    // before. With ink on the node, which is what keeps a glow-off frame
    // drawable, and so target maintenance running.
    scene.nodes[0].octaves.fill(1.0);
    scene.nodes[0].presence_with_opacity = 1.0;
    let fresh = shooter.shot(&scene);
    scene.view.glow_reach = 0.0;
    shooter.shot_again(&scene);
    assert!(target(&shooter).glow.is_none());
    scene.view.glow_reach = 0.8;
    assert_eq!(fresh, shooter.shot_again(&scene));
    // Silence while enabled retains the same bloom allocation for the next
    // note. Empty geometry gates allocation, not retention.
    let held_bloom = target(&shooter).bloom.as_ref().unwrap().input_view.clone();
    let nodes = std::mem::take(&mut scene.nodes);
    let pluses = std::mem::take(&mut scene.pluses);
    shooter.shot_again(&scene);
    assert_eq!(target(&shooter).bloom.as_ref().unwrap().input_view, held_bloom);
    scene.nodes = nodes;
    scene.pluses = pluses;
    shooter.shot_again(&scene);
    assert_eq!(target(&shooter).bloom.as_ref().unwrap().input_view, held_bloom);
    scene.nodes.clear();
    scene.pluses.clear();
    // Switching off while empty still retires the allocation, and switching
    // back on waits for a scene to draw before allocating again.
    scene.view.note_bloom = 0.0;
    shooter.shot_again(&scene);
    assert!(target(&shooter).bloom.is_none());
    scene.view.note_bloom = 1.0;
    shooter.shot_again(&scene);
    assert!(target(&shooter).bloom.is_none(), "no allocation before a scene can write it");
}
