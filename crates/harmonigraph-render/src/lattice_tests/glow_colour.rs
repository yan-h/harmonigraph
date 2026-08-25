//! Which colour a node's light takes, and which layer it takes it from.

use super::fixtures::*;
use crate::gpu_harness::headless_device;
use crate::*;

/// A ring WEARS THE WASH inside a pool the Shadow depth has cleared to the bare
/// ground: the two are one field asked for twice, and the answers are free of
/// each other.
///
/// The look the bar exists for. On one coupled dial a dark pool and a tinted
/// ring are mutually exclusive — the light the ink would wear is exactly the
/// light the standoff takes — so the first two claims below are measured at a
/// DEPTH OF 1, where the ground around the ring is the frame with no glow in it
/// at all and there is nothing left of the pool's light to tint anything with.
///
/// Three claims, and the third is the decoupling itself:
///
/// - A wash of 0 leaves the ink byte for byte what it is with the glow off,
///   whatever light is standing at it. Byte-identical rather than nearly so
///   because a factor of 0 on the light is no light: nothing is left to round.
/// - A wash of 1 lifts it, and lifts more than half of it.
/// - Moving the DEPTH moves the ink not at all, the wash reading the field
///   before the standoff's factor reaches it. A wash carried on the standoff's
///   remainder instead cannot say this at all, and that is the reason there is
///   a second bar.
///
/// Every lift is measured as a lift and never a loss, which is the wash's own
/// arithmetic (`node_paint`): the ink takes the light as a screen, so every
/// channel it moves it moves up.
///
/// [`the_shadow_depth_says_how_much_light_a_ring_stands_off`]'s fixture, whose
/// probe is the other side of this boundary — that pixel is outside the node's
/// ink and these are inside it, and neither bar answers for both.
///
/// The ink is found on the GROUND, as in
/// [`a_node_under_a_nearer_sheets_node_cuts_nothing_out_of_its_light`]: a pixel
/// the node paints opaquely is the pixel that does not move when the ground
/// does.
///
/// That set is not exact, and the third claim is stated to survive it: a pixel
/// the node paints at an alpha a hair under 1 carries a SUB-LSB sliver of
/// ground, which a black-and-white probe rounds away and the depth still moves.
/// The BOUND follows from how the set is chosen rather than from tuning —
/// agreeing over both grounds forces that sliver's coefficient under 1/255, and
/// the sliver is the only term the depth touches on such a pixel, so one byte
/// is the most it can carry. A wash reading the standoff's remainder would move
/// the ink by the light's own size instead, which is the scale the shot beside
/// it supplies.
#[test]
fn a_ring_wears_the_wash_inside_its_own_dark_pool() {
    const SIZE: [u32; 2] = [256, 256];
    let Some(mut shooter) = Shooter::new(SIZE) else {
        return;
    };
    let at = |reach: f32, depth: f32, wash: f32| -> Scene {
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
        // The fade the whole width of the shadow, which is the fresh pair.
        scene.glow_shadow_soft = 0.16;
        scene.glow_shadow_depth = depth;
        scene.glow_wash = wash;
        scene.glow_shadow = 0.16;
        scene
    };
    let mut on_ground = |bg: glam::Vec4| {
        let mut scene = at(0.0, 1.0, 0.0);
        scene.background = bg;
        shooter.shot(&scene)
    };
    let dark_ground = on_ground(glam::Vec4::new(0.0, 0.0, 0.0, 1.0));
    let pale_ground = on_ground(glam::Vec4::ONE);
    let ink: Vec<usize> = (0..pale_ground.len())
        .step_by(4)
        .filter(|&i| {
            pale_ground[i..i + 4] != [0u8, 0, 0, 255]
                && pale_ground[i..i + 4] == dark_ground[i..i + 4]
        })
        .collect();
    assert!(ink.len() > 500, "the node painted {} opaque pixels", ink.len());

    let off = shooter.shot(&at(0.0, 1.0, 1.0));
    let dry = shooter.shot(&at(0.8, 1.0, 0.0));
    let worn = shooter.shot(&at(0.8, 1.0, 1.0));
    let open = shooter.shot(&at(0.8, 0.0, 1.0));
    let moved = ink.iter().filter(|&&i| dry[i..i + 4] != off[i..i + 4]).count();
    assert_eq!(
        moved,
        0,
        "with no wash the glow reached {moved} of the ring's {} opaque pixels",
        ink.len(),
    );
    let lifted =
        ink.iter().filter(|&&i| brightness(&worn[i..i + 3]) > brightness(&off[i..i + 3])).count();
    assert!(
        lifted * 2 > ink.len(),
        "inside a pool cleared to the bare ground, a full wash lifted {lifted} of the ring's {} \
         opaque pixels",
        ink.len(),
    );
    let dimmed = ink.iter().filter(|&&i| (0..3).any(|c| worn[i + c] < off[i + c])).count();
    assert_eq!(
        dimmed,
        0,
        "the wash took light off {dimmed} of the ring's {} opaque pixels",
        ink.len(),
    );
    // The furthest any one channel of the ink moves between two shots, which is
    // what both halves of the last claim are read in.
    let spread = |a: &[u8], b: &[u8]| {
        ink.iter()
            .map(|&i| (0..3).map(|c| a[i + c].abs_diff(b[i + c])).max().unwrap())
            .max()
            .unwrap()
    };
    let by_wash = spread(&worn, &dry);
    let by_depth = spread(&worn, &open);
    assert!(
        by_wash > 20,
        "the fixture's wash moves the ink by {by_wash}; there is nothing here to be free of",
    );
    assert!(
        by_depth <= 1,
        "dropping the depth moved the ink by {by_depth} against the wash's own {by_wash}: the \
         wash is reading the standoff's remainder rather than the field",
    );
}

