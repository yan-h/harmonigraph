//! The tuning configuration, a plugin-owned setting rather than host
//! parameters (#1756): saved in the plugin's `musical-settings` field, edited
//! through one queue the audio owner drains before each callback's adoption,
//! and published through one seqlock. Learn over the held pitch classes the
//! Hub in `tuning` reads from its rows applies on the audio thread directly.
use harmonigraph_core::configuration::{
    ConfigEdit, ConfigMutation, ConfigReducer, PolicyConfig, ResolvedConfig, TuningModes,
};
use harmonigraph_core::tuning::{microcents, Incomplete, LearningState, PitchClass, Unlearnable};
use harmonigraph_core::{Tempered, Tuning};
use harmonigraph_ui::params::{ConfigurationView, ParamKey};
use nice_plug::plugin::PluginState;
use nice_plug::prelude::*;
use nice_plug::wrapper::clap::configuration::*;
use nice_plug::wrapper::clap::setup::{Prepared, Setup};
use parking_lot::Mutex;
use serde::{Deserialize, Serialize};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, OnceLock};

pub const MUSICAL_SETTINGS: &str = "musical-settings";
/// Edits the UI may queue ahead of the audio thread.
const COMMANDS: usize = 128;

/// The saved record: the raw axes the reducer holds, its links and Learn,
/// and the adaptive policy.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub(crate) struct MusicalSettings {
    /// Microcents: C offset, fifth, third, seventh and tolerance, as entered,
    /// before any link derives one from another.
    axes: [i32; 5],
    meantone: bool,
    marvel: bool,
    learning: bool,
    adaptive: harmonigraph_take::configuration::PolicyRecord,
}
impl Default for MusicalSettings {
    fn default() -> Self {
        Self {
            axes: ParamKey::TUNING.map(|key| microcents(key.default_value())),
            meantone: true,
            marvel: true,
            learning: false,
            adaptive: Default::default(),
        }
    }
}
impl MusicalSettings {
    fn of(raw: Tuning, resolved: ResolvedConfig) -> Self {
        Self {
            axes: axes(raw),
            meantone: resolved.modes.tempered.syntonic,
            marvel: resolved.modes.tempered.septimal_kleisma,
            learning: resolved.modes.learning,
            adaptive: resolved.policy.into(),
        }
    }
    fn raw(self) -> Tuning {
        let [c_offset, three, five, seven, tolerance] = self.axes;
        Tuning { c_offset, three, five, seven, tolerance }
    }
    fn modes(self) -> TuningModes {
        TuningModes {
            tempered: Tempered { syntonic: self.meantone, septimal_kleisma: self.marvel },
            learning: self.learning,
        }
    }
    /// What the reducer installs from this record.
    fn restore(self) -> ConfigMutation {
        ConfigMutation::Restore {
            raw: self.raw(),
            modes: self.modes(),
            policy: PolicyConfig::from(self.adaptive),
        }
    }
    /// The axes the sliders can show, so a restored value is the value on
    /// screen. The policy is sanitized where it converts.
    fn sanitize(mut self) -> Self {
        self.axes = std::array::from_fn(|i| clamp_axis(i, self.axes[i]));
        self
    }
}

fn axes(tuning: Tuning) -> [i32; 5] {
    [tuning.c_offset, tuning.three, tuning.five, tuning.seven, tuning.tolerance]
}

/// The microcents `ParamKey::TUNING[index]`'s slider spans.
pub(crate) fn clamp_axis(index: usize, value: i32) -> i32 {
    let range = ParamKey::TUNING[index].range();
    value.clamp(microcents(*range.start()), microcents(*range.end()))
}

/// What the audio owner last applied, as the editor and a save read it: the
/// id of the last edit it took, the reducer's raw axes and resolved
/// configuration, and its status bits (1: Learn heard an incomplete row,
/// 2: input or a revision was lost and the take refuses its configuration,
/// 16: Learn heard more distinct classes than it reads). The editor adds 8
/// when its last submission found the queue full.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct Record {
    pub applied: u64,
    pub raw: Tuning,
    pub resolved: ResolvedConfig,
    pub status: u32,
}

const WORDS: usize = 24;

