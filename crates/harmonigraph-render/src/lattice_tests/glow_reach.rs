//! How far a node's light reaches, and how it falls off across that reach.

use super::fixtures::*;
use crate::*;

/// The node glow's two claims about geometry: it puts light OUTSIDE the node it
/// comes from, and the Reach bar is what says how far out.
///
/// Measured as the farthest pixel the glow changes, rather than as an amount at
/// a chosen radius, because the reach is where the light STOPS and not how
/// bright it is on the way: the same number is the falloff's domain and the
/// point its window shuts (`glow_layer`), so what moves when the bar moves is
/// the edge.
///
/// One centered node on a cleared marker field — [`single_marked_node`]'s fixture,
/// which is the one scene here with a node whose surroundings are empty enough
/// for "outside the node" to mean anything.
#[test]
fn the_glow_reach_says_how_far_a_node_lights_past_its_own_edge() {
    const SIZE: [u32; 2] = [256, 256];
    let Some(mut shooter) = Shooter::new(SIZE) else {
        return;
    };
    let at = |reach: f32| -> Scene {
        let mut scene = single_marked_node(0, 0);
        scene.glow_reach = reach;
        scene.glow_strength = 1.5;
        scene
    };
    let off = shooter.shot(&at(0.0));
    let near = shooter.shot(&at(0.2));
    let far = shooter.shot(&at(0.8));

    // How far from the node's center — the frame's center, the fixture's one
    // node sitting at the world origin — the picture changed at all.
    let farthest = |a: &[u8], b: &[u8]| -> f32 {
        let row = SIZE[0] as usize;
        let center = (SIZE[0] as f32 / 2.0, SIZE[1] as f32 / 2.0);
        a.chunks(4)
            .zip(b.chunks(4))
            .enumerate()
            .filter(|(_, (x, y))| x != y)
            .map(|(i, _)| {
                let (px, py) = ((i % row) as f32, (i / row) as f32);
                ((px - center.0).powi(2) + (py - center.1).powi(2)).sqrt()
            })
            .fold(0.0f32, f32::max)
    };
    let (near_edge, far_edge) = (farthest(&near, &off), farthest(&far, &off));

    // Non-vacuous first: there has to BE light to measure. Against the reach
    // beside it rather than against the glow OFF, because the glow replaces the
    // core's own skirt rather than joining it — a narrow reach spreads that
    // same light thinner and can leave the frame dimmer overall, where one
    // reach against another is the monotone claim: the lit area grows with the
    // square of the span.
    assert!(
        total_light(&far) > total_light(&near),
        "a wider reach must spread more light: {} against {}",
        total_light(&far),
        total_light(&near),
    );
    assert!(near_edge > 0.0, "a glow at reach 0.2 changed no pixel at all");
    assert!(
        far_edge > near_edge + 4.0,
        "reach 0.8 must light further out than reach 0.2, and it reached {far_edge:.1}px \
         against {near_edge:.1}px",
    );
}

