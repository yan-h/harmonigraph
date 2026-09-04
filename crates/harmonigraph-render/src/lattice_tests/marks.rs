//! The melody and bass marks, and the audio ring worn under them.

use super::fixtures::*;
use crate::*;

#[test]
fn a_melody_bass_mark_extends_the_slice_it_names() {
    const SIZE: [u32; 2] = [256, 256];
    let Some(mut gpu) = Shooter::new(SIZE) else {
        return;
    };

    let unmarked = gpu.shot(&single_marked_node(0, 0));
    let changed_px = |other: &[u8]| -> usize {
        unmarked.chunks(4).zip(other.chunks(4)).filter(|(a, b)| a != b).count()
    };

    // Measure against the node's OWN footprint, not an absolute pixel
    // count: what matters is that the mark claims a real share of the
    // thing it is marking, at whatever size it happens to be drawn.
    let node_px = unmarked.chunks(4).filter(|px| px[..3] != [0, 0, 0]).count();
    let melody = gpu.shot(&single_marked_node(MIDDLE_C, 0));
    let melody_px = changed_px(&melody);
    eprintln!("node {node_px} px; mark {melody_px}");
    // A floor, not a target, measured against the node's whole lit
    // footprint (glow included). A mark is ONE octave's slice continued
    // outward, so it claims a wedge rather than a ring — a fifth of the turn
    // on the fresh five-octave wheel. What it catches is a strip that draws as
    // nothing at all, which is a hairline in the DAW: well under 1% against
    // the 15% the fresh thickness comes to here.
    assert!(
        melody_px * 32 > node_px,
        "the mark covers too little of the node to find: \
         {melody_px} px of {node_px}"
    );

    // Nothing marked draws no mark at all.
    let off = gpu.shot(&single_marked_node(0, 0));
    assert_eq!(changed_px(&off), 0, "an unmarked node must draw no mark");

    // A note claimed by BOTH ends on ONE octave -- a lone held note, or a
    // chord whose top and bottom share a pitch class -- must not be blanked:
    // that vanishes the mark exactly when two things are true at once. The
    // two name one slice, so what draws is that slice extended ONCE, over
    // exactly the pixels either end alone would have covered.
    let shared = gpu.shot(&single_marked_node(MIDDLE_C, MIDDLE_C));
    let shared_px = changed_px(&shared);
    eprintln!("both ends on one slice: {shared_px} px against {melody_px}");
    assert_eq!(
        shared_px, melody_px,
        "both ends on one octave drew a different shape from one end alone",
    );

    // Both ends on DIFFERENT octaves is the case only the shape can say: two
    // slices, each extended, so the picture covers more than either end alone
    // and matches neither.
    let beside = slot_beside_middle_c();
    let apart = gpu.shot(&single_marked_node(MIDDLE_C, beside));
    let apart_px = changed_px(&apart);
    let bass_only = gpu.shot(&single_marked_node(0, beside));
    eprintln!("two slices marked: {apart_px} px");
    assert!(
        apart_px > melody_px && apart_px > changed_px(&bass_only),
        "two marked octaves drew no more than one: {apart_px} px",
    );
    assert!(
        differing_pixels(&apart, &melody) > 0 && differing_pixels(&apart, &bass_only) > 0,
        "a two-octave mark is indistinguishable from a single-ended one",
    );
}

