//! The shadow a resting cross casts into the light standing over it.

use super::fixtures::*;
use crate::*;

/// Which pixels of `ground` the markers DARKEN, against the same frame with no
/// marker in it.
///
/// `ground` is the set the markers' ink never reaches, taken from the pair at a
/// depth of 0 where a marker writes ink and nothing else. It is the ink's own
/// footprint that is being excluded and the footprint does not move with the
/// depth, so one reading of it answers for every shot here.
fn shadowed_ground(
    shooter: &mut Shooter,
    shadow: f32,
    taper_start: f32,
) -> (Vec<usize>, std::collections::BTreeSet<usize>) {
    let flat_bare = shooter.shot(&{
        let mut s = shadowed_markers(0.0, shadow, taper_start);
        s.pluses.clear();
        s
    });
    let flat = shooter.shot(&shadowed_markers(0.0, shadow, taper_start));
    let ground: Vec<usize> =
        (0..flat.len()).step_by(4).filter(|&i| flat[i..i + 4] == flat_bare[i..i + 4]).collect();
    let deep_bare = shooter.shot(&{
        let mut s = shadowed_markers(1.0, shadow, taper_start);
        s.pluses.clear();
        s
    });
    let deep = shooter.shot(&shadowed_markers(1.0, shadow, taper_start));
    let dimmed: std::collections::BTreeSet<usize> = ground
        .iter()
        .copied()
        .filter(|&i| brightness(&deep[i..i + 3]) < brightness(&deep_bare[i..i + 3]))
        .collect();
    // The pair at a depth of 0 is the other half of every claim below: with the
    // Shadow shut a marker writes no standoff at all, and nothing else in the light
    // can subtract.
    let flat_dimmed = ground
        .iter()
        .filter(|&&i| brightness(&flat[i..i + 3]) < brightness(&flat_bare[i..i + 3]))
        .count();
    assert_eq!(flat_dimmed, 0, "a marker took light off the ground at a Shadow depth of 0");
    (ground, dimmed)
}

/// A marker holds a NODE's halo off its cross, on the Shadow bars the node's own
/// rings are held off by.
///
/// The whole claim of the feature, and the reason the standoff is written into
/// the glow's own target rather than onto the marker: what a marker stands off
/// is the melded field, which is somebody else's light entirely. A shadow that
/// never reached past the cross would mean the third attachment had been
/// written somewhere the composite does not read for the rest of the lattice.
///
/// The pair at a depth of 0 is asserted inside [`shadowed_ground`]: with the
/// Shadow shut nothing here subtracts, which is what says the darkening measured
/// is the standoff and not the marker draw finding some other way to take light
/// off the picture.
#[test]
fn a_marker_holds_a_nodes_halo_off_its_own_cross() {
    const SIZE: [u32; 2] = [256, 256];
    let Some(mut shooter) = Shooter::new(SIZE) else {
        return;
    };
    let (ground, dimmed) = shadowed_ground(&mut shooter, 0.8, 1.0);
    assert!(
        ground.len() > 1000,
        "the fixture must leave ground for the shadow to land on, not {}",
        ground.len(),
    );
    assert!(
        dimmed.len() > 200,
        "a full Shadow depth darkened {} of the {} pixels the markers' ink never reaches",
        dimmed.len(),
        ground.len(),
    );
}

