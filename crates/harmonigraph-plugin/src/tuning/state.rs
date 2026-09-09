//! One source's voices as the Hub scheduled them.
//!
//! There is one provenance here. The Hub derives this from its own sequenced
//! input at input plus that source's D, which is what the Tune will emit, so
//! the display and the take show a schedule rather than a confirmation. What
//! that costs is written down: a note the host refuses is drawn anyway, and
//! nothing in the protocol can tell.
use harmonigraph_core::canonical::{EventTiming, NoteDelta, SourceBaseline, VoiceBaseline};
use harmonigraph_core::confirmed::{
    ConfirmedPitch, ConfirmedPitches, PitchProvenance, HELD_PER_SOURCE,
};
use harmonigraph_core::{LatticePos, NoteEvent, NoteEventKind, SourceId, VoiceKey};

use super::event::Event;

/// Everything the Hub knows about one source's sounding notes.
pub struct State {
    voices: [Option<VoiceBaseline>; HELD_PER_SOURCE],
    /// MIDI channel displacement, which is also the attack pitch the policy
    /// scores. Bend and RPN 0 sensitivity both reduce into it.
    pitch: [harmonigraph_core::policy::channel::ChannelPitch; 16],
    pub pitch_changed: bool,
    /// False once a bounded store refused a voice. A source with an incomplete
    /// state publishes no baseline, because a partial one is worse than none.
    pub complete: bool,
}

impl Default for State {
    fn default() -> Self {
        Self {
            voices: [None; HELD_PER_SOURCE],
            pitch: [Default::default(); 16],
            pitch_changed: false,
            complete: true,
        }
    }
}

#[derive(Clone, Copy)]
pub struct Stamp {
    pub source: SourceId,
    pub sequence: u64,
    /// The onset's own serial. Only an attack supplies one; everything else
    /// finds its voice by what the event addresses.
    pub lifetime: u64,
    pub time: f64,
    pub input_time: f64,
    pub timing: EventTiming,
}

impl State {
    pub fn voices(&self) -> impl Iterator<Item = &VoiceBaseline> {
        self.voices.iter().flatten()
    }
    pub fn voice(&self, lifetime: u64) -> Option<&VoiceBaseline> {
        self.voices().find(|voice| voice.lifetime == lifetime)
    }
    pub fn count(&self) -> usize {
        self.voices().count()
    }
    /// The channel's current displacement in microcents, which is half of the
    /// absolute pitch an onset is scored at.
    pub fn channel_pitch(&self, channel: u8) -> i64 {
        self.pitch[usize::from(channel & 15)].microcents()
    }
    /// The cut, on this end. A source whose Tune cut has nothing sounding.
    pub fn clear(&mut self) {
        self.voices = [None; HELD_PER_SOURCE];
        self.pitch = [Default::default(); 16];
        self.complete = true;
    }
    /// Every voice a channel termination ends, so the caller can turn one
    /// controller into the note-offs the instrument is about to perform.
    pub fn on_channel(&self, channel: u8, out: &mut [(i32, u8, u8); HELD_PER_SOURCE]) -> usize {
        let mut count = 0;
        for voice in self.voices().filter(|voice| voice.channel == channel) {
            out[count] = (voice.host_note_id, voice.channel, voice.note);
            count += 1;
        }
        count
    }

    /// Freeze this onset's adaptive choice onto its voice. The emitted pitch
    /// includes the correction because the Tune states it as a per-note tuning
    /// expression at the same sample as the note.
    #[allow(clippy::too_many_arguments)]
    pub fn assign(
        &mut self,
        lifetime: u64,
        correction: i64,
        node: Option<LatticePos>,
        decision: u64,
        configuration: harmonigraph_core::configuration::ResolvedConfig,
        player: f64,
        channel_pitch: i64,
    ) {
        let Some(voice) = self.voices.iter_mut().flatten().find(|v| v.lifetime == lifetime) else {
            return;
        };
        voice.player_tuning = player;
        voice.frozen_offset_microcents = correction;
        voice.onset_pitch_microcents = i64::from(voice.note) * 100_000_000
            + correction
            + channel_pitch
            + (player * 100_000_000.0).round() as i64;
        voice.pitch_microcents = voice.onset_pitch_microcents;
        if decision != 0 {
            voice.assignment = Some(configuration);
            voice.decision = decision;
            voice.attack_node = node;
        }
    }

