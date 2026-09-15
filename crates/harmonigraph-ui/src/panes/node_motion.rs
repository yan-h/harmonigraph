//! One entrance per continuous visible node, shared by editor and offline panes.
//! Voice ownership can change without starting a second geometric gesture.
use std::collections::HashMap;

use harmonigraph_core::LatticePos;
use harmonigraph_scene::Scene;

#[derive(Clone, Default)]
pub(crate) struct NodeMotion {
    nodes: HashMap<LatticePos, Entrance>,
    at: Option<f64>,
}

#[derive(Clone, Copy)]
struct Entrance {
    started: f64,
    progress: f32,
    seen: bool,
}

pub(super) fn apply(scene: &mut Scene, state: &mut crate::PictureState, surface: usize, now: f64) {
    state.surfaces.node_motion.entry(surface).or_default().step(
        scene,
        state.runtime.frame_params.fade_time,
        now,
    );
}

impl NodeMotion {
    fn step(&mut self, scene: &mut Scene, duration: f32, now: f64) {
        if !now.is_finite() {
            return;
        }
        if self.at.is_some_and(|at| now < at) {
            self.nodes.clear();
        }
        self.at = Some(now);
        for entry in self.nodes.values_mut() {
            entry.seen = false;
        }
        let duration = f64::from(duration.max(0.0));
        let audio_draws = scene.spectral.ring_draws();
        for node in &mut scene.nodes {
            let present = node.transition_live
                || node.activation > 0.0
                || node.melody_level > 0.0
                || node.bass_level > 0.0
                || (audio_draws && node.audio_ring > 0.0);
            if !present {
                continue;
            }
            let entry = self.nodes.entry(node.lattice_pos).or_insert_with(|| {
                let progress = if node.transition_live && node.transition_phase >= 0.0 {
                    f64::from(node.transition_phase)
                } else {
                    1.0
                };
                Entrance {
                    started: now - progress * duration,
                    progress: progress as f32,
                    seen: true,
                }
            });
            entry.seen = true;
            // Timing edits affect an unfinished entrance without winding it
            // backward; once settled, increasing the dial cannot replay it.
            let arrival = if duration > 0.0 {
                ((now - entry.started) / duration).clamp(0.0, 1.0) as f32
            } else {
                1.0
            };
            entry.progress = entry.progress.max(arrival);
            let arrival = entry.progress;
            // The original onset survives a new octave, same-key replacement,
            // and pruning of its voice. A new illumination on a settled node
            // stays settled; only a departure may contract that geometry.
            node.transition_phase = if arrival < 1.0 {
                arrival
            } else if node.transition_phase < 0.0 {
                node.transition_phase
            } else {
                1.0
            };
        }
        // Visibility bounds the map, independently of optional Glow and of
        // the selector. Same-time layout passes neither advance nor restart it.
        self.nodes.retain(|_, entry| entry.seen);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use harmonigraph_core::{NoteEvent, NoteTracker, SourceId, Tuning};
    use harmonigraph_scene::{derive_scene, Camera, FrameParams, ViewConfig};

    fn phase(motion: &mut NodeMotion, tracker: &mut NoteTracker, now: f64, audio: bool) -> f32 {
        let view = ViewConfig { fade_shape: 0.0, mark_delay: 0.0, ..Default::default() };
        let frame = FrameParams { fade_time: 1.0, ..Default::default() };
        tracker.prune(now, &view.envelope(&frame));
        let mut scene = derive_scene(
            tracker,
            &Tuning::default(),
            &view,
            &view.reach(),
            &frame,
            Camera::default(),
            None,
            now,
        );
        if audio {
            scene.spectral.inner = 0.1;
            scene.spectral.outer = 0.4;
            for node in &mut scene.nodes {
                node.audio_ring = 1.0;
            }
        }
        motion.step(&mut scene, 1.0, now);
        scene.nodes.iter().find(|n| n.lattice_pos == LatticePos::ORIGIN).unwrap().transition_phase
    }

    #[test]
    fn existing_node_does_not_replay_for_new_octaves_retriggers_or_old_voice_pruning() {
        let mut tracker = NoteTracker::new();
        let mut motion = NodeMotion::default();
        tracker.handle_event(NoteEvent::on(0.0, SourceId::DIRECT, 0, 60, 1.0));
        assert_eq!(phase(&mut motion, &mut tracker, 0.0, false), 0.0);
        assert_eq!(phase(&mut motion, &mut tracker, 0.5, false), 0.5);
        assert_eq!(phase(&mut motion, &mut tracker, 1.0, false), 1.0);
        tracker.handle_event(NoteEvent::on(1.2, SourceId::DIRECT, 0, 72, 1.0));
        assert_eq!(phase(&mut motion, &mut tracker, 1.2, false), 1.0);
        tracker.handle_event(NoteEvent::on(1.3, SourceId::DIRECT, 0, 60, 1.0));
        assert_eq!(phase(&mut motion, &mut tracker, 1.3, false), 1.0);
        tracker.handle_event(NoteEvent::off(1.4, SourceId::DIRECT, 0, 60));
        assert_eq!(phase(&mut motion, &mut tracker, 3.5, false), 1.0);
        // The same-key zero-opacity onset must not masquerade as disappearance.
        tracker.handle_event(NoteEvent::on(3.6, SourceId::DIRECT, 0, 72, 1.0));
        assert_eq!(phase(&mut motion, &mut tracker, 3.6, false), 1.0);
    }

    #[test]
    fn contraction_does_not_retrigger_and_only_disappearance_rearms_the_entrance() {
        let mut tracker = NoteTracker::new();
        let mut motion = NodeMotion::default();
        tracker.handle_event(NoteEvent::on(0.0, SourceId::DIRECT, 0, 60, 1.0));
        phase(&mut motion, &mut tracker, 0.0, false);
        phase(&mut motion, &mut tracker, 1.0, false);
        tracker.handle_event(NoteEvent::off(2.0, SourceId::DIRECT, 0, 60));
        assert_eq!(phase(&mut motion, &mut tracker, 2.5, false), -0.5);
        tracker.handle_event(NoteEvent::on(2.6, SourceId::DIRECT, 0, 60, 1.0));
        assert_eq!(phase(&mut motion, &mut tracker, 2.6, false), 1.0);
        tracker.handle_event(NoteEvent::off(3.8, SourceId::DIRECT, 0, 60));
        phase(&mut motion, &mut tracker, 4.9, false);
        assert!(motion.nodes.is_empty());
        tracker.handle_event(NoteEvent::on(5.0, SourceId::DIRECT, 0, 60, 1.0));
        assert_eq!(phase(&mut motion, &mut tracker, 5.25, false), 0.25);
        assert_eq!(phase(&mut motion, &mut tracker, 5.25, false), 0.25);
    }

    #[test]
    fn an_existing_audio_node_acquires_midi_without_a_pop() {
        let mut tracker = NoteTracker::new();
        let mut motion = NodeMotion::default();
        assert_eq!(phase(&mut motion, &mut tracker, 0.0, true), 1.0);
        tracker.handle_event(NoteEvent::on(1.0, SourceId::DIRECT, 0, 60, 1.0));
        assert_eq!(phase(&mut motion, &mut tracker, 1.0, true), 1.0);
        assert_eq!(phase(&mut motion, &mut tracker, 1.5, true), 1.0);
        // Seeking backward resets the transient surface history.
        tracker = NoteTracker::new();
        phase(&mut motion, &mut tracker, -1.0, false);
        assert!(motion.nodes.is_empty());
    }
    #[test]
    fn a_key_up_voice_still_arriving_holds_geometry_across_old_release_pruning() {
        let mut tracker = NoteTracker::new();
        let mut motion = NodeMotion::default();
        tracker.handle_event(NoteEvent::on(0.0, SourceId::DIRECT, 0, 60, 1.0));
        phase(&mut motion, &mut tracker, 0.0, false);
        phase(&mut motion, &mut tracker, 1.0, false);
        tracker.handle_event(NoteEvent::off(1.1, SourceId::DIRECT, 0, 60));
        tracker.handle_event(NoteEvent::on(1.5, SourceId::DIRECT, 0, 72, 1.0));
        tracker.handle_event(NoteEvent::off(1.6, SourceId::DIRECT, 0, 72));
        for now in [1.8, 2.0, 2.2, 2.5] {
            assert_eq!(phase(&mut motion, &mut tracker, now, false), 1.0);
        }
        assert!((phase(&mut motion, &mut tracker, 2.6, false) + 0.9).abs() < 1e-5);
    }
}