/// A slower shape leaves the peak alone and carries visible light into the
/// outer reach. That is the picture the curve exists to make: a quiet tail
/// rather than a second strength control.
#[test]
fn the_glow_curve_can_hold_a_long_tail_without_moving_the_peak_or_edge() {
    const SIZE: [u32; 2] = [256, 256];
    let Some(mut shooter) = Shooter::new(SIZE) else {
        return;
    };
    let at = |curve: harmonigraph_scene::GlowCurve| -> Scene {
        let mut scene = single_marked_node(0, 0);
        scene.glow_reach = 0.8;
        scene.glow_strength = 1.0;
        scene.glow_curve = curve;
        // Aligned readback rows require an even width, whose geometric centre
        // lies between fragments. Pan the content onto the centre of one pixel
        // so the fixed full endpoint is the thing compared below.
        let pixel_center = glam::Vec2::new(SIZE[0] as f32 * 0.5 + 0.5, SIZE[1] as f32 * 0.5 + 0.5);
        for _ in 0..12 {
            let error = pixel_center - on_screen(&scene, SIZE, glam::Vec3::ZERO);
            scene.camera.pan(error);
        }
        let projected = on_screen(&scene, SIZE, glam::Vec3::ZERO);
        assert!(
            projected.distance(pixel_center) < 0.001,
            "the curve's full endpoint landed at {projected:?} rather than {pixel_center:?}",
        );
        scene
    };
    let compact_curve = harmonigraph_scene::GlowCurve { shape: 2.75 };
    let long_curve = harmonigraph_scene::GlowCurve { shape: 0.75 };
    let compact = shooter.shot(&at(compact_curve));
    let long = shooter.shot(&at(long_curve));
    let mut unlit = at(compact_curve);
    unlit.glow_reach = 0.0;
    let off = shooter.shot(&unlit);

    // The fixture's node is at the frame centre. Curve endpoint 0 is fixed at
    // full, so the pixel there is exactly the same when the shape moves — not
    // merely close after a retuned Strength.
    let centre = ((SIZE[1] / 2) * SIZE[0] + SIZE[0] / 2) as usize;
    assert_eq!(
        &long[centre * 4..centre * 4 + 4],
        &compact[centre * 4..centre * 4 + 4],
        "lifting the tail changed the glow at its peak",
    );

    let row = SIZE[0] as usize;
    let middle = (SIZE[0] as f32 / 2.0, SIZE[1] as f32 / 2.0);
    let edge = long
        .chunks(4)
        .zip(off.chunks(4))
        .enumerate()
        .filter(|(_, (a, b))| a != b)
        .map(|(i, _)| {
            let (x, y) = ((i % row) as f32, (i / row) as f32);
            ((x - middle.0).powi(2) + (y - middle.1).powi(2)).sqrt()
        })
        .fold(0.0f32, f32::max);
    assert!(edge > 20.0, "the fixture's glow reached only {edge:.1}px");

    let outer = |shot: &[u8]| {
        let mut sum = 0i64;
        let mut count = 0i64;
        for (i, (pixel, dark)) in shot.chunks(4).zip(off.chunks(4)).enumerate() {
            let (x, y) = ((i % row) as f32, (i / row) as f32);
            let radius = ((x - middle.0).powi(2) + (y - middle.1).powi(2)).sqrt();
            if radius >= edge * 0.68 && radius <= edge * 0.82 {
                sum += (brightness(pixel) - brightness(dark)).max(0);
                count += 1;
            }
        }
        assert!(count > 100, "the outer-reach annulus held only {count} pixels");
        sum as f64 / count as f64
    };
    let (ordinary, tailed) = (outer(&compact), outer(&long));
    assert!(ordinary > 0.0, "the compact curve left no outer halo to compare");
    assert!(
        tailed > ordinary * 1.5,
        "the slower shape left the outer reach at {tailed:.2} against \
         the compact curve's {ordinary:.2}",
    );
}

/// The widest the colours round one node's halo get from one another: the
/// annulus `inner..outer` about the frame's centre cut into wedges, each
/// wedge's mean taken, and the largest distance between any two of them.
///
/// A CHROMATICITY — every pixel divided by its own total — because the light
/// falls off across the annulus and a plain mean would read that falloff as a
/// colour difference. The question here is whether two directions are lit in
/// different COLOURS, not in different amounts.
fn halo_hue_spread(px: &[u8], size: [u32; 2], inner: f32, outer: f32) -> f64 {
    const BINS: usize = 16;
    let centre = (size[0] as f32 / 2.0, size[1] as f32 / 2.0);
    let mut sums = [[0.0f64; 3]; BINS];
    let mut counts = [0.0f64; BINS];
    for (i, p) in px.chunks(4).enumerate() {
        let (x, y) = ((i % size[0] as usize) as f32, (i / size[0] as usize) as f32);
        let (dx, dy) = (x - centre.0, y - centre.1);
        let r = (dx * dx + dy * dy).sqrt();
        if r < inner || r > outer {
            continue;
        }
        let total = f64::from(p[0]) + f64::from(p[1]) + f64::from(p[2]);
        // Too dark to have a colour at all: the chromaticity of a near-black
        // pixel is quantization noise, and out at the light's own edge there
        // are enough of those to drown the reading.
        if total < 24.0 {
            continue;
        }
        let turn = (dy.atan2(dx).rem_euclid(std::f32::consts::TAU)) / std::f32::consts::TAU;
        let bin = ((turn * BINS as f32) as usize).min(BINS - 1);
        for c in 0..3 {
            sums[bin][c] += f64::from(p[c]) / total;
        }
        counts[bin] += 1.0;
    }
    let means: Vec<[f64; 3]> = (0..BINS)
        .filter(|&b| counts[b] > 0.0)
        .map(|b| [sums[b][0] / counts[b], sums[b][1] / counts[b], sums[b][2] / counts[b]])
        .collect();
    let mut worst = 0.0f64;
    for (i, a) in means.iter().enumerate() {
        for b in &means[i + 1..] {
            let d = (a[0] - b[0]).powi(2) + (a[1] - b[1]).powi(2) + (a[2] - b[2]).powi(2);
            worst = worst.max(d.sqrt());
        }
    }
    worst
}

