//! The tuning configuration, a plugin-owned setting rather than host
//! parameters (#1756): saved in the plugin's `musical-settings` field, edited
//! through one queue the audio owner drains at each callback's start, and
//! published through one seqlock. Learn over the held pitch classes the Hub in
//! `tuning` reads from its rows applies on the audio thread directly.
use harmonigraph_clap::plugin::PluginState;
use harmonigraph_clap::prelude::*;
use harmonigraph_clap::wrapper::clap::setup::{Prepared, Serviced, Setup};
use harmonigraph_core::configuration::{
    ConfigEdit, ConfigMutation, ConfigReducer, PolicyConfig, ResolvedConfig, TuningModes,
};
use harmonigraph_core::tuning::{LearningState, PitchClass, TooManyClasses};
use harmonigraph_core::{Tempered, Tuning};
use harmonigraph_ui::params::ConfigurationView;
use parking_lot::Mutex;
use serde::{Deserialize, Serialize};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, OnceLock};

pub const MUSICAL_SETTINGS: &str = "musical-settings";
/// Edits the UI may queue ahead of the audio thread.
const COMMANDS: usize = 128;
/// The host parameter ids the axes had until #1756.
const LEGACY_AXIS_IDS: [&str; 5] =
    ["tuning-c-offset", "tuning-three", "tuning-five", "tuning-seven", "tuning-tolerance"];

/// The saved record: the raw axes the reducer holds, its links and Learn,
/// and the adaptive policy.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub(crate) struct MusicalSettings {
    /// Microcents, indexed by `TuningAxis::index`, as entered, before any link
    /// derives one from another.
    axes: [i32; 5],
    meantone: bool,
    marvel: bool,
    learning: bool,
    adaptive: harmonigraph_take::configuration::PolicyRecord,
}
impl Default for MusicalSettings {
    fn default() -> Self {
        let reducer = ConfigReducer::default();
        Self::of(reducer.raw(), reducer.resolved())
    }
}
impl MusicalSettings {
    fn of(raw: Tuning, resolved: ResolvedConfig) -> Self {
        Self {
            axes: raw.axes(),
            meantone: resolved.modes.tempered.syntonic,
            marvel: resolved.modes.tempered.septimal_kleisma,
            learning: resolved.modes.learning,
            adaptive: resolved.policy.into(),
        }
    }
    fn mutation(self) -> ConfigMutation {
        ConfigMutation::Restore {
            raw: Tuning::from_axes(self.axes),
            modes: TuningModes {
                tempered: Tempered { syntonic: self.meantone, septimal_kleisma: self.marvel },
                learning: self.learning,
            },
            policy: PolicyConfig::from(self.adaptive),
        }
    }
    /// The reducer this record restores, which holds every axis in its range.
    fn reducer(self) -> ConfigReducer {
        let mut reducer = ConfigReducer::default();
        reducer.apply(self.mutation());
        reducer
    }
}

/// What the audio owner last applied, as the editor and a save read it: the
/// id of the last edit it took, the reducer's raw axes and resolved
/// configuration, and its status bits, named on [`ConfigurationView`]. The
/// editor adds `QUEUE_FULL` when its last submission found the queue full.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct Record {
    pub applied: u64,
    pub raw: Tuning,
    pub resolved: ResolvedConfig,
    pub status: u32,
}

const WORDS: usize = 14 + PolicyConfig::WORDS;

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
        // Typed to the words left after the header and the two tunings, so a
        // policy that grew is a type error here rather than a word `zip`
        // drops.
        let policy: [i32; WORDS - 14] = record.resolved.policy.words();
        let mut words = [0u64; WORDS];
        words[..4].copy_from_slice(&[
            record.applied,
            record.resolved.revision,
            bits,
            u64::from(record.status),
        ]);
        let signed =
            record.raw.axes().into_iter().chain(record.resolved.tuning.axes()).chain(policy);
        for (cell, word) in words[4..].iter_mut().zip(signed) {
            *cell = word as u32 as u64;
        }
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
            let tuning =
                |first: usize| Tuning::from_axes(std::array::from_fn(|i| signed(first + i)));
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
    /// The id the next command takes; the last one queued has the one before.
    next_id: u64,
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
    /// A UI or Learn edit applied, a lattice-map document edited, or an edit
    /// to what the editor saves in `ui-state` (#1813), since the host last
    /// asked: it marks the project modified.
    modified: AtomicBool,
    /// A lattice-map document edit since the host last asked: the Lattice
    /// Map's label names its maps, so the host reads its text again.
    text: AtomicBool,
    /// The last submission found the queue full.
    rejected: AtomicBool,
    /// Counts committed restores, for the map editor's undo.
    restored: AtomicU64,
    /// The Console lines the editor owes, oldest first: a restore that opened
    /// a project at fresh axes, one the full queue refused, or what a restore
    /// skipped (`saved_fields`).
    notice: Mutex<std::collections::VecDeque<String>>,
    wake: OnceLock<Arc<dyn Fn() + Send + Sync>>,
}

