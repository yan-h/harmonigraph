//! The home sheet and what leaves it: off-sheet shrink and blanking, the
//! knockout each sounding node clears behind it, and comma spelling.

use super::harness::*;
use crate::*;
use harmonigraph_core::{NoteEvent, NoteTracker, Tuning};

#[test]
fn home_sheet_nodes_are_flagged_for_the_blank_ring() {
    // Follows the panned window center, not sevens == 0.
    let view = ViewConfig {
        extent_threes: 0,
        extent_fives: 0,
        extent_sevens: 1,
        center_sevens: 2,
        ..ViewConfig::default()
    };
    let scene = scene_of(&NoteTracker::new(), &Tuning::default(), &view, &plain_frame(), 0.0);
    for n in &scene.nodes {
        assert_eq!(n.on_home, n.lattice_pos.sevens == 2, "{:?}", n.lattice_pos);
    }
}

#[test]
fn the_marker_field_is_the_home_sheet_and_only_it() {
    // The resting picture belongs to ONE sheet, and that is the whole of what
    // makes it the ground: a position off it is unmarked at rest, which is
    // also why it is not hoverable. Nothing the music does changes that — an
    // off-sheet note floats over the field at the size its sheet gives it,
    // with nothing drawn between it and home.
    let view = ViewConfig { extent_threes: 1, extent_fives: 0, extent_sevens: 2, ..plain_view() };
    let home_count = |scene: &Scene| scene.nodes.iter().filter(|n| n.on_home).count();

    // Idle: a marker at every home position, and no mark anywhere off it.
    let scene = scene_of(&NoteTracker::new(), &Tuning::default(), &view, &plain_frame(), 0.0);
    assert_eq!(scene.pluses.len(), home_count(&scene));
    assert!(scene.pluses.iter().all(|d| d.pos.z.abs() < 1e-5), "{:?}", scene.pluses);

    // Hold the note two sevens steps up from C (12-TET default:
    // 2 × 1000¢ → pitch class 800¢ = G#/Ab, MIDI 68). It lights node
    // (0,0,2) only, and the field under it is untouched.
    let mut tracker = NoteTracker::new();
    tracker.handle_event(NoteEvent::on(0.0, 0, 68, 1.0));
    let played = scene_of(&tracker, &Tuning::default(), &view, &plain_frame(), 0.0);
    assert!(
        played.nodes.iter().any(|n| !n.on_home && n.activation > 0.0),
        "the off-sheet note has to be lit for this to mean anything",
    );
    assert_eq!(played.pluses.len(), scene.pluses.len(), "a note off the sheet changed the field");
    assert!(played.pluses.iter().all(|d| d.pos.z.abs() < 1e-5), "{:?}", played.pluses);
}

#[test]
fn the_mark_depth_reaches_the_scene_and_is_clamped() {
    // One thickness drives BOTH rings, so it lives on the scene rather
    // than per node; 0 is the off state, as it is for every layer's width.
    let view = ViewConfig { mark_thickness: 0.15, ..ViewConfig::default() };
    let scene = scene_of(&NoteTracker::new(), &Tuning::default(), &view, &plain_frame(), 0.0);
    assert_eq!(scene.mark_thickness, 0.15);

    let off = ViewConfig { mark_thickness: 0.0, ..ViewConfig::default() };
    let scene = scene_of(&NoteTracker::new(), &Tuning::default(), &off, &plain_frame(), 0.0);
    assert_eq!(scene.mark_thickness, 0.0, "0 passes through as the off state");

    let wild = ViewConfig { mark_thickness: 9.0, ..ViewConfig::default() };
    let scene = scene_of(&NoteTracker::new(), &Tuning::default(), &wild, &plain_frame(), 0.0);
    assert!(scene.mark_thickness <= 0.3, "got {}", scene.mark_thickness);
}

#[test]
fn the_octave_gap_reaches_the_scene_and_is_clamped() {
    // The ANGULAR padding, which is the one the shader has to be handed: it
    // cuts the sectors apart per fragment, so nothing upstream can spend it and
    // it survives as its own number rather than as a shader constant. (The
    // radial one reaches the picture as the radii themselves — that is
    // `the_rings_stack_outward_from_the_centre` below.)
    let view = ViewConfig { octave_gap: 0.2, ..ViewConfig::default() };
    let scene = scene_of(&NoteTracker::new(), &Tuning::default(), &view, &plain_frame(), 0.0);
    assert_eq!(scene.octave_gap, 0.2);

    // The cap is what a hand-edited blob is held to: anything past a fraction
    // of a turn erases every sector on the node.
    let wild = ViewConfig { octave_gap: 5.0, ..ViewConfig::default() };
    let scene = scene_of(&NoteTracker::new(), &Tuning::default(), &wild, &plain_frame(), 0.0);
    assert!(scene.octave_gap <= 0.4, "got {}", scene.octave_gap);
}

