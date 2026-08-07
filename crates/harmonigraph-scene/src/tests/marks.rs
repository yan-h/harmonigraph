//! The melody and bass marks: which held notes carry them, what color they
//! take, how they ease in, and what happens when the note under one is
//! released.

use crate::*;
use crate::derive::held_extremes;
use harmonigraph_core::{NoteEvent, NoteEventKind, NoteTracker, PitchClass, Tuning};
use super::harness::*;

/// Play `notes` on channel 0 and derive a scene marking both extremes.
fn marked_scene(notes: &[u8], mark_melody: bool, mark_bass: bool) -> Scene {
    let mut tracker = NoteTracker::new();
    for &note in notes {
        tracker.handle_event(NoteEvent {
            time: 0.0,
            channel: 0,
            note,
            kind: NoteEventKind::On { velocity: 1.0 },
        });
    }
    // The flat harness, because these read the masks at time 0: under a real
    // Fade and mark Delay that instant is the one moment a note is guaranteed
    // not to be drawn yet, and a slot bit asserted over a node drawing nothing
    // says less than it reads as saying.
    let view = ViewConfig { mark_melody, mark_bass, ..plain_view() };
    scene_of(&tracker, &Tuning::default(), &view, &plain_frame(), 0.0)
}

/// A note-on and a note-off on channel 0, for the timed sequences below.
fn on(time: f64, note: u8) -> NoteEvent {
    NoteEvent { time, channel: 0, note, kind: NoteEventKind::On { velocity: 1.0 } }
}

fn off(time: f64, note: u8) -> NoteEvent {
    NoteEvent { time, channel: 0, note, kind: NoteEventKind::Off }
}

/// The envelope duration these tests run on, pinned here rather than taken
/// from the Fade's own default. They are about WHEN a ring arrives — the
/// delay, the handoff, the threshold — and the ramp is only the clock they
/// read that off. Pinning it keeps them measuring the delay rather than the
/// day someone retunes the default, and lets them sample the ramp at a half
/// and read a half (see `fade_shape` below).
const ATTACK: f64 = 0.15;

/// Both ends marked, with the Delay bar at `mark_delay`.
///
/// Straight-line shape on purpose: these tests sample the ramp at fractions
/// of [`ATTACK`] and expect the same fraction back, which is a statement about
/// the DELAY and not about the curve. The curve has its own tests, in
/// `harmonigraph_core::notes`.
fn delayed_view(mark_delay: f32) -> ViewConfig {
    ViewConfig {
        mark_melody: true,
        mark_bass: true,
        mark_delay,
        fade_shape: 0.0,
        ..ViewConfig::default()
    }
}

/// The frame that carries [`ATTACK`] as the envelope's duration — the other
/// half of [`delayed_view`], now that the arrival and the fade are one number.
fn attack_frame() -> FrameParams {
    FrameParams { fade_time: ATTACK as f32, ..FrameParams::default() }
}

/// Union of a mask across every node, and the nodes carrying it.
fn marked_slots(scene: &Scene, melody: bool) -> (u32, usize) {
    let mut bits = 0u32;
    let mut nodes = 0usize;
    for n in &scene.nodes {
        let m = if melody { n.melody_slots } else { n.bass_slots };
        if m != 0 {
            bits |= m;
            nodes += 1;
        }
    }
    (bits, nodes)
}

#[test]
fn melody_and_bass_mark_the_outer_held_notes() {
    // C4/E4/G4: the melody is G4, the bass C4 -- middle C's octave for
    // both (same MIDI octave, but different nodes/pitch classes).
    let scene = marked_scene(&[60, 64, 67], true, true);
    let (melody_bits, melody_nodes) = marked_slots(&scene, true);
    let (bass_bits, bass_nodes) = marked_slots(&scene, false);
    assert_eq!(melody_bits, 1 << MIDDLE_C_SLOT, "G4 sounds in middle C's octave");
    assert_eq!(bass_bits, 1 << MIDDLE_C_SLOT, "C4 too");
    assert!(melody_nodes > 0 && bass_nodes > 0);
    // And the marks are DRAWN at the instant this reads them, which is what
    // makes the masks above a statement about the picture rather than about
    // the bookkeeping behind it: a slot bit with no level is a ring nobody
    // sees, and every assertion here would hold over a blank lattice.
    for n in &scene.nodes {
        if n.melody_slots != 0 {
            assert_eq!(n.melody_level, 1.0, "the melody ring is up, not just recorded");
        }
        if n.bass_slots != 0 {
            assert_eq!(n.bass_level, 1.0, "and the bass ring with it");
        }
    }

    // The marks land on the nodes those notes actually light, and the
    // middle note (E4) is marked as neither.
    let tuning = Tuning::default();
    for n in &scene.nodes {
        let pc = tuning.pitch_class(n.lattice_pos);
        if n.melody_slots != 0 {
            assert!(tuning.matches(PitchClass::from_midi_note(67), pc), "melody is G");
        }
        if n.bass_slots != 0 {
            assert!(tuning.matches(PitchClass::from_midi_note(60), pc), "bass is C");
        }
    }

    // Asking for one end leaves the other unmarked.
    let melody_only = marked_scene(&[60, 64, 67], true, false);
    assert_eq!(marked_slots(&melody_only, true).0, 1 << MIDDLE_C_SLOT);
    assert_eq!(marked_slots(&melody_only, false).0, 0, "bass not asked for");
    let off = marked_scene(&[60, 64, 67], false, false);
    assert_eq!(marked_slots(&off, true).0, 0);
    assert_eq!(marked_slots(&off, false).0, 0);
}

