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
    let view = ViewConfig { mark_melody, mark_bass, ..ViewConfig::default() };
    scene_of(&tracker, &Tuning::default(), &view, &FrameParams::default(), 0.0)
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
fn a_released_note_drops_its_mark_while_the_held_note_keeps_the_live_one() {
    // C4 and C5 share a pitch class, so they light ONE node. Release the
    // top one: the node stays fully lit by the held C4, and C4 — now the
    // only held note — takes BOTH live ends. The released C5's mark does not
    // linger: it snaps off with the key even as its octave glyph keeps
    // fading, so the marks never disagree about which notes are down.
    let mut tracker = NoteTracker::new();
    for note in [60u8, 72] {
        tracker.handle_event(NoteEvent {
            time: 0.0,
            channel: 0,
            note,
            kind: NoteEventKind::On { velocity: 1.0 },
        });
    }
    tracker.handle_event(NoteEvent { time: 0.0, channel: 0, note: 72, kind: NoteEventKind::Off });
    let frame = FrameParams { fade_time: 2.0, ..FrameParams::default() };
    let view = ViewConfig {
        mark_melody: true,
        mark_bass: true,
        ..ViewConfig::default()
    };
    let scene = scene_of(&tracker, &Tuning::default(), &view, &frame, 1.0);
    let origin = origin_node(&scene);
    assert_eq!(origin.activation, 1.0, "the held C4 keeps the node lit");
    // Only the held C4's slot is marked, at both ends; the released C5's
    // slot carries no mark, and the mark level is at full, not fading.
    assert_eq!(origin.melody_slots, 1 << MIDDLE_C_SLOT, "only the held C4 is the melody");
    assert_eq!(origin.bass_slots, 1 << MIDDLE_C_SLOT, "and the bass");
    assert_eq!(origin.melody_level, 1.0, "the held mark is at full, not mid-fade");
    // The octave glyph for the released C5 still fades on its own envelope —
    // it is only the RING that snaps off, not the disc or the glyph.
    assert!(
        (origin.octaves[MIDDLE_C_SLOT + 1] - 0.5).abs() < 1e-5,
        "the released C5's octave is half-faded, got {}",
        origin.octaves[MIDDLE_C_SLOT + 1]
    );
    assert_eq!(origin.octaves[MIDDLE_C_SLOT], 1.0, "the held C4's octave is at full");
}

#[test]
fn a_fresh_mark_eases_in_with_the_octave_it_links_to() {
    // A ring arriving at full the frame its note claims an end is the
    // jumpiest thing on the node, since the octave sector underneath it
    // eases in. Both ride the one ramp, so a note's outer layer arrives
    // as a single gesture.
    let mut tracker = NoteTracker::new();
    tracker.handle_event(NoteEvent {
        time: 0.0,
        channel: 0,
        note: 60, // C4: the origin node, in middle C's octave slot
        kind: NoteEventKind::On { velocity: 1.0 },
    });
    let view = ViewConfig {
        mark_melody: true,
        mark_bass: true,
        ..ViewConfig::default()
    };
    let at = |now: f64| {
        let scene = scene_of(&tracker, &Tuning::default(), &view, &FrameParams::default(), now);
        let n = origin_node(&scene);
        (n.melody_level, n.bass_level, n.octaves[MIDDLE_C_SLOT])
    };

    assert_eq!(at(0.0), (0.0, 0.0, 0.0), "nothing has arrived on the frame itself");

    let (melody, bass, octave) = at(ATTACK_TIME * 0.5);
    assert!((melody - 0.5).abs() < 1e-5, "half way in, got {melody}");
    assert_eq!(melody, bass, "a lone note's two ends arrive together");
    assert_eq!(melody, octave, "the ring rides the sector's own ramp");

    assert_eq!(at(ATTACK_TIME), (1.0, 1.0, 1.0), "full by the end of the attack");
}