/// The two paddings are two settings all the way to the scene: neither one
/// moves the other's picture, which is the whole of what splitting them buys
/// and the one thing a single number could not say.
#[test]
fn the_two_gaps_are_independent_at_the_scene() {
    let stack = |view: &ViewConfig| {
        let scene = scene_of(&NoteTracker::new(), &Tuning::default(), view, &plain_frame(), 0.0);
        (scene.outer_inner, scene.outer_outer, scene.rings_outer, scene.mark_inner)
    };
    let sound = ViewConfig {
        spectral_ring_width: 0.1,
        band_width: 0.15,
        ring_gap: 0.05,
        octave_gap: 0.05,
        mark_thickness: 0.1,
        ..ViewConfig::default()
    };

    // Widening the sectors' gap leaves every radius where it was: the slices
    // are cut out of rings the stack has already placed.
    let sliced = ViewConfig { octave_gap: 0.3, ..sound.clone() };
    assert_eq!(stack(&sliced), stack(&sound), "the angular gap moved a radius");

    // And widening the stack's leaves the sectors' cut where it was, at a
    // picture that has visibly moved: audio 0..0.1 | gap | band, so the band's
    // inner edge walks out with the padding it is a sum of.
    let spaced = ViewConfig { ring_gap: 0.12, ..sound.clone() };
    assert!(
        stack(&spaced).0 > stack(&sound).0,
        "the radial gap did not restack the rings: the band's inner edge is still at {}",
        stack(&sound).0,
    );
    let scene = scene_of(&NoteTracker::new(), &Tuning::default(), &spaced, &plain_frame(), 0.0);
    assert_eq!(scene.octave_gap, 0.05, "the radial gap moved the sectors' own");
}

#[test]
fn a_wild_radial_gap_is_clamped_and_the_stack_stops_at_the_ring_that_fits() {
    // The cap on the radial padding is no guarantee that the stack still fits —
    // at GAP_MAX the paddings alone are more than the quad, so the band's slot
    // ends past the node's edge and the layer comes out off.
    let wild = ViewConfig { ring_gap: 5.0, ..ViewConfig::default() };
    let scene = scene_of(&NoteTracker::new(), &Tuning::default(), &wild, &plain_frame(), 0.0);
    // What the refused slot comes out AS, which is the whole of how a layer
    // says it is not drawn: the empty pair. An inside-out pair (the slot's own
    // inner edge twice) or one clipped back to the quad edge are both things
    // the shader would go on drawing, at radii no bar reads out.
    assert_eq!((scene.outer_inner, scene.outer_outer), (0.0, 0.0));
    // And the stack stopped at the ring that DID fit rather than at the edge.
    assert!(
        scene.rings_outer > 0.0 && scene.rings_outer < 1.0,
        "the outermost ring is at {}, which is off the node",
        scene.rings_outer,
    );
    // The CEILING itself, read where the scene still carries it. Every ring
    // this padding stands off was refused, so the radii above say nothing about
    // how wide it is; the mark strip is the one slot a full stack still spends
    // it on, and the difference IS the gap the picture used.
    //
    // Worth its own assertion because the escape is silent and lands outside
    // the node: `mark_inner` is what the shader sizes every node's BILLBOARD on
    // (`node_rim`, then `quad_margin`), so an unclamped 5.0 here asks for a quad
    // some five node radii across, on every node in the window, with the marks
    // drawn nowhere a person can see them. Nothing in the two assertions above
    // moves when the clamp goes — the refused pair is still empty and the stack
    // still stops where the last drawn ring ended.
    // Measured against the node's own EDGE rather than against `rings_outer`.
    // A refusal leaves the last drawn ring wherever it happened to stop, while
    // the strip stands off the room that ran out — the edge — so the two are a
    // whole refused slot apart by design and the distance between them measures
    // the refusal rather than the padding. The edge plus one padding is the
    // real ceiling, and it is the one every billboard pays for.
    //
    // A tolerance rather than a bare `<=`, the radius being a sum: how far
    // `1.0 + gap` lands from itself is a matter of how large the cursor grew
    // getting there, not of the clamp.
    assert!(
        scene.mark_inner <= 1.0 + GAP_MAX + f32::EPSILON * 8.0,
        "the strip reached {}, past the {} a full stack can put it at",
        scene.mark_inner,
        1.0 + GAP_MAX,
    );
}