#[test]
fn a_lone_held_note_is_marked_as_both_ends() {
    // One note is at once the highest and the lowest held, and is marked
    // as both. The shader splits such a mark between the two colors (see
    // mark_paint) rather than blanking it -- blanking gives an outline that
    // vanishes exactly when two things are true at once.
    let scene = marked_scene(&[60], true, true);
    let mut seen = false;
    for n in &scene.nodes {
        assert_eq!(
            n.melody_slots, n.bass_slots,
            "a lone note must claim identical slots at both ends"
        );
        seen |= n.melody_slots != 0;
    }
    assert!(seen, "the note should have been marked somewhere");
}

#[test]
fn a_chord_inside_one_pitch_class_separates_on_the_octave_layer() {
    // C3 and C5: one pitch class, so both land on the SAME node and the
    // core can't say which is which -- but they sound in different
    // octave slots, which is what keeps them tellable apart.
    let scene = marked_scene(&[48, 72], true, true);
    let marked: Vec<_> = scene
        .nodes
        .iter()
        .filter(|n| n.melody_slots != 0 || n.bass_slots != 0)
        .collect();
    assert!(!marked.is_empty(), "C should be marked");
    for n in &marked {
        // Slot = MIDI octave + 1, so C5 (72) is one above middle C's slot
        // and C3 (48) is one below.
        assert_eq!(n.melody_slots, 1 << (MIDDLE_C_SLOT + 1), "the high C is the melody");
        assert_eq!(n.bass_slots, 1 << (MIDDLE_C_SLOT - 1), "the low C is the bass");
        // No slot claimed by both, so the octave layer marks each end
        // rather than suppressing them the way the core has to.
        assert_eq!(n.melody_slots & n.bass_slots, 0);
    }
}

#[test]
fn a_handoff_inside_one_pitch_class_rings_whichever_end_is_stronger() {
    // C4 and C5 share a pitch class, so they light ONE node — and once C5 is
    // released BOTH wear a melody: C4 the live end it inherited, C5 the stamp
    // it left with. A node carries one ring at one level, so it takes the
    // stronger voice entire, slot and level together (see `derive::Mark`).
    //
    // Admitting both slots under the one level is the thing being ruled out:
    // it draws the loser's link at the winner's brightness, which says the
    // octave it points at is ringing when it is half gone — or, right after
    // the handoff, that the incoming one is at full before it has begun.
    let mut tracker = NoteTracker::new();
    for note in [60u8, 72] {
        tracker.handle_event(NoteEvent {
            time: 0.0,
            channel: 0,
            note,
            kind: NoteEventKind::On { velocity: 1.0 },
        });
    }
    tracker.handle_event(NoteEvent { time: 1.0, channel: 0, note: 72, kind: NoteEventKind::Off });
    let view = delayed_view(0.0);
    // The key-up is a whole duration past the note-on, so the leaving ring is
    // leaving and nothing here is still arriving.
    let frame = attack_frame();
    let at = |now: f64| {
        let scene = scene_of(&tracker, &Tuning::default(), &view, &frame, now);
        *origin_node(&scene)
    };

    // A quarter of the way through the handoff the DEPARTING ring is still the
    // stronger of the two: C5 is a quarter into its fade while C4's inherited
    // ring is a quarter up its ease. So the ring points at C5's octave, at
    // C5's own level — it does not blink out at the key.
    let just_after = at(1.0 + ATTACK * 0.25);
    assert_eq!(
        just_after.melody_slots,
        1 << (MIDDLE_C_SLOT + 1),
        "the ring still links to the released C5, which is what is still bright",
    );
    assert!(
        (just_after.melody_level - 0.75).abs() < 1e-5,
        "at C5's own fading level, got {}",
        just_after.melody_level
    );

    // Three quarters through, the two have crossed: C4's ring is the stronger
    // and the sector the ring links to has moved with it. The level is
    // continuous across that switch — one duration drives both, so the two
    // curves are exactly equal half way — and what the viewer sees move is
    // the link rather than the brightness.
    let later = at(1.0 + ATTACK * 0.75);
    assert_eq!(later.activation, 1.0, "the held C4 keeps the node lit throughout");
    assert_eq!(later.melody_slots, 1 << MIDDLE_C_SLOT, "the ring has moved to the held C4");
    assert!(
        (later.melody_level - 0.75).abs() < 1e-5,
        "at C4's own arriving level, got {}",
        later.melody_level
    );
    assert_eq!(later.bass_slots, 1 << MIDDLE_C_SLOT, "only C4 was ever the bass");

    // The OCTAVE layer is per slot, so it shows both notes the whole time —
    // the released C5 fading on its own envelope under a ring that has left
    // it. That layer is where a doubling stays legible; the ring is one ring.
    assert!(
        (later.octaves[MIDDLE_C_SLOT + 1] - 0.25).abs() < 1e-5,
        "the released C5's octave is three quarters gone, got {}",
        later.octaves[MIDDLE_C_SLOT + 1]
    );
    assert_eq!(later.octaves[MIDDLE_C_SLOT], 1.0, "the held C4's octave is at full");
}

