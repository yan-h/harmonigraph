//! The Gap: how far a ring holds light off itself, and how that decays.

use crate::*;
use super::fixtures::*;

/// A node STANDS ITS LIGHT OFF the rings it draws, and the Gap depth is the
/// one switch on it.
///
/// The standoff is a term of the node's own clearing rather than a hole cut in
/// the light: the clearing paints the finished field over the ground
/// (`node_paint`), and around every ring the node draws it paints that field
/// dimmed. So what this measures is a pixel just outside the octave band —
/// inside the clearing, outside every shape the node inks — where the light is
/// otherwise at nearly its fullest, the falloff being measured from the node's
/// centre.
///
/// The GROUND is the whole of what this bar moves, a node's own ink being the
/// Wash bar's — [`a_ring_wears_the_wash_inside_its_own_dark_pool`] holds that
/// boundary from the other side. So the probe sits outside the ink on purpose,
/// and not merely for want of light there: a probe ON the ink would read
/// nothing this bar does at any setting.
///
/// TWO claims, and the second is what makes the bar an A/B rather than a
/// restyle. The depth takes light: the probe is darker at the fresh 85% than
/// at 0, and no pixel anywhere in the frame is brighter, the term the depth
/// scales being a factor on light that was going to be laid down anyway. And a
/// depth of 0 is the whole feature off: the frame is byte for byte the same at
/// any Gap, which is the one place the four dials can be proved not to leak
/// into a picture that is supposed to have no standoff in it.
///
/// A Gap of 0 is deliberately NOT the off position and is not compared here: it
/// is a standoff whose fade has collapsed onto the ring's own annulus, which is
/// a CRISPER one, not an absent one.
///
/// [`the_middle_of_a_node_is_where_its_light_is_fullest`]'s fixture, whose
/// clearing is what the standoff lives in, plus one calibration shot: with the
/// glow and the clearing both off, the outermost pixel the node inks along +x
/// IS the band's outer edge, which is `rings_outer` in the node's own uv. That
/// is the scale everything below is measured in, so the probe follows the
/// fixture instead of naming a pixel.
#[test]
fn the_gap_depth_says_how_much_light_a_ring_stands_off() {
    const SIZE: [u32; 2] = [256, 256];
    let Some(mut shooter) = Shooter::new(SIZE) else {
        return;
    };
    let at = |reach: f32, gap: f32, depth: f32| -> Scene {
        let mut scene = single_marked_node(0, 0);
        scene.camera = harmonigraph_scene::Camera {
            projection: harmonigraph_scene::Projection::Orthographic,
            yaw: 0.0,
            pitch: 0.0,
            ..Default::default()
        };
        scene.glow_reach = reach;
        scene.glow_strength = 1.5;
        scene.glow_gap = gap;
        // The fade the whole width of the gap, which is the fresh pair.
        scene.glow_gap_soft = gap;
        scene.glow_gap_depth = depth;
        scene.nodes[0].gutter = 0.16;
        scene
    };
    let row = SIZE[0] as usize;
    let centre = (SIZE[1] / 2) as usize * row + (SIZE[0] / 2) as usize;

    // The scale: the node's own ink with nothing around it. The clearing is off
    // for this shot alone — it paints the ground out past the ink, and what is
    // wanted here is where the INK stops.
    let mut bare = at(0.0, 0.0, 0.0);
    bare.nodes[0].gutter = 0.0;
    let plain = shooter.shot(&bare);
    let inked = |px: &[u8], i: usize| px[i * 4..i * 4 + 4] != [0u8, 0, 0, 255];
    let band_px = (1..(SIZE[0] / 2) as usize)
        .rfind(|&x| inked(&plain, centre + x))
        .expect("the fixture's node must ink something along +x");
    assert!(band_px > 20, "the node inked only {band_px}px of radius; there is nothing to read");
    // Half a Gap past the band's outer edge: the standoff is solid there and
    // the node inks nothing, so the whole of the difference below is light.
    let probe = centre
        + (band_px as f32 * (1.0 + 0.08 / bare.rings_outer)).round() as usize;
    assert!(
        !inked(&plain, probe),
        "the probe at {probe} sits on the node's own ink, not outside it",
    );

    let lit = |px: &[u8], i: usize| brightness(&px[i * 4..i * 4 + 3]);
    let stood_off = shooter.shot(&at(0.8, 0.16, 0.85));
    let flat = shooter.shot(&at(0.8, 0.16, 0.0));
    assert!(
        lit(&stood_off, probe) < lit(&flat, probe),
        "the standoff left the pixel outside the ring at {} against {} with the depth at 0",
        lit(&stood_off, probe),
        lit(&flat, probe),
    );
    // Non-vacuous: there has to be light there to stand off in the first place.
    let dark = shooter.shot(&at(0.0, 0.16, 0.85));
    assert!(
        lit(&flat, probe) > lit(&dark, probe),
        "the fixture lights the probe no more than the glow off does; the comparison is vacuous",
    );
    // A factor on light that was going to be laid down anyway, so it can only
    // take: no pixel in the frame comes out brighter for it.
    let brighter = stood_off
        .chunks(4)
        .zip(flat.chunks(4))
        .filter(|(a, b)| brightness(&a[..3]) > brightness(&b[..3]))
        .count();
    assert_eq!(brighter, 0, "the standoff brightened {brighter} pixels");

    // The bar's top is the bare ground: where the standoff is solid, a depth of
    // 1 leaves the pixel exactly what it is with the glow off — not nearly,
    // since the clearing at full coverage replaces what is under it and a
    // factor of 0 on the light is no light. BOTH fades are taken off for this
    // pair of shots: the standoff's, and the clearing's it is floored at,
    // which at the fresh width runs nearly the whole gutter — so the probe
    // sits in a solid band rather than on either ramp.
    let solid = |reach: f32| {
        let mut scene = at(reach, 0.16, 1.0);
        scene.glow_gap_soft = 0.0;
        scene.sevens_soft = 0.0;
        scene
    };
    let bare_ground = shooter.shot(&solid(0.8));
    let no_glow = shooter.shot(&solid(0.0));
    assert_eq!(
        bare_ground[probe * 4..probe * 4 + 4],
        no_glow[probe * 4..probe * 4 + 4],
        "at a depth of 1 the stood-off pixel is not the frame with no glow in it",
    );

    // And the depth is the whole switch: at 0 the Gap and its curve reach the
    // picture nowhere.
    for (name, gap) in [("no gap", 0.0), ("a wide gap", 0.5)] {
        let other = shooter.shot(&at(0.8, gap, 0.0));
        assert_eq!(
            differing_pixels(&other, &flat),
            0,
            "at a depth of 0, {name} drew a different frame from the fresh gap",
        );
    }
}