/// A cross's shadow is worth its ink: half a marker takes half the light off
/// the halo it stands in, and a marker that is not there takes none.
///
/// This is the whole of what a marker hands the picture — one number, spent on
/// the ink, the pool and the standoff alike (`PlusInstance::strength`) — and it
/// is what a position handing itself back to a name looks like: the cross grows
/// and the shadow under it grows on the same clock. A shadow closed against the
/// LIGHT instead runs on the Glow release, so a cross fully back would stand
/// with nothing under it for as long as the halo it stands in takes to leave.
///
/// Measured on the light TAKEN rather than on a pixel count, which is what
/// makes the middle of the ramp a claim rather than a rounding: a shallower
/// shadow lightens the pixels it bites before it stops biting them, and an
/// 8-bit count sees the second and not the first. A marker's opacity puts NO
/// light into these pixels, so the only thing it can do out there is take some
/// away.
///
/// The excluded footprint is read at full ink, where it is largest — a fainter
/// marker inks a subset of it — so one ground set answers for every shot.
///
/// The marker stands OUT in the node's halo and not at the node's own centre,
/// which is `shadowed_markers`' delicate number and the one thing this fixture
/// cannot do without: the shade layer is a `max`, so a cross standing inside
/// the node's own standoff can only be read where it holds off MORE than the
/// node does — and a half-strength one holds off less than that, which reads
/// as a marker with no shadow at all rather than as one with half.
#[test]
fn a_crosss_shadow_is_worth_its_ink() {
    const SIZE: [u32; 2] = [256, 256];
    let Some(mut shooter) = Shooter::new(SIZE) else {
        return;
    };
    // One marker out in the node's halo, in a frame whose only light is that
    // node's, `shadowed_markers` putting no other light in the frame.
    let at = |strength: f32, depth: f32| -> Scene {
        let mut scene = shadowed_markers(depth, 0.8, 1.0);
        scene.pluses =
            vec![one_marker(glam::Vec3::new(2.6, 0.0, 0.0), 0.4, scene.lattice_ground, strength)];
        scene
    };
    let bare = |shooter: &mut Shooter, depth: f32| {
        shooter.shot(&{
            let mut s = at(0.0, depth);
            s.pluses.clear();
            s
        })
    };

    // The ink's own footprint, and everything it reaches excluded from the
    // counts below: what is being measured is light the cross took off the
    // picture around it, not the cross itself.
    let flat_bare = bare(&mut shooter, 0.0);
    let flat = shooter.shot(&at(1.0, 0.0));
    let ground: Vec<usize> =
        (0..flat.len()).step_by(4).filter(|&i| flat[i..i + 4] == flat_bare[i..i + 4]).collect();
    assert!(
        ground.len() > 1000,
        "the fixture must leave halo for a bite to land in, not {}",
        ground.len(),
    );

    let deep_bare = bare(&mut shooter, 1.0);
    let taken = |frame: &[u8]| -> i64 {
        ground
            .iter()
            .map(|&i| (brightness(&deep_bare[i..i + 3]) - brightness(&frame[i..i + 3])).max(0))
            .sum()
    };
    let whole = taken(&shooter.shot(&at(1.0, 1.0)));
    let half = taken(&shooter.shot(&at(0.5, 1.0)));
    let none = taken(&shooter.shot(&at(0.0, 1.0)));
    assert!(
        whole > 0,
        "a whole cross took no light off the halo it stands in — with none there is nothing \
         for the rest of this to be a share of",
    );
    assert!(
        half > 0 && half < whole,
        "half a cross took {half} of the {whole} a whole one takes, which is not a share of it",
    );
    assert_eq!(none, 0, "a marker with no ink still took {none} of light off the halo");
}

/// The Shadow's WIDTH says how far a marker's shadow reaches, on the same bar it
/// says it to a node's rings.
///
/// The depth alone would be a shadow of one size that could be dialled darker,
/// which is not what the bar means anywhere else in the picture. A superset is
/// what says the width stretches one shape rather than deepening it: every
/// pixel a narrow Shadow darkens, a wide one darkens too.
///
/// Read as the DIFFERENCE the crosses make at each Shadow, which is what keeps the
/// claim about them: the node's own standoff widens on the same bar and the
/// shade layer is a `max`, so a frame read on its own says which of two shadows
/// won rather than how far this one reaches. The two frames at one Shadow carry
/// the same node, so what survives the cancellation is the crosses' own shadow.
#[test]
fn a_markers_shadow_reaches_as_far_as_its_width_says() {
    const SIZE: [u32; 2] = [256, 256];
    let Some(mut shooter) = Shooter::new(SIZE) else {
        return;
    };
    // [`shadowed_markers`]' own Shadow at the wide end, which is where its crosses
    // are calibrated to stand clear of the node's standoff. Past it that
    // standoff reaches them and wins the `max`, and pixels start leaving the
    // count for a reason that is the node's rather than the Shadow's — at 1.2 it
    // takes eight of them.
    const WIDE: f32 = 0.8;
    const NARROW: f32 = 0.15;
    let narrow = shadowed_ground(&mut shooter, NARROW, 1.0).1;
    let wide = shadowed_ground(&mut shooter, WIDE, 1.0).1;
    assert!(!narrow.is_empty(), "the narrow Shadow must cast a shadow at all");
    assert!(
        wide.len() > narrow.len() * 2,
        "widening the Shadow from {NARROW} to {WIDE} shadowed {} against {}",
        wide.len(),
        narrow.len(),
    );
    let missed = narrow.difference(&wide).count();
    assert_eq!(missed, 0, "the wider Shadow left {missed} of the narrow shadow's pixels lit");
}