/// The audio ring reads the spectrum AROUND each octave: it draws inside the
/// octave band on the band's own angles, a partial dead on the node paints
/// down the middle of the wedge, and a detuned one paints off-centre in the
/// direction pitch rises — further off the narrower the Range is dialled.
///
/// Pixels rather than a reading of the shader's arithmetic, because every
/// claim here is geometric. Both rings walk `oct_sector` off one `OctRing`,
/// and the failures this catches all compile, validate, and read as a picture
/// that is subtly lying: a second ring drawn on its own idea of where a slot
/// is, a pitch window mapped backwards across the wedge, a Range that scales
/// the wrong way.
#[test]
fn the_audio_ring_reads_the_spectrum_around_each_octave() {
    const SIZE: [u32; 2] = [256, 256];
    let Some(mut gpu) = Shooter::new(SIZE) else {
        return;
    };
    // Octaves the fresh wheel draws on a C node — five slices centered on
    // middle C, so slots 3..=7. Middle C's own, the one above it (a known
    // rise in pitch, which is what calibrates the shot's handedness), and one
    // two below, far enough that a wedge at 72 degrees an octave is well clear
    // of any fringe.
    const UP: usize = harmonigraph_scene::MIDDLE_C_SLOT;
    const OVER: usize = harmonigraph_scene::MIDDLE_C_SLOT + 1;
    const DOWN: usize = harmonigraph_scene::MIDDLE_C_SLOT - 2;
    // The fixture's node is a C (`cents` 0), so slot s names MIDI 12 * s.
    let slot_pitch = |slot: usize| slot as f32 * 12.0;
    // The probe Range, which every angle below is measured at unless it says
    // otherwise.
    let fresh_range = PROBE_RANGE;

    let base = gpu.shot(&ringing_node(None, None, fresh_range));
    let mut wedge = |held, sounding, range| {
        let shot = gpu.shot(&ringing_node(held, sounding, range));
        light_about_center(&light_over(&shot, &base), SIZE)
    };

    let band = wedge(Some(UP), None, fresh_range);
    let ring = wedge(None, Some(slot_pitch(UP)), fresh_range);
    assert!(ring.weight > 0.0, "the audio ring drew nothing at all");
    assert!(band.weight > 0.0, "the octave band drew nothing, so there is nothing to compare");
    eprintln!(
        "band {:.1}..{:.1} px at {:.1}°; ring {:.1}..{:.1} px at {:.1}°",
        band.near,
        band.far,
        band.angle.to_degrees(),
        ring.near,
        ring.far,
        ring.angle.to_degrees(),
    );
    // Inside, and clear of it: the ring's outermost lit pixel is nearer the
    // center than the band's innermost. A gap of at least a couple of pixels
    // at this size, so a ring that merely failed to overlap by a fraction of
    // one does not read as the design's "visible gap either side".
    assert!(
        ring.far + 2.0 < band.near,
        "the ring reaches {:.1} px against a band starting at {:.1}",
        ring.far,
        band.near,
    );
    // A partial exactly on the octave stands where that octave's own wedge
    // stands: the middle of it, which is the wheel's rule that an angle means
    // an absolute pitch, holding across both rings.
    let apart = angle_apart(ring.angle, band.angle);
    assert!(apart < 6.0, "a partial on the octave sits {apart:.1}° off the wedge that names it");

    // A different octave is a different angle in both — or the check above
    // would pass just as well for a ring pinned to one place on the node.
    let band_down = wedge(Some(DOWN), None, fresh_range);
    let ring_down = wedge(None, Some(slot_pitch(DOWN)), fresh_range);
    let moved = angle_apart(ring_down.angle, ring.angle);
    assert!(moved > 60.0, "two octaves apart moved the ring's wedge only {moved:.1}°");
    let apart = angle_apart(ring_down.angle, band_down.angle);
    assert!(apart < 6.0, "the lower octave sits {apart:.1}° off the wedge that names it");

    // Which way is UP on this shot, taken from the band itself: an octave
    // higher is a known rise in pitch, and the wheel turns clockwise with it.
    let rising = signed_apart(wedge(Some(OVER), None, fresh_range).angle, band.angle);
    assert!(
        rising.abs() > 30.0,
        "an octave moved the band only {rising:.1}°, so it cannot calibrate a direction",
    );

    // A partial a QUARTER of the window sharp lands a quarter of the wedge
    // clockwise of centre — the whole of what the segment is for, and the
    // reading a folded number per octave cannot give.
    let sharp = fresh_range / 4.0;
    let detuned = wedge(None, Some(slot_pitch(UP) + sharp / 100.0), fresh_range);
    let shift = signed_apart(detuned.angle, ring.angle);
    eprintln!("{sharp:.0}¢ sharp moved the wedge {shift:.1}°, an octave of band {rising:.1}°",);
    assert!(
        shift * rising > 0.0,
        "{sharp:.0}¢ SHARP moved the wedge {shift:.1}° where rising pitch moves it \
         {rising:.1}°: the pitch window is mapped backwards across the wedge",
    );
    // A quarter of the window across a 72° wedge is 18°, and the lit arc is
    // 80¢ of a 200¢ window wide, so its centroid moves with its centre. Well
    // inside the wedge either way — a shift that ran off the end would clamp
    // and read as a smaller one, which is the other way this can fail.
    assert!(
        (shift.abs() - 18.0).abs() < 5.0,
        "a quarter-window detune moved the wedge {:.1}°, not the 18° a quarter of it is",
        shift.abs(),
    );

    // ...and the Range is a ZOOM: the same detuning, read over twice the
    // window, moves the wedge half as far.
    let wide = wedge(None, Some(slot_pitch(UP) + sharp / 100.0), fresh_range * 2.0);
    let wide_shift = signed_apart(wide.angle, ring.angle);
    eprintln!("the same {sharp:.0}¢ over twice the Range moved it {wide_shift:.1}°");
    assert!(
        (wide_shift.abs() * 2.0 - shift.abs()).abs() < 5.0,
        "twice the Range moved the same detune {:.1}° against {:.1}° at the fresh one, \
         which is not half",
        wide_shift.abs(),
        shift.abs(),
    );

    // The ring OFF draws nothing, whatever the grid holds: the empty annulus
    // is how the toggle reaches the shader, so this is the "exactly today's
    // picture" claim in its smallest form.
    let mut off = ringing_node(None, Some(slot_pitch(UP)), fresh_range);
    off.spectral.inner = 0.0;
    off.spectral.outer = 0.0;
    let quiet = {
        let mut quiet = ringing_node(None, None, fresh_range);
        quiet.spectral.inner = 0.0;
        quiet.spectral.outer = 0.0;
        gpu.shot(&quiet)
    };
    assert_eq!(
        differing_pixels(&gpu.shot(&off), &quiet),
        0,
        "a sounding partial drew something with the ring switched off",
    );
}