/// A node wearing NOTHING BUT AN AUDIO RING gives off no light at all,
/// whatever its ring is reading.
///
/// A halo says something is being PLAYED at this position; the analyzer's ring
/// says something is being heard in the room. One is the node's own voice and
/// the other a reading it wears, so the ring is drawn and never shone — it is
/// the one layer left out of both halves of the light, the level
/// (`panes::glow_fade` in harmonigraph-ui) and the colour (`ink_at`).
///
/// Byte-identical with the Reach off, which is the whole claim: the light's
/// draw runs over this node, writes nothing into its target, and the composite
/// lays exactly nothing over the picture. Shot at a loud partial as well as at
/// silence, because those are the two ways a ring could have held a light on —
/// what is IN it, and merely that it is showing.
///
/// Non-vacuity is what the rest of the fixture is for: the ring has to be on
/// screen, and its silent end has to be a grey the eye can see, or "no light"
/// would be a claim about a frame with no ring in it.
#[test]
fn a_node_wearing_only_an_audio_ring_gives_off_no_light() {
    const SIZE: [u32; 2] = [256, 256];
    let Some(mut shooter) = Shooter::new(SIZE) else {
        return;
    };
    // [`ringing_node`] with no key down and no octave sounding: the band draws
    // nothing at all and the node's whole picture is the analyzer's.
    let at = |sounding: Option<f32>, reach: f32| -> Scene {
        let mut scene = ringing_node(None, sounding, PROBE_RANGE);
        // The app's ramp rather than the fixture's: its silent end PINNED to
        // the ground, which is what makes an empty wedge a grey the eye reads
        // as a ring rather than the black the probe's own ramp starts at
        // (`harmonigraph_scene::ring_gradient`).
        let ground = scene.lattice_ground;
        scene.spectral.lut = std::array::from_fn(|k| {
            let t = k as f32 / (harmonigraph_scene::PITCH_LUT_N - 1) as f32;
            ground.lerp(glam::Vec4::ONE, t)
        });
        let node = &mut scene.nodes[0];
        // Silence at the node, and the analyzer still reading: no key down, no
        // octave sounding, no mark at either end — and the ring at full, which
        // is the view's Gate answered for this node.
        node.activation = 0.0;
        node.melody_level = 0.0;
        node.bass_level = 0.0;
        node.melody_slots = 0;
        node.bass_slots = 0;
        node.audio_ring = 1.0;
        scene.glow_reach = reach;
        scene.glow_strength = 1.5;
        scene
    };
    const PARTIAL: f32 = harmonigraph_scene::MIDDLE_C_SLOT as f32 * 12.0;

    // Non-vacuous: the ring is ON SCREEN. Against the layer's own off switch
    // rather than against `total_light`, which the node's clearing alone would
    // satisfy — and safe as a one-layer diff only because this node draws
    // nothing else: no key, no octave, no mark, so there is no layer outside
    // the ring for its width to slide inward (the stack packs outward from the
    // centre).
    let loud_off = shooter.shot(&at(Some(PARTIAL), 0.0));
    let mut ringless = at(Some(PARTIAL), 0.0);
    (ringless.spectral.inner, ringless.spectral.outer) = (0.0, 0.0);
    assert!(
        loud_off != shooter.shot(&ringless),
        "the fixture drew no audio ring, so it cannot say a ring gives off nothing",
    );

    for (what, sounding) in [("a partial", Some(PARTIAL)), ("silence", None)] {
        let off = shooter.shot(&at(sounding, 0.0));
        let on = shooter.shot(&at(sounding, 0.8));
        assert!(
            on == off,
            "a ring reading {what} lit {} against {} with the glow off",
            total_light(&on),
            total_light(&off),
        );
    }
}

/// One node lit in TWO colours no mixing of the other's table can reach: the
/// pitch ramp flat RED, so every slice of the octave band is red however its
/// octave is voiced, and the melody mark flat GREEN, so the strip past the
/// rings is green wherever it is worn.
///
/// The suite's usual ramps — a blue-to-red pitch sweep and the marks' own two
/// hues — overlap in every channel, so a halo drawn out of both would answer
/// "somewhere between" and say nothing about which layer coloured it. With one
/// channel apiece the halo's red against its green IS the two layers' share of
/// it, which is what every claim below reads.
///
/// The band and the marks because they are the two LIT layers a node has: the
/// audio ring is drawn and never shone (`ink_at`), so a fixture that reached
/// for it would be asking which layer coloured a halo with a layer that colours
/// none. It is left off here rather than left dark, so nothing in these shots
/// stands light off that the claims are not about.
///
/// Every octave sounds, so the band is one red ring rather than a lit slice
/// among ghosts, and the marks name every octave, so the strip is one green
/// ring rather than a wedge.
fn two_colour_node(band_width: f32, mark_width: f32) -> Scene {
    let fresh = harmonigraph_scene::ViewConfig::default();
    let mut scene = single_marked_node(0, 0);
    // The probe's wide padding: the angular gap is a constant chord, so a layer
    // packed against the node's centre has its sectors eaten by that gap and
    // paints almost nothing — which would make that layer's share of the light
    // a reading of the padding rather than of its width.
    let rings = harmonigraph_scene::ViewConfig {
        ring_inner: PROBE_INNER,
        ring_gap: PROBE_GAP,
        spectral_ring_width: 0.0,
        band_width,
        mark_thickness: mark_width,
        ..fresh.clone()
    }
    .rings();
    scene.outer_inner = rings.band.0;
    scene.outer_outer = rings.band.1;
    scene.rings_outer = rings.outer;
    scene.mark_inner = rings.mark_inner;
    scene.mark_thickness = rings.mark_thickness;
    // A NARROW angular gap, where the radial one is the probe's wide one: the
    // sector gap is a constant Euclidean chord, so at the radii the innermost
    // ring occupies — it reaches the node's centre — the probe's own 0.12 would
    // blank that layer's sectors outright and the light would carry none of its
    // colour.
    scene.octave_gap = 0.03;
    scene.pitch_lut = [glam::Vec4::new(1.0, 0.0, 0.0, 1.0); harmonigraph_scene::PITCH_LUT_N];

    let node = &mut scene.nodes[0];
    node.octaves = [1.0; harmonigraph_scene::OCTAVE_SLOTS];
    node.activation = 1.0;
    // FLAT green rather than the fixture's two hues, so the strip's colour is
    // the strip's colour wherever it is read: what is being measured here is
    // which layer the light took its hue from.
    node.melody_color = glam::Vec4::new(0.0, 1.0, 0.0, 1.0);
    // Every slot named, so the strip closes into a ring — the shader draws a
    // mark only on the slots the wheel is showing, so the extras cost nothing.
    node.melody_slots = u32::MAX;
    node.melody_level = 1.0;
    node.bass_slots = 0;
    node.bass_level = 0.0;
    // No analyzer at all: `parity_scene` is silent, and an empty annulus is the
    // ring layer's own off switch.
    node.audio_ring = 1.0;

    scene.glow_reach = 0.8;
    scene.glow_strength = 1.5;
    scene
}

