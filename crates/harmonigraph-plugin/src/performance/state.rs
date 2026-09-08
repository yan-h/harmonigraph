//! Rich factual source state, owned by a serialized callback. The same reducer
//! accepts observed DIRECT ingress and actual accepted output; provenance is
//! explicit and neither a plan nor a publication acknowledgement is an event.
use harmonigraph_core::canonical::{EventTiming, NoteDelta, SourceBaseline, VoiceBaseline};
use harmonigraph_core::confirmed::{
    ConfirmedPitch, ConfirmedPitches, PitchProvenance, HELD_PER_SOURCE,
};
use harmonigraph_core::{NoteEvent, NoteEventKind, SourceId, VoiceKey};

use super::event::Event;

/// Latest accepted ordinary MIDI channel values. Validity accompanies each
/// value: an untouched control is not fabricated neutral state.
///
/// Audio-side only. It used to ride out on every `SourceBaseline` as well,
/// where nothing ever read it — the display, the roll, the take reader and the
/// offline renderer all take channel state from nowhere, and the two callers
/// that want it (the sustain sweep and the pitch-centre debt) read this copy.
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
    pub fn voices(&self) -> impl Iterator<Item = &VoiceBaseline> {
        self.voices.iter().flatten()
    }

    pub fn voice(&self, lifetime: u64) -> Option<&VoiceBaseline> {
        self.voices().find(|voice| voice.lifetime == lifetime)
    }

    /// Unknown timestamp is never a synthetic note delta. Preserve factual
    /// terminal transactions without assigning those accepted events a
    /// musical timestamp.
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
        } else if let Event::Midi { port: 0, data: [status, first, second], .. } = event {
            let channel = &mut self.channels[usize::from(status & 15)];
            match status & 0xf0 {
                0xb0 if matches!(first, 64 | 66 | 69) && second == 0 => {
                    channel.controllers[usize::from(first)] = second;
                    channel.controller_valid[usize::from(first / 64)] |= 1 << (first % 64);
                    true
                }
                // The participation boundary's recenter. Like a pedal reset it
                // is a neutral value the reset owes whatever the clock knows,
                // so an unknown timestamp must not turn it into a lost fact.
                0xe0 if (first, second) == (0, 64) => {
                    channel.pitch_bend = Some(u16::from(first) | (u16::from(second) << 7));
                    true
                }
                _ => false,
            }
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
    pub fn partial(&mut self, lifetime: u64) {
        if let Some(voice) =
            self.voices.iter_mut().flatten().find(|voice| voice.lifetime == lifetime)
        {
            voice.partial_output = true;
            voice.release_pending = true;
        }
    }
    pub fn assignment(&mut self, lifetime: u64, binding: super::protocol::Assignment, player: f64) {
        if let Some(voice) =
            self.voices.iter_mut().flatten().find(|voice| voice.lifetime == lifetime)
        {
            voice.player_tuning = player;
            voice.frozen_offset_microcents = i64::from(binding.correction);
            if binding.decision != 0 && binding.selection.musical() {
                voice.assignment = Some(binding.configuration);
                voice.decision = binding.decision;
                voice.attack_node = binding.node();
            }
        }
    }

    pub fn channels(&self) -> &[ChannelBaseline; 16] {
        &self.channels
    }

    /// Replace what learning knows about one source with what this state
    /// holds. Both observers of held pitch publish through here -- a Hub row
    /// from its accepted output, DIRECT from its observed ingress -- because
    /// the two differ in where they get their voices, not in what a confirmed
    /// pitch is. `live` is the caller's own reason to contribute at all: a row
    /// that is Off, or a state that has lost an event, has no confirmed
    /// pitches rather than an empty list of them, and the caller decides which
    /// of those two it means by clearing or by declining to call.
    ///
    /// The scratch rows past `count` are never read, so their key and
    /// provenance are padding rather than a claim about this source.
    pub fn publish_confirmed(
        &self,
        source: SourceId,
        live: bool,
        confirmed: &mut ConfirmedPitches,
    ) -> bool {
        let empty = ConfirmedPitch {
            key: VoiceKey { source, channel: 0, note: 0 },
            lifetime: None,
            host_note_id: None,
            pitch_microcents: 0,
            onset_sample: 0,
            provenance: PitchProvenance::ObservedDirect,
        };
        let mut rows = [empty; HELD_PER_SOURCE];
        let mut count = 0;
        if live {
            for voice in self.voices() {
                rows[count] = voice.confirmed(source);
                count += 1;
            }
        }
        confirmed.replace_source(source, &rows[..count]).is_ok()
    }

    pub fn baseline(
        &self,
        source: SourceId,
        id: u64,
        cut: u64,
        time: f64,
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
        SourceBaseline::new(source, id, time, cut, participating, &voices[..count]).ok()
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
            assignment: self.voice(stamp.lifetime).and_then(VoiceBaseline::metadata),
            partial_output: false,
        })
    }
}
