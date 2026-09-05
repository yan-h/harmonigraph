//! The canonical display/take boundary: observed direct input, accepted output,
//! complete source replacement and explicit missing history. Runtime lease and
//! credit validation belongs to the audio owner before this publication boundary.
use crate::configuration::ResolvedConfig;
use crate::confirmed::{ConfirmedPitch, PitchProvenance, HELD_PER_SOURCE};
use crate::{LatticePos, NoteEvent, NoteEventKind, SourceId, Time, VoiceKey};

/// Clock provenance only, never authorization for replayed data.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct ClockId {
    pub runtime_session: u64,
    pub epoch: u64,
}

/// Exact sample provenance survives presentation-clock mapping and take routing.
/// `sample` is actual accepted output for AcceptedOutput, observed input otherwise.
/// Aggregation has no assignment and leaves `planned` absent.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct EventTiming {
    pub clock: ClockId,
    pub input: i64,
    pub planned: Option<i64>,
    pub sample: i64,
    pub sample_rate: f64,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct NoteDelta {
    pub event: NoteEvent,
    /// Zero means an unsequenced direct/standalone observation. Accepted source
    /// output has a nonzero source-monotonic sequence and lifetime.
    pub sequence: u64,
    pub lifetime: u64,
    pub provenance: PitchProvenance,
    pub timing: Option<EventTiming>,
    /// Exact current accepted pitch, when this delta establishes one. The f32
    /// in NoteEvent remains the presentation adapter, never pitch authority.
    pub pitch_microcents: Option<i64>,
}

impl From<NoteEvent> for NoteDelta {
    fn from(event: NoteEvent) -> Self {
        Self {
            event,
            sequence: 0,
            lifetime: 0,
            provenance: PitchProvenance::ObservedDirect,
            timing: None,
            pitch_microcents: None,
        }
    }
}

impl NoteDelta {
    pub fn validate(&self) -> Result<(), InvalidCanonical> {
        if !self.event.time.is_finite()
            || self.event.channel >= 16
            || self.event.note >= 128
            || matches!(self.event.kind, NoteEventKind::On { velocity } if !velocity.is_finite() || !(0.0..=1.0).contains(&velocity))
            || matches!(self.event.kind, NoteEventKind::Tuning { semitones } if !semitones.is_finite())
            || (self.provenance == PitchProvenance::ObservedDirect
                && self.event.source != SourceId::DIRECT)
            || (self.provenance == PitchProvenance::AcceptedOutput
                && (self.sequence == 0 || self.lifetime == 0 || self.timing.is_none()))
            || self.timing.is_some_and(|t| !t.valid())
        {
            return Err(InvalidCanonical);
        }
        Ok(())
    }

    pub fn display_event(self) -> NoteEvent {
        let mut event = self.event;
        if let (NoteEventKind::Tuning { .. }, Some(pitch)) = (event.kind, self.pitch_microcents) {
            event.kind = NoteEventKind::Tuning {
                semitones: (pitch as f64 / 100_000_000.0 - f64::from(event.note)) as f32,
            };
        }
        event
    }
}

impl EventTiming {
    fn valid(self) -> bool {
        self.sample_rate.is_finite() && self.sample_rate > 0.0
    }
}

/// Saved factual state, not a new emitted attack. Optional assignment metadata
/// is absent in the ordinary aggregation milestone.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct VoiceBaseline {
    pub channel: u8,
    pub note: u8,
    pub port: i16,
    pub host_note_id: i32,
    pub lifetime: u64,
    pub input_onset: Time,
    pub actual_onset: Time,
    pub onset: Option<EventTiming>,
    pub pitch_microcents: i64,
    pub player_tuning: f64,
    pub frozen_offset_microcents: i64,
    pub velocity: f32,
    pub provenance: PitchProvenance,
    pub assignment: Option<ResolvedConfig>,
    pub attack_node: Option<LatticePos>,
    /// An accepted onset with unestablished intended tuning remains factual.
    pub partial_output: bool,
    pub release_pending: bool,
}

