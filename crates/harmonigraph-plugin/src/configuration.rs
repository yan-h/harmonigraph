//! Audio-owned effective tuning and observed direct pitches. This is the
//! configuration/confirmed-state part of #617, not session aggregation or an
//! accepted performance-output model.
use harmonigraph_core::configuration::timeline::{
    ConfigCommand, ConfigOrigin, ConfigTimeline, ControlBudget, TimelineError,
};
use harmonigraph_core::configuration::{
    ConfigEdit, ConfigMutation, ConfigReducer, ResolvedConfig, TuningModes,
};
use harmonigraph_core::confirmed::{ConfirmedPitches, LearningState};
use harmonigraph_core::{LearnedTuning, SourceId, Tempered, Tuning};
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
    pub frozen: bool,
    pub timeline: ConfigTimeline,
    pub(crate) confirmed: ConfirmedPitches,
    pub direct: crate::performance::direct::Direct,
    learning: LearningState,
    budget: ControlBudget,
    learned: Option<LearnedTuning>,
    pub snapshot: ConfigurationSnapshot,
    boundary: ConfigurationBoundary,
    pub(crate) recording: recording::Recording,
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
            frozen: false,
            timeline,
            recording: recording::Recording::default(),
            confirmed: ConfirmedPitches::default(),
            direct: crate::performance::direct::Direct::default(),
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
        presentation_time: f64,
    ) {
        if self.frozen {
            return;
        }
        self.direct.begin(
            self.recording.clock,
            boundary.steady_time,
            presentation_time,
            f64::from(boundary.sample_rate),
        );
        self.boundary = boundary;
        self.recording.captured_intent = recorder.capture_recording_intent();
        self.recording.block_start = boundary.steady_time;
        self.recording.block_frames = boundary.frames;
        self.budget = ControlBudget::default();
        self.retire_configuration();
    }

    /// Only already-proven input/binding frontiers authorize this reclamation.
    /// Spend the same enclosing grant before new commands can fill the timeline.
    fn retire_configuration(&mut self) {
        while matches!(self.timeline.retire_one(&mut self.budget), Ok(true)) {}
    }

    pub fn bind_input_cohort(
        &mut self,
        clock: harmonigraph_core::canonical::ClockId,
        sample: i64,
    ) -> Result<ResolvedConfig, TimelineError> {
        if self.frozen || clock != self.recording.clock {
            return Err(TimelineError::InvalidFrontier);
        }
        let raw =
            sample.checked_sub(self.recording.hub_offset).ok_or(TimelineError::InvalidFrontier)?;
        if raw >= self.recording.prefix {
            return Err(TimelineError::PendingBoundary);
        }
        self.timeline.begin_cohort(raw)
    }

    pub fn abandon_input_cohort(
        &mut self,
        clock: harmonigraph_core::canonical::ClockId,
        sample: i64,
    ) -> Result<(), TimelineError> {
        if clock != self.recording.clock {
            return Err(TimelineError::InvalidFrontier);
        }
        let raw =
            sample.checked_sub(self.recording.hub_offset).ok_or(TimelineError::InvalidFrontier)?;
        self.timeline.abandon_cohort(raw)
    }

    /// Called only after record() has registered this subblock's segments.
    /// Seeding is a separate consumer of historical configuration, so it must
    /// finish before even the logical lookup frontier can move past its start.
    pub fn finalize_input(
        &mut self,
        clock: harmonigraph_core::canonical::ClockId,
        finalized: i64,
        bindings_copied: i64,
        recorder: &mut harmonigraph_record::Recorder,
    ) -> Result<(), TimelineError> {
        if self.frozen || clock != self.recording.clock {
            return Err(TimelineError::InvalidFrontier);
        }
        let raw = |sample: i64| {
            sample.checked_sub(self.recording.hub_offset).ok_or(TimelineError::InvalidFrontier)
        };
        let finalized = raw(finalized)?;
        let bindings_copied = raw(bindings_copied)?;
        self.recording.finish(recorder, &self.timeline);
        let cap = self.recording.configuration_seed_frontier();
        if finalized > cap || bindings_copied > cap {
            return Err(TimelineError::PendingBoundary);
        }
        self.timeline.advance_frontiers(finalized, bindings_copied)?;
        self.retire_configuration();
        Ok(())
    }
    pub fn reset(&mut self, recorder: &harmonigraph_record::Recorder) {
        self.frozen = false;
        if self.direct.pending().is_some() {
            recorder.fail_configuration();
        }
        self.direct.reset();
        self.confirmed.reset();
        self.learning = LearningState::default();
        self.learned = None;
        self.timeline = ConfigTimeline::new(self.timeline.reducer().clone());
        self.snapshot.status = 0;
        self.recording.reset(recorder);
    }
    pub fn resume_clock(&mut self, offset: i64, discontinuous: bool) {
        if discontinuous {
            self.direct.reset();
        }
        self.direct.reanchor(offset);
        self.confirmed.reset();
        self.learning = LearningState::default();
        self.learned = None;
        self.timeline = ConfigTimeline::new(self.timeline.reducer().clone());
        self.snapshot.status = 0;
        self.frozen = false;
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
        if self.frozen || self.snapshot.status & 2 != 0 || self.budget.remaining() < 2 {
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
        if self.frozen {
            return;
        }
        self.recording.prefix = through;
    }
    pub fn segment(&mut self, start: u32, frames: u32) {
        if self.frozen {
            return;
        }
        self.recording.block_start = self.boundary.steady_time + i64::from(start);
        self.recording.block_frames = frames;
    }
    pub fn recording_intent(&self) -> u64 {
        self.recording.captured_intent
    }
    pub fn direct_timing(&self, offset: u32) -> Option<harmonigraph_core::canonical::EventTiming> {
        let sample = self.recording.block_start.checked_add(i64::from(offset))?;
        Some(harmonigraph_core::canonical::EventTiming {
            clock: self.recording.clock,
            input: sample,
            planned: None,
            sample,
            sample_rate: f64::from(self.boundary.sample_rate),
        })
    }

    pub fn publish_direct(
        &mut self,
        recorder: &mut harmonigraph_record::Recorder,
        observation_time: f64,
    ) {
        let Some(end) =
            self.recording.block_start.checked_add(i64::from(self.recording.block_frames))
        else {
            recorder.fail_configuration();
            return;
        };
        self.publish_direct_history(recorder, observation_time, Some(end));
        self.publish_direct_repair(recorder, observation_time);
    }

    /// A refused Hub has no peer or registry retirement owner. Its joined
    /// wrapper still owes every observed DIRECT delta at its original route.
    pub fn publish_retired_direct(
        &mut self,
        recorder: &mut harmonigraph_record::Recorder,
        observation_time: f64,
    ) {
        self.publish_direct_history(recorder, observation_time, None);
        self.publish_direct_repair(recorder, observation_time);
    }

    fn publish_direct_history(
        &mut self,
        recorder: &mut harmonigraph_record::Recorder,
        observation_time: f64,
        end: Option<i64>,
    ) {
        // The Hub is the sole dispatcher of aggregation resync requests. A
        // later hint must remain pending for its next all-source collection.
        for _ in 0..crate::performance::direct::OUTPUT_WINDOW {
            let Some(delta) = self.direct.pending() else {
                break;
            };
            let Some(timing) = delta.timing else {
                recorder.fail_configuration();
                recorder.publication_lost(observation_time, Default::default());
                self.direct.published();
                continue;
            };
            if end.is_some_and(|end| timing.sample >= end) {
                break;
            }
            let route = match self.recording_route(timing, delta.event.time) {
                Ok(route) => route,
                Err(()) => {
                    recorder.fail_configuration();
                    Default::default()
                }
            };
            if recorder.publish_note(delta, observation_time, route).is_err() {
                self.direct.recovery = true;
            }
            self.direct.published();
        }
    }
    fn publish_direct_repair(
        &mut self,
        recorder: &mut harmonigraph_record::Recorder,
        observation_time: f64,
    ) {
        use harmonigraph_record::publication::PublishError;
        // All available earlier history precedes this complete current-state
        // frame. Old onset metadata carries its original exact clock already;
        // only the baseline's present cut is routed through the current segment.
        if self.direct.pending().is_some() || (!self.direct.lost && !self.direct.recovery) {
            return;
        }
        let offset = self.recording.block_frames.saturating_sub(1);
        let Some(timing) = self.direct_timing(offset) else {
            return;
        };
        let time = observation_time - 1.0 / f64::from(self.boundary.sample_rate);
        let route = match self.recording_route(timing, time) {
            Ok(route) => route,
            Err(()) => {
                // A display-only repair has no new recording history. The old
                // map may correctly have retired after its complete frontier.
                if self.direct.lost {
                    recorder.fail_configuration();
                }
                Default::default()
            }
        };
        if self.direct.lost {
            recorder.publication_lost(time, route);
            self.direct.lost = false;
            self.direct.recovery = true;
        }
        if !self.direct.recovery || recorder.publication_free() < 2 {
            return;
        }
        let Some(id) = self.direct.baseline_id.checked_add(1) else {
            self.fault();
            return;
        };
        let Some(frame) = self.direct.state.baseline(
            SourceId::DIRECT,
            id,
            self.direct.sequence,
            time,
            self.direct.coverage_start,
            true,
        ) else {
            return;
        };
        match recorder.publish_baseline(0, &frame, observation_time, route) {
            Ok(()) => {
                self.direct.baseline_id = id;
                self.direct.recovery = false;
            }
            Err(PublishError::BaselineBusy | PublishError::Lost) => {}
            Err(PublishError::Invalid) => self.fault(),
        }
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
        self.recording.finish(recorder, &self.timeline);
    }
    pub fn record(
        &mut self,
        recorder: &mut harmonigraph_record::Recorder,
        origin: Option<f64>,
        observation_time: f64,
    ) {
        if self.frozen {
            return;
        }
        self.recording.observation_time = observation_time;
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
        if self.frozen {
            return;
        }
        self.direct.observe(event);
    }
    pub fn group_end(&mut self) -> Option<ConfigurationEdit> {
        if self.frozen {
            return None;
        }
        if !self.direct.sync_learning(&mut self.confirmed) {
            self.snapshot.status |= 1;
        }
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
pub(crate) fn inject_recorder(recorder: harmonigraph_record::Recorder) {
    tests::install_recorder(recorder);
}

#[cfg(test)]
mod tests;

#[cfg(test)]
impl Owner {
    pub fn print_test_memory_layout(&self) {
        use std::mem::size_of;
        println!(
            "LEDGER configuration [owner,timeline,confirmed,learning,recording,direct] {:?}",
            [
                size_of::<Self>(),
                size_of::<ConfigTimeline>(),
                size_of::<ConfirmedPitches>(),
                size_of::<LearningState>(),
                size_of::<recording::Recording>(),
                size_of::<crate::performance::direct::Direct>()
            ]
        );
        self.direct.print_test_memory_layout();
    }
}
