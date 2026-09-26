//! Bloom, and the order the sheets reach the frame in.

use super::fixtures::*;
use crate::*;

/// Fractional strength must change the halo independently of AA/render scale.
/// Read unsaturated pixels outside the original ink so clipping cannot hide a
/// swapped control, and compare both scales at the same screen resolution.
#[test]
fn fractional_bloom_strength_is_independent_of_render_scale() {
    let Some(mut shooter) = Shooter::new([256, 256]) else { return };
    let mut scene = single_marked_node(0, 0);
    scene.glow_reach = 0.0;
    scene.pluses.clear();
    scene.nodes[0].octaves.fill(1.0);
    scene.pitch_lut.fill(glam::Vec4::new(0.7, 0.5, 0.3, 1.0));
    let mut scale_totals = Vec::new();
    for scale in [1.0, 2.0] {
        scene.render_scale = scale;
        scene.bloom_strength = 0.0;
        let plain = shooter.shot(&scene);
        let mut halos = Vec::new();
        for strength in [0.25, 0.75] {
            scene.bloom_strength = strength;
            let frame = shooter.shot(&scene);
            let halo: u64 = plain
                .chunks_exact(4)
                .zip(frame.chunks_exact(4))
                .filter(|(a, _)| a[..3] == [0, 0, 0])
                .map(|(_, b)| b[..3].iter().map(|v| u64::from(*v)).sum::<u64>())
                .sum();
            halos.push(halo);
        }
        assert!(halos[0] > 500, "fixture needs a measurable halo: {halos:?}");
        let ratio = halos[1] as f64 / halos[0] as f64;
        assert!((2.8..3.2).contains(&ratio), "scale {scale}: halo ratio {ratio}, {halos:?}");
        eprintln!("fractional bloom scale {scale}: halo sums {halos:?}");
        scale_totals.push(halos[1]);
    }
    let ratio = scale_totals[1] as f64 / scale_totals[0] as f64;
    assert!((0.85..1.15).contains(&ratio), "render scale changes halo strength: {scale_totals:?}");
}

/// A node's bloom share (the Glow display) scales its halo and leaves its ink:
/// less share is less halo, none is no halo, and the full share draws exactly
/// what a node that never had one drew.
///
/// Not in proportion: the share dims the bright pass's copy of the ink, and
/// the threshold's knee then takes more than that away (half the share is
/// about an eighth of the halo here), as it does for a faded note.
#[test]
fn a_nodes_bloom_share_scales_its_halo_and_not_its_ink() {
    let Some(mut shooter) = Shooter::new([256, 256]) else { return };
    let mut scene = single_marked_node(0, 0);
    scene.glow_reach = 0.0;
    scene.pluses.clear();
    scene.nodes[0].octaves.fill(1.0);
    scene.pitch_lut.fill(glam::Vec4::new(0.7, 0.5, 0.3, 1.0));
    scene.bloom_strength = 0.0;
    let plain = shooter.shot(&scene);
    scene.bloom_strength = 0.75;
    let full = shooter.shot(&scene);
    let mut at = |share: f32| {
        scene.nodes[0].bloom = share;
        shooter.shot(&scene)
    };
    // Light outside the ink, where the plain frame is black.
    let halo = |frame: &[u8]| -> u64 {
        plain
            .chunks_exact(4)
            .zip(frame.chunks_exact(4))
            .filter(|(a, _)| a[..3] == [0, 0, 0])
            .map(|(_, b)| b[..3].iter().map(|v| u64::from(*v)).sum::<u64>())
            .sum()
    };
    assert!(at(1.0) == full, "the full share drew something other than the default");
    let shares = [0.0, 0.5, 0.75, 1.0].map(|share| halo(&at(share)));
    assert_eq!(shares[3], halo(&full));
    assert!(shares[3] > 500, "fixture needs a measurable halo: {shares:?}");
    assert!(shares.windows(2).all(|w| w[0] < w[1]), "less share, not less halo: {shares:?}");
    assert!(shares[0] * 20 < shares[3], "no share still gave a halo: {shares:?}");
}

/// A sheet in FRONT of the home sheet is drawn over it; a sheet BEHIND it
/// is drawn under. Both directions matter, and only one of them is obvious:
/// forcing the home sheet to the bottom (so an off-sheet note could never be
/// hidden by it) inverts the far half of the axis, and since a node is drawn
/// over — and casts over — whatever came before it, the sheet behind then
/// stands on top of the home sheet in front of it.
#[test]
fn sheets_draw_back_to_front_along_the_sevens_axis() {
    use harmonigraph_scene::{Camera, FrameParams, Projection, ViewConfig};

    let view = ViewConfig {
        extent_threes: 1,
        extent_fives: 1,
        min_sevens: -2,
        max_sevens: 2,
        ..ViewConfig::default()
    };
    for projection in [Projection::Cabinet, Projection::Perspective, Projection::Orthographic] {
        let mut scene = animated_snapshot(
            &harmonigraph_core::NoteTracker::new(),
            &harmonigraph_core::Tuning::default(),
            &view,
            &view.reach(),
            &FrameParams::default(),
            // Orbited, deliberately: this is the case a plain depth sort
            // gets wrong, because two nodes on one sheet then sit at
            // different depths and the sheets interleave.
            Camera { projection, ..Camera::default() },
            None,
            0.0,
        );
        // Every position SOUNDING. Set here rather than played in, because
        // what this test needs is one node per position on every sheet, and
        // which nodes a tracker lights is a question about tuning.
        //
        // Sounding is the only way to get one: an idle node paints nothing at
        // all now, so the cull drops it, and a scene of idle nodes would leave
        // the order below comparing an empty list with itself.
        for node in &mut scene.nodes {
            node.activation = 1.0;
        }
        let call = LatticeCallback::from_scene(
            &scene,
            LatticeLabels::default(),
            egui::vec2(800.0, 600.0),
            wgpu::TextureFormat::Bgra8Unorm,
            0,
            // No stats slot: this is about draw ORDER, not about timing.
            None,
        );
        // World z IS the sevens axis (see lattice_to_world), so the draw
        // order must run from the most negative sheet to the most positive
        // — and it has to hold under EVERY projection, not only the face-on
        // one. When it doesn't, the sheets interleave and every shadow in the
        // frame is cast in the wrong order, an item multiplying only what the
        // walk happened to put under it.
        let depths: Vec<f32> = call.instances.iter().map(|i| i.world_pos[2]).collect();
        // Several SHEETS, not several nodes. A node count passes on one
        // sheet's worth of identical depths, where every pair below holds
        // whatever the sort did — which is what culling the off-sheet nodes
        // reduced this to, silently, while it went on reading as coverage.
        let (lo, hi) =
            depths.iter().fold((f32::MAX, f32::MIN), |(lo, hi), &d| (lo.min(d), hi.max(d)));
        assert!(
            hi - lo > 1e-6,
            "{projection:?}: every node drawn is at one depth ({lo}), so the order \
             below compares a sheet with itself: {depths:?}",
        );
        for pair in depths.windows(2) {
            assert!(
                pair[1] >= pair[0] - 1e-6,
                "{projection:?}: a sheet behind is drawn after one in front: {pair:?}"
            );
        }
    }
}