/// The Console line for a project saved while the tuning axes were host
/// parameters, before #1756: those values are not read.
pub(crate) const LEGACY_AXES: &str = "This project saved its tuning axes as host parameters, \
     which are no longer read: it opened at fresh axes. Set the tuning again and save the project.";
/// The Console line for a restore the full edit queue refused.
const RESTORE_REFUSED: &str = "A tuning restore was refused: 128 tuning edits wait for the audio \
     thread. Let the host run the plugin, then load the project again.";

impl Default for Settings {
    fn default() -> Self {
        let (queue, commands) = rtrb::RingBuffer::new(COMMANDS);
        let reducer = ConfigReducer::default();
        Self {
            producer: Mutex::new(Producer { queue, next_id: 1, shadow: None }),
            commands: Mutex::new(Some(commands)),
            published: Published::new(Record {
                applied: 0,
                raw: reducer.raw(),
                resolved: reducer.resolved(),
                status: 0,
            }),
            modified: AtomicBool::new(false),
            text: AtomicBool::new(false),
            rejected: AtomicBool::new(false),
            restored: AtomicU64::new(0),
            notice: Mutex::default(),
            wake: OnceLock::new(),
        }
    }
}

impl Settings {
    /// Queue one UI edit for the audio owner. False when the queue is full.
    pub fn submit(&self, edit: ConfigEdit) -> bool {
        let mut producer = self.producer.lock();
        let id = producer.next_id;
        if producer.queue.push(Command { id, mutation: ConfigMutation::Edit(edit) }).is_err() {
            self.rejected.store(true, Ordering::Release);
            return false;
        }
        self.rejected.store(false, Ordering::Release);
        producer.next_id += 1;
        true
    }

    /// The configuration as the editor shows it and a save stores it, with
    /// the raw axes the save needs.
    fn current(&self) -> (Tuning, ConfigurationView) {
        let mut producer = self.producer.lock();
        let applied = self.published.load();
        let status = applied.status
            | if self.rejected.load(Ordering::Acquire) { ConfigurationView::QUEUE_FULL } else { 0 };
        // The latest edit queued, while the audio owner has not applied it.
        let latest = producer.next_id - 1;
        let unadopted = (latest > applied.applied).then_some(latest);
        if let Some((id, settings)) = producer.shadow {
            if applied.applied < id {
                let reducer = settings.reducer();
                let mut resolved = reducer.resolved();
                resolved.revision = applied.resolved.revision;
                return (reducer.raw(), ConfigurationView { resolved, status, unadopted });
            }
            producer.shadow = None;
        }
        (applied.raw, ConfigurationView { resolved: applied.resolved, status, unadopted })
    }

    pub fn view(&self) -> ConfigurationView {
        self.current().1
    }

    pub fn restored(&self) -> u64 {
        self.restored.load(Ordering::Acquire)
    }

    /// The Console line a restore since the last call left, if any.
    pub fn take_notice(&self) -> Option<String> {
        self.notice.lock().pop_front()
    }

    /// Owe the editor a Console line.
    pub(crate) fn notify(&self, line: impl Into<String>) {
        self.notice.lock().push_back(line.into());
    }

    /// Something the project saves changed: have the host mark it modified.
    /// Audio-thread safe; the wrapper defers the host call out of a callback.
    pub fn changed(&self) {
        self.modified.store(true, Ordering::Release);
        if let Some(wake) = self.wake.get() {
            wake();
        }
    }

    /// The lattice-map document changed: the project is modified and the
    /// Lattice Map's labels may read differently.
    pub fn maps_changed(&self) {
        self.text.store(true, Ordering::Release);
        self.changed();
    }