/// The Color blend bar's whole claim: a node lighting two directions in two colours
/// keeps them apart at the bottom of the bar and averages them into one tint at
/// the top.
///
/// Measured out in the HALO — an annulus past everything the node draws — and
/// not over the node, because the ink is drawn over the light there and what
/// would be read is the node rather than its glow. The two ends of the fixture's
/// mark are the two colours: gold one way, cyan another, on octave slices that
/// do not touch.
///
/// This is the reading the bar had no test for while it did nearly nothing. The
/// colour eased toward the strip's flat mean over the light's whole SPAN, and
/// the skirt is an exponential over that same length, so with any real Reach
/// dialled in the halo was that mean nearly everywhere — and the mean is the one
/// average the Color blend bar cannot move, being taken at no concentration at all by
/// definition. Against the node's own rim instead, the bottom of the bar reads
/// 0.11 here where the span ramp read 0.065, and the bar's travel roughly
/// doubles.
#[test]
fn the_glow_blend_says_how_separate_a_node_keeps_its_colours() {
    const SIZE: [u32; 2] = [256, 256];
    let Some(mut shooter) = Shooter::new(SIZE) else {
        return;
    };
    let beside = slot_beside_middle_c();
    let at = |blend: f32| -> Scene {
        let mut scene = single_marked_node(MIDDLE_C, beside);
        scene.glow_reach = 0.8;
        scene.glow_strength = 1.5;
        scene.glow_blend = blend;
        scene
    };
    let tight = shooter.shot(&at(0.0));
    let middle = shooter.shot(&at(0.5));
    let broad = shooter.shot(&at(1.0));

    // The annulus, sized off the light itself rather than guessed: the farthest
    // pixel the glow moves is where its window shuts, and the outer half of
    // that is halo and nothing else.
    let mut edge = 0.0f32;
    let mut unlit = at(0.0);
    unlit.glow_reach = 0.0;
    let dark = shooter.shot(&unlit);
    let row = SIZE[0] as usize;
    for (i, (a, b)) in tight.chunks(4).zip(dark.chunks(4)).enumerate() {
        if a != b {
            let (px, py) = ((i % row) as f32, (i / row) as f32);
            let (dx, dy) = (px - SIZE[0] as f32 / 2.0, py - SIZE[1] as f32 / 2.0);
            edge = edge.max((dx * dx + dy * dy).sqrt());
        }
    }
    assert!(
        edge > 16.0,
        "the fixture's light reached only {edge:.1}px and there is nothing to read",
    );
    let (inner, outer) = (edge * 0.6, edge * 0.9);

    let spreads = [
        halo_hue_spread(&tight, SIZE, inner, outer),
        halo_hue_spread(&middle, SIZE, inner, outer),
        halo_hue_spread(&broad, SIZE, inner, outer),
    ];
    eprintln!(
        "halo hue spread over {inner:.0}..{outer:.0}px — tight {:.4}, middle {:.4}, broad {:.4}",
        spreads[0], spreads[1], spreads[2],
    );
    // Non-vacuous first: the bottom of the bar has to draw two colours at all.
    // The fixture's two marks are 0.36 apart in this reading laid down pure, so
    // a tenth of that is a halo carrying both of them and not one tint.
    assert!(
        spreads[0] > 0.035,
        "at the bottom of the Color blend bar a node lighting two colours drew one: {:.4}",
        spreads[0],
    );
    // And monotone: every step up the bar averages further.
    assert!(
        spreads[0] > spreads[1] && spreads[1] > spreads[2],
        "the Color blend bar must average further at every step: {:.4} / {:.4} / {:.4}",
        spreads[0],
        spreads[1],
        spreads[2],
    );
    // The top of it is the mean, which has no direction left in it — read as a
    // RATIO against the bottom rather than as an absolute, because the annulus
    // is sized off the light's own edge and the node's outer ink reaches a
    // little way into it. That ink has a direction whatever the bar says, so
    // the top of the bar has a floor it cannot go under and only the two ends
    // compared say what the LIGHT did.
    assert!(
        spreads[0] > spreads[2] * 3.0,
        "the top of the bar must be one tint beside the bottom, and it kept \
         {:.4} against {:.4}",
        spreads[2],
        spreads[0],
    );
}

