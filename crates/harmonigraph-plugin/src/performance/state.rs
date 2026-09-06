//! Rich factual source state, owned by a serialized callback. The same reducer
//! accepts observed DIRECT ingress and actual accepted output; provenance is
//! explicit and neither a plan nor a publication acknowledgement is an event.
use harmonigraph_core::canonical::{
    ChannelBaseline, EventTiming, NoteDelta, SourceBaseline, VoiceBaseline,
};
use harmonigraph_core::confirmed::{ConfirmedPitch, PitchProvenance, HELD_PER_SOURCE};
use harmonigraph_core::{NoteEvent, NoteEventKind, SourceId};

use super::event::Event;

pub struct State {
    voices: [Option<VoiceBaseline>; HELD_PER_SOURCE],
    channels: [ChannelBaseline; 16],
    pub complete: bool,
}

impl Default for State {
    fn default() -> Self {
        Self {
            voices: [None; HELD_PER_SOURCE],
            channels: [ChannelBaseline::default(); 16],
            complete: true,
        }
    }
}

#[derive(Clone, Copy)]
pub struct Stamp {
    pub source: SourceId,
    pub sequence: u64,
    pub lifetime: u64,
    pub time: f64,
    pub input_time: f64,
    pub timing: Option<EventTiming>,
    pub provenance: PitchProvenance,
}

impl State {
    pub fn replace(&mut self, frame: &SourceBaseline) -> bool {
        if frame.validate().is_err() {
            self.complete = false;
            return false;
        }
        self.voices = [None; HELD_PER_SOURCE];
        for (cell, voice) in self.voices.iter_mut().zip(frame.voices().iter().copied()) {
            *cell = Some(voice);
        }
        self.channels = frame.channels;
        self.complete = true;
        true
    }
    pub fn voices(&self) -> impl Iterator<Item = &VoiceBaseline> {
        self.voices.iter().flatten()
    }

    pub fn voice(&self, lifetime: u64) -> Option<&VoiceBaseline> {
        self.voices().find(|voice| voice.lifetime == lifetime)
    }

    /// Unknown timestamp is never a synthetic note delta. Only factual terminal
    /// events can alter the known held set through this path.
    pub fn apply_unmapped_terminal(&mut self, event: Event, lifetime: u64) -> bool {
        if event.release() {
            if let Some(cell) = self
                .voices
                .iter_mut()
                .find(|cell| cell.is_some_and(|voice| voice.lifetime == lifetime))
            {
                *cell = None;
            }
            true
        } else if let Event::Midi { port: 0, data: [status, cc @ (64 | 66 | 69), 0], .. } = event {
            if status & 0xf0 != 0xb0 {
                return false;
            }
            let channel = &mut self.channels[usize::from(status & 15)];
            channel.controllers[usize::from(cc)] = 0;
            channel.controller_valid[usize::from(cc / 64)] |= 1 << (cc % 64);
            true
        } else {
            false
        }
    }

    pub fn pedals_held(&self) -> bool {
        self.channels
            .iter()
            .any(|channel| [64, 66, 69].iter().any(|cc| channel.controllers[*cc] >= 64))
    }

    pub fn count(&self) -> usize {
        self.voices().count()
    }

    pub fn channels(&self) -> &[ChannelBaseline; 16] {
        &self.channels
    }

    pub fn confirmed(
        &self,
        source: SourceId,
        rows: &mut [ConfirmedPitch; HELD_PER_SOURCE],
    ) -> usize {
        let mut count = 0;
        for voice in self.voices() {
            rows[count] = voice.confirmed(source);
            count += 1;
        }
        count
    }

    pub fn baseline(
        &self,
        source: SourceId,
        id: u64,
        cut: u64,
        time: f64,
        coverage_start: f64,
        participating: bool,
    ) -> Option<SourceBaseline> {
        if !self.complete {
            return None;
        }
        let mut voices = [VoiceBaseline::default(); HELD_PER_SOURCE];
        let mut count = 0;
        for voice in self.voices() {
            voices[count] = *voice;
            count += 1;
        }
        SourceBaseline::new(
            source,
            id,
            time,
            coverage_start,
            cut,
            participating,
            &voices[..count],
            self.channels,
        )
        .ok()
    }

    /// The caller has already assigned the original input lifetime and, for
    /// accepted output, settled host acceptance. A wildcard is applied to the
    /// lifetime set resolved at its original stream position by that caller.
    pub fn apply(&mut self, event: Event, stamp: Stamp) -> Option<NoteDelta> {
        let mut result = None;
        if let Some((id, channel, note, velocity)) = event.attack() {
            let index = self
                .voices
                .iter()
                .position(|v| v.is_some_and(|v| v.channel == channel && v.note == note))
                .or_else(|| self.voices.iter().position(Option::is_none));
            let Some(index) = index else {
                self.complete = false;
                return None;
            };
            let pitch = i64::from(note) * 100_000_000;
            self.voices[index] = Some(VoiceBaseline {
                channel,
                note,
                host_note_id: id,
                lifetime: stamp.lifetime,
                input_onset: stamp.input_time,
                actual_onset: stamp.time,
                onset: stamp.timing,
                velocity,
                pitch_microcents: pitch,
                provenance: stamp.provenance,
                ..VoiceBaseline::default()
            });
            result = Some((channel, note, NoteEventKind::On { velocity }, Some(pitch)));
        } else if event.release() {
            if let Some(cell) = self
                .voices
                .iter_mut()
                .find(|cell| cell.is_some_and(|v| v.lifetime == stamp.lifetime))
            {
                let voice = cell.take().unwrap();
                result = Some((voice.channel, voice.note, NoteEventKind::Off, None));
            }
        } else if let Event::Expression { kind: 2, value, .. } = event {
            if let Some(voice) =
                self.voices.iter_mut().flatten().find(|v| v.lifetime == stamp.lifetime)
            {
                let pitch = (f64::from(voice.note) + value) * 100_000_000.0;
                if !value.is_finite()
                    || !pitch.is_finite()
                    || pitch < i64::MIN as f64
                    || pitch >= i64::MAX as f64
                {
                    self.complete = false;
                    return None;
                }
                voice.player_tuning = value;
                voice.pitch_microcents = pitch.round() as i64;
                result = Some((
                    voice.channel,
                    voice.note,
                    NoteEventKind::Tuning { semitones: value as f32 },
                    Some(voice.pitch_microcents),
                ));
            }
        }
        if let Event::Midi { port: 0, data, .. } = event {
            let channel = &mut self.channels[usize::from(data[0] & 15)];
            match data[0] & 0xf0 {
                0xb0 if data[1] < 128 && data[2] < 128 => {
                    channel.controllers[usize::from(data[1])] = data[2];
                    channel.controller_valid[usize::from(data[1] / 64)] |= 1 << (data[1] % 64);
                }
                0xc0 => channel.program = Some(data[1]),
                0xd0 => channel.pressure = Some(data[1]),
                0xe0 => channel.pitch_bend = Some(u16::from(data[1]) | (u16::from(data[2]) << 7)),
                _ => {}
            }
        }
        result.map(|(channel, note, kind, pitch_microcents)| NoteDelta {
            event: NoteEvent { source: stamp.source, time: stamp.time, channel, note, kind },
            sequence: stamp.sequence,
            lifetime: stamp.lifetime,
            provenance: stamp.provenance,
            timing: stamp.timing,
            pitch_microcents,
        })
    }
}
