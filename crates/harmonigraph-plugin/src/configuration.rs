//! Audio-owned effective tuning and observed direct pitches. This is the
//! configuration/confirmed-state part of #617, not session aggregation or an
//! accepted performance-output model.
use harmonigraph_core::configuration::{
    ConfigEdit, ConfigMutation, ConfigReducer, PolicyConfig, ResolvedConfig, TuningModes,
};
use harmonigraph_core::confirmed::{ConfirmedPitches, LearningState};
use harmonigraph_core::{LearnedTuning, Tempered, Tuning};
use harmonigraph_ui::params::{ConfigurationView, ParamKey};
use nice_plug::plugin::ParamValue;
use nice_plug::prelude::*;
use nice_plug::wrapper::clap::configuration::*;
use serde::{Deserialize, Serialize};

const EDIT: i32 = 1;
const RESTORE: i32 = 2;
const LEARN: i32 = 3;
const RESOLVED: i32 = 4;
pub const MUSICAL_SETTINGS: &str = "musical-settings";

#[derive(Clone, Copy, Debug, Serialize, Deserialize)]
#[serde(default)]
struct MusicalSettings {
    meantone: bool,
    marvel: bool,
    meantone_auto: bool,
    marvel_auto: bool,
    learning: bool,
    adaptive: harmonigraph_take::configuration::PolicyRecord,
}
impl Default for MusicalSettings {
    fn default() -> Self {
        Self {
            meantone: true,
            marvel: true,
            meantone_auto: true,
            marvel_auto: true,
            learning: false,
            adaptive: Default::default(),
        }
    }
}
impl MusicalSettings {
    fn modes(self) -> TuningModes {
        TuningModes {
            tempered: Tempered { syntonic: self.meantone, septimal_kleisma: self.marvel },
            auto: [self.meantone_auto, self.marvel_auto],
            learning: self.learning,
        }
    }
    fn from_modes(modes: TuningModes, policy: PolicyConfig) -> Self {
        Self {
            meantone: modes.tempered.syntonic,
            marvel: modes.tempered.septimal_kleisma,
            meantone_auto: modes.auto[0],
            marvel_auto: modes.auto[1],
            learning: modes.learning,
            adaptive: policy.into(),
        }
    }
}
fn bits(modes: TuningModes) -> i32 {
    i32::from(modes.tempered.syntonic)
        | i32::from(modes.tempered.septimal_kleisma) << 1
        | i32::from(modes.auto[0]) << 2
        | i32::from(modes.auto[1]) << 3
        | i32::from(modes.learning) << 4
}
fn modes(bits: i32) -> TuningModes {
    TuningModes {
        tempered: Tempered { syntonic: bits & 1 != 0, septimal_kleisma: bits & 2 != 0 },
        auto: [bits & 4 != 0, bits & 8 != 0],
        learning: bits & 16 != 0,
    }
}
fn encode_option(value: Option<bool>) -> i32 {
    value.map_or(0, |on| if on { 2 } else { 1 })
}
fn decode_option(value: i32) -> Option<bool> {
    match value {
        1 => Some(false),
        2 => Some(true),
        _ => None,
    }
}

pub fn packet(edit: ConfigEdit) -> ConfigurationEdit {
    let mut payload = [0; PAYLOAD_WORDS];
    payload[0] = EDIT;
    payload[1] = encode_option(edit.tempered[0]);
    payload[2] = encode_option(edit.tempered[1]);
    payload[3] = encode_option(edit.auto[0]);
    payload[4] = encode_option(edit.auto[1]);
    payload[5] = encode_option(edit.learning);
    if let Some(policy) = edit.policy {
        payload[7..15].copy_from_slice(&policy.sanitize().words());
    }
    ConfigurationEdit {
        values: edit.axes.map(|value| value.map(|v| v as f32 / 1_000_000.0)),
        payload,
    }
}
fn tuning(values: [f32; CONFIG_PARAMETERS]) -> Tuning {
    Tuning::from_cents(values[0], values[1], values[2], values[3], values[4])
}
fn axes(tuning: Tuning) -> [i32; CONFIG_PARAMETERS] {
    [tuning.c_offset, tuning.three, tuning.five, tuning.seven, tuning.tolerance]
}
fn payload(resolved: ResolvedConfig) -> [i32; PAYLOAD_WORDS] {
    let mut payload = [0; PAYLOAD_WORDS];
    payload[0] = RESOLVED;
    payload[1] = bits(resolved.modes);
    payload[2..7].copy_from_slice(&axes(resolved.tuning));
    payload[7..15].copy_from_slice(&resolved.policy.words());
    payload
}