    fn save(&self, state: &mut PluginState) {
        let (raw, view) = self.current();
        state.fields.insert(
            MUSICAL_SETTINGS.to_owned(),
            serde_json::to_string(&MusicalSettings::of(raw, view.resolved))
                .expect("fixed musical settings serialize"),
        );
    }

    /// Main thread, like the commit that follows it, as is every editor edit
    /// on macOS: nothing can take the slot this saw free in between.
    fn prepare(self: &Arc<Self>, state: &PluginState) -> Result<Restore, &'static str> {
        let value: MusicalSettings = match state.fields.get(MUSICAL_SETTINGS) {
            Some(json) => serde_json::from_str(json).map_err(|error| {
                shell_error!("Musical settings refused: {error}");
                "invalid musical settings"
            })?,
            None => MusicalSettings::default(),
        };
        if self.producer.lock().queue.slots() == 0 {
            self.notify(RESTORE_REFUSED);
            return Err("the tuning edit queue is full");
        }
        // The parameter table skips an id it does not know, so the host
        // values of a project saved before #1756 vanish on their own; only
        // the notice is ours to give.
        let legacy = LEGACY_AXIS_IDS.iter().any(|id| state.params.contains_key(*id));
        Ok(Restore { settings: self.clone(), value, legacy })
    }
}

struct Restore {
    settings: Arc<Settings>,
    value: MusicalSettings,
    legacy: bool,
}
impl Prepared for Restore {
    fn commit(self: Box<Self>) {
        let settings = &self.settings;
        let mut producer = settings.producer.lock();
        let id = producer.next_id;
        let queued = producer.queue.push(Command { id, mutation: self.value.mutation() }).is_ok();
        // Only another thread's edit between prepare and here takes the slot
        // prepare saw free, and on macOS both run on the main thread (#1631).
        debug_assert!(queued, "an editor edit took the slot a restore's prepare saw free");
        if !queued {
            return;
        }
        producer.next_id += 1;
        producer.shadow = Some((id, self.value));
        drop(producer);
        settings.restored.fetch_add(1, Ordering::AcqRel);
        if self.legacy {
            // The log keeps it for a project saved before an editor opens.
            shell_error!("{LEGACY_AXES}");
            settings.notify(LEGACY_AXES);
        }
    }
}

