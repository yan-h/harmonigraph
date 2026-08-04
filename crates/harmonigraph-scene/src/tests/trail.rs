//! Where the music has already been (see the [`trail`](crate::trail)
//! module). The claim under test throughout is that a memory reaches the
//! IDLE layer and nothing else — that is what keeps it from reading as a
//! note.

use crate::*;
use harmonigraph_core::{NoteEvent, NoteEventKind, NoteTracker, Tuning};
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
    tracker.prune(off + 2.0, 1.0);
}

fn trail_view(mark: TrailMark) -> ViewConfig {
    ViewConfig { trail_mark: mark, ..ViewConfig::default() }
}

#[test]
fn nothing_is_remembered_while_the_trail_is_off() {
    let mut tracker = NoteTracker::new();
    play_and_forget(&mut tracker, 60, 0.0, 1.0);
    // Explicitly off rather than `ViewConfig::default()`: the fresh-view
    // look is Yan's, and is free to ship the trail on.
    let view = trail_view(TrailMark::Off);
    let scene = scene_of(&tracker, &Tuning::default(), &view, &FrameParams::default(), 10.0);
    assert!(scene.nodes.iter().all(|n| n.trail == 0.0));
}

#[test]
fn a_visited_node_is_marked_and_an_unvisited_one_is_not() {
    let mut tracker = NoteTracker::new();
    play_and_forget(&mut tracker, 60, 0.0, 1.0);
    let view = trail_view(TrailMark::Lift);
    let frame = FrameParams::default();
    let tuning = Tuning::default();

    // With no memory span the mark holds indefinitely: the point is a whole
    // piece's territory, not a rolling window.
    assert_eq!(view.trail_memory, 0.0, "never forgets by default");
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
fn a_memory_never_touches_any_layer_that_means_is_sounding() {
    // The load-bearing claim of the whole design. A trailed node must be
    // indistinguishable from an untouched one in every active channel, so
    // no amount of accumulated history can brighten the lattice.
    let mut tracker = NoteTracker::new();
    play_and_forget(&mut tracker, 60, 0.0, 1.0);
    let frame = FrameParams::default();
    let tuning = Tuning::default();
    let view = ViewConfig { extent_sevens: 1, ..ViewConfig::default() };
    let bare = scene_of(&tracker, &tuning, &view, &frame, 10.0);
    let marked = scene_of(
        &tracker,
        &tuning,
        &ViewConfig { trail_mark: TrailMark::Tint, ..view.clone() },
        &frame,
        10.0,
    );

    for (a, b) in bare.nodes.iter().zip(&marked.nodes) {
        assert_eq!(a.activation, b.activation, "at {:?}", a.lattice_pos);
        assert_eq!(a.octaves, b.octaves, "at {:?}", a.lattice_pos);
        assert_eq!(a.melody_slots, b.melody_slots);
        assert_eq!(a.bass_slots, b.bass_slots);
        assert_eq!(a.melody_level, b.melody_level);
        assert_eq!(a.bass_level, b.bass_level);
    }
    // The grid is derived from those same activations, so it follows — but
    // pin it too, since it is the layer a trail would most plausibly leak
    // into (the sevens chains read a node's activation and color).
    assert_eq!(bare.grid.len(), marked.grid.len());
    for (a, b) in bare.grid.iter().zip(&marked.grid) {
        assert_eq!(a.strength, b.strength);
        assert_eq!(a.color, b.color);
    }
}

#[test]
fn tint_puts_the_remembered_color_on_a_silent_node_only() {
    let mut tracker = NoteTracker::new();
    play_and_forget(&mut tracker, 60, 0.0, 1.0);
    let frame = FrameParams::default();
    let tuning = Tuning::default();

    // Silent and visited: `color` carries the remembered note's color for
    // the shader's idle branch to tint with, in place of the idle grey.
    let scene = scene_of(&tracker, &tuning, &trail_view(TrailMark::Tint), &frame, 10.0);
    let remembered = origin_node(&scene).color;
    // Against a trail that is OFF, which is what leaves the idle grey in
    // place -- every mark rewrites a silent visited node's color.
    let idle = origin_node(&scene_of(
        &tracker,
        &tuning,
        &trail_view(TrailMark::Off),
        &frame,
        10.0,
    ))
    .color;
    assert_ne!(remembered, idle);
    assert_eq!(
        remembered,
        channel_color(0, 60.0, frame.darkest_pitch, frame.brightest_pitch, PitchPalette::Ramp)
    );

    // Sounding again: the live note owns `color`, memory or not.
    tracker.handle_event(NoteEvent {
        time: 10.0,
        channel: 4,
        note: 60,
        kind: NoteEventKind::On { velocity: 1.0 },
    });
    let scene = scene_of(&tracker, &tuning, &trail_view(TrailMark::Tint), &frame, 10.5);
    let node = origin_node(&scene);
    assert_eq!(node.activation, 1.0);
    assert_eq!(node.trail, 1.0, "still remembered");
    assert_eq!(
        node.color,
        channel_color(4, 60.0, frame.darkest_pitch, frame.brightest_pitch, PitchPalette::Ramp),
        "the sounding voice's own color, not the memory's"
    );
}

#[test]
fn a_memory_span_fades_a_pitch_out_and_then_drops_it() {
    let mut tracker = NoteTracker::new();
    play_and_forget(&mut tracker, 60, 0.0, 1.0);
    let view = ViewConfig { trail_memory: 100.0, ..trail_view(TrailMark::Ring) };
    let frame = FrameParams::default();
    let tuning = Tuning::default();
    let trail_at = |now: f64| origin_node(&scene_of(&tracker, &tuning, &view, &frame, now)).trail;

    // Measured from the release, so the mark takes over at full strength.
    assert!((trail_at(1.0) - 1.0).abs() < 1e-6);
    assert!((trail_at(51.0) - 0.5).abs() < 1e-5);
    assert_eq!(trail_at(101.0), 0.0);
    assert_eq!(trail_at(10_000.0), 0.0);
}

#[test]
fn an_off_sheet_node_stays_blank_even_after_it_is_played() {
    // Trails live on the home sheet only. The history remembers the note
    // wherever it landed, but an off-sheet node stays blank — a lone mark out
    // in the sevens dimension reads as noise, not as territory. The home node
    // of the same pitch class carries the memory instead.
    let view = ViewConfig { extent_sevens: 1, ..trail_view(TrailMark::Ring) };
    let tuning = Tuning::default();
    let frame = FrameParams::default();
    let off_sheet = LatticePos::new(0, 0, 1);
    let node_at = |scene: &Scene, pos: LatticePos| {
        *scene.nodes.iter().find(|n| n.lattice_pos == pos).unwrap()
    };

    let mut tracker = NoteTracker::new();
    // Bend a note onto the sevens node's pitch class (a harmonic seventh
    // above C under the default 12-TET axes).
    tracker.handle_event(NoteEvent {
        time: 0.0,
        channel: 0,
        note: 70,
        kind: NoteEventKind::On { velocity: 1.0 },
    });
    tracker.handle_event(NoteEvent { time: 1.0, channel: 0, note: 70, kind: NoteEventKind::Off });
    tracker.prune(5.0, 1.0);

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
    let view = trail_view(TrailMark::Lift);
    let frame = FrameParams::default();
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