/// A ring that never cleared the Delay contributes no SLOT either, not just no
/// level. The mask is what the shader slits the ring at, so a rejected voice
/// left in it would cut the ring of the note that did earn one at an octave
/// the wait was there to reject — a link pointing at a note that never rang.
#[test]
fn an_end_dropped_inside_the_delay_does_not_slit_the_ring_that_replaced_it() {
    const DELAY: f64 = 0.2;
    let mut tracker = NoteTracker::new();
    // C4 and C5 hold the node; C6 takes the melody for less than the wait.
    tracker.handle_event(on(0.0, 60));
    tracker.handle_event(on(0.0, 72));
    tracker.handle_event(on(0.5, 84));
    tracker.handle_event(off(0.55, 84));
    let view = delayed_view(DELAY as f32);
    let frame = attack_frame();
    // C5 retook the melody at 0.55; one wait and one attack later its ring is
    // whole, and the C6 that came and went inside the wait is still in the
    // tracker's released tail.
    let scene = scene_of(&tracker, &Tuning::default(), &view, &frame, 0.55 + DELAY + ATTACK);
    let origin = origin_node(&scene);
    // Sampled at the ramp's own endpoint, which the delay puts a sum of three
    // f64s away from a round number — so the claim is "up", not a bit pattern.
    let level = origin.melody_level;
    assert!((level - 1.0).abs() < 1e-5, "C5's ring is up, got {level}");
    assert_eq!(
        origin.melody_slots,
        1 << (MIDDLE_C_SLOT + 1),
        "and links to C5 alone — C6 never earned a slot to cut it at",
    );
}

/// A ring leaves on the note's own release rather than with its key, and
/// leaves on exactly the envelope the octave sector beside it does — the two
/// belong to one note, and a ring that outlived or predeceased its sector
/// would read as a second thing happening.
#[test]
fn a_lone_notes_ring_fades_out_with_it() {
    let mut tracker = NoteTracker::new();
    tracker.handle_event(on(0.0, 60));
    tracker.handle_event(off(1.0, 60));
    let view = delayed_view(0.0);
    let frame = attack_frame();
    let at = |now: f64| {
        let scene = scene_of(&tracker, &Tuning::default(), &view, &frame, now);
        let n = origin_node(&scene);
        (n.melody_level, n.bass_level, n.octaves[MIDDLE_C_SLOT])
    };

    assert_eq!(at(1.0), (1.0, 1.0, 1.0), "at the key-up it is still whole");
    let (melody, bass, octave) = at(1.0 + ATTACK * 0.5);
    assert!((melody - 0.5).abs() < 1e-5, "half a fade later, half gone: got {melody}");
    assert_eq!(melody, bass, "a lone note's two rings leave together");
    assert_eq!(melody, octave, "and on the sector's own envelope");
    assert_eq!(at(1.0 + ATTACK), (0.0, 0.0, 0.0), "gone at the end of the fade");
}

/// A note shorter than its arrival leaves its ring LEVEL with the sector it
/// brackets, the whole way through. The ring's ramp runs on past the key
/// exactly as the disc's does; frozen where the key-up found it, a staccato
/// note would ring at a fraction of the octave it points at until it was gone
/// — one layer disagreeing with the next about how fast the note arrived,
/// which is what a shared curve is there to prevent. Both reach FULL on the
/// way (`Voice::release_level`), so the level they agree on is the whole one.
#[test]
fn a_note_shorter_than_its_arrival_still_rings_level_with_its_sector() {
    let mut tracker = NoteTracker::new();
    tracker.handle_event(on(0.0, 60));
    // Lifted a third of the way up the ramp, so a frozen reading and a running
    // one are far apart rather than a rounding away.
    tracker.handle_event(off(ATTACK / 3.0, 60));
    let view = delayed_view(0.0);
    let frame = attack_frame();
    let ring = |now: f64| {
        let scene = scene_of(&tracker, &Tuning::default(), &view, &frame, now);
        let n = *origin_node(&scene);
        (n.melody_level, n.bass_level, n.octaves[MIDDLE_C_SLOT])
    };

    for step in 1..=6 {
        let now = ATTACK * f64::from(step) / 3.0;
        let (melody, bass, octave) = ring(now);
        assert_eq!(melody, octave, "at {now}s the ring and the sector it links to disagree");
        assert_eq!(melody, bass, "and a lone note's two rings leave together");
    }
    // And it is a rising ramp being read rather than a flat one: the note goes
    // on getting brighter after its own key-up, which is the whole reason the
    // attack outlives the key.
    assert!(ring(ATTACK).0 > ring(ATTACK / 3.0).0, "the ring still climbs past the key");
}

/// The Delay stays a THRESHOLD now that a ring outlives its key. An end
/// dropped before the wait is up never rang, and must not ring on the way out:
/// the threshold is answered at the key-up, so the ramp that runs on from
/// there has nothing to carry.
#[test]
fn an_end_dropped_inside_the_delay_never_rings_on_its_way_out() {
    const DELAY: f64 = 0.5;
    let mut tracker = NoteTracker::new();
    tracker.handle_event(on(0.0, 60));
    // Held for less than the wait, and then released with a long envelope —
    // long enough that an ease still running would sail past the threshold.
    tracker.handle_event(off(0.2, 60));
    let view = delayed_view(DELAY as f32);
    let frame = FrameParams { fade_time: 4.0, ..FrameParams::default() };
    for now in [0.2, 0.5, 1.0, 2.0, 3.0] {
        let scene = scene_of(&tracker, &Tuning::default(), &view, &frame, now);
        let n = origin_node(&scene);
        assert_eq!(n.melody_level, 0.0, "a ring appeared at {now}s on a note that never rang");
        assert_eq!(n.bass_level, 0.0, "and a bass ring at {now}s");
    }
}

