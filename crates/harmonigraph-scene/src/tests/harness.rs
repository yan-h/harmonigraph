//! Fixtures the suites share: a scene built from a tuning and a tracker,
//! and the lookups that find one node in it.

use crate::*;
use harmonigraph_core::{NoteEvent, NoteTracker, Tuning};

pub(super) fn scene_of(
    tracker: &NoteTracker,
    tuning: &Tuning,
    view: &ViewConfig,
    frame: &FrameParams,
    now: f64,
) -> Scene {
    derive_scene(tracker, tuning, view, &view.reach(), frame, Camera::default(), None, now)
}

/// [`ViewConfig::default`] with the note envelope pinned flat: a straight-line
/// curve, so half a duration in reads half way along, and no mark Delay, so a
/// ring is part of the note's own arrival rather than a layer that answers
/// later. Its other half is [`plain_frame`], which holds the duration.
///
/// The suites that spread this are about what a SOUNDING note draws — the
/// gutter it clears, the marker it cuts, the end it marks, what is left of it
/// mid-release — and they say so by sampling at time 0 and by naming levels
/// in fractions. Under the default envelope neither holds: time 0 is the one
/// instant a note is guaranteed not to be drawn yet, and a curved fade is not
/// half gone at half way.
///
/// Pinning it keeps each of them measuring its own subject instead of the
/// envelope, and keeps them from turning red the day the default envelope is
/// retuned — which is a look, and looks move. The envelope is tested where it
/// lives: the curve in `harmonigraph_core::notes`, and its reach into these
/// layers in `a_fresh_mark_eases_in_with_the_octave_it_links_to`.
pub(super) fn plain_view() -> ViewConfig {
    ViewConfig { fade_shape: 0.0, mark_delay: 0.0, ..ViewConfig::default() }
}

/// The frame half of the flat fixture: an envelope duration of 0, so a note
/// is fully lit on the frame it sounds and gone on the frame it is released.
///
/// One duration drives both ends ([`ViewConfig::envelope`]), so "no arrival"
/// and "no fade" are the same number and neither can be pinned from the view
/// side. A test that wants a fade to sample along says so by naming
/// a `fade_time` of its own — and buys an arrival of that length with it,
/// which is why several of them start their note before the window they
/// measure.
pub(super) fn plain_frame() -> FrameParams {
    FrameParams { fade_time: 0.0, ..FrameParams::default() }
}

pub(super) fn origin_node(scene: &Scene) -> &NodeInstance {
    scene
        .nodes
        .iter()
        .find(|n| n.lattice_pos == LatticePos::ORIGIN)
        .unwrap()
}

/// A held note, for the suites that need something sounding to measure.
pub(super) fn sounding() -> NoteTracker {
    let mut tracker = NoteTracker::new();
    tracker.handle_event(NoteEvent::on(0.0, 0, 60, 1.0));
    tracker
}

/// A tracker with one note held from time 0.
pub(super) fn held(note: u8) -> NoteTracker {
    let mut tracker = NoteTracker::new();
    tracker.handle_event(NoteEvent::on(0.0, 0, note, 1.0));
    tracker
}

pub(super) fn node_at(scene: &Scene, pos: LatticePos) -> &NodeInstance {
    scene.nodes.iter().find(|n| n.lattice_pos == pos).unwrap()
}
