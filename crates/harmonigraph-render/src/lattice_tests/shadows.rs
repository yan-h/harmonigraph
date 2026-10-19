//! The shadow a node's rings, a resting cross and a name cast: the Gaussian
//! blur of the item's own ink, multiplied into everything already in the
//! frame under it by the item's own draw (`shadow_through` in lattice.wgsl).

use super::fixtures::*;
use crate::*;

const SIZE: [u32; 2] = [256, 256];

/// The pane fill every reading on bare ground is taken against.
///
/// Nothing in the pass paints a background or carries its colour in uniforms,
/// so a shadow over a region no ink covers
/// composites as `pane * T` and the Shooter's clear is the whole of what it
/// lands on. Bright, so one multiply has the range of a channel to move in.
const GROUND: f64 = 0.8;

fn over_ground() -> wgpu::Color {
    wgpu::Color { r: GROUND, g: GROUND, b: GROUND, a: 1.0 }
}

/// The brightness of one pixel of `shot`, at `(x, y)` of the frame.
fn bright_at(shot: &[u8], x: u32, y: u32) -> i64 {
    let i = ((y * SIZE[0] + x) * 4) as usize;
    brightness(&shot[i..i + 3])
}

/// How many points of the pane one world unit spans, off the projection the
/// scene itself hands out rather than a copy of the camera's arithmetic.
fn points_per_world(scene: &Scene) -> f32 {
    let centre = on_screen(scene, SIZE, glam::Vec3::ZERO);
    on_screen(scene, SIZE, glam::Vec3::X).distance(centre)
}

/// How far a node's outermost ring reaches from its centre, in points: the
/// stack's own outer radius, whose uv 1 is [`Scene::marker_unit`] world units.
fn ink_radius(scene: &Scene) -> f32 {
    scene.rings_outer * scene.marker_unit * points_per_world(scene)
}

/// σ of every caster's shadow in this frame, in points. Every fixture here
/// dials both groups alike, so one number answers for the whole frame.
fn sigma(scene: &Scene) -> f32 {
    crate::shadow::sigma_points(
        scene.view.shadow.lattice_geometry.width,
        scene.node_radius * points_per_world(scene),
    )
}

/// One face-on node on bare ground, with no light in the frame at all: what a
/// shadow lands on is the pane's fill and nothing else, so a reading is the
/// multiply by itself.
fn on_ground(shadow: f32, depth: f32) -> Scene {
    let mut scene = single_marked_node(0, 0);
    scene.camera = harmonigraph_scene::Camera {
        projection: harmonigraph_scene::Projection::Orthographic,
        yaw: 0.0,
        pitch: 0.0,
        ..Default::default()
    };
    // These low-resolution shadow probes need stable, wide wedges; the fresh
    // look is free to change its octave composition independently.
    scene.octave_layout = probe_octave_layout();
    scene.view.glow_reach = 0.0;
    for style in scene.view.shadow.groups_mut() {
        style.width = shadow;
        style.depth = depth;
    }
    // The crosses away: a marker casts a shadow of its own into every frame
    // these fixtures read.
    scene.pluses.clear();
    scene
}

/// A single caster's shadow is a fixed coverage profile scaled by darkness.
/// Probe the real shader with an unchanged atlas and quad, so depth zero is a
/// photometric baseline rather than a different rasterization of the ink.
#[test]
fn node_shadow_darkness_scales_the_entire_profile() {
    let Some(mut shooter) = Shooter::new(SIZE) else { return };
    shooter.clear = over_ground();
    let mut scene = on_ground(1.0, 1.0);
    scene.view.note_bloom = 0.0;
    let mut shot = |depth| {
        shooter.draw_modified(&scene, LatticeLabels::default(), |cb| {
            cb.uniforms.geometry_shadow.depth = depth;
        })
    };
    let (clear, quarter, half, full) = (shot(0.0), shot(0.25), shot(0.5), shot(1.0));
    let (mut body, mut tail) = (0, 0);
    for (((a, b), c), d) in clear
        .chunks_exact(4)
        .zip(quarter.chunks_exact(4))
        .zip(half.chunks_exact(4))
        .zip(full.chunks_exact(4))
    {
        let removed = i32::from(a[0]) - i32::from(d[0]);
        let at_quarter = i32::from(a[0]) - i32::from(b[0]);
        let at_half = i32::from(a[0]) - i32::from(c[0]);
        // Quantization after compositing permits one code per reading.
        assert!((4 * at_quarter - removed).abs() <= 5, "quarter depth changed shape");
        assert!((2 * at_half - removed).abs() <= 3, "half depth changed shape");
        body += usize::from(removed > 80);
        tail += usize::from((6..=30).contains(&removed));
    }
    assert!(body > 100 && tail > 100, "fixture reached only {body} body and {tail} tail pixels");
}

/// The grey a marker's ink is drawn in here — dark against [`GROUND`], so the
/// pixels the cross covers are told from the ones its shadow lands on.
const CROSS_INK: glam::Vec4 = glam::Vec4::new(0.1, 0.1, 0.12, 1.0);

/// [`on_ground`] with its node replaced by `arms` crosses: `(x, strength)`
/// each, standing along the centre row.
fn crosses_on_ground(arms: &[(f32, f32)], arm: f32, shadow: f32, depth: f32) -> Scene {
    let mut scene = on_ground(shadow, depth);
    scene.nodes.clear();
    scene.glow_rows = 0;
    scene.pluses = arms
        .iter()
        .map(|&(x, strength)| {
            standalone_marker(
                &mut scene.nodes,
                glam::Vec3::new(x, 0.0, 0.0),
                arm,
                CROSS_INK,
                strength,
            )
        })
        .collect();
    scene
}

/// The atlas this pane holds after its last shot, if it holds one.
fn target_of(shooter: &Shooter) -> Option<&shadow::ShadowTarget> {
    shooter
        .resources
        .get::<LatticeResources>()
        .and_then(|res| res.panes.get(&shooter.pane))
        .and_then(|pane| pane.offscreen.as_ref())
        .and_then(|o| o.shadow.as_ref())
}

/// Its size, which is all most readings here want of it.
fn atlas_of(shooter: &Shooter) -> Option<[u32; 2]> {
    target_of(shooter).map(|target| target.size)
}

/// A lit node's own light is DARKER just outside its rings than it is one
/// Shadow width further out, and at the Shadow's bottom it is neither.
///
/// The halo held off the ring, re-expressed against the blur that now casts
/// it. The light is composited under everything, so a node's own shadow lands
/// on its own halo; what says the holding-off is LOCAL — a moat rather than a
/// dimmer on the whole light — is that it has all but run out at the width the
/// bar names, σ being half of it (`shadow::sigma_points`) and the blur of a
/// half-plane keeping 2.3% of the light at 2σ.
///
/// Each reading is a SHARE of the light at its own pixel, taken against the
/// same pixel with the Shadow depth at 0, so the falloff's own slope between
/// the two divides out and what is left is the shadow. The fixture asserts
/// only that both points stand on light enough for a share of it to be more
/// than quantization.
#[test]
fn the_light_beside_a_ring_is_held_off_within_one_shadow_width() {
    const SHADOW: f32 = 0.6;
    // HALF the depth bar. At the top of it the multiply under a solid caster
    // is a factor of a thousand, and its 2σ tail is still a third of the
    // light — a reading of the floor rather than of the Gaussian.
    const DEPTH: f32 = 0.5;
    let Some(mut shooter) = Shooter::new(SIZE) else {
        return;
    };
    let scene = lit_node_and_a_name(1.6, SHADOW, DEPTH);
    let flat = shooter.shot(&lit_node_and_a_name(1.6, SHADOW, 0.0));
    let deep = shooter.shot(&scene);

    let centre = on_screen(&scene, SIZE, glam::Vec3::ZERO);
    let row = centre.y.round() as u32;
    // A pixel clear of the rings' own antialiased edge, and one a whole Shadow
    // width — 2σ — further out along the same row.
    let beside = (centre.x + ink_radius(&scene)).round() as u32 + 2;
    let out = beside + (2.0 * sigma(&scene)).round() as u32;
    assert!(out < SIZE[0], "the pair runs off the pane at {out}");
    let (near_flat, far_flat) = (bright_at(&flat, beside, row), bright_at(&flat, out, row));
    // Enough light at BOTH to measure a share of, which is all the pair needs:
    // the two losses below are each taken against this same shot at the same
    // pixel, so the falloff between them is divided out rather than assumed
    // away. The floor is set by the smaller claim — `far < 0.05` is a reading
    // of a Gaussian tail and not of the quantizer only while a twentieth of
    // the far value is still several levels.
    assert!(
        near_flat > 120 && far_flat > 80,
        "the pair must stand on light enough to take a share of: {near_flat} beside the ring \
         against {far_flat} one Shadow width out",
    );

    let loss = |x: u32, flat_at: i64| 1.0 - bright_at(&deep, x, row) as f64 / flat_at as f64;
    let (near, far) = (loss(beside, near_flat), loss(out, far_flat));
    assert!(
        near > 0.25,
        "the ring took {near:.3} of the light standing beside it, so there is no shadow here",
    );
    assert!(
        far < 0.05 && far * 8.0 < near,
        "one Shadow width out the ring still takes {far:.3} of the light, against {near:.3} \
         beside it",
    );
}

