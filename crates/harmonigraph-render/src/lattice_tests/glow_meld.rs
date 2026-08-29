//! How two nodes' overlapping light adds up.

use super::fixtures::*;
use crate::*;

/// Two nodes' halos MELD where they overlap: brighter than either alone, and
/// bounded — screen (`src + dst * (1 - src)`), not a sum.
///
/// The two claims have to be made together, and the second is the one that
/// says which blend is in the pipeline: any blend that adds light at all
/// passes the first, and `a + b` passes it too while blowing a chord's middle
/// out to white. Screen is strictly under the sum wherever both sides are lit,
/// which is the discriminator this measures.
///
/// At the Meld bar's TOP, which is where the fixture leaves it: that bar dials
/// this blend against the max beside it, and what a melded overlap is is this
/// end of it (`the_meld_says_how_much_two_nodes_overlapping_light_adds_up`).
#[test]
fn two_nodes_light_melds_rather_than_summing() {
    const SIZE: [u32; 2] = [256, 256];
    let Some(mut shooter) = Shooter::new(SIZE) else {
        return;
    };
    // The fixture's node, moved off center, and its mirror image — so the pair
    // straddles the origin the camera is pointed at and their halos cross in
    // between. Far enough apart that neither node's own layers reach the other
    // (the rim is well under one uv unit, which is 1.8 world units here).
    let scene_of = |xs: &[f32]| -> Scene {
        let mut scene = single_marked_node(0, 0);
        let node = scene.nodes[0];
        scene.nodes = xs
            .iter()
            .enumerate()
            .map(|(i, x)| {
                let mut n = node;
                n.world_pos = glam::Vec3::new(*x, 0.0, 0.0);
                n.lattice_pos = harmonigraph_core::LatticePos::new(i as i32, 0, 0);
                n
            })
            .collect();
        rows_per_node(&mut scene);
        scene.glow_reach = 0.8;
        // High on its bar, so the overlap is a reading with room in it rather
        // than three quantization steps: the claim is about the SHAPE of the
        // blend, and at four levels of grey any blend passes.
        scene.glow_strength = 2.0;
        scene
    };
    const APART: f32 = 1.8;
    let left = shooter.shot(&scene_of(&[-APART]));
    let right = shooter.shot(&scene_of(&[APART]));
    let both = shooter.shot(&scene_of(&[-APART, APART]));

    // Where the two overlap most: the pixel whose dimmer half is brightest.
    // Found rather than named, so the probe follows the camera instead of a
    // pixel coordinate that a change of fixture would quietly move off the
    // overlap.
    let at = |shot: &[u8], i: usize| -> [u8; 3] { std::array::from_fn(|c| shot[i * 4 + c]) };
    let probe = (0..(SIZE[0] * SIZE[1]) as usize)
        .max_by_key(|&i| {
            let (l, r) = (at(&left, i), at(&right, i));
            brightness(&l).min(brightness(&r))
        })
        .expect("a non-empty frame");
    let (l, r, b) = (at(&left, probe), at(&right, probe), at(&both, probe));
    assert!(
        brightness(&l) > 24 && brightness(&r) > 24,
        "the two halos never overlapped: {l:?} and {r:?} at pixel {probe}",
    );
    assert!(
        brightness(&b) > brightness(&l).max(brightness(&r)),
        "the overlap {b:?} is no brighter than the brighter half ({l:?}, {r:?})",
    );
    for c in 0..3 {
        let (l, r, b) = (i32::from(l[c]), i32::from(r[c]), i32::from(b[c]));
        assert!(b <= 255, "a channel left the range: {b}");
        // Screen is strictly under the sum wherever both sides carry light;
        // the slack is 8-bit rounding on three composited values.
        if l > 25 && r > 25 {
            assert!(b < l + r - 1, "channel {c} summed rather than melded: {b} against {l} + {r}",);
        }
    }
}