#[test]
fn a_fresh_mark_eases_in_with_the_octave_it_links_to() {
    // A ring arriving at full the frame its note claims an end is the
    // jumpiest thing on the node, since the octave sector underneath it
    // eases in. Both ride the one ramp, so a note's outer layer arrives
    // as a single gesture — and the ramp is the note's own attack, which is
    // what keeps the ring from arriving as the SQUARE of the sector's ease
    // (the ring carries its own attack, so only the release goes under it).
    let mut tracker = NoteTracker::new();
    tracker.handle_event(NoteEvent {
        time: 0.0,
        channel: 0,
        note: 60, // C4: the origin node, in middle C's octave slot
        kind: NoteEventKind::On { velocity: 1.0 },
    });
    let view = delayed_view(0.0);
    let at = |now: f64| {
        let scene = scene_of(&tracker, &Tuning::default(), &view, &attack_frame(), now);
        let n = origin_node(&scene);
        (n.melody_level, n.bass_level, n.octaves[MIDDLE_C_SLOT])
    };

    assert_eq!(at(0.0), (0.0, 0.0, 0.0), "nothing has arrived on the frame itself");

    let (melody, bass, octave) = at(ATTACK * 0.5);
    assert!((melody - 0.5).abs() < 1e-5, "half way in, got {melody}");
    assert_eq!(melody, bass, "a lone note's two ends arrive together");
    assert_eq!(melody, octave, "the ring rides the sector's own ramp");

    assert_eq!(at(ATTACK), (1.0, 1.0, 1.0), "full by the end of the attack");
}

#[test]
fn an_inherited_end_eases_in_from_the_handoff_not_from_its_note_on() {
    // Hold C4 and G4, then lift the top: the melody drops to C4, whose own
    // note-on is long past. Easing from THAT would be no ease at all — the
    // ring has to grow from the moment it moved. C4's bass ring never
    // changed hands, so it stays at full right through the handoff.
    //
    // A fifth apart rather than an octave, so the two notes land on their own
    // nodes: the outgoing ring fades out where G is drawn and the incoming
    // one eases in at C, and neither reading is mixed into the other by one
    // node having to carry both.
    let mut tracker = NoteTracker::new();
    for note in [60u8, 67] {
        tracker.handle_event(NoteEvent {
            time: 0.0,
            channel: 0,
            note,
            kind: NoteEventKind::On { velocity: 1.0 },
        });
    }
    tracker.handle_event(NoteEvent { time: 1.0, channel: 0, note: 67, kind: NoteEventKind::Off });
    let view = delayed_view(0.0);
    let frame = attack_frame();
    let at = |now: f64| {
        let scene = scene_of(&tracker, &Tuning::default(), &view, &frame, now);
        let n = origin_node(&scene);
        (n.melody_level, n.bass_level)
    };
    // 12-TET: a fifth is one step along the threes axis.
    let leaving = |now: f64| {
        let scene = scene_of(&tracker, &Tuning::default(), &view, &frame, now);
        node_at(&scene, LatticePos::new(1, 0, 0)).melody_level
    };

    assert_eq!(at(1.0), (0.0, 1.0), "the melody has only just moved");
    let (melody, bass) = at(1.0 + ATTACK * 0.5);
    assert!((melody - 0.5).abs() < 1e-5, "half way in, got {melody}");
    assert_eq!(bass, 1.0, "the end that never moved does not re-attack");
    assert_eq!(at(1.0 + ATTACK), (1.0, 1.0));

    // The other half of the same handoff: G's ring leaves as C's arrives,
    // on G's own release rather than on the incoming ramp.
    assert_eq!(leaving(1.0), 1.0, "the outgoing ring is whole at the key-up");
    assert!(
        (leaving(1.0 + ATTACK * 0.5) - 0.5).abs() < 1e-5,
        "and half gone half a fade later — exactly as the incoming one is half in",
    );
    assert_eq!(leaving(1.0 + ATTACK), 0.0, "and gone with the note");
}

#[test]
fn a_delay_holds_the_ring_off_until_its_note_has_worn_the_end_that_long() {
    // The wait sits in FRONT of the ease rather than stretching it: the ring
    // is at nothing for the whole delay and then arrives on the same ramp it
    // always did, so the two settings say when and how fast independently.
    const DELAY: f64 = 0.25;
    let tracker = held(60); // C4: the origin node, in middle C's octave slot
    let view = delayed_view(DELAY as f32);
    let at = |now: f64| {
        let scene = scene_of(&tracker, &Tuning::default(), &view, &attack_frame(), now);
        let n = origin_node(&scene);
        (n.melody_level, n.bass_level, n.octaves[MIDDLE_C_SLOT])
    };

    // Not `assert_eq!(.., 0.0)` at the delay itself: the claim is one-sided
    // ("has not started"), and the ramp's own zero sits exactly there. These
    // constants are binary-exact so the equality would in fact hold, but a
    // later reader retuning DELAY to 0.3 would land a hair past the corner
    // and fail on a smoothstep of about 1e-32, which is nothing drawn.
    assert!(at(DELAY * 0.5).0 < 1e-6, "half way through the wait, nothing has started");
    assert!(at(DELAY).0 < 1e-6, "the ease starts AT the delay, not before it");

    let (melody, bass, octave) = at(DELAY + ATTACK * 0.5);
    assert!((melody - 0.5).abs() < 1e-5, "half way in, got {melody}");
    assert_eq!(melody, bass, "a lone note's two ends wait together");
    assert_eq!(at(DELAY + ATTACK).0, 1.0, "full one attack after the wait");

    // The layer UNDER the ring keeps its own timing: a sector eases from its
    // note-on whatever this bar says, so the delay reads as the ring arriving
    // late over a lit octave rather than as the whole outer layer being late.
    assert_eq!(octave, 1.0, "the octave sector is not delayed with the ring");
}