/// The light one shot ADDED over the same frame with the glow off, summed per
/// channel over the whole frame.
///
/// Everything the node itself draws stands in both shots and cancels, so what
/// is left is the halo's own colour — which is what a claim about where the
/// light took its hue from has to read. Clamped at 0 per channel because the
/// glow also takes the core's skirt away, and a channel that went DOWN is that
/// subtraction rather than any hue.
fn added_light(on: &[u8], off: &[u8]) -> [i64; 3] {
    let mut sum = [0i64; 3];
    for (a, b) in on.chunks(4).zip(off.chunks(4)) {
        for (c, s) in sum.iter_mut().enumerate() {
            *s += (i64::from(a[c]) - i64::from(b[c])).max(0);
        }
    }
    sum
}

/// The glow's colour is the node's own INK, whatever layer laid it down.
///
/// Three nodes, one code path: a node wearing nothing but its audio ring lights
/// in the ring's colour, a node wearing nothing but its octave band lights in
/// the band's, and a node wearing both lights in a mixture that is greener than
/// the one and redder than the other. Nothing here names a layer — the light is
/// `ink_at` read round the node — so the ring's hue reaching the halo and the
/// band's reaching it are the same mechanism, and a layer added to a node is
/// lit without a line of its own.
///
/// See [`two_colour_node`] for why the two ramps are one channel apiece.
#[test]
fn a_nodes_light_takes_the_colour_of_whichever_layer_is_drawing() {
    const SIZE: [u32; 2] = [256, 256];
    const BAND: f32 = 0.16;
    const RING: f32 = 0.16;
    let Some(mut shooter) = Shooter::new(SIZE) else {
        return;
    };
    let dark = |mut scene: Scene| -> Scene {
        scene.glow_reach = 0.0;
        scene
    };
    // The ring alone: no key down and no octave sounding, so the band draws
    // nothing at all and the node's whole picture is the analyzer's.
    let ring_only = || -> Scene {
        let mut scene = two_colour_node(BAND, RING);
        let node = &mut scene.nodes[0];
        node.activation = 0.0;
        node.octaves = [0.0; harmonigraph_scene::OCTAVE_SLOTS];
        scene
    };
    // The band alone: the ring's width dialled to nothing, which is the layer's
    // own off switch.
    let band_only = || two_colour_node(BAND, 0.0);

    let ring = added_light(&shooter.shot(&ring_only()), &shooter.shot(&dark(ring_only())));
    let band = added_light(&shooter.shot(&band_only()), &shooter.shot(&dark(band_only())));
    let both = added_light(
        &shooter.shot(&two_colour_node(BAND, RING)),
        &shooter.shot(&dark(two_colour_node(BAND, RING))),
    );

    assert!(ring[1] > 0 && band[0] > 0, "neither fixture lit anything: {ring:?}, {band:?}");
    assert!(
        ring[1] > ring[0] * 4,
        "a node wearing only its audio ring lit {ring:?} — its light is not the ring's green",
    );
    assert!(
        band[0] > band[1] * 4,
        "a node wearing only its octave band lit {band:?} — its light is not the band's red",
    );
    // And the two together are a mixture rather than either one winning: the
    // SHARE is what moves, the three halos not being the same size.
    let share = |c: [i64; 3]| c[0] as f64 / (c[0] + c[1]).max(1) as f64;
    assert!(
        share(ring) < share(both) && share(both) < share(band),
        "a node wearing both lit {both:?}, which is not between {ring:?} and {band:?}",
    );
}

/// A slice the node is not sounding puts NO ground in its light.
///
/// A note voiced in a single octave lights one slice of the octave band and
/// leaves the rest of the wheel ghosts — eight of them at the fresh view's
/// span of nine, four at this fixture's five — and a ghost is
/// `Scene::lattice_ground` flat and opaque. Weighing the band by its INK
/// therefore hands that note a halo that is mostly grey, with its own pitch a
/// lobe inside it. The light weighs each slice by its LEVEL instead (`ink_at`,
/// through `oct_slot_lit`), so the halo is the octave's own colour and the
/// ghosts are a thing drawn rather than a thing shining.
///
/// The ground is set to pure BLUE here, which is a colour the view cannot
/// actually hold — `grey_of_lightness` is what fills that field in the app —
/// and that is the point: the pitch ramp is flat red, so a blue channel in the
/// halo can only have come from the ghosts, and no mixture of the one is
/// reachable from the other. The ghosts themselves are checked to be on
/// screen, or the claim is being made about a node that has none.
#[test]
fn a_silent_slice_puts_none_of_its_ground_in_the_light() {
    const SIZE: [u32; 2] = [256, 256];
    const BAND: f32 = 0.18;
    let Some(mut shooter) = Shooter::new(SIZE) else {
        return;
    };
    // The band alone — the ring dialled to nothing — voiced in ONE octave, so
    // every other slice on the wheel is a ghost.
    let one_octave = || -> Scene {
        let mut scene = two_colour_node(BAND, 0.0);
        scene.lattice_ground = glam::Vec4::new(0.0, 0.0, 1.0, 1.0);
        let node = &mut scene.nodes[0];
        node.octaves = [0.0; harmonigraph_scene::OCTAVE_SLOTS];
        node.octaves[harmonigraph_scene::MIDDLE_C_SLOT] = 1.0;
        scene
    };
    let dark = || -> Scene {
        let mut scene = one_octave();
        scene.glow_reach = 0.0;
        scene
    };
    let off = shooter.shot(&dark());
    // Non-vacuous: the ghosts have to be ON the node, or "the light carries
    // none of them" is a claim about a ring that is not there.
    let ghosts: i64 = off.chunks(4).map(|px| i64::from(px[2])).sum();
    let red: i64 = off.chunks(4).map(|px| i64::from(px[0])).sum();
    assert!(ghosts > red, "the fixture drew no blue ghosts: {ghosts} against {red} of red");

    let lit = added_light(&shooter.shot(&one_octave()), &off);
    assert!(lit[0] > 0, "the node's one lit octave lit nothing: {lit:?}");
    // Four slices of five are the ground here, so weighing the band by its ink
    // puts four times as much of that grey in the halo as of the pitch, and a
    // factor of eight is well clear of either reading.
    assert!(
        lit[0] > lit[2] * 8,
        "a note voiced in one octave lit {lit:?} — the halo is carrying its ghosts",
    );
}

