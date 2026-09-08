//! DIRECT observations retain exact ingress independently from forwarding's
//! accepted-output facet. Configuration and canonical publication read this
//! one rich owner; neither reconstructs facts from a display or pitch-only row.
use harmonigraph_core::canonical::{ClockId, EventTiming, NoteDelta};
use harmonigraph_core::confirmed::{ConfirmedPitch, ConfirmedPitches, PitchProvenance};
use harmonigraph_core::{SourceId, VoiceKey};
use nice_plug::wrapper::clap::configuration::OwnedInput;

use super::{
    event::Event,
    queue::Queue,
    state::{Stamp, State},
};

pub const OUTPUT_WINDOW: usize = 2048;

/// DIRECT cannot have an adaptive assignment or partial accepted output. Keep
/// its retained observation in the existing 128-byte window; enrich only when
/// publishing through the shared canonical boundary.
#[derive(Clone, Copy)]
struct Observation {
    event: harmonigraph_core::NoteEvent,
    sequence: u64,
    lifetime: u64,
    timing: Option<EventTiming>,
    pitch_microcents: Option<i64>,
}
impl From<NoteDelta> for Observation {
    fn from(delta: NoteDelta) -> Self {
        assert!(
            delta.assignment.is_none()
                && !delta.partial_output
                && delta.provenance == PitchProvenance::ObservedDirect
        );
        Self {
            event: delta.event,
            sequence: delta.sequence,
            lifetime: delta.lifetime,
            timing: delta.timing,
            pitch_microcents: delta.pitch_microcents,
        }
    }
}
impl From<Observation> for NoteDelta {
    fn from(delta: Observation) -> Self {
        Self {
            event: delta.event,
            sequence: delta.sequence,
            lifetime: delta.lifetime,
            timing: delta.timing,
            pitch_microcents: delta.pitch_microcents,
            provenance: PitchProvenance::ObservedDirect,
            assignment: None,
            partial_output: false,
        }
    }
}

pub struct Direct {
    pub state: State,
    pending: Queue<Observation, OUTPUT_WINDOW>,
    pub sequence: u64,
    lifetime: u64,
    pub baseline_id: u64,
    pub recovery: bool,
    pub lost: bool,
    anchor: Option<(ClockId, i64, f64, f64)>,
    pub coverage_start: f64,
    pub offset: i64,
}

impl Default for Direct {
    fn default() -> Self {
        Self {
            state: State::default(),
            pending: Queue::default(),
            sequence: 0,
            lifetime: 0,
            baseline_id: 0,
            recovery: false,
            lost: false,
            anchor: None,
            coverage_start: 0.0,
            offset: 0,
        }
    }
}

impl Direct {
    pub fn reanchor(&mut self, offset: i64) {
        assert!(self.pending().is_none());
        self.offset = offset;
        self.anchor = None;
        self.recovery = true;
    }
    pub fn begin(&mut self, clock: ClockId, raw: i64, time: f64, rate: f64) {
        match self.anchor {
            None => {
                self.anchor = Some((clock, raw, time, rate));
                self.coverage_start = time;
            }
            Some((old, _, _, old_rate)) if old != clock || old_rate != rate => {
                // Only an explicit owner reset can establish new certainty.
                self.state.complete = false;
            }
            _ => {}
        }
    }

    pub fn reset(&mut self) {
        self.state = State::default();
        self.pending.clear();
        self.anchor = None;
        self.recovery = false;
        self.lost = false;
        // Source sequences/lifetimes do not repeat when the raw clock resets.
    }

    pub fn observe(&mut self, input: OwnedInput) {
        let Some(event) = Event::from_input(input.value) else {
            return;
        };
        let (Some(sample), Some((clock, raw, time, rate))) = (input.sample, self.anchor) else {
            self.state.complete = false;
            self.lost = true;
            return;
        };
        let Some(distance) = sample.checked_sub(raw) else {
            self.state.complete = false;
            self.lost = true;
            return;
        };
        let time = time + distance as f64 / rate;
        let Some(sample) = sample.checked_add(self.offset) else {
            self.state.complete = false;
            self.lost = true;
            return;
        };
        let mut targets = [0; 64];
        let count = if event.attack().is_some() {
            let Some(next) = self.lifetime.checked_add(1) else {
                self.state.complete = false;
                self.lost = true;
                return;
            };
            self.lifetime = next;
            targets[0] = next;
            1
        } else {
            let mut count = 0;
            for voice in self.state.voices().filter(|voice| {
                if event.channel_termination().is_some() {
                    event.channel() == Some(voice.channel)
                } else {
                    event.matches(voice.host_note_id, voice.channel, voice.note)
                }
            }) {
                targets[count] = voice.lifetime;
                count += 1;
            }
            count
        };
        // Channel MIDI must be observed once even when it addresses no voice.
        for (index, lifetime) in targets[..count.max(1)].iter().copied().enumerate() {
            let Some(sequence) = self.sequence.checked_add(1) else {
                self.state.complete = false;
                self.lost = true;
                return;
            };
            let stamp = Stamp {
                source: SourceId::DIRECT,
                sequence,
                lifetime,
                time,
                input_time: time,
                timing: Some(EventTiming {
                    clock,
                    input: sample,
                    planned: None,
                    sample,
                    sample_rate: rate,
                }),
                provenance: PitchProvenance::ObservedDirect,
            };
            let applied = if let Some(choke) = event.channel_termination() {
                // DIRECT is an observation. One raw controller updates channel
                // state; its captured held lifetimes yield semantic endings,
                // independently from forwarding's host-acceptance witness.
                if index == 0 {
                    self.state.apply(event, stamp);
                }
                self.state.voice(lifetime).map(|voice| {
                    if choke {
                        Event::terminate(voice.host_note_id, voice.channel, voice.note)
                    } else {
                        Event::note_off(voice.host_note_id, voice.channel, voice.note, false)
                    }
                })
            } else {
                Some(event)
            };
            if let Some(delta) = applied.and_then(|event| self.state.apply(event, stamp)) {
                self.sequence = sequence;
                if self.pending.push(delta.into()).is_err() {
                    self.lost = true;
                    self.recovery = true;
                }
            }
        }
    }

    pub fn sync_learning(&self, confirmed: &mut ConfirmedPitches) -> bool {
        if !self.state.complete {
            return false;
        }
        let empty = ConfirmedPitch {
            key: VoiceKey { source: SourceId::DIRECT, channel: 0, note: 0 },
            lifetime: None,
            host_note_id: None,
            pitch_microcents: 0,
            onset_sample: 0,
            provenance: PitchProvenance::ObservedDirect,
        };
        let mut rows = [empty; 64];
        let count = self.state.confirmed(SourceId::DIRECT, &mut rows);
        confirmed.replace_source(SourceId::DIRECT, &rows[..count]).is_ok()
    }

    pub fn pending(&self) -> Option<NoteDelta> {
        self.pending.front().map(Into::into)
    }
    pub fn pending_end(&self) -> Option<i64> {
        self.pending.get(self.pending.len().checked_sub(1)?)?.timing?.sample.checked_add(1)
    }
    pub fn published(&mut self) {
        self.pending.pop();
    }
}

#[cfg(test)]
impl Direct {
    pub fn print_test_memory_layout(&self) {
        println!(
            "LEDGER observed DIRECT output [cell,count,backing] {:?}",
            self.pending.test_layout()
        );
    }
}
const _: () = assert!(std::mem::size_of::<Option<Observation>>() <= 128);
const _: () = assert!(std::mem::align_of::<Option<Observation>>() <= 8);