/// What the glow is a layer of is a node, and a lattice drawing none has none of
/// it: the glow's target is cleared, nothing writes to it, and the composite
/// lays exactly nothing over the picture.
///
/// The guard is against the light ever becoming a POST-PROCESS over the finished
/// picture. One of those would find the markers' ink and bloom it here whatever
/// any bar said; a draw off an instance buffer cannot, and the fixture's buffer
/// is full of markers while the Reach is dialled right up.
///
/// Byte-identical rather than nearly so, which is what a cleared target and a
/// draw discarding every fragment are worth together.
#[test]
fn a_lattice_with_no_node_grows_no_glow() {
    const SIZE: [u32; 2] = [256, 256];
    let Some(mut shooter) = Shooter::new(SIZE) else {
        return;
    };
    let at = |reach: f32| -> Scene {
        let mut scene = parity_scene();
        scene.nodes.clear();
        scene.glow_reach = reach;
        scene.glow_strength = 1.5;
        scene
    };
    let off = shooter.shot(&at(0.0));
    let on = shooter.shot(&at(0.8));
    // The markers have to be drawing something, or this passes on a blank frame.
    assert!(total_light(&off) > 0, "the fixture must draw its markers");
    assert_eq!(differing_pixels(&on, &off), 0, "the glow lit something no node drew",);
}

/// The MIDDLE of a node glows, and a SHEET behind it makes it glow more.
///
/// Two halves, and the second is #435. The first: inside the innermost ring
/// there is nothing painted at all — [`parity_scene`]'s octave band is an
/// annulus and the audio ring is off — so what that pixel carries is the light
/// and only the light, which is what makes the glow the note's own light
/// rather than a rim around it. Read against the glow OFF rather than against
/// a neighbouring pixel, because the thing that must not happen is the middle
/// going DARK: nothing else is drawn there to take the light's place.
///
/// The second: a node on a sheet BEHIND adds its halo to the field, and the
/// field is composited under every node (`fs_glow_over`), so the near node's
/// middle comes out brighter than it is with nothing behind it. A nearer node's
/// body taking the light of the sheets behind off itself is what inverts this —
/// its middle would then hold its own light alone while the ground a few pixels
/// away held everyone's, and the node would read as a hole rather than as a
/// lamp.
#[test]
fn the_middle_of_a_node_is_where_its_light_is_fullest() {
    const SIZE: [u32; 2] = [256, 256];
    let Some(mut shooter) = Shooter::new(SIZE) else {
        return;
    };
    let at = |reach: f32| -> Scene {
        let mut scene = single_marked_node(0, 0);
        scene.camera = harmonigraph_scene::Camera {
            projection: harmonigraph_scene::Projection::Orthographic,
            yaw: 0.0,
            pitch: 0.0,
            ..Default::default()
        };
        scene.glow_reach = reach;
        scene.glow_strength = 1.5;
        for style in scene.shadow.groups_mut() {
            style.width = 0.16;
        }
        scene
    };
    // The fixture's one node sits at the origin, which the camera is pointed
    // at, so the frame's centre is the node's.
    let mid = ((SIZE[1] / 2) * SIZE[0] + SIZE[0] / 2) as usize;
    let middle = |px: &[u8]| brightness(&px[mid * 4..mid * 4 + 3]);

    let off = shooter.shot(&at(0.0));
    let on = shooter.shot(&at(0.8));
    assert!(
        middle(&on) > middle(&off),
        "the node's middle is no brighter with the glow on: {} against {}",
        middle(&on),
        middle(&off),
    );

    // A second sheet: one node behind, its light reaching far enough to wash
    // over the near node's whole footprint.
    let mut flat = at(0.8);
    flat.glow_reach = 3.0;
    let mut far = flat.nodes[0];
    far.world_pos.z = -1.0;
    far.world_pos.x += 0.6;
    far.glow = harmonigraph_scene::GlowStep { level: 1.0, row: 1, mix: 1.0, marked: 0.0 };
    far.color = glam::Vec4::new(0.9, 0.2, 0.2, 1.0);
    let mut sheets = at(0.8);
    sheets.glow_reach = 3.0;
    sheets.nodes.push(far);
    rows_per_node(&mut sheets);
    rows_per_node(&mut flat);

    let one_sheet = shooter.shot(&flat);
    let two_sheets = shooter.shot(&sheets);
    assert!(
        middle(&two_sheets) > middle(&one_sheet),
        "a sheet behind left the near node's middle at {} against {} with nothing behind it: \
         its light is being taken off its own body",
        middle(&two_sheets),
        middle(&one_sheet),
    );
}
