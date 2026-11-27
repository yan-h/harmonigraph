//! The take's canonical records. Serde stays out of the musical core; complete
//! frames are validated before either live replay or full-roll reconstruction.
use harmonigraph_core::canonical::*;
use harmonigraph_core::{Expressions, LatticePos, NoteTracker, SourceId};
use serde::{Deserialize, Serialize};

use crate::NoteRecord;

/// Compact provenance for a normal accepted musical delta. Configuration values
/// are retained separately by the existing configuration records.
#[derive(Clone, Copy, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct AssignmentRecord {
    pub decision: u64,
    pub node: Option<[i32; 3]>,
    pub correction_microcents: i64,
    pub player_tuning: f64,
}
impl From<AssignmentMetadata> for AssignmentRecord {
    fn from(a: AssignmentMetadata) -> Self {
        Self {
            decision: a.decision,
            node: a.node.map(|p| [p.threes, p.fives, p.sevens]),
            correction_microcents: a.correction_microcents,
            player_tuning: a.player_tuning,
        }
    }
}
impl From<AssignmentRecord> for AssignmentMetadata {
    fn from(a: AssignmentRecord) -> Self {
        Self {
            decision: a.decision,
            node: a.node.map(|p| LatticePos::new(p[0], p[1], p[2])),
            correction_microcents: a.correction_microcents,
            player_tuning: a.player_tuning,
        }
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct DeltaRecord {
    pub event: NoteRecord,
    pub sequence: u64,
    pub lifetime: u64,
    pub pitch_microcents: Option<i64>,
    pub assignment: Option<AssignmentRecord>,
}
impl From<NoteDelta> for DeltaRecord {
    fn from(d: NoteDelta) -> Self {
        Self {
            event: d.event.into(),
            sequence: d.sequence,
            lifetime: d.lifetime,
            pitch_microcents: d.pitch_microcents,
            assignment: d.assignment.map(Into::into),
        }
    }
}
/// An unsequenced direct observation: what `NoteDelta::from` makes of a
/// `NoteEvent`, in record form.
impl From<NoteRecord> for DeltaRecord {
    fn from(event: NoteRecord) -> Self {
        Self { event, ..Self::default() }
    }
}
impl From<DeltaRecord> for NoteDelta {
    fn from(d: DeltaRecord) -> Self {
        Self {
            event: d.event.into(),
            sequence: d.sequence,
            lifetime: d.lifetime,
            pitch_microcents: d.pitch_microcents,
            assignment: d.assignment.map(Into::into),
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
    pub actual_onset: f64,
    pub pitch_microcents: i64,
    pub player_tuning: f64,
    pub frozen_offset_microcents: i64,
    pub velocity: f32,
    pub expressions: ExpressionsRecord,
    pub decision: u64,
    pub attack_node: Option<[i32; 3]>,
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
            actual_onset: v.actual_onset,
            pitch_microcents: v.pitch_microcents,
            player_tuning: v.player_tuning,
            frozen_offset_microcents: v.frozen_offset_microcents,
            velocity: v.velocity,
            expressions: v.expressions.into(),
            decision: v.decision,
            attack_node: v.attack_node.map(|p| [p.threes, p.fives, p.sevens]),
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
            actual_onset: v.actual_onset,
            pitch_microcents: v.pitch_microcents,
            player_tuning: v.player_tuning,
            frozen_offset_microcents: v.frozen_offset_microcents,
            velocity: v.velocity,
            expressions: v.expressions.into(),
            decision: v.decision,
            attack_node: v.attack_node.map(|p| LatticePos::new(p[0], p[1], p[2])),
        }
    }
}

/// Mirrors `harmonigraph_core::Expressions`; a take written before voices
/// carried expressions reads every one as neutral.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct ExpressionsRecord {
    pub pressure: f32,
    pub gain: f32,
    pub timbre: f32,
}
impl Default for ExpressionsRecord {
    fn default() -> Self {
        Expressions::NEUTRAL.into()
    }
}
impl From<Expressions> for ExpressionsRecord {
    fn from(e: Expressions) -> Self {
        Self { pressure: e.pressure, gain: e.gain, timbre: e.timbre }
    }
}
impl From<ExpressionsRecord> for Expressions {
    fn from(e: ExpressionsRecord) -> Self {
        Self { pressure: e.pressure, gain: e.gain, timbre: e.timbre }
    }
}

/// What one source was sounding at time `t`, and nothing else.
///
/// It also carried the source's 16 channels of folded controller state and the
/// time its knowledge began. Both existed for the repair protocol #712
/// replaced, nothing on the reading side ever looked at either, and the
/// channel block was 15% of every published frame — see [`SourceBaseline`].
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct BaselineRecord {
    pub source: u64,
    pub t: f64,
    pub output_cut: u64,
    pub shown: bool,
    pub voices: Vec<VoiceRecord>,
}
impl From<&SourceBaseline> for BaselineRecord {
    fn from(b: &SourceBaseline) -> Self {
        Self {
            source: b.source.0,
            t: b.time,
            output_cut: b.output_cut,
            shown: b.shown,
            voices: b.voices().iter().copied().map(Into::into).collect(),
        }
    }
}
impl BaselineRecord {
    pub fn baseline(&self) -> Result<SourceBaseline, InvalidCanonical> {
        if self.voices.len() > 64 {
            return Err(InvalidCanonical);
        }
        let mut voices = [VoiceBaseline::default(); 64];
        for (to, from) in voices.iter_mut().zip(&self.voices) {
            *to = from.into();
        }
        SourceBaseline::new(
            SourceId(self.source),
            self.t,
            self.output_cut,
            self.shown,
            &voices[..self.voices.len()],
        )
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct GapRecord {
    pub t: f64,
    pub through: f64,
    pub first: u64,
    pub last: u64,
}
impl From<PublicationGap> for GapRecord {
    fn from(g: PublicationGap) -> Self {
        Self { t: g.time, through: g.through, first: g.first, last: g.last }
    }
}
impl From<GapRecord> for PublicationGap {
    fn from(g: GapRecord) -> Self {
        Self { time: g.t, through: g.through, first: g.first, last: g.last }
    }
}

/// One ordered note/control list in memory and on replay. Baseline payloads
/// allocate only on this non-RT side of the publication boundary.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub enum CanonicalRecord {
    Delta(DeltaRecord),
    Baseline(Box<BaselineRecord>),
    Gap(GapRecord),
}
impl CanonicalRecord {
    pub fn time(&self) -> f64 {
        match self {
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
            Self::Delta(d) => NoteDelta::from(*d).validate(),
            Self::Baseline(b) => b.baseline().map(|_| ()),
            Self::Gap(g) => PublicationGap::from(*g).validate(),
        }
    }
    pub fn apply(&self, tracker: &mut NoteTracker) -> Result<(), InvalidCanonical> {
        self.validate()?;
        match self {
            Self::Delta(d) => tracker.handle_canonical(CanonicalEvent::Note((*d).into())),
            Self::Baseline(b) => tracker.replace_source(&b.baseline()?),
            Self::Gap(g) => tracker.handle_canonical(CanonicalEvent::Gap((*g).into())),
        }
    }
    /// Whether this record keeps its source's order (see [`CanonicalOrder`]),
    /// which the reader refuses a take for breaking.
    pub(crate) fn ordered(&self, order: &mut CanonicalOrder) -> bool {
        match self {
            Self::Delta(d) => order.note(SourceId(d.event.source), d.sequence),
            Self::Baseline(b) => order.baseline(SourceId(b.source), b.output_cut),
            Self::Gap(_) => true,
        }
    }
    pub fn note(&self) -> Option<NoteRecord> {
        match self {
            Self::Delta(d) => Some(NoteDelta::from(*d).display_event().into()),
            _ => None,
        }
    }
}

/// The take failed: it lost records, lost input, or the plugin was removed
/// mid-take. A failed recording is different from a malformed trailing line.
/// It remains durable even when no ordinary publication slot ever becomes
/// free. It carries nothing; an older marker's range and reason are skipped.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct IncompleteRecord {}

#[cfg(test)]
mod tests {
    use super::*;

    /// A frame read back out of a take is the frame that went in: all 64 voices
    /// a source can hold, each still at the expressions it was sounding with.
    #[test]
    fn a_full_baseline_record_reads_back_as_the_frame_it_came_from() {
        let voices: Vec<_> = (0..64)
            .map(|note| VoiceBaseline {
                note,
                lifetime: u64::from(note) + 1,
                velocity: 0.8,
                pitch_microcents: i64::from(note) * 100_000_000,
                expressions: Expressions { pressure: 0.25, gain: 0.5, timbre: 0.75 },
                ..Default::default()
            })
            .collect();
        let frame = SourceBaseline::new(SourceId::DIRECT, 2.0, 5, true, &voices).unwrap();
        assert_eq!(BaselineRecord::from(&frame).baseline(), Ok(frame));
    }
}
