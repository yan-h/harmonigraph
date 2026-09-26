//! Lattice-only event-time animation. Core envelopes and the other panes keep
//! their own timing. A checkpoint survives voice pruning and roll retention.
use crate::{IntensityReading, NoteAnimationConfig, OctaveLayout, RingFade, Scene, ViewConfig};
use harmonigraph_core::{
    Envelope, LatticePos, NoteTracker, PitchClass, Tuning, VoiceKey, VoiceState,
};
use std::hash::{Hash, Hasher};

// Fixed-seed rather than per-process: nothing here is keyed by input an
// attacker chooses, and a fixed seed keeps iteration order the same on every
// run rather than varying between them.
type HashMap<K, V> = std::collections::HashMap<K, V, foldhash::fast::FixedState>;
type HashSet<K> = std::collections::HashSet<K, foldhash::fast::FixedState>;

type Identity = (VoiceKey, u64);
#[derive(Clone, Copy)]
struct FactualTip {
    bend: (u64, u32),
    end: Option<u64>,
}
#[derive(Clone, Copy)]
struct Held {
    pitch: f32,
    /// `pitch`'s class, converted once rather than per visible node.
    class: PitchClass,
    /// What the way it is played comes to on each display (see
    /// [`crate::intensity`]).
    reading: IntensityReading,
}
impl Held {
    fn new(pitch: f32, reading: IntensityReading) -> Self {
        Self { pitch, class: PitchClass::from_cents(pitch * 100.0), reading }
    }
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
/// Event-time lattice animation, owned separately by each drawing surface.
#[derive(Clone, Default)]
pub struct NodeMotion {
    nodes: HashMap<LatticePos, Motion>,
    held: HashMap<Identity, Held>,
    at: Option<f64>,
    boundary: HashSet<(Identity, u64, u32)>,
    seen: HashMap<Identity, FactualTip>,
    #[cfg(test)]
    replayed_edges: usize,
}
#[derive(Clone)]
struct Motion {
    progress: [f32; 11],
    delay: [f32; 11],
    levels: [f32; 11],
    targets: [f32; 11],
    /// Each slot's reading from the notes lighting it: the largest a held one
    /// gives each display, and once none is held, the last, so a release fades
    /// from where its note left off. Separate from `levels`, which each display
    /// is read against only on the way out, so nothing that reads the envelope
    /// — the gate, the release, the trail — can see it.
    readings: [IntensityReading; 11],
    gate: bool,
    melody: MarkMotion,
    bass: MarkMotion,
    order_delay: [f32; 11],
    order_seed: u32,
    audio_waiting: bool,
}
impl Default for Motion {
    fn default() -> Self {
        Self {
            progress: [0.0; 11],
            delay: [0.0; 11],
            levels: [0.0; 11],
            targets: [0.0; 11],
            readings: [IntensityReading::REST; 11],
            gate: false,
            melody: MarkMotion::default(),
            bass: MarkMotion::default(),
            order_delay: [0.0; 11],
            order_seed: 0,
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
/// The longest Fade a host can set: the top of `ParamKey::Fade`'s range, which
/// this crate cannot see. It only bounds how long [`NodeMotion`] keeps an
/// ended note's cursor; a longer fade still animates correctly, and at worst
/// replays its horizon once when it is raised further.
const FADE_MAX: f32 = 1.0;

/// How far back a surface replays note edges.
///
/// The 2.0 is not a round number: one animation now spans
/// `duration * (1 + stagger_spread)`, so the horizon has to exceed that
/// or a gap longer than it seeds a MID-FLIGHT arrival as settled and the
/// wheel pops. The spread's 0.9 ceiling (`ValueBar` and `sanitize` both)
/// puts the longest arrival at 1.9, leaving 0.1 of margin — so raising
/// that ceiling means raising this too, and 1.0 would leave none.
fn horizon(duration: f32, mark_delay: f32) -> f64 {
    f64::from(duration.max(0.0)) * 2.0 + f64::from(mark_delay) + 0.001
}
fn mark_delay(view: &ViewConfig) -> f32 {
    crate::view::finite_or(view.mark_delay, 0.0).clamp(0.0, crate::MARK_DELAY_MAX)
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
    /// Nothing held, nothing moving and nothing left to fade: `advance` would
    /// write back exactly what is here. Most visible nodes are this at once.
    fn at_rest(&self) -> bool {
        !self.gate
            && !self.audio_waiting
            && self.melody.target.is_none()
            && self.bass.target.is_none()
            && [self.progress, self.delay, self.levels, self.targets]
                .iter()
                .chain([
                    &self.melody.levels,
                    &self.melody.waits,
                    &self.bass.levels,
                    &self.bass.waits,
                ])
                .all(|values| values.iter().all(|&v| v == 0.0))
    }
    fn advance(&mut self, dt: f64, env: &Envelope) {
        if self.at_rest() {
            return;
        }
        // The slice reveal runs the envelope's own length rather than a second
        // duration handed in beside it: `ViewConfig::envelope` puts one time on
        // both ends, so the two were always the same number and a parameter
        // for each is a pair that can be made to disagree.
        for i in 0..11 {
            let moving = (dt as f32 - self.delay[i]).max(0.0);
            self.delay[i] = (self.delay[i] - dt as f32).max(0.0);
            self.progress[i] = if self.audio_waiting {
                1.0
            } else {
                approach(self.progress[i], f32::from(self.gate), f64::from(moving), env)
            };
            // Undelayed, and over the WHOLE duration, on purpose. This is the
            // node's presence rather than any one slice's: it reaches the shader
            // as `ink.w` (`params.x` for a slot no note lights) and is
            // multiplied by the slice's own reveal, which is already nothing
            // before that slice's delay. Waiting here too -- the tempting
            // symmetry -- makes the whole wheel wait for whichever slice the
            // note happens to light, and a note on a LATE slice then collapses
            // the stagger: measured, every slice arrived within 0.08 of the
            // others instead of spanning 0.9.
            self.levels[i] = approach(self.levels[i], self.targets[i], dt, env);
        }
        // A departure that has run out of ink is over, whatever its slices are
        // still holding. The reveal reaches the shader multiplied by this
        // node's presence, so a slice caught mid-retraction behind a zero level
        // draws nothing -- but it is not nothing to the NEXT press, which reads
        // `progress` to tell a reversal from a fresh ordered arrival. Under
        // #885 the two ended together, because a slice's reveal ran
        // `1 - stagger_spread` of a fade and finished exactly when the level
        // did. Once the spread became a start offset instead of a compression,
        // the slices outlived the ink by `stagger_spread` of a fade, and a
        // re-press inside that window took the reversal branch: no order at
        // all, from leftover poses ranked opposite to the entrance they owed.
        if !self.gate && !self.audio_waiting && self.levels.iter().all(|&l| l <= 0.0) {
            self.progress = [0.0; 11];
            self.delay = [0.0; 11];
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
    #[allow(clippy::too_many_arguments)]
    fn gates(
        &mut self,
        scene: &Scene,
        tuning: &Tuning,
        view: &ViewConfig,
        env: &Envelope,
        fade: &RingFade,
        tracker: &NoteTracker,
        now: f64,
        seed_settled: bool,
    ) {
        let duration = env.fade_time;
        let high = self.held.values().map(|v| v.pitch).max_by(f32::total_cmp);
        let low = self.held.values().map(|v| v.pitch).min_by(f32::total_cmp);
        for node in &scene.nodes {
            let node_class = PitchClass::from_cents(node.cents);
            let newly_visible = !self.nodes.contains_key(&node.lattice_pos);
            let motion = self.nodes.entry(node.lattice_pos).or_default();
            let (lo, hi) = scene.octave_layout.slots(node.cents);
            motion.targets = [0.0; 11];
            let mut readings = [None::<IntensityReading>; 11];
            let mut melody = None;
            let mut bass = None;
            let mut preexisting = false;
            for (id, held) in &self.held {
                if !tuning.matches(held.class, node_class) {
                    continue;
                }
                preexisting |= self.at.is_some_and(|at| f64::from_bits(id.1) < at);
                let slot = (((held.pitch - node.cents / 100.0) / 12.0).round() as i32)
                    .clamp(lo, hi)
                    .clamp(0, 10) as usize;
                // Activation measures occupancy; intensity rides beside it.
                motion.targets[slot] = 1.0;
                readings[slot] = Some(readings[slot].map_or(held.reading, |r| r.max(held.reading)));
                if Some(held.pitch) == high {
                    melody = Some(slot);
                }
                if Some(held.pitch) == low {
                    bass = Some(slot);
                }
            }
            for (slot, reading) in readings.into_iter().enumerate() {
                if let Some(reading) = reading {
                    motion.readings[slot] = reading;
                }
            }
            motion.melody.target(melody, mark_delay(view));
            motion.bass.target(bass, mark_delay(view));
            let gate = motion.targets.iter().any(|&v| v > 0.0);
            let audio =
                scene.spectral.ring_draws() && fade.level(&scene.octave_layout, node.cents) > 0.0;
            if !audio {
                motion.audio_waiting = false;
            }
            // A short release can still be tracked after carried ink reaches
            // zero. Seeding an audio-only pose then would change the next MIDI
            // entrance. Check visible lifetime membership only for candidates;
            // no second voice envelope is needed. At exact onset a positive
            // attack has not started, matching the existing audio-start boundary.
            if audio
                && !gate
                && motion.levels.iter().all(|&v| v == 0.0)
                && !tracker.voices().any(|voice| {
                    (env.attack_time <= 0.0 || voice.on_time < now)
                        && tuning.matches(voice.pitch_class, node_class)
                })
            {
                motion.audio_waiting = true;
                motion.progress = [1.0; 11];
            }
            if gate {
                motion.audio_waiting = false;
            }
            if gate && !motion.gate && motion.progress.iter().all(|&p| p == 0.0) {
                let mut hash = std::collections::hash_map::DefaultHasher::new();
                node.lattice_pos.hash(&mut hash);
                self.held
                    .iter()
                    .filter(|(_, held)| tuning.matches(held.class, node_class))
                    .map(|(id, _)| id.1)
                    .min()
                    .hash(&mut hash);
                motion.order_seed = hash.finish() as u32;
                motion.order_delay = delays(
                    &scene.octave_layout,
                    node.cents,
                    view.note_animation,
                    motion.order_seed,
                    duration,
                );
                motion.delay = motion.order_delay;
                // Ordered departure needs a COMPLETE arrival, and an arrival now
                // takes `1 + stagger_spread` fades rather than one, so the hold
                // that earns this has got longer by the same factor. At a high
                // spread most notes a player actually holds fall short and take
                // the reversal branch below, departing without order. That is the
                // cost of the spread being a start offset; the alternative was
                // compressing every piece into a tenth of the fade.
            } else if !gate && motion.gate && motion.progress.iter().all(|&p| p == 1.0) {
                motion.order_delay = delays(
                    &scene.octave_layout,
                    node.cents,
                    view.note_animation,
                    motion.order_seed,
                    duration,
                );
                motion.delay = motion.order_delay;
            } else if gate != motion.gate {
                // A reversal never schedules new waiting: pending pieces cancel
                // on off and every piece reverses its current pose immediately.
                motion.delay = [0.0; 11];
            }
            motion.gate = gate;
            if (seed_settled || (newly_visible && preexisting)) && gate {
                motion.progress = [1.0; 11];
                motion.delay = [0.0; 11];
                motion.levels = motion.targets;
                motion.melody.advance(f64::from(duration + mark_delay(view)), env);
                motion.bass.advance(f64::from(duration + mark_delay(view)), env);
            }
        }
    }
    fn advance(&mut self, dt: f64, env: &Envelope) {
        for motion in self.nodes.values_mut() {
            motion.advance(dt.max(0.0), env);
        }
    }
    /// Apply carried motion, then marker/name complement and the MIDI ring floor.
    /// `fade` contains measured audio presence only, independent of that floor.
    /// The caller prunes `tracker` at `now` with this envelope before composition.
    #[allow(clippy::too_many_arguments)]
    pub fn step(
        &mut self,
        scene: &mut Scene,
        tracker: &NoteTracker,
        tuning: &Tuning,
        view: &ViewConfig,
        env: &Envelope,
        fade: &RingFade,
        now: f64,
    ) {
        if !now.is_finite() {
            return;
        }
        let horizon = horizon(env.fade_time, mark_delay(view));
        // A hidden surface cannot benefit from replaying minutes of settled
        // history. Seed current state and replay only the visible horizon.
        if self.at.is_some_and(|at| now < at || now - at > horizon) {
            *self = Self::default();
        }
        let floor = now - horizon;
        let intensity = view.intensity.sanitized();
        // A note that ended before every horizon a host can set can never
        // count as late again, whatever the settings do next, so its cursor
        // has nothing left to guard. Skipping it bounds this scan by the last
        // few seconds of playing rather than by the whole roll.
        let retained = now - horizon.max(self::horizon(FADE_MAX, crate::MARK_DELAY_MAX));
        let notes: Vec<_> = tracker
            .roll()
            .notes()
            .filter(|note| note.end.is_none_or(|at| at >= retained))
            .collect();
        let mut seen = HashMap::default();
        let mut late = false;
        for note in &notes {
            let id = (note.key(), note.start.to_bits());
            let old = self.seen.get(&id);
            let end = note.end.map(f64::to_bits);
            // Completed old lifetimes cannot acquire more bends. Retain their
            // cursor even outside today's horizon, up to the longest a host can
            // set, so a longer fade setting cannot mistake them for newly
            // delivered notes.
            let tip = if note.end.is_some_and(|at| at < floor) && old.is_some_and(|v| v.end == end)
            {
                *old.unwrap()
            } else {
                let ((at, pitch), _) = note.segments(now).last().unwrap_or((
                    (note.start, note.start_pitch()),
                    (note.start, note.start_pitch()),
                ));
                FactualTip { bend: (at.to_bits(), pitch.to_bits()), end }
            };
            let behind = |at| self.at.is_some_and(|begin| at < begin) && at >= floor && at <= now;
            late |= old.is_none() && behind(note.start);
            late |= old.is_none_or(|v| v.bend != tip.bend) && behind(f64::from_bits(tip.bend.0));
            late |= old.is_none_or(|v| v.end != tip.end) && note.end.is_some_and(behind);
            seen.insert(id, tip);
        }
        // The display clock can run ahead of a newly delivered audio block.
        // Reconstruct the bounded factual timeline when an unseen edge arrives
        // behind the checkpoint, rather than losing a complete short lifetime
        // or inventing a delayed key-down that current-state reconciliation ends.
        if late {
            *self = Self::default();
        }
        self.seen = seen;
        let initial = self.at.is_none();
        let begin = self.at.unwrap_or(now - horizon);
        let mut edges = Vec::new();
        // One bounded roll scan per surface, never one scan per node. Only
        // edges since the checkpoint are replayed; retained old notes cannot
        // restart a finished animation when history is trimmed.
        for note in notes {
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
                let reading = intensity.read(note.velocity, note.expressions_at(begin));
                self.held.insert(id, Held::new(pitch, reading));
            }
            let held = |at, pitch| {
                Held::new(pitch, intensity.read(note.velocity, note.expressions_at(at)))
            };
            let mut add = |at, value| {
                let edge = Edge { at, id, value };
                if at >= begin
                    && at <= now
                    && (at > begin || !self.boundary.contains(&edge.stamp()))
                {
                    edges.push(edge);
                }
            };
            add(note.start, Some(held(note.start, note.start_pitch())));
            for ((at, pitch), _) in note.segments(now) {
                add(at, Some(held(at, pitch)));
            }
            if let Some(at) = note.end {
                add(at, None);
            }
            // observed_until is loss of observation, not a factual key-up.
            // Current voices reconcile it below without inventing an off time.
        }
        // The checkpoint already has the previous frame's targets. Recompute
        // them only for initial seeding, an event, or current-state reconciliation.
        if initial {
            self.gates(scene, tuning, view, env, fade, tracker, now, true);
        }
        // Each ended lifetime finishes absent, including an equal-time bend.
        // A replacement has a different identity and remains in the held union.
        edges.sort_by(|a, b| {
            a.at.total_cmp(&b.at).then_with(|| b.value.is_some().cmp(&a.value.is_some()))
        });
        #[cfg(test)]
        {
            self.replayed_edges = edges.len();
        }
        let mut at = begin;
        let mut index = 0;
        if now != begin {
            self.boundary.clear();
        }
        while index < edges.len() {
            let time = edges[index].at;
            self.advance(time - at, env);
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
            self.gates(scene, tuning, view, env, fade, tracker, now, false);
            at = time;
        }
        self.advance(now - at, env);
        // Current-state reconciliation handles retuning/baselines and gaps,
        // whose missing history must not be treated as fabricated note-offs.
        self.held.clear();
        for voice in tracker.voices().filter(|v| matches!(v.state, VoiceState::Held)) {
            let reading = intensity.read(voice.velocity, voice.expressions);
            self.held
                .insert((voice.key(), voice.on_time.to_bits()), Held::new(voice.pitch, reading));
        }
        self.gates(scene, tuning, view, env, fade, tracker, now, false);
        self.advance(0.0, env);
        self.at = Some(now);
        let visible: HashSet<_> = scene.nodes.iter().map(|n| n.lattice_pos).collect();
        self.nodes.retain(|pos, _| visible.contains(pos));
        for node in &mut scene.nodes {
            let motion = &self.nodes[&node.lattice_pos];
            node.slice_progress = motion.progress;
            // Opacity fades each slot's ink, and the node's presence with it,
            // while the envelope under it runs untouched. Thickness reshapes a
            // LIT slot only: a slot keeps its last reading once released, and
            // an unlit one draws the ghost at full width whatever it read, so
            // it goes as 1 and the shader keeps its full-slice path there —
            // and its mark goes back to standing off the band.
            let fades = motion.readings.map(|reading| reading.opacity);
            node.octaves = std::array::from_fn(|i| motion.levels[i] * fades[i]);
            node.thickness = std::array::from_fn(|i| {
                if node.octaves[i] > 0.0 {
                    motion.readings[i].thickness
                } else {
                    1.0
                }
            });
            node.activation = node.octaves.iter().copied().fold(0.0, f32::max);
            node.departing = !motion.gate;
            let (melody_slots, melody_level, melody_slot) = motion.melody.strongest();
            let (bass_slots, bass_level, bass_slot) = motion.bass.strongest();
            node.melody_slots = melody_slots;
            node.melody_level = melody_level * fades[melody_slot];
            node.bass_slots = bass_slots;
            node.bass_level = bass_level * fades[bass_slot];
            let color = |slot| {
                crate::pitch_lut_color(
                    scene.octave_layout.slot_pitch(slot as i32, node.cents),
                    scene.darkest_pitch,
                    scene.brightest_pitch,
                    view.pitch_gradient,
                )
            };
            node.melody_color = color(melody_slot);
            node.bass_color = color(bass_slot);
            if scene.spectral.ring_draws() {
                node.audio_ring = fade.level(&scene.octave_layout, node.cents).max(node.activation);
            }
            // The node glow is the node's presence, unfaded and unread by any
            // display: the loudest slot or mark. Stateless snapshots draw it
            // as it stands; the shell's glow pass carries it as its target.
            node.glow.level = (0..11)
                .map(|i| motion.levels[i])
                .chain([melody_level, bass_level])
                .fold(0.0, f32::max);
            // The Glow display drives the BLOOM instead, per node: the reading
            // of its loudest lit slot, as its share of the pass's strength
            // (`scene.bloom_strength`, the same reference).
            let loudest = (0..11).max_by(|&a, &b| motion.levels[a].total_cmp(&motion.levels[b]));
            node.bloom = match loudest {
                Some(slot) if motion.levels[slot] > 0.0 => {
                    view.intensity.bloom_share(motion.readings[slot], view.bloom_strength)
                }
                _ => 1.0,
            };
        }
        scene.pluses = crate::derive::derive_pluses(
            view,
            &scene.nodes,
            crate::grey_of_lightness(view.marker_ink_lightness()),
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{derive_scene, AnimationOrder, Camera, FrameParams, MIDDLE_C_SLOT};
    use harmonigraph_core::{NoteEvent, SourceId};

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
        let env = view.envelope(&frame);
        tracker.prune(now, &env);
        let mut scene = derive_scene(
            tracker,
            &Tuning::default(),
            view,
            &view.reach(),
            &frame,
            Camera::default(),
            None,
        );
        let mut fade = RingFade::default();
        if audio {
            scene.spectral.inner = 0.1;
            scene.spectral.outer = 0.4;
            scene.spectral.gate = 0.0;
            fade.advance(&crate::RingGate::new(&scene.spectral), &env, now);
        }
        motion.step(&mut scene, tracker, &Tuning::default(), view, &env, &fade, now);
        scene
    }
    fn origin(scene: &Scene) -> &crate::NodeInstance {
        scene.nodes.iter().find(|n| n.lattice_pos == LatticePos::ORIGIN).unwrap()
    }
    fn on(t: f64, note: u8) -> NoteEvent {
        NoteEvent::on(t, SourceId::DIRECT, 0, note, 0.2)
    }
    fn off(t: f64, note: u8) -> NoteEvent {
        NoteEvent::off(t, SourceId::DIRECT, 0, note)
    }

    #[test]
    fn smooth_slice_motion_uses_the_note_fade_curve_in_both_directions() {
        let sample = |shape: f32| {
            let view = ViewConfig { fade_shape: shape, ..Default::default() };
            let mut tracker = NoteTracker::new();
            let mut motion = NodeMotion::default();
            tracker.handle_event(on(0.0, 60));
            draw(&mut motion, &mut tracker, &view, 0.0, false);
            let arriving = origin(&draw(&mut motion, &mut tracker, &view, 0.25, false))
                .slice_progress[MIDDLE_C_SLOT];
            draw(&mut motion, &mut tracker, &view, 1.0, false);
            tracker.handle_event(off(1.0, 60));
            draw(&mut motion, &mut tracker, &view, 1.0, false);
            let departing = origin(&draw(&mut motion, &mut tracker, &view, 1.25, false))
                .slice_progress[MIDDLE_C_SLOT];
            (arriving, departing)
        };

        let linear = sample(0.0);
        assert!((linear.0 - 0.25).abs() < 1e-5);
        assert!((linear.1 - 0.75).abs() < 1e-5);
        let curved = sample(1.0);
        assert!((curved.0 - 0.683_593_75).abs() < 1e-5);
        assert!((curved.1 - 0.316_406_25).abs() < 1e-5);
    }

    /// A slice's reveal used to end exactly when its node's ink did, because
    /// the spread COMPRESSED each piece into `1 - stagger_spread` of a fade.
    /// Once the spread became a start offset instead -- right for the arrival,
    /// and applied to the settled departure with it -- the slices outlived the
    /// ink by `stagger_spread` of a fade. A press inside that window found
    /// `progress` not all zero and took the reversal branch, which cancels
    /// every delay and starts from leftover poses ranked OPPOSITE to the
    /// entrance they owe: the same repeated note animating two different ways
    /// depending on invisible state.
    #[test]
    fn a_faded_departure_leaves_no_pose_for_the_next_press_to_reverse() {
        for order in AnimationOrder::ALL {
            let mut view = ViewConfig { fade_shape: 0.0, mark_delay: 0.0, ..Default::default() };
            view.note_animation.order = order;
            view.note_animation.stagger_spread = 0.9;
            // One press inside the window the slices used to outlive the ink
            // by, and one well past it. Both owe the same fresh entrance.
            let mut poses = Vec::new();
            for repress in [6.25, 7.5] {
                let mut tracker = NoteTracker::new();
                let mut motion = NodeMotion::default();
                tracker.handle_event(on(0.0, 60));
                draw(&mut motion, &mut tracker, &view, 0.0, false);
                draw(&mut motion, &mut tracker, &view, 5.0, false);
                tracker.handle_event(off(5.0, 60));
                let dark = draw(&mut motion, &mut tracker, &view, 6.0, false);
                assert_eq!(origin(&dark).activation, 0.0, "{order:?}: one fade after the off");
                assert_eq!(
                    origin(&dark).slice_progress,
                    [0.0; 11],
                    "{order:?}: the ink is gone and the poses are not",
                );
                draw(&mut motion, &mut tracker, &view, repress, false);
                tracker.handle_event(on(repress, 60));
                let back = draw(&mut motion, &mut tracker, &view, repress + 0.01, false);
                poses.push(origin(&back).slice_progress);
            }
            // Sorted, because `RandomStagger` mints a seed per press and owes
            // the same SHAPE of entrance rather than the same permutation of
            // it: as many slices under way, by as much. A press that reversed
            // leftover poses instead sorts to a spread of them, not to one
            // tick's worth.
            let shape = |mut p: [f32; 11]| {
                p.sort_by(f32::total_cmp);
                p
            };
            assert_eq!(
                shape(poses[0]),
                shape(poses[1]),
                "{order:?}: the earlier press did not start a fresh entrance",
            );
        }
    }
    #[test]
    fn late_delivered_short_notes_recover_the_factual_timeline_once() {
        let view = ViewConfig { fade_shape: 0.0, mark_delay: 0.0, ..Default::default() };
        let mut tracker = NoteTracker::new();
        let mut motion = NodeMotion::default();
        draw(&mut motion, &mut tracker, &view, 0.21, false);
        tracker.handle_event(on(0.15, 60));
        tracker.handle_event(off(0.20, 60));
        let late = draw(&mut motion, &mut tracker, &view, 0.22, false);
        assert!((origin(&late).activation - 0.03).abs() < 1e-5);
        let mut timely_tracker = NoteTracker::new();
        let mut timely = NodeMotion::default();
        timely_tracker.handle_event(on(0.15, 60));
        draw(&mut timely, &mut timely_tracker, &view, 0.15, false);
        timely_tracker.handle_event(off(0.20, 60));
        let reference = draw(&mut timely, &mut timely_tracker, &view, 0.22, false);
        assert_eq!(origin(&late).slice_progress, origin(&reference).slice_progress);
        let repeated = draw(&mut motion, &mut tracker, &view, 0.22, false);
        assert_eq!(origin(&late).slice_progress, origin(&repeated).slice_progress);
        assert_eq!(motion.replayed_edges, 0, "late edges replayed twice");
        let mut tracker = NoteTracker::new();
        let mut motion = NodeMotion::default();
        tracker.handle_event(on(0.15, 60));
        draw(&mut motion, &mut tracker, &view, 0.21, false);
        tracker.handle_event(off(0.20, 60));
        let late_off = draw(&mut motion, &mut tracker, &view, 0.22, false);
        assert_eq!(origin(&late_off).slice_progress, origin(&reference).slice_progress);
        assert!((origin(&late_off).activation - 0.03).abs() < 1e-5);
        draw_duration(&mut motion, &mut tracker, &view, 0.5, false, 0.05);
        let longer = draw_duration(&mut motion, &mut tracker, &view, 0.6, false, 1.0);
        assert_eq!(origin(&longer).activation, 0.0, "a longer horizon replayed an old note");
        // The activation alone cannot tell: this note's replay also ends dark.
        assert_eq!(motion.replayed_edges, 0, "a longer horizon forgot an old note's cursor");
    }
    #[test]
    fn reopening_a_hidden_surface_bounds_replay_to_the_settle_horizon() {
        let view = ViewConfig::default();
        let mut tracker = NoteTracker::new();
        let mut motion = NodeMotion::default();
        draw(&mut motion, &mut tracker, &view, 0.0, false);
        for i in 0..4096 {
            let at = 1.0 + f64::from(i) * 0.05;
            tracker.handle_event(on(at, 60));
            tracker.handle_event(off(at + 0.03, 60));
        }
        let now = 206.0;
        tracker.handle_event(on(now - 0.1, 60));
        draw(&mut motion, &mut tracker, &view, now, false);
        assert!(motion.replayed_edges < 200, "replayed {} old edges", motion.replayed_edges);
        assert!(motion.nodes[&LatticePos::ORIGIN].gate);
    }
    #[test]
    fn held_nodes_reentering_the_visible_window_do_not_replay_an_entrance() {
        let view = ViewConfig::default();
        let mut tracker = NoteTracker::new();
        let mut motion = NodeMotion::default();
        tracker.handle_event(on(0.0, 60));
        let mut scene = draw(&mut motion, &mut tracker, &view, 1.1, false);
        scene.nodes.retain(|node| node.lattice_pos != LatticePos::ORIGIN);
        let env = view.envelope(&FrameParams { fade_time: 1.0, ..Default::default() });
        motion.step(
            &mut scene,
            &tracker,
            &Tuning::default(),
            &view,
            &env,
            &RingFade::default(),
            1.2,
        );
        assert!(!motion.nodes.contains_key(&LatticePos::ORIGIN));
        let returned = draw(&mut motion, &mut tracker, &view, 1.3, false);
        assert_eq!(origin(&returned).slice_progress, [1.0; 11]);
        assert_eq!(origin(&returned).activation, 1.0);
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
    /// Pressure routed to opacity over a base of 0 fades a slot's ink and the
    /// node's presence, straight away as the pressure moves, and nothing else:
    /// a note faded to nothing is still held, and still departs on its own
    /// release. The glow, with nothing routed to it, stays in full.
    #[test]
    fn intensity_fades_the_ink_but_not_the_note() {
        use crate::{IntensitySource, IntensityTarget};
        let intensity = crate::IntensitySettings {
            pressure: IntensitySource { target: IntensityTarget::Opacity, weight: 1.0 },
            opacity_rest: 0.0,
            ..Default::default()
        };
        let view = ViewConfig { fade_shape: 0.0, intensity, ..Default::default() };
        let mut tracker = NoteTracker::new();
        let mut motion = NodeMotion::default();
        let slot = |scene: &Scene| {
            let node = origin(scene);
            let lit = node.octaves.iter().position(|&l| l > 0.0);
            (node.activation, lit.map(|i| node.octaves[i]), node.departing, node.glow.level)
        };
        tracker.handle_event(on(0.0, 60));
        let silent = draw(&mut motion, &mut tracker, &view, 1.1, false);
        assert_eq!(slot(&silent), (0.0, None, false, 1.0), "unpressed: invisible, still held");

        tracker.handle_event(NoteEvent {
            source: SourceId::DIRECT,
            time: 1.1,
            channel: 0,
            note: 60,
            kind: harmonigraph_core::NoteEventKind::Expression {
                expression: harmonigraph_core::Expression::Pressure,
                value: 0.5,
            },
        });
        let pressed = draw(&mut motion, &mut tracker, &view, 1.1, false);
        assert_eq!(slot(&pressed), (0.5, Some(0.5), false, 1.0), "the ink follows at once");
        assert_eq!(origin(&pressed).bloom, 1.0, "nothing is routed to Glow");

        tracker.handle_event(off(1.2, 60));
        let (activation, _, departing, glow) =
            slot(&draw(&mut motion, &mut tracker, &view, 1.7, false));
        assert!(departing, "released");
        assert!((activation - 0.25).abs() < 1e-5, "half the release left, at half: {activation}");
        assert!((glow - 0.5).abs() < 1e-5, "the glow departs on the envelope alone: {glow}");

        // Pressure routed to the glow instead, at half weight over a Bloom of
        // half: a note at half pressure blooms at three quarters, half again
        // the bar's own, and neither the slice ink nor the node glow is
        // touched by it.
        let view = ViewConfig {
            bloom_strength: 0.5,
            intensity: crate::IntensitySettings {
                pressure: IntensitySource { target: IntensityTarget::Glow, weight: 0.5 },
                ..Default::default()
            },
            ..view
        };
        let mut tracker = NoteTracker::new();
        let mut motion = NodeMotion::default();
        tracker.handle_event(on(0.0, 60));
        tracker.handle_event(NoteEvent {
            source: SourceId::DIRECT,
            time: 0.0,
            channel: 0,
            note: 60,
            kind: harmonigraph_core::NoteEventKind::Expression {
                expression: harmonigraph_core::Expression::Pressure,
                value: 0.5,
            },
        });
        let held = draw(&mut motion, &mut tracker, &view, 1.1, false);
        assert_eq!(slot(&held), (1.0, Some(1.0), false, 1.0));
        assert_eq!(origin(&held).bloom, 1.5);
    }
    /// Pressure routed to thickness swells the lit slot from rest and nothing
    /// else: its ink and light stay full, and once the note is gone the slot
    /// reads full width again though it keeps its last reading.
    #[test]
    fn thickness_reshapes_a_lit_slot_only() {
        use crate::{IntensitySource, IntensityTarget};
        let intensity = crate::IntensitySettings {
            pressure: IntensitySource { target: IntensityTarget::Thickness, weight: 1.0 },
            ..Default::default()
        };
        let view = ViewConfig { fade_shape: 0.0, intensity, ..Default::default() };
        let mut tracker = NoteTracker::new();
        let mut motion = NodeMotion::default();
        tracker.handle_event(on(0.0, 60));
        tracker.handle_event(NoteEvent {
            source: SourceId::DIRECT,
            time: 0.0,
            channel: 0,
            note: 60,
            kind: harmonigraph_core::NoteEventKind::Expression {
                expression: harmonigraph_core::Expression::Pressure,
                value: 0.25,
            },
        });
        let held = draw(&mut motion, &mut tracker, &view, 1.1, false);
        let node = origin(&held);
        let lit = node.octaves.iter().position(|&l| l > 0.0).expect("a lit slot");
        assert_eq!((node.octaves[lit], node.glow.level), (1.0, 1.0), "ink and light stay full");
        assert_eq!(node.thickness[lit], 1.25);
        assert!(
            node.thickness.iter().enumerate().all(|(i, &t)| i == lit || t == 1.0),
            "{:?}",
            node.thickness,
        );
        tracker.handle_event(off(1.2, 60));
        let releasing = *origin(&draw(&mut motion, &mut tracker, &view, 1.7, false));
        assert!(releasing.octaves[lit] > 0.0 && releasing.octaves[lit] < 1.0);
        assert_eq!(releasing.thickness[lit], 1.25, "a release keeps the note's width");
        let gone = *origin(&draw(&mut motion, &mut tracker, &view, 3.0, false));
        assert_eq!(gone.octaves[lit], 0.0);
        assert_eq!(gone.thickness, [1.0; 11], "a spent slot reads full");
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
    fn stagger_spread_spans_starts_and_reverses_without_waiting() {
        let layout = crate::octave_layout(4, 60.0, 1, 0.3, 0.7);
        for order in AnimationOrder::ALL {
            for spread in [0.0, NoteAnimationConfig::default().stagger_spread, 0.9] {
                let mut view = ViewConfig { fade_shape: 0.0, ..Default::default() };
                view.note_animation.order = order;
                view.note_animation.stagger_spread = spread;
                let ds = view.note_animation.delays(&layout, 350.0, 42, 1.0);
                let active = &ds[..layout.span as usize];
                assert_eq!(active.iter().copied().fold(f32::INFINITY, f32::min), 0.0);
                let expected = if order == AnimationOrder::Simultaneous { 0.0 } else { spread };
                assert!((active.iter().copied().fold(0.0, f32::max) - expected).abs() < 1e-6);
                let mut tracker = NoteTracker::new();
                let mut motion = NodeMotion::default();
                tracker.handle_event(on(0.0, 60));
                draw(&mut motion, &mut tracker, &view, 0.0, false);
                draw(&mut motion, &mut tracker, &view, 0.04, false);
                let before = motion.nodes[&LatticePos::ORIGIN].progress;
                tracker.handle_event(off(0.04, 60));
                draw(&mut motion, &mut tracker, &view, 0.05, false);
                let m = &motion.nodes[&LatticePos::ORIGIN];
                assert_eq!(m.delay, [0.0; 11]);
                for (a, b) in before.into_iter().zip(m.progress) {
                    assert!(if a > 0.0 { b < a } else { b == 0.0 });
                }
                tracker.handle_event(on(0.05, 60));
                draw(&mut motion, &mut tracker, &view, 0.06, false);
                assert_eq!(motion.nodes[&LatticePos::ORIGIN].delay, [0.0; 11]);
                draw(&mut motion, &mut tracker, &view, 1.1, false);
                assert_eq!(motion.nodes[&LatticePos::ORIGIN].progress, [1.0; 11]);
                // A held control edit governs the next settled departure.
                view.note_animation.stagger_spread = 0.9 - spread;
                tracker.handle_event(off(1.1, 60));
                draw(&mut motion, &mut tracker, &view, 1.1, false);
                let expected =
                    if order == AnimationOrder::Simultaneous { 0.0 } else { 0.9 - spread };
                assert!(
                    (motion.nodes[&LatticePos::ORIGIN].delay.into_iter().fold(0.0, f32::max)
                        - expected)
                        .abs()
                        < 1e-6
                );
                // The widest delay plus one whole duration: a departure now
                // SPANS `1 + spread`, because the spread offsets starts and no
                // longer buys its waiting out of each slice's own time.
                let end = 1.1 + 1.0 + f64::from(0.9 - spread) + 1e-5;
                draw(&mut motion, &mut tracker, &view, end, false);
                assert_eq!(motion.nodes[&LatticePos::ORIGIN].progress, [0.0; 11]);
            }
        }
    }
    #[test]
    fn every_staggered_slice_animates_for_the_whole_duration_from_its_own_start() {
        // The complaint this answers: at a high spread some slices faded in over
        // the whole note and others waited and then popped. Measured on the old
        // compression at spread 0.9, one wheel's slices took 0.98, 0.84, 0.69,
        // 0.54, 0.39, 0.24 and 0.09 of a duration -- every slice finishing
        // together at the fade time, so the earlier it started the longer it
        // took. Both assertions below are needed: the lengths were all EQUAL to
        // each other then too (each reveal ran 0.1), so only measuring them
        // against ONE WHOLE DURATION separates the two contracts.
        let step = 0.01f64;
        let spread = 0.9f32;
        for order in AnimationOrder::ALL {
            let mut view = ViewConfig { fade_shape: 0.0, mark_delay: 0.0, ..Default::default() };
            view.note_animation.order = order;
            view.note_animation.stagger_spread = spread;
            let mut tracker = NoteTracker::new();
            let mut motion = NodeMotion::default();
            // C5, deliberately NOT middle C, whose slice is the one the orders
            // start from: the presence check below is about a note landing on a
            // LATE slice, and middle C would give it a delay of zero to pass at.
            tracker.handle_event(on(0.0, 72));
            let first = draw(&mut motion, &mut tracker, &view, 0.0, false);
            let span = first.octave_layout.span as usize;
            let mut started = [None; 11];
            let mut done = [None; 11];
            let mut now = 0.0;
            // Past the last slice's finish at `spread + 1`, which is exactly
            // what a sweep stopping at one duration would never see.
            while now < f64::from(spread) + 1.5 {
                now += step;
                let scene = draw(&mut motion, &mut tracker, &view, now, false);
                let node = origin(&scene);
                // The node's presence carries every slot no note lights, so it
                // must NOT wait for the lit slice. Staggering it too makes the
                // whole wheel wait on whichever slot the note happens to take,
                // and C5's is a late one: measured, that collapsed seven starts
                // spanning 0.9 into all of them landing within 0.08.
                assert!(node.activation > 0.0, "{order:?}: presence waited for its lit slice");
                for i in 0..span {
                    started[i] = started[i].or((node.slice_progress[i] > 0.0).then_some(now));
                    done[i] = done[i].or((node.slice_progress[i] >= 1.0).then_some(now));
                }
            }
            let at = |v: Option<f64>, what: &str| v.unwrap_or_else(|| panic!("{order:?}: {what}"));
            for i in 0..span {
                let length = at(done[i], "slice never finished") - at(started[i], "never started");
                assert!(
                    (length - 1.0).abs() <= 2.0 * step,
                    "{order:?}: slice {i} animated for {length}, not one whole duration"
                );
            }
            let begins: Vec<f64> = (0..span).map(|i| at(started[i], "never started")).collect();
            let widest = begins.iter().copied().fold(0.0, f64::max)
                - begins.iter().copied().fold(f64::INFINITY, f64::min);
            let expected =
                if order == AnimationOrder::Simultaneous { 0.0 } else { f64::from(spread) };
            assert!(
                (widest - expected).abs() <= 2.0 * step,
                "{order:?}: starts span {widest}, expected {expected}"
            );
        }
    }
    #[test]
    fn random_order_and_grow_bounds_are_stable() {
        let layout = crate::octave_layout(4, 60.0, 1, 0.3, 0.7);
        let config = NoteAnimationConfig {
            order: AnimationOrder::RandomStagger,
            radial_start: -1.0,
            ..Default::default()
        };
        assert_eq!(
            delays(&layout, 350.0, config, 42, 1.0),
            delays(&layout, 350.0, config, 42, 1.0)
        );
        assert_eq!(config.reach(1.0), 1.0);
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