#[test]
fn an_end_given_up_inside_the_delay_never_rings_at_all() {
    // The flicker the setting exists for. Playing fast, the top of what is
    // down changes every few notes, and a ring easing in on each of them
    // reads as flicker over the band rather than as the line being traced.
    // A note that has lost the end again before its wait is up draws no ring
    // at any point: it never cleared the threshold while it was down, and a
    // released voice that never cleared it has no ring to fade out.
    const DELAY: f64 = 0.25;
    let mut tracker = NoteTracker::new();
    tracker.handle_event(on(0.0, 60)); // C4, held right through
    tracker.handle_event(on(0.05, 67)); // G4 takes the melody...
    tracker.handle_event(off(0.15, 67)); // ...and hands it back inside the wait
    let view = delayed_view(DELAY as f32);
    let frame = attack_frame();
    let ring = |now: f64| {
        let scene = scene_of(&tracker, &Tuning::default(), &view, &frame, now);
        // The loudest melody ring ANYWHERE: G4 and C4 light different nodes,
        // and the claim is about the whole picture, not one node of it.
        scene.nodes.iter().fold(0.0f32, |peak, n| peak.max(n.melody_level))
    };

    for step in 0..=10 {
        let now = 0.15 + DELAY * f64::from(step) / 10.0;
        assert!(ring(now) < 1e-6, "a ring rang at {now}, inside C4's own wait");
    }
    // C4 re-took the melody at the handoff, so its ring is due one wait and
    // one attack after THAT — not after its own note-on, which is older than
    // both put together.
    assert_eq!(ring(0.15 + DELAY + ATTACK), 1.0, "and then C4's ring arrives");
    // The bass end never changed hands through any of it, so it is measured
    // from C4's note-on and has been up since well before.
    let scene = scene_of(&tracker, &Tuning::default(), &view, &frame, 0.15 + DELAY);
    assert_eq!(origin_node(&scene).bass_level, 1.0, "the end that never moved is unaffected");
}

#[test]
fn a_delay_past_the_note_fade_still_measures_from_the_handoff() {
    // The handoff moment has to outlive the note that made it. Lift the top
    // of a held chord and the note below inherits the melody AT THAT MOMENT
    // — but the note that handed it over is pruned one Fade later, and the
    // default fade is 0.1s, shorter than most of this bar. Read the moment
    // off the released tail and any longer delay would lose it mid-wait and
    // land the ring at full in a single frame, which is the pop the wait was
    // set to avoid. The tracker's own stamp is what survives the pruning.
    //
    // The Fade named here is shorter than the default, so the pruning happens
    // well inside the wait rather than at the edge of it.
    const DELAY: f64 = 0.5;
    const FADE: f32 = 0.1;
    let mut tracker = NoteTracker::new();
    tracker.handle_event(on(0.0, 60)); // C4 and C5 share a pitch class, so
    tracker.handle_event(on(0.0, 72)); // both land on the origin node
    tracker.handle_event(off(1.0, 72));
    let view = delayed_view(DELAY as f32);
    let frame = FrameParams { fade_time: FADE, ..FrameParams::default() };
    // The frame order the UI runs in: prune, then derive.
    tracker.prune(1.2, &view.envelope(&frame));
    assert_eq!(tracker.voices().count(), 1, "the C5 that handed the end over is gone");

    let at = |now: f64| {
        let scene = scene_of(&tracker, &Tuning::default(), &view, &frame, now);
        origin_node(&scene).melody_level
    };

    // The ramp is the Fade's own length here rather than [`ATTACK`], this
    // being the one test that names a duration of its own.
    let ramp = f64::from(FADE);
    assert!(at(1.0 + DELAY * 0.99) < 1e-6, "still waiting, long after the C5 was pruned");
    assert!((at(1.0 + DELAY + ramp * 0.5) - 0.5).abs() < 1e-5, "then it eases in");
    assert_eq!(at(1.0 + DELAY + ramp), 1.0);
}

