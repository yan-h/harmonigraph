//! The tuning configuration, a plugin-owned setting rather than host
//! parameters (#1756), saved in the plugin's `musical-settings` field. The
//! main thread owns it, as it owns the Lattice Map document (#1938): the
//! editor's edits, Learn (`background`) and a restore write it through one
//! door, and audio copies it at each callback's start.
use crate::shared::Shared;
use harmonigraph_clap::plugin::PluginState;
use harmonigraph_clap::prelude::*;
use harmonigraph_clap::wrapper::clap::setup::{Parsed, Serviced, Setup};
use harmonigraph_core::configuration::{
    ConfigEdit, ConfigMutation, ConfigReducer, PolicyConfig, ResolvedConfig, TuningModes,
};
use harmonigraph_core::lattice_map::TuningEngine;
use harmonigraph_core::tuning::LearnedTuning;
use harmonigraph_core::{Tempered, Tuning};
use harmonigraph_ui::params::ConfigurationView;
use parking_lot::Mutex;
use serde::{Deserialize, Serialize};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, OnceLock};

pub const MUSICAL_SETTINGS: &str = "musical-settings";
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
}

/// The tuning document and audio's half of what it publishes.
struct Document {
    reducer: ConfigReducer,
    /// Each write's resolved configuration, published as the write ends, so
    /// publication keeps the writes' order.
    audio: triple_buffer::Input<ResolvedConfig>,
}

/// The configuration's shared half (#1938): the document the main thread
/// owns and the editor edits, whose latest value audio reads at each
/// callback's start; what audio reports back about it; and the host's save
/// and restore.
pub(crate) struct Settings {
    /// The tuning configuration. Written by the editor's edits, Learn and a
    /// restore, never by audio.
    document: Shared<Document>,
    /// What audio reads: the latest configuration a write published, wait-free
    /// and with no lock, which a reader's release could otherwise take while a
    /// writer is parked. Taken once, by the audio owner.
    audio: Mutex<Option<triple_buffer::Output<ResolvedConfig>>>,
    /// The document revision audio last copied, for the pending label.
    adopted: AtomicU64,
    /// Host input was lost, until the host's reset: audio's one status bit,
    /// [`ConfigurationView::INPUT_LOST`].
    input_lost: AtomicBool,
    /// A UI or Learn edit applied, a lattice-map document edited, or an edit
    /// to what the editor saves in `ui-state` (#1813), since the host last
    /// asked: it marks the project modified.
    modified: AtomicBool,
    /// A lattice-map document edit since the host last asked: the Lattice
    /// Map's label names its maps, so the host reads its text again.
    text: AtomicBool,
    /// The Console lines the editor owes, oldest first: a restore that opened
    /// a project at fresh axes, what a restore skipped (`saved_fields`), or a
    /// line about the host.
    notice: Mutex<std::collections::VecDeque<Notice>>,
    wake: OnceLock<Arc<dyn Fn() + Send + Sync>>,
}

/// A Console line the editor owes, by who left it.
#[derive(Clone, Debug, PartialEq)]
pub enum Notice {
    /// A restore lost something: the Console opens on it, as on
    /// the editor door's own losses (Yan, 2026-11-22, in
    /// `docs/intent/editor-shell.md`).
    Restore(String),
    /// A line about the host, which only appends.
    Host(String),
}

/// The Console line for a project saved while the tuning axes were host
/// parameters, before #1756: those values are not read.
pub(crate) const LEGACY_AXES: &str = "This project saved its tuning axes as host parameters, \
     which are no longer read: it opened at fresh axes. Set the tuning again and save the project.";

impl Default for Settings {
    fn default() -> Self {
        let reducer = ConfigReducer::default();
        let (audio, output) = triple_buffer::triple_buffer(&reducer.resolved());
        Self {
            document: Shared::new(Document { reducer, audio }),
            audio: Mutex::new(Some(output)),
            adopted: AtomicU64::new(0),
            input_lost: AtomicBool::new(false),
            modified: AtomicBool::new(false),
            text: AtomicBool::new(false),
            notice: Mutex::default(),
            wake: OnceLock::new(),
        }
    }
}

impl Settings {
    /// Apply one editor edit to the document, marking the project modified
    /// when what it saves changed.
    pub fn submit(&self, edit: ConfigEdit) {
        if self.apply(|_| Some(ConfigMutation::Edit(edit))) == Some(true) {
            self.changed();
        }
    }

    /// One step of Learn through the door, in one write: whether Learn acts
    /// under the engine `engine` reads there, `infer`'s answer given that, and
    /// what it found applied. Nothing can disarm Learn or change the engine
    /// between the gate and the result. Whether what the project saves
    /// changed, for the caller to mark once out of every lock it holds.
    /// Inference costs at most about 0.10 ms at its class cap, under the lock.
    pub fn learn(
        &self,
        engine: impl FnOnce() -> TuningEngine,
        infer: impl FnOnce(bool) -> Option<LearnedTuning>,
    ) -> bool {
        let decide = |reducer: &ConfigReducer| {
            let learned = infer(reducer.resolved().learn_acts(engine()))?;
            Some(ConfigMutation::LearnResolved { learned })
        };
        self.apply(decide) == Some(true)
    }