/// A marker's shadow is cast by the WHOLE arm, tip and taper alike: a tapered
/// cross shadows nearly as far out as a square-ended one of its length, and far
/// past what its SOLID length alone would cast.
///
/// The taper is a length beside the reach, not a share of it, and the fresh one
/// is nearly half the arm — so a shadow cast from where the taper STARTS comes
/// off a cross barely longer than the square its arms cross in, and one Shadow
/// of dilation rounds that into a dark square standing under a plus. What the
/// eye reads a marker by is its ARMS, and this is the claim that keeps them in
/// the shadow.
///
/// Three arms and not two, because the taper also costs the shadow DEPTH
/// (`plus_standoff`) and a reach read off a threshold gives a little of itself
/// up with it. The third is the ruler that says how little: an arm cut to the
/// tapered one's solid length, which is the shadow the truncation drew.
///
/// Read along the arm rather than as a pixel count, and that is what makes it a
/// claim about the SHAPE: a count also moves when the taper moves the ink's own
/// footprint out of the ground being counted, so two counts can differ with the
/// shadow's reach unchanged. Each cross is walked against its OWN depth-0 frame,
/// which is what cancels that ink; what is left is the standoff alone.
#[test]
fn a_markers_shadow_is_cast_by_the_whole_arm_however_it_ends() {
    const SIZE: [u32; 2] = [256, 256];
    // `one_shadow_is_one_distance_whatever_the_cross_it_stands_off`'s numbers,
    // and for its reasons: an arm wide enough on screen to rule a shadow by,
    // and a Shadow that finishes well inside the halo doing the ruling.
    const SHADOW: f32 = 0.30;
    const ARM: f32 = 0.9;
    const SQUARE: f32 = 1.0;
    const TAPERED: f32 = 0.25;
    let Some(mut shooter) = Shooter::new(SIZE) else {
        return;
    };
    let row = SIZE[1] as usize / 2;
    let at = |buf: &[u8], x: usize| -> i64 {
        let i = (row * SIZE[0] as usize + x) * 4;
        brightness(&buf[i..i + 3])
    };
    // The crossing's own column, off the square-ended cross's ink against a
    // frame with no cross in it: the marker stands in the same place at every
    // arm here, so one reading of it starts all three walks.
    let bare = shooter.shot(&{
        let mut s = lone_tapered_marker(ARM, SHADOW, 0.0, SQUARE);
        s.pluses.clear();
        s
    });
    let flat_square = shooter.shot(&lone_tapered_marker(ARM, SHADOW, 0.0, SQUARE));
    let cols: Vec<usize> = (0..SIZE[0] as usize)
        .filter(|&x| {
            let i = (row * SIZE[0] as usize + x) * 4;
            flat_square[i..i + 4] != bare[i..i + 4]
        })
        .collect();
    let mid = cols.first().unwrap().midpoint(*cols.last().unwrap());

    // The outermost column right of the crossing where the standoff reads
    // darker than the same frame without one. Rightward because the node
    // lighting the frame is to the LEFT: that half holds no other ink and no
    // other shadow.
    let reach = |shooter: &mut Shooter, arm: f32, taper: f32| -> usize {
        let cold = shooter.shot(&lone_tapered_marker(arm, SHADOW, 0.0, taper));
        let hot = shooter.shot(&lone_tapered_marker(arm, SHADOW, 1.0, taper));
        let mut out = mid;
        for x in mid..SIZE[0] as usize {
            if at(&hot, x) < at(&cold, x) {
                out = x;
            }
        }
        out
    };
    let square = reach(&mut shooter, ARM, SQUARE);
    let tapered = reach(&mut shooter, ARM, TAPERED);
    let solid = reach(&mut shooter, ARM * TAPERED, SQUARE);
    assert!(square > mid, "the square-ended cross cast no shadow past its own crossing");
    assert!(
        solid < square,
        "the solid length cast as far as the whole arm ({solid} against {square}), so there is \
         no span here for the tapered cross to be placed along",
    );
    // Two thirds of the way, where the taper is three quarters of the arm: the
    // claim is which of the two lengths the shadow is cast from, and a third of
    // that span is more room than the depth's own share of the threshold needs.
    let span = square - solid;
    assert!(
        tapered >= solid + span * 2 / 3,
        "a cross tapering from {TAPERED} of its arm shadowed out to {tapered}, against {solid} \
         for its solid length alone and {square} for the whole arm",
    );
}