/// The Meld bar's claim: it says how much two nodes' overlapping light adds
/// up, and nothing else. At its top an overlap is the screen the light has
/// always been (`two_nodes_light_melds_rather_than_summing`); at its bottom it
/// is exactly as bright as the brighter of the nodes making it.
///
/// Measured at a FLAT feather, which is the setting the bar exists for. A
/// falloff spread across its whole reach is still near its peak halfway to a
/// neighbour, so screening two of them puts more light in the GAP between two
/// nodes than either node has of its own — the count of overlapping nodes
/// becomes the brightest thing on screen, which is the failure the screen was
/// picked over a sum to avoid. Under a max there is no count to read.
///
/// Against the two nodes' own single shots at the SAME pixel rather than
/// against a chosen brightness: what the bar changes is how two lots of light
/// combine, so the reading that says which operator is in the pipeline is the
/// overlap against its own two halves.
#[test]
fn the_meld_says_how_much_two_nodes_overlapping_light_adds_up() {
    const SIZE: [u32; 2] = [256, 256];
    let Some(mut shooter) = Shooter::new(SIZE) else {
        return;
    };
    // The mirrored pair `two_nodes_light_melds_rather_than_summing` measures,
    // at a flat feather: far enough apart that neither node's own layers reach
    // the other and only their light crosses.
    let scene_of = |xs: &[f32], meld: f32| -> Scene {
        let mut scene = single_marked_node(0, 0);
        let node = scene.nodes[0];
        scene.nodes = xs
            .iter()
            .enumerate()
            .map(|(i, x)| {
                let mut n = node;
                n.world_pos = glam::Vec3::new(*x, 0.0, 0.0);
                n.lattice_pos = harmonigraph_core::LatticePos::new(i as i32, 0, 0);
                n
            })
            .collect();
        rows_per_node(&mut scene);
        scene.glow_reach = 0.8;
        scene.glow_strength = 2.0;
        scene.glow_feather = 1.0;
        scene.glow_meld = meld;
        scene
    };
    const APART: f32 = 1.8;
    let left = shooter.shot(&scene_of(&[-APART], 1.0));
    let right = shooter.shot(&scene_of(&[APART], 1.0));
    let melded = shooter.shot(&scene_of(&[-APART, APART], 1.0));
    let brightest = shooter.shot(&scene_of(&[-APART, APART], 0.0));

    let at = |shot: &[u8], i: usize| -> [u8; 3] { std::array::from_fn(|c| shot[i * 4 + c]) };
    // Where the two overlap most: the pixel whose dimmer half is brightest,
    // found rather than named so the probe follows the camera.
    let probe = (0..(SIZE[0] * SIZE[1]) as usize)
        .max_by_key(|&i| {
            let (l, r) = (at(&left, i), at(&right, i));
            brightness(&l).min(brightness(&r))
        })
        .expect("a non-empty frame");
    let (l, r) = (at(&left, probe), at(&right, probe));
    let (m, b) = (at(&melded, probe), at(&brightest, probe));
    assert!(
        brightness(&l) > 24 && brightness(&r) > 24,
        "the two halos never overlapped: {l:?} and {r:?} at pixel {probe}",
    );

    let half = brightness(&l).max(brightness(&r));
    assert!(
        brightness(&m) > half,
        "at a full Meld the overlap {m:?} must be brighter than the brighter half \
         ({l:?}, {r:?})",
    );
    // The bar's bottom: no light added at all. A per-channel max of two
    // premultiplied colours, so a pixel whose two halves differ in hue can come
    // out a shade over either — the slack is that, plus a composite's rounding.
    assert!(
        brightness(&b) <= half + 2,
        "at a Meld of 0 the overlap {b:?} must be no brighter than the brighter half \
         ({l:?}, {r:?})",
    );
    assert!(
        brightness(&b) + 8 < brightness(&m),
        "the Meld moved the overlap by {} of 255, which is no picture to dial",
        brightness(&m) - brightness(&b),
    );
}

/// The Meld's other half, and what makes it a bar about OVERLAP rather than a
/// second Strength: a pixel one node lights on its own is the same at every
/// setting of it.
///
/// Exact equality, not a tolerance. The two blends the bar mixes between agree
/// wherever only one node writes — a screen over nothing and a max against
/// nothing are both the source — so mixing them there returns that source
/// whatever the weight, and any drift at all would mean the pair had stopped
/// agreeing.
#[test]
fn the_meld_leaves_a_node_lighting_a_pixel_alone_untouched() {
    const SIZE: [u32; 2] = [256, 256];
    let Some(mut shooter) = Shooter::new(SIZE) else {
        return;
    };
    let at = |meld: f32| -> Scene {
        let mut scene = single_marked_node(0, 0);
        scene.glow_reach = 0.8;
        scene.glow_strength = 2.0;
        // Flat, so the light fills its whole reach: the widest picture one
        // node's halo has, and the most of a frame this can hold still.
        scene.glow_feather = 1.0;
        scene.glow_meld = meld;
        scene
    };
    let melded = shooter.shot(&at(1.0));
    let brightest = shooter.shot(&at(0.0));
    let differ = melded.chunks(4).zip(brightest.chunks(4)).filter(|(a, b)| a != b).count();
    assert_eq!(differ, 0, "the Meld moved {differ} pixels of a lone node's own light");
    // Non-vacuous: there has to BE light for the equality to be about anything,
    // the fixture's node being the only thing lighting the frame.
    let mut dark = at(1.0);
    dark.glow_reach = 0.0;
    let unlit = shooter.shot(&dark);
    assert!(
        total_light(&melded) > total_light(&unlit),
        "the fixture's node lit nothing, so holding its light still says nothing",
    );
}