/// The mark strip only ever moves OUTWARD as the stack's start moves outward.
///
/// `Stack::full` exists to stop a refused layer's slot becoming "a gift to the
/// one outside it" — its own doc says so, and names the reappearing band it was
/// written against. The strip is seated OUTSIDE `Stack::take`, though, because
/// its slot is the one allowed to run past the quad, so the flag never reached
/// it: with the octave band refused for want of room, the cursor stays at the
/// audio ring's outer edge and the marks seat a gap out from there — in the
/// slot the band was just refused.
///
/// On the Layers bar that is a strip travelling the opposite way to the
/// pointer: one ten-thousandth more Inner takes the octave band away and drops
/// the melody/bass marks a fifth of a node INWARD, from outside the quad's
/// hairline to inside it, where they stay until the handle catches back up.
///
/// Swept rather than pinned at the crossing, because where the crossing falls
/// is a function of every other width in the stack and moves whenever a fresh
/// default does.
#[test]
fn the_mark_strip_never_falls_into_a_refused_layers_slot() {
    let fresh = ViewConfig::default();
    let mut previous = f32::NEG_INFINITY;
    let mut worst: Option<(f32, f32, f32)> = None;
    for step in 0..=9000 {
        // The Inner handle's whole travel, at a step fine enough to land either
        // side of a refusal rather than stepping over it.
        let ring_inner = step as f32 / 10_000.0;
        let mark = ViewConfig { ring_inner, ..fresh.clone() }.rings().mark_inner;
        // The strip's radius sizes every node's billboard, so "outward" must
        // not become "away": the whole travel stays under the node's edge plus
        // one padding, refusals included.
        assert!(
            mark <= 1.0 + GAP_MAX + f32::EPSILON * 8.0,
            "the strip reached {mark} at an Inner of {ring_inner}",
        );
        if mark < previous - 1e-6 && worst.is_none() {
            worst = Some((ring_inner, previous, mark));
        }
        previous = mark;
    }
    if let Some((at, before, after)) = worst {
        panic!(
            "the strip fell {} inward when Inner reached {at}: {before} -> {after}",
            before - after,
        );
    }
}

#[test]
fn the_rings_stack_outward_from_where_the_stack_starts() {
    // The whole of what the width bars mean, at the scene: the innermost ring
    // seats on the stack's own start and each ring after it starts a gap past
    // the one inside it, so the band's radii are a SUM rather than a pair of
    // settings, and nothing has to be dragged twice to keep the layers from
    // overlapping.
    let view = ViewConfig {
        ring_inner: 0.2,
        spectral_ring_width: 0.1,
        band_width: 0.15,
        ring_gap: 0.05,
        ..ViewConfig::default()
    };
    let scene = scene_of(&NoteTracker::new(), &Tuning::default(), &view, &plain_frame(), 0.0);
    // middle 0..0.2 | audio 0.2..0.3 | gap | band 0.35..0.50. To a tolerance,
    // the radii being sums: 0.2 + 0.1 + 0.05 is not 0.35 in binary.
    let close =
        |(a, b): (f32, f32), (c, d): (f32, f32)| (a - c).abs() < 1e-5 && (b - d).abs() < 1e-5;
    assert!(
        close((scene.outer_inner, scene.outer_outer), (0.35, 0.5)),
        "the band is at {}..{}, not a gap out from the audio ring",
        scene.outer_inner,
        scene.outer_outer,
    );
    assert!((scene.rings_outer - 0.5).abs() < 1e-5, "the marks stand off the band");

    // The audio ring off: its slot AND its gap go back to the band, which
    // slides in to the stack's start — no layer left to stand off, so no gap
    // spent standing off one.
    let view = ViewConfig { spectral_ring_width: 0.0, ..view };
    let scene = scene_of(&NoteTracker::new(), &Tuning::default(), &view, &plain_frame(), 0.0);
    assert!(
        close((scene.outer_inner, scene.outer_outer), (0.2, 0.35)),
        "the band did not slide in to the stack's start: {}..{}",
        scene.outer_inner,
        scene.outer_outer,
    );

    // The band off: an empty pair, which is what says the layer is not drawn,
    // and the marks fall back to standing off the ring that IS the outermost.
    let view = ViewConfig { spectral_ring_width: 0.1, band_width: 0.0, ..view };
    let scene = scene_of(&NoteTracker::new(), &Tuning::default(), &view, &plain_frame(), 0.0);
    assert_eq!((scene.outer_inner, scene.outer_outer), (0.0, 0.0));
    assert!((scene.rings_outer - 0.3).abs() < 1e-5, "the marks kept the band's slot");

    // And a start of 0 is the stack seated on the node's own centre, which is
    // the same rule read at the bottom of that bar rather than a case of its
    // own: the innermost ring reaches the centre and its sectors close into pie
    // wedges.
    let view = ViewConfig { ring_inner: 0.0, band_width: 0.15, ..view };
    let scene = scene_of(&NoteTracker::new(), &Tuning::default(), &view, &plain_frame(), 0.0);
    assert!(
        close((scene.outer_inner, scene.outer_outer), (0.15, 0.3)),
        "a stack seated on the centre put the band at {}..{}",
        scene.outer_inner,
        scene.outer_outer,
    );
}