impl Default for VoiceBaseline {
    fn default() -> Self {
        Self {
            channel: 0,
            note: 0,
            port: 0,
            host_note_id: -1,
            lifetime: 0,
            input_onset: 0.0,
            actual_onset: 0.0,
            onset: None,
            pitch_microcents: 0,
            player_tuning: 0.0,
            frozen_offset_microcents: 0,
            velocity: 0.0,
            provenance: PitchProvenance::ObservedDirect,
            assignment: None,
            attack_node: None,
            partial_output: false,
            release_pending: false,
        }
    }
}

impl VoiceBaseline {
    pub fn key(&self, source: SourceId) -> VoiceKey {
        VoiceKey { source, channel: self.channel, note: self.note }
    }

    pub fn pitch(&self) -> f32 {
        (self.pitch_microcents as f64 / 100_000_000.0) as f32
    }

    pub fn confirmed(&self, source: SourceId) -> ConfirmedPitch {
        ConfirmedPitch {
            key: self.key(source),
            lifetime: (self.lifetime != 0).then_some(self.lifetime),
            host_note_id: (self.host_note_id >= 0).then_some(self.host_note_id),
            pitch_microcents: self.pitch_microcents,
            onset_sample: self.onset.map_or(0, |onset| onset.sample),
            provenance: self.provenance,
        }
    }
}

/// Latest accepted ordinary MIDI channel values. Validity accompanies each
/// value: an untouched control is not fabricated neutral state.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ChannelBaseline {
    pub controllers: [u8; 128],
    pub controller_valid: [u64; 2],
    pub pitch_bend: Option<u16>,
    pub pressure: Option<u8>,
    pub program: Option<u8>,
}