/// A node the gate holds back draws exactly the picture it would draw with the
/// ring layer OFF: the annulus goes, and the octave band, the marks and the
/// node's own body stay pixel for pixel.
///
/// Two claims in one comparison, and the second is the one worth the GPU. That
/// the gate removes the ring is arithmetic anyone can read off the shader; that
/// it removes NOTHING ELSE is a property of where the test sits in the fragment
/// program, and the ways it can fail all draw a plausible picture — a gate
/// applied before the wedge walk instead of inside it, or one that fell through
/// to the layer under it, would take the band's ghost or the glyph's edge with
/// it and read as a node that changed shape when the music went quiet.
#[test]
fn a_gated_node_loses_its_ring_and_nothing_else() {
    const SIZE: [u32; 2] = [256, 256];
    let Some(mut gpu) = Shooter::new(SIZE) else {
        return;
    };
    let fresh_range = PROBE_RANGE;
    // A node with an octave held and a partial sounding at that same octave, so
    // both rings have something to draw and the two can be told apart in the
    // shot.
    let slot = harmonigraph_scene::MIDDLE_C_SLOT;
    let sounding = slot as f32 * 12.0;
    let lit = ringing_node(Some(slot), Some(sounding), fresh_range);
    let ringing = gpu.shot(&lit);

    // The same node held back by the gate...
    let mut gated = ringing_node(Some(slot), Some(sounding), fresh_range);
    gated.nodes[0].audio_ring = 0.0;
    // ...against the same node with the LAYER off, which is the picture a gated
    // node has to come out as.
    let mut layer_off = ringing_node(Some(slot), Some(sounding), fresh_range);
    layer_off.spectral.inner = 0.0;
    layer_off.spectral.outer = 0.0;

    let dark = gpu.shot(&gated);
    assert!(
        differing_pixels(&ringing, &dark) > 0,
        "the ungated node drew no ring, so there is nothing for the gate to take",
    );
    assert_eq!(
        differing_pixels(&dark, &gpu.shot(&layer_off)),
        0,
        "a gated node is not the picture the ring layer being off draws",
    );
    // And the ring is what went: the light that differs sits in the audio
    // ring's own annulus, well inside the band. (`light_over` is the ungated
    // shot less the gated one, so what it holds is exactly the ring.)
    let ring = light_about_center(&light_over(&ringing, &dark), SIZE);
    let bare = gpu.shot(&ringing_node(None, None, fresh_range));
    let band = light_about_center(&light_over(&ringing, &bare), SIZE);
    assert!(ring.weight > 0.0, "nothing at all was taken away");
    assert!(
        ring.far + 2.0 < band.far,
        "what the gate took reaches {:.1} px, past the node's own band at {:.1}",
        ring.far,
        band.far,
    );
}