/// A tapered arm gives up its shadow with its ink, and gives up the SHARE of it
/// that it gives up of its own length.
///
/// An arm's ink fades to nothing over its last stretch so that it arrives at
/// nothing rather than stopping at something (`plus_coverage`); a shadow held at
/// full depth under that stretch caps the arm in dark exactly where the ink has
/// gone, which is the arrival the taper is there to prevent.
///
/// Measured as light TAKEN off the halo rather than as a reach, because what
/// moves here is the depth and not the footprint — the test above is the one
/// that holds the footprint still. The ground is the square end's, which is the
/// largest ink of the three: a tapered cross draws inside that same box at a
/// lower alpha, so one exclusion answers for all of them.
///
/// The square end is the top of the family and that is the other half of the
/// claim: the fade is spent at the taper's own width, so an arm with no taper
/// pays nothing for having none.
#[test]
fn a_tapered_arm_gives_up_the_share_of_its_shadow_it_gives_up_of_its_length() {
    const SIZE: [u32; 2] = [256, 256];
    const SHADOW: f32 = 0.30;
    const ARM: f32 = 0.9;
    let Some(mut shooter) = Shooter::new(SIZE) else {
        return;
    };
    let bare = |shooter: &mut Shooter, depth: f32| {
        shooter.shot(&{
            let mut s = lone_tapered_marker(ARM, SHADOW, depth, 1.0);
            s.pluses.clear();
            s
        })
    };
    let flat_bare = bare(&mut shooter, 0.0);
    let flat = shooter.shot(&lone_tapered_marker(ARM, SHADOW, 0.0, 1.0));
    let ground: Vec<usize> =
        (0..flat.len()).step_by(4).filter(|&i| flat[i..i + 4] == flat_bare[i..i + 4]).collect();
    assert!(
        ground.len() > 1000,
        "the fixture must leave halo for a bite to land in, not {}",
        ground.len(),
    );
    let deep_bare = bare(&mut shooter, 1.0);
    let taken = |shooter: &mut Shooter, taper: f32| -> i64 {
        let frame = shooter.shot(&lone_tapered_marker(ARM, SHADOW, 1.0, taper));
        ground
            .iter()
            .map(|&i| (brightness(&deep_bare[i..i + 3]) - brightness(&frame[i..i + 3])).max(0))
            .sum()
    };
    let square = taken(&mut shooter, 1.0);
    let some = taken(&mut shooter, 0.6);
    let most = taken(&mut shooter, 0.2);
    assert!(square > 0, "a square-ended cross took no light off the halo it stands in");
    assert!(
        square > some && some > most,
        "an arm fading over none, two fifths and four fifths of its length took {square}, {some} \
         and {most} of light off the halo, which is not one order",
    );
}