#[test]
fn an_inherited_end_eases_in_from_the_handoff_not_from_its_note_on() {
    // Hold C4 and C5, then lift the top: the melody drops to C4, whose own
    // note-on is long past. Easing from THAT would be no ease at all — the
    // ring has to grow from the moment it moved. C4's bass ring never
    // changed hands, so it stays at full right through the handoff.
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
    let view = ViewConfig {
        mark_melody: true,
        mark_bass: true,
        ..ViewConfig::default()
    };
    // Long enough that the released C5 is still in the tracker, which is
    // where the handoff moment is read from.
    let frame = FrameParams { fade_time: 2.0, ..FrameParams::default() };
    let at = |now: f64| {
        let scene = scene_of(&tracker, &Tuning::default(), &view, &frame, now);
        let n = origin_node(&scene);
        (n.melody_level, n.bass_level)
    };

    assert_eq!(at(1.0), (0.0, 1.0), "the melody has only just moved");
    let (melody, bass) = at(1.0 + ATTACK_TIME * 0.5);
    assert!((melody - 0.5).abs() < 1e-5, "half way in, got {melody}");
    assert_eq!(bass, 1.0, "the end that never moved does not re-attack");
    assert_eq!(at(1.0 + ATTACK_TIME), (1.0, 1.0));
}

#[test]
fn held_extremes_never_names_a_released_voice() {
    // A released voice wears no mark at all (above), and it is likewise out
    // of the running for the LIVE ends: letting it stay "the melody" would
    // steal that from the note that actually replaced it.
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
    assert_eq!(melody, Some((0, 60)), "the released G must not stay the melody");
    assert_eq!(bass, Some((0, 60)));

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
        ..ViewConfig::default()
    };
    let frame = FrameParams::default();
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
fn sector_color(node: &NodeInstance, slot: u32, frame: &FrameParams) -> Vec4 {
    let pitch = slot as f32 * 12.0 + node.cents / 100.0;
    let lut = pitch_ramp_lut();
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
    assert_eq!(node.color, channel_color(0, 60.0, frame.darkest_pitch, frame.brightest_pitch));
    assert_ne!(node.color, node.melody_color, "the disc keeps the channel's own color");
}


/// The mark rings' pulse folds off with the rings themselves.
///
/// Every mode animates the ring, so a thickness of 0 — the rings' documented
/// off position, where `mark_ring` returns no coverage — leaves all of them
/// with nothing to animate, and the pane grays the row there.
///
/// [`Pulse::Shimmer`] is the one that needs the fold: it also sweeps the
/// octave SLICE a ring points at, which the glyph layer draws and no ring
/// coverage multiplies away, so without this, switching the rings off leaves
/// the marked octaves sweeping from a control the user can no longer reach to
/// stop. The other modes are folded on the same grounds rather than because
/// they misbehave — which mode is safe to leave standing is a fact about
/// where each one draws, and pinning that to today's answer would make a mode
/// reaching past the ring later a silent bug rather than an edit here.
/// The rings go off two ways, and BOTH have to fold the mode: no thickness to
/// draw one with, and no end marked for one to belong to
/// ([`ViewConfig::mark_rings_draw`], which the pane grays the row on). The
/// marks-off half is the easier one to leave out, because nothing is visibly
/// wrong when it is: no slot is marked, so `in.marks` is 0 and every term the
/// mode drives collapses to zero on its own. That is the accident this fold
/// exists not to depend on.
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
            &FrameParams::default(),
            0.0,
        )
        .pulse_marks
    };

    for mode in [Pulse::Together, Pulse::Alternating, Pulse::Shimmer] {
        assert_eq!(
            pulse(0.0, true, mode),
            Pulse::Off,
            "{mode:?} survived the ring thickness going to 0, and Shimmer at least \
             keeps drawing there -- on the marked octave's own slice",
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

/// The shimmer's three settings reach the scene, and the width arrives
/// strictly positive however the view is set: the shader divides the band
/// phase by it, so a 0 here is a whole lattice of NaN rather than a
/// stationary sweep. (Speed 0 IS the stationary sweep, and passes through, as
/// intensity 0 is the layer drawn unshimmered.)
///
/// The width's FLOOR is the second claim, and it is a look rather than a
/// safety margin: it has to leave room for several bands across one node, so
/// it is checked against the node's own world size (`spacing` ×
/// `NODE_RADIUS_FACTOR`) rather than against a bare "> 0".
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
            &FrameParams::default(),
            0.0,
        );
        (scene.shimmer_speed, scene.shimmer_width, scene.shimmer_intensity)
    };

    assert_eq!(sweep(3.0, 8.0, 0.5), (3.0, 8.0, 0.5), "a settable set passes through untouched");
    assert_eq!(sweep(0.0, 5.0, 0.0).0, 0.0, "speed 0 is a look -- the sheet, held still");
    assert_eq!(sweep(1.6, 5.0, 0.0).2, 0.0, "and intensity 0 is the layer, unshimmered");
    assert!(sweep(1.6, 0.0, 1.0).1 > 0.0, "a width of 0 divides by zero in the band phase");
    assert!(sweep(1.6, -4.0, 1.0).1 > 0.0, "and so does a negative one, having flipped it first");

    // Several bands across ONE node is the tight end's whole point, so the
    // floor has to sit a good way under a node's diameter rather than merely
    // above zero.
    let node = ViewConfig::default().spacing * crate::NODE_RADIUS_FACTOR;
    let floor = sweep(1.6, 0.0, 1.0).1;
    assert!(
        floor * 4.0 < node * 2.0,
        "the width floor is {floor} against a node {} across: too coarse for the \
         bands to cross one several at a time",
        node * 2.0,
    );

    let (speed, width, intensity) = sweep(1e9, 1e9, 1e9);
    assert!(
        speed <= 40.0 && width <= 40.0 && intensity <= 4.0,
        "got {speed} / {width} / {intensity}",
    );
}