/// The Delay's range is held in `derive_scene` and nowhere else — `sanitize`
/// deliberately does range work for nothing, only finiteness — so a view out
/// of range comes from a file and lands here. Both ends matter and they fail
/// in opposite directions: a negative delay starts the ramp BEFORE the note
/// took the end, which is every ring at full the frame it is claimed, exactly
/// what easing them in exists to prevent; a huge one is the mark layer gone
/// for as long as the take lasts, from a bar that cannot say so.
///
/// Asserted on the LEVEL rather than on a scene field, because unlike every
/// other clamp in `derive_scene` this one reaches the picture only through
/// the ring's ease — there is no `Scene::mark_delay` to read back.
#[test]
fn the_mark_delay_is_clamped_to_the_bar_its_own_ends() {
    let tracker = held(60);
    let level = |mark_delay: f32, now: f64| {
        let view = delayed_view(mark_delay);
        let scene = scene_of(&tracker, &Tuning::default(), &view, &attack_frame(), now);
        origin_node(&scene).melody_level
    };

    // A settable delay passes through untouched, which is what makes the two
    // clamps below a range rather than a constant.
    assert!(level(0.5, 0.5 + ATTACK * 0.5) > 0.4, "a delay inside the bar still eases");

    // Below the bar: floored at 0, the ring arriving with its note as it
    // always did — not ahead of it.
    assert_eq!(level(-5.0, 0.0), 0.0, "a negative delay must not pre-start the ramp");
    assert_eq!(level(-5.0, ATTACK), 1.0, "it is the undelayed ramp, not a broken one");

    // Above it: capped at the bar's end, so the rings still arrive.
    assert_eq!(level(1e9, MARK_DELAY_MAX as f64 + ATTACK), 1.0, "a huge delay stops at 1s");
}

/// A non-finite delay is the one value that would take the mark layer down
/// silently: `clamp` passes a NaN straight through (every comparison against
/// it is false), the ease comes out NaN, and `Mark::add`'s `>=` is false
/// against it — so the node keeps a level of 0 and no slot at all. No ring
/// anywhere, and nothing to say why. `ViewConfig::sanitize` is the only
/// guard, and this is the test that keeps it from being deleted as redundant
/// with the clamp above.
#[test]
fn a_non_finite_delay_loads_as_no_delay_at_all() {
    let tracker = held(60);
    let mut view = delayed_view(f32::NAN);
    view.sanitize();
    assert_eq!(view.mark_delay, 0.0, "the blob's door is where a NaN is repaired");

    let scene = scene_of(&tracker, &Tuning::default(), &view, &plain_frame(), ATTACK);
    assert_eq!(origin_node(&scene).melody_level, 1.0, "and the rings draw as they always did");
}

#[test]
fn held_extremes_never_names_a_released_voice() {
    // A released voice rings from its own stamp (above), and is for that very
    // reason out of the running for the LIVE ends: letting it stay "the
    // melody" would steal that from the note that actually replaced it, and
    // leave the incoming ring nothing to ease from.
    let mut tracker = NoteTracker::new();
    for note in [60u8, 67] {
        tracker.handle_event(NoteEvent {
            time: 0.0,
            channel: 0,
            note,
            kind: NoteEventKind::On { velocity: 1.0 },
        });
    }
    // Release the top note; C is now both the highest and lowest held.
    tracker.handle_event(NoteEvent {
        time: 0.1,
        channel: 0,
        note: 67,
        kind: NoteEventKind::Off,
    });
    let (melody, bass) = held_extremes(&tracker, true, true);
    assert_eq!(melody.map(|e| e.key), Some((0, 60)), "the released G must not stay the melody");
    assert_eq!(bass.map(|e| e.key), Some((0, 60)));
    // And C took the melody at the handoff, not at its own note-on: the ring
    // grows from the moment it moved (see `an_inherited_end_eases_in_...`).
    assert_eq!(melody.map(|e| e.since), Some(0.1));
    assert_eq!(bass.map(|e| e.since), Some(0.0), "the end that never moved keeps its stamp");

    // Nothing held at all: nothing to mark.
    tracker.handle_event(NoteEvent {
        time: 0.2,
        channel: 0,
        note: 60,
        kind: NoteEventKind::Off,
    });
    assert_eq!(held_extremes(&tracker, true, true), (None, None));
}

/// `notes` held on `channel` with both ends marked and the octave wheel set to
/// `count` octaves centered on `center`.
fn marked(channel: u8, notes: &[u8], count: u32, center: f32) -> (Scene, FrameParams) {
    let mut tracker = NoteTracker::new();
    for &note in notes {
        tracker.handle_event(NoteEvent {
            time: 0.0,
            channel,
            note,
            kind: NoteEventKind::On { velocity: 1.0 },
        });
    }
    let view = ViewConfig {
        octave_count: count,
        octave_center: center,
        mark_melody: true,
        mark_bass: true,
        ..plain_view()
    };
    let frame = plain_frame();
    (scene_of(&tracker, &Tuning::default(), &view, &frame, 0.0), frame)
}

/// A lone held note on `channel` — its own melody and bass — and the origin
/// node it lights.
fn lone_mark(channel: u8, note: u8, count: u32, center: f32) -> (NodeInstance, FrameParams) {
    let (scene, frame) = marked(channel, &[note], count, center);
    (*origin_node(&scene), frame)
}

/// What the octave layer paints slot `slot` on this node. Built the shader's
/// way rather than through the scene's own helper — the pitch of the slot on
/// this pitch class, then `pitch_lut_color`'s walk across `pitch_ramp_lut` —
/// so this pins the arithmetic in `lattice.wgsl` and not merely the fact that
/// two call sites agree.
///
/// The gradient comes off the view the helpers above build with, NOT from
/// `PitchGradient::default()` — that one is the gradient type's own default
/// and is free to differ from what a fresh view opens on. Reading it here
/// would pin the two together and fail the moment either moves.
fn sector_color(node: &NodeInstance, slot: u32, frame: &FrameParams) -> Vec4 {
    let pitch = slot as f32 * 12.0 + node.cents / 100.0;
    let lut = pitch_ramp_lut(ViewConfig::default().pitch_gradient);
    let t = ((pitch - frame.darkest_pitch) / (frame.brightest_pitch - frame.darkest_pitch))
        .clamp(0.0, 1.0);
    let f = t * (PITCH_LUT_N - 1) as f32;
    let i0 = f.floor() as usize;
    lut[i0].lerp(lut[(i0 + 1).min(PITCH_LUT_N - 1)], f - f.floor())
}