/// A slice part way through its envelope carries that much of the light, and
/// no more.
///
/// The one place the weight is neither what it was nor zero, and the reason
/// the drawn ink cannot be asked for it: a slice's OPACITY is the node's
/// presence, with the ghost filling in whatever its own level does not
/// account for. So a pitch class held in one octave while another octave
/// releases draws both slices fully opaque, and weighing the band by its ink
/// hands the releasing one a full share of the light for the whole of its
/// release — in a colour that is itself part ghost by then.
///
/// THREE channels, one per thing that could be in the halo: the ground pure
/// green, and a pitch ramp that is pure blue under the two slices' midpoint
/// and pure red over it, so the held slice is red, the releasing one blue, and
/// any ghost that reaches the light is green. None of the three is reachable
/// from the others.
///
/// Green is the discriminator — a ghost weighs nothing at any level, so the
/// halo has none of it while the slice is half out. The blue share falling
/// with the slice's own level is the positive claim, over three levels rather
/// than two so that it is the ENVELOPE being followed and not a switch at 0.
#[test]
fn a_slice_part_way_out_carries_that_much_of_the_light() {
    const SIZE: [u32; 2] = [256, 256];
    let Some(mut shooter) = Shooter::new(SIZE) else {
        return;
    };
    let beside = slot_beside_middle_c().trailing_zeros() as usize;
    let lit = harmonigraph_scene::MIDDLE_C_SLOT;
    assert_ne!(beside, lit, "the two slices have to be different slices");
    // The node fully PRESENT throughout — that is what makes both slices
    // opaque and the ink weight blind to which of them is sounding.
    let at = |releasing: f32, reach: f32| -> Scene {
        let mut scene = single_marked_node(0, 0);
        scene.lattice_ground = glam::Vec4::new(0.0, 1.0, 0.0, 1.0);
        // Split at the two slices' midpoint: one octave either side of it, so
        // each wears one end of the ramp whole and neither is a blend.
        let (dark, bright) = (scene.darkest_pitch, scene.brightest_pitch);
        let mid = (harmonigraph_scene::MIDDLE_C_SLOT + beside) as f32 * 6.0;
        let split = (mid - dark) / (bright - dark);
        scene.pitch_lut = std::array::from_fn(|k| {
            let t = k as f32 / (harmonigraph_scene::PITCH_LUT_N - 1) as f32;
            if t < split {
                glam::Vec4::new(0.0, 0.0, 1.0, 1.0)
            } else {
                glam::Vec4::new(1.0, 0.0, 0.0, 1.0)
            }
        });
        let node = &mut scene.nodes[0];
        node.activation = 1.0;
        node.octaves = [0.0; harmonigraph_scene::OCTAVE_SLOTS];
        node.octaves[lit] = 1.0;
        node.octaves[beside] = releasing;
        scene.glow_reach = reach;
        scene.glow_strength = 1.5;
        scene
    };
    // The two slices sit either side of the split, so which of them is red and
    // which blue follows from their order rather than being assumed.
    let (held_ch, going_ch) = if beside > lit { (2, 0) } else { (0, 2) };
    let mut halo = |releasing: f32| -> [i64; 3] {
        added_light(&shooter.shot(&at(releasing, 0.8)), &shooter.shot(&at(releasing, 0.0)))
    };
    let (held, half, gone) = (halo(1.0), halo(0.5), halo(0.0));
    eprintln!("halo r/g/b — releasing 1.0 {held:?}, 0.5 {half:?}, 0.0 {gone:?}");
    // Non-vacuous: the two slices do light the halo in their own colours.
    assert!(
        held[held_ch] > 0 && held[going_ch] > 0,
        "both slices sounding lit {held:?}, which is not two colours",
    );
    // The ghost never reaches the light — not while the slice is HALF out,
    // which is where the drawn ink is half ground and fully weighed.
    for (what, c) in [("both sounding", held), ("half out", half), ("gone", gone)] {
        let colour = c[0] + c[2];
        assert!(
            c[1] * 8 < colour,
            "with the slice {what} the halo carried {} of ground against {colour} of pitch: {c:?}",
            c[1],
        );
    }
    // And the share follows the envelope.
    let share = |c: [i64; 3]| c[going_ch] as f64 / (c[0] + c[2]).max(1) as f64;
    let (a, b, d) = (share(held), share(half), share(gone));
    assert!(
        a > b && b > d,
        "the light did not follow the releasing slice's level: {a:.4} / {b:.4} / {d:.4}",
    );
}