/// A node's own rings are not darkened by its own shadow.
///
/// The blend's ink term is not multiplied — `a = 1 - (1 - alpha) * T` leaves
/// `ink + (1 - alpha) * T * dst` — so where a node covers a pixel whole there
/// is nothing of the frame left for its own shadow to take, and the wash it
/// wears is the RAW light, which that shadow has not been through. Read at the
/// top of the depth bar against the bottom of it, on the pixels the node's ink
/// covers in full; the same shot's ground says the shadow was there to be
/// taken.
#[test]
fn a_nodes_own_rings_are_not_darkened_by_its_own_shadow() {
    const SHADOW: f32 = 0.6;
    let Some(mut shooter) = Shooter::new(SIZE) else {
        return;
    };
    // The node's own INK, read with the light OUT of the picture: the pixels
    // it paints the same over black as over the grey, which is ink with
    // nothing showing through it. The light has to go for this reading and not
    // for the claim — a halo is opaque where it is strong, and a halo pixel is
    // exactly what the node's shadow is entitled to darken.
    let mut unlit = lit_node_and_a_name(1.6, SHADOW, 0.0);
    unlit.view.glow_reach = 0.0;
    let over_black = shooter.shot(&unlit);
    shooter.clear = over_ground();
    let over_grey = shooter.shot(&unlit);
    shooter.clear = wgpu::Color::BLACK;
    let opaque: std::collections::BTreeSet<usize> = (0..over_black.len())
        .step_by(4)
        .filter(|&i| over_black[i..i + 4] == over_grey[i..i + 4] && over_black[i + 3] == 255)
        .collect();
    assert!(opaque.len() > 500, "the node covers {} pixels in full", opaque.len());

    let flat = lit_node_and_a_name(1.6, SHADOW, 0.0);
    let flat_shot = shooter.shot(&flat);
    let deep_scene = lit_node_and_a_name(1.6, SHADOW, 1.0);
    let deep = shooter.shot(&deep_scene);
    // Bar a level of rounding: `opaque` is the pixels whose readback ALPHA
    // reaches 255, and a coverage a fraction short of 1 lets that fraction of
    // the light behind it through.
    let worst = opaque
        .iter()
        .map(|&i| (0..4).map(|c| deep[i + c].abs_diff(flat_shot[i + c])).max().unwrap())
        .max()
        .unwrap_or(0);
    let moved = opaque
        .iter()
        .filter(|&&i| (0..4).any(|c| deep[i + c].abs_diff(flat_shot[i + c]) > 1))
        .count();
    assert_eq!(
        moved,
        0,
        "the Shadow depth moved {moved} of the node's own {} covered pixels, by up to {worst}",
        opaque.len(),
    );
    // And the same depth darkens what the node does NOT cover, so there was a
    // shadow in this frame to keep off the ink.
    let dimmed = (0..flat_shot.len())
        .step_by(4)
        .filter(|i| !opaque.contains(i))
        .filter(|&i| brightness(&deep[i..i + 3]) < brightness(&flat_shot[i..i + 3]))
        .count();
    assert!(dimmed > 500, "the node's shadow darkened only {dimmed} pixels beside its rings");
}

/// Two nodes' shadows crossing MULTIPLY: the frame where both reach is the
/// ground times each node's own factor, not the deeper of the two.
///
/// Every caster spends its cell in its own draw, over whatever the frame
/// already holds, so a second shadow lands on a first that is already there —
/// where a `max` into one attachment, or a nearest-distance field, would have
/// the deeper of the two explain the whole of it and the pair be no darker
/// than one.
///
/// Each factor is read off the same frame with the other node left out, which
/// is what makes the product a prediction rather than a fit. Both are asserted
/// to be a measurable bite before the product is: two nodes at a spacing whose
/// blurs never meet pass a product test by both factors being 1.
#[test]
fn two_nodes_crossing_shadows_multiply_rather_than_take_the_deeper() {
    const SHADOW: f32 = 0.6;
    /// How far each node stands from the probe, in world units. Under 3σ past
    /// its own rings, which is what puts the probe inside both blurs.
    const APART: f32 = 2.0;
    // HALF the depth bar: at the top of it the product of two shadows is
    // within a code value of black, and darker than black is not a reading.
    const DEPTH: f32 = 0.5;
    let Some(mut shooter) = Shooter::new(SIZE) else {
        return;
    };
    shooter.clear = over_ground();
    let scene_of = |xs: &[f32]| -> Scene {
        let mut scene = on_ground(SHADOW, DEPTH);
        let node = scene.nodes[0];
        scene.nodes.clear();
        for &x in xs {
            let mut at = node;
            at.world_pos = glam::Vec3::new(x, 0.0, 0.0);
            scene.nodes.push(at);
        }
        rows_per_node(&mut scene);
        scene
    };
    let scene = scene_of(&[-APART, APART]);
    let centre = on_screen(&scene, SIZE, glam::Vec3::ZERO);
    let gap = APART * points_per_world(&scene) - ink_radius(&scene);
    assert!(
        gap > 0.0 && gap < crate::shadow::REACH_SIGMAS * sigma(&scene),
        "the probe stands {gap} px from each node's rings, which is not inside both blurs",
    );

    let (x, y) = (centre.x.round() as u32, centre.y.round() as u32);
    let ground = bright_at(&shooter.shot(&scene_of(&[])), x, y);
    let alone = |shooter: &mut Shooter, at: f32| {
        bright_at(&shooter.shot(&scene_of(&[at])), x, y) as f64 / ground as f64
    };
    let left = alone(&mut shooter, -APART);
    let right = alone(&mut shooter, APART);
    assert!(
        left < 0.95 && right < 0.95,
        "each node alone leaves {left:.4} and {right:.4} of the ground at the probe, so there \
         is no pair of shadows here to cross",
    );
    let both = bright_at(&shooter.shot(&scene), x, y) as f64 / ground as f64;
    let (product, deeper) = (left * right, left.min(right));
    assert!(
        (both - product).abs() < 0.02,
        "the pair leaves {both:.4} of the ground where the product of the two is {product:.4}",
    );
    assert!(
        (both - deeper).abs() > 0.05,
        "the pair leaves {both:.4} and the deeper of the two alone leaves {deeper:.4}: the \
         fixture cannot tell a product from a maximum",
    );
}

