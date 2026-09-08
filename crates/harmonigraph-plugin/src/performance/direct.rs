//! DIRECT observations retain exact ingress independently from forwarding's
//! accepted-output facet. Configuration and canonical publication read this
//! one rich owner; neither reconstructs facts from a display or pitch-only row.
use harmonigraph_core::canonical::{ClockId, EventTiming, NoteDelta, VoiceBaseline};
use harmonigraph_core::confirmed::{ConfirmedPitches, PitchProvenance};
use harmonigraph_core::SourceId;
use nice_plug::wrapper::clap::configuration::OwnedInput;

use super::{
    event::Event,
    queue::Queue,
    state::{Stamp, State},
};

pub const OUTPUT_WINDOW: usize = 2048;
/// The replay window for observation-owned context, sized to
/// `CAPTURES_PER_SOURCE`: every carried change but a channel termination's
/// arrives with a source-0 capture record standing beside it in a queue of the
/// same length, and that queue is what latches first.
const CARRIED_WINDOW: usize = super::protocol::CAPTURES_PER_SOURCE;

/// One change the observation makes to a voice a clock boundary carried into
/// the Hub's tuning context, stamped with the sample it was heard at.
///
/// The Hub's merge is by sample, so these are a stream and not a state: the
/// policy has to score an onset against what was sounding at the onset's own
/// sample, and the observation runs a whole callback ahead of the merge
/// because the wrapper walks every input before performance sees any of it.
#[derive(Clone, Copy)]
pub struct Carried {
    pub sample: i64,
    pub lifetime: u64,
    /// What the voice sounds at now, or None where the observation ended it.
    pub player: Option<f64>,
}

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
    /// Changes to fenced voices, for the Hub's merge to replay at their own
    /// samples. A second consumer of the same observation as `pending`, with
    /// its own frontier: publication and sequencing advance independently, so
    /// neither can be made to read the other's cursor.
    carried: Queue<Carried, CARRIED_WINDOW>,
    /// The replay overflowed and its order is gone with it.
    pub carried_lost: bool,
    /// Lifetimes at or below this were struck before the boundary that ended
    /// forwarding's ownership of them, so no capture record can address them
    /// again and the Hub carries them as observation-owned context. Everything
    /// above it belongs to the capture stream, whose retained onset record
    /// survives the closing and arrives with its own identity -- carrying one
    /// of those as well is what gives a single physical note two owners.
    fence: u64,
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
            carried: Queue::default(),
            carried_lost: false,
            fence: 0,
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

    /// Cut the observation where a boundary ends forwarding's ownership of
    /// what it has already played. Everything struck up to here is what a
    /// healthy commit may carry; everything after it stays the capture
    /// stream's alone.
    pub fn fence(&mut self) {
        self.fence = self.lifetime;
    }
    /// The voices that fence orphaned and the observation still holds.
    pub fn fenced_voices(&self) -> impl Iterator<Item = &VoiceBaseline> {
        let fence = self.fence;
        self.state.voices().filter(move |voice| voice.lifetime <= fence)
    }
    /// A fresh carry starts from the state it seeded, so nothing the replay
    /// still holds may be applied to it a second time.
    pub fn start_carry(&mut self) {
        self.carried.clear();
        self.carried_lost = false;
    }
    /// The next carried change at or before `through`, removed.
    pub fn next_carried(&mut self, through: i64) -> Option<Carried> {
        self.carried.front().filter(|update| update.sample <= through)?;
        self.carried.pop()
    }
    pub fn reset(&mut self) {
        self.state = State::default();
        self.pending.clear();
        self.carried.clear();
        self.carried_lost = false;
        self.fence = 0;
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
                // A fenced voice is one the Hub scores from here rather than
                // from a capture record, so every change to one is owed to the
                // merge at the sample it was heard. The fence is the bound as
                // well as the rule: a change to any other voice would address
                // no observation-owned cell, and before the first boundary
                // there are none at all, so this queue stays empty.
                if lifetime != 0 && lifetime <= self.fence {
                    let player = self.state.voice(lifetime).map(|voice| voice.player_tuning);
                    if self.carried.push(Carried { sample, lifetime, player }).is_err() {
                        // Bounded failure: an unorderable replay ends the
                        // carried contribution rather than freezing it at a
                        // value the player has already left. Display,
                        // recording and learning read `state` and keep it.
                        self.carried.clear();
                        self.carried_lost = true;
                        self.fence = 0;
                    }
                }
            }
        }
    }

    /// An incomplete observation declines rather than clearing: what learning
    /// already holds for DIRECT is still the last thing that was true of it.
    pub fn sync_learning(&self, confirmed: &mut ConfirmedPitches) -> bool {
        self.state.complete && self.state.publish_confirmed(SourceId::DIRECT, true, confirmed)
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
