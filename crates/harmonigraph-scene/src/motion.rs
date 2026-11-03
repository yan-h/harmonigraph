//! Lattice-only event-time animation. Core envelopes and the other panes keep
//! their own timing. A checkpoint survives voice pruning and roll retention.
use crate::derive::lattice_order;
use crate::{IntensityReading, RingFade, Scene, ViewConfig};
use harmonigraph_core::{
    Envelope, LatticePos, NoteTracker, PitchClass, Tuning, VoiceKey, VoiceState,
};
use std::collections::hash_map::Entry;

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
    /// The visible nodes whose motion is not at rest, and no others: an absent
    /// position is exactly [`Motion::REST`]. A table of every visible node held
    /// 20480 of these at the node cap, about 19 MB a surface, nearly all of it
    /// identical rest.
    ///
    /// A node at rest can still differ from `REST` in what it last read
    /// (`readings`, `level_wait`), and dropping it forgets that. Each is
    /// written before it is next read: a slot's reading before its level can
    /// leave 0, the wait at a departure. The equivalence
    /// test holds a table that never drops one against this one, frame for
    /// frame.
    nodes: HashMap<LatticePos, Motion>,
    /// The visible nodes holding nothing but an audio pose ([`Motion::POSED`]),
    /// kept out of `nodes`. With the ring on at its floor every unlit node in
    /// view wears one, which is the whole window again in full entries.
    posed: HashSet<LatticePos>,
    /// The positions the last step drew, in the scene's lattice order: what
    /// tells a node scrolling into view from one that sat at rest in it, which
    /// was the absent entry's job while every visible node had one.
    visible: Vec<LatticePos>,
    held: HashMap<Identity, Held>,
    at: Option<f64>,
    boundary: HashSet<(Identity, u64, u32)>,
    seen: HashMap<Identity, FactualTip>,
    #[cfg(test)]
    replayed_edges: usize,
    /// Keep an entry for every visible node, as the table did before it was
    /// sparse: the reference the equivalence test holds this one against.
    #[cfg(test)]
    dense: bool,
    #[cfg(test)]
    reached: Reached,
}
/// How often a step took each branch the sparse table answers differently
/// from a dense one, so the equivalence test can show its fixture got there.
#[cfg(test)]
#[derive(Clone, Copy, Default, Debug)]
struct Reached {
    /// An entry dropped at rest, lit or rung again while still in view.
    revived: usize,
    /// A held note's node scrolling into view, which seeds a settled pose.
    scrolled_in: usize,
    /// A held note lighting a node that was in view and at rest, which
    /// arrives rather than settling.
    lit_in_view: usize,
    /// A held note lighting a node new to the view on a later pass of the
    /// step than the first, which arrives as well.
    lit_after_entering: usize,
}
#[derive(Clone)]
/// Two index spaces share these arrays. `progress`, `seen` and each edge's
/// `delay` count the slices a node draws, lowest first (slot `ring.base + i`),
/// as `delays` ranks them and the shader reads them: an 11-slice ring can draw a
/// slice below slot 0, which no slot index holds. `levels`, `targets`,
/// `readings` and `level_wait` count octave slots, as `NodeInstance::octaves`
/// and the marks do.
struct Motion {
    progress: [f32; 11],
    /// The gate each slice is moving toward: the node's own, as it was when
    /// the last edge to reach that slice left the node.
    seen: [bool; 11],
    /// The node's gate edges still on their way to some slice, oldest first.
    /// Every slice follows the gate late by its place in the sweep, so an
    /// interruption is a staggered gesture like any other rather than a
    /// moment where every slice turns at once.
    edges: Vec<SliceEdge>,
    /// Each slot's presence: what keeps the wheel drawn, held at a release
    /// for as long as the release takes to reach the last slice
    /// (`level_wait`), so the late slices are still there to retract.
    levels: [f32; 11],
    /// Each slot's colour: how much of its pitch it wears. The same ramp as
    /// `levels` but never held, so a released note starts losing its colour
    /// at once, and a note tapped again and again dims between taps rather
    /// than staying lit while its presence waits. The shader draws the gap
    /// between the two as the ghost.
    colors: [f32; 11],
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
    /// Time each slot's presence holds before a departure starts it falling:
    /// the fade-out delay and then the last slice's delay, so the ink outlasts
    /// every retraction rather than dimming the whole wheel at the off. Only
    /// for slots lit at the off -- one released earlier keeps fading, rather
    /// than freezing mid-fade until the rest catch up. 0 otherwise.
    level_wait: [f32; 11],
    audio_waiting: bool,
}
/// One gate edge on its way through a node's slices.
#[derive(Clone)]
struct SliceEdge {
    on: bool,
    /// Time since the edge left the node.
    age: f64,
    /// When it reaches each slice, after leaving the node; infinite once it
    /// has, or once a newer edge has overtaken it there.
    delay: [f64; 11],
}
#[derive(Clone)]
struct MarkMotion {
    levels: [f32; 11],
    waits: [f32; 11],
    target: Option<usize>,
}
impl MarkMotion {
    const REST: Self = Self { levels: [0.0; 11], waits: [0.0; 11], target: None };
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
            let moving = (dt - f64::from(self.waits[i])).max(0.0);
            self.levels[i] = approach(self.levels[i], target, moving, env);
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
///
/// A departure waits out `fade_out_delay` before its sweep starts, so the
/// horizon carries that too.
fn horizon(duration: f32, mark_delay: f32, fade_out_delay: f32) -> f64 {
    f64::from(duration.max(0.0)) * 2.0 + f64::from(mark_delay) + f64::from(fade_out_delay) + 0.001
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
// What an absent entry reads as, by reference. Statics, because a const that
// owns a `Vec` is not promoted to one: `&Motion::REST` would build a fresh
// temporary for every visible node on every frame.
static REST: Motion = Motion::REST;
static POSED: Motion = Motion::POSED;
impl Motion {
    /// A node nothing has lit, and what every position [`NodeMotion`] holds no
    /// entry for reads as.
    const REST: Self = Self::still_at(0.0, false);
    /// A node at rest that the audio ring has posed, settled and waiting for a
    /// note. Nothing in `advance` moves it.
    const POSED: Self = Self::still_at(1.0, true);
    /// Spelled out rather than `..Self::REST`: `edges` owns an allocation, so
    /// a const cannot drop the fields such an update leaves behind.
    const fn still_at(progress: f32, audio_waiting: bool) -> Self {
        Self {
            progress: [progress; 11],
            seen: [false; 11],
            edges: Vec::new(),
            levels: [0.0; 11],
            colors: [0.0; 11],
            targets: [0.0; 11],
            readings: [IntensityReading::REST; 11],
            gate: false,
            melody: MarkMotion::REST,
            bass: MarkMotion::REST,
            level_wait: [0.0; 11],
            audio_waiting,
        }
    }
    /// Nothing held, nothing moving and nothing left to fade: `advance` would
    /// write back exactly what is here. Most visible nodes are this at once,
    /// and [`NodeMotion`] keeps no entry for one.
    fn at_rest(&self) -> bool {
        self.still() && !self.audio_waiting && self.progress == [0.0; 11]
    }
    /// [`at_rest`](Self::at_rest) but for an audio pose: exactly [`Self::POSED`]
    /// in everything that is read before it is written.
    fn posed(&self) -> bool {
        self.still() && self.audio_waiting && self.progress == [1.0; 11]
    }
    /// No gate, no mark, and nothing delayed, lit or fading.
    fn still(&self) -> bool {
        !self.gate
            && self.melody.target.is_none()
            && self.bass.target.is_none()
            && self.edges.is_empty()
            && self.seen == [false; 11]
            && [self.levels, self.colors, self.targets]
                .iter()
                .chain([
                    &self.melody.levels,
                    &self.melody.waits,
                    &self.bass.levels,
                    &self.bass.waits,
                ])
                .all(|values| values.iter().all(|&v| v == 0.0))
    }
    /// Send a gate edge through the slices, reaching slice `i` after
    /// `delay[i]` (infinite: never). It overtakes any older edge that would
    /// reach a slice no sooner, so each slice follows the newest edge to have
    /// reached it, and the edges still travelling reach it in order.
    fn push_edge(&mut self, on: bool, delay: [f32; 11]) {
        let mut delay = delay.map(f64::from);
        for edge in &mut self.edges {
            for (old, &new) in edge.delay.iter_mut().zip(&delay) {
                if *old - edge.age >= new {
                    *old = f64::INFINITY;
                }
            }
        }
        for (seen, delay) in self.seen.iter_mut().zip(&mut delay) {
            if *delay <= 0.0 {
                *seen = on;
                *delay = f64::INFINITY;
            }
        }
        self.edges.retain(|edge| edge.delay.iter().any(|d| d.is_finite()));
        if delay.iter().any(|d| d.is_finite()) {
            self.edges.push(SliceEdge { on, age: 0.0, delay });
        }
    }
    /// Every slice at once, as an arrival seeded settled is.
    fn settle_slices(&mut self, on: bool) {
        self.seen = [on; 11];
        self.edges.clear();
    }
    fn advance(&mut self, dt: f64, env: &Envelope) {
        if self.at_rest() {
            return;
        }
        self.melody.advance(dt, env);
        self.bass.advance(dt, env);
        // The slice reveal runs the envelope's own length rather than a second
        // duration handed in beside it: `ViewConfig::envelope` puts one time on
        // both ends, so the two were always the same number and a parameter
        // for each is a pair that can be made to disagree.
        for i in 0..11 {
            // Split at each edge reaching this slice inside the step, so the
            // pose is the same whatever the frame cadence.
            let (mut progress, mut moved) = (self.progress[i], 0.0);
            for edge in &mut self.edges {
                let reach = edge.delay[i] - edge.age;
                if reach <= dt {
                    progress = approach(progress, f32::from(self.seen[i]), reach - moved, env);
                    moved = reach;
                    self.seen[i] = edge.on;
                    edge.delay[i] = f64::INFINITY;
                }
            }
            self.progress[i] = if self.audio_waiting {
                1.0
            } else {
                approach(progress, f32::from(self.seen[i]), dt - moved, env)
            };
            // Undelayed per slice, and over the WHOLE duration, on purpose.
            // This is the node's presence rather than any one slice's: it
            // reaches the shader as `ink.w` (`params.x` for a slot no note
            // lights) and is multiplied by the slice's own reveal, which is
            // already nothing before that slice's delay.
            //
            // The departure is the other way round. Falling from the off, the
            // presence dimmed every slice at once and reached 0 before the late
            // ones had begun to retract, so the stagger barely showed and the
            // exit ran one fade where the entrance ran `1 + stagger_spread`. So
            // a departure holds it for the LAST slice's delay, one wait for
            // every slot lit at the off (`level_wait`), and both ends span the
            // same.
            let level_dt = (dt - f64::from(self.level_wait[i])).max(0.0);
            self.level_wait[i] = (self.level_wait[i] - dt as f32).max(0.0);
            self.levels[i] = approach(self.levels[i], self.targets[i], level_dt, env);
            self.colors[i] = approach(self.colors[i], self.targets[i], dt, env);
        }
        for edge in &mut self.edges {
            edge.age += dt;
        }
        self.edges.retain(|edge| edge.delay.iter().any(|d| d.is_finite()));
        // A departure that has run out of ink is over, whatever its slices are
        // still holding. The reveal reaches the shader multiplied by this
        // node's presence, so a slice caught mid-retraction behind a zero level
        // draws nothing -- but it is not nothing to the NEXT press, whose
        // presence rises at once and would show those leftover poses rather
        // than the entrance it owes: the same repeated note animating two ways
        // depending on invisible state.
        if !self.gate && !self.audio_waiting && self.levels.iter().all(|&l| l <= 0.0) {
            self.progress = [0.0; 11];
            self.settle_slices(false);
        }
    }
}
/// Whether `pos` is among `visible`, a frame's positions in lattice order.
fn shown(visible: &[LatticePos], pos: &LatticePos) -> bool {
    visible.binary_search_by_key(&lattice_order(pos), lattice_order).is_ok()
}
/// The octave slot a held pitch lights on the node at `cents`, inside the
/// node's `(lo, hi)` slots.
fn slot_of(lo: i32, hi: i32, cents: f32, pitch: f32) -> usize {
    (((pitch - cents / 100.0) / 12.0).round() as i32).clamp(lo, hi).clamp(0, 10) as usize
}
/// Each slot's reading from the held notes lighting it: the largest each
/// display gets. A slot none lights keeps its last, so a release fades from
/// where its note left off.
#[derive(Default)]
struct SlotReadings([Option<IntensityReading>; 11]);
impl SlotReadings {
    fn add(&mut self, slot: usize, reading: IntensityReading) {
        let current = &mut self.0[slot];
        *current = Some(current.map_or(reading, |r| r.max(reading)));
    }
    fn apply(self, motion: &mut Motion) {
        for (slot, reading) in self.0.into_iter().enumerate() {
            if let Some(reading) = reading {
                motion.readings[slot] = reading;
            }
        }
    }
}
impl NodeMotion {
    /// Start over from nothing, as a surface seen for the first time does.
    fn reset(&mut self) {
        *self = Self {
            visible: std::mem::take(&mut self.visible),
            #[cfg(test)]
            dense: self.dense,
            #[cfg(test)]
            reached: self.reached,
            ..Self::default()
        };
        self.visible.clear();
    }
    /// Whether the table keeps an entry for every visible node.
    fn dense(&self) -> bool {
        #[cfg(test)]
        return self.dense;
        #[cfg(not(test))]
        false
    }
    /// Whether a position has carried MIDI ink or marks. Audio poses emit no light.
    pub fn owns_light(&self, pos: LatticePos) -> bool {
        self.nodes.get(&pos).is_some_and(|motion| {
            motion.gate
                || motion
                    .levels
                    .iter()
                    .chain(&motion.melody.levels)
                    .chain(&motion.bass.levels)
                    .any(|&level| level > 0.0)
        })
    }

    /// Conservative pitch candidates for the same bounded history `step` replays.
    /// This selects geometry only; `step` remains the sole event/envelope reader.
    pub fn light_classes(
        tracker: &NoteTracker,
        view: &ViewConfig,
        env: &Envelope,
        now: f64,
    ) -> Vec<PitchClass> {
        let floor =
            now - horizon(env.fade_time, view.mark_delay, view.note_animation.fade_out_delay);
        let mut classes: Vec<_> = tracker.voices().map(|voice| voice.pitch_class).collect();
        for note in tracker.roll().notes().filter(|note| note.end.is_none_or(|at| at >= floor)) {
            for ((at, pitch), (end, next)) in note.segments(now) {
                if at <= now && end >= floor {
                    classes.push(PitchClass::from_cents(pitch * 100.0));
                    classes.push(PitchClass::from_cents(next * 100.0));
                }
            }
        }
        classes.sort_unstable();
        classes.dedup();
        classes
    }

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
        first: bool,
    ) {
        let duration = env.fade_time;
        // The sweep every node's slices follow: one layout and one fade for
        // the whole step, so the same for each.
        let order = view.note_animation.delays(&scene.octave_layout, duration);
        let last = order.into_iter().fold(0.0, f32::max);
        // A departure's sweep starts once the fade-out delay has passed.
        let hold = view.note_animation.fade_out_delay;
        let departure = order.map(|delay| delay + hold);
        let high = self.held.values().map(|v| v.pitch).max_by(f32::total_cmp);
        let low = self.held.values().map(|v| v.pitch).min_by(f32::total_cmp);
        let dense = self.dense();
        for node in &scene.nodes {
            let node_class = PitchClass::from_cents(node.cents);
            let (lo, hi) = scene.octave_layout.slots(node.cents);
            // The slice a slot is drawn as, if it is drawn (see `Motion`).
            let displayed = |slot: usize| {
                usize::try_from(slot as i32 - lo).ok().filter(|&i| i < (hi - lo + 1) as usize)
            };
            let mut targets = [0.0; 11];
            let mut melody = None;
            let mut bass = None;
            let mut preexisting = false;
            let mut readings = SlotReadings::default();
            // The one node x held-note match per step: occupancy, marks and
            // readings all come out of it, rather than a second pass over
            // the same pairs for the readings.
            for (id, held) in &self.held {
                if !tuning.matches(held.class, node_class) {
                    continue;
                }
                preexisting |= self.at.is_some_and(|at| f64::from_bits(id.1) < at);
                let slot = slot_of(lo, hi, node.cents, held.pitch);
                // Activation measures occupancy; intensity rides beside it.
                targets[slot] = 1.0;
                readings.add(slot, held.reading);
                if Some(held.pitch) == high {
                    melody = Some(slot);
                }
                if Some(held.pitch) == low {
                    bass = Some(slot);
                }
            }
            let gate = targets.iter().any(|&v| v > 0.0);
            let audio =
                scene.spectral.ring_draws() && fade.level(&scene.octave_layout, node.cents) > 0.0;
            let pos = node.lattice_pos;
            let (motion, vacant) = match self.nodes.entry(pos) {
                Entry::Occupied(entry) => (entry.into_mut(), false),
                Entry::Vacant(entry) => {
                    let posed = self.posed.contains(&pos);
                    // Nothing lights it, and the ring neither starts nor stops
                    // ringing at it: every write below puts back what `REST`,
                    // or the pose, already holds.
                    if !gate && audio == posed && !dense {
                        continue;
                    }
                    if posed {
                        self.posed.remove(&pos);
                    }
                    (entry.insert(if posed { Motion::POSED } else { Motion::REST }), true)
                }
            };
            // New to the view on the step's first pass alone, as it was while
            // every visible node had an entry: that pass entered every node,
            // so a later one found each already there. Past it, an absent
            // node is one at rest rather than one arriving.
            let in_view = |visible: &[LatticePos]| shown(visible, &pos);
            let newly_visible = |visible: &[LatticePos]| {
                if dense {
                    vacant
                } else {
                    vacant && first && !in_view(visible)
                }
            };
            #[cfg(test)]
            if vacant && !dense && in_view(&self.visible) {
                self.reached.revived += 1;
            }
            let was_lit = std::mem::replace(&mut motion.targets, targets);
            readings.apply(motion);
            motion.melody.target(melody, view.mark_delay);
            motion.bass.target(bass, view.mark_delay);
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
            let posed = motion.audio_waiting;
            if gate {
                motion.audio_waiting = false;
            }
            // Each slice sees the gate late by its place in the sweep, on the
            // way in and on the way out alike, however the note was cut short:
            // a re-press mid-departure ripples round the ring rather than
            // turning every slice at once. The slices a note lights take the
            // rise first, so they appear first; on the way out they keep their
            // place in the sweep like any other.
            let settle = seed_settled || (gate && preexisting && newly_visible(&self.visible));
            if gate {
                // A slot lit now and not before starts at once: at the rise
                // that is every lit slot, and with the gate held it is one a
                // second note lights, rather than waiting out the arrival it
                // joined. Only the rise sends the rest of the sweep.
                let mut delay = if motion.gate { [f32::INFINITY; 11] } else { order };
                for slot in (0..11).filter(|&slot| targets[slot] > 0.0 && was_lit[slot] == 0.0) {
                    if let Some(i) = displayed(slot) {
                        delay[i] = 0.0;
                    }
                }
                if !motion.gate {
                    motion.level_wait = [0.0; 11];
                }
                // A lifetime lit over an audio pose starts at full progress,
                // and one seeded settled is already there.
                if posed || settle {
                    motion.settle_slices(true);
                } else {
                    motion.push_edge(true, delay);
                }
            } else if motion.gate {
                // The presence of a slot lit at the off outlasts the slice
                // that retracts last; its colour does not wait (`colors`).
                motion.level_wait = was_lit.map(|t| if t > 0.0 { hold + last } else { 0.0 });
                motion.push_edge(false, departure);
            }
            motion.gate = gate;
            #[cfg(test)]
            if gate && preexisting && vacant && !seed_settled && !dense {
                if settle {
                    self.reached.scrolled_in += 1;
                } else if in_view(&self.visible) {
                    self.reached.lit_in_view += 1;
                } else {
                    self.reached.lit_after_entering += 1;
                }
            }
            if settle && gate {
                motion.progress = [1.0; 11];
                motion.settle_slices(true);
                motion.level_wait = [0.0; 11];
                motion.levels = motion.targets;
                motion.colors = motion.targets;
                motion.melody.advance(f64::from(duration + view.mark_delay), env);
                motion.bass.advance(f64::from(duration + view.mark_delay), env);
            }
        }
    }
    /// Each slot's reading from the held notes lighting it, and nothing else,
    /// so a replay can bring a note to its reading at the off without moving
    /// any gate. [`gates`](Self::gates) takes the same readings in its own
    /// match; this is for the one moment that needs them alone.
    fn read_slots(&mut self, scene: &Scene, tuning: &Tuning) {
        for node in &scene.nodes {
            let Some(motion) = self.nodes.get_mut(&node.lattice_pos) else {
                continue;
            };
            let node_class = PitchClass::from_cents(node.cents);
            let (lo, hi) = scene.octave_layout.slots(node.cents);
            let mut readings = SlotReadings::default();
            for held in self.held.values().filter(|held| tuning.matches(held.class, node_class)) {
                readings.add(slot_of(lo, hi, node.cents, held.pitch), held.reading);
            }
            readings.apply(motion);
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
        let horizon = horizon(env.fade_time, view.mark_delay, view.note_animation.fade_out_delay);
        // A hidden surface cannot benefit from replaying minutes of settled
        // history. Seed current state and replay only the visible horizon.
        if self.at.is_some_and(|at| now < at || now - at > horizon) {
            self.reset();
        }
        let floor = now - horizon;
        let intensity = view.intensity;
        // A note that ended before every horizon a host can set can never
        // count as late again, whatever the settings do next, so its cursor
        // has nothing left to guard. Skipping it bounds this scan by the last
        // few seconds of playing rather than by the whole roll.
        let retained = now
            - horizon.max(self::horizon(
                FADE_MAX,
                crate::MARK_DELAY_MAX,
                *crate::FADE_OUT_DELAY_RANGE.end(),
            ));
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
            self.reset();
        }
        self.seen = seen;
        let initial = self.at.is_none();
        let begin = self.at.unwrap_or(now - horizon);
        let mut edges = Vec::new();
        // Each ended note's reading at its off. Expressions are not edges, so
        // a replayed held note otherwise reads what it did at its last edge —
        // the seed or onset, for a note that never bent — and its slot would
        // release from that rather than from where the note left off.
        let mut endings = HashMap::default();
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
                endings.insert(id, intensity.read(note.velocity, note.expressions_at(at)));
            }
            // observed_until is loss of observation, not a factual key-up.
            // Current voices reconcile it below without inventing an off time.
        }
        // The checkpoint already has the previous frame's targets. Recompute
        // them only for initial seeding, an event, or current-state reconciliation.
        let mut first = true;
        if initial {
            self.gates(
                scene,
                tuning,
                view,
                env,
                fade,
                tracker,
                now,
                true,
                std::mem::take(&mut first),
            );
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
            // Capture the event-time reading before either a bend or an off
            // removes a note from its old slot. The departing ink must not
            // depend on whether a frame happened between expression and bend.
            let mut departing = false;
            for edge in edges[index..].iter().take_while(|e| e.at == time) {
                let reading =
                    edge.value.map(|held| held.reading).or_else(|| endings.get(&edge.id).copied());
                if let (Some(held), Some(reading)) = (self.held.get_mut(&edge.id), reading) {
                    held.reading = reading;
                    departing = true;
                }
            }
            if departing {
                self.read_slots(scene, tuning);
            }
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
            self.gates(
                scene,
                tuning,
                view,
                env,
                fade,
                tracker,
                now,
                false,
                std::mem::take(&mut first),
            );
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
        self.gates(scene, tuning, view, env, fade, tracker, now, false, std::mem::take(&mut first));
        self.advance(0.0, env);
        self.at = Some(now);
        // The scene's nodes are in lattice order already (`derive_scene`), so
        // this is a copy and a check.
        self.visible.clear();
        self.visible.extend(scene.nodes.iter().map(|n| n.lattice_pos));
        if !self.visible.is_sorted_by_key(lattice_order) {
            self.visible.sort_unstable_by_key(lattice_order);
        }
        let dense = self.dense();
        let Self { nodes, posed, visible, .. } = self;
        posed.retain(|pos| shown(visible, pos));
        nodes.retain(|pos, motion| {
            if !shown(visible, pos) {
                return false;
            }
            if dense {
                return true;
            }
            if motion.posed() {
                posed.insert(*pos);
                return false;
            }
            !motion.at_rest()
        });
        // Give back what a lattice-wide chord grew, once it has faded, and
        // with room either side so a held chord does not grow and shrink it.
        if nodes.capacity() > 4 * (nodes.len() + 64) {
            nodes.shrink_to(2 * nodes.len());
        }
        if posed.capacity() > 4 * (posed.len() + 64) {
            posed.shrink_to(2 * posed.len());
        }
        for node in &mut scene.nodes {
            let pos = &node.lattice_pos;
            let resting = if self.posed.contains(pos) { &POSED } else { &REST };
            let motion = self.nodes.get(pos).unwrap_or(resting);
            node.slice_progress = motion.progress;
            // Opacity fades each slot's colour, and the node's presence with
            // it, while the envelope under it runs untouched. Thickness reshapes a
            // LIT slot only: a slot keeps its last reading once released, and
            // an unlit one draws the ghost at full width whatever it read, so
            // it goes as 1 and the shader keeps its full-slice path there —
            // and its mark goes back to standing off the band.
            let fades = motion.readings.map(|reading| reading.opacity);
            node.octaves = std::array::from_fn(|i| motion.colors[i] * fades[i]);
            node.thickness = std::array::from_fn(|i| {
                if node.octaves[i] > 0.0 {
                    motion.readings[i].thickness
                } else {
                    1.0
                }
            });
            // Never below the colour: the two ramp together and only the
            // presence ever waits.
            node.activation = (0..11).map(|i| motion.levels[i] * fades[i]).fold(0.0, f32::max);
            node.envelope = motion.levels.iter().copied().fold(0.0, f32::max);
            node.departing = !motion.gate;
            let (melody_slots, melody_level, melody_slot) = motion.melody.strongest();
            let (bass_slots, bass_level, bass_slot) = motion.bass.strongest();
            node.melody_slots = melody_slots;
            node.melody_level = melody_level * fades[melody_slot];
            node.bass_slots = bass_slots;
            node.bass_level = bass_level * fades[bass_slot];
            let color = |slot| {
                let pitch = scene.octave_layout.slot_pitch(slot as i32, node.cents);
                let t = crate::color::ramp_t(pitch, scene.darkest_pitch, scene.brightest_pitch);
                crate::color::sample_lut(t as f32, &scene.pitch_lut, scene.pitch_lut_spacing)
            };
            node.melody_color = color(melody_slot);
            node.bass_color = color(bass_slot);
            if scene.spectral.ring_draws() {
                node.audio_ring = fade.level(&scene.octave_layout, node.cents).max(node.activation);
            }
            // The node glow is the drawn colour: the most opaque slot or mark,
            // each after its Opacity, as the bloom reads it, rather than the
            // presence that waits on the slices. Stateless
            // snapshots draw it as it stands; the shell's glow pass carries it
            // as its target. The ink strip's history (`ink_history` in
            // harmonigraph-render) compares this same max of what it ships, so
            // the colour picks attack or release by the test the level does.
            node.glow.level = node.drawn_level().max(node.melody_level).max(node.bass_level);
        }
        scene.pluses = crate::derive::derive_pluses(
            view,
            &scene.nodes,
            crate::grey_of_lightness(view.marker_ink),
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::NoteAnimationConfig;

    impl NodeMotion {
        /// A position's motion, absent read as the rest it stands for.
        fn at(&self, pos: LatticePos) -> &Motion {
            let resting = if self.posed.contains(&pos) { &POSED } else { &REST };
            self.nodes.get(&pos).unwrap_or(resting)
        }
    }

    #[test]
    fn halo_candidates_keep_between_frame_bends_after_voice_pruning() {
        use harmonigraph_core::{NoteEvent, NoteEventKind, SourceId};
        let mut tracker = NoteTracker::new();
        let on = NoteEvent::on(0.0, SourceId::DIRECT, 0, 60, 1.0);
        tracker.handle_event(on);
        tracker.handle_event(NoteEvent {
            time: 0.02,
            kind: NoteEventKind::Tuning { semitones: 2.0 },
            ..on
        });
        tracker.handle_event(NoteEvent {
            time: 0.04,
            kind: NoteEventKind::Tuning { semitones: 4.0 },
            ..on
        });
        tracker.handle_event(NoteEvent::off(0.06, SourceId::DIRECT, 0, 60));
        let view = ViewConfig::default();
        let frame = FrameParams { fade_time: 0.1, ..Default::default() };
        let env = view.envelope(&frame);
        tracker.prune(0.2, &env);
        assert_eq!(
            tracker.voices().count(),
            0,
            "fixture must rely on bounded roll, not current voices"
        );
        let classes = NodeMotion::light_classes(&tracker, &view, &env, 0.2);
        for note in [60, 62, 64] {
            assert!(classes.contains(&PitchClass::from_midi_note(note)));
        }
        assert!(NodeMotion::light_classes(&tracker, &view, &env, 2.0).is_empty());
    }

    use crate::{derive_scene, Camera, FrameParams, MIDDLE_C_SLOT};
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
    /// The fresh view with every note drawn in full and every slice moving
    /// at once, so a test about timing reads one slice's progress and one
    /// node's activation without the fresh intensity routes or stagger in
    /// them. The tests about the stagger or intensity set their own.
    fn view() -> ViewConfig {
        let mut view =
            ViewConfig { intensity: crate::IntensitySettings::unrouted(), ..ViewConfig::default() };
        view.note_animation.stagger_spread = 0.0;
        view
    }
    fn origin(scene: &Scene) -> &crate::NodeInstance {
        scene.nodes.iter().find(|n| n.lattice_pos == LatticePos::ORIGIN).unwrap()
    }
    /// The octave slot `note` lights on the origin, and the slice that slot is
    /// drawn as: `octaves` is indexed by the one, `slice_progress` by the other.
    fn lit_by(scene: &Scene, note: u8) -> (usize, usize) {
        let cents = origin(scene).cents;
        let (lo, hi) = scene.octave_layout.slots(cents);
        let slot = slot_of(lo, hi, cents, f32::from(note));
        (slot, (slot as i32 - lo) as usize)
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
            let view = ViewConfig { fade_shape: shape, ..view() };
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
    /// Once the spread became a start offset instead the slices outlived the
    /// ink by `stagger_spread` of a fade, and a press inside that window
    /// started from leftover poses behind ink that rises at once: the same
    /// repeated note animating two different ways depending on invisible
    /// state.
    #[test]
    fn a_faded_departure_leaves_no_pose_for_the_next_press_to_carry() {
        let mut view = ViewConfig { fade_shape: 0.0, mark_delay: 0.0, ..view() };
        view.note_animation.stagger_spread = 0.9;
        // One press just after the ink runs out, and one well past it.
        // Both owe the same fresh entrance.
        let mut poses = Vec::new();
        for repress in [1.6, 4.0] {
            let mut tracker = NoteTracker::new();
            let mut motion = NodeMotion::default();
            // A short note: its lit slice arrives at once and takes the
            // release late in the sweep, so it is still well out when the
            // presence, held at 0.3 for the last delay, has faded.
            tracker.handle_event(on(0.0, 60));
            let first = draw(&mut motion, &mut tracker, &view, 0.0, false);
            let (_, lit) = lit_by(&first, 60);
            tracker.handle_event(off(0.3, 60));
            let fading = draw(&mut motion, &mut tracker, &view, 1.45, false);
            assert!(origin(&fading).activation > 0.0, "the ink ran out early");
            assert!(origin(&fading).slice_progress[lit] > 0.5, "the lit slice left in time");
            let dark = draw(&mut motion, &mut tracker, &view, 1.55, false);
            assert_eq!(origin(&dark).activation, 0.0, "past the departure's span");
            assert_eq!(
                origin(&dark).slice_progress,
                [0.0; 11],
                "the ink is gone and the poses are not",
            );
            draw(&mut motion, &mut tracker, &view, repress, false);
            tracker.handle_event(on(repress, 60));
            let back = draw(&mut motion, &mut tracker, &view, repress + 0.01, false);
            poses.push(origin(&back).slice_progress);
        }
        // A press that reversed leftover poses instead holds a spread of
        // them rather than one tick's worth.
        assert_eq!(poses[0], poses[1], "the earlier press did not start a fresh entrance");
    }
    /// Each slice's lowest pose over `frames`, and when it reached it.
    fn dips(frames: &[(f64, Scene)], span: usize) -> Vec<(f32, f64)> {
        (0..span)
            .map(|i| {
                frames.iter().fold((f32::INFINITY, 0.0), |(low, at), (now, scene)| {
                    let p = origin(scene).slice_progress[i];
                    if p < low {
                        (p, *now)
                    } else {
                        (low, at)
                    }
                })
            })
            .collect()
    }
    /// Middle C at spread 0.9, drawn every 0.02 from 0 to `end`, each event
    /// delivered on the frame that reaches it: the sweep's delays, its span,
    /// and the frames.
    fn played(events: &[NoteEvent], end: f64) -> ([f32; 11], usize, Vec<(f64, Scene)>) {
        let mut view = ViewConfig { fade_shape: 0.0, mark_delay: 0.0, ..view() };
        view.note_animation.stagger_spread = 0.9;
        let (mut tracker, mut motion) = (NoteTracker::new(), NodeMotion::default());
        let mut pending = events.iter().peekable();
        let frames: Vec<_> = (0..)
            .map(|k| f64::from(k) * 0.02)
            .take_while(|&now| now <= end)
            .map(|now| {
                while let Some(event) = pending.next_if(|event| event.time <= now) {
                    tracker.handle_event(*event);
                }
                (now, draw(&mut motion, &mut tracker, &view, now, false))
            })
            .collect();
        let layout = &frames[0].1.octave_layout;
        (view.note_animation.delays(layout, 1.0), layout.span as usize, frames)
    }
    /// A quick re-press keeps the stagger: the release and the press both
    /// travel the sweep, so every slice dips in its turn, the seam first,
    /// rather than the few the release had reached turning back at once while
    /// the rest never move. The lit slice takes the re-press at once, before
    /// the release has reached it, so it never leaves -- but its colour does
    /// not wait, so a note tapped again and again dims between the taps.
    #[test]
    fn a_quick_repress_ripples_round_the_ring() {
        let (order, span, frames) = played(&[on(0.0, 60), off(5.0, 60), on(5.16, 60)], 8.0);
        let (slot, lit) = lit_by(&frames[0].1, 60);
        let after = &frames[frames.iter().position(|(now, _)| *now >= 5.0).unwrap()..];
        let dips = dips(after, span);
        for (i, &(low, at)) in dips.iter().enumerate().filter(|&(i, _)| i != lit) {
            assert!((low - 0.84).abs() < 0.03, "slice {i} dipped to {low}, not by the gap");
            let expected = 5.16 + f64::from(order[i]);
            assert!((at - expected).abs() <= 0.021, "slice {i} dipped at {at}, not {expected}");
        }
        assert_eq!(dips[lit].0, 1.0, "the lit slice retracted under a re-press");
        // The light and the mark go with the colour, and the presence holds.
        let lowest = |f: &dyn Fn(&crate::NodeInstance) -> f32| {
            after.iter().map(|(_, s)| f(origin(s))).fold(1.0, f32::min)
        };
        for (what, low) in [
            ("colour", lowest(&|n| n.octaves[slot])),
            ("light", lowest(&|n| n.glow.level)),
            ("melody mark", lowest(&|n| n.melody_level)),
        ] {
            assert!((low - 0.84).abs() < 0.03, "the lit slot's {what} held at {low}");
        }
        assert_eq!(lowest(&|n| n.activation), 1.0, "the presence left with the colour");
        assert_eq!(origin(&frames.last().unwrap().1).slice_progress, [1.0; 11]);
    }
    /// A note released before its arrival finished plays the same short rise
    /// and fall on every unlit slice, each in its turn: a pulse travelling
    /// round the ring, rather than the slices it reached turning back at once
    /// and the rest never starting. The lit slice, which arrived first and
    /// takes the release late in the sweep, keeps arriving until then, while
    /// its colour leaves at the release.
    #[test]
    fn a_short_note_sends_its_pulse_round_the_ring() {
        let (order, span, frames) = played(&[on(0.0, 60), off(0.3, 60)], 2.0);
        let (slot, lit) = lit_by(&frames[0].1, 60);
        assert!(
            (0..span).any(|i| i != lit && order[i] > 0.3),
            "every unlit slice had started by the release: {order:?}"
        );
        for i in (0..span).filter(|&i| i != lit) {
            let peak = frames.iter().map(|(_, s)| origin(s).slice_progress[i]).fold(0.0, f32::max);
            assert!((peak - 0.3).abs() < 0.03, "slice {i} peaked at {peak}, not 0.3");
        }
        // Two tenths after the release, where the presence still holds.
        let (_, later) = frames.iter().find(|(now, _)| *now >= 0.5).unwrap();
        let colour = origin(later).octaves[slot];
        assert!((colour - 0.1).abs() < 0.03, "the lit slot's colour waited at {colour}");
        assert!(origin(later).activation >= 0.3 - 1e-6, "the presence left with the colour");
    }
    /// The fade-out delay holds the whole departure -- the sweep and the
    /// presence -- and not the colour, so the released wheel stands uncoloured
    /// through the wait, then leaves in the sweep it would have left in at the
    /// release. A press inside the wait takes it back with nothing retracted.
    #[test]
    fn the_fade_out_delay_holds_the_departure_but_not_the_colour() {
        let hold = 0.5;
        let mut view = ViewConfig { fade_shape: 0.0, mark_delay: 0.0, ..view() };
        view.note_animation.stagger_spread = 0.9;
        view.note_animation.fade_out_delay = hold;
        let run = |events: &[NoteEvent]| {
            let (mut tracker, mut motion) = (NoteTracker::new(), NodeMotion::default());
            let mut pending = events.iter().peekable();
            (0..)
                .map(|k| f64::from(k) * 0.01)
                .take_while(|&now| now <= 6.0)
                .map(|now| {
                    while let Some(event) = pending.next_if(|event| event.time <= now) {
                        tracker.handle_event(*event);
                    }
                    (now, draw(&mut motion, &mut tracker, &view, now, false))
                })
                .collect::<Vec<_>>()
        };
        let off_at = 2.5;
        let frames = run(&[on(0.0, 60), off(off_at, 60)]);
        let (slot, _) = lit_by(&frames[0].1, 60);
        let layout = &frames[0].1.octave_layout;
        let (order, span) = (view.note_animation.delays(layout, 1.0), layout.span as usize);
        let at = |t: f64| origin(&frames.iter().find(|(now, _)| *now >= t - 1e-9).unwrap().1);
        // Most of the way through the wait: the colour has gone that far, and
        // nothing else has moved.
        let waiting = at(off_at + 0.4);
        assert!((waiting.octaves[slot] - 0.6).abs() < 0.02, "the colour waited");
        assert_eq!(waiting.activation, 1.0, "the presence did not wait");
        assert_eq!(waiting.slice_progress[..span], [1.0; 11][..span], "a slice did not wait");
        for (i, &delay) in order.iter().enumerate().take(span) {
            let leaves = frames
                .iter()
                .find(|(now, s)| *now >= off_at && origin(s).slice_progress[i] < 1.0)
                .map(|(now, _)| *now)
                .unwrap();
            let expected = off_at + f64::from(hold + delay);
            assert!(
                (leaves - expected).abs() <= 0.02,
                "slice {i} left at {leaves}, not {expected}"
            );
        }
        let last = order.iter().copied().fold(0.0, f32::max);
        let end = off_at + f64::from(hold + last) + 1.0;
        assert!(at(end - 0.05).activation > 0.0, "the presence left before the last slice");
        assert_eq!(at(end + 0.02).activation, 0.0, "the departure outlasted its span");
        // Pressed again inside the wait, nothing has left to come back.
        let frames = run(&[on(0.0, 60), off(off_at, 60), on(off_at + 0.3, 60)]);
        let after = frames.iter().filter(|(now, _)| *now >= off_at).map(|(_, s)| origin(s));
        for node in after {
            assert_eq!(node.activation, 1.0, "the presence fell under a re-press");
            assert_eq!(node.slice_progress[..span], [1.0; 11][..span], "a slice retracted");
        }
    }
    /// A surface hidden through most of a wait far longer than the fade comes
    /// back to the wheel still standing: the replay horizon carries the delay,
    /// or the reset would seed the node at rest mid-wait.
    #[test]
    fn a_surface_reopened_inside_the_fade_out_delay_finds_the_wheel_waiting() {
        let mut view = view();
        view.note_animation.fade_out_delay = *crate::FADE_OUT_DELAY_RANGE.end();
        let (mut tracker, mut motion) = (NoteTracker::new(), NodeMotion::default());
        tracker.handle_event(on(0.0, 60));
        draw_duration(&mut motion, &mut tracker, &view, 0.0, false, 0.1);
        tracker.handle_event(off(0.5, 60));
        draw_duration(&mut motion, &mut tracker, &view, 0.5, false, 0.1);
        // Hidden for well past twice the fade, which alone resets the surface.
        let back = draw_duration(&mut motion, &mut tracker, &view, 2.0, false, 0.1);
        assert_eq!(origin(&back).activation, 1.0, "the wheel left inside its wait");
        assert_eq!(origin(&back).octaves, [0.0; 11], "the colour waited");
    }
    #[test]
    fn late_delivered_short_notes_recover_the_factual_timeline_once() {
        let view = ViewConfig { fade_shape: 0.0, mark_delay: 0.0, ..view() };
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

        // Every note above has ENDED by the frame it arrives at, so its late
        // off says it is late whatever its onset says. A note still held
        // carries no off, and one bent since the frame carries no late bend
        // either: its onset is the only edge that says it is late. A bend on
        // a held note is the same, the other way round.
        let bend = |time: f64, semitones: f32| NoteEvent {
            time,
            kind: harmonigraph_core::NoteEventKind::Tuning { semitones },
            ..on(0.0, 60)
        };
        let held_late = |deliver_after: bool| {
            let mut tracker = NoteTracker::new();
            let mut motion = NodeMotion::default();
            draw(&mut motion, &mut tracker, &view, 0.0, false);
            if deliver_after {
                draw(&mut motion, &mut tracker, &view, 0.21, false);
            }
            tracker.handle_event(on(0.15, 60));
            if !deliver_after {
                draw(&mut motion, &mut tracker, &view, 0.15, false);
            }
            tracker.handle_event(bend(0.215, 2.0));
            origin(&draw(&mut motion, &mut tracker, &view, 0.22, false)).activation
        };
        let (late, timely) = (held_late(true), held_late(false));
        assert!(timely > 0.03, "the onset's fixture must have lit the old node: {timely}");
        assert!((late - timely).abs() < 1e-5, "a late onset left at {late}, not {timely}");
        let bent_late = |deliver_after: bool| {
            let mut tracker = NoteTracker::new();
            let mut motion = NodeMotion::default();
            tracker.handle_event(on(0.0, 60));
            draw(&mut motion, &mut tracker, &view, 0.0, false);
            draw(&mut motion, &mut tracker, &view, if deliver_after { 1.21 } else { 1.0 }, false);
            tracker.handle_event(bend(1.15, 2.0));
            if !deliver_after {
                draw(&mut motion, &mut tracker, &view, 1.15, false);
            }
            origin(&draw(&mut motion, &mut tracker, &view, 1.22, false)).activation
        };
        let (late, timely) = (bent_late(true), bent_late(false));
        assert!(timely < 0.95, "the bend's fixture must be fading the old node: {timely}");
        assert!((late - timely).abs() < 1e-5, "a late bend left at {late}, not {timely}");
    }
    #[test]
    fn reopening_a_hidden_surface_bounds_replay_to_the_settle_horizon() {
        let view = view();
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
        assert!(motion.at(LatticePos::ORIGIN).gate);
    }
    #[test]
    fn held_nodes_reentering_the_visible_window_do_not_replay_an_entrance() {
        let view = view();
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
    /// A node whose note has faded all the way keeps no entry, so a frame's
    /// motion costs the notes in play rather than every node they ever lit.
    /// The sparse-against-dense run empties its table too, but through its
    /// three-second gap's reset and the seventh of the lattice it scrolls out
    /// of view, either of which empties a table that never lets a node go.
    #[test]
    fn a_faded_note_leaves_nothing_in_the_table() {
        let view = view();
        let mut tracker = NoteTracker::new();
        let mut motion = NodeMotion::default();
        tracker.handle_event(on(0.0, 60));
        draw(&mut motion, &mut tracker, &view, 0.0, false);
        draw(&mut motion, &mut tracker, &view, 1.5, false);
        assert!(motion.nodes.contains_key(&LatticePos::ORIGIN), "the held note kept no entry");
        tracker.handle_event(off(1.5, 60));
        for step in 0..=40 {
            draw(&mut motion, &mut tracker, &view, 1.5 + f64::from(step) * 0.1, false);
        }
        assert!(motion.nodes.is_empty(), "{} entries outlived their fade", motion.nodes.len());
    }
    #[test]
    fn short_stabs_carry_pose_and_opacity_at_event_time_across_frame_cadences() {
        let view = ViewConfig { fade_shape: 0.0, mark_delay: 0.0, ..view() };
        let mut view = view.clone();
        view.note_animation.stagger_spread = NoteAnimationConfig::default().stagger_spread;
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
            // The presence holds at the off's 0.2 while the off travels the
            // sweep, then the re-press raises it from there at event time.
            assert!((node.activation - 0.25).abs() < 1e-5, "{}", node.activation);
            snapshots.push((node.activation, node.slice_progress));
            let repeated = draw(&mut motion, &mut tracker, &view, 0.3, false);
            assert_eq!(node.slice_progress, origin(&repeated).slice_progress);
        }
        for i in 0..11 {
            assert!((snapshots[0].1[i] - snapshots[1].1[i]).abs() < 1e-5, "{i}");
        }
    }
    /// Pressure routed to opacity over a base of 0 fades a slot's ink, the
    /// node's presence and its glow, straight away as the pressure moves, and
    /// nothing else: a note faded to nothing is still held, and still departs
    /// on its own release.
    #[test]
    fn intensity_fades_the_ink_but_not_the_note() {
        use crate::IntensitySource;
        let intensity = crate::IntensitySettings {
            pressure: IntensitySource { opacity: Some(1.0), ..Default::default() },
            opacity_rest: 0.0,
            ..crate::IntensitySettings::unrouted()
        };
        let view = ViewConfig { fade_shape: 0.0, intensity, ..view() };
        let mut tracker = NoteTracker::new();
        let mut motion = NodeMotion::default();
        let slot = |scene: &Scene| {
            let node = origin(scene);
            let lit = node.octaves.iter().position(|&l| l > 0.0);
            (node.activation, lit.map(|i| node.octaves[i]), node.departing, node.glow.level)
        };
        tracker.handle_event(on(0.0, 60));
        let silent = draw(&mut motion, &mut tracker, &view, 1.1, false);
        assert_eq!(slot(&silent), (0.0, None, false, 0.0), "unpressed: invisible, still held");

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
        assert_eq!(slot(&pressed), (0.5, Some(0.5), false, 0.5), "ink and glow follow at once");

        tracker.handle_event(off(1.2, 60));
        let (activation, _, departing, glow) =
            slot(&draw(&mut motion, &mut tracker, &view, 1.7, false));
        assert!(departing, "released");
        assert!((activation - 0.25).abs() < 1e-5, "half the release left, at half: {activation}");
        assert!((glow - 0.25).abs() < 1e-5, "the glow departs with the ink: {glow}");
    }
    /// With velocity and gain routed to Opacity over a rest of about a third
    /// (the fresh routes captured on 2026-09-26), the glow takes the drawn ink
    /// there too, the melody mark included: a soft note's halo is as faint as
    /// its most opaque slice, a loud one's is full.
    #[test]
    fn opacity_routes_dim_the_glow_with_the_ink() {
        use crate::IntensitySource;
        let intensity = crate::IntensitySettings {
            velocity: IntensitySource { opacity: Some(0.606_835_6), thickness: None },
            gain: IntensitySource { opacity: Some(0.511_064_47), thickness: None },
            opacity_rest: 0.302_850_54,
            ..crate::IntensitySettings::unrouted()
        };
        let view = ViewConfig { fade_shape: 0.0, intensity, ..view() };
        let held = |velocity| {
            let mut tracker = NoteTracker::new();
            let mut motion = NodeMotion::default();
            tracker.handle_event(NoteEvent::on(0.0, SourceId::DIRECT, 0, 60, velocity));
            *origin(&draw(&mut motion, &mut tracker, &view, 1.1, false))
        };
        let (soft, loud) = (held(0.0), held(1.0));
        assert!(soft.melody_level > 0.0, "the fixture lights a mark as well as a slot");
        assert!((soft.activation - 0.81).abs() < 0.01, "{}", soft.activation);
        assert_eq!(soft.glow.level, soft.drawn_level().max(soft.melody_level));
        assert!(soft.glow.level < 0.82, "a soft note's halo dims: {}", soft.glow.level);
        assert_eq!(loud.glow.level, 1.0, "a loud note's halo is full");
    }
    /// An off delivered after the frame past it replays the horizon, and the
    /// release it replays fades from the note's reading at its off, as an
    /// off delivered on time does: not from the reading at its onset, which
    /// under pressure routed to opacity over a base of 0 is nothing at all.
    #[test]
    fn a_late_off_releases_from_the_reading_at_the_off() {
        use crate::IntensitySource;
        let intensity = crate::IntensitySettings {
            pressure: IntensitySource { opacity: Some(1.0), ..Default::default() },
            opacity_rest: 0.0,
            ..crate::IntensitySettings::unrouted()
        };
        let view = ViewConfig { fade_shape: 0.0, intensity, ..view() };
        let release = |late: bool| {
            let mut tracker = NoteTracker::new();
            let mut motion = NodeMotion::default();
            tracker.handle_event(on(0.0, 60));
            tracker.handle_event(NoteEvent {
                source: SourceId::DIRECT,
                time: 0.05,
                channel: 0,
                note: 60,
                kind: harmonigraph_core::NoteEventKind::Expression {
                    expression: harmonigraph_core::Expression::Pressure,
                    value: 1.0,
                },
            });
            for frame in 1..10 {
                draw(&mut motion, &mut tracker, &view, f64::from(frame) * 0.1, false);
            }
            if late {
                draw(&mut motion, &mut tracker, &view, 1.0, false);
            }
            tracker.handle_event(off(0.95, 60));
            origin(&draw(&mut motion, &mut tracker, &view, 1.2, false)).activation
        };
        let (on_time, late) = (release(false), release(true));
        assert!(on_time > 0.5, "the on-time release is still fading: {on_time}");
        assert!((late - on_time).abs() < 1e-5, "late {late} against on time {on_time}");
    }

    #[test]
    fn a_bend_leaves_the_same_expression_reading_at_any_frame_cadence() {
        let view = ViewConfig {
            fade_shape: 0.0,
            intensity: crate::IntensitySettings {
                pressure: crate::IntensitySource { opacity: Some(1.0), ..Default::default() },
                opacity_rest: 0.0,
                ..crate::IntensitySettings::unrouted()
            },
            ..view()
        };
        let run = |intermediate| {
            let mut tracker = NoteTracker::new();
            let mut motion = NodeMotion::default();
            let onset = on(0.0, 60);
            tracker.handle_event(onset);
            draw(&mut motion, &mut tracker, &view, 0.0, false);
            draw(&mut motion, &mut tracker, &view, 1.0, false);
            tracker.handle_event(NoteEvent {
                time: 1.05,
                kind: harmonigraph_core::NoteEventKind::Expression {
                    expression: harmonigraph_core::Expression::Pressure,
                    value: 1.0,
                },
                ..onset
            });
            if intermediate {
                draw(&mut motion, &mut tracker, &view, 1.075, false);
            }
            tracker.handle_event(NoteEvent {
                time: 1.1,
                kind: harmonigraph_core::NoteEventKind::Tuning { semitones: 2.0 },
                ..onset
            });
            origin(&draw(&mut motion, &mut tracker, &view, 1.2, false)).activation
        };
        let (sparse, dense) = (run(false), run(true));
        assert!(dense > 0.8, "the old slot is visibly fading: {dense}");
        assert!((sparse - dense).abs() < 1e-5, "sparse {sparse}, dense {dense}");
    }
    /// Pressure routed to thickness swells the lit slot from rest and nothing
    /// else: its ink and light stay full, and once the note is gone the slot
    /// reads full width again though it keeps its last reading.
    #[test]
    fn thickness_reshapes_a_lit_slot_only() {
        use crate::IntensitySource;
        let intensity = crate::IntensitySettings {
            pressure: IntensitySource { thickness: Some(1.0), ..Default::default() },
            ..crate::IntensitySettings::unrouted()
        };
        let view = ViewConfig { fade_shape: 0.0, intensity, ..view() };
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
        let view = ViewConfig { fade_shape: 0.0, mark_delay: 0.0, ..view() };
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
    fn an_interrupted_arrival_and_a_settled_departure_each_keep_the_sweep() {
        let mut view = ViewConfig { fade_shape: 0.0, ..view() };
        view.note_animation.stagger_spread = NoteAnimationConfig::default().stagger_spread;
        let mut tracker = NoteTracker::new();
        let mut motion = NodeMotion::default();
        tracker.handle_event(on(0.0, 60));
        let first = draw(&mut motion, &mut tracker, &view, 0.0, false);
        let order = view.note_animation.delays(&first.octave_layout, 1.0);
        let arrival = draw(&mut motion, &mut tracker, &view, 0.15, false);
        tracker.handle_event(off(0.15, 60));
        let departure = draw(&mut motion, &mut tracker, &view, 0.2, false);
        // Each slice where its own edges have brought it: one the rise has
        // not reached is still out, one the off has reached is on its way
        // back, and one between the two is still arriving. The lit slice
        // takes the rise at once and the off in its turn.
        let (_, lit) = lit_by(&arrival, 60);
        let (a, b) = (origin(&arrival).slice_progress, origin(&departure).slice_progress);
        let mut reached = [false; 3];
        for i in 0..first.octave_layout.span as usize {
            let rise = if i == lit { 0.0 } else { order[i] };
            let (case, holds) = if rise >= 0.2 {
                (0, b[i] == 0.0)
            } else if 0.15 + order[i] <= 0.2 {
                (1, b[i] < a[i])
            } else {
                (2, b[i] > a[i])
            };
            reached[case] = true;
            assert!(holds, "slice {i} (case {case}) went from {} to {}", a[i], b[i]);
        }
        assert_eq!(reached, [true; 3], "the fixture missed a case: {order:?}");
        tracker.handle_event(on(0.21, 60));
        draw(&mut motion, &mut tracker, &view, 1.6, false);
        tracker.handle_event(off(1.7, 60));
        let scene = draw(&mut motion, &mut tracker, &view, 1.8, false);
        let p = origin(&scene).slice_progress;
        assert!(
            p[..scene.octave_layout.span as usize].iter().any(|&x| (x - p[0]).abs() > 1e-4),
            "lost departure order"
        );
    }
    #[test]
    fn audio_presence_stays_settled_and_mark_delay_cancels_on_short_notes() {
        let view = ViewConfig { fade_shape: 0.0, mark_delay: 0.4, ..view() };
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
    fn stagger_spread_spans_starts_and_carries_through_interruptions() {
        let layout = crate::octave_layout(6, 60.0);
        for spread in [0.0, NoteAnimationConfig::default().stagger_spread, 0.9] {
            let mut view = ViewConfig { fade_shape: 0.0, ..view() };
            view.note_animation.stagger_spread = spread;
            let ds = view.note_animation.delays(&layout, 1.0);
            let active = &ds[..layout.span as usize];
            assert_eq!(active.iter().copied().fold(f32::INFINITY, f32::min), 0.0);
            let expected = spread;
            assert!((active.iter().copied().fold(0.0, f32::max) - expected).abs() < 1e-6);
            let mut tracker = NoteTracker::new();
            let mut motion = NodeMotion::default();
            tracker.handle_event(on(0.0, 60));
            draw(&mut motion, &mut tracker, &view, 0.0, false);
            draw(&mut motion, &mut tracker, &view, 0.04, false);
            let before = motion.at(LatticePos::ORIGIN).progress;
            tracker.handle_event(off(0.04, 60));
            let scene = draw(&mut motion, &mut tracker, &view, 0.05, false);
            // The off reaches the seam at once and the rest in their turn: a
            // slice the arrival had not reached yet has not moved. The lit
            // slice arrived at once and takes the off in its turn, so it may
            // still be arriving, unless there is no spread to wait out.
            let (_, lit) = lit_by(&scene, 60);
            let m = motion.at(LatticePos::ORIGIN);
            for i in (0..11).filter(|&i| i != lit || spread == 0.0) {
                let (a, b) = (before[i], m.progress[i]);
                assert!(if a > 0.0 { b < a } else { b == 0.0 }, "slice {i}");
            }
            // A re-press is one more edge through the same sweep, so every
            // slice is whole one duration after it reaches the last.
            tracker.handle_event(on(0.05, 60));
            draw(&mut motion, &mut tracker, &view, 0.06, false);
            draw(&mut motion, &mut tracker, &view, 1.05 + f64::from(spread) + 1e-5, false);
            assert_eq!(motion.at(LatticePos::ORIGIN).progress, [1.0; 11]);
            // A held control edit governs the next departure: the widest
            // delay plus one whole duration, because the spread offsets starts
            // and does not buy its waiting out of each slice's own time.
            let spread = 0.9 - spread;
            view.note_animation.stagger_spread = spread;
            tracker.handle_event(off(2.0, 60));
            draw(&mut motion, &mut tracker, &view, 2.0, false);
            let end = 2.0 + 1.0 + f64::from(spread);
            let late = draw(&mut motion, &mut tracker, &view, end - 0.05, false);
            assert!(origin(&late).slice_progress.iter().any(|&p| p > 0.0), "{spread}: done early");
            draw(&mut motion, &mut tracker, &view, end + 1e-5, false);
            assert_eq!(motion.at(LatticePos::ORIGIN).progress, [0.0; 11]);
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
        let mut view = ViewConfig { fade_shape: 0.0, mark_delay: 0.0, ..view() };
        view.note_animation.stagger_spread = spread;
        let mut tracker = NoteTracker::new();
        let mut motion = NodeMotion::default();
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
            // must NOT wait on any slice's delay. Staggering it too made the
            // whole wheel wait on whichever slot the note happened to take:
            // measured on a late one, that collapsed seven starts spanning
            // 0.9 into all of them landing within 0.08.
            assert!(node.activation > 0.0, "presence waited on a slice");
            for i in 0..span {
                started[i] = started[i].or((node.slice_progress[i] > 0.0).then_some(now));
                done[i] = done[i].or((node.slice_progress[i] >= 1.0).then_some(now));
            }
        }
        let at = |v: Option<f64>, what: &str| v.unwrap_or_else(|| panic!("{what}"));
        let expected = f64::from(spread);
        let whole = |started: [Option<f64>; 11], done: [Option<f64>; 11], way: &str| {
            for i in 0..span {
                let length = at(done[i], "slice never finished") - at(started[i], "never started");
                assert!(
                    (length - 1.0).abs() <= 2.0 * step,
                    "{way}: slice {i} animated for {length}, not one whole duration"
                );
            }
            let begins: Vec<f64> = (0..span).map(|i| at(started[i], "never started")).collect();
            let widest = begins.iter().copied().fold(0.0, f64::max)
                - begins.iter().copied().fold(f64::INFINITY, f64::min);
            assert!(
                (widest - expected).abs() <= 2.0 * step,
                "{way}: starts span {widest}, expected {expected}"
            );
        };
        whole(started, done, "arriving");

        // The same contract on the way out. The presence used to fall from
        // the off and reach 0 one duration later, which zeroes every slice
        // still retracting: the late ones never got their whole duration,
        // and the exit ran one fade where the entrance ran `1 + spread`.
        tracker.handle_event(off(now, 72));
        let off_at = now;
        let (mut started, mut done) = ([None; 11], [None; 11]);
        while now < off_at + f64::from(spread) + 1.5 {
            now += step;
            let scene = draw(&mut motion, &mut tracker, &view, now, false);
            let node = origin(&scene);
            for i in 0..span {
                started[i] = started[i].or((node.slice_progress[i] < 1.0).then_some(now));
                done[i] = done[i].or((node.slice_progress[i] <= 0.0).then_some(now));
            }
        }
        whole(started, done, "departing");
    }
    #[test]
    fn the_lit_slice_arrives_first_and_leaves_in_the_sweep() {
        let step = 0.01f64;
        let spread = 0.9f32;
        let mut view = ViewConfig { fade_shape: 0.0, mark_delay: 0.0, ..view() };
        view.note_animation.stagger_spread = spread;
        let (mut tracker, mut motion) = (NoteTracker::new(), NodeMotion::default());
        // C3: a slice the sweep starts neither first nor last, 0.6 of the 0.9
        // spread. C5 is the seam's, already first.
        tracker.handle_event(on(0.0, 48));
        let first = draw(&mut motion, &mut tracker, &view, 0.0, false);
        let slot = motion.at(LatticePos::ORIGIN).targets.iter().position(|&t| t > 0.0);
        // The slice that slot is drawn as, which is what `slice_progress` is
        // indexed by.
        let base = first.octave_layout.slots(origin(&first).cents).0 as usize;
        let lit = slot.expect("lit slot") - base;
        let span = first.octave_layout.span as usize;
        let order = view.note_animation.delays(&first.octave_layout, 1.0);
        // The fixture reaches the change only if the sweep puts the lit slice
        // somewhere in the middle; at either end it would be there anyway.
        assert!(order[lit] > 0.1 && order[lit] < spread - 0.1, "{order:?}");
        let off_at = 2.5;
        let mut frames = Vec::new();
        let mut now = 0.0;
        while now < 2.0 * f64::from(spread) + 3.0 {
            now += step;
            if (now - off_at).abs() < step / 2.0 {
                tracker.handle_event(off(now, 48));
            }
            let scene = draw(&mut motion, &mut tracker, &view, now, false);
            frames.push((now, origin(&scene).slice_progress));
        }
        let when = |slot: usize, f: fn(f32) -> bool, from| {
            frames.iter().find(|(t, p)| *t >= from && f(p[slot])).map(|(t, _)| *t).unwrap()
        };
        let starts = |slot| when(slot, |p| p > 0.0, 0.0);
        let leaves = |slot| when(slot, |p| p < 1.0, off_at);
        let near = |at: f64, expected: f64| (at - expected).abs() <= 2.0 * step;
        assert!(starts(lit) <= step + 1e-9, "the lit slice waited to arrive");
        for i in (0..span).filter(|&i| i != lit) {
            assert!(near(starts(i), f64::from(order[i])), "unlit slice {i} left the sweep");
        }
        for (i, &delay) in order.iter().enumerate().take(span) {
            let at = leaves(i);
            assert!(near(at, off_at + f64::from(delay)), "slice {i} left at {at}, off the sweep");
        }
    }
    /// The table that drops a node at rest draws, frame for frame, what one
    /// keeping every visible node draws, over a long run of what reaches
    /// every branch where the two could part: chords at random velocities and
    /// pressures, bends into nodes at rest, retriggers at one instant, notes
    /// delivered after the frame that passed them, the view scrolling nodes
    /// lit by held notes out and back in, the audio ring posing nodes between
    /// notes, and a gap long enough to reset the surface.
    #[test]
    fn a_sparse_table_draws_what_a_dense_one_does() {
        use harmonigraph_core::{Expression, NoteEventKind};
        for run in 0i32..2 {
            let mut view = ViewConfig::default();
            view.note_animation.stagger_spread = 0.6;
            view.mark_delay = 0.15;
            view.note_animation.fade_out_delay = 0.2;
            view.intensity.pressure.thickness = Some(0.5);
            let frame = FrameParams { fade_time: 0.35, ..Default::default() };
            let env = view.envelope(&frame);
            let mut seed = 0x9e37_79b9_7f4a_7c15u64 + run as u64;
            let mut unit = move || {
                seed = seed.wrapping_mul(6_364_136_223_846_793_005).wrapping_add(1);
                (seed >> 40) as f64 / f64::from(1u32 << 24)
            };
            let mut tracker = NoteTracker::new();
            let mut dense = NodeMotion { dense: true, ..NodeMotion::default() };
            let mut sparse = NodeMotion::default();
            let (mut now, mut clock) = (0.0f64, 0.0f64);
            let mut held: Vec<u8> = Vec::new();
            let (mut fewest, mut most) = (usize::MAX, 0);
            let (mut fewest_ringing, mut posed_most) = (usize::MAX, 0);
            for i in 0..900i32 {
                let shown = now;
                now += match i {
                    450 => 3.0,
                    _ if unit() < 0.1 => 0.0,
                    _ if unit() < 0.1 => 0.1,
                    _ => 1.0 / 60.0,
                };
                // Delivered in time order, and now and then behind the frame
                // already drawn past it: a late note replays the horizon.
                let mut times: Vec<f64> = (0..(unit() * 4.0) as usize)
                    .map(|_| {
                        let from = if unit() < 0.05 { clock } else { clock.max(shown) };
                        from + (now - from) * unit()
                    })
                    .collect();
                times.sort_by(f64::total_cmp);
                // Each note lights a twelfth of an equal-tempered lattice, so
                // a quarter of the run is silence for that much to come to rest.
                let silent = (i / 60) % 4 == 3;
                for t in times {
                    clock = t;
                    if silent {
                        if let Some(note) = held.pop() {
                            tracker.handle_event(off(t, note));
                        }
                        continue;
                    }
                    let note = 36 + (unit() * 48.0) as u8;
                    let pick = |unit: f64, held: &[u8]| held[(unit * held.len() as f64) as usize];
                    let event = NoteEvent::on(t, SourceId::DIRECT, 0, note, unit() as f32);
                    match unit() {
                        _ if held.is_empty() => {
                            tracker.handle_event(event);
                            held.push(note);
                        }
                        x if x < 0.3 && held.len() < 3 && !held.contains(&note) => {
                            tracker.handle_event(event);
                            held.push(note);
                        }
                        x if x < 0.55 => {
                            let note = held.remove((unit() * held.len() as f64) as usize);
                            tracker.handle_event(off(t, note));
                        }
                        x if x < 0.65 => {
                            let note = pick(unit(), &held);
                            tracker.handle_event(off(t, note));
                            tracker.handle_event(NoteEvent { note, ..event });
                        }
                        x if x < 0.85 => {
                            let note = pick(unit(), &held);
                            let semitones = (unit() * 6.0 - 3.0).round() as f32;
                            let kind = NoteEventKind::Tuning { semitones };
                            tracker.handle_event(NoteEvent { note, kind, ..event });
                        }
                        _ => {
                            let note = pick(unit(), &held);
                            let kind = NoteEventKind::Expression {
                                expression: Expression::Pressure,
                                value: unit() as f32,
                            };
                            tracker.handle_event(NoteEvent { note, kind, ..event });
                        }
                    }
                }
                tracker.prune(now, &env);
                let audio = (i / 70) % 3 == 1;
                let scene = || {
                    let mut scene = derive_scene(
                        &tracker,
                        &Tuning::default(),
                        &view,
                        &view.reach(),
                        &frame,
                        Camera::default(),
                        None,
                    );
                    // A seventh of the lattice out of view at a time, a
                    // different seventh every five frames.
                    let shift = i / 5;
                    scene.nodes.retain(|n| {
                        (n.lattice_pos.threes + 3 * n.lattice_pos.fives + shift).rem_euclid(7) != 0
                    });
                    if audio {
                        scene.spectral.inner = 0.1;
                        scene.spectral.outer = 0.4;
                        scene.spectral.gate = 0.0;
                    }
                    scene
                };
                let mut fade = RingFade::default();
                if audio {
                    fade.advance(&crate::RingGate::new(&scene().spectral), &env, now);
                }
                let visible = scene().nodes.len();
                let mut drawn = Vec::new();
                for motion in [&mut dense, &mut sparse] {
                    let mut scene = scene();
                    motion.step(&mut scene, &tracker, &Tuning::default(), &view, &env, &fade, now);
                    let lights: Vec<_> =
                        scene.nodes.iter().map(|n| motion.owns_light(n.lattice_pos)).collect();
                    drawn.push(format!("{:?} {:?} {lights:?}", scene.nodes, scene.pluses));
                }
                assert!(drawn[0] == drawn[1], "frame {i} at {now} drew differently");
                assert_eq!(dense.nodes.len(), visible);
                fewest = fewest.min(sparse.nodes.len());
                most = most.max(sparse.nodes.len());
                if audio {
                    fewest_ringing = fewest_ringing.min(sparse.nodes.len());
                    posed_most = posed_most.max(sparse.posed.len());
                }
                assert!(sparse.nodes.values().all(|m| !m.posed()), "a pose kept a full entry");
            }
            let reached = sparse.reached;
            eprintln!(
                "{reached:?}, the table between {fewest} and {most} entries, \
                 up to {posed_most} poses"
            );
            assert!(reached.revived > 0 && reached.scrolled_in > 0 && reached.lit_in_view > 0);
            assert!(reached.lit_after_entering > 0);
            assert_eq!(fewest, 0, "the table never emptied");
            // The ring at its floor poses every unlit node in view, and those
            // poses are what a full table of them would be.
            assert_eq!(fewest_ringing, 0, "the table never emptied under the ring");
            assert!(posed_most > 500, "the ring posed too little of the window: {posed_most}");
            assert!(most > 50, "nothing lit enough of the lattice to measure: {most}");
        }
    }
    #[test]
    fn a_slot_a_second_note_lights_mid_arrival_starts_at_once() {
        let mut view = ViewConfig { fade_shape: 0.0, mark_delay: 0.0, ..view() };
        view.note_animation.stagger_spread = 0.9;
        let (mut tracker, mut motion) = (NoteTracker::new(), NodeMotion::default());
        tracker.handle_event(on(0.0, 48));
        let first = draw(&mut motion, &mut tracker, &view, 0.0, false);
        let base = first.octave_layout.slots(origin(&first).cents).0;
        let early = motion.at(LatticePos::ORIGIN).targets;
        // C4 a rolled moment later, on the slot the sweep delays by 0.3:
        // the arrival is already running, so it is never scheduled afresh.
        tracker.handle_event(on(0.05, 60));
        draw(&mut motion, &mut tracker, &view, 0.05, false);
        let order = view.note_animation.delays(&first.octave_layout, 1.0);
        let late = motion.at(LatticePos::ORIGIN).targets;
        let slot = (0..11).find(|&i| late[i] > early[i]).expect("C4's slot");
        // The slice that slot is drawn as, which `delays` and
        // `slice_progress` are indexed by.
        let second = slot - base as usize;
        assert!(order[second] > 0.2, "C4's slot would start first anyway: {order:?}");
        let scene = draw(&mut motion, &mut tracker, &view, 0.1, false);
        assert!(origin(&scene).slice_progress[second] > 0.0, "the later lit slot waited");
    }
    #[test]
    fn tuned_onsets_and_late_observation_use_the_factual_pitch_and_mark_clock() {
        use harmonigraph_core::NoteEventKind;
        let view = ViewConfig { fade_shape: 0.0, mark_delay: 0.4, ..view() };
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
        let view = ViewConfig { fade_shape: 0.0, mark_delay: 0.4, ..view() };
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