/// The standoff has no radius at which it stops: it is still taking light a
/// tenth of a Gap PAST the Gap bar's outer handle, and less of it further out
/// again.
///
/// The claim the picture rests on. A fade that lands on nothing at one distance
/// puts a closed contour into a field that has no other — every length in the
/// light is an exponential (`glow_layer`) — and a circle is what the eye finds
/// in a smooth field however gently the ramp meets it. What that reads as is a
/// dark disc with a rim on it rather than a shadow, and it is exactly what a
/// strong glow shows: the halo out there is flat enough to have no gradient of
/// its own for the fade's own end to hide in.
///
/// TWO annuli, both outside the handle, and the second is what makes it a decay
/// rather than an overhang: a fade that merely reached further would pass the
/// first and could still be a band with a wider edge. Annuli and not pixels
/// because what is out there is a few code values on an 8-bit target — the
/// tail is under the quantum long before it is under the arithmetic — and a
/// ring of pixels is what carries a fraction of one.
///
/// At the depth's top and the curve's bottom, which is where the tail has the
/// most to show: the depth sets how many e-folds the fade spends
/// (`glow_shade`), and the curve's bottom is the plain exponential. The shape
/// is the same at every setting; what changes is whether a byte can hold it.
#[test]
fn the_standoff_reaches_past_the_gap_it_is_dialled_to() {
    const SIZE: [u32; 2] = [256, 256];
    const GAP: f32 = 0.34;
    let Some(mut shooter) = Shooter::new(SIZE) else {
        return;
    };
    let at = |depth: f32| -> Scene {
        let mut scene = single_marked_node(0, 0);
        scene.camera = harmonigraph_scene::Camera {
            projection: harmonigraph_scene::Projection::Orthographic,
            yaw: 0.0,
            pitch: 0.0,
            ..Default::default()
        };
        scene.glow_reach = 2.5;
        scene.glow_strength = 2.0;
        scene.glow_gap = GAP;
        scene.glow_gap_soft = GAP;
        scene.glow_gap_shape = 0.0;
        scene.glow_gap_depth = depth;
        scene.nodes[0].gutter = 0.0;
        scene
    };
    let row = SIZE[0] as usize;
    let (cx, cy) = ((SIZE[0] / 2) as usize, (SIZE[1] / 2) as usize);
    let centre = cy * row + cx;

    // The scale, as `the_gap_depth_says_how_much_light_a_ring_stands_off` takes
    // it: the outermost pixel the node inks along +x is the band's outer edge,
    // which is `rings_outer` in the node's own uv.
    let bare = at(0.0);
    let plain = shooter.shot(&{
        let mut scene = at(0.0);
        scene.glow_reach = 0.0;
        scene
    });
    let inked = |px: &[u8], i: usize| px[i * 4..i * 4 + 4] != [0u8, 0, 0, 255];
    let band_px = (1..(SIZE[0] / 2) as usize)
        .rfind(|&x| inked(&plain, centre + x))
        .expect("the fixture's node must ink something along +x");
    let gap_px = band_px as f32 * GAP / bare.rings_outer;

    let flat = shooter.shot(&at(0.0));
    let stood_off = shooter.shot(&at(1.0));
    // What the standoff took per lit pixel, over the ring of pixels standing
    // between `from` and `to` Gaps out from the band's own edge.
    let took = |from: f32, to: f32| -> (f64, usize) {
        let (mut sum, mut n) = (0i64, 0usize);
        for y in 0..SIZE[1] as usize {
            for x in 0..SIZE[0] as usize {
                let i = y * row + x;
                let dx = x as f32 - cx as f32;
                let dy = y as f32 - cy as f32;
                let g = ((dx * dx + dy * dy).sqrt() - band_px as f32) / gap_px;
                if g < from || g >= to || inked(&plain, i) {
                    continue;
                }
                sum += brightness(&flat[i * 4..i * 4 + 3])
                    - brightness(&stood_off[i * 4..i * 4 + 3]);
                n += 1;
            }
        }
        (sum as f64 / n.max(1) as f64, n)
    };

    let (near, near_n) = took(1.1, 1.35);
    let (far, far_n) = took(1.35, 1.6);
    assert!(near_n > 200 && far_n > 200, "the annuli hold {near_n} and {far_n} pixels to read");
    assert!(
        near > 0.5,
        "the standoff stopped at the Gap's own handle: a tenth of a Gap past it, it took \
         {near:.2} of a code value per pixel",
    );
    assert!(
        far > 0.0 && far < near,
        "the standoff took {far:.2} per pixel out at a Gap and a half against {near:.2} just \
         past one, which is a wider band rather than a decay",
    );
}