/// A NEARER node shadows a farther node's rings and is not shadowed by them,
/// with both on ONE sheet.
///
/// #469's measurement, and the case that is a per-ITEM answer and nothing
/// coarser: two nodes of the same sheet overlapping on screen under an oblique
/// camera, so anything that grouped casters by sheet or by depth layer would
/// have to leave the pair alone. The painter's order is the whole of what
/// decides which way round it goes — the near node draws last, so its blur
/// multiplies the far node's ink and its own ink is already down when the far
/// node's blur is spent.
///
/// The fixture asserts the discs overlap, that the probes stand where the near
/// node paints nothing, and that the far node's shadow WOULD have reached the
/// near node's ink with the near node out of the frame — so the second claim
/// is an occlusion and not a shadow that never got there.
#[test]
fn a_nearer_node_on_one_sheet_shadows_the_farther_and_not_the_reverse() {
    const SHADOW: f32 = 0.6;
    /// How far apart the pair stands along the sheet, in world units: under
    /// the pitch below about a drawn disc on screen, so the far node keeps a
    /// crescent of its own out past the near node's ink.
    const APART: f32 = 4.4;
    let Some(mut shooter) = Shooter::new(SIZE) else {
        return;
    };
    shooter.clear = over_ground();
    let scene_of = |along: &[f32], depth: f32| -> Scene {
        let mut scene = on_ground(SHADOW, depth);
        scene.camera = harmonigraph_scene::Camera {
            projection: harmonigraph_scene::Projection::Perspective,
            pitch: 1.1,
            distance: 9.0,
            target: glam::Vec3::new(0.0, APART / 2.0, 0.0),
            ..Default::default()
        };
        let node = scene.nodes[0];
        scene.nodes.clear();
        for &y in along {
            let mut at = node;
            at.world_pos = glam::Vec3::new(0.0, y, 0.0);
            scene.nodes.push(at);
        }
        rows_per_node(&mut scene);
        scene
    };
    let scene = scene_of(&[0.0, APART], 1.0);

    // Which of the two is nearer, in the terms the pass sorts by (`order` in
    // lib.rs), and how far each disc reaches on the pane.
    let eye = scene.camera.eye();
    let forward = (scene.camera.target - eye).normalize_or_zero();
    let depth_of = |i: usize| (scene.nodes[i].world_pos - eye).dot(forward);
    let (near, far) = if depth_of(0) < depth_of(1) { (0usize, 1usize) } else { (1, 0) };
    let centre = |i: usize| on_screen(&scene, SIZE, scene.nodes[i].world_pos);
    let radius = |i: usize| {
        let edge = scene.nodes[i].world_pos + glam::Vec3::X * scene.marker_unit;
        on_screen(&scene, SIZE, edge).distance(centre(i))
    };
    assert!(
        centre(near).distance(centre(far)) < radius(near) + radius(far),
        "the fixture's nodes must overlap on screen: {} apart at radii {} and {}",
        centre(near).distance(centre(far)),
        radius(near),
        radius(far),
    );

    let solo = |along: f32, depth: f32| scene_of(&[along], depth);
    let (near_at, far_at) = (scene.nodes[near].world_pos.y, scene.nodes[far].world_pos.y);
    // The near node's OPAQUE pixels, and the ones it leaves exactly as it
    // found them — read over two grounds, so what parts them is coverage.
    let near_flat = solo(near_at, 0.0);
    let over_grey = shooter.shot(&near_flat);
    shooter.clear = wgpu::Color::BLACK;
    let over_black = shooter.shot(&near_flat);
    shooter.clear = over_ground();
    let bare = shooter.shot(&scene_of(&[], 0.0));
    let near_opaque: Vec<usize> = (0..over_grey.len())
        .step_by(4)
        .filter(|&i| over_grey[i..i + 4] == over_black[i..i + 4] && over_grey[i + 3] == 255)
        .collect();
    let near_paints_nothing =
        |i: usize| over_grey[i..i + 4] == bare[i..i + 4] && !near_opaque.contains(&i);
    assert!(near_opaque.len() > 500, "the near node covers {} pixels in full", near_opaque.len());

    // The far node's own opaque ink, where the near node paints nothing: the
    // only pixels whose whole story at depth is somebody else's shadow. A far
    // pixel the near node covers in part is a blend of two draws, and one it
    // covers whole holds no far ink at all.
    let far_flat_alone = shooter.shot(&solo(far_at, 0.0));
    let far_deep_alone = shooter.shot(&solo(far_at, 1.0));
    let far_opaque: Vec<usize> = (0..far_flat_alone.len())
        .step_by(4)
        .filter(|&i| far_flat_alone[i + 3] == 255 && far_flat_alone[i..i + 4] != bare[i..i + 4])
        .filter(|&i| near_paints_nothing(i))
        .collect();
    assert!(
        far_opaque.len() > 100,
        "the far node shows {} pixels of its own ink clear of the near node",
        far_opaque.len(),
    );

    let flat = shooter.shot(&scene_of(&[0.0, APART], 0.0));
    let deep = shooter.shot(&scene);
    let onto_far = far_opaque
        .iter()
        .filter(|&&i| brightness(&deep[i..i + 3]) < brightness(&flat[i..i + 3]))
        .count();
    assert!(
        onto_far * 4 > far_opaque.len(),
        "the near node darkened {onto_far} of the far node's {} visible ink pixels",
        far_opaque.len(),
    );

    // The far node leaves the near node's own ink exactly alone, bar a level
    // of rounding: `near_opaque` is the pixels whose alpha reaches 255, and a
    // coverage a thousandth short of 1 lets a thousandth of what is behind
    // through.
    let onto_near = near_opaque
        .iter()
        .filter(|&&i| (0..4).any(|c| deep[i + c].abs_diff(flat[i + c]) > 1))
        .count();
    assert_eq!(onto_near, 0, "the far node's shadow reached {onto_near} pixels of the near node");
    // ...though it does land there with the near node out of the way.
    let would_have = near_opaque
        .iter()
        .filter(|&&i| brightness(&far_deep_alone[i..i + 3]) < brightness(&far_flat_alone[i..i + 3]))
        .count();
    assert!(
        would_have > 100,
        "the far node's shadow reaches only {would_have} of the {} pixels the near node covers, \
         so the fixture is not measuring an occlusion",
        near_opaque.len(),
    );
}

/// Every resting cross casts out of ONE cell at its own share of it — a marker
/// at half the opacity takes half the ground beside it — and the light standing
/// over the position moves that share by nothing.
///
/// The whole of what a marker hands the picture is one number
/// (`PlusInstance::strength`, which `derive_pluses` hands over as the share of a
/// position a name has not yet taken), and it rides the ink and the shadow
/// together. A blur is linear and every cross is the same shape at the same σ,
/// so the field is blurred once and the share is spent where the cell is READ
/// (`plus_paint`) — which is what makes a frame of crosses one cell.
///
/// A share EXACTLY, where `a_crosss_shadow_is_worth_its_ink` reads the same
/// number as an order against a halo: `1 - level * (1 - T)` is a caster of that
/// opacity letting the rest straight through, and the same level inside the
/// exponent would have half a cross cast most of one.
///
/// That the light is not in it is the second half. The Glow release is a clock
/// of its own, seconds long, and a cross whose shadow ran on it would arrive
/// whole with nothing under it — so a node lit over the position has to leave
/// the factor exactly where it is.
///
/// Read as the factor the cross leaves rather than as a brightness, so nothing
/// here depends on what is under it: every draw before this one lands in both
/// shots and cancels, and the composite over a transparent region is `pane * T`.
#[test]
fn a_crosss_shadow_is_its_own_share_of_the_one_cell_the_field_casts_from() {
    const SHADOW: f32 = 0.6;
    const ARM: f32 = 0.5;
    /// Where the reading stands, in points out from the cross's own tip.
    const OUT: u32 = 4;
    let Some(mut shooter) = Shooter::new(SIZE) else {
        return;
    };
    shooter.clear = over_ground();
    let scene = crosses_on_ground(&[(0.0, 1.0)], ARM, SHADOW, 1.0);
    let centre = on_screen(&scene, SIZE, glam::Vec3::ZERO);
    let row = centre.y.round() as u32;
    let at = (centre.x + ARM * points_per_world(&scene)).round() as u32 + OUT;
    let ground = bright_at(&shooter.shot(&crosses_on_ground(&[], ARM, SHADOW, 1.0)), at, row);
    assert!(ground > 500, "the ground beside the cross reads {ground}");
    let taken = |shooter: &mut Shooter, strength: f32| -> f64 {
        let shot = shooter.shot(&crosses_on_ground(&[(0.0, strength)], ARM, SHADOW, 1.0));
        1.0 - bright_at(&shot, at, row) as f64 / ground as f64
    };
    let whole = taken(&mut shooter, 1.0);
    let half = taken(&mut shooter, 0.5);
    assert!(
        whole > 0.2,
        "a whole cross took {whole:.3} of the ground beside it — with none there is nothing for \
         the rest of this to be a share of",
    );
    assert!(
        (half / whole - 0.5).abs() < 0.03,
        "half a cross took {half:.3} of the {whole:.3} a whole one takes",
    );

    // And a field of crosses is one cell however many stand in it: the whole
    // frame's casters, against a frame with no marker at all.
    let cells = |scene: &Scene| {
        LatticeCallback::from_scene(
            scene,
            LatticeLabels::default(),
            egui::vec2(SIZE[0] as f32, SIZE[1] as f32),
            wgpu::TextureFormat::Rgba8Unorm,
            1,
            None,
        )
        .casters
        .len()
    };
    assert_eq!(
        cells(&crosses_on_ground(&[], ARM, SHADOW, 1.0)),
        0,
        "a frame with no cross in it packed a marker cell",
    );
    assert_eq!(
        cells(&crosses_on_ground(&[(-1.5, 1.0), (0.0, 0.6), (1.5, 0.3)], ARM, SHADOW, 1.0)),
        1,
        "three crosses of three opacities must pack ONE cell between them",
    );

    // And the LIGHT over the position leaves that factor alone: one lit node,
    // with a cross standing out in its halo, read at two levels of the light.
    // The node draws after the cross and casts a shadow of its own, so both
    // shots of a pair carry it and the ratio is the cross's multiply alone.
    let lit = |level: f32, cross: bool| -> Scene {
        let mut scene = on_ground(SHADOW, 1.0);
        scene.view.glow_reach = 1.6;
        scene.view.glow_strength = 1.5;
        scene.nodes[0].glow.level = level;
        if cross {
            scene.pluses = vec![standalone_marker(
                &mut scene.nodes,
                glam::Vec3::new(3.2, 0.0, 0.0),
                ARM,
                CROSS_INK,
                1.0,
            )];
        }
        scene
    };
    let out = |scene: &Scene| {
        (on_screen(scene, SIZE, glam::Vec3::new(3.2, 0.0, 0.0)).x + ARM * points_per_world(scene))
            .round() as u32
            + OUT
    };
    let factor = |shooter: &mut Shooter, level: f32| -> f64 {
        let bare = shooter.shot(&lit(level, false));
        let with = shooter.shot(&lit(level, true));
        let at = out(&lit(level, true));
        assert!(
            bright_at(&bare, at, row) > 60,
            "the halo the cross stands in reads {} at light {level}",
            bright_at(&bare, at, row),
        );
        bright_at(&with, at, row) as f64 / bright_at(&bare, at, row) as f64
    };
    let (full, dim) = (factor(&mut shooter, 1.0), factor(&mut shooter, 0.35));
    assert!(full < 0.9, "the cross leaves {full:.3} of the halo it stands in, which is no shadow");
    assert!(
        (full - dim).abs() < 0.03,
        "the light moved the cross's factor from {full:.3} to {dim:.3}",
    );
}