/// A ring part way through its fade is the ring drawn OVER the picture without
/// it, at a fraction of its coverage — every pixel of the node between the two
/// pictures the ends of the fade draw, and no pixel outside the annulus moved.
///
/// What the fade has to be if it is to read as a ring arriving rather than as
/// the node changing: the level scales the RING's coverage, so what shows
/// through is the octave layer under it. The ways it can fail all draw a
/// plausible picture and none of them is this — a level mixed into the wedge's
/// COLOUR would draw a reading of a quieter spectrum, and one applied to the
/// composite would fade the band and the marks with it.
///
/// A quarter and not a half, so that a shot which merely picked one END of the
/// fade cannot pass by landing between the two.
#[test]
fn a_ring_part_way_through_its_fade_sits_between_the_two() {
    const SIZE: [u32; 2] = [256, 256];
    let Some(mut gpu) = Shooter::new(SIZE) else {
        return;
    };
    let fresh_range = PROBE_RANGE;
    let slot = harmonigraph_scene::MIDDLE_C_SLOT;
    let sounding = slot as f32 * 12.0;
    let full = gpu.shot(&ringing_node(Some(slot), Some(sounding), fresh_range));

    let mut none = ringing_node(Some(slot), Some(sounding), fresh_range);
    none.nodes[0].audio_ring = 0.0;
    let none = gpu.shot(&none);

    let mut part = ringing_node(Some(slot), Some(sounding), fresh_range);
    part.nodes[0].audio_ring = 0.25;
    let part = gpu.shot(&part);

    assert!(differing_pixels(&full, &none) > 0, "the ring drew nothing to fade");
    assert!(differing_pixels(&part, &none) > 0, "a quarter of a ring drew nothing at all");
    assert!(differing_pixels(&part, &full) > 0, "a quarter of a ring is the whole of one");

    // Between the two, channel by channel. The slack is the packed level and
    // the final target's own dither/rounding, not a tolerance on the claim: a
    // level that reached the colour instead would leave the wedges the same
    // coverage and paint them a different colour, which lands outside the pair
    // wherever the ramp is not monotone in the channel.
    let mut moved = 0;
    for ((p, a), b) in part.chunks(4).zip(full.chunks(4)).zip(none.chunks(4)) {
        for c in 0..3 {
            let (low, high) = (a[c].min(b[c]), a[c].max(b[c]));
            assert!(
                i32::from(p[c]) >= i32::from(low) - 2 && i32::from(p[c]) <= i32::from(high) + 2,
                "a quarter-faded pixel reads {} where the ends read {low} and {high}",
                p[c],
            );
        }
        if a != b {
            moved += 1;
        }
    }
    assert!(moved > 0, "the two ends of the fade drew one picture");
}