/// The Gap depth is a number of STOPS, spent geometrically across the fade, and
/// not a share of the light taken off in proportion.
///
/// What that buys is the whole of why the fade is worth spending on: sight
/// answers ratios, so a factor walked evenly in VALUE spends most of what can
/// be SEEN of it in the first fraction of its width, and the pool then reads as
/// a dark ring hugging the ink with an edge on it whatever the Gap is dialled
/// to. See `glow_shade`, which is the one line this pins.
///
/// Measured as a SQUARE, which is what makes it an identity rather than a
/// direction: keeping a quarter of the light is keeping a half of it twice, so
/// at every pixel of the halo the shot at the depth that leaves a quarter is
/// the shot at the depth that leaves a half, squared — against the unstood-off
/// light as the unit. A linear factor gets that wrong in the middle of the fade
/// and right at both of its ends, so the probes are taken across the fade
/// rather than at one radius.
///
/// On the black ground every shot here is taken against: the light is
/// premultiplied, so a pixel outside the node's ink IS the light times what the
/// standoff keeps of it, and the ratio of two shots is the ratio of two keeps
/// with the colour divided out.
#[test]
fn the_gap_depth_is_spent_in_stops_and_not_in_proportion() {
    const SIZE: [u32; 2] = [256, 256];
    const GAP: f32 = 0.34;
    let Some(mut shooter) = Shooter::new(SIZE) else {
        return;
    };
    let at = |depth: f32| -> Scene {
        let mut scene = single_marked_node(0, 0);
        scene.camera = harmonigraph_scene::Camera {
            projection: harmonigraph_scene::Projection::Orthographic,
            yaw: 0.0,
            pitch: 0.0,
            ..Default::default()
        };
        scene.glow_reach = 1.6;
        scene.glow_strength = 1.5;
        scene.glow_gap = GAP;
        scene.glow_gap_soft = GAP;
        scene.glow_gap_depth = depth;
        scene.nodes[0].gutter = 0.0;
        scene
    };
    let row = SIZE[0] as usize;
    let centre = (SIZE[1] / 2) as usize * row + (SIZE[0] / 2) as usize;
    let bare = at(0.0);
    let plain = shooter.shot(&{
        let mut scene = at(0.0);
        scene.glow_reach = 0.0;
        scene
    });
    let inked = |px: &[u8], i: usize| px[i * 4..i * 4 + 4] != [0u8, 0, 0, 255];
    let band_px = (1..(SIZE[0] / 2) as usize)
        .rfind(|&x| inked(&plain, centre + x))
        .expect("the fixture's node must ink something along +x");

    let whole = shooter.shot(&at(0.0));
    let half = shooter.shot(&at(0.5));
    let quarter = shooter.shot(&at(0.75));
    let lit = |px: &[u8], i: usize| brightness(&px[i * 4..i * 4 + 3]) as f64;

    // Across the fade rather than at one radius: a proportional factor agrees
    // with this at both ends of the band and differs most in the middle of it.
    let mut checked = 0;
    for step in 1..=8 {
        let p = 0.15 + 0.1 * step as f32;
        let probe = centre + (band_px as f32 * (1.0 + p * GAP / bare.rings_outer)).round() as usize;
        if inked(&plain, probe) {
            continue;
        }
        let (l0, l1, l2) = (lit(&whole, probe), lit(&half, probe), lit(&quarter, probe));
        // Under a tenth of the unstood-off light there is not enough left in
        // the byte to square anything with.
        if l0 < 76.0 {
            continue;
        }
        let want = (l1 / l0) * (l1 / l0);
        let got = l2 / l0;
        assert!(
            (got - want).abs() < 0.035,
            "{p} of a Gap out the depth that leaves a quarter kept {got:.3} of the light \
             where the depth that leaves a half, squared, is {want:.3}",
        );
        checked += 1;
    }
    assert!(checked >= 4, "only {checked} probes had light to read; the test proves nothing");
}

