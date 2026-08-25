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

/// The Feather bar's claim: it fills a node's own reach in rather than
/// reaching further. The light's centre of mass moves outward, and the far
/// edge — which the Reach alone decides — stays where it was.
///
/// A light-weighted mean RADIUS, over the pixels the glow changed, rather than
/// an amount at a chosen distance: what the bar moves is where inside one span
/// the light sits, and any single annulus reads that as brightness. One
/// centered node on a cleared marker field ([`single_marked_node`]), so the profile
/// under the measurement is the falloff and nothing else.
#[test]
fn the_glow_feather_fills_a_nodes_reach_in_rather_than_reaching_further() {
    const SIZE: [u32; 2] = [256, 256];
    let Some(mut shooter) = Shooter::new(SIZE) else {
        return;
    };
    let at = |feather: f32| -> Scene {
        let mut scene = single_marked_node(0, 0);
        scene.glow_reach = 0.8;
        scene.glow_strength = 1.0;
        scene.glow_feather = feather;
        scene
    };
    let mut off = at(0.0);
    off.glow_reach = 0.0;
    let dark = shooter.shot(&off);
    let tight = shooter.shot(&at(0.0));
    let flat = shooter.shot(&at(1.0));

    // Every pixel the glow changed, as (radius from the node's centre, how much
    // light it gained) — the frame's centre being where the fixture's one node
    // sits.
    let lit = |a: &[u8]| -> Vec<(f32, f64)> {
        let row = SIZE[0] as usize;
        let centre = (SIZE[0] as f32 / 2.0, SIZE[1] as f32 / 2.0);
        a.chunks(4)
            .zip(dark.chunks(4))
            .enumerate()
            .filter_map(|(i, (x, y))| {
                let gained = (brightness(x) - brightness(y)) as f64;
                if gained <= 0.0 {
                    return None;
                }
                let (px, py) = ((i % row) as f32, (i / row) as f32);
                let r = ((px - centre.0).powi(2) + (py - centre.1).powi(2)).sqrt();
                Some((r, gained))
            })
            .collect()
    };
    let mean_radius = |lit: &[(f32, f64)]| -> f64 {
        let weight: f64 = lit.iter().map(|&(_, w)| w).sum();
        assert!(weight > 0.0, "a glow at reach 0.8 added no light at all");
        lit.iter().map(|&(r, w)| r as f64 * w).sum::<f64>() / weight
    };
    let edge = |lit: &[(f32, f64)]| -> f32 { lit.iter().fold(0.0f32, |m, &(r, _)| m.max(r)) };

    let (tight_lit, flat_lit) = (lit(&tight), lit(&flat));
    let (tight_r, flat_r) = (mean_radius(&tight_lit), mean_radius(&flat_lit));
    assert!(
        flat_r > tight_r * 1.2,
        "a feathered light must sit further out in its own span: {flat_r:.1}px against \
         {tight_r:.1}px",
    );
    // And barely further out than the unfeathered one: the window is the same
    // smoothstep across the same span at either setting, so the edge belongs to
    // the Reach. Not "the same to the pixel", and the tenth is not slack —
    // what a picture shows is where the light last cleared 1/255, and the flat
    // profile arrives at the window's own tail with sixteen times the amplitude
    // under it, which buys a few more pixels of a cubic tail before it
    // quantizes away. A tenth of the span is small against the difference the
    // Reach itself makes over that range (`the_glow_reach_says_how_far_a_node_
    // lights_past_its_own_edge`), which is the distinction being drawn.
    let (tight_edge, flat_edge) = (edge(&tight_lit), edge(&flat_lit));
    assert!(
        flat_edge <= tight_edge * 1.1,
        "the Feather bar moved the light's far edge from {tight_edge:.1}px to \
         {flat_edge:.1}px — the span is the Reach's to say",
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

/// A node a nearer sheet's node COVERS cuts nothing out of it: not a ring, and
/// not a shadow anywhere in the near node's halo. What a hidden node may do is
/// BRIGHTEN, and that is the whole of what it may do.
///
/// The case is a harmonic seventh over its home node, face on: the seventh's
/// disc and knockout hide the home node entirely. Every node draws AFTER the
/// whole light, so a covered node's ink and name are simply covered, and its
/// halo is one term of the melded field the near node stands on rather than a
/// layer laid over the near node's ink. Nothing in that field is subtractive,
/// so there is nothing a hidden node could cut with even if it reached.
///
/// Measured over the near node's own INK — the pixels its paint covers in full
/// — and per CHANNEL, since that is the claim the wash rests on: the light a
/// node's ink takes is a screen (`node_paint`), which can add to a channel and
/// never take from one, so a far sheet's red halo cannot pull the green out of
/// a white name in front of it.
///
/// It rests on the fixture's fresh WASH, that being what puts any light on a
/// node's ink at all: at a wash of 0 the near node's ink is untouchable and
/// there is nothing here to measure. The count of pixels the hidden node
/// brightens is asserted for that reason, and says so when it is 0.
///
/// That set is found rather than described, and found on the GROUND rather than
/// on the light, which reaches the ink too: a pixel the node paints opaquely is
/// a pixel no ground shows through, so it is the pixel that does not move when
/// the ground does. Two shots with the glow off and the darkest
/// and brightest grounds there are agree exactly over that ink and nowhere else
/// the node draws, the clearing painting the ground by definition. Every other
/// pixel of the node is that clearing, which paints the light over the ground
/// on purpose — the halo behind it belongs there, and asking for it back would
/// be asking for the hole this design exists to not have.
///
/// The hidden node is LIT and NAMED, both at the middle of the near node where
/// the light is fullest. The name is measured on its OWN footprint and not on
/// the ink: a glyph lands in the empty middle the rings stand around, so no
/// opaque pixel can carry that claim.
///
/// And that middle is where the near node's knockout STOPS: a clearing is the
/// rings a node draws, one reach out (`node_clearing`), so a hidden node's name
/// standing in the middle of the node in front of it is not hidden by it. That
/// is the last block below, and it is a fact about the knockout rather than a
/// claim about names: what hides a name is INK, and a node has none in its
/// middle. Hiding a covered node's name outright would need a mechanism of its
/// own, and the knockout is not it.
#[test]
fn a_node_under_a_nearer_sheets_node_cuts_nothing_out_of_its_light() {
    const SIZE: [u32; 2] = [256, 256];
    let Some(mut shooter) = Shooter::new(SIZE) else {
        return;
    };
    let near = |reach: f32| {
        let mut scene = single_marked_node(0, 0);
        scene.camera = harmonigraph_scene::Camera {
            projection: harmonigraph_scene::Projection::Orthographic,
            yaw: 0.0,
            pitch: 0.0,
            ..Default::default()
        };
        scene.glow_reach = reach;
        scene.glow_strength = 1.5;
        // A clearing, so the near node's knockout covers the far node's ink
        // in the scene pass; what is measured is the light, not the knockout.
        scene.glow_shadow = 0.4;
        scene
    };
    let covering = |wash: f32| -> Scene {
        let mut both = near(0.8);
        both.glow_wash = wash;
        let mut far = both.nodes[0];
        far.world_pos.z = -1.0;
        // Smaller and off centre, so the whole of what it draws falls inside
        // the near node's clearing while its halo reaches where the near node's
        // light is PARTIAL — at the middle the light is full, and full light
        // melded with anything is still full.
        far.scale = 0.5;
        far.world_pos.x += 0.4;
        far.glow = harmonigraph_scene::GlowStep { level: 0.8, row: 1, mix: 1.0, marked: 0.0 };
        far.color = glam::Vec4::new(0.9, 0.2, 0.2, 1.0);
        both.nodes.push(far);
        rows_per_node(&mut both);
        both
    };
    let fresh_wash = single_marked_node(0, 0).glow_wash;
    let both = covering(fresh_wash);
    // The hidden node names itself, at the middle of the near node.
    let name = |node: u32| LatticeLabels {
        glyphs: vec![GlyphInstance {
            rect: [112.0, 112.0, 32.0, 32.0],
            fill: [255, 255, 255, 255],
            rim: [0, 0, 0, 255],
            ..crate::text::tests::glyph()
        }],
        labels: vec![Label { node, glyphs: 1 }],
        rings: [TextRing::default(); 2],
        atlas: Some(crate::text::tests::atlas()),
        marks: None,
        slide: SlideAxis::default(),
    };

    let call = LatticeCallback::from_scene(
        &both,
        name(1),
        egui::vec2(SIZE[0] as f32, SIZE[1] as f32),
        wgpu::TextureFormat::Rgba8Unorm,
        0,
        None,
    );
    assert_eq!(
        call.instances[0].world_pos[2], -1.0,
        "the fixture puts the second node BEHIND the first, or it is covering rather than covered",
    );

    let alone_on = shooter.shot(&near(0.8));
    // The two grounds, with the glow off so that the ground is the only thing
    // moving. Black and white rather than two greys: the widest step there is
    // leaves no pixel of the clearing agreeing across it by rounding, and a
    // pixel too faint to differ even here is a pixel the pass discarded and the
    // `drawn` test drops.
    let mut on_ground = |bg: glam::Vec4| {
        let mut scene = near(0.0);
        scene.background = bg;
        shooter.shot(&scene)
    };
    let dark_ground = on_ground(glam::Vec4::new(0.0, 0.0, 0.0, 1.0));
    let pale_ground = on_ground(glam::Vec4::ONE);
    let covered = shooter.shot_with(&both, name(1));
    // The near node's opaque ink: drawn (so not the cleared black the pass
    // starts from) and the same over either ground (so neither its clearing nor
    // a soft edge of its own paint).
    let ink: Vec<usize> = (0..alone_on.len())
        .step_by(4)
        .filter(|&i| {
            pale_ground[i..i + 4] != [0u8, 0, 0, 255]
                && pale_ground[i..i + 4] == dark_ground[i..i + 4]
        })
        .collect();
    assert!(ink.len() > 500, "the near node painted {} opaque pixels", ink.len());
    let moved = |cmp: fn(u8, u8) -> bool| {
        ink.iter().filter(|&&i| (0..3).any(|c| cmp(covered[i + c], alone_on[i + c]))).count()
    };
    let dimmed = moved(|a, b| a < b);
    // Non-vacuous over the INK itself, not merely somewhere in the frame: the
    // hidden node's halo is part of the melded field the near node's own paint
    // is washed with, so it DOES reach that paint, and "never darker" is being
    // asked of pixels a hidden node actually moves. That reach is the tradeoff
    // the melded field is taken for — see `node_paint`, where the alternative
    // is a light pass per sheet.
    assert!(
        moved(|a, b| a > b) > 0,
        "the hidden node moved none of the near node's {} opaque pixels; the comparison is vacuous",
        ink.len(),
    );
    assert_eq!(
        dimmed,
        0,
        "a node hidden behind the near one darkened {dimmed} of its {} opaque pixels",
        ink.len(),
    );
    // The NAME's own reading, which the ink set cannot carry at all: a glyph
    // sits in the node's empty MIDDLE, where the rings stand around nothing and
    // there is no opaque ink to move. So it is measured over the pixels a name
    // actually covers, found by giving the same glyph to the NEAR node — whose
    // name is drawn — and read against a shot of the same two nodes with nobody
    // named. Both carry the hidden node's halo, so what differs between them is
    // the name and only the name.
    let unnamed = shooter.shot(&covering(fresh_wash));
    let named_near = shooter.shot_with(&covering(fresh_wash), name(0));
    let glyph: Vec<usize> = (0..unnamed.len())
        .step_by(4)
        .filter(|&i| named_near[i..i + 4] != unnamed[i..i + 4])
        .collect();
    assert!(
        glyph.len() > 500,
        "the fixture's name covers {} pixels; there is no name here to read",
        glyph.len(),
    );
    // Most of it, not all: the near node's rings reach over the edge of the
    // glyph's box and cover what lands on them, which is the ink doing the
    // hiding and is the whole of what still does.
    let through = glyph.iter().filter(|&&i| covered[i..i + 4] != unnamed[i..i + 4]).count();
    assert!(
        through * 2 > glyph.len(),
        "a hidden node's name reached {through} of the {} pixels it covers — the knockout is \
         filling the near node's middle again, and the name behind it with it",
        glyph.len(),
    );
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
/// The second: a node on a sheet BEHIND adds its halo to the field, and a
/// node's clearing paints that field over the ground (`node_paint`), so the
/// near node's middle comes out brighter than it is with nothing behind it. A
/// nearer node's body taking the light of the sheets behind off itself is what
/// inverts this — its middle then holds its own light alone while the ground a
/// few pixels away holds everyone's, and the node reads as a hole rather than
/// as a lamp. The near node carries a CLEARING here for exactly that reason:
/// its footprint is the surface such a pass would erase.
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
        scene.glow_shadow = 0.16;
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