/// How much of the light a layer's colour owns is how much of the NODE that
/// layer occupies: the same node with its octave band twice as wide glows
/// redder — the band's colour — than it does at the narrower width.
///
/// No knob of its own. The weight in `ink_at` is the radial width the ring
/// stack handed the layer, so this follows the Layers bar directly: widen a
/// ring and its colour takes more of the halo, dial it to nothing and it takes
/// none.
///
/// The audio ring is held at one width and sits INSIDE the band, so widening
/// the band leaves the ring's own radii exactly where they were — what changes
/// is the share, not the other layer.
#[test]
fn widening_a_layer_gives_its_colour_more_of_the_light() {
    const SIZE: [u32; 2] = [256, 256];
    const RING: f32 = 0.16;
    let Some(mut shooter) = Shooter::new(SIZE) else {
        return;
    };
    let at = |band: f32| -> Scene { two_colour_node(band, RING) };
    let dark = |band: f32| -> Scene {
        let mut scene = at(band);
        scene.glow_reach = 0.0;
        scene
    };
    let narrow = added_light(&shooter.shot(&at(0.11)), &shooter.shot(&dark(0.11)));
    let wide = added_light(&shooter.shot(&at(0.22)), &shooter.shot(&dark(0.22)));
    // Both layers are drawing in both shots, or "the share moved" is one of
    // them arriving rather than the widths being read.
    for (what, c) in [("narrow", narrow), ("wide", wide)] {
        assert!(c[0] > 0 && c[1] > 0, "the {what} shot lit {c:?} — one layer is missing");
    }
    let share = |c: [i64; 3]| c[0] as f64 / (c[0] + c[1]).max(1) as f64;
    assert!(
        share(wide) > share(narrow) + 0.05,
        "doubling the band's width moved the light's red share from {:.3} to {:.3}",
        share(narrow),
        share(wide),
    );
}

/// The light carries no ripple the ink it is read from cannot hold.
///
/// This is the artefact the strip exists to remove, and it has a name: the
/// colour used to be averaged at twelve FIXED angles per fragment, so anything
/// the ink held near that rate came through the average intact, and every node
/// wore a fan of dark spokes converging on its middle. Nothing in the picture
/// has twelve-fold symmetry — the wheel is cut into at most eleven — so the
/// ripple could only be the sampling.
///
/// Measured as ANGULAR HARMONICS of the light's brightness round a circle,
/// because that is what a spoke is: a ripple that goes round the node a whole
/// number of times. The band under test starts above what a blurred ink can
/// hold — the tightest lobe the Color blend bar reaches is GLOW_LOBE_KAPPA, whose
/// von Mises coefficients are already under a thousandth of the mean by the
/// eighth harmonic — so anything found there is the machinery and not the node.
/// At fbc6cd5 the twelfth carried 12% to 17% of the mean at every radius inside
/// the node; the bound below is a quarter of that.
///
/// A node lit by its OCTAVE BAND ALONE, voiced every other octave, which is the
/// sharpest angular structure the light reads: a slice per octave, alternately
/// the pitch's own colour and a ghost that weighs nothing (`ink_at`), with the
/// Octave gap cut between them. The Spread is at the bottom of its bar, where
/// the blur is tightest and a sampled one has the least room to hide.
#[test]
fn a_nodes_light_has_no_ripple_the_ink_does_not() {
    const SIZE: [u32; 2] = [512, 512];
    // Where to read, as multiples of where the node's own ink ends: out past
    // every layer it draws, where the light is all there is and a spoke has
    // nothing to hide behind, and inside the Reach, which carries the light
    // rather further than the furthest of these.
    const PAST: [f32; 5] = [1.15, 1.3, 1.45, 1.6, 1.75];
    let Some(mut shooter) = Shooter::new(SIZE) else {
        return;
    };
    let at = |reach: f32| -> Scene {
        // The band the node's only layer: no analyzer at all, no mark at either
        // end, so the only ink on the node is the band's and the only light is
        // the glow's.
        let mut scene = two_colour_node(PROBE_BAND_WIDTH, 0.0);
        // Alternating, so neighbouring slices differ as far as the ramp allows.
        // The pitch table is flat, so what varies round the turn is which
        // slices carry light at all.
        let node = &mut scene.nodes[0];
        node.octaves = std::array::from_fn(|i| f32::from(i % 2 == 0));
        scene.glow_reach = reach;
        scene.glow_strength = 1.5;
        scene.glow_blend = 0.0;
        // Big enough that a circle inside the node is hundreds of pixels round,
        // which is what resolving a ripple at these rates takes.
        scene.node_radius = 1.6;
        scene
    };
    let off = shooter.shot(&at(0.0));
    let on = shooter.shot(&at(0.8));

    // Where the node's ink ends, measured rather than assumed: the radii below
    // are shares of it, so a retune of the probe stack moves the probes with
    // the picture instead of dropping them onto the node. The furthest inked
    // pixel ANYWHERE rather than along one ray — half the slices here are
    // ghosts drawn in the ground, so a ray that lands on one finds the node's
    // edge nowhere near where it is.
    let row = SIZE[0] as usize;
    let (cx, cy) = (SIZE[0] as f32 / 2.0, SIZE[1] as f32 / 2.0);
    let bare: [u8; 4] = off[0..4].try_into().expect("a frame has a first pixel");
    let edge = (0..off.len() / 4)
        .filter(|&i| off[i * 4..i * 4 + 4] != bare)
        .map(|i| {
            let (x, y) = ((i % row) as f32 + 0.5, (i / row) as f32 + 0.5);
            (x - cx).hypot(y - cy)
        })
        .fold(0.0f32, f32::max);
    assert!(edge > 10.0, "the node inked only {edge}px of radius; there is nothing to read");

    for radius in PAST.map(|f| f * edge) {
        let lit = ring_profile(&on, SIZE, radius);
        let dark = ring_profile(&off, SIZE, radius);
        let mean = |p: &[f64]| p.iter().sum::<f64>() / p.len() as f64;
        // Non-vacuous first: there has to BE light on this circle, or a dark
        // frame passes every bound below.
        let (bright, unlit) = (mean(&lit), mean(&dark));
        assert!(
            bright > unlit + 4.0,
            "the fixture puts no light at {radius} px: {bright:.1} against {unlit:.1} unlit",
        );
        // The ripple, band by band. Against the mean because that is what a
        // spoke reads as — a dip against the light around it.
        for k in 8..=32 {
            let ripple = harmonic(&lit, k) / bright;
            assert!(
                ripple < 0.03,
                "the light ripples {:.1}% of its own brightness {k} times round the node at \
                 {radius} px — nothing the node draws is cut that fine, so it is the sampling",
                ripple * 100.0,
            );
        }
        // ...and the same thing said the plain way: neighbouring samples round
        // the circle are within a step of each other. Cruder, since 8-bit
        // rounding is most of what is left in it, and here because a spoke is
        // something you SEE rather than a coefficient — at fbc6cd5 this reads
        // 2 to 3 at every radius above.
        let step = lit.windows(2).map(|w| (w[1] - w[0]).abs()).fold(0.0f64, f64::max);
        assert!(
            step < 0.75,
            "the light steps {step:.2}/255 between neighbouring samples round the node at \
             {radius} px",
        );
    }
}

