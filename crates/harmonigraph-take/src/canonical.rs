//! Version 4 canonical records. Serde stays out of the musical core; complete
//! frames are validated before either live replay or full-roll reconstruction.
use harmonigraph_core::canonical::*;
use harmonigraph_core::confirmed::PitchProvenance;
use harmonigraph_core::{LatticePos, NoteTracker, SourceId};
use serde::{Deserialize, Serialize};

use crate::{ConfigurationRecord, NoteRecord};

#[derive(Clone, Copy, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct TimingRecord {
    pub runtime_session: u64,
    pub clock_epoch: u64,
    pub input: i64,
    pub planned: Option<i64>,
    pub sample: i64,
    pub sample_rate: f64,
}
impl From<EventTiming> for TimingRecord {
    fn from(t: EventTiming) -> Self {
        Self {
            runtime_session: t.clock.runtime_session,
            clock_epoch: t.clock.epoch,
            input: t.input,
            planned: t.planned,
            sample: t.sample,
            sample_rate: t.sample_rate,
        }
    }
}
impl From<TimingRecord> for EventTiming {
    fn from(t: TimingRecord) -> Self {
        Self {
            clock: ClockId { runtime_session: t.runtime_session, epoch: t.clock_epoch },
            input: t.input,
            planned: t.planned,
            sample: t.sample,
            sample_rate: t.sample_rate,
        }
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum ProvenanceRecord {
    #[default]
    ObservedDirect,
    AcceptedOutput,
}
impl From<PitchProvenance> for ProvenanceRecord {
    fn from(p: PitchProvenance) -> Self {
        match p {
            PitchProvenance::ObservedDirect => Self::ObservedDirect,
            PitchProvenance::AcceptedOutput => Self::AcceptedOutput,
        }
    }
}
impl From<ProvenanceRecord> for PitchProvenance {
    fn from(p: ProvenanceRecord) -> Self {
        match p {
            ProvenanceRecord::ObservedDirect => Self::ObservedDirect,
            ProvenanceRecord::AcceptedOutput => Self::AcceptedOutput,
        }
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct DeltaRecord {
    pub event: NoteRecord,
    pub sequence: u64,
    pub lifetime: u64,
    pub provenance: ProvenanceRecord,
    pub timing: Option<TimingRecord>,
    pub pitch_microcents: Option<i64>,
}
impl From<NoteDelta> for DeltaRecord {
    fn from(d: NoteDelta) -> Self {
        Self {
            event: d.event.into(),
            sequence: d.sequence,
            lifetime: d.lifetime,
            provenance: d.provenance.into(),
            timing: d.timing.map(Into::into),
            pitch_microcents: d.pitch_microcents,
        }
    }
}
impl From<DeltaRecord> for NoteDelta {
    fn from(d: DeltaRecord) -> Self {
        Self {
            event: d.event.into(),
            sequence: d.sequence,
            lifetime: d.lifetime,
            provenance: d.provenance.into(),
            timing: d.timing.map(Into::into),
            pitch_microcents: d.pitch_microcents,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct VoiceRecord {
    pub channel: u8,
    pub note: u8,
    pub port: i16,
    pub host_note_id: i32,
    pub lifetime: u64,
    pub input_onset: f64,
    pub actual_onset: f64,
    pub onset: Option<TimingRecord>,
    pub pitch_microcents: i64,
    pub player_tuning: f64,
    pub frozen_offset_microcents: i64,
    pub velocity: f32,
    pub provenance: ProvenanceRecord,
    pub assignment: Option<ConfigurationRecord>,
    pub attack_node: Option<[i32; 3]>,
    pub partial_output: bool,
    pub release_pending: bool,
}
impl Default for VoiceRecord {
    fn default() -> Self {
        VoiceBaseline::default().into()
    }
}
impl From<VoiceBaseline> for VoiceRecord {
    fn from(v: VoiceBaseline) -> Self {
        Self {
            channel: v.channel,
            note: v.note,
            port: v.port,
            host_note_id: v.host_note_id,
            lifetime: v.lifetime,
            input_onset: v.input_onset,
            actual_onset: v.actual_onset,
            onset: v.onset.map(Into::into),
            pitch_microcents: v.pitch_microcents,
            player_tuning: v.player_tuning,
            frozen_offset_microcents: v.frozen_offset_microcents,
            velocity: v.velocity,
            provenance: v.provenance.into(),
            assignment: v.assignment.map(|a| ConfigurationRecord::new(v.actual_onset, a)),
            attack_node: v.attack_node.map(|p| [p.threes, p.fives, p.sevens]),
            partial_output: v.partial_output,
            release_pending: v.release_pending,
        }
    }
}
impl From<&VoiceRecord> for VoiceBaseline {
    fn from(v: &VoiceRecord) -> Self {
        Self {
            channel: v.channel,
            note: v.note,
            port: v.port,
            host_note_id: v.host_note_id,
            lifetime: v.lifetime,
            input_onset: v.input_onset,
            actual_onset: v.actual_onset,
            onset: v.onset.map(Into::into),
            pitch_microcents: v.pitch_microcents,
            player_tuning: v.player_tuning,
            frozen_offset_microcents: v.frozen_offset_microcents,
            velocity: v.velocity,
            provenance: v.provenance.into(),
            assignment: v.assignment.map(ConfigurationRecord::resolved),
            attack_node: v.attack_node.map(|p| LatticePos::new(p[0], p[1], p[2])),
            partial_output: v.partial_output,
            release_pending: v.release_pending,
        }
    }
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct ChannelRecord {
    pub controllers: Vec<u8>,
    pub controller_valid: [u64; 2],
    pub pitch_bend: Option<u16>,
    pub pressure: Option<u8>,
    pub program: Option<u8>,
}
impl From<ChannelBaseline> for ChannelRecord {
    fn from(c: ChannelBaseline) -> Self {
        Self {
            controllers: c.controllers.to_vec(),
            controller_valid: c.controller_valid,
            pitch_bend: c.pitch_bend,
            pressure: c.pressure,
            program: c.program,
        }
    }
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct BaselineRecord {
    pub source: u64,
    pub id: u64,
    pub t: f64,
    pub coverage_start: f64,
    pub output_cut: u64,
    pub participating: bool,
    pub voices: Vec<VoiceRecord>,
    pub channels: Vec<ChannelRecord>,
}
impl From<&SourceBaseline> for BaselineRecord {
    fn from(b: &SourceBaseline) -> Self {
        Self {
            source: b.source.0,
            id: b.id,
            t: b.time,
            coverage_start: b.coverage_start,
            output_cut: b.output_cut,
            participating: b.participating,
            voices: b.voices().iter().copied().map(Into::into).collect(),
            channels: b.channels.into_iter().map(Into::into).collect(),
        }
    }
}
impl BaselineRecord {
    pub fn baseline(&self) -> Result<SourceBaseline, InvalidCanonical> {
        if self.voices.len() > 64 || self.channels.len() != 16 {
            return Err(InvalidCanonical);
        }
        let mut voices = [VoiceBaseline::default(); 64];
        for (to, from) in voices.iter_mut().zip(&self.voices) {
            *to = from.into();
        }
        let mut channels = [ChannelBaseline::default(); 16];
        for (to, from) in channels.iter_mut().zip(&self.channels) {
            to.controllers =
                from.controllers.as_slice().try_into().map_err(|_| InvalidCanonical)?;
            to.controller_valid = from.controller_valid;
            to.pitch_bend = from.pitch_bend;
            to.pressure = from.pressure;
            to.program = from.program;
        }
        SourceBaseline::new(
            SourceId(self.source),
            self.id,
            self.t,
            self.coverage_start,
            self.output_cut,
            self.participating,
            &voices[..self.voices.len()],
            channels,
        )
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum GapReasonRecord {
    #[default]
    PublicationFull,
    InvalidRecord,
    ProducerLost,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct GapRecord {
    pub source: Option<u64>,
    pub t: f64,
    pub through: f64,
    pub first: u64,
    pub last: u64,
    pub reason: GapReasonRecord,
}
impl From<PublicationGap> for GapRecord {
    fn from(g: PublicationGap) -> Self {
        Self {
            source: g.source.map(|s| s.0),
            t: g.time,
            through: g.through,
            first: g.first,
            last: g.last,
            reason: match g.reason {
                GapReason::PublicationFull => GapReasonRecord::PublicationFull,
                GapReason::InvalidRecord => GapReasonRecord::InvalidRecord,
                GapReason::ProducerLost => GapReasonRecord::ProducerLost,
            },
        }
    }
}
impl From<GapRecord> for PublicationGap {
    fn from(g: GapRecord) -> Self {
        Self {
            source: g.source.map(SourceId),
            time: g.t,
            through: g.through,
            first: g.first,
            last: g.last,
            reason: match g.reason {
                GapReasonRecord::PublicationFull => GapReason::PublicationFull,
                GapReasonRecord::InvalidRecord => GapReason::InvalidRecord,
                GapReasonRecord::ProducerLost => GapReason::ProducerLost,
            },
        }
    }
}

/// One ordered note/control list in memory and on replay. Baseline payloads
/// allocate only on this non-RT side of the publication boundary.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub enum CanonicalRecord {
    Note(NoteRecord),
    Delta(DeltaRecord),
    Baseline(Box<BaselineRecord>),
    Gap(GapRecord),
}
impl CanonicalRecord {
    pub fn time(&self) -> f64 {
        match self {
            Self::Note(n) => n.t,
            Self::Delta(d) => d.event.t,
            Self::Baseline(b) => b.t,
            Self::Gap(g) => g.t,
        }
    }
    pub fn from_event(event: CanonicalEvent<'_>) -> Self {
        match event {
            CanonicalEvent::Note(d) => Self::Delta(d.into()),
            CanonicalEvent::Baseline(b) => Self::Baseline(Box::new(b.into())),
            CanonicalEvent::Gap(g) => Self::Gap(g.into()),
        }
    }
    pub fn validate(&self) -> Result<(), InvalidCanonical> {
        match self {
            Self::Note(n)
                if n.t.is_finite()
                    && n.channel < 16
                    && n.note < 128
                    && !matches!(n.kind, crate::NoteKind::On { velocity } if !velocity.is_finite() || !(0.0..=1.0).contains(&velocity))
                    && !matches!(n.kind, crate::NoteKind::Tuning { semitones } if !semitones.is_finite()) =>
            {
                Ok(())
            }
            Self::Note(_) => Err(InvalidCanonical),
            Self::Delta(d) => NoteDelta::from(*d).validate(),
            Self::Baseline(b) => b.baseline().map(|_| ()),
            Self::Gap(g) => PublicationGap::from(*g).validate(),
        }
    }
    pub fn apply(&self, tracker: &mut NoteTracker) -> Result<bool, InvalidCanonical> {
        self.validate()?;
        match self {
            Self::Note(n) => {
                tracker.handle_event((*n).into());
                Ok(true)
            }
            Self::Delta(d) => tracker.handle_canonical(CanonicalEvent::Note((*d).into())),
            Self::Baseline(b) => tracker.replace_source(&b.baseline()?),
            Self::Gap(g) => tracker.handle_canonical(CanonicalEvent::Gap((*g).into())),
        }
    }
    pub fn translate(&mut self, offset: f64) {
        match self {
            Self::Note(n) => n.t += offset,
            Self::Delta(d) => d.event.t += offset,
            Self::Baseline(b) => {
                b.t += offset;
                b.coverage_start += offset;
                for v in &mut b.voices {
                    v.input_onset += offset;
                    v.actual_onset += offset;
                    if let Some(a) = &mut v.assignment {
                        a.t += offset;
                    }
                }
            }
            Self::Gap(g) => {
                g.t += offset;
                g.through += offset;
            }
        }
    }
    pub fn note(&self) -> Option<NoteRecord> {
        match self {
            Self::Note(n) => Some(*n),
            Self::Delta(d) => Some(NoteDelta::from(*d).display_event().into()),
            _ => None,
        }
    }
    pub fn voiced(&self) -> bool {
        match self {
            Self::Baseline(b) => b.participating && !b.voices.is_empty(),
            _ => self.note().is_some_and(|n| matches!(n.kind, crate::NoteKind::On { .. })),
        }
    }
}

/// An incomplete recording is different from a malformed trailing line. It
/// remains durable even when no ordinary publication slot ever becomes free.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct IncompleteRecord {
    pub first_publication: u64,
    pub last_publication: u64,
    pub reason: GapReasonRecord,
}