impl Default for ChannelBaseline {
    fn default() -> Self {
        Self {
            controllers: [0; 128],
            controller_valid: [0; 2],
            pitch_bend: None,
            pressure: None,
            program: None,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct InvalidCanonical;

/// One complete immutable source frame. A transport carries a handle to this
/// separately allocated payload; it must not inline 64 voices in ordinary cells.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct SourceBaseline {
    pub source: SourceId,
    pub id: u64,
    pub time: Time,
    pub coverage_start: Time,
    pub output_cut: u64,
    pub participating: bool,
    pub channels: [ChannelBaseline; 16],
    count: usize,
    voices: [VoiceBaseline; HELD_PER_SOURCE],
}

impl SourceBaseline {
    // All envelope fields and both complete payloads must enter validation
    // together; no partially initialized baseline is publicly constructible.
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        source: SourceId,
        id: u64,
        time: Time,
        coverage_start: Time,
        output_cut: u64,
        participating: bool,
        voices: &[VoiceBaseline],
        channels: [ChannelBaseline; 16],
    ) -> Result<Self, InvalidCanonical> {
        if voices.len() > HELD_PER_SOURCE {
            return Err(InvalidCanonical);
        }
        let mut result = Self {
            source,
            id,
            time,
            coverage_start,
            output_cut,
            participating,
            channels,
            count: voices.len(),
            voices: [VoiceBaseline::default(); HELD_PER_SOURCE],
        };
        result.voices[..voices.len()].copy_from_slice(voices);
        result.validate()?;
        Ok(result)
    }

    pub fn voices(&self) -> &[VoiceBaseline] {
        &self.voices[..self.count]
    }

    pub fn validate(&self) -> Result<(), InvalidCanonical> {
        if self.id == 0
            || !self.time.is_finite()
            || !self.coverage_start.is_finite()
            || self.coverage_start > self.time
            || self.count > HELD_PER_SOURCE
        {
            return Err(InvalidCanonical);
        }
        for (i, voice) in self.voices().iter().enumerate() {
            if voice.channel >= 16
                || voice.note >= 128
                || voice.port != 0
                || voice.host_note_id < -1
                || !voice.input_onset.is_finite()
                || !voice.actual_onset.is_finite()
                || voice.actual_onset > self.time
                || !voice.player_tuning.is_finite()
                || !voice.velocity.is_finite()
                || !(0.0..=1.0).contains(&voice.velocity)
                || voice.onset.is_some_and(|t| !t.valid())
                || (voice.provenance == PitchProvenance::ObservedDirect
                    && self.source != SourceId::DIRECT)
                || (voice.provenance == PitchProvenance::AcceptedOutput
                    && (voice.lifetime == 0 || voice.onset.is_none()))
                || self.voices()[..i].iter().any(|old| {
                    old.key(self.source) == voice.key(self.source)
                        || (voice.lifetime != 0 && old.lifetime == voice.lifetime)
                })
            {
                return Err(InvalidCanonical);
            }
        }
        if self.channels.iter().any(|channel| {
            channel.controllers.iter().any(|&v| v >= 128)
                || channel.pitch_bend.is_some_and(|v| v >= 16384)
                || channel.pressure.is_some_and(|v| v >= 128)
                || channel.program.is_some_and(|v| v >= 128)
        }) {
            return Err(InvalidCanonical);
        }
        Ok(())
    }

    /// Map all presentation times with the SAME translation. Exact sample
    /// provenance and assignment values are deliberately unchanged.
    pub fn translate(&mut self, offset: f64) {
        self.time += offset;
        self.coverage_start += offset;
        for voice in &mut self.voices[..self.count] {
            voice.input_onset += offset;
            voice.actual_onset += offset;
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum GapReason {
    PublicationFull,
    InvalidRecord,
    ProducerLost,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct PublicationGap {
    /// None affects the whole canonical stream; Some scopes a known source.
    pub source: Option<SourceId>,
    pub time: Time,
    pub through: Time,
    /// Inclusive publication sequence range, distinct from a source output cut.
    pub first: u64,
    pub last: u64,
    pub reason: GapReason,
}

impl PublicationGap {
    pub fn validate(&self) -> Result<(), InvalidCanonical> {
        if !self.time.is_finite()
            || !self.through.is_finite()
            || self.through < self.time
            || self.first == 0
            || self.last < self.first
        {
            Err(InvalidCanonical)
        } else {
            Ok(())
        }
    }
}

/// Borrowed fanout view. Baseline storage stays owned until every consumer has
/// copied/applied the complete frame. No consumer fabricates attacks to recover.
#[derive(Clone, Copy, Debug)]
pub enum CanonicalEvent<'a> {
    Note(NoteDelta),
    Baseline(&'a SourceBaseline),
    Gap(PublicationGap),
}

impl CanonicalEvent<'_> {
    pub fn time(self) -> Time {
        match self {
            Self::Note(delta) => delta.event.time,
            Self::Baseline(frame) => frame.time,
            Self::Gap(gap) => gap.time,
        }
    }
}

const _: () = assert!(std::mem::size_of::<VoiceBaseline>() <= 256);
const _: () = assert!(std::mem::align_of::<VoiceBaseline>() <= 8);

#[cfg(test)]
mod tests {
    use super::*;
    use crate::NoteTracker;

    fn voice(note: u8) -> VoiceBaseline {
        VoiceBaseline {
            note,
            lifetime: u64::from(note) + 1,
            input_onset: 0.0,
            actual_onset: 1.0,
            onset: Some(EventTiming {
                clock: ClockId::default(),
                input: 0,
                planned: None,
                sample: 48000,
                sample_rate: 48000.0,
            }),
            pitch_microcents: i64::from(note) * 100_000_000,
            velocity: 0.8,
            provenance: PitchProvenance::AcceptedOutput,
            ..Default::default()
        }
    }

    fn frame(voices: &[VoiceBaseline]) -> Result<SourceBaseline, InvalidCanonical> {
        SourceBaseline::new(
            SourceId(1),
            1,
            2.0,
            1.0,
            64,
            true,
            voices,
            [ChannelBaseline::default(); 16],
        )
    }

    #[test]
    fn complete_64_voice_replacement_validates_before_touching_any_source() {
        let voices: Vec<_> = (0..64).map(voice).collect();
        let mut tracker = NoteTracker::new();
        tracker.handle_event(NoteEvent::on(0.0, SourceId(2), 0, 0, 0.6));
        let baseline = frame(&voices).unwrap();
        assert_eq!(tracker.replace_source(&baseline), Ok(true));
        assert_eq!(tracker.held_count(), 65);
        assert_eq!(tracker.replace_source(&baseline), Ok(false));
        let mut invalid = voices.clone();
        invalid.push(voice(64));
        assert_eq!(frame(&invalid), Err(InvalidCanonical));
        invalid.pop();
        invalid[63] = invalid[0];
        assert_eq!(frame(&invalid), Err(InvalidCanonical));
        let mut malformed = baseline;
        malformed.id = 2;
        malformed.voices[63].actual_onset = f64::NAN;
        assert_eq!(tracker.replace_source(&malformed), Err(InvalidCanonical));
        assert_eq!(tracker.held_count(), 65);
        assert!(tracker.voices().any(|v| v.source == SourceId(2) && v.on_time == 0.0));
        assert!(
            tracker.roll().notes().filter(|n| n.source == SourceId(1)).all(|n| {
                !n.history_complete && n.start == 1.0 && n.segments(3.0).next().unwrap().0 .0 == 2.0
            }),
            "baseline state creates no invented pre-cut trajectory"
        );
    }

    fn delta(event: NoteEvent, sequence: u64) -> NoteDelta {
        NoteDelta {
            event,
            sequence,
            lifetime: 61,
            provenance: PitchProvenance::AcceptedOutput,
            timing: Some(EventTiming {
                clock: ClockId::default(),
                input: (event.time * 48000.0) as i64,
                planned: None,
                sample: (event.time * 48000.0) as i64,
                sample_rate: 48000.0,
            }),
            pitch_microcents: None,
        }
    }

    #[test]
    fn empty_baseline_keeps_completed_history_and_duplicates_do_not_replay() {
        let mut tracker = NoteTracker::new();
        let source = SourceId(1);
        let events = [
            delta(NoteEvent::on(0.0, source, 0, 60, 0.8), 1),
            delta(
                NoteEvent {
                    time: 0.01,
                    source,
                    channel: 0,
                    note: 60,
                    kind: NoteEventKind::Tuning { semitones: 0.25 },
                },
                2,
            ),
            delta(NoteEvent::off(1.0, source, 0, 60), 3),
        ];
        for event in events {
            assert_eq!(tracker.handle_canonical(CanonicalEvent::Note(event)), Ok(true));
        }
        let baseline = frame(&[]).unwrap();
        tracker.replace_source(&baseline).unwrap();
        for event in events {
            assert_eq!(tracker.handle_canonical(CanonicalEvent::Note(event)), Ok(false));
        }
        let note = tracker.roll().notes().next().unwrap();
        assert_eq!((note.start, note.end, note.settled_pitch()), (0.0, Some(1.0), 60.25));
        assert_eq!(tracker.roll().notes().count(), 1);
        let later = delta(NoteEvent::on(3.0, source, 0, 60, 0.8), 65);
        tracker.handle_canonical(CanonicalEvent::Note(later)).unwrap();
        assert_eq!(tracker.held_count(), 1);
        assert_eq!(tracker.roll().notes().count(), 2);
    }

    #[test]
    fn matching_baseline_preserves_held_ends_and_bends_without_an_attack() {
        let mut tracker = NoteTracker::new();
        let source = SourceId(1);
        tracker
            .handle_canonical(CanonicalEvent::Note(delta(
                NoteEvent::on(1.0, source, 0, 60, 0.8),
                1,
            )))
            .unwrap();
        tracker
            .handle_canonical(CanonicalEvent::Note(delta(
                NoteEvent {
                    time: 1.01,
                    source,
                    channel: 0,
                    note: 60,
                    kind: NoteEventKind::Tuning { semitones: 0.25 },
                },
                2,
            )))
            .unwrap();
        let high = tracker.highest_held();
        let before: Vec<_> = tracker.roll().notes().next().unwrap().segments(3.0).collect();
        let row = VoiceBaseline { pitch_microcents: 6_025_000_000, ..voice(60) };
        tracker.replace_source(&frame(&[row]).unwrap()).unwrap();
        assert_eq!(tracker.highest_held(), high);
        let note = tracker.roll().notes().next().unwrap();
        assert_eq!(note.segments(3.0).collect::<Vec<_>>(), before);
        assert_eq!(note.settled_pitch(), 60.25);
        assert!(note.history_complete);
        assert_eq!(tracker.roll().notes().count(), 1);
        let mut off = frame(&[row]).unwrap();
        off.id = 2;
        off.participating = false;
        tracker.replace_source(&off).unwrap();
        assert_eq!(tracker.held_count(), 0);
        assert!(tracker.highest_held().is_none());
        let mut tuning = delta(
            NoteEvent {
                time: 2.1,
                source,
                channel: 0,
                note: 60,
                kind: NoteEventKind::Tuning { semitones: 0.5 },
            },
            65,
        );
        tuning.pitch_microcents = Some(6_050_000_000);
        tracker.handle_canonical(CanonicalEvent::Note(tuning)).unwrap();
        assert_eq!(tracker.held_count(), 0, "Off output remains factual but hidden");
        let mut rejoin = SourceBaseline::new(
            source,
            3,
            2.2,
            1.0,
            65,
            true,
            &[VoiceBaseline { pitch_microcents: 6_050_000_000, ..row }],
            [ChannelBaseline::default(); 16],
        )
        .unwrap();
        tracker.replace_source(&rejoin).unwrap();
        assert_eq!(tracker.held_count(), 1);
        let note = tracker.roll().notes().next().unwrap();
        assert_eq!((note.start, note.settled_pitch(), note.end_pitch()), (1.0, 60.25, 60.5));
        assert_eq!(note.lifetime, Some(61));
        assert!(note.history_complete);
        rejoin.id += 1;
        tracker.replace_source(&rejoin).unwrap();
        assert_eq!(tracker.roll().notes().count(), 1);
    }
    #[test]
    fn a_full_bend_buffer_preserves_the_gap_after_a_later_expression() {
        let source = SourceId(1);
        let mut tracker = NoteTracker::new();
        tracker
            .handle_canonical(CanonicalEvent::Note(delta(
                NoteEvent::on(0.0, source, 0, 60, 0.8),
                1,
            )))
            .unwrap();
        for i in 1..64 {
            tracker
                .handle_canonical(CanonicalEvent::Note(delta(
                    NoteEvent {
                        time: i as f64 / 100.0,
                        source,
                        channel: 0,
                        note: 60,
                        kind: NoteEventKind::Tuning { semitones: i as f32 / 100.0 },
                    },
                    i + 1,
                )))
                .unwrap();
        }
        assert_eq!(tracker.roll().notes().next().unwrap().segments(0.7).count(), 64);
        tracker
            .handle_canonical(CanonicalEvent::Gap(PublicationGap {
                source: Some(source),
                time: 1.0,
                through: 1.9,
                first: 65,
                last: 66,
                reason: GapReason::PublicationFull,
            }))
            .unwrap();
        let row = VoiceBaseline {
            actual_onset: 0.0,
            input_onset: 0.0,
            pitch_microcents: 6_063_000_000,
            ..voice(60)
        };
        let frame = SourceBaseline::new(
            source,
            1,
            2.0,
            2.0,
            66,
            true,
            &[row],
            [ChannelBaseline::default(); 16],
        )
        .unwrap();
        tracker.replace_source(&frame).unwrap();
        tracker
            .handle_canonical(CanonicalEvent::Note(delta(
                NoteEvent {
                    time: 3.0,
                    source,
                    channel: 0,
                    note: 60,
                    kind: NoteEventKind::Tuning { semitones: 1.0 },
                },
                67,
            )))
            .unwrap();
        let segments: Vec<_> = tracker.roll().notes().next().unwrap().segments(3.1).collect();
        assert!(
            segments.iter().all(|((start, _), (end, _))| *end <= 1.0 || *start >= 2.0),
            "fold crossed missing interval: {segments:?}"
        );
        assert!(segments.iter().any(|((start, _), (end, _))| *start == 2.0 && *end == 3.0));
        assert!(segments.len() <= 64);
    }
}
