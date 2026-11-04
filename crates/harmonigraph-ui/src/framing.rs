//! Framing the lattice on the notes a piece plays: the Video pane's Frame
//! notes button over what has been drawn, and the offline renderer's Frame
//! lattice to notes over a whole take.
//!
//! The geometry is `harmonigraph_scene::framing`. This is the half that knows
//! the appearance — the camera's pan parameters and the frame's split — and
//! what counts as heard.

use glam::Vec2;
use harmonigraph_core::{LatticePos, NoteTracker, PitchClass, Tuning};
use harmonigraph_scene::framing;
use std::collections::BTreeSet;

use crate::{AppearanceDocument, RenderFrame};

/// Pitch classes heard, under the tuning each was heard under: a piece whose
/// tuning moves lights different nodes with the same pitch before and after.
#[derive(Default)]
pub struct Heard {
    by_tuning: Vec<(Tuning, BTreeSet<PitchClass>)>,
}

impl Heard {
    /// Everything the lattice has drawn since its history was last cleared
    /// (Clear display history): the remembered pitches the label trail draws,
    /// and the voices still sounding or fading. The history keeps the most
    /// recent [`NoteHistory::MAX_VISITS`](harmonigraph_core::NoteHistory::MAX_VISITS)
    /// distinct pitches, which a piece only passes by bending through them.
    pub fn drawn(tracker: &NoteTracker, tuning: Tuning) -> Heard {
        let mut heard = Heard::default();
        heard.hear(tuning, tracker.history().visits().map(|visit| visit.pitch_class));
        heard.hear(tuning, tracker.voices().map(|voice| voice.pitch_class));
        heard
    }

    pub fn hear(&mut self, tuning: Tuning, classes: impl IntoIterator<Item = PitchClass>) {
        let at = match self.by_tuning.iter().position(|(t, _)| *t == tuning) {
            Some(at) => at,
            None => {
                self.by_tuning.push((tuning, BTreeSet::new()));
                self.by_tuning.len() - 1
            }
        };
        self.by_tuning[at].1.extend(classes);
    }

    pub fn is_empty(&self) -> bool {
        self.by_tuning.iter().all(|(_, classes)| classes.is_empty())
    }

    /// The nodes these pitches light on `appearance`'s sheets, each the copy
    /// nearest where its camera stands (see [`framing::lit_positions`]).
    pub fn positions(&self, appearance: &AppearanceDocument) -> Vec<LatticePos> {
        let [_, _, _, fives, threes] = appearance.camera_movement();
        let mut positions: Vec<_> = self
            .by_tuning
            .iter()
            .flat_map(|(tuning, classes)| {
                let classes: Vec<_> = classes.iter().copied().collect();
                framing::lit_positions(&classes, tuning, &appearance.view, Vec2::new(fives, threes))
            })
            .collect();
        positions.sort_unstable_by_key(|p| (p.threes, p.fives, p.sevens));
        positions.dedup();
        positions
    }
}