/// Only ALTERNATING needs a marked slot. It is the mode whose two phases
/// differ, so with neither mark on `extreme_slots` is 0, every glyph takes
/// the "rest" phase, and what was a half-cycle split becomes a UNIFORM
/// breathe of the whole octave layer between the shader's pulse floor and
/// full — a mode doing something other than what it says, from a control the
/// pane has grayed out and that therefore cannot be switched off. Folding it
/// here is what keeps the picture and the grayed control agreeing.
///
/// Together is NOT that mode, however much the pairing of the two names
/// suggests it is. `pulse_pair` gives it `near == rest` (both phases are
/// `pulse_wave(0.0)`), so its picture is a uniform breathe of the whole layer
/// whether or not anything is marked — the marked slot changes nothing to
/// collapse. Folding it would take away a look that is reachable and steady,
/// on the grounds that a split it never had has gone.
///
/// Shimmer sweeps the layer whole and never wanted a slot either.
#[test]
fn only_the_alternating_octave_breathe_needs_a_marked_end() {
    let breathe = |mark_melody: bool, mark_bass: bool, pulse: Pulse| {
        let mut tracker = NoteTracker::new();
        tracker.handle_event(NoteEvent {
            time: 0.0,
            channel: 0,
            note: 60,
            kind: NoteEventKind::On { velocity: 1.0 },
        });
        let view = ViewConfig {
            mark_melody,
            mark_bass,
            pulse_octaves: pulse,
            ..ViewConfig::default()
        };
        scene_of(&tracker, &Tuning::default(), &view, &FrameParams::default(), 0.0).pulse_octaves
    };

    assert_eq!(
        breathe(false, false, Pulse::Alternating),
        Pulse::Off,
        "Alternating has no slot to single out with neither end marked, and would \
         breathe the whole octave layer from a control the pane has grayed out",
    );
    // ...and one mark is enough to give it its two parts back.
    let alt = Pulse::Alternating;
    assert_eq!(breathe(true, false, alt), alt, "the melody mark alone is a slot");
    assert_eq!(breathe(false, true, alt), alt, "so is the bass mark alone");

    // The two modes that draw the same picture marked or not have to reach
    // that picture either way.
    for whole_layer in [Pulse::Together, Pulse::Shimmer] {
        assert_eq!(
            breathe(false, false, whole_layer),
            whole_layer,
            "{whole_layer:?} animates the layer as one and never asked for a marked \
             slot, so nothing about it collapses when both marks come off",
        );
        assert_eq!(breathe(true, true, whole_layer), whole_layer, "and it survives marks too");
    }
}