/// The Gap reaches as far as it says, and the Clearance is not a lid on it.
///
/// The standoff is written into a layer of the LIGHT (`fs_glow`), so it dims
/// the field wherever that field reaches. A standoff carried instead by the
/// ground a node's own clearing paints is bounded at the Clearance's reach —
/// solid inward, where the clearing fills every footprint to the node's centre,
/// and gone a fraction of a node-radius outward, where the clearing has faded
/// out — which makes a Gap wider than the Clearance half a dial: it eats inward
/// and does nothing outward.
///
/// So the probe sits OUTSIDE the clearing altogether — five times further from
/// the ring than the Clearance reaches — and the two claims are what pin the
/// difference. The standoff dims it, which is the light being held off where no
/// node paints. And dialling the Clearance to nothing does not move it: what
/// holds the light off there is the Gap alone, so the pixel is identical with a
/// clearing and with none. Under the bounded shape neither shot moves, both
/// being the undimmed field.
///
/// The Clearance is deliberately not 0 in the first shot. A node that clears
/// nothing is the easy case — there is no lid to prove the standoff has got out
/// from under. The case worth pinning is a node whose clearing exists, ends,
/// and does not take the standoff's reach with it.
#[test]
fn the_gap_reaches_past_the_clearance_the_node_cuts() {
    const SIZE: [u32; 2] = [256, 256];
    let Some(mut shooter) = Shooter::new(SIZE) else {
        return;
    };
    // A narrow clearing under a wide gap, which is the pair a standoff bounded
    // by the clearing cannot draw. The fade is left at the full gap, the fresh
    // pairing, so the
    // probe reads the ramp rather than a band edge.
    const CLEARANCE: f32 = 0.02;
    const GAP: f32 = 0.5;
    let at = |reach: f32, depth: f32, gutter: f32| -> Scene {
        let mut scene = single_marked_node(0, 0);
        scene.camera = harmonigraph_scene::Camera {
            projection: harmonigraph_scene::Projection::Orthographic,
            yaw: 0.0,
            pitch: 0.0,
            ..Default::default()
        };
        scene.glow_reach = reach;
        scene.glow_strength = 1.5;
        scene.glow_gap = GAP;
        scene.glow_gap_soft = GAP;
        scene.glow_gap_depth = depth;
        scene.nodes[0].gutter = gutter;
        scene
    };
    let row = SIZE[0] as usize;
    let centre = (SIZE[1] / 2) as usize * row + (SIZE[0] / 2) as usize;

    // The scale, as `the_gap_depth_says_how_much_light_a_ring_stands_off` takes
    // it: the node's own ink with no clearing and no light, whose outermost lit
    // pixel along +x is `rings_outer` in the node's uv.
    let bare = at(0.0, 0.0, 0.0);
    let plain = shooter.shot(&bare);
    let inked = |px: &[u8], i: usize| px[i * 4..i * 4 + 4] != [0u8, 0, 0, 255];
    let band_px = (1..(SIZE[0] / 2) as usize)
        .rfind(|&x| inked(&plain, centre + x))
        .expect("the fixture's node must ink something along +x");
    let per_uv = band_px as f32 / bare.rings_outer;

    // Five times the Clearance out from the ink, and well inside the Gap: no
    // clearing of this node reaches here at any level, and the standoff still
    // has most of its own width left to spend.
    const PAST: f32 = CLEARANCE * 5.0;
    const { assert!(PAST < GAP, "the probe has to sit inside the gap it is measuring") };
    let probe = centre + (band_px as f32 + PAST * per_uv).round() as usize;
    assert!(
        !inked(&plain, probe),
        "the probe at {probe} sits on the node's own ink, not outside it",
    );

    let lit = |px: &[u8], i: usize| brightness(&px[i * 4..i * 4 + 3]);
    let stood_off = shooter.shot(&at(0.8, 0.85, CLEARANCE));
    let flat = shooter.shot(&at(0.8, 0.0, CLEARANCE));
    assert!(
        lit(&stood_off, probe) < lit(&flat, probe),
        "outside the clearing the standoff left the pixel at {} against {} with the depth at 0",
        lit(&stood_off, probe),
        lit(&flat, probe),
    );
    // Non-vacuous: there has to be light out there to hold off.
    let dark = shooter.shot(&at(0.0, 0.85, CLEARANCE));
    assert!(
        lit(&flat, probe) > lit(&dark, probe),
        "the fixture lights the probe no more than the glow off does; the comparison is vacuous",
    );

    // ...and it is the Gap's own doing, not the clearing's: take the Clearance
    // away entirely and the pixel does not move.
    let clearless = shooter.shot(&at(0.8, 0.85, 0.0));
    assert_eq!(
        clearless[probe * 4..probe * 4 + 4],
        stood_off[probe * 4..probe * 4 + 4],
        "the standoff outside the clearing changed when the Clearance was dialled off",
    );
}