#[test]
fn a_mark_is_the_color_of_the_sector_it_brackets() {
    // The reported bug: move the octave wheel off the register being played
    // and the melody/bass rings stop matching the note they mark. A note past
    // either end folds onto the outermost slot, so a ring carrying the VOICE's
    // color paints C7 around the C6 indicator it is bracketing. The ring is
    // part of the octave layer, so it takes the color of the slot it links
    // back to — whatever the wheel, and whether or not the note folded.
    // Held on channel 9, which is pitch-colored, so the note's own color is on
    // the ramp too and the axis is the only thing under test.
    //
    // Every marked node is checked rather than the origin alone: one voice
    // lights every node its pitch class matches, and those nodes are the SAME
    // class at different tunings of it, so their cents differ and the pitch
    // class's half of the slot pitch is live. C4 alone would leave it at zero
    // on every node it reaches.
    let (mut folded, mut off_c, mut ramp_top) = (false, false, false);
    for (count, center) in [(9, 60.0), (4, 74.0), (4, 60.0), (4, 48.0)] {
        for note in [96u8, 64, 108] {
            // C7, E4, C8
            let (scene, frame) = marked(9, &[note], count, center);
            let lit: Vec<_> = scene.nodes.iter().filter(|n| n.melody_slots != 0).collect();
            assert!(!lit.is_empty(), "note {note} must light a node at span {count}/{center}");
            for node in lit {
                assert_eq!(node.melody_slots.count_ones(), 1, "one note, one marked sector");
                assert_eq!(node.melody_slots, node.bass_slots, "a lone note is both ends");

                let slot = node.melody_slots.trailing_zeros();
                folded |= note == 96 && slot != 8; // C7's own slot, when the ring reaches it
                off_c |= node.cents != 0.0;
                ramp_top |= slot as f32 * 12.0 + node.cents / 100.0 >= frame.brightest_pitch;
                let sector = sector_color(node, slot, &frame);
                let where_ =
                    format!("note {note}, span {count} at {center}, {:?}", node.lattice_pos);
                assert_eq!(node.melody_color, sector, "melody ring: {where_}");
                assert_eq!(node.bass_color, sector, "bass ring: {where_}");
            }
        }
    }
    assert!(folded, "a wheel here has to fold C7, or the fold is not being tested");
    assert!(off_c, "a marked node here has to be off C, or its cents are not being tested");
    assert!(ramp_top, "a sector here has to reach the top of the ramp, or its clamp is not");
}

#[test]
fn the_two_rings_on_one_node_carry_their_own_sectors_colors() {
    // A chord voiced inside one pitch class puts both ends on ONE node, in
    // different octaves — which is the case that says the color has to be
    // derived per slot and not once per voice. Deriving it once and handing it
    // to both would paint the bass ring at the top of the ramp.
    let (scene, frame) = marked(9, &[36, 96], 9, 60.0); // C2 and C7
    let node = origin_node(&scene);
    let (melody, bass) = (node.melody_slots.trailing_zeros(), node.bass_slots.trailing_zeros());
    assert_eq!((melody, bass), (8, 3), "C7 is the melody up top, C2 the bass below");
    assert_eq!(node.melody_color, sector_color(node, melody, &frame));
    assert_eq!(node.bass_color, sector_color(node, bass, &frame));
    assert_ne!(node.melody_color, node.bass_color, "five octaves apart on the ramp");
}

#[test]
fn a_fixed_color_channel_keeps_its_disc_and_marks_the_lit_sector_on_the_ramp() {
    // A fixed-color channel colors the note itself; it does not reach the LIT
    // glyph, which the shader tints by its own pitch whatever the channel
    // (`pitch_lut_color`), so the ring that brackets one is on the ramp too.
    // Otherwise a red voice wears a red ring around a ramp-colored indicator.
    // The band's other parts still carry the channel — its ghosts are the
    // whitened node color — which is why this is about the lit sector alone.
    let (node, frame) = lone_mark(0, 60, 5, 60.0); // channel 0 is red
    let slot = node.melody_slots.trailing_zeros();
    assert_eq!(node.melody_color, sector_color(&node, slot, &frame));
    assert_eq!(
        node.color,
        channel_color(
            0,
            60.0,
            frame.darkest_pitch,
            frame.brightest_pitch,
            ViewConfig::default().pitch_gradient,
        )
    );
    assert_ne!(node.color, node.melody_color, "the disc keeps the channel's own color");
}