/// The Meld reaches what a NODE paints, not just the ground between nodes.
///
/// A node's ink is washed with the light standing at its own pixel rather than
/// bare ground, reading the glow target back through `node_paint` — so it mixes
/// the same pair the composite does, and has to mix it the same way. A node's
/// ink left on the screen while the ground around it took the max is a node
/// sitting on a plateau with a step at its edge, which is a halo drawn round
/// every node: the one failure the light being ONE field under the whole
/// lattice exists to prevent.
///
/// The probe is the brightest pixel of a ONE-node frame — the middle of that
/// node, where its own light is fullest and its ink is what the pass wrote
/// (`the_middle_of_a_node_is_where_its_light_is_fullest`). Bare ground is what
/// `the_meld_says_how_much_two_nodes_overlapping_light_adds_up` measures, and
/// bare ground is written by the composite alone: probing it says nothing about
/// this path. A second node then lights that same pixel from outside, which is
/// what gives the mix two lots of light to combine there.
#[test]
fn the_meld_reaches_the_light_a_node_paints_over_its_own_body() {
    const SIZE: [u32; 2] = [256, 256];
    let Some(mut shooter) = Shooter::new(SIZE) else {
        return;
    };
    // A reach wide enough that each node stands INSIDE its neighbour's light
    // rather than merely touching it at the midpoint: the pixel under test is
    // on a node's own body, so the other node's halo has to reach that far in
    // for there to be two lots of light to mix there at all.
    let scene_of = |xs: &[f32], meld: f32| -> Scene {
        let mut scene = single_marked_node(0, 0);
        let node = scene.nodes[0];
        scene.nodes = xs
            .iter()
            .enumerate()
            .map(|(i, x)| {
                let mut n = node;
                n.world_pos = glam::Vec3::new(*x, 0.0, 0.0);
                n.lattice_pos = harmonigraph_core::LatticePos::new(i as i32, 0, 0);
                n
            })
            .collect();
        rows_per_node(&mut scene);
        scene.glow_reach = 3.0;
        scene.glow_strength = 1.0;
        scene.glow_feather = 1.0;
        scene.glow_meld = meld;
        scene
    };
    const APART: f32 = 1.8;
    let lone = shooter.shot(&scene_of(&[-APART], 1.0));
    let melded = shooter.shot(&scene_of(&[-APART, APART], 1.0));
    let brightest = shooter.shot(&scene_of(&[-APART, APART], 0.0));

    let at = |shot: &[u8], i: usize| -> [u8; 3] { std::array::from_fn(|c| shot[i * 4 + c]) };
    // The one node's own middle: the brightest pixel of the frame it is alone
    // in. Found rather than named, so the probe follows the camera.
    let probe = (0..(SIZE[0] * SIZE[1]) as usize)
        .max_by_key(|&i| brightness(&at(&lone, i)))
        .expect("a non-empty frame");
    let (l, m, b) = (at(&lone, probe), at(&melded, probe), at(&brightest, probe));
    assert!(brightness(&l) > 24, "the probe {l:?} is not on the node the frame was searched for",);
    // The neighbour has to be lighting this pixel for the mix to have anything
    // to do here: without that, both blends see one contribution and agree.
    assert!(
        brightness(&m) > brightness(&l),
        "the second node did not reach the first node's own middle: {m:?} against {l:?}",
    );
    assert!(
        brightness(&b) < brightness(&m),
        "the Meld did not reach what the node paints: its middle is {b:?} at 0 and {m:?} at \
         1, so this pixel took the screen either way while the ground around it did not",
    );
}

