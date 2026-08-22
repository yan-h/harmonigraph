//! Where the music has already been (see the [`trail`](crate::trail)
//! module). The claim under test throughout is that a memory reaches the
//! LABEL layer and nothing else — that is what keeps it from reading as a
//! note.

use crate::*;
use harmonigraph_core::{Envelope, NoteEvent, NoteEventKind, NoteTracker, Tuning};
use super::harness::*;

/// Play `note` from `on` to `off` and let its fade finish, which is what
/// moves it out of the live voices and into history.
fn play_and_forget(tracker: &mut NoteTracker, note: u8, on: f64, off: f64) {
    for (time, kind) in [
        (on, NoteEventKind::On { velocity: 1.0 }),
        (off, NoteEventKind::Off),
    ] {
        tracker.handle_event(NoteEvent { time, channel: 0, note, kind });
    }
    tracker.prune(off + 2.0, &Envelope::default());
}

/// The trail switched explicitly on or off. The kept past names ARE the
/// trail, so that one mode is its on/off — nothing fills `trail` under either
/// of the others.
fn trail_view(on: bool) -> ViewConfig {
    let note_names = if on { NoteNames::Past } else { NoteNames::Played };
    ViewConfig { note_names, ..ViewConfig::default() }
}

#[test]
fn nothing_is_remembered_under_any_mode_but_past() {
    let mut tracker = NoteTracker::new();
    play_and_forget(&mut tracker, 60, 0.0, 1.0);
    // Explicit modes rather than `ViewConfig::default()`: the fresh-view look
    // is Yan's, and is free to ship any of the three.
    //
    // All is in here because it is the one that looks like it should mark
    // something: it names every node, but off the label layer's own answer
    // rather than off a memory, so `trail` stays a record of where the music
    // has actually been.
    for names in [NoteNames::Played, NoteNames::All] {
        let view = ViewConfig { note_names: names, ..ViewConfig::default() };
        let scene = scene_of(&tracker, &Tuning::default(), &view, &plain_frame(), 10.0);
        assert!(scene.nodes.iter().all(|n| n.trail == 0.0), "{names:?} marked a node");
    }
}

#[test]
fn a_visited_node_is_marked_and_an_unvisited_one_is_not() {
    let mut tracker = NoteTracker::new();
    play_and_forget(&mut tracker, 60, 0.0, 1.0);
    let view = trail_view(true);
    let frame = plain_frame();
    let tuning = Tuning::default();

    // The mark holds indefinitely: the point is a whole piece's territory,
    // not a rolling window, so nothing ages out however long the piece runs.
    for now in [5.0, 600.0, 100_000.0] {
        let scene = scene_of(&tracker, &tuning, &view, &frame, now);
        assert_eq!(origin_node(&scene).trail, 1.0, "at t={now}");
    }
    let scene = scene_of(&tracker, &tuning, &view, &frame, 5.0);
    let elsewhere = scene
        .nodes
        .iter()
        .find(|n| n.lattice_pos == LatticePos::new(0, 1, 0))
        .unwrap();
    assert_eq!(elsewhere.trail, 0.0);
}