/// A melody/bass mark stands off the OUTERMOST RING the node draws, which on a
/// node with no octave band is the audio ring rather than the node's center.
///
/// The mark's inner edge is the one radius the shader is handed rather than
/// deriving. Deriving it from the BAND's outer edge is the same answer whenever
/// a band draws and the wrong one the moment that layer's width bar reaches 0:
/// the strip jumps inward across the whole node and lands against the core,
/// marking a slice of nothing.
///
/// Measured off the picture and not the uniform, because the two ways this
/// fails — the wrong radius packed, or the shader reading a different slot —
/// look identical from the Rust side.
#[test]
fn a_mark_stands_off_the_outermost_ring_the_node_draws() {
    const SIZE: [u32; 2] = [256, 256];
    let Some(mut gpu) = Shooter::new(SIZE) else {
        return;
    };

    // The probe's stack, so the layers are pixels apart: audio ring, band, and
    // the mark outside both. The claim is that the mark finds whichever ring is
    // outermost, which wants a stack that draws all of them rather than the one
    // the fresh view happens to be dialled to.
    let fresh = harmonigraph_scene::ViewConfig::default();
    let rings = harmonigraph_scene::ViewConfig {
        ring_inner: PROBE_INNER,
        ring_gap: PROBE_GAP,
        spectral_ring_width: PROBE_RING_WIDTH,
        band_width: PROBE_BAND_WIDTH,
        ..fresh.clone()
    }
    .rings();
    let staged = |band: bool| -> Scene {
        let mut scene = single_marked_node(MIDDLE_C, 0);
        scene.octave_gap = PROBE_GAP;
        scene.mark_thickness = rings.mark_thickness;
        // The audio ring is drawn from an all-zero grid, which paints the
        // ramp's floor colour across the annulus — light at a known radius,
        // which is all this needs of it.
        let mut paint = harmonigraph_scene::SpectralPaint::silent();
        paint.lut = std::array::from_fn(|_| glam::Vec4::new(1.0, 1.0, 1.0, 1.0));
        (paint.inner, paint.outer) = rings.audio;
        scene.spectral = paint;
        (scene.outer_inner, scene.outer_outer) = if band { rings.band } else { (0.0, 0.0) };
        // A ring is on either way here, so the strip is owed its padding in
        // both — the case where it is not is
        // `a_mark_with_no_ring_under_it_reaches_the_nodes_centre`.
        scene.rings_outer = if band { rings.band.1 } else { rings.audio.1 };
        scene.mark_inner = scene.rings_outer + rings.gap;
        scene
    };

    // The mark alone, over the same node with the marks off: what is left is
    // the strip, wherever it landed.
    let mark_light = |gpu: &mut Shooter, band: bool| -> Light {
        let mut bare = staged(band);
        bare.nodes[0].melody_slots = 0;
        bare.nodes[0].melody_level = 0.0;
        let bare = gpu.shot(&bare);
        light_about_center(&light_over(&gpu.shot(&staged(band)), &bare), SIZE)
    };

    let with_band = mark_light(&mut gpu, true);
    let without = mark_light(&mut gpu, false);
    assert!(with_band.weight > 0.0 && without.weight > 0.0, "the mark drew nothing to measure");
    eprintln!(
        "mark {:.1}..{:.1} px with the band, {:.1}..{:.1} px without",
        with_band.near, with_band.far, without.near, without.far,
    );
    // The band is the wider stack, so its mark is further out — and by about
    // the band's own width, which is the slot the layer gave back.
    let band_px = with_band.near - without.near;
    let scale = with_band.far / (rings.band.1 + rings.gap + rings.mark_thickness) as f64;
    let want = (rings.band.1 - rings.audio.1) as f64 * scale;
    assert!(
        (band_px - want).abs() < 4.0,
        "dropping the band moved the mark in {band_px:.1} px, not the {want:.1} px \
         of band and gap it gave back",
    );
    // And the mark did NOT fall back to the node's center, which is what
    // anchoring it to a band that is not there would do.
    assert!(
        without.near > rings.audio.1 as f64 * scale - 4.0,
        "with the band off the mark starts at {:.1} px, inside the audio ring's own edge",
        without.near,
    );
}