/// A ring draws at exactly the width its bar reads, or it is not drawn at all:
/// a slot that would cross the quad edge is REFUSED rather than squeezed into
/// whatever room is left, and the layers outside it go with it.
///
/// The squeeze is what makes this worth a test, because it is invisible from
/// everywhere but the picture: the bar goes on reading the width somebody
/// dialled, the geometry holds a fraction of it, and at the far end that
/// fraction is a hairline at the node's rim — a band 0.0008 wide is a
/// twenty-fifth of a pixel on the 52-px node the DAW draws, where `glyph_band`'s
/// two soft edges overlap instead of cancelling and paint a faint ring around
/// nothing. Dropping the layer says what the room ran out at; clipping it says
/// the bar was a suggestion.
#[test]
fn a_layer_with_no_room_left_in_the_node_is_not_drawn() {
    // A stack whose inner layer and padding together leave the band no room at
    // all: the audio ring still fits whole, and the band's slot ends past the
    // quad edge.
    //
    // The whole stack is stated, the way the sound/silent fixtures above state
    // theirs. The audio ring is a THRESHOLD here — the one width at which the
    // band is refused and the ring is not — and where that threshold falls is a
    // function of every other width, so a ring inherited alongside a retuned
    // gap names a different state than the one under test. The alternative is
    // a number that has to be re-found by hand each time a dialled-in look is
    // captured.
    //
    // The START is what makes the corner reachable at the fresh padding: a ring
    // is held to RING_WIDTH_MAX, so a stack seated on the node's centre fits
    // every layer whatever the bars read, and it takes a stack pushed out — or
    // a padding dialled well up — to run the quad out.
    let view = ViewConfig {
        ring_inner: 0.3,
        spectral_ring_width: RING_WIDTH_MAX,
        band_width: 0.163_084_63,
        ring_gap: 0.05,
        octave_gap: 0.05,
        ..ViewConfig::default()
    };
    let rings = view.rings();
    assert!(rings.audio.1 > rings.audio.0, "the audio ring lost a slot that fits");
    assert_eq!(rings.band, (0.0, 0.0), "the band was squeezed into {:?}", rings.band);
    assert_eq!(rings.outer, rings.audio.1, "the marks stand off the ring that IS outermost");
    let scene = scene_of(&NoteTracker::new(), &Tuning::default(), &view, &plain_frame(), 0.0);
    assert_eq!((scene.outer_inner, scene.outer_outer), (0.0, 0.0));

    // Every pair the stack hands out, at every ring width the bar reaches: the
    // band is the width its own bar reads and sits inside the quad, or it is
    // off. There is no third answer, which is the whole of the rule.
    //
    // And the ORDER the answers come in, which the width alone cannot see: the
    // stack empties from the outside in, so a layer that has gone stays gone as
    // the ring inside it keeps widening. Both bars are open here, so the only
    // way to a `(0, 0)` is a refusal.
    let fresh = ViewConfig { ring_inner: 0.3, ring_gap: 0.05, ..ViewConfig::default() };
    let mut band_gone = None::<f32>;
    for step in 0..=60 {
        let width = step as f32 / 100.0;
        let rings = ViewConfig { spectral_ring_width: width, ..fresh.clone() }.rings();
        // Seated on the stack's start at exactly its own width — or the empty
        // pair at the bottom of its bar, where the layer is switched off rather
        // than refused and gives its slot back.
        let seated =
            if width > 0.0 { (fresh.ring_inner, fresh.ring_inner + width) } else { (0.0, 0.0) };
        assert!(
            (rings.audio.0 - seated.0).abs() < 1e-5 && (rings.audio.1 - seated.1).abs() < 1e-5,
            "a ring of {width} came out at {:?} rather than {seated:?}",
            rings.audio,
        );
        let (inner, outer) = rings.band;
        assert!(
            (inner, outer) == (0.0, 0.0)
                || (outer <= 1.0 && (outer - inner - fresh.band_width).abs() < 1e-5),
            "a ring of {width} drew the band at {inner}..{outer}, not {} wide",
            fresh.band_width,
        );
        match (rings.band != (0.0, 0.0), band_gone) {
            (false, None) => band_gone = Some(width),
            (true, Some(at)) => panic!(
                "the band went at a ring of {at} and is back at {width}; the stack empties \
                 from the outside in, so a layer that has gone stays gone",
            ),
            _ => {}
        }
    }
    assert!(band_gone.is_some(), "the sweep never ran the band out of room, so it proved nothing");
}