/// The ink strip is as tall as the scene says it is, however that changes —
/// asked across a frame that ADDS a node, on the pane that drew the frame
/// before it.
///
/// A node's row is handed out by the light's own clock and the scene carries
/// the height that goes with it (`Scene::glow_rows`), so a strip left at the
/// previous frame's height is a node writing past the end of the texture and
/// reading zeros back — which looks like a node that has stopped glowing rather
/// than like a bug. The fixture here settles for a row per node, which is what
/// a scene assembled by hand has (`rows_per_node`).
///
/// The same pane through every frame, which is the whole point — a fresh pane
/// allocates a fresh strip and could not be wrong about this. What is under
/// test is the resize, so each frame has to find the last one's.
#[test]
fn the_ink_strip_has_a_row_for_every_node() {
    const SIZE: [u32; 2] = [256, 256];
    let Some((device, queue)) = headless_device() else {
        return;
    };
    let format = wgpu::TextureFormat::Rgba8Unorm;
    let points = egui::vec2(SIZE[0] as f32, SIZE[1] as f32);
    let screen = ScreenDescriptor { size_in_pixels: SIZE, pixels_per_point: 1.0 };
    let mut resources = CallbackResources::default();

    // The fixture's node, copied along a row far enough apart that no node's
    // own layers reach another's.
    let scene_of = |n: usize| -> Scene {
        let mut scene = single_marked_node(0, 0);
        let node = scene.nodes[0];
        scene.nodes = (0..n)
            .map(|i| {
                let mut nd = node;
                nd.world_pos = glam::Vec3::new(i as f32 * 1.8 - 1.8, 0.0, 0.0);
                nd.lattice_pos = harmonigraph_core::LatticePos::new(i as i32, 0, 0);
                nd
            })
            .collect();
        rows_per_node(&mut scene);
        scene.glow_reach = 0.8;
        scene.glow_strength = 1.5;
        scene
    };
    let frame = |resources: &mut CallbackResources, n: usize| -> (u32, u32) {
        let cb = LatticeCallback::from_scene(
            &scene_of(n),
            LatticeLabels::default(),
            points,
            format,
            9,
            None,
        );
        let mut encoder = device.create_command_encoder(&Default::default());
        let bufs = cb.prepare(&device, &queue, &screen, &mut encoder, resources);
        queue.submit(bufs.into_iter().chain([encoder.finish()]));
        let pane = resources
            .get::<LatticeResources>()
            .expect("prepare built the resources")
            .panes
            .get(&9)
            .expect("...and this pane's buffers");
        let strip = &pane
            .offscreen
            .as_ref()
            .expect("the pane drew something")
            .glow
            .as_ref()
            .expect("the view asks for a glow")
            .strip;
        (pane.instance_count, strip.rows)
    };
    // Up and back down: a strip that only ever grew would pass a rising
    // sequence while leaving rows no node writes into.
    for n in [2usize, 3, 5, 4] {
        let (instances, rows) = frame(&mut resources, n);
        // The instance count rather than `n`: a node that can paint nothing is
        // dropped before it reaches the buffer, and the strip follows what is
        // IN the buffer.
        assert_eq!(instances, n as u32, "the fixture's {n} nodes must all be drawn");
        assert_eq!(
            rows, instances,
            "a frame of {instances} nodes drew into a strip {rows} rows tall",
        );
    }
}

/// The light of a node ADDED to a pane mid-session is that node's own.
///
/// The behavioural half of [`the_ink_strip_has_a_row_for_every_node`], and the
/// half that says the rows are the right way round: a strip that grew but was
/// read at the wrong offset would still be one row per node.
///
/// Two nodes lit in colours no mixture of the other's could be mistaken for —
/// [`two_colour_node`]'s two flat ramps, the band's red and the mark strip's
/// green. The one already on screen wears the band alone and the one arriving
/// wears the mark alone, so the added node's light is green exactly where the
/// other's is red.
#[test]
fn a_node_added_to_a_pane_lights_in_its_own_colour() {
    const SIZE: [u32; 2] = [256, 256];
    let Some(mut shooter) = Shooter::new(SIZE) else {
        return;
    };
    let scene_of = |arrived: bool| -> Scene {
        let mut scene = two_colour_node(0.16, 0.16);
        let node = scene.nodes[0];
        let band = {
            let mut node = node;
            node.melody_slots = 0;
            node.melody_level = 0.0;
            node.world_pos = glam::Vec3::new(-1.8, 0.0, 0.0);
            node.lattice_pos = harmonigraph_core::LatticePos::new(-1, 0, 0);
            node
        };
        let mark = {
            let mut node = node;
            node.activation = 0.0;
            node.octaves = [0.0; harmonigraph_scene::OCTAVE_SLOTS];
            node.world_pos = glam::Vec3::new(1.8, 0.0, 0.0);
            node.lattice_pos = harmonigraph_core::LatticePos::new(1, 0, 0);
            node
        };
        scene.nodes = if arrived { vec![band, mark] } else { vec![band] };
        rows_per_node(&mut scene);
        scene
    };
    let one = shooter.shot(&scene_of(false));
    let two = shooter.shot(&scene_of(true));

    // What the second frame added, per channel, over the right-hand half of the
    // picture — where the added node is, and where the first frame has nothing.
    let mut added = [0i64; 3];
    let row = SIZE[0] as usize;
    for i in 0..(SIZE[0] * SIZE[1]) as usize {
        if i % row < row / 2 {
            continue;
        }
        for (c, sum) in added.iter_mut().enumerate() {
            *sum += (i64::from(two[i * 4 + c]) - i64::from(one[i * 4 + c])).max(0);
        }
    }
    assert!(added[1] > 0, "the added node lit nothing at all: {added:?}");
    assert!(
        added[1] > added[0] * 4,
        "a node wearing only its mark strip lit {added:?} — that is the BAND's red, which is \
         the other node's ink and so the other node's row of the strip",
    );
}