/// One Shadow is ONE distance: what a marker's shadow reaches past the ink casting
/// it is a world length off the bar, not a share of the cross.
///
/// This is the whole of what sharing the node's Shadow bar buys, and it is a claim
/// no relative measurement can hold. The standoff is taken in the QUAD's uv,
/// where the box's half-extents carry the arm; taking it in the ARM's instead —
/// the reading `plus_coverage` twenty lines away invites, half-extents of
/// `misc5.y` and `misc5.x` with the distance divided by the arm — leaves every
/// other shadow test here passing, each being monotone in the Shadow, a superset,
/// or a comparison between two arms of one length. What it changes is that each
/// marker's shadow scales with its own cross, so the lattice's Shadow is as many
/// distances as there are marker sizes; only a frame holding two different arms
/// can see it, and only against a world ruler.
///
/// The ruler is the cross's own INK: a square-ended arm draws out to exactly its
/// own length, so the longer marker's two arm tips are a known number of world
/// units apart and every length below is read through that. Its own ink and not
/// the halo it stands in, which reaches the pane's every corner and has no edge
/// to measure — and an edge on a decay would be a threshold where this is a
/// footprint. The shadow's own edge IS such a threshold (`standoff_coverage`
/// never reaches zero), which is why the claim is a DIFFERENCE between two arms
/// under one threshold rather than either arm's reach against the Shadow.
#[test]
fn one_shadow_is_one_distance_whatever_the_cross_it_stands_off() {
    const SIZE: [u32; 2] = [256, 256];
    // Narrow enough that the longer arm's shadow finishes well inside the halo
    // ruling it, which is a condition on the READING and not on the claim: the
    // edge below is where a difference crosses the 8-bit floor, and out where
    // the light is already dim the deeper of two shadows crosses it early, so
    // two reaches that differ by an arm's length are measured as differing by
    // less.
    const SHADOW: f32 = 0.30;
    const SHORT: f32 = 0.35;
    const LONG: f32 = 0.9;
    let Some(mut shooter) = Shooter::new(SIZE) else {
        return;
    };
    let row = SIZE[1] as usize / 2;
    let at = |buf: &[u8], x: usize| -> i64 {
        let i = (row * SIZE[0] as usize + x) * 4;
        brightness(&buf[i..i + 3])
    };
    // The columns on the centre row where `marked` differs from `bare` at all:
    // one contiguous run per shot here, the frame holding one cross and nothing
    // else to the node's right.
    let span = |marked: &[u8], bare: &[u8]| -> (usize, usize) {
        let cols: Vec<usize> = (0..SIZE[0] as usize)
            .filter(|&x| {
                let i = (row * SIZE[0] as usize + x) * 4;
                marked[i..i + 4] != bare[i..i + 4]
            })
            .collect();
        (*cols.first().unwrap(), *cols.last().unwrap())
    };

    // The ruler: the long cross's own ink, against a frame with no cross in it.
    // Its arms run 2 * LONG world tip to tip along this row.
    let bare = shooter.shot(&{
        let mut s = lone_shadowed_marker(LONG, SHADOW, 0.0);
        s.pluses.clear();
        s
    });
    let flat_long = shooter.shot(&lone_shadowed_marker(LONG, SHADOW, 0.0));
    let (left, right) = span(&flat_long, &bare);
    let per_world = (right - left) as f32 / (LONG * 2.0);
    assert!(
        per_world > 8.0,
        "the cross has to be drawn wide enough to rule a shadow by, not {per_world:.1}px per world",
    );
    // Its own centre, so the walk below starts at the crossing and goes out.
    let mid = left.midpoint(right);

    // The outermost column right of the crossing where `hot` reads darker than
    // `cold`. Rightward because the node lighting the frame is to the LEFT: the
    // half being walked holds no other ink and no other shadow.
    let edge = |hot: &[u8], cold: &[u8]| -> usize {
        let mut out = mid;
        for x in mid..SIZE[0] as usize {
            if at(hot, x) < at(cold, x) {
                out = x;
            }
        }
        out
    };

    let flat_short = shooter.shot(&lone_shadowed_marker(SHORT, SHADOW, 0.0));
    let short_reach = edge(&shooter.shot(&lone_shadowed_marker(SHORT, SHADOW, 1.0)), &flat_short);
    let long_reach = edge(&shooter.shot(&lone_shadowed_marker(LONG, SHADOW, 1.0)), &flat_long);
    assert!(
        long_reach > short_reach && short_reach > mid,
        "both arms must cast a shadow, the longer one further: {short_reach} and {long_reach}",
    );

    // What the longer cross buys is its own extra ink and nothing else: the Shadow
    // past the ink is one distance, so the two shadows differ by exactly the
    // two arms' difference.
    let grew = (long_reach - short_reach) as f32 / per_world;
    let want = LONG - SHORT;
    assert!(
        (grew - want).abs() < 0.12,
        "an arm {} longer pushed its shadow {grew:.2} further ({short_reach}px to {long_reach}px \
         at {per_world:.1}px per world) — the Shadow is being read as a share of the ink",
        want,
    );
}