/// A size a hand-edited blob holds but no bar can produce reaches the scene as
/// the layer's own OFF position, and takes nothing else down with it.
///
/// A NaN is the case worth the test: it walks through a `clamp` untouched (it
/// is its own answer to every comparison), and one that reached the shader as a
/// radius would take the node's whole radial coverage to NaN — the layer, and
/// every layer measured off it, silently gone while the bars read out numbers.
///
/// All four sizes and BOTH paddings, because `size` guards each of them and
/// `sanitize` repairs only the audio ring's: any of the other five is a door a
/// non-finite reaches the picture through, and each has a different layer to
/// take down with it.
#[test]
fn a_hand_edited_size_reaches_the_scene_as_that_layers_off_position() {
    // Every layer on, at sizes far enough apart to read the whole stack off the
    // scene: middle 0..0.1 | audio 0.1..0.2 | gap | band 0.25..0.45 | marks. The
    // two paddings are one number here so a case that moves either is reading
    // against the same stack.
    let sound = ViewConfig {
        ring_inner: 0.1,
        spectral_ring_width: 0.1,
        band_width: 0.2,
        ring_gap: 0.05,
        octave_gap: 0.05,
        mark_thickness: 0.1,
        ..ViewConfig::default()
    };
    // The node the scene reads out: the band's two radii, the outermost ring
    // the marks stand off, how deep the marks are, and the two paddings.
    // The radial one is read as the stand-off it BOUGHT, that being the only
    // form it reaches the scene in — the scene carries no field for it.
    let stack = |view: &ViewConfig| {
        let scene = scene_of(&NoteTracker::new(), &Tuning::default(), view, &plain_frame(), 0.0);
        [
            scene.outer_inner,
            scene.outer_outer,
            scene.rings_outer,
            scene.mark_inner - scene.rings_outer,
            scene.mark_thickness,
            scene.octave_gap,
        ]
    };
    // To a tolerance, as every case below is: the radial padding is read as a
    // difference of two radii, so it carries whatever the sum that placed them
    // left behind — 0.75 - 0.7 is not 0.05 in binary.
    let close =
        |got: [f32; 6], want: [f32; 6]| got.iter().zip(want).all(|(a, b)| (a - b).abs() < 1e-5);
    assert!(
        close(stack(&sound), [0.25, 0.45, 0.45, 0.05, 0.1, 0.05]),
        "the sound stack is {:?}, not the one every case below is a departure from",
        stack(&sound),
    );

    for wild in [f32::NAN, f32::INFINITY, f32::NEG_INFINITY, -5.0] {
        for (field, view, want) in [
            (
                "stack start",
                ViewConfig { ring_inner: wild, ..sound.clone() },
                // The whole stack seated on the node's own centre, which is
                // this bar's own bottom rather than anything being switched
                // off, and every layer still the width its own bar reads.
                [0.15, 0.35, 0.35, 0.05, 0.1, 0.05],
            ),
            (
                "audio ring width",
                ViewConfig { spectral_ring_width: wild, ..sound.clone() },
                // The ring off, and the band closes over its slot AND its gap
                // rather than being carried off by it — in to the stack's own
                // start, there being no layer left to stand off.
                [0.1, 0.3, 0.3, 0.05, 0.1, 0.05],
            ),
            (
                "band width",
                ViewConfig { band_width: wild, ..sound.clone() },
                // The octave layer off — the empty pair — and the marks fall
                // back to standing off the audio ring.
                [0.0, 0.0, 0.2, 0.05, 0.1, 0.05],
            ),
            (
                "ring gap",
                ViewConfig { ring_gap: wild, ..sound.clone() },
                // The stack closes up: every layer meets the one inside it and
                // the marks seat against the band. The sectors are still cut,
                // that being the other bar's to say.
                [0.2, 0.4, 0.4, 0.0, 0.1, 0.05],
            ),
            (
                "octave gap",
                ViewConfig { octave_gap: wild, ..sound.clone() },
                // The sectors close into a solid annulus, and not one radius
                // moves — the layer is still exactly where the stack put it.
                [0.25, 0.45, 0.45, 0.05, 0.1, 0.0],
            ),
            (
                "mark depth",
                ViewConfig { mark_thickness: wild, ..sound.clone() },
                // The marks off, and the rings they stand off untouched.
                [0.25, 0.45, 0.45, 0.05, 0.0, 0.05],
            ),
        ] {
            let got = stack(&view);
            assert!(
                close(got, want),
                "a {field} of {wild} reached the scene as {got:?}, not {want:?}",
            );
        }
    }
}