/// A node whose only ink is its audio ring: the ring's coverage rides
/// `audio_ring` (`in.ring`), so one number moves the whole of what it draws.
fn ringing_only(ring: f32, shadow: f32, depth: f32) -> Scene {
    let mut scene = on_ground(shadow, depth);
    let rings = layered_rings();
    let mut paint = harmonigraph_scene::SpectralPaint::silent();
    // The ramp's floor across the whole annulus, dark against the ground, so
    // the ink's own footprint is told from the shadow beside it.
    paint.lut = std::array::from_fn(|_| glam::Vec4::new(0.08, 0.08, 0.1, 1.0));
    (paint.inner, paint.outer) = rings.audio;
    scene.spectral = paint;
    // The octave band off, so the node has nothing else to paint.
    scene.outer_inner = 0.0;
    scene.outer_outer = 0.0;
    scene.rings_outer = rings.audio.1;
    scene.mark_inner = scene.rings_outer + rings.gap;
    scene.view.mark_thickness = rings.mark_thickness;
    scene.octave_gap = PROBE_GAP;
    let node = &mut scene.nodes[0];
    node.octaves = [0.0; harmonigraph_scene::OCTAVE_SLOTS];
    node.activation = 0.0;
    node.melody_level = 0.0;
    node.bass_level = 0.0;
    node.glow.level = 0.0;
    node.audio_ring = ring;
    scene
}

/// A released node's shadow fades with its ink, and ends with it.
///
/// A cell carries LEVEL 1 and the coverage rasterized into it is the ink's own
/// (`fs_node_cell`), envelopes and all — so what fades is what is blurred, and
/// there is no second clock for a shadow to snap off on. At the bottom of the
/// envelope the node paints nothing, ships no instance, and takes its cell
/// with it.
#[test]
fn a_released_nodes_shadow_fades_with_its_ink_and_ends_with_it() {
    const SHADOW: f32 = 0.6;
    let Some(mut shooter) = Shooter::new(SIZE) else {
        return;
    };
    shooter.clear = over_ground();
    let scene = ringing_only(1.0, SHADOW, 1.0);
    let centre = on_screen(&scene, SIZE, glam::Vec3::ZERO);
    let row = centre.y.round() as u32;
    // Clear of the ring's own antialiased edge, and well inside its blur.
    let at = (centre.x + ink_radius(&scene)).round() as u32 + 5;
    let ground = bright_at(&shooter.shot(&ringing_only(1.0, SHADOW, 0.0)), at, row);
    assert!(ground > 500, "the ground the ring's shadow lands on reads {ground}");

    let mut taken = Vec::new();
    for ring in [1.0f32, 0.6, 0.3, 0.0] {
        let scene = ringing_only(ring, SHADOW, 1.0);
        let call = LatticeCallback::from_scene(
            &scene,
            LatticeLabels::default(),
            egui::vec2(SIZE[0] as f32, SIZE[1] as f32),
            wgpu::TextureFormat::Rgba8Unorm,
            1,
            None,
        );
        assert_eq!(
            call.instances.len(),
            usize::from(ring > 0.0),
            "a node at {ring} of its ring shipped {} instances",
            call.instances.len(),
        );
        assert_eq!(
            call.casters.len(),
            usize::from(ring > 0.0),
            "a node at {ring} of its ring packed {} cells",
            call.casters.len(),
        );
        let shot = shooter.shot(&scene);
        taken.push(1.0 - bright_at(&shot, at, row) as f64 / ground as f64);
    }
    // Linear coverage is lighter here than the old exponential transfer;
    // twenty percent still spans forty output codes per channel on this ground.
    assert!(
        taken[0] > 0.2,
        "a whole ring took {:.3} of the ground beside it, which is nothing to release from",
        taken[0],
    );
    assert!(
        taken.windows(2).all(|w| w[0] > w[1] + 0.05),
        "a ring released over 1, 0.6, 0.3 and 0 of itself took {taken:?} off the ground, which \
         is not one order",
    );
    assert_eq!(taken[3], 0.0, "a node with no ink left took {:.3} off the ground", taken[3]);
}

/// A shadow stays outside the caster while its note fades.
///
/// The blur cell already carries the ring's fading alpha. Feeding that value
/// through the Gaussian's calibration gain can still leave a deep shadow under
/// the translucent ring, where its own ink no longer covers it. The caster's
/// footprint masks that self-shadow, without masking the blur just outside it.
#[test]
fn a_fading_node_does_not_show_its_shadow_through_its_own_ring() {
    const SHADOW: f32 = 0.6;
    let Some(mut shooter) = Shooter::new(SIZE) else {
        return;
    };
    for release in [0.3, 0.7, 1.0] {
        shooter.clear = wgpu::Color::BLACK;
        let scene = |ring: f32, depth: f32| ringing_only(ring, SHADOW, depth);
        let empty = shooter.shot(&scene(0.0, 0.0));
        // White exposes fractional edge coverage that the dark probe's
        // quantized color would otherwise round up to a full pixel.
        let mut footprint = scene(1.0, 0.0);
        footprint.spectral.lut.fill(glam::Vec4::ONE);
        let full = shooter.shot(&footprint);
        let body = fully_inked(&empty, &full);
        assert!(body.len() > 30, "only {} pixels reach the ring's full coverage", body.len());

        shooter.clear = over_ground();
        let bare_scene = scene(release, 0.0);
        let bare = shooter.shot(&bare_scene);
        let cast = shooter.shot(&scene(release, 1.0));
        let centre = on_screen(&bare_scene, SIZE, glam::Vec3::ZERO);
        let inside_loss = body
            .iter()
            .map(|&i| brightness(&bare[i..i + 3]) - brightness(&cast[i..i + 3]))
            .max()
            .unwrap_or(0);
        let row = centre.y.round() as u32;
        let outside = (centre.x + ink_radius(&bare_scene)).round() as u32 + 5;
        let outside_loss = bright_at(&bare, outside, row) - bright_at(&cast, outside, row);
        // One final-output code in each colour channel is the dither/rounding
        // boundary; anything past their sum is shadow showing through the ring.
        assert!(
            inside_loss.abs() <= 3,
            "at {release}: self-shadow removed {inside_loss} brightness levels",
        );
        assert!(outside_loss > 20, "at {release}: outside loss only {outside_loss}",);
    }
}