/// The Gap reaches light this node never lit — a NEIGHBOUR's halo, out past
/// where its own light has shut.
///
/// The standoff is written per node into a layer of the light (`fs_glow`), one
/// quad per node, so what bounds it is that node's own billboard. The billboard
/// is sized to hold the LIGHT — the lit rim plus the Reach — and the Gap is a
/// length of its own with a ceiling of its own (`GLOW_GAP_MAX` against
/// `GLOW_REACH_MAX`), so a Gap dialled past the Reach asks for a standoff out
/// where this node draws no fragment at all. What an unheld bound looks like is
/// not a wrong value but a DISCONTINUITY: the fade stops dead partway down its
/// ramp, on a line that is straight and screen-aligned — `node_vertex` builds
/// the quad from `cam_right`/`cam_up` — so it slides around every node as the
/// camera turns while the lattice under it does not.
///
/// Hence a probe past `QUAD_MARGIN`, the floor the billboard takes at this
/// Reach, and inside `rings_outer + GAP`, where the standoff's own fade still
/// has most of its depth left. The light there is worth measuring only because
/// it is somebody else's: a node's own light shuts at its rim plus the Reach,
/// which is always inside its own quad, so the far side of the bound is lit by
/// the neighbour alone — the same split `fs_glow`'s early-out turns on, where
/// a node with no light of its own still stands its rings off a neighbour's.
#[test]
fn the_gap_reaches_light_the_nodes_own_never_lit() {
    // Wide enough for both nodes and a multiple of 64, so the readback's rows
    // stay aligned.
    const SIZE: [u32; 2] = [1408, 320];
    let Some(mut shooter) = Shooter::new(SIZE) else {
        return;
    };
    // The widest Gap the bar has against the fresh Reach — the pair that puts
    // the standoff outside the quad the light alone would size.
    const GAP: f32 = harmonigraph_scene::GLOW_GAP_MAX;
    const REACH: f32 = 0.35;
    // Where the neighbour stands, and where the light is read, both in the
    // probed node's uv. The probe is past `QUAD_MARGIN` (1.6) and inside the
    // fixture's `rings_outer + GAP`.
    const APART: f32 = 2.25;
    const PROBE: f32 = 1.65;

    // One node, and every bar of the standoff open but the depth.
    let alone = |reach: f32, depth: f32| -> Scene {
        let mut scene = single_marked_node(0, 0);
        scene.camera = harmonigraph_scene::Camera {
            projection: harmonigraph_scene::Projection::Orthographic,
            yaw: 0.0,
            pitch: 0.0,
            ..Default::default()
        };
        scene.glow_reach = reach;
        scene.glow_strength = 2.0;
        // An even field, so the light is still worth something out at the bound
        // rather than an exponential's tail.
        scene.glow_feather = 1.0;
        scene.glow_gap = GAP;
        scene.glow_gap_soft = GAP;
        // The fade held longest, which is what leaves a measurable share of the
        // standoff this far out.
        scene.glow_gap_shape = 1.0;
        scene.glow_gap_depth = depth;
        scene.nodes[0].gutter = 0.0;
        scene
    };
    // ...and the neighbour that lights it. Dialled almost off, so every term of
    // its OWN standoff — each one scaled by the level of the layer it stands off
    // — is worth nothing at the probe, while its light rides the glow's own
    // clock at full.
    let with_neighbour = |reach: f32, depth: f32| -> Scene {
        let mut scene = alone(reach, depth);
        let mut lamp = scene.nodes[0];
        lamp.world_pos = glam::Vec3::new(APART * scene.node_radius * 1.8, 0.0, 0.0);
        lamp.activation = 0.02;
        lamp.audio_ring = 0.0;
        lamp.glow.row = 1;
        lamp.glow.marked = 0.0;
        scene.nodes.push(lamp);
        scene.glow_rows = scene.nodes.len() as u32;
        scene
    };

    let row = SIZE[0] as usize;
    let centre = (SIZE[1] / 2) as usize * row + (SIZE[0] / 2) as usize;
    // The scale, taken off ONE node so the outermost ink along +x is this node's
    // own rings and not the neighbour's: that pixel is `rings_outer` in its uv.
    let solo = alone(0.0, 0.0);
    let plain = shooter.shot(&solo);
    let inked = |px: &[u8], i: usize| px[i * 4..i * 4 + 4] != [0u8, 0, 0, 255];
    let band_px = (1..(SIZE[0] / 2) as usize)
        .rfind(|&x| inked(&plain, centre + x))
        .expect("the fixture's node must ink something along +x");
    let per_uv = band_px as f32 / solo.rings_outer;
    let probe = centre + (PROBE * per_uv).round() as usize;
    assert!(
        !inked(&plain, probe),
        "the probe at {probe} sits on the node's own ink, not outside it",
    );

    let lit = |px: &[u8], i: usize| brightness(&px[i * 4..i * 4 + 3]);
    let flat = shooter.shot(&with_neighbour(REACH, 0.0));
    let gapped = shooter.shot(&with_neighbour(REACH, 1.0));
    // The light at the probe is the NEIGHBOUR's: take the neighbour away and
    // this node's own light does not reach, so what the standoff is holding off
    // out here was never this node's to lay down.
    let lonely = shooter.shot(&alone(REACH, 0.0));
    assert!(
        lit(&lonely, probe) < lit(&flat, probe) / 4,
        "the probe is lit by the node's own light ({} against {} with a neighbour), so it \
         measures nothing about a neighbour's",
        lit(&lonely, probe),
        lit(&flat, probe),
    );

    // And the Gap holds it off. A tenth is far under the share the bars ask for
    // here and far over what the neighbour's own hundredth of a standoff can
    // account for, so the threshold is the bound and not the arithmetic.
    assert!(
        lit(&gapped, probe) * 10 < lit(&flat, probe) * 9,
        "the Gap left a neighbour's light at {} against {} with the depth at 0, so the \
         standoff stopped at this node's own billboard",
        lit(&gapped, probe),
        lit(&flat, probe),
    );
}

/// How much of the light the standoff takes at one radius, angle by angle, at a
/// closed Octave gap and at the widest one — the two rings of shares every claim
/// about the standoff following the slices is stated on.
///
/// The share is `(lit - stood off) / lit` at a Gap depth of 1, which is the
/// standoff's own coverage and nothing else. Three things have to hold for that,
/// and the fixtures below carry all three: the clearing the standoff rides is
/// radial, so it contributes one constant factor around the turn; the light's
/// own falloff is radial too (`glow_layer`), so the only thing varying with
/// angle is what is being measured; and the ground is black, so a pixel's
/// brightness IS its light and the division is exact rather than nearly so.
///
/// `ink_uv` is where the fixture's own ink ends in the node's uv, and `past` how
/// far outside it to read. The scale between the two is taken from a calibration
/// shot rather than assumed: the ink is found at a CLOSED gap, which is the
/// widest it is ever drawn, and that same shot is what proves the probe ring
/// clears it at every angle.
///
/// A ratio between the two rings at ONE pixel is the strongest reading it
/// supports, and the reason is the pixel grid: the ring lands on integer pixels
/// so its radius wobbles by up to half of one, which the fade's slope turns into
/// a couple of hundredths of share. Two shots at one pixel share that wobble
/// exactly, so a ratio has none of it; a claim made across angles has to carry a
/// budget for it.
fn standoff_share_rings(
    shooter: &mut Shooter,
    size: [u32; 2],
    at: &dyn Fn(f32, f32, f32) -> Scene,
    ink_uv: f32,
    past: f32,
    angles: usize,
) -> (Vec<f64>, Vec<f64>) {
    let row = size[0] as usize;
    // The node projects to the frame's exact middle, which is a CORNER of the
    // pixel grid on an even frame and not the middle of any pixel: every radius
    // here is taken from there and floored into the pixel that holds it, so the
    // probe ring is centred on the node rather than half a pixel off it.
    let cx = 0.5 * size[0] as f32;
    let cy = 0.5 * size[1] as f32;
    let centre = (size[1] / 2) as usize * row + (size[0] / 2) as usize;

    let mut bare = at(0.0, 0.0, 0.0);
    bare.nodes[0].gutter = 0.0;
    let plain = shooter.shot(&bare);
    let inked = |px: &[u8], i: usize| px[i * 4..i * 4 + 4] != [0u8, 0, 0, 255];
    let ink_px = (1..(size[0] / 2) as usize)
        .rfind(|&x| inked(&plain, centre + x))
        .expect("the fixture's node must ink something along +x");
    assert!(ink_px > 20, "the node inked only {ink_px}px of radius; there is nothing to read");
    let probe_px = ink_px as f32 * (1.0 + past / ink_uv);
    let probe = |k: usize| -> usize {
        let a = std::f32::consts::TAU * k as f32 / angles as f32;
        // The framebuffer's rows run down where the node's own uv runs up.
        let y = (cy - probe_px * a.sin()).floor() as usize;
        y * row + (cx + probe_px * a.cos()).floor() as usize
    };
    for k in 0..angles {
        assert!(
            !inked(&plain, probe(k)),
            "the probe ring crosses the node's own ink {k} steps round",
        );
    }

    let lit = |px: &[u8], i: usize| brightness(&px[i * 4..i * 4 + 3]);
    let mut shares = |octave_gap: f32| -> Vec<f64> {
        let flat = shooter.shot(&at(octave_gap, 0.8, 0.0));
        let stood = shooter.shot(&at(octave_gap, 0.8, 1.0));
        (0..angles)
            .map(|k| {
                let i = probe(k);
                let light = lit(&flat, i);
                assert!(
                    light > 60,
                    "the fixture lit the probe {k} steps round to only {light}: there is \
                     too little light there to measure a share of",
                );
                (light - lit(&stood, i)) as f64 / light as f64
            })
            .collect()
    };
    (shares(0.0), shares(harmonigraph_scene::GAP_MAX))
}