/// One writer, the audio owner, and readers off audio that retry. Every word
/// is atomic, so a torn read is retried rather than undefined.
struct Published {
    sequence: AtomicU64,
    words: [AtomicU64; WORDS],
}
impl Published {
    fn new(record: Record) -> Self {
        let published =
            Self { sequence: AtomicU64::new(0), words: std::array::from_fn(|_| AtomicU64::new(0)) };
        published.publish(record);
        published
    }
    fn publish(&self, record: Record) {
        let sequence = self.sequence.load(Ordering::Relaxed);
        self.sequence.store(sequence + 1, Ordering::SeqCst);
        let modes = record.resolved.modes;
        let bits = u64::from(modes.tempered.syntonic)
            | u64::from(modes.tempered.septimal_kleisma) << 1
            | u64::from(modes.learning) << 2;
        let words = [record.applied, record.resolved.revision, bits, u64::from(record.status)]
            .into_iter()
            .chain(
                axes(record.raw)
                    .into_iter()
                    .chain(axes(record.resolved.tuning))
                    .chain(record.resolved.policy.words())
                    .map(|word| word as u32 as u64),
            );
        for (cell, word) in self.words.iter().zip(words) {
            cell.store(word, Ordering::SeqCst);
        }
        self.sequence.store(sequence + 2, Ordering::SeqCst);
    }
    fn load(&self) -> Record {
        loop {
            let before = self.sequence.load(Ordering::SeqCst);
            if before & 1 != 0 {
                std::hint::spin_loop();
                continue;
            }
            let words = self.words.each_ref().map(|word| word.load(Ordering::SeqCst));
            if before != self.sequence.load(Ordering::SeqCst) {
                continue;
            }
            let signed = |i: usize| words[i] as u32 as i32;
            let tuning = |first: usize| Tuning {
                c_offset: signed(first),
                three: signed(first + 1),
                five: signed(first + 2),
                seven: signed(first + 3),
                tolerance: signed(first + 4),
            };
            return Record {
                applied: words[0],
                raw: tuning(4),
                resolved: ResolvedConfig {
                    revision: words[1],
                    tuning: tuning(9),
                    modes: TuningModes {
                        tempered: Tempered {
                            syntonic: words[2] & 1 != 0,
                            septimal_kleisma: words[2] & 2 != 0,
                        },
                        learning: words[2] & 4 != 0,
                    },
                    policy: PolicyConfig::from_words(std::array::from_fn(|i| signed(14 + i))),
                },
                status: words[3] as u32,
            };
        }
    }
}

#[derive(Clone, Copy)]
pub(crate) struct Command {
    id: u64,
    mutation: ConfigMutation,
}

struct Producer {
    queue: rtrb::Producer<Command>,
    next_id: u64,
    /// The id of the last command queued.
    accepted: u64,
    /// A prepared restore holds one queue slot until it commits or drops.
    reserved: bool,
    /// A committed restore the audio owner has not applied yet: what the
    /// editor shows and a save stores until it has.
    shadow: Option<(u64, MusicalSettings)>,
}

/// The configuration's shared half: the editor's edits go in, the audio
/// owner's record comes out, and the host's save and restore meet both.
pub(crate) struct Settings {
    producer: Mutex<Producer>,
    /// Taken once, by the audio owner.
    commands: Mutex<Option<rtrb::Consumer<Command>>>,
    published: Published,
    /// A UI or Learn edit applied, or a lattice-map document edited, since the
    /// host last asked: it marks the project dirty.
    dirty: AtomicBool,
    /// The last submission found the queue full.
    rejected: AtomicBool,
    /// Counts committed restores, for the map editor's undo.
    restored: AtomicU64,
    /// A restored project held its axes as host parameters, so it opened at
    /// fresh axes; the editor's Console says so.
    legacy: AtomicBool,
    wake: OnceLock<Arc<dyn Fn() + Send + Sync>>,
}

/// The Console line for a project saved while the tuning axes were host
/// parameters, before #1756: those values are not read.
pub(crate) const LEGACY_AXES: &str = "This project saved its tuning axes as host parameters, \
     which are no longer read: it opened at fresh axes. Set the tuning again and save the project.";

/// What the editor draws from: the raw axes the sliders show, and the
/// resolved configuration.
#[derive(Clone, Copy, Debug)]
pub(crate) struct Visible {
    pub raw: Tuning,
    pub view: ConfigurationView,
}

impl Default for Settings {
    fn default() -> Self {
        let (queue, commands) = rtrb::RingBuffer::new(COMMANDS);
        let fresh = MusicalSettings::default();
        let reducer = ConfigReducer::new(fresh.raw(), fresh.modes());
        Self {
            producer: Mutex::new(Producer {
                queue,
                next_id: 1,
                accepted: 0,
                reserved: false,
                shadow: None,
            }),
            commands: Mutex::new(Some(commands)),
            published: Published::new(Record {
                applied: 0,
                raw: reducer.raw(),
                resolved: reducer.resolved(),
                status: 0,
            }),
            dirty: AtomicBool::new(false),
            rejected: AtomicBool::new(false),
            restored: AtomicU64::new(0),
            legacy: AtomicBool::new(false),
            wake: OnceLock::new(),
        }
    }
}