/// A mark too faint for the scene pass cannot cut a bright copy of its shape
/// out of the Gaussian shadow underneath it.
///
/// Mark presence and mark level travel separately. The first nonzero packed
/// level therefore reaches the mark geometry while its ink is still below the
/// shader's floor. Its mask follows that level through the floor, and the
/// final no-ink branch clears the mask with the ink; otherwise an invisible
/// strip removes a whole strip of shadow for one frame at either end of its
/// fade.
#[test]
fn a_subfloor_mark_does_not_mask_a_gaussian_shadow_before_it_is_visible() {
    const SHADOW: f32 = 0.6;
    const SUBFLOOR: f32 = 1.0 / 255.0;
    let ink_floor: f32 = shadow::tests::shader_const(SHADER_SRC, "INK_FLOOR")
        .parse()
        .expect("the shader's ink floor is a number");
    assert!(SUBFLOOR < ink_floor, "the mark must reach the shader's subfloor path");
    let Some(mut shooter) = Shooter::new(SIZE) else {
        return;
    };
    let scene = |level: f32, depth: f32| {
        let mut scene = on_ground(SHADOW, depth);
        // Keep the caster bounds identical: changing the slot bit changes
        // atlas sampling even while its mark is invisible. This test isolates
        // mask leakage from that separate rasterization difference.
        scene.nodes[0].melody_slots = MIDDLE_C;
        scene.nodes[0].melody_level = level;
        scene
    };

    // Find pixels well inside the mark from a full-strength copy over black,
    // rather than assuming its screen angle or accepting a fixture whose mark
    // never reached the frame.
    let absent = shooter.shot(&scene(0.0, 0.0));
    let full = shooter.shot(&scene(1.0, 0.0));
    let body = fully_inked(&absent, &full);
    assert!(body.len() > 20, "only {} pixels reach the mark's full coverage", body.len());

    shooter.clear = over_ground();
    let unmarked = shooter.shot(&scene(0.0, 1.0));
    let subfloor = shooter.shot(&scene(SUBFLOOR, 1.0));
    let lifted = body
        .iter()
        .map(|&i| brightness(&subfloor[i..i + 3]) - brightness(&unmarked[i..i + 3]))
        .max()
        .unwrap_or(0);
    assert!(
        lifted.abs() <= 2,
        "a {SUBFLOOR:.4} mark below the {ink_floor:.2} ink floor lifted its Gaussian shadow by \
         {lifted} brightness levels",
    );
}

/// Neither Shadow bar at its bottom casts anything, allocates anything, or
/// moves a pixel past the final output code — and the two bottoms draw the same
/// frame within that stochastic-rounding boundary.
///
/// The vacuity the whole atlas rests on, asked of a frame carrying one of
/// everything that casts: a node, a cross and a name. A width of 0 packs no
/// cell because σ is 0, and a depth of 0 packs none because the multiply would
/// be 1 everywhere — with the Hide behind bar at 0 too, since it spends the
/// node group's field on its own; the two are different tests in `prepare` and
/// have to reach the same picture. The pair at a width the bar does open is the other half —
/// a frame that allocates and does move pixels, so the equalities above are
/// not two ways of drawing nothing.
#[test]
fn neither_shadow_bar_at_its_bottom_casts_or_allocates() {
    let Some(mut shooter) = Shooter::new(SIZE) else {
        return;
    };
    shooter.clear = over_ground();
    let scene_of = |shadow: f32, depth: f32| -> Scene {
        let mut scene = on_ground(shadow, depth);
        scene.view.hide_behind = 0.0;
        scene.pluses = vec![standalone_marker(
            &mut scene.nodes,
            glam::Vec3::new(1.6, 0.0, 0.0),
            0.3,
            CROSS_INK,
            1.0,
        )];
        scene
    };
    let mut shot = |shadow: f32, depth: f32| -> (Vec<u8>, Option<[u32; 2]>) {
        let scene = scene_of(shadow, depth);
        let named = name_at(&scene, SIZE, glam::Vec3::new(0.0, 1.2, 0.0));
        let frame = shooter.shot_with(&scene, named);
        let atlas = atlas_of(&shooter);
        (frame, atlas)
    };
    let (no_width, no_width_atlas) = shot(0.0, 1.0);
    let (no_depth, no_depth_atlas) = shot(0.4, 0.0);
    let (cast, cast_atlas) = shot(0.4, 1.0);
    assert_eq!(no_width_atlas, None, "a frame at no Shadow width allocated a cell");
    assert_eq!(no_depth_atlas, None, "a frame at no Shadow depth allocated a cell");
    assert!(cast_atlas.is_some(), "a frame of three casters at a Shadow open allocated nothing");
    let bottoms_apart = no_width
        .chunks(4)
        .zip(no_depth.chunks(4))
        .filter(|(a, b)| (0..4).any(|c| a[c].abs_diff(b[c]) > 1))
        .count();
    assert_eq!(
        bottoms_apart, 0,
        "the two ways of shutting the Shadow draw frames apart by more than one output code",
    );
    let moved = differing_pixels(&no_width, &cast);
    assert!(moved > 500, "opening the Shadow moved {moved} pixels, so neither claim above bites");
}

/// At the top of the Shadow bar a caster's quad still holds the whole blur:
/// the profile out from its ink falls all the way to nothing, with no step
/// where the billboard ends.
///
/// The trap the grown quad exists for. A node's billboard and a marker's are
/// each sized to their ink plus `REACH_SIGMAS` σ (`shadow_reach_uv`),
/// and one sized to the ink alone cuts the Gaussian off in a straight line at
/// a value the eye reads as an edge — while every other reading in this suite,
/// each monotone in the Shadow or a superset of a narrower one, goes on
/// passing. Read at the WIDEST the bar goes, where σ is largest against a quad
/// whose other terms do not grow with it.
#[test]
fn the_grown_quad_holds_the_whole_blur_at_the_top_of_the_shadow_bar() {
    const SHADOW: f32 = harmonigraph_scene::GLOW_SHADOW_MAX;
    const ARM: f32 = 0.5;
    // Pull the camera back so every caster's tail lands comfortably inside the
    // pane. A caster's quad and the reach it has to hold are both sized in the
    // node's own uv (`shadow_reach_uv`), so this leaves the ratio under test
    // unchanged while keeping the pane's edge out of the measurement.
    const PULL_BACK: f32 = 2.0;
    let Some(mut shooter) = Shooter::new(SIZE) else {
        return;
    };
    let far = |mut scene: Scene| -> Scene {
        scene.camera.distance *= PULL_BACK;
        scene
    };
    shooter.clear = over_ground();
    // A node's rings and a cross, each with its own ink radius on the pane:
    // the two quads are built by different code and each has to hold its own
    // blur.
    let node = (far(on_ground(SHADOW, 1.0)), far(on_ground(SHADOW, 0.0)));
    let cross = (
        far(crosses_on_ground(&[(0.0, 1.0)], ARM, SHADOW, 1.0)),
        far(crosses_on_ground(&[(0.0, 1.0)], ARM, SHADOW, 0.0)),
    );
    for (what, (deep_scene, flat_scene), edge) in [
        ("a node's rings", node, ink_radius(&far(on_ground(SHADOW, 1.0)))),
        ("a cross", cross, ARM * points_per_world(&far(crosses_on_ground(&[], ARM, SHADOW, 1.0)))),
    ] {
        let flat = shooter.shot(&flat_scene);
        let deep = shooter.shot(&deep_scene);
        let centre = on_screen(&deep_scene, SIZE, glam::Vec3::ZERO);
        let row = centre.y.round() as u32;
        // From two points past the ink, which is clear of its antialiased
        // edge, out to the pane.
        let start = (centre.x + edge).round() as u32 + 2;
        let mut profile = Vec::new();
        for x in start..SIZE[0] {
            let ground = bright_at(&flat, x, row);
            assert!(ground > 500, "{what} leaves {ground} of ground at column {x}");
            profile.push(1.0 - bright_at(&deep, x, row) as f64 / ground as f64);
        }
        let last = profile.iter().rposition(|&v| v > 0.0).expect("a shadow to walk");
        // `profile[0]` starts two points past the ink, so its index alone is
        // two points short of the physical reach being tested.
        let reach = last as f32 + 2.0;
        // A substantial shadow, so that `last` is the end of a descent
        // rather than the one column a faint caster
        // wrote. Not a fixed half of the ground: the two casters here are
        // deliberately different thicknesses against σ, and at the top of the
        // bar a cross an arm wide is thin enough that the gain leaves its
        // shadow at a quarter of the ground where a ring stack's is at three
        // quarters. The threshold must admit both caster thicknesses.
        // Linear notation transmittance also
        // reaches the 8-bit floor sooner than the old power-law tail; the
        // final-step assertion below still rejects a visibly clipped quad.
        assert!(
            profile[0] > 0.1 && reach > 0.65 * crate::shadow::REACH_SIGMAS * sigma(&deep_scene),
            "{what} cast {:.3} at its ink and out to {reach} px, against a σ of {}",
            profile[0],
            sigma(&deep_scene),
        );
        for (i, pair) in profile.windows(2).enumerate() {
            assert!(
                pair[1] <= pair[0] + 1e-9,
                "{what}'s shadow rises from {:.4} to {:.4} at {i} px out from its ink",
                pair[0],
                pair[1],
            );
        }
        // Reach the final output's smallest step before disappearing. The old
        // one-percent cutoff stopped at several codes and the old two-percent
        // tolerance accepted exactly the contour this reading should catch.
        assert!(
            profile[last] <= 1.0 / (GROUND * 255.0) + 1e-9,
            "{what}'s shadow stops at {:.4} of the ground {last} px out: the quad ended inside \
             the blur",
            profile[last],
        );
    }
}