/// A node stands its light off the ink its rings DRAW, and a ring is slices
/// with gaps between them rather than a closed annulus.
///
/// What this measures is the SHARE of the light one pixel loses to the standoff
/// — `(lit - stood off) / lit` at a depth of 1 — taken at one radius half a Gap
/// outside the band, all the way round the node. That share is the standoff's
/// own coverage and nothing else: the clearing carrying it is a disc
/// (`node_clearing` reads the band as a rim), the light's falloff is radial
/// (`glow_layer`), and whatever the light's colour does around the turn divides
/// out of a ratio taken per pixel.
///
/// TWO claims, and the second is what keeps the first from being a restyle.
/// Against a CLOSED ring's share at the same pixel, a wide Octave gap keeps
/// nearly all of its light in the middle of a gap — where the nearest ink is
/// half a gap away, further off than the Gap reaches — and loses all of the
/// same share as the closed ring over the middle of a slice, the ink there
/// being no further off than the annulus itself. And with the gap closed the
/// share is FLAT around the turn, which is the picture an angular term must not
/// be able to touch: a dark band no setting on the node asked for.
///
/// Per pixel is what makes the tolerance on that flatness a tight one. The
/// probe ring lands on integer pixels, so its radius wobbles by up to half of
/// one, which on the fade's own slope is a couple of hundredths of the share —
/// hence a budget rather than an equality, and the ratio in the first claim,
/// which compares two shots at ONE pixel and so has no wobble in it at all.
///
/// [`the_gap_depth_says_how_much_light_a_ring_stands_off`]'s fixture and its
/// probe radius, with no fade on the clearing so the probe sits in solid
/// coverage and the share is the standoff's whole answer.
#[test]
fn the_standoff_follows_the_gaps_between_the_slices() {
    // A big frame for a small measurement: the Gap's fade is 0.16 of a node's
    // uv, so at 256 it spans some seven pixels and half of one of those is a
    // twentieth of the share below. The node is drawn at the same size in uv
    // whatever the frame, so the pixels are what buys the resolution.
    const SIZE: [u32; 2] = [1024, 1024];
    const GAP: f32 = 0.16;
    // Enough of them that one lands near the middle of a gap and one near the
    // middle of a slice, at every wheel the view can be dialled to.
    const ANGLES: usize = 360;
    let Some(mut shooter) = Shooter::new(SIZE) else {
        return;
    };
    let at = |octave_gap: f32, reach: f32, depth: f32| -> Scene {
        let mut scene = single_marked_node(0, 0);
        scene.camera = harmonigraph_scene::Camera {
            projection: harmonigraph_scene::Projection::Orthographic,
            yaw: 0.0,
            pitch: 0.0,
            ..Default::default()
        };
        // The fresh wheel's own FRINGE, which `OctaveLayout::default` leaves
        // off: with no extras every slice is one width and the walk reads a
        // uniform ladder, which a boundary table built by arithmetic rather
        // than read out of the uniform would satisfy just as well.
        //
        // What no wheel can pin is the DIRECTION the walk takes that table in.
        // `octave_layout` mirrors the fringe, so the bounds satisfy
        // `b[k] + b[span-k] = TAU` at every setting, and a min over all of them
        // is the same answer whichever way round the fragment's angle is
        // measured. That is a property of the wheel, not a hole in this fixture.
        scene.octave_layout = harmonigraph_scene::octave_layout(
            harmonigraph_scene::DEFAULT_COUNT,
            harmonigraph_scene::DEFAULT_CENTER,
            2,
            harmonigraph_scene::DEFAULT_EXTRA_SIZE,
            harmonigraph_scene::DEFAULT_EXTRA_BLEND,
        );
        scene.octave_gap = octave_gap;
        scene.glow_reach = reach;
        scene.glow_strength = 1.5;
        scene.glow_gap = GAP;
        // The fade the whole width of the gap, which is the fresh pair.
        scene.glow_gap_soft = GAP;
        scene.glow_gap_depth = depth;
        scene.nodes[0].gutter = GAP;
        scene.sevens_soft = 0.0;
        scene
    };
    // The band's own outer edge is where this fixture's ink ends, and the Scene
    // names it.
    let ink_uv = at(0.0, 0.0, 0.0).rings_outer;
    let (closed, wide) =
        standoff_share_rings(&mut shooter, SIZE, &at, ink_uv, 0.5 * GAP, ANGLES);

    // The closed ring first, which is both the reference for the wide gap below
    // and a claim of its own.
    let mean = closed.iter().sum::<f64>() / ANGLES as f64;
    let drift = closed.iter().fold(0.0f64, |w, s| w.max((s - mean).abs()));
    assert!(
        mean > 0.15,
        "a closed ring took only {mean:.3} of the light at the probe; there is no \
         standoff here to find a gap in",
    );
    assert!(
        drift <= 0.05,
        "a closed ring's standoff swung by {drift:.3} around the turn, off a mean of \
         {mean:.3}: it is not standing the light off evenly the whole way round",
    );

    // And the wide gap, against that same pixel's share.
    let ratio: Vec<f64> = wide.iter().zip(&closed).map(|(w, c)| w / c).collect();
    let emptiest = ratio.iter().fold(f64::MAX, |m, r| m.min(*r));
    let deepest = ratio.iter().fold(f64::MIN, |m, r| m.max(*r));
    assert!(
        emptiest < 0.15,
        "the widest Octave gap still took {emptiest:.3} of the closed ring's share at \
         its emptiest angle: the standoff is not following the slices",
    );
    assert!(
        deepest > 0.85,
        "over a slice the widest Octave gap took only {deepest:.3} of the closed ring's \
         share: the standoff is following something narrower than the ink",
    );
}