#[test]
fn the_ring_geometry_is_sanitized_into_the_scene() {
    // A stack dialled past the quad's own edge: each width is held to the bar's
    // own top, and the layer the quad has no room left for is dropped rather
    // than handed to the shader as a radius it would draw off the node.
    let view = ViewConfig {
        ring_inner: 0.0,
        spectral_ring_width: 0.8,
        band_width: 0.9,
        ..ViewConfig::default()
    };
    let scene = scene_of(&NoteTracker::new(), &Tuning::default(), &view, &plain_frame(), 0.0);
    // The audio ring reaches the center and takes its whole clamped width; the
    // band's slot starts a gap past that and ends outside the quad, so the
    // layer is off.
    assert_eq!(scene.rings_outer, RING_WIDTH_MAX, "the audio ring is not its bar's full width");
    assert_eq!((scene.outer_inner, scene.outer_outer), (0.0, 0.0));

    // The audio ring off and the gap closed: the band takes the centre itself,
    // at exactly its bar's clamped width.
    let view = ViewConfig { spectral_ring_width: 0.0, band_width: 0.9, ring_gap: 0.0, ..view };
    let scene = scene_of(&NoteTracker::new(), &Tuning::default(), &view, &plain_frame(), 0.0);
    assert_eq!((scene.outer_inner, scene.outer_outer), (0.0, RING_WIDTH_MAX));
}

#[test]
fn off_sheet_nodes_shrink_away_from_the_home_sheet_both_ways() {
    // Size says DISTANCE from the home sheet, not depth toward the eye: a
    // sheet in front shrinks exactly as much as one behind. The home sheet
    // is the ground the music is read against and stays full size.
    let view = ViewConfig { extent_sevens: 2, sevens_size: 0.5, ..ViewConfig::default() };
    let scene = scene_of(&NoteTracker::new(), &Tuning::default(), &view, &plain_frame(), 0.0);
    assert_eq!(node_at(&scene, LatticePos::new(0, 0, 0)).scale, 1.0);
    for sevens in [-1, 1] {
        assert_eq!(node_at(&scene, LatticePos::new(0, 0, sevens)).scale, 0.5);
    }
    for sevens in [-2, 2] {
        assert_eq!(node_at(&scene, LatticePos::new(0, 0, sevens)).scale, 0.25);
    }
}

#[test]
fn sevens_size_never_enlarges_and_never_vanishes() {
    // The axis only ever makes off-sheet nodes SMALLER (a value above 1
    // would put the sevens layer in front of the picture it annotates), and
    // never small enough to disappear at the far extents.
    let scene_with = |size: f32| {
        let view = ViewConfig { extent_sevens: 4, sevens_size: size, ..ViewConfig::default() };
        scene_of(&NoteTracker::new(), &Tuning::default(), &view, &plain_frame(), 0.0)
    };
    let huge = scene_with(4.0);
    assert_eq!(node_at(&huge, LatticePos::new(0, 0, 4)).scale, 1.0, "clamped to no growth");
    let tiny = scene_with(0.0);
    assert!(
        node_at(&tiny, LatticePos::new(0, 0, 4)).scale > 0.0001,
        "the farthest sheet still draws something"
    );
}