/// A node the camera all but stands on packs a cell the atlas can hold, and
/// every other caster that darkens something still gets one.
///
/// Under perspective a node a fraction of a unit in front of the eye projects
/// to a box thousands of panes wide, and `pack` sizes the WHOLE atlas off the
/// widest box it is handed. Unclipped, that one node takes the atlas to the
/// device's limit, and every cell packed after it falls outside it. A cell that
/// did not fit is ZEROED, which puts it at the atlas ORIGIN — on top of
/// whichever cell is packed there, which is always the markers' one cross. The
/// whole resting field then reads a node's dense ink as its own blur and every
/// marker paints an opaque box the size of its quad.
///
/// The reach is the first assertion: the fixture only touches this at all if
/// some node really does project many panes wide, so the projection is
/// measured rather than assumed.
///
/// Read at two widths: at the fixture's own the kernel's reach is a rounding
/// error on the pane, so only the wider reading says the bound below holds
/// the reach as well as the pane ([`BOXES`]).
#[test]
fn a_node_close_to_the_eye_packs_a_cell_the_atlas_can_hold() {
    /// How many CLIPPED BOXES across the atlas may be — the pane plus the
    /// kernel's reach at either edge, which is what a caster's box is clipped
    /// to. A frame's cells cover a few of those however deep the lattice runs,
    /// so counting them is what says the atlas is sized off the PICTURE and
    /// not off one projection.
    ///
    /// The reach is half the unit and not a detail of it: a bound counted in
    /// bare PANES holds only while the Shadow is narrow enough for the reach
    /// to be a rounding error on the pane, and silently becomes a bound on the
    /// Shadow bar once it is not. Widening the fresh lattice geometry to the
    /// top of the bar was what walked a correctly clipped frame through it.
    const BOXES: f32 = 8.0;
    let shot = super::golden::names_overlapping_on_one_sheet();
    let pane = glam::Vec2::new(SIZE[0] as f32, SIZE[1] as f32);
    let projector = shot.scene.projector(pane);
    // Each node's own radius on the pane, projected rather than scaled off the
    // target plane: under perspective that is the whole point.
    let widest = shot
        .scene
        .nodes
        .iter()
        .filter_map(|n| {
            let at = projector.project(n.world_pos)?;
            let edge = n.world_pos + glam::Vec3::X * shot.scene.node_radius * n.scale;
            Some(projector.project(edge)?.distance(at))
        })
        .fold(0.0f32, f32::max);
    assert!(
        widest > 10.0 * pane.x,
        "the widest node on this fixture draws {widest} points across a {} pane, so nothing here \
         projects far enough to size an atlas off",
        pane.x,
    );

    let cb = LatticeCallback::from_scene(
        &shot.scene,
        shot.labels,
        egui::vec2(pane.x, pane.y),
        wgpu::TextureFormat::Rgba8Unorm,
        1,
        None,
    );
    let sigma = cb.casters.iter().map(|c| c.sigma_points).fold(0.0f32, f32::max);
    assert!(sigma > 0.0, "the fixture's Shadow is shut, so it packs no cell at all");
    // The clip that keeps a near node's box on the pane is per caster and has
    // to hold at either width (#505).
    let wide = sigma * 4.0;
    for (width, sigma) in [("the fixture's Shadow", sigma), ("a wide Shadow", wide)] {
        let casters: Vec<crate::shadow::Caster> = cb
            .casters
            .iter()
            .map(|c| crate::shadow::Caster { sigma_points: sigma, ..*c })
            .collect();
        let packed = crate::shadow::pack(&casters, 1.0, 16384);
        // Read over the casters that darken something: one at level 0 is packed
        // as no cell on purpose (`pack`), and most of this fixture's nodes
        // project clean off the pane.
        let casting: Vec<crate::shadow::ShadowBox> = cb
            .casters
            .iter()
            .enumerate()
            .filter(|(_, c)| c.level > 0.0)
            .map(|(i, _)| packed.boxes[i])
            .collect();
        let unfit = casting.iter().filter(|b| b.cell[2] <= 0.0 || b.cell[3] <= 0.0).count();
        assert_eq!(
            unfit,
            0,
            "at {width}: {unfit} of {} cells that darken something got none, and a \
         zeroed cell is drawn at the atlas origin over the markers' own",
            casting.len(),
        );
        // The widest a box can be after the clip, in points: the pane with
        // the kernel's reach added at both edges.
        let reach = sigma * crate::shadow::REACH_SIGMAS;
        let box_points = pane + 2.0 * reach;
        // The allocator rounds both atlas dimensions to powers of two.
        // Apply the same rounding to this continuous size bound.
        let bound =
            box_points.to_array().map(|v| (BOXES * v).ceil() as u32).map(u32::next_power_of_two);
        assert!(
            packed.size[0] <= bound[0] && packed.size[1] <= bound[1],
            "at {width}: the atlas came out {:?} for a clipped box of {box_points} \
             points: a caster's box is sized off a projection the pane cannot show",
            packed.size,
        );
    }
}

/// A node's shadow at the bottom of the Shadow darkness bar is next to no
/// shadow at all, with the bloom open over its own halo.
///
/// The darkness is the whole of how dark a node's shadow lands: it no longer
/// darkens the bright pass's copy of the picture at full depth whatever the bar
/// says, which left 1% visibly darker than 0 and made 0 a jump rather than an
/// end. At 1% the shadow may take at most a hundredth off the frame under it, so
/// no channel moves more than a few codes from the frame with the group off.
#[test]
fn a_nodes_faintest_shadow_is_next_to_none_over_its_bloom() {
    const SHADOW: f32 = 0.6;
    const FAINTEST: f32 = 0.01;
    let Some(mut shooter) = Shooter::new(SIZE) else {
        return;
    };
    let mut shot = |bloom: f32, depth: f32| -> Vec<u8> {
        let mut scene = lit_node_and_a_name(1.6, SHADOW, depth);
        scene.view.note_bloom = bloom;
        shooter.shot(&scene)
    };
    // The frame has to be carrying a bloom at all, or the reading below is
    // taken on a picture with nothing in it for a shadow to darken.
    let bloomed = differing_pixels(&shot(1.0, FAINTEST), &shot(0.0, FAINTEST));
    assert!(bloomed > 1000, "opening the Bloom moved {bloomed} pixels");
    let (faint, off) = (shot(1.0, FAINTEST), shot(1.0, 0.0));
    let worst = faint.iter().zip(&off).map(|(a, b)| a.abs_diff(*b)).max().unwrap();
    assert!(worst <= 3, "a 1% shadow moved a channel {worst} codes from no shadow at all",);
}

/// A mark whose strip has no width casts no shadow.
///
/// The collapsed radial pair remains the layer's off switch even while its
/// slot and fade inputs would otherwise keep it live.
#[test]
fn a_zero_width_mark_casts_no_shadow() {
    const SHADOW: f32 = 0.5;
    let Some(mut shooter) = Shooter::new(SIZE) else {
        return;
    };
    shooter.clear = over_ground();

    let staged = |slots| {
        let mut scene = on_ground(SHADOW, 1.0);
        scene.view.mark_thickness = 0.0;
        scene.nodes[0].melody_slots = slots;
        scene.nodes[0].melody_level = f32::from(slots != 0);
        scene
    };
    let absent = shooter.shot(&staged(0));
    let collapsed = shooter.shot(&staged(MIDDLE_C));
    let moved = differing_pixels(&collapsed, &absent);
    assert_eq!(moved, 0, "a collapsed mark strip left ink or a shadow in the frame",);
}