/// With the core, the audio ring and the octave band ALL dialled off, the
/// melody/bass mark is the only layer the node has left — and it reaches the
/// node's CENTRE, rather than standing a padding off nothing.
///
/// The stack ([`ViewConfig::rings`](harmonigraph_scene::ViewConfig::rings))
/// writes that rule down for every layer it owns: the gap is skipped at a
/// cursor of 0, where there
/// is nothing to stand off, so the innermost layer closes into a disc instead
/// of opening a hole the size of a padding around nothing. The mark is the one
/// layer it does NOT own — the strip's inner edge is re-derived in WGSL off
/// `rings_outer`, which is handed the cursor and not the rule — so the two
/// answers part company at exactly the one cursor the rule is about.
///
/// The state is a reduction the Lattice page's own Layers bar reaches: the
/// core, the audio ring and the octave band all have 0 as their off position,
/// which is their handle dragged home, and reading the lattice as melody/bass
/// marks alone is what taking all three there is for.
/// Every other fixture in this file leaves a ring under the mark, where the
/// gap is owed and both answers agree.
#[test]
fn a_mark_with_no_ring_under_it_reaches_the_nodes_centre() {
    const SIZE: [u32; 2] = [256, 256];
    let Some(mut gpu) = Shooter::new(SIZE) else {
        return;
    };

    // Two stacks off one view at the probe's wide padding: the band alone, and
    // nothing at all.
    //
    // The strip is dialled to its deepest on purpose. A sector's gap is a
    // constant EUCLIDEAN thickness at every radius (`outer_glyph`), so the two
    // edge lines blank a disc of half a padding about the node's centre — and
    // a strip no deeper than that disc would have nothing left to measure once
    // it reached the centre, which is the very state under test.
    let fresh = harmonigraph_scene::ViewConfig {
        ring_inner: PROBE_INNER,
        ring_gap: PROBE_GAP,
        spectral_ring_width: 0.0,
        mark_thickness: harmonigraph_scene::MARK_THICKNESS_MAX,
        ..harmonigraph_scene::ViewConfig::default()
    };
    let band_only = fresh.rings();
    let empty = harmonigraph_scene::ViewConfig { band_width: 0.0, ..fresh.clone() }.rings();
    assert!(band_only.outer > 0.0, "the reference stack must draw a ring");
    assert_eq!(empty.outer, 0.0, "the fixture must empty the stack to test anything");

    let staged = |rings: &harmonigraph_scene::RingStack, mark: bool| -> Scene {
        let mut scene = single_marked_node(MIDDLE_C, 0);
        scene.octave_gap = PROBE_GAP;
        scene.mark_thickness = rings.mark_thickness;
        // Silent paint carries the empty pair, so the audio ring is off the
        // way the bar leaves it rather than merely unlit.
        scene.spectral = harmonigraph_scene::SpectralPaint::silent();
        (scene.outer_inner, scene.outer_outer) = rings.band;
        scene.rings_outer = rings.outer;
        scene.mark_inner = rings.mark_inner;
        if !mark {
            scene.nodes[0].melody_slots = 0;
            scene.nodes[0].melody_level = 0.0;
        }
        scene
    };

    // The mark alone, read off the same node with the marks off, so what the
    // difference holds is the strip and nothing under it.
    let mark_light = |gpu: &mut Shooter, rings: &harmonigraph_scene::RingStack| -> Light {
        let bare = gpu.shot(&staged(rings, false));
        light_about_center(&light_over(&gpu.shot(&staged(rings, true)), &bare), SIZE)
    };

    let reference = mark_light(&mut gpu, &band_only);
    let stripped = mark_light(&mut gpu, &empty);
    assert!(reference.weight > 0.0 && stripped.weight > 0.0, "the mark drew nothing");

    // The strip's OUTER edge is what both readings are taken from: it is the
    // one end the octave gap does not eat into, since a sector is wider than
    // the padding out there and narrower than it near the node's centre.
    // Calibrated on the reference, where a ring IS under the strip and the
    // padding is genuinely owed.
    let want_ref = (band_only.mark_inner + band_only.mark_thickness) as f64;
    let scale = reference.far / want_ref;
    let far_uv = stripped.far / scale;
    eprintln!(
        "band under it: {:.1} px = {want_ref:.4} uv ({:.1} px/uv); \
         nothing under it: {:.1} px = {far_uv:.4} uv, thickness {:.4}, gap {:.4}",
        reference.far, scale, stripped.far, empty.mark_thickness, empty.gap,
    );
    assert!(
        (far_uv - empty.mark_thickness as f64).abs() < empty.gap as f64 / 2.0,
        "with every ring off the strip reaches {far_uv:.4} uv, not the {:.4} it is deep — \
         it is standing a padding off nothing, with a hole at the node's centre. \
         `stacked` skips the gap at a cursor of 0 and the strip has to skip it too",
        empty.mark_thickness,
    );
}