/// The mark rings' shimmer folds off with the rings themselves.
///
/// A thickness of 0 is the rings' documented off position, where `mark_ring`
/// returns no coverage, and the pane grays the row there.
///
/// The fold is needed because a mark sheet also sweeps the octave SLICE a
/// ring points at, which the glyph layer draws and no ring coverage
/// multiplies away: without it, switching the rings off leaves the marked
/// octaves sweeping from a control the user can no longer reach to stop.
/// Every pattern is folded, not only the ones that reach furthest — which
/// pattern is safe to leave standing is a fact about where it draws, and
/// pinning that to today's answer would make a pattern reaching past the ring
/// later a silent bug rather than an edit here.
/// The rings go off two ways, and BOTH have to fold the mode: no thickness to
/// draw one with, and no end marked for one to belong to
/// ([`ViewConfig::mark_rings_draw`], which the pane grays the row on). The
/// marks-off half is the easier one to leave out, because nothing is visibly
/// wrong when it is: no slot is marked, so `in.marks` is 0 and the slice the
/// sheet would reach collapses to zero on its own. That is the accident this
/// fold exists not to depend on.
#[test]
fn the_mark_pulse_folds_off_when_the_rings_are_off() {
    let pulse = |mark_thickness: f32, marked: bool, pulse_marks: Pulse| {
        let view = ViewConfig {
            mark_thickness,
            mark_melody: marked,
            mark_bass: marked,
            pulse_marks,
            ..ViewConfig::default()
        };
        scene_of(
            &NoteTracker::new(),
            &Tuning::default(),
            &view,
            &plain_frame(),
            0.0,
        )
        .pulse_marks
    };

    for mode in [Pulse::Bands, Pulse::Checker, Pulse::Hex, Pulse::Weave, Pulse::Rings] {
        assert_eq!(
            pulse(0.0, true, mode),
            Pulse::Off,
            "{mode:?} survived the ring thickness going to 0, where it keeps \
             drawing -- on the marked octave's own slice",
        );
        assert_eq!(
            pulse(0.09, false, mode),
            Pulse::Off,
            "{mode:?} survived both marks coming off, where there is no ring for it \
             to animate and the pane has grayed its row",
        );
        assert_eq!(
            pulse(0.09, true, mode),
            mode,
            "{mode:?} must survive a ring it can animate",
        );
    }
}

/// The shimmer's four settings reach the scene, and the width arrives
/// strictly positive however the view is set: the shader divides the
/// pattern's phase by it, so a 0 here is a whole lattice of NaN rather than a
/// stationary sweep. (Speed 0 IS the stationary sweep, and passes through, as
/// intensity 0 is the layer drawn unshimmered.)
///
/// The width's FLOOR is the second claim, and it is a look rather than a
/// safety margin: it has to leave room for several periods across one node,
/// so it is checked against the node's own world size (`spacing` ×
/// `NODE_RADIUS_FACTOR`) rather than against a bare "> 0".
///
/// Softness is the one clamped to exactly its bar, both ends. It drives an
/// exponent, and either side of 0..1 is a different shape rather than more of
/// this one — past 1 the lit part widens past the dark and the pattern reads
/// as rifts in a lit layer instead of light crossing a clear one.
#[test]
fn the_shimmer_settings_reach_the_scene_and_the_width_stays_positive() {
    let sweep = |shimmer_speed: f32, shimmer_width: f32, shimmer_intensity: f32| {
        let view = ViewConfig {
            shimmer_speed,
            shimmer_width,
            shimmer_intensity,
            ..ViewConfig::default()
        };
        let scene = scene_of(
            &NoteTracker::new(),
            &Tuning::default(),
            &view,
            &plain_frame(),
            0.0,
        );
        (scene.shimmer_speed, scene.shimmer_width, scene.shimmer_intensity)
    };
    let softness = |shimmer_softness: f32| {
        let view = ViewConfig { shimmer_softness, ..ViewConfig::default() };
        scene_of(
            &NoteTracker::new(),
            &Tuning::default(),
            &view,
            &plain_frame(),
            0.0,
        )
        .shimmer_softness
    };

    assert_eq!(sweep(3.0, 8.0, 0.5), (3.0, 8.0, 0.5), "a settable set passes through untouched");
    assert_eq!(sweep(0.0, 5.0, 0.0).0, 0.0, "speed 0 is a look -- the sheet, held still");
    assert_eq!(sweep(1.6, 5.0, 0.0).2, 0.0, "and intensity 0 is the layer, unshimmered");
    assert!(sweep(1.6, 0.0, 1.0).1 > 0.0, "a width of 0 divides by zero in the phase");
    assert!(sweep(1.6, -4.0, 1.0).1 > 0.0, "and so does a negative one, having flipped it first");

    // Several periods across ONE node is the tight end's whole point, so the
    // floor has to sit a good way under a node's diameter rather than merely
    // above zero.
    let node = ViewConfig::default().spacing * crate::NODE_RADIUS_FACTOR;
    let floor = sweep(1.6, 0.0, 1.0).1;
    assert!(
        floor * 4.0 < node * 2.0,
        "the width floor is {floor} against a node {} across: too coarse for the \
         pattern to cross one several times over",
        node * 2.0,
    );

    let (speed, width, intensity) = sweep(1e9, 1e9, 1e9);
    assert!(
        speed <= 40.0 && width <= 40.0 && intensity <= 4.0,
        "got {speed} / {width} / {intensity}",
    );

    assert_eq!(softness(0.35), 0.35, "a settable softness passes through untouched");
    assert_eq!(softness(4.0), 1.0, "and both ends stop at the bar: this one is an exponent");
    assert_eq!(softness(-1.0), 0.0, "including the low end, which would flatten the shape");
}