/// How much further a MARKED node's shadow reaches than an unmarked one's is
/// the strip's own depth, and the Shadow bar does not move it.
///
/// The strip's depth is a length in the node's uv (`node_ink`), so the cell the
/// atlas rasterizes holds the strip the pane draws, and widening the bar moves
/// the two nodes' reach by one blur alike. A depth in SCREEN widths would not
/// be: a cell is drawn at `min(1, SIGMA_CELL_MAX / σ)` of the target's pixels,
/// where a fragment step is an atlas texel rather than a pane pixel, so such a
/// depth comes out σ wide however thin the strip is dialled, and the gap
/// between the two nodes opens as the bar widens — the marked one casting from
/// ink nothing painted, in the mark's wedge alone.
///
/// A DIFFERENCE of two reaches at each width, and the same difference twice.
/// Each reach is read where the frame stops being darkened enough to see,
/// which sits inside the kernel's own edge by an amount that grows with σ — so
/// it is subtracted off rather than modelled, once between the two nodes and
/// again between the two widths.
#[test]
fn a_marked_nodes_shadow_stands_off_an_unmarked_ones_by_the_strip_alone() {
    // Both widths past `SIGMA_CELL_MAX` (asserted below), so every cell in the
    // four shots is drawn smaller than the pane. Under it a cell is at the
    // pane's own resolution, where a screen width IS the pane's own and there
    // is nothing here to tell apart.
    const NARROW: f32 = 0.4;
    // The top of the bar, so the two ends are as far apart as the picture can
    // put them: what separates the readings is the DIFFERENCE between the σ at
    // each, and the whole shadow still finishes inside the pane here.
    const WIDE: f32 = harmonigraph_scene::GLOW_SHADOW_MAX;
    const DEPTH: f32 = 0.85;
    let Some(mut shooter) = Shooter::new(SIZE) else {
        return;
    };
    shooter.clear = over_ground();

    let staged = |slots: u32, shadow: f32| -> Scene {
        let mut scene = on_ground(shadow, DEPTH);
        scene.nodes[0].melody_slots = slots;
        scene.nodes[0].melody_level = f32::from(slots != 0);
        scene
    };
    // The pane with no node on it at all: what both reaches below are read
    // against, so the two are one kind of measurement and their difference is
    // the reach alone.
    let bare_ground = {
        let mut empty = staged(0, NARROW);
        empty.nodes.clear();
        shooter.shot(&empty)
    };
    let mut reach = |slots: u32, shadow: f32| -> f64 {
        let shot = shooter.shot(&staged(slots, shadow));
        let touched = light_about_center(&light_over(&bare_ground, &shot), SIZE);
        assert!(touched.weight > 0.0, "the node darkened nothing at Shadow {shadow}");
        touched.far
    };
    let at_narrow = reach(MIDDLE_C, NARROW) - reach(0, NARROW);
    let at_wide = reach(MIDDLE_C, WIDE) - reach(0, WIDE);

    let s = sigma(&staged(MIDDLE_C, NARROW));
    assert!(
        s > crate::shadow::SIGMA_CELL_MAX,
        "σ is {s:.2} px at the narrow end, so its cell is drawn at the pane's own resolution \
         and the four shots cannot reach the claim",
    );
    let marked = staged(MIDDLE_C, WIDE);
    let strip = (marked.mark_inner + marked.view.mark_thickness - marked.rings_outer)
        * marked.marker_unit
        * points_per_world(&marked);
    eprintln!(
        "a marked node reaches {at_narrow:.1} px further at Shadow {NARROW} and {at_wide:.1} at \
         {WIDE}; the strip stands {strip:.1} px past the rings",
    );
    // Half the strip's own depth of slack, which is the coarsest the pair can
    // be read at and still say the standoff did not follow the bar: a depth in
    // screen widths puts a whole σ of the WIDE end's blur in here, and σ there
    // is wider than the strip.
    assert!(
        (at_wide - at_narrow).abs() < f64::from(strip) / 2.0,
        "widening the Shadow moved a marked node's standoff from {at_narrow:.1} px to \
         {at_wide:.1}, so the strip the atlas draws is following the bar",
    );
}

/// The TEXT group's width moves a name's shadow and leaves every other
/// caster's exactly as it was, byte for byte.
///
/// The whole point of the groups, and the thing to prove is that σ is per
/// CASTER and not per frame: it is dialled where a caster is built
/// (`from_scene`) and read where its cell is sized, so a term left outside that
/// would take the ring stack and the cross along with the names. A frame with
/// no name in it is the control, and it has to be IDENTICAL rather than close —
/// the group reaches it through no code path at all.
///
/// Read at the geometry group's own width, at the top of the bar and at the
/// bottom. The bottom is the group's OFF switch: a group at no width packs no
/// cell and multiplies by 1, which is the same picture as a group at no depth,
/// and the pair of them agreeing is what says the two switches are one state.
#[test]
fn the_text_groups_width_moves_a_names_shadow_and_no_other_casters() {
    let Some(mut shooter) = Shooter::new(SIZE) else {
        return;
    };
    shooter.clear = over_ground();
    const GEOMETRY: f32 = 0.4;
    let scene_of = |text: f32| -> Scene {
        let mut scene = on_ground(GEOMETRY, 0.85);
        scene.view.shadow.lattice_text.width = text;
        scene
    };
    // The name clear of the node under it, so what the group widens lands on
    // bare ground and is read against the ground alone.
    let named = |scene: &Scene| name_at(scene, SIZE, glam::Vec3::new(0.0, 1.2, 0.0));
    let with_name = |shooter: &mut Shooter, text: f32| {
        let scene = scene_of(text);
        let labels = named(&scene);
        shooter.shot_with(&scene, labels)
    };
    let without_name =
        |shooter: &mut Shooter, text: f32| shooter.shot_with(&scene_of(text), a_name(Vec::new()));

    let one = with_name(&mut shooter, GEOMETRY);
    let wide = with_name(&mut shooter, harmonigraph_scene::GLOW_SHADOW_MAX);
    let off = with_name(&mut shooter, 0.0);
    let moved = differing_pixels(&one, &wide);
    assert!(moved > 200, "the whole text group's width moved {moved} pixels, which is no bar");
    let gone = differing_pixels(&one, &off);
    assert!(gone > 200, "shutting the text group moved {gone} pixels off its own width");

    // The control: the same three widths with the name's glyphs taken out.
    let bare = without_name(&mut shooter, GEOMETRY);
    for text in [0.0, harmonigraph_scene::GLOW_SHADOW_MAX] {
        let other = without_name(&mut shooter, text);
        assert_eq!(
            differing_pixels(&bare, &other),
            0,
            "the text group at {text} moved a frame with no name in it, so σ is still one number \
             for every caster",
        );
    }

    // The group's two off switches are ONE state: a name at no width and a name
    // at no depth are the same picture, with the geometry group untouched in
    // both.
    let no_depth = {
        let mut scene = scene_of(GEOMETRY);
        scene.view.shadow.lattice_text.depth = 0.0;
        let labels = named(&scene);
        shooter.shot_with(&scene, labels)
    };
    assert_eq!(
        differing_pixels(&off, &no_depth),
        0,
        "a name at no width and a name at no depth drew different pictures, so one of the two \
         bars is not the group's off switch",
    );
}

/// A resting marker inherits the WHOLE lattice-text shadow style and none of
/// the node geometry's: width, spread, reach and depth.
///
/// The caster and uniform assertions pin the handoff on either side of the
/// CPU/GPU boundary. The rendered controls then prove the shader spends that
/// handoff: shutting the text depth removes the marker's shadow, while moving
/// every geometry term leaves a markers-only frame byte-identical.
#[test]
fn a_marker_inherits_the_text_groups_whole_shadow() {
    use harmonigraph_scene::ShadowStyle;

    // A spread on the text group alone, so the reach the marker grows its quad
    // by is a third term the two groups disagree on.
    let geometry = ShadowStyle { width: 0.12, spread: 0.0, depth: 0.2 };
    let text = ShadowStyle { width: 0.62, spread: 0.3, depth: 0.85 };
    let scene_of = |geometry: ShadowStyle, text: ShadowStyle| {
        let mut scene = crosses_on_ground(&[(0.0, 1.0)], 0.5, text.width, text.depth);
        scene.view.shadow.lattice_geometry = geometry;
        scene.view.shadow.lattice_text = text;
        scene
    };
    let scene = scene_of(geometry, text);
    let callback = LatticeCallback::from_scene(
        &scene,
        LatticeLabels::default(),
        egui::vec2(SIZE[0] as f32, SIZE[1] as f32),
        wgpu::TextureFormat::Rgba8Unorm,
        1,
        None,
    );
    assert_eq!(callback.casters.len(), 1, "the marker fixture did not make its one caster");
    let want_sigma = shadow::sigma_points(text.width, scene.node_radius * points_per_world(&scene));
    assert!(
        (callback.casters[0].sigma_points - want_sigma).abs() < 1e-4,
        "the marker's σ is {} rather than the text group's {want_sigma}",
        callback.casters[0].sigma_points,
    );
    assert_eq!(
        (callback.casters[0].spread_points, callback.casters[0].stops),
        (0.0, text.text_spread_stops()),
        "the marker does not spend the text group's Spread as stops",
    );
    assert_eq!(
        (
            callback.uniforms.marker_shadow.width,
            callback.uniforms.marker_shadow.reach_sigmas,
            callback.uniforms.marker_shadow.depth,
        ),
        (text.width, crate::shadow::REACH_SIGMAS, text.depth),
        "the marker shader did not inherit the text group's whole style",
    );

    let Some(mut shooter) = Shooter::new(SIZE) else {
        return;
    };
    shooter.clear = over_ground();
    let inherited = shooter.shot(&scene);
    let text_off = shooter.shot(&scene_of(geometry, ShadowStyle { depth: 0.0, ..text }));
    let moved = differing_pixels(&inherited, &text_off);
    assert!(moved > 200, "the text group's depth moved only {moved} marker pixels");

    let other_geometry =
        ShadowStyle { width: harmonigraph_scene::GLOW_SHADOW_MAX, spread: 1.0, depth: 1.0 };
    let geometry_moved = shooter.shot(&scene_of(other_geometry, text));
    assert_eq!(
        differing_pixels(&inherited, &geometry_moved),
        0,
        "changing the node shadow moved a markers-only frame",
    );
}