impl Settings {
    /// Queue one UI edit for the audio owner. False when the queue is full.
    pub fn submit(&self, mut edit: ConfigEdit) -> bool {
        for (i, axis) in edit.axes.iter_mut().enumerate() {
            *axis = axis.map(|value| clamp_axis(i, value));
        }
        let mut producer = self.producer.lock();
        let id = producer.next_id;
        let room = producer.queue.slots() > usize::from(producer.reserved);
        if !room
            || producer.queue.push(Command { id, mutation: ConfigMutation::Edit(edit) }).is_err()
        {
            self.rejected.store(true, Ordering::Release);
            return false;
        }
        self.rejected.store(false, Ordering::Release);
        producer.next_id += 1;
        producer.accepted = id;
        true
    }

    pub fn visible(&self) -> Visible {
        let mut producer = self.producer.lock();
        let applied = self.published.load();
        let status = applied.status | if self.rejected.load(Ordering::Acquire) { 8 } else { 0 };
        if let Some((id, settings)) = producer.shadow {
            if applied.applied < id {
                let mut resolved = ConfigReducer::new(settings.raw(), settings.modes()).resolved();
                resolved.policy = PolicyConfig::from(settings.adaptive);
                resolved.revision = applied.resolved.revision;
                let view = ConfigurationView { resolved, status, pending: true };
                return Visible { raw: settings.raw(), view };
            }
            producer.shadow = None;
        }
        let pending = producer.accepted > applied.applied;
        Visible {
            raw: applied.raw,
            view: ConfigurationView { resolved: applied.resolved, status, pending },
        }
    }

    pub fn restored(&self) -> u64 {
        self.restored.load(Ordering::Acquire)
    }

    /// Whether a restore since the last call opened a project at fresh axes
    /// because it held them as host parameters.
    pub fn take_legacy_notice(&self) -> bool {
        self.legacy.swap(false, Ordering::AcqRel)
    }

    /// Something the project saves changed: have the host mark it dirty.
    /// Audio-thread safe; the wrapper defers the host call out of a callback.
    pub fn changed(&self) {
        self.dirty.store(true, Ordering::Release);
        if let Some(wake) = self.wake.get() {
            wake();
        }
    }

    fn save(&self, state: &mut PluginState) {
        let visible = self.visible();
        let settings = MusicalSettings::of(visible.raw, visible.view.resolved);
        state.fields.insert(
            MUSICAL_SETTINGS.to_owned(),
            serde_json::to_string(&settings).expect("fixed musical settings serialize"),
        );
    }

    fn prepare(self: &Arc<Self>, state: &PluginState) -> Result<Restore, &'static str> {
        let settings: MusicalSettings = match state.fields.get(MUSICAL_SETTINGS) {
            Some(json) => serde_json::from_str(json).map_err(|error| {
                nice_error!("Musical settings refused: {error}");
                "invalid musical settings"
            })?,
            None => MusicalSettings::default(),
        };
        let mut producer = self.producer.lock();
        if producer.reserved || producer.queue.slots() == 0 {
            return Err("the tuning edit queue is full");
        }
        producer.reserved = true;
        // Nice-plug drops a parameter id it does not know, so the host values
        // of a project saved before #1756 vanish on their own; only the
        // notice is ours to give.
        let legacy = ParamKey::TUNING.iter().any(|key| state.params.contains_key(key.id()));
        Ok(Restore { settings: Some(self.clone()), value: settings.sanitize(), legacy })
    }
}

/// A parsed restore holding its queue slot.
struct Restore {
    settings: Option<Arc<Settings>>,
    value: MusicalSettings,
    legacy: bool,
}
impl Prepared for Restore {
    fn commit(mut self: Box<Self>) {
        let settings = self.settings.take().expect("an uncommitted restore");
        let mut producer = settings.producer.lock();
        producer.reserved = false;
        let id = producer.next_id;
        producer
            .queue
            .push(Command { id, mutation: self.value.restore() })
            .unwrap_or_else(|_| unreachable!("the slot prepare reserved"));
        producer.next_id += 1;
        producer.accepted = id;
        producer.shadow = Some((id, self.value));
        settings.restored.fetch_add(1, Ordering::AcqRel);
        if self.legacy {
            settings.legacy.store(true, Ordering::Release);
        }
    }
}
impl Drop for Restore {
    fn drop(&mut self) {
        if let Some(settings) = &self.settings {
            settings.producer.lock().reserved = false;
        }
    }
}