#[test]
fn every_sounding_node_clears_what_is_behind_it_the_home_sheet_included() {
    // The knockout is not an off-sheet ornament, it is how one sheet hides
    // another — so the home sheet needs one too, or the sheets behind it
    // show straight through the gaps in a home node's body (thin gapped
    // annuli cover very little) and neither sheet
    // reads as being in front. Which layer the clearing is DRAWN in differs
    // — the home sheet's goes ahead of the markers, see the renderer — but
    // that is not this layer's business.
    let view = ViewConfig { extent_sevens: 1, sevens_gutter: 0.2, ..plain_view() };
    let scene = scene_of(&held(60), &Tuning::default(), &view, &plain_frame(), 0.0);

    // C sounds, so every node whose pitch class is C lights — on the home
    // sheet and off it — and all of them clear.
    //
    // The WIDTH is what this field is, and it is the view's for every node: the
    // strength is per LAYER and belongs to the shader, which scales each
    // layer's hole by the level that paints that layer (`node_clearing`). So a
    // silent node carrying a width here punches nothing THERE unless some layer
    // of it is drawing — and one is, when the audio ring's Gate has let it wear
    // a ring with no note under it, which is exactly the node this cannot gate
    // on `activation` for. `a_node_wearing_only_an_audio_ring_clears_around_it`
    // is that case in pixels, and it is a render test because the whole answer
    // is in the shader.
    let mut lit_home = 0;
    let mut lit_off = 0;
    for node in &scene.nodes {
        assert_eq!(node.gutter, 0.2, "the reach is the view's, on every node it ships");
        if node.activation > 0.0 {
            if node.on_home {
                lit_home += 1;
            } else {
                lit_off += 1;
            }
        }
    }
    assert!(lit_home > 0 && lit_off > 0, "the case needs both kinds lit");
}

#[test]
fn a_flat_lattice_still_clears_its_markers() {
    // With the sevenths extent at 0 there is no sheet behind anything, but
    // the clearing is not only an inter-sheet device: it cuts the marker
    // field, so a sounding node sits in a clean gap in the lattice rather
    // than on top of it. That reading is wanted at any depth, so the home
    // sheet clears on a flat lattice exactly as it does on a deep one.
    // Gating it on the extent would make the look reachable only by growing
    // depth the view doesn't want.
    let view = ViewConfig { extent_sevens: 0, sevens_gutter: 0.2, ..plain_view() };
    let scene = scene_of(&held(60), &Tuning::default(), &view, &plain_frame(), 0.0);
    let mut lit = 0;
    for node in &scene.nodes {
        // The view's reach, on every node — see
        // `every_sounding_node_clears_what_is_behind_it_the_home_sheet_included`
        // for why this is not gated on the note here.
        assert_eq!(node.gutter, 0.2, "a home node carries the reach with no depth at all");
        lit += usize::from(node.activation > 0.0);
    }
    assert!(lit > 0, "something is lit");
}

#[test]
fn a_releasing_note_keeps_its_gutters_width() {
    // `gutter` is the clearing's WIDTH, and it must not follow the
    // envelope: the shader fades the clearing's STRENGTH by the same
    // `activation` it paints the node with, and doing both would shrink
    // the hole as it faded.
    //
    // The regression this pins is the other way round, and much worse.
    // Scaling the width alone (the first cut) leaves the clearing fully
    // opaque for the entire release and merely hardens its soft edge — so
    // the hole sits there at full strength while its note fades away under
    // it, then disappears the instant the voice is pruned. It reads as a
    // pop, which is exactly what it is.
    let view = ViewConfig { extent_sevens: 1, sevens_gutter: 0.2, ..ViewConfig::default() };
    let frame = FrameParams { fade_time: 1.0, ..FrameParams::default() };
    let tuning = Tuning::default();
    let mut tracker = held(60);
    // Released a whole duration in, so the note is leaving rather than still
    // arriving at the two samples below: the departure waits out the arrival
    // (`Voice::release_level`).
    tracker.handle_event(NoteEvent::off(1.0, 0, 60));

    let off_sheet = |now: f64| {
        let scene = scene_of(&tracker, &tuning, &view, &frame, now);
        *scene
            .nodes
            .iter()
            .find(|n| n.activation > 0.0 && n.lattice_pos.sevens != 0)
            .expect("a lit off-sheet node")
    };
    // A quarter and three quarters of the way through the release: the note
    // is measurably dimmer, and the clearing is exactly as wide.
    let early = off_sheet(1.25);
    let late = off_sheet(1.75);
    assert!(late.activation < early.activation, "the note really is fading");
    assert_eq!(early.gutter, 0.2);
    assert_eq!(late.gutter, 0.2, "the clearing holds its width through the fade");
}