/// Changing only pane width translates these face-on casters without changing
/// their point-space shadows. Both the pane and the allocated Gaussian atlas
/// are materially rectangular, so swapping either coordinate pair is visible.
/// Glow is off: the marker's world unit still has to reach its shadow quad.
#[test]
fn rectangular_panes_and_atlases_preserve_shadow_coordinates() {
    const NARROW: [u32; 2] = [384, 192];
    const WIDE: [u32; 2] = [576, 192];
    let Some(mut shooter) = Shooter::new(NARROW) else { return };
    shooter.clear = over_ground();
    let mut scene = on_ground(0.7, 0.85);
    scene.nodes[0].world_pos = glam::Vec3::new(-1.8, 0.8, 0.0);
    scene.pluses = vec![standalone_marker(
        &mut scene.nodes,
        glam::Vec3::new(2.0, -0.7, 0.0),
        0.4,
        scene.lattice_ground,
        1.0,
    )];
    let mut shadows = Vec::new();
    for size in [NARROW, WIDE] {
        shooter.size = size;
        let shadowed = shooter.shot(&scene);
        let target = shooter.resources.get::<LatticeResources>().unwrap().panes[&shooter.pane]
            .offscreen
            .as_ref()
            .unwrap();
        let atlas = target.shadow.as_ref().expect("Gaussian cells must be allocated");
        assert_ne!(atlas.size[0], atlas.size[1], "the atlas must really have unequal axes");
        assert!(target.glow.is_none(), "marker unit is tested without a glow transport");
        eprintln!("rectangular shadow pane {size:?}, actual atlas {:?}", atlas.size);
        let styles = scene.view.shadow;
        scene.view.shadow.lattice_geometry.depth = 0.0;
        scene.view.shadow.lattice_text.depth = 0.0;
        let plain = shooter.shot(&scene);
        scene.view.shadow = styles;
        let delta: Vec<i64> = plain
            .chunks_exact(4)
            .zip(shadowed.chunks_exact(4))
            .map(|(a, b)| brightness(&a[..3]) - brightness(&b[..3]))
            .collect();
        for world in [scene.nodes[0].world_pos, scene.pluses[0].pos] {
            let at = on_screen(&scene, size, world);
            let mut mass = 0i64;
            for y in (at.y as u32 - 24)..(at.y as u32 + 24) {
                for x in (at.x as u32 - 24)..(at.x as u32 + 24) {
                    mass += delta[(y * size[0] + x) as usize].max(0);
                }
            }
            assert!(mass > 500, "caster at {at:?} must cast a visible shadow: {mass}");
        }
        shadows.push(delta);
    }
    let mut error = 0i64;
    let mut mass = 0i64;
    for y in 0..NARROW[1] {
        for x in 0..NARROW[0] {
            let a = shadows[0][(y * NARROW[0] + x) as usize];
            let b = shadows[1][(y * WIDE[0] + x + (WIDE[0] - NARROW[0]) / 2) as usize];
            error += (a - b).abs();
            mass += a.max(0);
        }
    }
    assert!(error < mass / 40, "translated shadow changed shape/width: error {error}, mass {mass}");
}

/// A disappearing octave slice must spend its own opacity after the blur's
/// spatial profile, including when Spread makes that profile fully dark.
#[test]
fn a_disappearing_slice_fades_its_shadow_in_proportion_to_its_ink() {
    let Some(mut shooter) = Shooter::new(SIZE) else { return };
    shooter.clear = over_ground();
    for spread in [0.0, 0.8] {
        let mut scene = on_ground(0.6, 1.0);
        scene.view.shadow.lattice_geometry.spread = spread;
        scene.spectral.outer = 0.0;
        scene.nodes[0].audio_ring = 0.0;
        scene.nodes[0].melody_level = 0.0;
        scene.nodes[0].bass_level = 0.0;
        scene.nodes[0].octaves.fill(0.0);
        scene.nodes[0].octaves[harmonigraph_scene::MIDDLE_C_SLOT] = 1.0;
        scene.nodes[0].activation = 1.0;
        let full = shooter.shot(&scene);
        scene.view.shadow.lattice_geometry.depth = 0.0;
        let bare = shooter.shot(&scene);
        let exterior: Vec<_> = bare
            .chunks_exact(4)
            .zip(full.chunks_exact(4))
            .enumerate()
            .filter_map(|(i, (a, b))| {
                (a[0] == 204 && a[1] == 204 && a[2] == 204 && a[0].saturating_sub(b[0]) > 30)
                    .then_some(i * 4)
            })
            .collect();
        assert!(exterior.len() > 40, "spread={spread}: only {} shadow pixels", exterior.len());
        scene.view.shadow.lattice_geometry.depth = 1.0;
        for level in [0.75, 0.5, 0.25, 0.05, 0.0] {
            scene.nodes[0].octaves[harmonigraph_scene::MIDDLE_C_SLOT] = level;
            scene.nodes[0].activation = level;
            let faded = shooter.shot(&scene);
            let worst = exterior
                .iter()
                .map(|&i| {
                    let full_loss = 204.0 - full[i] as f32;
                    let faded_loss = 204.0 - faded[i] as f32;
                    (faded_loss - full_loss * level).abs()
                })
                .fold(0.0, f32::max);
            assert!(worst <= 3.0, "spread={spread} level={level}: fade error {worst}");
        }
    }
}

/// A held slice must not raise a fading neighbor's shadow ceiling where its
/// own blur contributes almost nothing. This reaches staggered slice motion,
/// rather than fading every layer of a node together.
#[test]
fn a_held_slice_does_not_pin_a_neighbors_shadow_outside_its_own_blur() {
    let Some(mut shooter) = Shooter::new(SIZE) else { return };
    shooter.clear = over_ground();
    for level in [0.75, 0.25, 0.05] {
        let mut scene = on_ground(0.6, 1.0);
        scene.view.shadow.lattice_geometry.spread = 0.8;
        scene.view.note_animation.radial_start = 0.0;
        scene.spectral.outer = 0.0;
        scene.nodes[0].audio_ring = 0.0;
        scene.nodes[0].melody_level = 0.0;
        scene.nodes[0].bass_level = 0.0;
        scene.nodes[0].slice_progress.fill(0.0);
        scene.nodes[0].slice_progress[0] = level;
        let fading = shooter.shot(&scene);
        scene.nodes[0].slice_progress[0] = 0.0;
        scene.nodes[0].slice_progress[2] = 1.0;
        let held = shooter.shot(&scene);
        scene.nodes[0].slice_progress[0] = level;
        let combined = shooter.shot(&scene);
        scene.view.shadow.lattice_geometry.depth = 0.0;
        let bare = shooter.shot(&scene);
        let mut reached = 0;
        let mut excess = 0;
        for (i, pixel) in bare.chunks_exact(4).enumerate() {
            if pixel[..3] != [204, 204, 204] || held[i * 4] < 203 {
                continue;
            }
            let i = i * 4;
            reached += usize::from(fading[i] < 200);
            // Where the held slice casts at most one code of shadow,
            // it cannot amplify the fading slice beyond RGBA8 rounding.
            // Visible overlapping shadows may combine nonlinearly.
            excess = excess
                .max(i32::from(fading[i]) + i32::from(held[i]) - i32::from(combined[i]) - 204);
        }
        assert!(reached > 40, "level={level}: only {reached} fading shadow pixels");
        assert!(excess <= 4, "level={level}: neighboring ink amplified the shadow by {excess}");
    }
}