/// The Hub's setup: its own instance settings beside the musical settings.
pub(crate) struct HubSetup {
    pub instance: crate::tuning::setup::Adapter,
    pub settings: Arc<Settings>,
}
struct Both(Box<dyn Prepared>, Box<dyn Prepared>);
impl Prepared for Both {
    fn commit(self: Box<Self>) {
        self.0.commit();
        self.1.commit();
    }
}
impl Setup for HubSetup {
    fn track_name_changed(&self, name: Option<&str>) {
        self.instance.track_name_changed(name);
    }
    fn prepare(&self, state: &PluginState) -> Result<Box<dyn Prepared>, &'static str> {
        let musical = self.settings.prepare(state)?;
        Ok(Box::new(Both(self.instance.prepare(state)?, Box::new(musical))))
    }
    fn save(&self, state: &mut PluginState) {
        self.instance.save(state);
        self.settings.save(state);
    }
    fn install_wakeup(&self, wake: Box<dyn Fn() + Send + Sync>) {
        let wake: Arc<dyn Fn() + Send + Sync> = Arc::from(wake);
        let instance = wake.clone();
        self.instance.install_wakeup(Box::new(move || instance()));
        assert!(self.settings.wake.set(wake).is_ok(), "one wakeup");
    }
    fn service(&self) -> bool {
        let instance = self.instance.service();
        self.settings.dirty.swap(false, Ordering::AcqRel) || instance
    }
}

mod recording;
pub(crate) use recording::Presentation;

pub struct Owner {
    pub(crate) maps: crate::lattice_maps::AudioMaps,
    /// Every edit reduces here, in arrival order, so the reducer's own
    /// combined-edit/preset/unlock semantics are untouched.
    reducer: ConfigReducer,
    learning: LearningState<{ crate::tuning::HELD_VOICES }>,
    settings: Arc<Settings>,
    commands: rtrb::Consumer<Command>,
    /// The id of the last command applied.
    applied: u64,
    status: u32,
    /// A UI or Learn edit applied this callback.
    edited: bool,
    boundary: ConfigurationBoundary,
    pub(crate) recording: recording::Recording,
}
impl Owner {
    pub fn new(params: &super::HarmonigraphParams) -> Self {
        let settings = params.settings.clone();
        let commands = settings.commands.lock().take().expect("one audio owner");
        let record = settings.published.load();
        Self {
            maps: crate::lattice_maps::AudioMaps::new(params),
            reducer: ConfigReducer::new(record.raw, record.resolved.modes),
            recording: recording::Recording::default(),
            learning: LearningState::default(),
            settings,
            commands,
            applied: 0,
            status: 0,
            edited: false,
            boundary: ConfigurationBoundary {
                steady_time: 0,
                frames: 0,
                sample_rate: 44100.0,
                transport_seconds: None,
                playing: false,
            },
        }
    }
    fn publish(&self) {
        self.settings.published.publish(Record {
            applied: self.applied,
            raw: self.reducer.raw(),
            resolved: self.reducer.resolved(),
            status: self.status,
        });
    }
    pub fn begin(
        &mut self,
        boundary: ConfigurationBoundary,
        recorder: &harmonigraph_record::Recorder,
        _presentation_time: f64,
    ) {
        self.boundary = boundary;
        self.maps.begin(boundary);
        self.recording.captured_intent = recorder.capture_recording_intent();
        self.recording.block_start = boundary.steady_time;
        self.recording.block_frames = boundary.frames;
    }

    /// Apply the edits queued before this callback, in arrival order, then
    /// seed the callback's history entry. Edits arriving later wait for the
    /// next callback; automation the walk observes lands in the history at
    /// its own sample.
    pub fn adopt(&mut self) {
        for _ in 0..self.commands.slots() {
            let command = *self.commands.peek().expect("a counted command");
            // Revision exhaustion: the command stays queued.
            if !self.reducer.apply(command.mutation) {
                self.fault();
                break;
            }
            let _ = self.commands.pop();
            self.applied = command.id;
            self.edited |= matches!(command.mutation, ConfigMutation::Edit(_));
        }
        self.maps.adopt(self.reducer.resolved());
    }