    /// Apply the mutation `decide` makes of the document, if any: `None`
    /// when it made none, else whether what the project saves changed.
    fn apply(&self, decide: impl FnOnce(&ConfigReducer) -> Option<ConfigMutation>) -> Option<bool> {
        self.write(|reducer| {
            let mutation = decide(reducer)?;
            let before = MusicalSettings::of(reducer.raw(), reducer.resolved());
            reducer.apply(mutation);
            Some(MusicalSettings::of(reducer.raw(), reducer.resolved()) != before)
        })
    }

    fn read<R>(&self, read: impl FnOnce(&ConfigReducer) -> R) -> R {
        self.document.read(|document| read(&document.reducer))
    }

    /// Write the document, then publish its configuration to audio before the
    /// door closes, when the write changed it.
    fn write<R>(&self, write: impl FnOnce(&mut ConfigReducer) -> R) -> R {
        self.document.write(|document| {
            let before = document.reducer.resolved();
            let written = write(&mut document.reducer);
            let after = document.reducer.resolved();
            if after != before {
                document.audio.write(after);
            }
            written
        })
    }

    /// The configuration as the editor shows it and a save stores it.
    pub fn view(&self) -> ConfigurationView {
        let resolved = self.resolved();
        let adopted = self.adopted.load(Ordering::Acquire);
        ConfigurationView {
            resolved,
            board: false,
            status: if self.input_lost.load(Ordering::Acquire) {
                ConfigurationView::INPUT_LOST
            } else {
                0
            },
            unadopted: (resolved.revision != adopted).then_some(resolved.revision),
        }
    }

    /// The document's resolved configuration.
    pub fn resolved(&self) -> ResolvedConfig {
        self.read(ConfigReducer::resolved)
    }

    /// The oldest Console line owed since the last call, if any.
    pub fn next_notice(&self) -> Option<Notice> {
        self.notice.lock().pop_front()
    }

    /// Owe the editor a Console line.
    pub(crate) fn notify(&self, notice: Notice) {
        self.notice.lock().push_back(notice);
    }

    /// Write every line owed into the editor's Console, the one place they
    /// reach it: a restore's opens the Console, a host's only appends.
    pub(crate) fn deliver_notices(&self, ui: &mut harmonigraph_ui::SharedState) {
        while let Some(notice) = self.next_notice() {
            match notice {
                Notice::Restore(line) => ui.report_loss(line),
                Notice::Host(line) => ui.log(line),
            }
        }
    }

    /// Something the project saves changed: have the host mark it modified.
    /// Safe from any thread; the wrapper defers the host call out of a
    /// callback.
    pub fn changed(&self) {
        crate::shared::assert_unheld();
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
        let saved = self.read(|reducer| MusicalSettings::of(reducer.raw(), reducer.resolved()));
        state.fields.insert(
            MUSICAL_SETTINGS.to_owned(),
            serde_json::to_string(&saved).expect("fixed musical settings serialize"),
        );
    }

    /// Settings that do not parse open at the defaults a save that lacks them
    /// opens at, with a Console line pushed onto `notes`, and the rest of the
    /// restore goes ahead (#1927).
    fn read_saved(self: &Arc<Self>, state: &PluginState, notes: &mut Vec<String>) -> Restore {
        let value: MusicalSettings = crate::saved_fields::parse(
            state,
            MUSICAL_SETTINGS,
            "the tuning opened at its defaults",
            notes,
        )
        .unwrap_or_default();
        // The parameter table skips an id it does not know, so the host
        // values of a project saved before #1756 vanish on their own; only
        // the notice is ours to give.
        let legacy = LEGACY_AXIS_IDS.iter().any(|id| state.params.contains_key(*id));
        Restore { settings: self.clone(), value, legacy }
    }
}

/// What [`HubSetup::parse`] read, installed in its order.
struct HubParsed {
    params: Arc<crate::HarmonigraphParams>,
    fields: crate::saved_fields::Fields,
    instance: crate::tuning::setup::Restore,
    musical: Restore,
}
impl Parsed for HubParsed {
    fn install(self: Box<Self>) {
        self.fields.install(&self.params);
        self.instance.install();
        self.musical.install();
    }
}