#[test]
fn a_memory_touches_no_field_but_trail() {
    // The load-bearing claim of the whole design. A trailed node must be
    // indistinguishable from an untouched one in EVERY other field, so no
    // amount of accumulated history can change the picture: the labels read
    // `trail`, and nothing that draws does.
    //
    // `color` is in the sweep deliberately. It is the one field a trail ever
    // wrote — a remembered node used to carry its note's color for the idle
    // marker to tint with — and with the marker gone that write would be a
    // silent hand on the disc's own channel.
    let mut tracker = NoteTracker::new();
    play_and_forget(&mut tracker, 60, 0.0, 1.0);
    let frame = plain_frame();
    let tuning = Tuning::default();
    let view = ViewConfig { extent_sevens: 1, ..ViewConfig::default() };
    let bare = scene_of(
        &tracker,
        &tuning,
        &ViewConfig { note_names: NoteNames::Played, ..view.clone() },
        &frame,
        10.0,
    );
    let marked = scene_of(
        &tracker,
        &tuning,
        &ViewConfig { note_names: NoteNames::Past, ..view.clone() },
        &frame,
        10.0,
    );
    assert!(marked.nodes.iter().any(|n| n.trail > 0.0), "nothing was remembered at all");

    for (a, b) in bare.nodes.iter().zip(&marked.nodes) {
        assert_eq!(a.color, b.color, "at {:?}", a.lattice_pos);
        assert_eq!(a.activation, b.activation, "at {:?}", a.lattice_pos);
        assert_eq!(a.octaves, b.octaves, "at {:?}", a.lattice_pos);
        assert_eq!(a.melody_slots, b.melody_slots);
        assert_eq!(a.bass_slots, b.bass_slots);
        assert_eq!(a.melody_level, b.melody_level);
        assert_eq!(a.bass_level, b.bass_level);
    }
    // The dot field stands at the same positions whatever is remembered
    // there, and pinning that is what says a memory is drawn in TYPE alone:
    // the one drawn layer under a trailed node has to be the one an unvisited
    // node stands on.
    assert_eq!(bare.dots.len(), marked.dots.len());
    for (a, b) in bare.dots.iter().zip(&marked.dots) {
        assert_eq!(a.strength, b.strength);
        assert_eq!(a.color, b.color);
        assert_eq!(a.radius, b.radius);
    }
}

#[test]
fn an_off_sheet_node_stays_blank_even_after_it_is_played() {
    // Trails live on the home sheet only. The history remembers the note
    // wherever it landed, but an off-sheet node stays blank — a lone name out
    // in the sevens dimension reads as noise, not as territory. The home node
    // of the same pitch class carries the memory instead.
    let view = ViewConfig { extent_sevens: 1, ..trail_view(true) };
    let tuning = Tuning::default();
    let frame = plain_frame();
    let off_sheet = LatticePos::new(0, 0, 1);
    let node_at = |scene: &Scene, pos: LatticePos| {
        *scene.nodes.iter().find(|n| n.lattice_pos == pos).unwrap()
    };

    let mut tracker = NoteTracker::new();
    // Bend a note onto the sevens node's pitch class (a harmonic seventh
    // above C under the default 12-TET axes).
    tracker.handle_event(NoteEvent::on(0.0, 0, 70, 1.0));
    tracker.handle_event(NoteEvent::off(1.0, 0, 70));
    tracker.prune(5.0, &Envelope::default());

    let scene = scene_of(&tracker, &tuning, &view, &frame, 10.0);
    // The off-sheet node is where the note was, but it draws nothing.
    let off = node_at(&scene, off_sheet);
    assert!(!off.on_home && off.trail == 0.0 && !off.is_visible(), "off-sheet stays blank");
    // The memory is not lost — it shows on the home sheet — and it shows there
    // ONLY: no off-sheet node is ever trailed.
    assert!(
        scene.nodes.iter().any(|n| n.on_home && n.trail > 0.0),
        "the home sheet carries the memory",
    );
    assert!(
        scene.nodes.iter().all(|n| n.trail == 0.0 || n.on_home),
        "a trail leaked onto an off-sheet node",
    );
}

#[test]
fn clearing_the_history_wipes_every_mark() {
    let mut tracker = NoteTracker::new();
    play_and_forget(&mut tracker, 60, 0.0, 1.0);
    play_and_forget(&mut tracker, 64, 2.0, 3.0);
    let view = trail_view(true);
    let frame = plain_frame();
    let tuning = Tuning::default();
    assert!(scene_of(&tracker, &tuning, &view, &frame, 6.0)
        .nodes
        .iter()
        .any(|n| n.trail > 0.0));

    tracker.clear_history();
    assert!(scene_of(&tracker, &tuning, &view, &frame, 6.0)
        .nodes
        .iter()
        .all(|n| n.trail == 0.0));
}
