//! Fixtures the suites share: a scene built from a tuning and a tracker,
//! and the lookups that find one node in it.

use crate::*;
use harmonigraph_core::{NoteEvent, NoteEventKind, NoteTracker, Tuning};

pub(super) fn scene_of(
    tracker: &NoteTracker,
    tuning: &Tuning,
    view: &ViewConfig,
    frame: &FrameParams,
    now: f64,
) -> Scene {
    derive_scene(tracker, tuning, view, frame, Camera::default(), None, now)
}

pub(super) fn origin_node(scene: &Scene) -> &NodeInstance {
    scene
        .nodes
        .iter()
        .find(|n| n.lattice_pos == LatticePos::ORIGIN)
        .unwrap()
}

/// A held note, so the off-sheet sevens links light up and appear in
/// the grid at all (idle ones are skipped as fully invisible).
pub(super) fn sounding() -> NoteTracker {
    let mut tracker = NoteTracker::new();
    tracker.handle_event(NoteEvent {
        time: 0.0,
        channel: 0,
        note: 60,
        kind: NoteEventKind::On { velocity: 1.0 },
    });
    tracker
}

/// A tracker with one note held from time 0.
pub(super) fn held(note: u8) -> NoteTracker {
    let mut tracker = NoteTracker::new();
    tracker.handle_event(NoteEvent {
        time: 0.0,
        channel: 0,
        note,
        kind: NoteEventKind::On { velocity: 1.0 },
    });
    tracker
}

pub(super) fn node_at(scene: &Scene, pos: LatticePos) -> &NodeInstance {
    scene.nodes.iter().find(|n| n.lattice_pos == pos).unwrap()
}
