//! Audio-owned effective tuning and observed direct pitches. This is the
//! configuration/confirmed-state part of #617, not session aggregation or an
//! accepted performance-output model.
use harmonigraph_core::configuration::timeline::{
    ConfigCommand, ConfigOrigin, ConfigTimeline, ControlBudget, TimelineError,
};
use harmonigraph_core::configuration::{
    ConfigEdit, ConfigMutation, ConfigReducer, ResolvedConfig, TuningModes,
};
use harmonigraph_core::confirmed::{
    ConfirmedPitch, ConfirmedPitches, LearningState, PitchProvenance,
};
use harmonigraph_core::{LearnedTuning, NoteEvent as CoreEvent, SourceId, Tempered, Tuning};
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
}
impl Default for MusicalSettings {
    fn default() -> Self {
        Self {
            meantone: true,
            marvel: true,
            meantone_auto: true,
            marvel_auto: true,
            learning: false,
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
    fn from_modes(modes: TuningModes) -> Self {
        Self {
            meantone: modes.tempered.syntonic,
            marvel: modes.tempered.septimal_kleisma,
            meantone_auto: modes.auto[0],
            marvel_auto: modes.auto[1],
            learning: modes.learning,
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
    let settings = MusicalSettings::from_modes(modes(snapshot.payload[1]));
    state.fields.insert(
        MUSICAL_SETTINGS.to_owned(),
        serde_json::to_string(&settings).expect("fixed musical settings serialize"),
    );
}

mod recording;

pub struct Owner {
    pub timeline: ConfigTimeline,
    confirmed: ConfirmedPitches,
    learning: LearningState,
    budget: ControlBudget,
    learned: Option<LearnedTuning>,
    pub snapshot: ConfigurationSnapshot,
    boundary: ConfigurationBoundary,
    recording: recording::Recording,
}
impl Owner {
    pub fn new(params: &super::HarmonigraphParams) -> Self {
        let raw = ParamKey::TUNING.map(|key| params.param_for(key).value());
        let timeline = ConfigTimeline::new(ConfigReducer::new(
            tuning(raw),
            MusicalSettings::default().modes(),
        ));
        let snapshot = ConfigurationSnapshot {
            raw,
            unmodulated: raw,
            normalized: ParamKey::TUNING
                .map(|key| params.param_for(key).unmodulated_normalized_value()),
            payload: payload(timeline.reducer().resolved()),
            ..Default::default()
        };

        Self {
            timeline,
            recording: recording::Recording::default(),
            confirmed: ConfirmedPitches::default(),
            learning: LearningState::default(),
            budget: ControlBudget::default(),
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
    ) {
        self.boundary = boundary;
        self.recording.captured_intent = recorder.capture_recording_intent();
        self.recording.block_start = boundary.steady_time;
        self.recording.block_frames = boundary.frames;
        self.budget = ControlBudget::default();
    }
    pub fn reset(&mut self, recorder: &harmonigraph_record::Recorder) {
        self.confirmed.reset();
        self.learning = LearningState::default();
        self.learned = None;
        self.timeline = ConfigTimeline::new(self.timeline.reducer().clone());
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
        recorder: &harmonigraph_record::Recorder,
    ) -> Option<ConfigurationSnapshot> {
        if self.snapshot.status & 2 != 0 || self.budget.remaining() < 2 {
            return None;
        }
        let previous = self.timeline.reducer().resolved();
        let raw = tuning(commit.raw);
        let mutation = match command.edit.payload[0] {
            RESTORE => ConfigMutation::Restore { raw, modes: modes(command.edit.payload[1]) },
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
            }),
        };
        let origin = match command.origin {
            ConfigurationOrigin::Ui => ConfigOrigin::Ui,
            ConfigurationOrigin::Restore => ConfigOrigin::Restore,
            ConfigurationOrigin::Automation => ConfigOrigin::Automation,
            ConfigurationOrigin::Flush => ConfigOrigin::Flush,
            ConfigurationOrigin::Learning => ConfigOrigin::Learning,
        };
        let effective = match self.timeline.effective_at(commit.sample) {
            Ok(sample) => sample,
            Err(_) => {
                self.fault();
                return None;
            }
        };
        match self.timeline.insert(
            ConfigCommand { command_id: command.id, origin, mutation },
            commit.sample,
            effective,
            &mut self.budget,
        ) {
            Ok(_) => {}
            Err(TimelineError::StorageFull) => {
                // UI/restore remain in producer storage. Required automation stays
                // owned in the input pool, with an explicit configuration fault.
                if matches!(
                    command.origin,
                    ConfigurationOrigin::Automation
                        | ConfigurationOrigin::Flush
                        | ConfigurationOrigin::Learning
                ) {
                    self.timeline.required_storage_fault();
                    self.fault();
                }
                return None;
            }
            Err(_) => {
                self.fault();
                return None;
            }
        }
        let marker = match self.timeline.apply_next(&mut self.budget) {
            Ok(Some(marker)) => marker,
            _ => {
                self.fault();
                return None;
            }
        };
        let resolved = marker.resolved?;
        if resolved != previous {
            self.recording.change(marker.effective_sample, resolved, recorder);
        }
        if command.edit.payload[0] == LEARN {
            self.learned = None;
        }
        if command.id != 0 {
            self.snapshot.applied_id = command.id;
        }
        self.snapshot.revision = resolved.revision;
        self.snapshot.effective_sample = marker.effective_sample;
        self.snapshot.payload = payload(resolved);
        self.snapshot.raw = commit.raw;
        self.snapshot.unmodulated = commit.unmodulated;
        self.snapshot.normalized = commit.normalized;
        self.snapshot.modulation = commit.modulation;
        Some(self.snapshot)
    }
    pub fn prefix(&mut self, through: i64) {
        self.recording.prefix = through;
    }
    pub fn segment(&mut self, start: u32, frames: u32) {
        self.recording.block_start = self.boundary.steady_time + i64::from(start);
        self.recording.block_frames = frames;
    }
    pub fn recording_intent(&self) -> u64 {
        self.recording.captured_intent
    }
    pub fn record(&mut self, recorder: &mut harmonigraph_record::Recorder, origin: Option<f64>) {
        if self.snapshot.status & 2 != 0 && recorder.recording_epoch() != 0 {
            recorder.fail_configuration();
        }
        self.recording.segment(
            recorder,
            &self.timeline,
            origin,
            f64::from(self.boundary.sample_rate),
        );
    }

    pub fn observe(&mut self, event: OwnedInput) {
        let Some(sample) = event.sample else {
            self.fault();
            return;
        };
        let time = sample as f64 / f64::from(self.boundary.sample_rate);
        match event.value {
            InputValue::Note { kind: 0, port: 0, channel, key, note_id, .. }
                if (0..16).contains(&channel) && (0..128).contains(&key) =>
            {
                let row = ConfirmedPitch {
                    key: harmonigraph_core::VoiceKey {
                        source: SourceId::DIRECT,
                        channel: channel as u8,
                        note: key as u8,
                    },
                    lifetime: None,
                    host_note_id: (note_id >= 0).then_some(note_id),
                    pitch_microcents: i64::from(key) * 100_000_000,
                    onset_sample: sample,
                    provenance: PitchProvenance::ObservedDirect,
                };
                if self.confirmed.on(row).is_err() {
                    self.snapshot.status |= 1;
                }
            }
            InputValue::Note { kind: 1 | 2, port: 0 | -1, channel, key, note_id, .. } => {
                let mut keys = [None; 64];
                for (i, row) in self
                    .confirmed
                    .rows()
                    .filter(|row| {
                        (note_id == -1 || row.host_note_id == Some(note_id))
                            && (channel == -1 || channel == i16::from(row.key.channel))
                            && (key == -1 || key == i16::from(row.key.note))
                    })
                    .enumerate()
                {
                    keys[i] = Some(row.key);
                }
                for key in keys.into_iter().flatten() {
                    self.confirmed.release(key, None);
                }
            }
            InputValue::Expression {
                expression: 2,
                port: 0 | -1,
                channel,
                key,
                value,
                note_id,
                ..
            } => {
                if !value.is_finite() {
                    self.fault();
                    return;
                }
                let mut keys = [None; 64];
                for (i, row) in self
                    .confirmed
                    .rows()
                    .filter(|row| {
                        (note_id == -1 || row.host_note_id == Some(note_id))
                            && (channel == -1 || channel == i16::from(row.key.channel))
                            && (key == -1 || key == i16::from(row.key.note))
                    })
                    .enumerate()
                {
                    keys[i] = Some(row.key);
                }
                for key in keys.into_iter().flatten() {
                    self.confirmed.pitch(
                        key,
                        None,
                        ((f64::from(key.note) + value) * 100_000_000.0).round() as i64,
                    );
                }
            }
            InputValue::Midi { port: 0, data, .. } => {
                let channel = data[0] & 15;
                let kind = data[0] & 0xf0;
                if kind == 0x90 && data[2] != 0 {
                    if self
                        .confirmed
                        .observe_direct(
                            CoreEvent::on(
                                time,
                                SourceId::DIRECT,
                                channel,
                                data[1],
                                f32::from(data[2]) / 127.0,
                            ),
                            sample,
                        )
                        .is_err()
                    {
                        self.snapshot.status |= 1;
                    }
                } else if kind == 0x80 || kind == 0x90 {
                    self.confirmed.release(
                        harmonigraph_core::VoiceKey {
                            source: SourceId::DIRECT,
                            channel,
                            note: data[1],
                        },
                        None,
                    );
                }
            }
            _ => {}
        }
    }
    pub fn group_end(&mut self) -> Option<ConfigurationEdit> {
        if self.snapshot.status != 0 {
            return None;
        }
        match self
            .learning
            .infer(&self.confirmed, self.timeline.reducer().resolved().modes.learning)
        {
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
mod tests;