/// The Hub's setup, the one owner of everything Harmonigraph saves beside its
/// host parameters: the editor's and the maps' fields (`saved_fields`), its
/// own instance settings and the musical settings.
pub(crate) struct HubSetup {
    pub instance: crate::tuning::setup::Adapter,
    pub params: Arc<crate::HarmonigraphParams>,
}
/// Prepared restores, committed in order.
struct All([Box<dyn Prepared>; 3]);
impl Prepared for All {
    fn commit(self: Box<Self>) {
        for prepared in self.0 {
            prepared.commit();
        }
    }
}
impl Setup for HubSetup {
    fn track_name_changed(&self, name: Option<&str>) {
        self.instance.track_name_changed(name);
    }
    /// Only the musical settings and the instance settings can refuse; the
    /// other fields are installed first, as nice-plug's derive did before
    /// either committed.
    fn prepare(&self, state: &PluginState) -> Result<Box<dyn Prepared>, &'static str> {
        let musical = self.params.settings.prepare(state)?;
        let instance = self.instance.prepare(state)?;
        let fields = crate::saved_fields::prepare(&self.params, state);
        Ok(Box::new(All([Box::new(fields), instance, Box::new(musical)])))
    }
    fn save(&self, state: &mut PluginState) {
        crate::saved_fields::save(&self.params, state);
        self.instance.save(state);
        self.params.settings.save(state);
    }
    fn install_wakeup(&self, wake: Box<dyn Fn() + Send + Sync>) {
        let wake: Arc<dyn Fn() + Send + Sync> = Arc::from(wake);
        let instance = wake.clone();
        self.instance.install_wakeup(Box::new(move || instance()));
        assert!(self.params.settings.wake.set(wake).is_ok(), "one wakeup");
    }
    fn service(&self) -> Serviced {
        let mut serviced = self.instance.service();
        let settings = &self.params.settings;
        serviced.modified |= settings.modified.swap(false, Ordering::AcqRel);
        serviced.text |= settings.text.swap(false, Ordering::AcqRel);
        serviced
    }
    /// A line about the host, for the editor's Console.
    fn notice(&self, line: &str) {
        self.params.settings.notify(line);
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

    /// The callback's boundary: apply the edits queued before it, in arrival
    /// order, then seed its history entry. Edits arriving later wait for the
    /// next callback; automation the walk observes lands in the history at
    /// its own sample.
    pub fn begin(
        &mut self,
        boundary: &Boundary,
        seed: crate::lattice_maps::Seed,
        recorder: &harmonigraph_record::Recorder,
    ) {
        self.maps.begin(boundary);
        self.recording.captured_intent = recorder.capture_recording_intent();
        self.recording.block_start = boundary.steady_time;
        self.recording.block_frames = boundary.frames;
        for _ in 0..self.commands.slots() {
            let command = self.commands.pop().expect("a counted command");
            self.reducer.apply(command.mutation);
            self.applied = command.id;
            self.edited |= matches!(command.mutation, ConfigMutation::Edit(_));
        }
        self.maps.adopt(self.reducer.resolved(), seed);
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
    /// Host input was lost in this callback. A take armed at it fails here,
    /// whatever the host does next: this callback records nothing (it runs no
    /// `play`), so judging at a later one missed a take disarmed before it, or
    /// a loss the host's reset cleared first, and refused a take whose Stop
    /// had landed before the loss. A take armed later refuses its
    /// configuration until the host's reset clears the loss (#1832).
    pub fn fault(&mut self, recorder: &harmonigraph_record::Recorder) {
        self.status |= ConfigurationView::INPUT_LOST;
        if recorder.capture_recording_intent().is_armed() {
            recorder.fail_configuration();
        }
        self.publish();
    }
    pub fn recording_intent(&self) -> harmonigraph_record::configuration::RecordIntent {
        self.recording.captured_intent
    }

    /// The take's presentation time at this callback's start, on the Hub's
    /// own mapping: what a parameter recorded here is timed at.
    pub fn take_time(&self) -> f64 {
        self.recording.presentation.at(self.recording.block_start)
    }

    pub fn record(&mut self, recorder: &mut harmonigraph_record::Recorder, origin: Option<f64>) {
        // A take armed after lost input, before the host's reset clears it,
        // refuses its configuration. One armed at the loss failed there
        // (`fault`), and a loss while none is armed fails nothing (#1832).
        let armed = self.recording.captured_intent.is_armed();
        if self.status & ConfigurationView::INPUT_LOST != 0 && armed {
            recorder.fail_configuration();
        }
        let start = self.recording.block_start;
        let end = start + i64::from(self.recording.block_frames);
        recorder.record(self.take_time(), origin);
        if recorder.recording() {
            // What the Hub assigned from, at the block's start and at each
            // change inside it. The recorder writes each only when it changed.
            let clock = self.recording.presentation;
            recorder.configuration(clock.at(start), self.in_force(start).config);
            for (sample, state) in self.maps.changes(start, end) {
                recorder.configuration(clock.at(sample), state.config);
            }
        }
        self.recording.registered(end);
    }

    /// Learn, then publish the callback's record. A Retune-enabled source
    /// reserves the lattice axes only while the engine actually retunes. Pass
    /// through learns the axes from unchanged input.
    ///
    /// `classes` is the Hub's `held_classes`, asked only while Learn is armed.
    /// `TOO_MANY_CLASSES` reports this evaluation alone rather than latching,
    /// so it clears as soon as what is held allows Learn again.
    ///
    /// Learn applies at the block's start after the observer has pushed
    /// later samples, so it is not pushed into the history: the next `begin`
    /// carries it.
    pub fn group_end(
        &mut self,
        retuning: bool,
        classes: impl FnOnce(&mut [PitchClass; crate::tuning::HELD_VOICES]) -> usize,
    ) {
        let retuning = retuning
            && self.maps.playback.engine != harmonigraph_core::lattice_map::TuningEngine::Off;
        self.status &= !ConfigurationView::TOO_MANY_CLASSES;
        if self.maps.playback.engine.learns() {
            match self.learning.infer(self.reducer.resolved().modes.learning, classes) {
                Ok(Some(learned)) => {
                    self.reducer.apply(ConfigMutation::LearnResolved { learned, retuning });
                    self.edited = true;
                }
                Err(TooManyClasses) => self.status |= ConfigurationView::TOO_MANY_CLASSES,
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
mod tests;