impl AppearanceDocument {
    /// Pan and zoom the lattice camera onto `positions` and size the lattice's
    /// share of the frame to hold them: the camera pulls back until they fit
    /// the frame's full height (or width, beside a Top or Bottom lattice), and
    /// the split crops the other way to their shape. Orbit and projection are
    /// kept.
    ///
    /// `None` with nothing to frame, which changes nothing. Otherwise whether
    /// everything fits: only a piece wider than the camera's farthest zoom
    /// shows is cut off at its edges. One whose shape asks for more of the
    /// frame than [`RenderFrame::SPLIT_MAX`] gets the widest share, and the
    /// camera fits it to that share's width instead, with room the other way.
    pub fn frame_lattice(&mut self, positions: &[LatticePos]) -> Option<bool> {
        let frame = self.render.frame;
        let aspect = frame.aspect_w.max(1) as f32 / frame.aspect_h.max(1) as f32;
        let across = frame.lattice.sizes_by_height();
        let pane = |split: f32| if across { aspect / split } else { aspect * split };
        // The content's shape is the camera's to say, and it does not depend
        // on the pane it is framed in except under perspective, where the
        // pane it will actually fill is the better one to measure in.
        let shape = framing::frame(self.camera, &self.view, positions, pane(frame.split))?.aspect;
        let wanted = if across { aspect / shape } else { shape / aspect };
        let split = wanted.clamp(RenderFrame::SPLIT_MIN, RenderFrame::SPLIT_MAX);
        let framed = framing::frame(self.camera, &self.view, positions, pane(split))?;
        self.render.frame.split = split;
        let center = self.view.center();
        let camera = framed.camera;
        self.set_camera_movement([
            camera.yaw,
            camera.pitch,
            camera.distance,
            center.fives as f32 + camera.target.x,
            center.threes as f32 + camera.target.y,
        ]);
        Some(framed.fits)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::LatticeSide;
    use harmonigraph_core::{NoteEvent, SourceId};

    /// Taller than wide, for a lattice beside the spectral pane; wider than
    /// tall, for one above or below it. Each crops its frame to its own shape.
    fn piece(side: LatticeSide) -> [LatticePos; 2] {
        if side.sizes_by_height() {
            [LatticePos::new(0, -5, 0), LatticePos::new(1, 5, 0)]
        } else {
            [LatticePos::new(-2, 0, 0), LatticePos::new(2, 1, 0)]
        }
    }

    /// The camera fills the frame's long way round and the split crops the
    /// other to the piece's shape, on every side; a piece that wants more
    /// than the widest share gets it and is fitted to its width.
    #[test]
    fn framing_sizes_the_split_to_the_piece_on_every_side() {
        let aspect = 16.0 / 9.0;
        for side in LatticeSide::ALL {
            let mut look = AppearanceDocument::default();
            look.render.frame.lattice = side;
            assert_eq!(look.frame_lattice(&piece(side)), Some(true), "{side:?}");
            let split = look.render.frame.split;
            assert!(split > RenderFrame::SPLIT_MIN && split < RenderFrame::SPLIT_MAX, "{split}");
            let pane = if side.sizes_by_height() { aspect / split } else { aspect * split };
            let reframed =
                framing::frame(look.camera, &look.view, &piece(side), pane).expect("still frames");
            assert!((reframed.aspect - pane).abs() < 1e-3, "{side:?}: {reframed:?} in {pane}");

            let crosswise =
                if side.sizes_by_height() { LatticeSide::Left } else { LatticeSide::Top };
            let mut look = AppearanceDocument::default();
            look.render.frame.lattice = side;
            assert_eq!(look.frame_lattice(&piece(crosswise)), Some(true), "{side:?}");
            assert_eq!(look.render.frame.split, RenderFrame::SPLIT_MAX, "{side:?}");
        }
        assert_eq!(AppearanceDocument::default().frame_lattice(&[]), None);
    }

    /// The framing is written through the pan parameters' shape, so the
    /// window's center follows it and the camera parameters carry it.
    #[test]
    fn a_framed_camera_is_what_its_parameters_say() {
        let mut look = AppearanceDocument::default();
        let far = [LatticePos::new(6, 10, 0), LatticePos::new(8, 14, 0)];
        look.frame_lattice(&far).unwrap();
        let [_, _, _, fives, threes] = look.camera_movement();
        assert!((fives - 12.0).abs() < 1e-3 && (threes - 7.0).abs() < 1e-3, "{fives} {threes}");
        assert_eq!((look.view.center_fives, look.view.center_threes), (12, 7));
        assert!(look.camera.target.truncate().abs().max_element() <= 0.5);
    }

    /// What the lattice remembers and what is still sounding are both heard;
    /// Clear display history forgets the first.
    #[test]
    fn heard_is_the_history_and_the_voices() {
        let mut tracker = NoteTracker::new();
        let env = harmonigraph_core::Envelope::default();
        tracker.handle_event(NoteEvent::on(0.0, SourceId::DIRECT, 0, 60, 0.8));
        tracker.handle_event(NoteEvent::off(0.1, SourceId::DIRECT, 0, 60));
        tracker.prune(100.0, &env);
        tracker.handle_event(NoteEvent::on(100.0, SourceId::DIRECT, 0, 64, 0.8));
        let look = AppearanceDocument::default();
        let tuning = Tuning::default();
        let positions = Heard::drawn(&tracker, tuning).positions(&look);
        assert_eq!(positions.len(), 2, "{positions:?}");
        tracker.clear_history();
        assert_eq!(Heard::drawn(&tracker, tuning).positions(&look).len(), 1);
    }
}
