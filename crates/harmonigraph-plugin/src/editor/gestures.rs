//! The host parameter gestures the editor holds open, in one registry.
//!
//! A gesture is a `begin_set_parameter` the host has been sent and whose
//! `end_set_parameter` it is owed; while one is open, Bitwig treats the
//! parameter as touched, and in Touch mode writes over its automation. Every
//! begin and end the editor makes goes through here, the camera's and the six
//! map-offset lanes' alike, so whatever ends the editor's hold on the host,
//! the window closing or the editor stopping being seen, ends every gesture
//! once ([`EditorLock::end_window_session`], [`EditorLock::end_unseen`]).
//!
//! Beginning and ending are idempotent. After the gestures were ended while
//! the editor was unseen, egui may still believe a drag is held on the
//! reveal's pass: a value that drag sets begins a new gesture first (the
//! camera's in `camera_movement::finish_edits`, a lane's at its next
//! [`MapEdit::Offset`](harmonigraph_ui::lattice_maps::MapEdit::Offset)), and
//! the drag's own end then ends that one, so no end is sent twice and no
//! value is written outside a gesture.
//!
//! [`EditorLock::end_window_session`]: super::EditorLock::end_window_session
//! [`EditorLock::end_unseen`]: super::EditorLock::end_unseen

use harmonigraph_ui::lattice_maps::{MapAxis, MapOffsetLane, LANES};
use harmonigraph_ui::params::ParamKey;

/// One host parameter the editor can hold a gesture on.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) enum Gesture {
    /// A camera parameter, or any other `ParamKey`.
    Param(ParamKey),
    /// A Lattice Map offset lane.
    Lane(MapAxis, MapOffsetLane),
}

impl Gesture {
    /// The host parameter the gesture is on.
    pub(crate) fn row(self) -> crate::Row {
        match self {
            Gesture::Param(key) => crate::Row::camera(key),
            Gesture::Lane(axis, lane) => crate::lattice_maps::lane_row(axis, lane),
        }
    }
}

/// Which gestures are open. A value, so the editor state holds it in a `Cell`
/// beside the frame's parameter backend, which is rebuilt every frame.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub(crate) struct Gestures {
    params: [bool; ParamKey::ALL.len()],
    lanes: [bool; LANES.len()],
}

impl Gestures {
    fn slot(&mut self, gesture: Gesture) -> &mut bool {
        match gesture {
            Gesture::Param(key) => &mut self.params[key as usize],
            Gesture::Lane(axis, lane) => {
                let index = LANES.iter().position(|&l| l == (axis, lane));
                &mut self.lanes[index.expect("every lane is in LANES")]
            }
        }
    }

    pub(crate) fn is_open(mut self, gesture: Gesture) -> bool {
        *self.slot(gesture)
    }

    /// Open `gesture`, answering whether it was closed: the host is owed a
    /// begin only then.
    pub(crate) fn begin(&mut self, gesture: Gesture) -> bool {
        !std::mem::replace(self.slot(gesture), true)
    }

    /// Close `gesture`, answering whether it was open: the host is owed an
    /// end only then.
    pub(crate) fn end(&mut self, gesture: Gesture) -> bool {
        std::mem::replace(self.slot(gesture), false)
    }

    /// Every open gesture, leaving none: what a close or a loss of sight ends.
    pub(crate) fn take(&mut self) -> Gestures {
        std::mem::take(self)
    }

    /// The open gestures, in a fixed order.
    pub(crate) fn open(self) -> impl Iterator<Item = Gesture> {
        let params = ParamKey::ALL.into_iter().filter(move |&key| self.params[key as usize]);
        let lanes = LANES.into_iter().zip(self.lanes).filter(|&(_, open)| open);
        params.map(Gesture::Param).chain(lanes.map(|((axis, lane), _)| Gesture::Lane(axis, lane)))
    }
}

/// Send the host the end of each gesture in `open`. Called with the editor
/// lock free: `end_set_parameter` is a host callback, and a host may save from
/// inside it, which takes that lock.
pub(crate) fn end(open: Gestures, host: &dyn nice_plug::prelude::GuiContext) {
    for gesture in open.open() {
        host.end_set_parameter(gesture.row().index());
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A begin and an end are owed once each, whatever repeats them; taking
    /// the open gestures leaves none, so a second drain, or the close after a
    /// loss of sight, ends nothing more; and a gesture begun again after a
    /// drain, as a drag still held at the reveal does, is owed a begin anew.
    #[test]
    fn each_gesture_is_begun_and_ended_once() {
        let yaw = Gesture::Param(ParamKey::CameraYaw);
        let lane = Gesture::Lane(MapAxis::Thirds, MapOffsetLane::Extension);
        let mut gestures = Gestures::default();
        assert!(gestures.begin(yaw));
        assert!(!gestures.begin(yaw), "a second begin is owed nothing");
        assert!(gestures.begin(lane));

        let drained = gestures.take();
        assert_eq!(drained.open().collect::<Vec<_>>(), [yaw, lane]);
        assert_eq!(gestures.take().open().count(), 0, "a second drain ends nothing");
        assert!(!gestures.end(yaw), "an end after the drain is owed nothing");

        assert!(gestures.begin(yaw), "a drag still held at the reveal begins anew");
        assert!(gestures.end(yaw));
        assert!(!gestures.is_open(yaw));
    }
}