#[test]
fn a_real_held_chord_shows_its_melody_and_bass_marks() {
    // End to end, exactly how the app runs it: a held chord through
    // derive_scene, NOT a Scene assembled by hand. The by-hand test
    // above pins the shader down but would happily pass while the
    // tracker -> view -> node-mask path was broken, which is the half
    // that actually reaches a user.
    let Some(mut gpu) = Shooter::new(SIZE) else {
        return;
    };
    use harmonigraph_core::{NoteEvent, NoteTracker, Tuning};
    use harmonigraph_scene::{derive_scene, Camera, FrameParams, ViewConfig};

    const SIZE: [u32; 2] = [256, 256];

    let mut tracker = NoteTracker::new();
    for note in [60u8, 64, 67] {
        tracker.handle_event(NoteEvent::on(0.0, 0, note, 1.0));
    }
    // A small window so the nodes draw big enough to measure.
    let base =
        ViewConfig { extent_threes: 2, extent_fives: 2, extent_sevens: 0, ..ViewConfig::default() };
    let scene_for = |marks: bool| {
        derive_scene(
            &tracker,
            &Tuning::default(),
            &ViewConfig { mark_melody: marks, mark_bass: marks, ..base.clone() },
            &base.reach(),
            // No envelope: every layer of a node eases in from its note-on
            // over the Fade, so under a real one t=0 is the instant nothing
            // is drawn yet and any later sample is a fraction. What is
            // compared below is a lit node against a lit node.
            &FrameParams { fade_time: 0.0, ..FrameParams::default() },
            Camera::default(),
            None,
            0.5,
        )
    };

    // The masks must survive derive_scene in the first place.
    let marked = scene_for(true);
    let melody_nodes = marked.nodes.iter().filter(|n| n.melody_slots != 0).count();
    let bass_nodes = marked.nodes.iter().filter(|n| n.bass_slots != 0).count();
    assert!(
        melody_nodes > 0 && bass_nodes > 0,
        "derive_scene marked nothing: {melody_nodes} melody, {bass_nodes} bass nodes"
    );

    let off = gpu.shot(&scene_for(false));
    let on = gpu.shot(&marked);
    let lit = off.chunks(4).filter(|px| px[..3] != [0, 0, 0]).count();
    let changed = off.chunks(4).zip(on.chunks(4)).filter(|(a, b)| a != b).count();
    eprintln!("chord: {lit} lit px, {changed} changed by the marks");
    // Same reasoning as the by-hand test above, at a density where a node is
    // tens of pixels across rather than hundreds: the strip is dialled in the
    // node's uv, so it shrinks with the node and the share it claims does not.
    // Current: 53% of the lit pixels, against the 5% asked for.
    assert!(
        changed * 20 > lit,
        "turning the marks on barely changed a real chord: \
         {changed} px of {lit} lit"
    );
}