/// A RESTING MARKER wears the wash, out of the same field a node's ink takes it
/// from.
///
/// The marker field is drawn over ground the light is already under, so unwashed
/// a marker inside a halo would be the one thing in the picture that gets darker
/// the more light stands at it — flat ground laid over lit ground — and the
/// lattice would read as a field of holes punched exactly where the light is
/// brightest. That is what the wash closes here.
///
/// Two claims, on the marker's own pixels:
///
/// - The light lifts more than half of them. That claim is also what makes
///   the fixture honest — nothing lifts where no light reaches, so a marker
///   parked outside the halo fails here rather than passing vacuously.
/// - Nothing is ever dimmed, the wash being a screen (`wash_over`).
///
/// The pixels are found by DIFFING the field in and out of the scene rather
/// than off the geometry, so a marker the node happened to cover leaves the set
/// empty instead of quietly handing these claims to the node — and then
/// narrowed to the ones the marker covers in FULL. What shows through its
/// antialiased rim is the ground's share of the light, which is the Shadow bars'
/// answer and not this bar's.
#[test]
fn a_resting_marker_wears_the_wash_it_stands_in() {
    const SIZE: [u32; 2] = [256, 256];
    let Some(mut shooter) = Shooter::new(SIZE) else {
        return;
    };
    // The lit node at the origin, and one marker beside it. uv 1 is 1.8 node
    // radii (`node_vertex`), so this node's outermost ring at uv 0.795 reaches
    // 1.57 world units and the marker's near tip stands at 2.2 — clear of the
    // ink, and inside a reach dialled to carry light well past it. The feather
    // is at the top of its bar so what stands out there is an even field
    // rather than the skirt of a falloff heaped on the node.
    let at = |reach: f32, marker: bool| -> Scene {
        let mut scene = single_marked_node(0, 0);
        scene.camera = harmonigraph_scene::Camera {
            projection: harmonigraph_scene::Projection::Orthographic,
            yaw: 0.0,
            pitch: 0.0,
            ..Default::default()
        };
        scene.glow_reach = reach;
        scene.glow_strength = 1.5;
        scene.glow_feather = 1.0;
        if marker {
            scene.pluses =
                vec![one_marker(glam::Vec3::new(3.0, 0.0, 0.0), 0.8, scene.lattice_ground, 1.0)];
        }
        scene
    };
    let bare = shooter.shot(&at(0.0, false));
    // The glow OFF is what the marker is measured against: no light at the
    // pixel is the one setting left that takes the wash out of the picture.
    let off = shooter.shot(&at(0.0, true));
    let drawn: Vec<usize> = (0..bare.len())
        .step_by(4)
        .filter(|&i| bare[i..i + 4] == [0u8, 0, 0, 255] && off[i..i + 4] != bare[i..i + 4])
        .collect();
    // The marker's colour, read off the picture rather than converted by hand:
    // it is laid down flat and premultiplied over a black frame, so a pixel it
    // covers COMPLETELY carries that colour exactly and every other one carries
    // a fraction of it. The brightest value in the set is therefore the colour
    // itself, and the pixels holding it are the ones with nothing showing
    // through.
    let full: [u8; 3] =
        std::array::from_fn(|c| drawn.iter().map(|&i| off[i + c]).max().unwrap_or(0));
    let marker: Vec<usize> = drawn.into_iter().filter(|&i| off[i..i + 3] == full).collect();
    assert!(
        marker.len() > 300,
        "the marker covers {} pixels the node had not already covered",
        marker.len(),
    );

    let worn = shooter.shot(&at(1.6, true));
    let lifted = marker
        .iter()
        .filter(|&&i| brightness(&worn[i..i + 3]) > brightness(&off[i..i + 3]))
        .count();
    assert!(
        lifted * 2 > marker.len(),
        "the light lifted {lifted} of the marker's {} pixels: the marker is not wearing the \
         light it stands in",
        marker.len(),
    );
    let dimmed = marker.iter().filter(|&&i| (0..3).any(|c| worn[i + c] < off[i + c])).count();
    assert_eq!(
        dimmed,
        0,
        "the wash took light off {dimmed} of the marker's {} pixels",
        marker.len(),
    );
    // The furthest any one channel of the marker moves between the two shots,
    // which is what the fixture's own non-vacuity is read in.
    let spread = |a: &[u8], b: &[u8]| {
        marker
            .iter()
            .map(|&i| (0..3).map(|c| a[i + c].abs_diff(b[i + c])).max().unwrap())
            .max()
            .unwrap()
    };
    let by_wash = spread(&worn, &off);
    assert!(
        by_wash > 20,
        "the fixture's wash moves the marker by {by_wash}; there is nothing here to measure",
    );
}