/// The Meld reaches what a RESTING MARKER paints, the way it reaches a node's
/// own body: a cross standing where two halos overlap wears the mix the ground
/// beside it wears, at every setting of the bar.
///
/// A marker reads the field back through `plus_paint` and has to mix the same
/// pair the composite does — the one reader of the field
/// `the_meld_reaches_the_light_a_node_paints_over_its_own_body` does not cover.
/// A cross left on the screen while the ground took the max is, below a Meld of
/// 1, a bright cross-shaped patch on every marker two halos reach: the
/// halo-round-every-node failure, drawn round every cross instead.
///
/// ONE node cannot tell. The two blends the bar mixes agree wherever only one
/// node writes (`the_meld_leaves_a_node_lighting_a_pixel_alone_untouched`), so
/// a marker lit by one node wears the same wash at every setting and
/// `a_resting_marker_wears_the_wash_it_stands_in` is blind to the mix by
/// construction. This needs two halos on the marker's own pixels, and asserts
/// that it has them before it asserts anything else: each node ALONE has to
/// lift every pixel measured, or the cross is not standing in the overlap and
/// the rest passes for the wrong reason.
///
/// "The way the ground does" is exact rather than a direction. A resting cross
/// takes the whole field (`wash_over` at a share of 1), so a pixel it covers in
/// full is `light + ink * (1 - light)`, and moving the Meld moves it by the
/// ground's own move scaled by what the ink leaves, `1 - ink`, channel by
/// channel. Any other reading of the field at the cross — a different mix, a
/// different share — puts a different number there, where the direction alone
/// would let a cross wearing half the Meld through.
#[test]
fn the_meld_reaches_the_light_a_resting_marker_wears() {
    const SIZE: [u32; 2] = [256, 256];
    let Some(mut shooter) = Shooter::new(SIZE) else {
        return;
    };
    // The mirrored pair at the reach and feather
    // `the_meld_reaches_the_light_a_node_paints_over_its_own_body` measures at,
    // and one cross standing ABOVE the pair, clear of both nodes' ink: its
    // nearest point to either node is 2.5 world units out against an outermost
    // ring at 1.57 (`a_resting_marker_wears_the_wash_it_stands_in` has the
    // arithmetic). Orthographic and face-on, so that clearance is a clearance
    // on screen too.
    //
    // A Shadow depth of 0, at which a marker's draw multiplies the frame under
    // it by exactly 1 (`shadowed_ground`): the ground beside the cross is read
    // here as the field itself, and a shadow standing on it would be the
    // marker's own darkening measured as the bar's.
    let scene_of = |xs: &[f32], reach: f32, meld: f32, marker: bool| -> Scene {
        let mut scene = single_marked_node(0, 0);
        let node = scene.nodes[0];
        scene.nodes = xs
            .iter()
            .enumerate()
            .map(|(i, x)| {
                let mut n = node;
                n.world_pos = glam::Vec3::new(*x, 0.0, 0.0);
                n.lattice_pos = harmonigraph_core::LatticePos::new(i as i32, 0, 0);
                n
            })
            .collect();
        rows_per_node(&mut scene);
        scene.camera = harmonigraph_scene::Camera {
            projection: harmonigraph_scene::Projection::Orthographic,
            yaw: 0.0,
            pitch: 0.0,
            ..Default::default()
        };
        scene.glow_reach = reach;
        scene.glow_strength = 1.0;
        scene.glow_feather = 1.0;
        scene.glow_meld = meld;
        scene.glow_shadow_depth = 0.0;
        if marker {
            scene.pluses =
                vec![one_marker(glam::Vec3::new(0.0, 2.6, 0.0), 0.8, scene.lattice_ground, 1.0)];
        }
        scene
    };
    const APART: f32 = 1.8;
    const REACH: f32 = 3.0;
    let pair = [-APART, APART];

    // The marker's own pixels and the ground beside them, read with the glow
    // OFF: no light at the pixel is the one setting that takes every wash out
    // of the picture.
    let bare = shooter.shot(&scene_of(&pair, 0.0, 1.0, false));
    let off = shooter.shot(&scene_of(&pair, 0.0, 1.0, true));
    let marker = fully_inked(&bare, &off);
    assert!(
        marker.len() > 300,
        "the marker covers {} pixels in full that the nodes had not already covered",
        marker.len(),
    );
    // BESIDE: inside the cross's own bounding box, which its arms leave four
    // corners of, and touched by neither the nodes nor the marker — the ground
    // the cross stands in, and the set each of its pixels is paired with below.
    let w = SIZE[0] as usize;
    let (xs, ys): (Vec<usize>, Vec<usize>) =
        marker.iter().map(|&i| ((i / 4) % w, (i / 4) / w)).unzip();
    let (x0, x1) = (*xs.iter().min().unwrap(), *xs.iter().max().unwrap());
    let (y0, y1) = (*ys.iter().min().unwrap(), *ys.iter().max().unwrap());
    let ground: Vec<usize> = (y0..=y1)
        .flat_map(|y| (x0..=x1).map(move |x| (y * w + x) * 4))
        .filter(|&i| bare[i..i + 4] == [0u8, 0, 0, 255] && off[i..i + 4] == bare[i..i + 4])
        .collect();
    assert!(
        ground.len() > 300,
        "the cross leaves {} pixels of bare ground beside it",
        ground.len()
    );

    let left = shooter.shot(&scene_of(&[-APART], REACH, 1.0, true));
    let right = shooter.shot(&scene_of(&[APART], REACH, 1.0, true));
    let melded = shooter.shot(&scene_of(&pair, REACH, 1.0, true));
    let brightest = shooter.shot(&scene_of(&pair, REACH, 0.0, true));

    // The fixture's reach: each halo on its own lifts EVERY pixel the claims
    // below are made on. Where only one of them reaches, both blends see one
    // contribution and agree, and a pixel there says nothing about the mix.
    let lifted_by = |shot: &[u8]| {
        marker.iter().filter(|&&i| brightness(&shot[i..i + 3]) > brightness(&off[i..i + 3])).count()
    };
    for (name, shot) in [("left", &left), ("right", &right)] {
        assert_eq!(
            lifted_by(shot),
            marker.len(),
            "the {name} node's halo alone reaches {} of the marker's {} pixels: the cross is \
             not standing where both halos do",
            lifted_by(shot),
            marker.len(),
        );
    }

    // What the Meld moves a pixel by, per channel, from the bar's bottom to its
    // top.
    let moved = |i: usize, c: usize| i32::from(melded[i + c]) - i32::from(brightest[i + c]);
    // BESIDE, literally: the ground pixel nearest each pixel of the cross. The
    // field is smooth rather than flat — a halo at a feather of 1 still slopes
    // by a few code values across the box — so the ground read for a pixel is
    // the ground under half an arm's width of it, where the slope is under a
    // code value, and not the box's average.
    let xy = |i: usize| ((i / 4) % w, (i / 4) / w);
    let beside = |i: usize| -> usize {
        let (x, y) = xy(i);
        *ground
            .iter()
            .min_by_key(|&&g| {
                let (gx, gy) = xy(g);
                gx.abs_diff(x).pow(2) + gy.abs_diff(y).pow(2)
            })
            .unwrap()
    };
    let ground_moved: Vec<[i32; 3]> =
        marker.iter().map(|&i| std::array::from_fn(|c| moved(beside(i), c))).collect();
    let least = ground_moved.iter().map(|m| m.iter().sum::<i32>()).min().unwrap();
    assert!(
        least > 8,
        "the Meld moved the ground beside the marker by as little as {least} of 255, which is \
         no picture to dial",
    );

    // The cross moves with the bar at all: at a Meld of 0 it is dimmer than at
    // 1, on every pixel, as the ground beside it is.
    let stood_still = marker
        .iter()
        .filter(|&&i| brightness(&melded[i..i + 3]) <= brightness(&brightest[i..i + 3]))
        .count();
    assert_eq!(
        stood_still,
        0,
        "the Meld did not reach what the marker paints: {stood_still} of its {} pixels are no \
         dimmer at 0 than at 1, so the cross took one blend either way while the ground beside \
         it moved",
        marker.len(),
    );
    // And by the ground's move through the wash: the light's share of a
    // fully-covered pixel is what the ink leaves of it. Three code values of
    // slack: each move is the difference of two rounded readings, the two
    // moves round different quantities so their errors do not cancel, and the
    // field slopes a little between a pixel and the ground beside it. The mix
    // being wrong by any share of the bar is tens.
    for (&i, g) in marker.iter().zip(&ground_moved) {
        for c in 0..3 {
            let ink = i32::from(off[i + c]);
            let want = (g[c] * (255 - ink) + 127) / 255;
            let got = moved(i, c);
            assert!(
                (got - want).abs() <= 3,
                "the cross reads a different mix from the ground it stands in: at pixel {:?} \
                 channel {c} the Meld moved it by {got}, where the ground beside it moved by {} \
                 and over ink {ink} that is {want} through the wash",
                xy(i),
                g[c],
            );
        }
    }
}