struct Restore {
    settings: Arc<Settings>,
    value: MusicalSettings,
    legacy: bool,
}
impl Restore {
    /// The document takes the saved value whole, without marking the project
    /// modified: it holds what the host just gave.
    fn install(self) {
        let settings = &self.settings;
        settings.write(|reducer| reducer.apply(self.value.mutation()));
        if self.legacy {
            // The log keeps it for a project saved before an editor opens.
            shell_error!("{LEGACY_AXES}");
            settings.notify(Notice::Restore(LEGACY_AXES.into()));
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
impl Setup for HubSetup {
    fn track_name_changed(&self, name: Option<&str>) {
        self.instance.track_name_changed(name);
    }
    /// Nothing refuses: a field that does not parse costs that field alone.
    /// Every field is read here, before the parameters are restored, so the
    /// Console lines come in one order; the editor's fields install first,
    /// then the instance's and the tuning. The restore is still three writes,
    /// so a callback between them can meet one without the others.
    fn parse(&self, state: &PluginState) -> Box<dyn Parsed> {
        let mut notes = Vec::new();
        let musical = self.params.settings.read_saved(state, &mut notes);
        let instance = self.instance.read(state, &mut notes);
        let fields = crate::saved_fields::read(&self.params, state, notes);
        Box::new(HubParsed { params: self.params.clone(), fields, instance, musical })
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
        self.params.settings.notify(Notice::Host(line.into()));
    }
}

mod recording;

pub struct Owner {
    pub(crate) maps: crate::lattice_maps::AudioMaps,
    /// The configuration this callback read.
    config: ResolvedConfig,
    /// The latest configuration the document published.
    published: triple_buffer::Output<ResolvedConfig>,
    settings: Arc<Settings>,
    pub(crate) recording: recording::Recording,
}
impl Owner {
    pub fn new(params: &super::HarmonigraphParams) -> Self {
        let published = params.settings.audio.lock().take().expect("one audio owner");
        let mut owner = Self {
            maps: crate::lattice_maps::AudioMaps::new(params),
            config: params.settings.resolved(),
            published,
            settings: params.settings.clone(),
            recording: recording::Recording::default(),
        };
        owner.seed();
        owner
    }

    /// Read the latest configuration published: at creation, at activation
    /// and at each callback's start. Wait-free.
    pub fn seed(&mut self) {
        let config = *self.published.read();
        self.config = config;
        self.settings.adopted.store(config.revision, Ordering::Release);
    }

    /// The callback's boundary: read the configuration the document last
    /// published, then seed its history entry. An edit still being written
    /// sounds a callback later; automation the walk observes lands in the
    /// history at its own sample.
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
        self.seed();
        self.maps.adopt(self.config, seed);
    }

    /// The configuration, engine and map in force at `sample`: the history's
    /// entry for it, or, where the history no longer reaches it, the live
    /// state. The Hub assigns from this and a take records it.
    pub fn in_force(&self, sample: i64) -> crate::lattice_maps::AttackState {
        self.maps.at(sample).unwrap_or_else(|| self.maps.live(self.current()))
    }

    /// The configuration this callback copied.
    pub fn current(&self) -> ResolvedConfig {
        self.config
    }

    pub fn reset(&mut self, recorder: &mut harmonigraph_record::Recorder) {
        self.maps.reset();
        self.settings.input_lost.store(false, Ordering::Release);
        self.recording.reset(recorder);
    }
    /// Host input was lost in this callback. A take armed at it fails here,
    /// whatever the host does next: this callback records nothing (it runs no
    /// `play`), so judging at a later one missed a take disarmed before it, or
    /// a loss the host's reset cleared first, and refused a take whose Stop
    /// had landed before the loss. A take armed later refuses its
    /// configuration until the host's reset clears the loss (#1832).
    pub fn fault(&mut self, recorder: &harmonigraph_record::Recorder) {
        self.settings.input_lost.store(true, Ordering::Release);
        if recorder.capture_recording_intent().is_armed() {
            recorder.fail_configuration();
        }
    }
    pub fn recording_intent(&self) -> harmonigraph_record::configuration::RecordIntent {
        self.recording.captured_intent
    }

    /// The take's presentation time at this callback's start, on the
    /// presentation clock's mapping: what a parameter recorded here is timed
    /// at.
    pub fn take_time(&self) -> f64 {
        self.recording.presentation.at(self.recording.block_start)
    }

    pub fn record(&mut self, recorder: &mut harmonigraph_record::Recorder, origin: Option<f64>) {
        // A take armed after lost input, before the host's reset clears it,
        // refuses its configuration. One armed at the loss failed there
        // (`fault`), and a loss while none is armed fails nothing (#1832).
        let armed = self.recording.captured_intent.is_armed();
        if self.settings.input_lost.load(Ordering::Acquire) && armed {
            recorder.fail_configuration();
        }
        let start = self.recording.block_start;
        let end = start + i64::from(self.recording.block_frames);
        recorder.record(self.take_time(), origin);
        if recorder.recording() {
            // What the Hub assigned from, at the block's start and at each
            // change inside it, as the lattice draws it under the engine in
            // force there (`ResolvedConfig::drawn`). The recorder writes each
            // only when it changed.
            let clock = self.recording.presentation;
            let drawn =
                |state: crate::lattice_maps::AttackState| state.config.drawn(state.playback.engine);
            recorder.configuration(clock.at(start), drawn(self.in_force(start)));
            for (sample, state) in self.maps.changes(start, end) {
                recorder.configuration(clock.at(sample), drawn(state));
            }
        }
        self.recording.registered(end);
    }
}

#[cfg(test)]
mod tests;