pub fn view(snapshot: ConfigurationSnapshot, pending: bool) -> ConfigurationView {
    let mut resolved =
        ConfigReducer::new(tuning(snapshot.raw), modes(snapshot.payload[1])).resolved();
    if snapshot.payload[0] == RESOLVED {
        resolved.tuning = Tuning {
            c_offset: snapshot.payload[2],
            three: snapshot.payload[3],
            five: snapshot.payload[4],
            seven: snapshot.payload[5],
            tolerance: snapshot.payload[6],
        };
        resolved.modes = modes(snapshot.payload[1]);
    }
    if snapshot.payload[7] == 2 {
        resolved.policy = PolicyConfig::from_words(snapshot.payload[7..15].try_into().unwrap());
    }
    resolved.revision = snapshot.revision;
    ConfigurationView { resolved, status: snapshot.status, pending }
}

pub fn resolve_preview(snapshot: &mut ConfigurationSnapshot) {
    snapshot.payload = payload(view(*snapshot, true).resolved);
}

pub fn prepare(state: &PluginState) -> Result<ConfigurationEdit, SubmitError> {
    let settings: MusicalSettings = match state.fields.get(MUSICAL_SETTINGS) {
        Some(json) => serde_json::from_str(json).map_err(|error| {
            nice_error!("Musical settings refused: {error}");
            SubmitError::Invalid
        })?,
        None => MusicalSettings::default(),
    };
    let mut payload = [0; PAYLOAD_WORDS];
    payload[0] = RESTORE;
    payload[1] = bits(settings.modes());
    payload[7..15].copy_from_slice(&PolicyConfig::from(settings.adaptive).words());
    let mut values = [None; CONFIG_PARAMETERS];
    for (i, key) in ParamKey::TUNING.into_iter().enumerate() {
        let value = match state.params.get(key.id()) {
            Some(ParamValue::F32(value)) => *value,
            None => key.default_value(),
            _ => return Err(SubmitError::Invalid),
        };
        if !value.is_finite() {
            return Err(SubmitError::Invalid);
        }
        values[i] = Some(value.clamp(*key.range().start(), *key.range().end()));
    }
    Ok(ConfigurationEdit { values, payload })
}

pub fn save(snapshot: ConfigurationSnapshot, state: &mut PluginState) {
    let settings = MusicalSettings::from_modes(
        modes(snapshot.payload[1]),
        view(snapshot, false).resolved.policy,
    );
    state.fields.insert(
        MUSICAL_SETTINGS.to_owned(),
        serde_json::to_string(&settings).expect("fixed musical settings serialize"),
    );
}

mod recording;

pub struct Owner {
    /// Every edit reduces here the moment it arrives, in arrival order, so the
    /// reducer's own combined-edit/preset/unlock semantics are untouched.
    pub(crate) reducer: ConfigReducer,
    /// What the reducer held at this callback's boundary. One value for every
    /// assignment group the Hub starts inside the block; edits that land during
    /// the block are adopted by the next `begin`.
    block: ResolvedConfig,
    pub(crate) confirmed: ConfirmedPitches,
    learning: LearningState,
    learned: Option<LearnedTuning>,
    pub snapshot: ConfigurationSnapshot,
    boundary: ConfigurationBoundary,
    pub(crate) recording: recording::Recording,
}
impl Owner {
    pub fn new(params: &super::HarmonigraphParams) -> Self {
        let raw = ParamKey::TUNING.map(|key| params.param_for(key).value());
        let reducer = ConfigReducer::new(tuning(raw), MusicalSettings::default().modes());
        let snapshot = ConfigurationSnapshot {
            raw,
            unmodulated: raw,
            normalized: ParamKey::TUNING
                .map(|key| params.param_for(key).unmodulated_normalized_value()),
            payload: payload(reducer.resolved()),
            ..Default::default()
        };

        Self {
            block: reducer.resolved(),
            reducer,
            recording: recording::Recording::default(),
            confirmed: ConfirmedPitches::default(),
            learning: LearningState::default(),
            learned: None,
            snapshot,
            boundary: ConfigurationBoundary {
                steady_time: 0,
                frames: 0,
                sample_rate: 44100.0,
                transport_seconds: None,
                playing: false,
            },
        }
    }
    pub fn begin(
        &mut self,
        boundary: ConfigurationBoundary,
        recorder: &harmonigraph_record::Recorder,
        _presentation_time: f64,
    ) {
        self.boundary = boundary;
        self.recording.captured_intent = recorder.capture_recording_intent();
        self.recording.block_start = boundary.steady_time;
        self.recording.block_frames = boundary.frames;
        // The whole callback's input is applied and observed inside the same
        // `process_configuration` call, so configuration for it is settled here
        // and never holds output publication behind an unconsumed event. Local
        // steady time, like every other frontier the recording compares.
        self.recording.prefix = boundary.steady_time.saturating_add(i64::from(boundary.frames));
    }

    /// THE block boundary. Every command accepted before this callback has
    /// reduced by now; this is where the whole of it becomes effective, and
    /// nothing later in the callback moves the value again.
    pub fn adopt(&mut self) {
        self.block = self.reducer.resolved();
    }