/// A slice PAST A HALF TURN is ink all the way in to the node's centre down its
/// own middle, and the standoff follows it there.
///
/// The wheel hands one out at the bottom of its own bar: an octave count of 1
/// with the fresh two extras either side leaves the middle slice 259 degrees
/// (`octave_layout`). `outer_glyph` cuts each gap only on the side its edge runs
/// to, so down the middle of a slice that wide NO edge cuts anything, however
/// close the edges' own lines pass on their way through the centre — which is
/// why `oct_arc_coverage` carries a union branch for exactly this wedge.
///
/// A standoff measuring the distance to the nearest boundary RAY has to say the
/// same, and the reading that asks only "how far is the nearest ray" says the
/// opposite: near the centre every ray is close, so it calls the widest slice's
/// middle a gap and hands the light back exactly where the ink is.
///
/// Measured inside HALF the Octave gap, which is where the two readings can
/// disagree at all — further out than that, half a gap is spent before the
/// nearest ray is reached and both call it ink. The reading is the MAXIMUM
/// share around the turn, against a closed ring's share at the same pixel:
/// somewhere on that circle is the wide slice's middle, ink in both pictures, so
/// the two have to stand the light off there by the same amount. A per-pixel
/// ratio is also what makes the probe ring's half-pixel wobble cancel — where
/// the two shots agree on the shape, they agree whatever radius the pixel
/// landed at.
///
/// The OCTAVE BAND carries it, dialled in to the node's centre: the walk is
/// shared by every term of `glow_standoff`, and the band is the layer that both
/// stands light off and gives it off (`ink_at` leaves the analyzer's ring out of
/// the light), so one layer can be the whole fixture.
#[test]
fn a_slice_past_a_half_turn_is_stood_off_down_its_middle() {
    const SIZE: [u32; 2] = [1024, 1024];
    // Small against the band's own radius, so the probe sits well inside half
    // the Octave gap with the fade still spending most of itself there.
    const GAP: f32 = 0.05;
    const BAND_OUTER: f32 = 0.15;
    const PAST: f32 = 0.01;
    const ANGLES: usize = 360;
    let Some(mut shooter) = Shooter::new(SIZE) else {
        return;
    };
    let at = |octave_gap: f32, reach: f32, depth: f32| -> Scene {
        let mut scene = single_marked_node(0, 0);
        scene.camera = harmonigraph_scene::Camera {
            projection: harmonigraph_scene::Projection::Orthographic,
            yaw: 0.0,
            pitch: 0.0,
            ..Default::default()
        };
        // One slice past a half turn, which is the whole fixture: the count at
        // its floor with the fresh fringe either side.
        scene.octave_layout = harmonigraph_scene::octave_layout(
            harmonigraph_scene::MIN_COUNT,
            harmonigraph_scene::DEFAULT_CENTER,
            2,
            harmonigraph_scene::DEFAULT_EXTRA_SIZE,
            harmonigraph_scene::DEFAULT_EXTRA_BLEND,
        );
        // No analyzer at all, and the BAND reaching the node's centre: one layer
        // standing the light off, and its footprint is where the two readings
        // differ.
        scene.spectral.inner = 0.0;
        scene.spectral.outer = 0.0;
        scene.outer_inner = 0.0;
        scene.outer_outer = BAND_OUTER;
        // Every slice voiced, so each reads the same: a slice at a level of its
        // own would put the light's own pattern into the share.
        scene.nodes[0].octaves = [1.0; harmonigraph_scene::OCTAVE_SLOTS];
        scene.octave_gap = octave_gap;
        scene.glow_reach = reach;
        scene.glow_strength = 1.5;
        scene.glow_gap = GAP;
        scene.glow_gap_soft = GAP;
        scene.glow_gap_depth = depth;
        scene.nodes[0].gutter = 0.16;
        scene.sevens_soft = 0.0;
        scene
    };
    // Inside half the widest gap, which is the only radius at which the two
    // readings can differ at all.
    const {
        assert!(
            BAND_OUTER + PAST < 0.5 * harmonigraph_scene::GAP_MAX,
            "the probe sits outside half an Octave gap, where every reading agrees",
        )
    };
    let (closed, wide) =
        standoff_share_rings(&mut shooter, SIZE, &at, BAND_OUTER, PAST, ANGLES);

    let least = closed.iter().fold(f64::MAX, |m, s| m.min(*s));
    assert!(
        least > 0.3,
        "a closed ring took only {least:.3} of the light somewhere on the probe ring; \
         there is no standoff there to compare a wide gap against",
    );

    let ratio: Vec<f64> = wide.iter().zip(&closed).map(|(w, c)| w / c).collect();
    let deepest = ratio.iter().fold(f64::MIN, |m, r| m.max(*r));
    let emptiest = ratio.iter().fold(f64::MAX, |m, r| m.min(*r));
    assert!(
        deepest > 0.85,
        "down the middle of a 259-degree slice the widest Octave gap took only \
         {deepest:.3} of the closed ring's share: the standoff is reading ink as gap \
         where no edge cuts",
    );
    // Non-vacuous the other way: the narrow slices really are eaten at this
    // radius, so the ring the ratio is taken over is not simply solid.
    assert!(
        emptiest < 0.15,
        "every angle kept {emptiest:.3} or more of the closed ring's share; the wide \
         gap is not opening anywhere and the claim above is vacuous",
    );
}