/// A node the analyzer lit, with no key down, keeps the cross under it.
///
/// The cross goes when a NAME stands over it and at no other time
/// (`derive_pluses`), and a node ringing under no key has no name to put there.
/// What used to take it anyway was the node's own knockout, drawn over the
/// markers because it drew after them: the Gate hands rings out freely, so
/// every node the analyzer reached lost the mark of the position it stands on
/// and kept the marker's standoff, a cross-shaped hole in the light with
/// nothing standing in it.
///
/// Measured against the SAME node with the Clearance dialled off, which is the
/// one setting where the hole was never painted — so the claim is that the
/// Clearance no longer decides this, rather than that some ink survives.
#[test]
fn a_node_lit_by_no_key_keeps_the_cross_under_it() {
    const SIZE: [u32; 2] = [256, 256];
    let Some(mut gpu) = Shooter::new(SIZE) else {
        return;
    };
    // Silent but ringing: the Gate's own gift, and the case the knockout used
    // to swallow. The light is on so there is something for a clearing to
    // knock out and something for the marker's ink to be washed by.
    let build = |gutter: f32, marker: bool| -> Scene {
        let mut scene = clearing_node(0, 1.0, true, gutter);
        scene.background = glam::Vec4::new(0.05, 0.05, 0.07, 1.0);
        scene.glow_reach = 3.0;
        scene.glow_strength = 2.0;
        scene.glow_feather = 1.0;
        let node = &mut scene.nodes[0];
        node.activation = 0.0;
        node.octaves = [0.0; harmonigraph_scene::OCTAVE_SLOTS];
        node.glow.level = 1.0;
        scene.pluses = if marker {
            vec![one_marker(glam::Vec3::ZERO, 0.35, glam::Vec4::new(0.6, 0.6, 0.6, 1.0), 1.0)]
        } else {
            Vec::new()
        };
        scene
    };
    // What the cross covers, read where no clearing is painted at all.
    let mut inked = |gutter: f32| -> Vec<usize> {
        let with = gpu.shot(&build(gutter, true));
        let without = gpu.shot(&build(gutter, false));
        (0..with.len()).step_by(4).filter(|&i| with[i..i + 4] != without[i..i + 4]).collect()
    };
    let bare = inked(0.0);
    assert!(bare.len() > 50, "the fixture draws no cross to lose: {} pixels", bare.len(),);
    for gutter in [0.2f32, 0.5, 1.0] {
        let cut = inked(gutter);
        assert!(
            cut.len() >= bare.len(),
            "at a Clearance of {gutter} the ringing node took {} of the {} pixels its \
             cross covers with the Clearance off",
            bare.len() - cut.len(),
            bare.len(),
        );
    }
}