    /// What learning may read from this source. A source whose state has lost
    /// an event has no confirmed pitches rather than an empty list of them.
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
            provenance: PitchProvenance::AcceptedOutput,
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
        SourceBaseline::new(source, id, time, cut, true, &voices[..count]).ok()
    }

    /// Apply one sequenced input at the time it is scheduled to sound. A
    /// release and a per-note expression find their voice by what the event
    /// addresses, which is the same rule the Tune's held set uses.
    pub fn apply(&mut self, event: Event, stamp: Stamp) -> Option<NoteDelta> {
        self.pitch_changed = false;
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
            let pitch =
                i64::from(note) * 100_000_000 + self.pitch[usize::from(channel)].microcents();
            self.voices[index] = Some(VoiceBaseline {
                channel,
                note,
                host_note_id: id,
                lifetime: stamp.lifetime,
                input_onset: stamp.input_time,
                actual_onset: stamp.time,
                onset: Some(stamp.timing),
                velocity,
                pitch_microcents: pitch,
                onset_pitch_microcents: pitch,
                provenance: PitchProvenance::AcceptedOutput,
                ..VoiceBaseline::default()
            });
            result = Some((
                stamp.lifetime,
                channel,
                note,
                NoteEventKind::On { velocity },
                Some(pitch),
            ));
        } else if event.release() {
            if let Some(cell) = self.voices.iter_mut().find(|cell| {
                cell.is_some_and(|v| event.matches(v.host_note_id, v.channel, v.note))
            }) {
                let voice = cell.take().unwrap();
                result = Some((voice.lifetime, voice.channel, voice.note, NoteEventKind::Off, None));
            }
        } else if let Event::Expression { kind: 2, value, .. } = event {
            if let Some(voice) = self
                .voices
                .iter_mut()
                .flatten()
                .find(|v| event.matches(v.host_note_id, v.channel, v.note))
            {
                // The Tune emits the player's value composed with the frozen
                // correction, so the pitch this draws is the pitch that goes
                // on the wire rather than the player's half of it.
                let pitch = (f64::from(voice.note) + value) * 100_000_000.0
                    + self.pitch[usize::from(voice.channel)].microcents() as f64
                    + voice.frozen_offset_microcents as f64;
                if !value.is_finite() || !pitch.is_finite() || !in_range(pitch) {
                    self.complete = false;
                    return None;
                }
                voice.player_tuning = value;
                voice.pitch_microcents = pitch.round() as i64;
                result = Some((
                    voice.lifetime,
                    voice.channel,
                    voice.note,
                    NoteEventKind::Tuning { semitones: value as f32 },
                    Some(voice.pitch_microcents),
                ));
            }
        }
        if let Event::Midi { port: 0, data, .. } = event {
            let index = usize::from(data[0] & 15);
            let before = self.pitch[index].microcents();
            self.pitch_changed = self.pitch[index].apply(data);
            let change = self.pitch[index].microcents() - before;
            if change != 0 {
                for voice in
                    self.voices.iter_mut().flatten().filter(|v| usize::from(v.channel) == index)
                {
                    voice.pitch_microcents = voice.pitch_microcents.saturating_add(change);
                }
            }
        }
        result.map(|(lifetime, channel, note, kind, pitch_microcents)| NoteDelta {
            event: NoteEvent { source: stamp.source, time: stamp.time, channel, note, kind },
            sequence: stamp.sequence,
            lifetime,
            provenance: PitchProvenance::AcceptedOutput,
            timing: Some(stamp.timing),
            pitch_microcents,
            assignment: self.voice(lifetime).and_then(VoiceBaseline::metadata),
            partial_output: false,
        })
    }
}

fn in_range(pitch: f64) -> bool {
    pitch >= i64::MIN as f64 && pitch < i64::MAX as f64
}
