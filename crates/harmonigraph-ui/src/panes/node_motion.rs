//! Lattice-only event-time animation. Core envelopes and the other panes keep
//! their own timing. A checkpoint survives voice pruning and roll retention.
use harmonigraph_core::{
    Envelope, LatticePos, NoteTracker, PitchClass, Tuning, VoiceKey, VoiceState,
};
use harmonigraph_scene::{AnimationOrder, NoteAnimationConfig, OctaveLayout, Scene, ViewConfig};
use std::collections::{HashMap, HashSet};
use std::hash::{Hash, Hasher};

type Identity = (VoiceKey, u64);
#[derive(Clone, Copy)]
struct Held {
    pitch: f32,
}
#[derive(Clone, Copy)]
struct Edge {
    at: f64,
    id: Identity,
    value: Option<Held>,
}
impl Edge {
    fn stamp(self) -> (Identity, u64, u32) {
        (self.id, self.at.to_bits(), self.value.map_or(u32::MAX, |v| v.pitch.to_bits()))
    }
}
#[derive(Clone, Default)]
pub(crate) struct NodeMotion {
    nodes: HashMap<LatticePos, Motion>,
    held: HashMap<Identity, Held>,
    at: Option<f64>,
    boundary: HashSet<(Identity, u64, u32)>,
}
#[derive(Clone)]
struct Motion {
    progress: [f32; 11],
    delay: [f32; 11],
    levels: [f32; 11],
    targets: [f32; 11],
    gate: bool,
    melody: MarkMotion,
    bass: MarkMotion,
    order_delay: [f32; 11],
    audio_waiting: bool,
}
impl Default for Motion {
    fn default() -> Self {
        Self {
            progress: [0.0; 11],
            delay: [0.0; 11],
            levels: [0.0; 11],
            targets: [0.0; 11],
            gate: false,
            melody: MarkMotion::default(),
            bass: MarkMotion::default(),
            order_delay: [0.0; 11],
            audio_waiting: false,
        }
    }
}
#[derive(Clone, Default)]
struct MarkMotion {
    levels: [f32; 11],
    waits: [f32; 11],
    target: Option<usize>,
}
impl MarkMotion {
    fn target(&mut self, target: Option<usize>, delay: f32) {
        if target != self.target {
            self.waits = [0.0; 11];
            if let Some(slot) = target {
                self.waits[slot] = delay;
            }
            self.target = target;
        }
    }
    fn advance(&mut self, dt: f64, env: &Envelope) {
        for i in 0..11 {
            let target = f32::from(self.target == Some(i));
            self.levels[i] =
                approach(self.levels[i], target, (dt - f64::from(self.waits[i])).max(0.0), env);
            self.waits[i] = (self.waits[i] - dt as f32).max(0.0);
        }
    }
    fn strongest(&self) -> (u32, f32, usize) {
        let (slot, &level) =
            self.levels.iter().enumerate().max_by(|a, b| a.1.total_cmp(b.1)).unwrap();
        (if level > 0.0 { 1 << slot } else { 0 }, level, slot)
    }
}
pub(super) fn apply(scene: &mut Scene, state: &mut crate::PictureState, surface: usize, now: f64) {
    state.surfaces.node_motion.entry(surface).or_default().step(
        scene,
        &state.runtime.tracker,
        &state.runtime.tuning,
        &state.appearance.view,
        state.runtime.frame_params.fade_time,
        now,
    );
}
fn approach(level: f32, target: f32, dt: f64, env: &Envelope) -> f32 {
    if level == target {
        return level;
    }
    if target > level {
        target * env.carried(level / target, dt, true)
    } else {
        target
            + (1.0 - target) * env.carried((level - target) / (1.0 - target).max(1e-6), dt, false)
    }
}
impl Motion {
    fn advance(&mut self, dt: f64, duration: f32, env: &Envelope) {
        for i in 0..11 {
            let moving = (dt as f32 - self.delay[i]).max(0.0);
            self.delay[i] = (self.delay[i] - dt as f32).max(0.0);
            self.progress[i] = if self.audio_waiting {
                1.0
            } else if duration <= 0.0 {
                f32::from(self.gate)
            } else {
                (self.progress[i] + if self.gate { moving / duration } else { -moving / duration })
                    .clamp(0.0, 1.0)
            };
            self.levels[i] = approach(self.levels[i], self.targets[i], dt, env);
        }
        self.melody.advance(dt, env);
        self.bass.advance(dt, env);
    }
}
fn delays(
    layout: &OctaveLayout,
    cents: f32,
    config: NoteAnimationConfig,
    seed: u32,
    duration: f32,
) -> [f32; 11] {
    config.delays(layout, cents, seed, duration)
}
impl NodeMotion {
    fn gates(
        &mut self,
        scene: &Scene,
        tuning: &Tuning,
        view: &ViewConfig,
        duration: f32,
        seed_settled: bool,
    ) {
        let high = self.held.values().map(|v| v.pitch).max_by(f32::total_cmp);
        let low = self.held.values().map(|v| v.pitch).min_by(f32::total_cmp);
        for node in &scene.nodes {
            let motion = self.nodes.entry(node.lattice_pos).or_default();
            let (lo, hi) = scene.octave_layout.slots(node.cents);
            motion.targets = [0.0; 11];
            let mut melody = None;
            let mut bass = None;
            for held in self.held.values() {
                if !tuning.matches(
                    PitchClass::from_cents(held.pitch * 100.0),
                    PitchClass::from_cents(node.cents),
                ) {
                    continue;
                }
                let slot = (((held.pitch - node.cents / 100.0) / 12.0).round() as i32)
                    .clamp(lo, hi)
                    .clamp(0, 10) as usize;
                // Existing lattice activation measures occupancy, not velocity.
                motion.targets[slot] = 1.0;
                if view.mark_melody && Some(held.pitch) == high {
                    melody = Some(slot);
                }
                if view.mark_bass && Some(held.pitch) == low {
                    bass = Some(slot);
                }
            }
            motion.melody.target(melody, view.mark_delay);
            motion.bass.target(bass, view.mark_delay);
            let gate = motion.targets.iter().any(|&v| v > 0.0);
            let audio = scene.spectral.ring_draws() && node.audio_ring > 0.0;
            if !audio {
                motion.audio_waiting = false;
            }
            if audio && !gate && node.activation == 0.0 && motion.levels.iter().all(|&v| v == 0.0) {
                motion.audio_waiting = true;
                motion.progress = [1.0; 11];
            }
            if gate {
                motion.audio_waiting = false;
            }
            if gate && !motion.gate && motion.progress.iter().all(|&p| p == 0.0) {
                let mut hash = std::collections::hash_map::DefaultHasher::new();
                node.lattice_pos.hash(&mut hash);
                self.held.keys().map(|id| id.1).min().hash(&mut hash);
                motion.order_delay = delays(
                    &scene.octave_layout,
                    node.cents,
                    view.note_animation,
                    hash.finish() as u32,
                    duration,
                );
                motion.delay = motion.order_delay;
            } else if !gate && motion.gate && motion.progress.iter().all(|&p| p == 1.0) {
                motion.delay = motion.order_delay;
            } else if gate != motion.gate {
                // A reversal never schedules new waiting: pending pieces cancel
                // on off and every piece reverses its current pose immediately.
                motion.delay = [0.0; 11];
            }
            motion.gate = gate;
            if seed_settled && gate {
                motion.progress = [1.0; 11];
                motion.delay = [0.0; 11];
                motion.levels = motion.targets;
                let env =
                    Envelope { attack_time: duration, fade_time: duration, shape: view.fade_shape };
                motion.melody.advance(f64::from(duration + view.mark_delay), &env);
                motion.bass.advance(f64::from(duration + view.mark_delay), &env);
            }
        }
    }
    fn advance(&mut self, dt: f64, duration: f32, env: &Envelope) {
        for motion in self.nodes.values_mut() {
            let moving_time = if motion.order_delay.iter().any(|&d| d > 0.0) {
                duration * 0.72
            } else {
                duration
            };
            motion.advance(dt.max(0.0), moving_time, env);
        }
    }
    fn step(
        &mut self,
        scene: &mut Scene,
        tracker: &NoteTracker,
        tuning: &Tuning,
        view: &ViewConfig,
        duration: f32,
        now: f64,
    ) {
        if !now.is_finite() {
            return;
        }
        if self.at.is_some_and(|at| now < at) {
            *self = Self::default();
        }
        let initial = self.at.is_none();
        let begin = self.at.unwrap_or(
            now - f64::from(duration.max(0.0)) * 2.0 - f64::from(view.mark_delay) - 0.001,
        );
        let mut edges = Vec::new();
        // One bounded roll scan per surface, never one scan per node. Only
        // edges since the checkpoint are replayed; retained old notes cannot
        // restart a finished animation when history is trimmed.
        for note in tracker.roll().notes() {
            if note.end.is_some_and(|at| at < begin) {
                continue;
            }
            let id = (note.key(), note.start.to_bits());
            if initial
                && note.start < begin
                && note.end.is_none_or(|at| at > begin)
                && note.observed_until.is_none_or(|at| at > begin)
            {
                let pitch = note
                    .segments(now)
                    .filter(|((at, _), _)| *at <= begin)
                    .map(|((_, pitch), _)| pitch)
                    .last()
                    .unwrap_or(note.start_pitch());
                self.held.insert(id, Held { pitch });
            }
            let mut add = |at, value| {
                let edge = Edge { at, id, value };
                if at >= begin
                    && at <= now
                    && (at > begin || !self.boundary.contains(&edge.stamp()))
                {
                    edges.push(edge);
                }
            };
            add(note.start, Some(Held { pitch: note.start_pitch() }));
            for ((at, pitch), _) in note.segments(now) {
                add(at, Some(Held { pitch }));
            }
            if let Some(at) = note.end {
                add(at, None);
            }
            // observed_until is loss of observation, not a factual key-up.
            // Current voices reconcile it below without inventing an off time.
        }
        let env = Envelope { attack_time: duration, fade_time: duration, shape: view.fade_shape };
        self.gates(scene, tuning, view, duration, initial);
        // Each ended lifetime finishes absent, including an equal-time bend.
        // A replacement has a different identity and remains in the held union.
        edges.sort_by(|a, b| {
            a.at.total_cmp(&b.at).then_with(|| b.value.is_some().cmp(&a.value.is_some()))
        });
        let mut at = begin;
        let mut index = 0;
        if now != begin {
            self.boundary.clear();
        }
        while index < edges.len() {
            let time = edges[index].at;
            self.advance(time - at, duration, &env);
            // Equal-time off/on edges form one gate update, so a replacement
            // key cannot falsely end an otherwise continuous node presence.
            while index < edges.len() && edges[index].at == time {
                let edge = edges[index];
                if let Some(value) = edge.value {
                    self.held.insert(edge.id, value);
                } else {
                    self.held.remove(&edge.id);
                }
                if time == now {
                    self.boundary.insert(edge.stamp());
                }
                index += 1;
            }
            self.gates(scene, tuning, view, duration, false);
            at = time;
        }
        self.advance(now - at, duration, &env);
        // Current-state reconciliation handles retuning/baselines and gaps,
        // whose missing history must not be treated as fabricated note-offs.
        self.held.clear();
        for voice in tracker.voices().filter(|v| matches!(v.state, VoiceState::Held)) {
            self.held.insert((voice.key(), voice.on_time.to_bits()), Held { pitch: voice.pitch });
        }
        self.gates(scene, tuning, view, duration, false);
        self.advance(0.0, duration, &env);
        self.at = Some(now);
        let visible: HashSet<_> = scene.nodes.iter().map(|n| n.lattice_pos).collect();
        self.nodes.retain(|pos, _| visible.contains(pos));
        for node in &mut scene.nodes {
            let motion = &self.nodes[&node.lattice_pos];
            node.slice_progress = motion.progress;
            node.octaves = motion.levels;
            node.activation = motion.levels.iter().copied().fold(0.0, f32::max);
            node.departing = !motion.gate;
            let (melody_slots, melody_level, melody_slot) = motion.melody.strongest();
            let (bass_slots, bass_level, bass_slot) = motion.bass.strongest();
            node.melody_slots = melody_slots;
            node.melody_level = melody_level;
            node.bass_slots = bass_slots;
            node.bass_level = bass_level;
            let color = |slot| {
                harmonigraph_scene::pitch_lut_color(
                    scene.octave_layout.slot_pitch(slot as i32, node.cents),
                    scene.darkest_pitch,
                    scene.brightest_pitch,
                    view.pitch_gradient,
                )
            };
            node.melody_color = color(melody_slot);
            node.bass_color = color(bass_slot);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use harmonigraph_core::{NoteEvent, SourceId};
    use harmonigraph_scene::{derive_scene, Camera, FrameParams};
    fn draw(
        motion: &mut NodeMotion,
        tracker: &mut NoteTracker,
        view: &ViewConfig,
        now: f64,
        audio: bool,
    ) -> Scene {
        draw_duration(motion, tracker, view, now, audio, 1.0)
    }
    fn draw_duration(
        motion: &mut NodeMotion,
        tracker: &mut NoteTracker,
        view: &ViewConfig,
        now: f64,
        audio: bool,
        duration: f32,
    ) -> Scene {
        let frame = FrameParams { fade_time: duration, ..Default::default() };
        tracker.prune(now, &view.envelope(&frame));
        let mut scene = derive_scene(
            tracker,
            &Tuning::default(),
            view,
            &view.reach(),
            &frame,
            Camera::default(),
            None,
            now,
        );
        if audio {
            scene.spectral.inner = 0.1;
            scene.spectral.outer = 0.4;
            for n in &mut scene.nodes {
                n.audio_ring = 1.0;
            }
        }
        motion.step(&mut scene, tracker, &Tuning::default(), view, duration, now);
        scene
    }
    fn origin(scene: &Scene) -> &harmonigraph_scene::NodeInstance {
        scene.nodes.iter().find(|n| n.lattice_pos == LatticePos::ORIGIN).unwrap()
    }
    fn on(t: f64, note: u8) -> NoteEvent {
        NoteEvent::on(t, SourceId::DIRECT, 0, note, 0.2)
    }
    fn off(t: f64, note: u8) -> NoteEvent {
        NoteEvent::off(t, SourceId::DIRECT, 0, note)
    }
    #[test]
    fn short_stabs_reverse_pose_and_opacity_at_event_time_across_frame_cadences() {
        let view = ViewConfig { fade_shape: 0.0, mark_delay: 0.0, ..Default::default() };
        for order in AnimationOrder::ALL {
            let mut view = view.clone();
            view.note_animation.order = order;
            let mut snapshots = Vec::new();
            for detailed in [false, true] {
                let mut tracker = NoteTracker::new();
                let mut motion = NodeMotion::default();
                tracker.handle_event(on(0.0, 60));
                draw(&mut motion, &mut tracker, &view, 0.0, false);
                if detailed {
                    draw(&mut motion, &mut tracker, &view, 0.1, false);
                }
                tracker.handle_event(off(0.2, 60));
                if detailed {
                    draw(&mut motion, &mut tracker, &view, 0.2, false);
                }
                tracker.handle_event(on(0.25, 60));
                let scene = draw(&mut motion, &mut tracker, &view, 0.3, false);
                let node = origin(&scene);
                assert!((node.activation - 0.2).abs() < 1e-5, "{order:?}: {}", node.activation);
                snapshots.push((node.activation, node.slice_progress));
                let repeated = draw(&mut motion, &mut tracker, &view, 0.3, false);
                assert_eq!(node.slice_progress, origin(&repeated).slice_progress);
            }
            for i in 0..11 {
                assert!((snapshots[0].1[i] - snapshots[1].1[i]).abs() < 1e-5, "{order:?}/{i}");
            }
        }
    }
    #[test]
    fn octaves_and_same_time_replacements_do_not_replay_but_true_disappearance_does() {
        let view = ViewConfig { fade_shape: 0.0, mark_delay: 0.0, ..Default::default() };
        let mut tracker = NoteTracker::new();
        let mut motion = NodeMotion::default();
        tracker.handle_event(on(0.0, 60));
        draw(&mut motion, &mut tracker, &view, 0.0, false);
        let held = draw(&mut motion, &mut tracker, &view, 1.1, false);
        assert_eq!(origin(&held).activation, 1.0, "velocity must not dim occupancy");
        tracker.handle_event(on(1.2, 72));
        tracker.handle_event(off(1.3, 60));
        assert_eq!(
            origin(&draw(&mut motion, &mut tracker, &view, 1.5, false)).slice_progress,
            [1.0; 11]
        );
        tracker.handle_event(off(1.5, 72));
        tracker.handle_event(on(1.5, 72));
        assert_eq!(
            origin(&draw(&mut motion, &mut tracker, &view, 1.5, false)).slice_progress,
            [1.0; 11]
        );
        tracker.handle_event(off(2.0, 72));
        draw(&mut motion, &mut tracker, &view, 3.1, false);
        tracker.handle_event(on(3.2, 60));
        assert!(
            origin(&draw(&mut motion, &mut tracker, &view, 3.3, false)).slice_progress[0] < 0.2
        );
    }
    #[test]
    fn orders_apply_to_settled_departure_and_pending_pieces_cancel_on_interrupt() {
        for order in AnimationOrder::ALL {
            let mut view = ViewConfig { fade_shape: 0.0, ..Default::default() };
            view.note_animation.order = order;
            let mut tracker = NoteTracker::new();
            let mut motion = NodeMotion::default();
            tracker.handle_event(on(0.0, 60));
            draw(&mut motion, &mut tracker, &view, 0.0, false);
            let arrival = draw(&mut motion, &mut tracker, &view, 0.15, false);
            tracker.handle_event(off(0.15, 60));
            let departure = draw(&mut motion, &mut tracker, &view, 0.2, false);
            for (a, b) in
                origin(&arrival).slice_progress.iter().zip(origin(&departure).slice_progress)
            {
                assert!(b <= *a);
            }
            tracker.handle_event(on(0.21, 60));
            draw(&mut motion, &mut tracker, &view, 1.6, false);
            tracker.handle_event(off(1.7, 60));
            let scene = draw(&mut motion, &mut tracker, &view, 1.8, false);
            let p = origin(&scene).slice_progress;
            if order != AnimationOrder::Simultaneous {
                assert!(
                    p[..scene.octave_layout.span as usize].iter().any(|&x| (x - p[0]).abs() > 1e-4),
                    "{order:?} lost departure order"
                );
            }
        }
    }
    #[test]
    fn audio_presence_stays_settled_and_mark_delay_cancels_on_short_notes() {
        let view = ViewConfig { fade_shape: 0.0, mark_delay: 0.4, ..Default::default() };
        let mut tracker = NoteTracker::new();
        let mut motion = NodeMotion::default();
        for now in [0.0, 0.5, 1.5] {
            assert_eq!(
                origin(&draw(&mut motion, &mut tracker, &view, now, true)).slice_progress,
                [1.0; 11]
            );
        }
        tracker.handle_event(on(1.6, 60));
        let scene = draw(&mut motion, &mut tracker, &view, 1.7, true);
        assert_eq!(origin(&scene).slice_progress, [1.0; 11]);
        assert_eq!(origin(&scene).melody_level, 0.0);
        tracker.handle_event(off(1.8, 60));
        assert_eq!(origin(&draw(&mut motion, &mut tracker, &view, 2.1, true)).melody_level, 0.0);
    }
    #[test]
    fn random_order_and_grow_bounds_are_stable() {
        let layout = harmonigraph_scene::octave_layout(4, 60.0, 1, 0.3, 0.7);
        let config = NoteAnimationConfig {
            order: AnimationOrder::RandomStagger,
            radial_start: -1.0,
            start_size: 0.0,
            ..Default::default()
        };
        assert_eq!(
            delays(&layout, 350.0, config, 42, 1.0),
            delays(&layout, 350.0, config, 42, 1.0)
        );
        assert_eq!(config.reach(1.0), 1.0);
        assert!(
            NoteAnimationConfig { animation: harmonigraph_scene::NoteAnimation::Pop, ..config }
                .reach(1.0)
                < 1.05
        );
    }
    #[test]
    fn tuned_onsets_and_late_observation_use_the_factual_pitch_and_mark_clock() {
        use harmonigraph_core::NoteEventKind;
        let view = ViewConfig { fade_shape: 0.0, mark_delay: 0.4, ..Default::default() };
        let tune = |at| NoteEvent {
            time: at,
            source: SourceId::DIRECT,
            channel: 0,
            note: 60,
            kind: NoteEventKind::Tuning { semitones: 4.0 },
        };
        for (at, now) in [(0.0, 0.2), (1.0, 10.0)] {
            let mut tracker = NoteTracker::new();
            let mut motion = NodeMotion::default();
            tracker.handle_event(on(0.0, 60));
            tracker.handle_event(tune(at));
            let scene = draw(&mut motion, &mut tracker, &view, now, false);
            assert_eq!(origin(&scene).activation, 0.0, "old ET pitch lit at {now}");
            assert!(scene.nodes.iter().any(|n| n.activation > 0.0));
        }
        let mut tracker = NoteTracker::new();
        let mut motion = NodeMotion::default();
        tracker.handle_event(on(0.0, 60));
        let scene = draw_duration(&mut motion, &mut tracker, &view, 0.2, false, 0.05);
        assert_eq!(origin(&scene).activation, 1.0);
        assert_eq!(origin(&scene).melody_level, 0.0, "late first observation skipped mark delay");
        tracker = NoteTracker::new();
        motion = NodeMotion::default();
        tracker.handle_event(on(0.0, 60));
        draw(&mut motion, &mut tracker, &view, 0.0, false);
        tracker.handle_event(tune(0.2));
        tracker.handle_event(off(0.2, 60));
        let scene = draw(&mut motion, &mut tracker, &view, 0.35, false);
        assert!(
            scene.nodes.iter().all(|n| n.activation < 0.11),
            "release-time bend resurrected a lifetime"
        );
    }
    #[test]
    fn octave_mark_handoff_keeps_delay_and_releases_the_old_slot() {
        let view = ViewConfig { fade_shape: 0.0, mark_delay: 0.4, ..Default::default() };
        let mut tracker = NoteTracker::new();
        let mut motion = NodeMotion::default();
        tracker.handle_event(on(0.0, 60));
        draw(&mut motion, &mut tracker, &view, 1.5, false);
        tracker.handle_event(on(1.6, 72));
        let scene = draw(&mut motion, &mut tracker, &view, 1.8, false);
        assert_eq!(origin(&scene).melody_slots, 1 << 5);
        let scene = draw(&mut motion, &mut tracker, &view, 3.1, false);
        assert_eq!(origin(&scene).melody_slots, 1 << 6);
        assert_eq!(origin(&scene).bass_slots, 1 << 5);
    }
}
