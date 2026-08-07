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

/// [`ViewConfig::default`] with the note envelope pinned flat: no attack, so
/// a note is fully lit on the frame it sounds, a straight-line fade, so half a
/// fade time in reads half gone, and no mark Delay, so a ring is part of that
/// same instant arrival rather than a layer that answers later.
///
/// The suites that spread this are about what a SOUNDING note draws — the
/// gutter it clears, the grid it cuts, the outline its channel gives it, the
/// end it marks, what is left of it mid-release — and they say so by sampling
/// at time 0 and by naming levels in fractions. Under the default view's
/// envelope neither holds: time 0 is the one instant a note is guaranteed not
/// to be drawn yet, and a curved fade is not half gone at half way.
///
/// Pinning it keeps each of them measuring its own subject instead of the
/// envelope, and keeps them from turning red the day the default envelope is
/// retuned — which is a look, and looks move. The envelope is tested where it
/// lives: the curve in `harmonigraph_core::notes`, and its reach into these
/// layers in `a_fresh_mark_eases_in_with_the_octave_it_links_to`.
pub(super) fn plain_view() -> ViewConfig {
    ViewConfig { attack_time: 0.0, fade_shape: 0.0, mark_delay: 0.0, ..ViewConfig::default() }
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