#[test]
fn the_comma_measures_the_node_against_its_own_namesake() {
    // `note_name` walks the fifths with `threes + fives*4 - sevens*2`, so a
    // sevens step lands on the LETTER two fifths down. The comma is the
    // distance to that node — the septimal comma, 64/63, ~27 cents at just
    // intonation.
    let view = ViewConfig { extent_sevens: 1, ..ViewConfig::default() };
    let tuning = Tuning::just();
    let scene = scene_of(&NoteTracker::new(), &tuning, &view, &plain_frame(), 0.0);

    let seventh = LatticePos::new(0, 0, 1);
    let namesake = LatticePos::new(-2, 0, 0);
    // The premise: the two share a letter and an accidental, which is what
    // makes one the other's namesake. They are no longer the same NAME —
    // the septimal mark is what tells them apart, and this comma is the
    // distance that mark stands for.
    let (a, b) = (seventh.note_name(), namesake.note_name());
    assert_eq!((a.letter, a.accidental_mark()), (b.letter, b.accidental_mark()));
    assert_ne!(a.to_string(), b.to_string());
    let comma = node_at(&scene, seventh).comma;
    assert!((comma - -27.26).abs() < 0.05, "7/4 sits a septimal comma below 16/9, got {comma}");
    // The other direction is the same distance the other way, and the home
    // sheet has no namesake to measure against.
    let below = node_at(&scene, LatticePos::new(0, 0, -1)).comma;
    assert!((below - 27.26).abs() < 0.05, "got {below}");
    assert_eq!(node_at(&scene, LatticePos::ORIGIN).comma, 0.0);
}

#[test]
fn the_comma_takes_the_short_way_round_the_octave() {
    // Pitch classes wrap, so a raw subtraction can come out an octave off
    // and report a 1173-cent "comma". Two sevens steps land far enough
    // round the circle to catch it.
    let view = ViewConfig { extent_sevens: 3, ..ViewConfig::default() };
    let scene = scene_of(&NoteTracker::new(), &Tuning::just(), &view, &plain_frame(), 0.0);
    for sevens in [-3, -2, -1, 1, 2, 3] {
        let comma = node_at(&scene, LatticePos::new(0, 0, sevens)).comma;
        assert!(comma.abs() <= 600.0, "sevens {sevens}: comma {comma} is the long way round");
    }
}

#[test]
fn the_knockout_clears_to_the_ground_not_to_black() {
    // The gutter has no color of its own, so a premultiplied layer would
    // knock out to BLACK — and the well the lattice pane stands on is lighter
    // than black, which is what makes a cleared disc read as a dark plate
    // sitting on the picture instead of a hole through it. The scene
    // therefore carries the ground, and it must be the well the pane paints,
    // not zero.
    let scene = scene_of(
        &NoteTracker::new(),
        &Tuning::default(),
        &ViewConfig::default(),
        &plain_frame(),
        0.0,
    );
    let well = crate::skin::active_skin().well;
    assert_eq!(scene.background, crate::skin::ground_color((well[0], well[1], well[2])));
    assert!(scene.background.truncate().length() > 0.0, "not black");
    assert_eq!(scene.background.w, 1.0, "opaque, or it would not cover");
}

#[test]
fn ground_color_keeps_srgb_bytes_as_they_are() {
    // A straight divide by 255, NOT a gamma decode: every color the shader
    // handles is sRGB-encoded 0..1 (the offscreen target is a plain Unorm
    // format), so decoding here would clear the gutter to a ground far
    // darker than the pane it is supposed to disappear into.
    let c = crate::skin::ground_color((24, 25, 29));
    assert!((c.x - 24.0 / 255.0).abs() < 1e-6);
    assert!((c.y - 25.0 / 255.0).abs() < 1e-6);
    assert!((c.z - 29.0 / 255.0).abs() < 1e-6);
}
