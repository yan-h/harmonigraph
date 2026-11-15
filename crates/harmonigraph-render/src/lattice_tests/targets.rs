//! A pane's targets are keyed on what decides their size, and nothing else.

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

/// A drag of the bloom's strength keeps the bloom chain it drew on, and a
/// resize or a render-scale change keeps the ink strip: neither decides those
/// textures' sizes, so neither may reallocate them every frame of a drag.
#[test]
fn a_bloom_drag_and_a_resize_keep_the_targets_they_do_not_size() {
    let Some(mut shooter) = Shooter::new([256, 256]) else { return };
    let mut scene = single_marked_node(MIDDLE_C, 0);
    scene.view.glow_reach = 0.8;
    scene.view.glow_strength = 1.5;
    scene.view.note_bloom = 1.0;
    shooter.shot(&scene);
    let bloom = target(&shooter).bloom.as_ref().unwrap().input_view.clone();
    let raw = strip(&shooter).raw_view.clone();

    scene.view.note_bloom = 0.5;
    shooter.shot_again(&scene);
    assert_eq!(target(&shooter).bloom.as_ref().unwrap().input_view, bloom, "a drag reallocated");

    for (size, scale) in [([257, 261], 1.0), ([257, 261], 1.5)] {
        shooter.size = size;
        scene.view.render_scale = scale;
        shooter.shot_again(&scene);
        assert_eq!(strip(&shooter).raw_view, raw, "{size:?} at {scale} replaced the strip");
    }
}