/// One profile of a shot's brightness round a circle of `radius` about the
/// frame's centre, sampled bilinearly so the reading follows the circle rather
/// than the pixel grid.
///
/// The step between samples is well under a pixel, which is what the claims
/// above need: a ripple is measured against the light beside it, and a profile
/// that skipped pixels would read the grid's own steps as one.
fn ring_profile(shot: &[u8], size: [u32; 2], radius: f32) -> Vec<f64> {
    let at = |x: f32, y: f32| -> f64 {
        let (x0, y0) = (x.floor(), y.floor());
        let px = |ix: f32, iy: f32| -> f64 {
            let ix = (ix as i32).clamp(0, size[0] as i32 - 1) as usize;
            let iy = (iy as i32).clamp(0, size[1] as i32 - 1) as usize;
            let i = (iy * size[0] as usize + ix) * 4;
            brightness(&shot[i..i + 3]) as f64 / 3.0
        };
        let (fx, fy) = (f64::from(x - x0), f64::from(y - y0));
        let top = px(x0, y0) + (px(x0 + 1.0, y0) - px(x0, y0)) * fx;
        let bot = px(x0, y0 + 1.0) + (px(x0 + 1.0, y0 + 1.0) - px(x0, y0 + 1.0)) * fx;
        top + (bot - top) * fy
    };
    let (cx, cy) = (size[0] as f32 / 2.0, size[1] as f32 / 2.0);
    // Four samples per pixel of circumference, and never so few that a whole
    // turn is under a reading.
    let n = ((radius * std::f32::consts::TAU * 4.0) as usize).max(64);
    (0..n)
        .map(|i| {
            let t = i as f32 / n as f32 * std::f32::consts::TAU;
            at(cx + radius * t.cos(), cy + radius * t.sin())
        })
        .collect()
}

/// A node with no light of its own writes into no other node's colour.
///
/// `GlowFade` hands a strip row only to a node that has a light. Everything
/// else is given `GlowStep::default()` — and that default's row is 0, its mix
/// 1.0. A node with no light is still SHIPPED whenever it draws anything at
/// all (`paints`: an audio ring is enough), and `fs_ink_strip` draws for every
/// instance without looking at the level, so such a node settles its own ink
/// into row 0 at full weight and takes the colour of whichever node actually
/// owns that row.
///
/// Ordinary material reaches this, not a stress test: turn the audio ring on
/// and every ringing node that is not itself lit — most of them, with the Gate
/// low — writes over row 0. The node holding row 0 is the first node to have
/// lit in the session, so what a listener sees is one node's halo wearing the
/// wrong hue and flickering between wrong hues, since which of the several
/// writers lands last is the rasteriser's business and not stable frame to
/// frame.
///
/// Two nodes, two layers, one colour each: the lit node draws the RED octave
/// band and no mark, the unlit one draws the GREEN mark strip and nothing
/// else. The lit node's halo has to stay red.
#[test]
fn a_node_with_no_light_writes_into_no_other_nodes_colour() {
    const SIZE: [u32; 2] = [256, 256];
    const WIDTH: f32 = 0.18;
    let Some(mut shooter) = Shooter::new(SIZE) else {
        return;
    };
    // The band red and the mark green, with the LIT node wearing only the
    // band — so every green pixel of light in the frame came off the other
    // node's ink rather than out of this one's own strip.
    let scene = || -> Scene {
        let mut scene = two_colour_node(WIDTH, WIDTH);
        scene.nodes[0].melody_slots = 0;
        scene.nodes[0].melody_level = 0.0;
        scene.nodes[0].glow.mix = 1.0;
        let mut idle = scene.nodes[0];
        idle.world_pos.x += 1.2;
        idle.octaves = [0.0; harmonigraph_scene::OCTAVE_SLOTS];
        idle.activation = 0.0;
        idle.melody_slots = u32::MAX;
        idle.melody_level = 1.0;
        // What `GlowFade` hands a node it gave no row to.
        idle.glow = harmonigraph_scene::GlowStep::default();
        scene.nodes.push(idle);
        scene
    };
    let unlit = |mut scene: Scene| -> Scene {
        scene.glow_reach = 0.0;
        scene
    };

    let ground = shooter.shot(&unlit(scene()));
    let light = added_light(&shooter.shot(&scene()), &ground);

    // Non-vacuous first, on the WHOLE halo rather than on its red: the defect
    // takes the red to nothing, so a red-only check here reports "nothing is
    // lit" for a frame that is brightly lit in the wrong colour.
    assert!(light.iter().sum::<i64>() > 64, "the lit node lit nothing at all: {light:?}",);
    assert!(
        light[0] > light[1] * 4,
        "the lit node's halo came out {light:?}: it is drawing the RED band and no mark, so \
         the green is the idle node's ink settling into the row it was never given",
    );
}