/// A marker knocks a hole in what was drawn before it, the way a node does —
/// so a cross standing in front of a sheet covers that sheet's rings instead of
/// sitting on them.
///
/// The claim and the reading are [`a_name_covers_the_rings_it_stands_on`]'s in
/// `super::labels`, and deliberately so: the hole is one shape across the three
/// things the lattice draws, and it is measured the same way in each. A hole is
/// a premultiplied over of the GROUND at its own coverage, so every pixel it
/// touches lands BETWEEN the picture with no marker in it and that ground. The
/// Reach is 0, so the ground is one value for the whole frame.
///
/// The cross is LOOSE — at a lattice position no home node holds — which is
/// exactly the arrangement that puts it over the sheets behind home and under
/// the home sheet (`push_loose`). One node behind it, and the ring that node
/// draws is what the hole is read on.
#[test]
fn a_marker_covers_the_sheet_behind_it() {
    const SIZE: [u32; 2] = [256, 256];
    const SHADOW: f32 = 0.6;
    /// Where the sheet behind stands, in world units: far enough back to sort
    /// before the loose markers and square behind the cross on screen.
    const BEHIND: f32 = -1.0;
    let Some(mut shooter) = Shooter::new(SIZE) else {
        return;
    };
    let at = |shadow: f32| -> Scene {
        let mut scene = single_marked_node(0, 0);
        scene.camera = harmonigraph_scene::Camera {
            projection: harmonigraph_scene::Projection::Orthographic,
            yaw: 0.0,
            pitch: 0.0,
            ..Default::default()
        };
        // No light anywhere, so the ground a hole clears to is the background
        // and nothing else in the frame moves with the bar below.
        scene.glow_reach = 0.0;
        scene.glow_shadow = shadow;
        scene.glow_shadow_soft = shadow;
        scene.glow_shadow_depth = 1.0;
        // The one node, pushed back a sheet so the loose cross is drawn after
        // it. `on_home` is what `from_scene` sorts the markers against.
        scene.nodes[0].world_pos = glam::Vec3::new(0.0, 0.0, BEHIND);
        scene.nodes[0].on_home = false;
        scene
    };
    let cross = |scene: &Scene| -> Vec<harmonigraph_scene::PlusInstance> {
        vec![one_marker(glam::Vec3::ZERO, scene.node_radius * 0.9, scene.lattice_ground, 1.0)]
    };
    let mut shots = |shadow: f32| -> (Scene, Vec<u8>, Vec<u8>) {
        let bare = shooter.shot(&at(shadow));
        let mut with = at(shadow);
        with.pluses = cross(&with);
        let marked = shooter.shot(&with);
        (at(shadow), bare, marked)
    };

    // The cross's own INK, taken at a Shadow of 0 where a marker paints that
    // and nothing else. It does not move with the bar.
    let (_, flat_bare, flat) = shots(0.0);
    let ink: std::collections::BTreeSet<usize> =
        (0..flat.len()).step_by(4).filter(|&i| flat[i..i + 4] != flat_bare[i..i + 4]).collect();
    assert!(ink.len() > 200, "the fixture's cross must land on the pane, not {} px", ink.len());

    let (scene, bare, marked) = shots(SHADOW);
    let byte = |v: f32| (v.clamp(0.0, 1.0) * 255.0).round() as i32;
    let ground = [byte(scene.background.x), byte(scene.background.y), byte(scene.background.z)];
    let mut touched = 0usize;
    for i in (0..marked.len()).step_by(4) {
        if ink.contains(&i) || marked[i..i + 4] == bare[i..i + 4] {
            continue;
        }
        touched += 1;
        for c in 0..3 {
            let (was, now, to) = (bare[i + c] as i32, marked[i + c] as i32, ground[c]);
            assert!(
                now >= was.min(to) - 2 && now <= was.max(to) + 2,
                "a marker moved a pixel outside its own ink to {now}, which is not between \
                 the {was} it stood on and the {to} a hole clears to",
            );
        }
    }
    assert!(touched > 250, "a cross at Shadow {SHADOW} cleared only {touched} pixels");

    // And the pixels it cleared were RING: with the sheet behind taken out of
    // the scene, the same frame is bare ground there and there is nothing for a
    // hole to take away.
    let empty = shooter.shot(&Scene { nodes: Vec::new(), ..at(SHADOW) });
    let marked_empty =
        shooter.shot(&Scene { nodes: Vec::new(), pluses: cross(&scene), ..at(SHADOW) });
    let on_nothing = (0..marked_empty.len())
        .step_by(4)
        .filter(|&i| !ink.contains(&i) && marked_empty[i..i + 4] != empty[i..i + 4])
        .count();
    assert_eq!(
        on_nothing, 0,
        "with nothing behind it a cross must clear nothing, and it changed {on_nothing} pixels",
    );
}