/// One marker alone on a black field, at the size an area measurement wants:
/// no nodes, so nothing composites over it, and nothing else in the shot can
/// be mistaken for its ink.
fn lone_marker_scene(half_width: f32, taper_start: f32) -> Scene {
    let mut scene = idle_scene();
    scene.nodes.clear();
    scene.pluses = vec![one_marker(
        glam::Vec3::ZERO,
        // Big enough that the screen-constant soft band is a thin rim on it
        // rather than a share of the area — the band is the error term in
        // every ratio below, and a small marker is mostly band.
        0.5,
        glam::Vec4::ONE,
        1.0,
    )];
    scene.plus_half_width = half_width;
    scene.plus_taper_start = taper_start;
    scene
}

/// The two proportions a marker carries are read by the SHADER as the shapes
/// they name: a filled square at the top of the width bar, and ends that
/// actually run out at the bottom of the taper.
///
/// `the_width_reaches_the_scene_as_a_share_of_the_arm` pins the conversion on
/// the way in, and it cannot see this: the number arriving correct in the
/// uniform says nothing about the distance field spending it. What is measured
/// here is AREA, which is a number rather than a look — a cross of half-width
/// `t` covers `8t - 4t^2` of an arm-squared where a filled square covers 4, so
/// the ratio between two widths is arithmetic the picture either agrees with
/// or does not.
///
/// The premultiplied ink is linear in coverage (`plus_paint` returns
/// `rgb * alpha`), so summing the light IS integrating the area, with the soft
/// band as a proportional rim on both — which is why the marker is drawn big
/// and the tolerance is a tenth rather than a percent.
///
/// What stays a look, and stays with `the_resting_markers_draw_a_picture`:
/// whether a cross reads as a crossing rather than as a glyph, and whether the
/// tapered end arrives at nothing rather than stopping at something.
#[test]
fn the_shader_spends_a_markers_proportions_on_the_shape_they_name() {
    const SIZE: [u32; 2] = [256, 256];
    let Some(mut gpu) = Shooter::new(SIZE) else {
        return;
    };

    // Square ends throughout, so this half measures the WIDTH alone.
    let ink = |gpu: &mut Shooter, t: f32| total_light(&gpu.shot(&lone_marker_scene(t, 1.0)));
    let cross = ink(&mut gpu, 0.275) as f64;
    let square = ink(&mut gpu, 1.0) as f64;
    assert!(cross > 0.0, "the fixture drew no marker at all");

    let want = 4.0 / (8.0 * 0.275 - 4.0 * 0.275 * 0.275);
    let got = square / cross;
    assert!(
        (got - want).abs() / want < 0.1,
        "a half-width of 1 covers {got:.3}x what 0.275 does, and the shape says {want:.3}x \
         — the box's y-extent is not being read as half the arm's thickness",
    );

    // And the square really is filled rather than a very fat cross: at
    // half-width 1 the box covers the whole octant, so there is nothing left
    // for a wider one to add.
    let over = ink(&mut gpu, 1.0) as f64;
    assert!(
        (over - square).abs() / square < 0.01,
        "past a full half-width the marker is still growing: {square} then {over}",
    );

    // The taper, at one width: an arm solid to its tip, then to half way, then
    // fading the whole way from the crossing. Ink has to fall each time, and by
    // a share the smoothstep can account for — it integrates to half over the
    // span it covers, and the crossing keeps its own.
    let taper = |gpu: &mut Shooter, start: f32| {
        total_light(&gpu.shot(&lone_marker_scene(0.275, start))) as f64
    };
    let (square_end, half, whole) =
        (taper(&mut gpu, 1.0), taper(&mut gpu, 0.5), taper(&mut gpu, 0.0));
    assert!(
        square_end > half && half > whole,
        "a longer taper has to take more ink, not less: {square_end} {half} {whole}",
    );
    let lost = (square_end - whole) / square_end;
    assert!(
        (0.25..0.65).contains(&lost),
        "tapering the whole arm took {:.0}% of the ink; a smoothstep over the arm \
         with the crossing keeping its own is nearer 40%",
        lost * 100.0,
    );
}