/// A light already in its RELEASE survives the pane changing size.
///
/// The colour half of a node's light lives only in the ink strip, and a node
/// whose note fade has run out draws no layer at all — `ink_at` gates the band
/// on `params.x`, the ring on `in.ring` and the marks on `params.y`/`z`, and
/// every one of them is 0. That is the designed state, not an edge: a level can
/// stand above zero on a node whose every layer has gone silent, and such a
/// node is shipped for exactly that reason. Its halo's colour is therefore
/// entirely what the strip already HELD.
///
/// So the strip is the one thing that must not be dropped underneath it. Any
/// change to the pane's pixel size rebuilds the offscreen targets, and a strip
/// rebuilt from nothing hands a releasing node `held = 0` with no ink to seed
/// from — `glow_layer` reads `ink.w <= 0` and returns nothing, on that frame
/// and every frame after. The halo does not fade, it disappears.
///
/// What that looks like: hold a chord, let go, and while the light is still
/// running out drag the window's edge, drag the dock separator over the
/// lattice, or drag the window between a Retina display and an external
/// monitor — that last one moves `pixels_per_point`, so the pixel size changes
/// at an unchanged point size. Every lingering halo snaps off in one frame,
/// while halos on nodes still holding keys are untouched (they have ink of
/// their own). It reads as a bug in the release rather than in the resize.
///
/// Measured against the SAME node one ordinary frame on, so the claim is
/// "a resize is not different from a frame" rather than a number.
#[test]
fn a_light_in_its_release_survives_the_pane_changing_size() {
    const SIZE: [u32; 2] = [256, 256];
    const GROWN: [u32; 2] = [256, 260];
    const WIDTH: f32 = 0.18;
    let Some(mut shooter) = Shooter::new(SIZE) else {
        return;
    };
    // Sounding: the octave band alone, which is what puts a colour in the row.
    let sounding = || -> Scene {
        let mut scene = two_colour_node(WIDTH, 0.0);
        scene.nodes[0].glow.mix = 1.0;
        scene
    };
    // Releasing: the note fade has run out, so the node draws no layer at all
    // and takes none of this frame's ink — only its light is left, and only
    // the strip knows what colour it is.
    let releasing = || -> Scene {
        let mut scene = two_colour_node(0.0, 0.0);
        scene.nodes[0].glow.mix = 0.0;
        scene
    };
    let unlit = |mut scene: Scene| -> Scene {
        scene.glow_reach = 0.0;
        scene
    };

    // The two grounds first, each on a pane of its own, so that the carrying
    // sequence below runs without a shot in the middle of it taking the pane.
    let ground = shooter.shot(&unlit(releasing()));
    shooter.size = GROWN;
    let ground_grown = shooter.shot(&unlit(releasing()));
    shooter.size = SIZE;

    // A sounding frame to put a colour in the row, then the release.
    let _ = shooter.shot(&sounding());
    let kept = added_light(&shooter.shot_again(&releasing()), &ground);
    // And the same release again, with the pane one pixel wider.
    shooter.size = GROWN;
    let resized = added_light(&shooter.shot_again(&releasing()), &ground_grown);

    // Non-vacuous first: the release has to light the halo at all, or the
    // comparison below is between two nothings.
    assert!(kept[0] > 64, "a releasing node lit nothing to begin with: {kept:?}",);
    // The claim. Half is generous — the frame is a little larger, and the
    // light is stepped once more — where the failure takes it to zero.
    assert!(
        resized[0] > kept[0] / 2,
        "the light went from {kept:?} to {resized:?} when the pane changed size: \
         a node drawing no ink of its own has only the strip's held colour, and \
         a strip rebuilt with the offscreen targets hands it none",
    );
}

/// A node's light takes its colour from the frame before, not from this frame's
/// ink alone.
///
/// The COLOUR half of the glow's own clock. A node's ink is read in WGSL and
/// kept in a strip on the GPU, so this is where it is carried: the reading is
/// mixed into the row that node already had
/// (`harmonigraph_scene::GlowStep::mix`), on the same coefficient the level
/// took on the CPU. What that buys is a hue that MORPHS when the chord under it
/// changes, rather than one that cuts.
///
/// [`two_colour_node`]'s two layers, which is the fixture built for exactly
/// this reading: the octave band flat RED and the audio ring flat GREEN, so a
/// halo's red against its green is which layer coloured it, with no mixture of
/// one able to be mistaken for the other. The node keeps its identity across
/// the two frames — same position, same row — and swaps which of the two layers
/// it is drawing, which is as sharp a change of hue as a node can make.
#[test]
fn a_nodes_light_takes_its_colour_from_the_frame_before() {
    const SIZE: [u32; 2] = [256, 256];
    const WIDTH: f32 = 0.18;
    let Some(mut shooter) = Shooter::new(SIZE) else {
        return;
    };
    // The band alone, and the ring alone, each at the same width so neither
    // layer carries more of the node than the other.
    let red = |mix: f32| -> Scene {
        let mut scene = two_colour_node(WIDTH, 0.0);
        scene.nodes[0].glow.mix = mix;
        scene
    };
    let green = |mix: f32| -> Scene {
        let mut scene = two_colour_node(0.0, WIDTH);
        scene.nodes[0].glow.mix = mix;
        scene
    };
    let unlit = |mut scene: Scene| -> Scene {
        scene.glow_reach = 0.0;
        scene
    };
    // The two ends, each settled on a pane of its own.
    let red_off = shooter.shot(&unlit(red(1.0)));
    let green_off = shooter.shot(&unlit(green(1.0)));
    let all_red = added_light(&shooter.shot(&red(1.0)), &red_off);
    let all_green = added_light(&shooter.shot(&green(1.0)), &green_off);
    // And the frame after a red one, on the same pane, taking a tenth of the
    // new reading — a Glow attack long against the frame it is stepped over.
    let _ = shooter.shot(&red(1.0));
    let carried = added_light(&shooter.shot_again(&green(0.1)), &green_off);

    // Non-vacuous first: each layer alone has to light the halo in its own
    // colour, or the reading below is measuring nothing.
    assert!(all_red[0] > all_red[1] * 4, "the band alone must light the halo red: {all_red:?}",);
    assert!(
        all_green[1] > all_green[0] * 4,
        "the ring alone must light the halo green: {all_green:?}",
    );
    // The claim: one frame in, the light is still mostly the colour it was,
    // though the node is drawing nothing but the ring.
    assert!(
        carried[0] > carried[1],
        "a light that took a tenth of the new reading came out {carried:?} — that is the \
         ring's green, so the row was written rather than mixed into",
    );
}