/// A MARK's standoff stops where the gap cuts the mark's own sides.
///
/// The wedge a mark is drawn in is not the wedge its slot owns: `outer_glyph`
/// takes half an Octave gap off each of its sides, exactly as it does for the
/// slices of a ring. `sector_distance` measures the slot's wedge, so a standoff
/// reading it alone stands the light off from the BOUNDARY — half a gap wider
/// than the ink, on both sides of every mark.
///
/// The measurement that separates the two is the middle of a gap between two
/// marks: the nearest ink there is half an Octave gap away, and at the widest
/// gap that is 0.2 against a Gap of 0.16, so the light has to be fully back.
/// Read off the boundary the two wedges share, which is where the un-eroded
/// wedge puts its own edge and so reads as a distance of nothing.
///
/// Every slot marked, and the band and the ring both dialled off: the strip is
/// then a full ring of wedges cut by the one gap, which is what lets the same
/// probe ring read it, and the marks are the only term `glow_standoff` has.
#[test]
fn a_marks_standoff_stops_where_the_gap_cuts_its_sides() {
    const SIZE: [u32; 2] = [1024, 1024];
    const GAP: f32 = 0.16;
    const STRIP_IN: f32 = 0.5;
    const STRIP_THICK: f32 = 0.12;
    const ANGLES: usize = 360;
    let Some(mut shooter) = Shooter::new(SIZE) else {
        return;
    };
    let at = |octave_gap: f32, reach: f32, depth: f32| -> Scene {
        // Every slot the wheel can show, so the strip closes into a ring.
        let mut scene = single_marked_node((1 << harmonigraph_scene::OCTAVE_SLOTS) - 1, 0);
        scene.camera = harmonigraph_scene::Camera {
            projection: harmonigraph_scene::Projection::Orthographic,
            yaw: 0.0,
            pitch: 0.0,
            ..Default::default()
        };
        scene.octave_layout = harmonigraph_scene::octave_layout(
            harmonigraph_scene::DEFAULT_COUNT,
            harmonigraph_scene::DEFAULT_CENTER,
            2,
            harmonigraph_scene::DEFAULT_EXTRA_SIZE,
            harmonigraph_scene::DEFAULT_EXTRA_BLEND,
        );
        // The band and the ring off, so the marks are the only thing standing
        // any light off and the only term the share can be reading.
        scene.outer_inner = 0.0;
        scene.outer_outer = 0.0;
        scene.mark_inner = STRIP_IN;
        scene.mark_thickness = STRIP_THICK;
        scene.octave_gap = octave_gap;
        scene.glow_reach = reach;
        scene.glow_strength = 1.5;
        scene.glow_gap = GAP;
        scene.glow_gap_soft = GAP;
        scene.glow_gap_depth = depth;
        scene.nodes[0].gutter = GAP;
        scene.sevens_soft = 0.0;
        scene
    };
    // Half an Octave gap has to outreach the Gap, or the light is held off in
    // the middle of a gap whatever the sides do.
    const {
        assert!(
            0.5 * harmonigraph_scene::GAP_MAX > GAP,
            "the widest gap is too narrow for its middle to be clear of the ink",
        )
    };
    let (closed, wide) = standoff_share_rings(
        &mut shooter,
        SIZE,
        &at,
        STRIP_IN + STRIP_THICK,
        0.5 * GAP,
        ANGLES,
    );

    let least = closed.iter().fold(f64::MAX, |m, s| m.min(*s));
    assert!(
        least > 0.3,
        "a closed strip took only {least:.3} of the light somewhere on the probe ring; \
         there is no standoff there for a gap to open",
    );
    let ratio: Vec<f64> = wide.iter().zip(&closed).map(|(w, c)| w / c).collect();
    let emptiest = ratio.iter().fold(f64::MAX, |m, r| m.min(*r));
    let deepest = ratio.iter().fold(f64::MIN, |m, r| m.max(*r));
    assert!(
        emptiest < 0.15,
        "between two marks the widest Octave gap still took {emptiest:.3} of the closed \
         strip's share: the standoff is measuring the slot's wedge rather than the ink \
         drawn in it",
    );
    assert!(
        deepest > 0.85,
        "over a mark the widest Octave gap took only {deepest:.3} of the closed strip's \
         share: the standoff is following something narrower than the mark",
    );
}