    /// The one configuration for assignment groups started in this block. A
    /// group that spans a boundary keeps the value it was handed here.
    pub fn block_configuration(
        &self,
        clock: harmonigraph_core::canonical::ClockId,
    ) -> Option<ResolvedConfig> {
        (clock == self.recording.clock).then_some(self.block)
    }

    pub fn reset(&mut self, recorder: &harmonigraph_record::Recorder) {
        self.confirmed.reset();
        self.learning = LearningState::default();
        self.learned = None;
        self.snapshot.status = 0;
        self.recording.reset(recorder);
    }
    pub fn fault(&mut self) {
        self.snapshot.status |= 2;
    }
    pub fn apply(
        &mut self,
        command: ConfigurationCommand,
        commit: ConfigurationCommit,
    ) -> Option<ConfigurationSnapshot> {
        if self.snapshot.status & 2 != 0 {
            return None;
        }
        let raw = tuning(commit.raw);
        let mutation = match command.edit.payload[0] {
            RESTORE => ConfigMutation::Restore {
                raw,
                modes: modes(command.edit.payload[1]),
                policy: PolicyConfig::from_words(command.edit.payload[7..15].try_into().unwrap()),
            },
            LEARN => ConfigMutation::LearnResolved { learned: self.learned?, raw },
            _ => ConfigMutation::Edit(ConfigEdit {
                // Full normalized/modulated raw input is coherent here. Unchanged
                // axes carry no musical revision or fresh comma judgement.
                axes: axes(raw).map(Some),
                tempered: [
                    decode_option(command.edit.payload[1]),
                    decode_option(command.edit.payload[2]),
                ],
                auto: [
                    decode_option(command.edit.payload[3]),
                    decode_option(command.edit.payload[4]),
                ],
                learning: decode_option(command.edit.payload[5]),
                policy: (command.edit.payload[7] == 2).then(|| {
                    PolicyConfig::from_words(command.edit.payload[7..15].try_into().unwrap())
                }),
            }),
        };
        // Reduce in arrival order; the value only becomes the block's at the
        // next boundary, so the effective sample is where this callback ends.
        if !self.reducer.apply(mutation) {
            self.fault();
            return None;
        }
        let resolved = self.reducer.resolved();
        if command.edit.payload[0] == LEARN {
            self.learned = None;
        }
        if command.id != 0 {
            self.snapshot.applied_id = command.id;
        }
        self.snapshot.revision = resolved.revision;
        self.snapshot.effective_sample =
            self.boundary.steady_time.saturating_add(i64::from(self.boundary.frames));
        self.snapshot.payload = payload(resolved);
        self.snapshot.raw = commit.raw;
        self.snapshot.unmodulated = commit.unmodulated;
        self.snapshot.normalized = commit.normalized;
        self.snapshot.modulation = commit.modulation;
        Some(self.snapshot)
    }
    pub fn segment(&mut self, start: u32, frames: u32) {
        self.recording.block_start = self.boundary.steady_time + i64::from(start);
        self.recording.block_frames = frames;
    }
    pub fn recording_intent(&self) -> u64 {
        self.recording.captured_intent
    }

    pub fn recording_route(
        &self,
        timing: harmonigraph_core::canonical::EventTiming,
        presentation_time: f64,
    ) -> Result<harmonigraph_record::publication::Route, ()> {
        self.recording.route(timing.clock, timing.sample, presentation_time)
    }

    pub fn finish_recording_publication(
        &mut self,
        recorder: &mut harmonigraph_record::Recorder,
        observation_time: f64,
    ) {
        self.recording.observation_time = observation_time;
        self.recording.finish(recorder);
    }
    pub fn record(
        &mut self,
        recorder: &mut harmonigraph_record::Recorder,
        origin: Option<f64>,
        observation_time: f64,
    ) {
        self.recording.observation_time = observation_time;
        if self.snapshot.status & 2 != 0 && recorder.recording_epoch() != 0 {
            recorder.fail_configuration();
        }
        self.recording.segment(recorder, origin, f64::from(self.boundary.sample_rate), self.block);
    }

    pub fn group_end(&mut self) -> Option<ConfigurationEdit> {
        if self.snapshot.status != 0 {
            return None;
        }
        match self.learning.infer(&self.confirmed, self.reducer.resolved().modes.learning) {
            Ok(Some(learned)) => {
                self.learned = Some(learned);
                let mut payload = [0; PAYLOAD_WORDS];
                payload[0] = LEARN;
                Some(ConfigurationEdit {
                    values: [learned.c_offset, learned.three, learned.five, learned.seven, None],
                    payload,
                })
            }
            Err(_) => {
                self.snapshot.status |= 1;
                None
            }
            Ok(None) => None,
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