    /// The configuration, engine and map in force at `sample`: the history's
    /// entry for it, or, where the history no longer reaches it, the live
    /// state. The Hub assigns from this and a take records it.
    pub fn in_force(&self, sample: i64) -> crate::lattice_maps::AttackState {
        self.maps.at(sample).unwrap_or_else(|| self.maps.live(self.current()))
    }

    /// The configuration every edit reduced so far leaves.
    pub fn current(&self) -> ResolvedConfig {
        self.reducer.resolved()
    }

    pub fn reset(&mut self, recorder: &mut harmonigraph_record::Recorder) {
        self.maps.reset();
        self.learning = LearningState::default();
        self.status = 0;
        self.recording.reset(recorder);
        self.publish();
    }
    pub fn fault(&mut self) {
        self.status |= 2;
        self.publish();
    }
    pub fn segment(&mut self, start: u32, frames: u32) {
        self.recording.block_start = self.boundary.steady_time + i64::from(start);
        self.recording.block_frames = frames;
    }
    pub fn recording_intent(&self) -> harmonigraph_record::configuration::RecordIntent {
        self.recording.captured_intent
    }

    /// The take's presentation time at this sub-block's start, on the Hub's
    /// own mapping: what a parameter recorded here is timed at.
    pub fn take_time(&self) -> f64 {
        self.recording.presentation.at(self.recording.block_start)
    }

    pub fn record(&mut self, recorder: &mut harmonigraph_record::Recorder, origin: Option<f64>) {
        if self.status & 2 != 0 && recorder.recording_epoch() != 0 {
            recorder.fail_configuration();
        }
        let start = self.recording.block_start;
        let end = start + i64::from(self.recording.block_frames);
        recorder.record(self.take_time(), origin);
        if recorder.recording() {
            // What the Hub assigned from, at the segment's start and at each
            // change inside it.
            self.recording.configuration(recorder, start, self.in_force(start).config);
            for (sample, state) in self.maps.changes(start, end) {
                self.recording.configuration(recorder, sample, state.config);
            }
        }
        self.recording.registered(end);
    }

    /// Learn, then publish the callback's record. A Retune-enabled source
    /// reserves the lattice axes only while the engine actually retunes. Pass
    /// through learns the axes from unchanged input.
    ///
    /// `classes` is the Hub's `held_classes`, asked only while Learn is armed.
    /// Status bits 1 (the rows are incomplete) and 16 (more distinct classes
    /// than Learn reads) report this evaluation alone rather than latching, so
    /// each clears as soon as what is held allows Learn again.
    ///
    /// Learn applies at the block's start after the walk has pushed later
    /// samples, so it is not pushed into the history: the next `adopt`
    /// carries it.
    pub fn group_end(
        &mut self,
        retuning: bool,
        classes: impl FnOnce(&mut [PitchClass; crate::tuning::HELD_VOICES]) -> Result<usize, Incomplete>,
    ) {
        let retuning = retuning
            && self.maps.playback.engine != harmonigraph_core::lattice_map::TuningEngine::Off;
        self.status &= !(1 | 16);
        if self.maps.playback.engine.learns() {
            match self.learning.infer(self.reducer.resolved().modes.learning, classes) {
                Ok(Some(learned)) => {
                    // With retuning active only the origin is learned: the
                    // lattice intervals remain the target.
                    let mut raw = axes(self.reducer.raw());
                    let found = [learned.c_offset, learned.three, learned.five, learned.seven];
                    for (i, cents) in found.into_iter().enumerate() {
                        if let Some(cents) = cents.filter(|_| i == 0 || !retuning) {
                            raw[i] = clamp_axis(i, microcents(cents));
                        }
                    }
                    let raw = MusicalSettings { axes: raw, ..Default::default() }.raw();
                    if self.reducer.apply(ConfigMutation::LearnResolved { learned, retuning, raw })
                    {
                        self.edited = true;
                    } else {
                        self.fault();
                    }
                }
                Err(Unlearnable::Incomplete) => self.status |= 1,
                Err(Unlearnable::TooManyClasses) => self.status |= 16,
                Ok(None) => {}
            }
        }
        self.publish();
        if std::mem::take(&mut self.edited) {
            self.settings.changed();
        }
    }
}

#[cfg(test)]
pub(crate) fn injected_recorder() -> Option<harmonigraph_record::Recorder> {
    tests::take_recorder()
}
#[cfg(test)]
pub(crate) fn inject_recorder(recorder: harmonigraph_record::Recorder) {
    tests::install_recorder(recorder);
}

#[cfg(test)]
mod tests;
