//! Serialized ordinary performance owner. Storage is allocated before activation;
//! only actual host completions establish output facts or settle reservations.
use super::{
    clock::{Calibration, Clock, Coverage},
    event::Event,
    protocol::*,
    queue::{Indexed, Queue},
    registry::{SourceOffer, SourceReturn},
    setup,
    state::{Stamp, State},
};
use harmonigraph_core::canonical::{ClockId, EventTiming};
use harmonigraph_core::confirmed::PitchProvenance;
use harmonigraph_core::SourceId;
use nice_plug::wrapper::clap::{
    configuration::{InputValue, OwnedInput},
    performance as api,
};
use std::sync::atomic::Ordering;
use std::sync::Arc;

mod channel;
#[cfg(all(test, debug_assertions, not(feature = "tuning-probe")))]
mod replay_tests;
mod stop;
mod work;

const NONE: u16 = u16::MAX;
#[cfg(all(test, not(feature = "tuning-probe")))]
#[derive(Debug, PartialEq)]
pub struct Snapshot {
    pub pending: usize,
    pub references: usize,
    pub obligations: usize,
    pub old_obligations: usize,
    pub reference_high_water: usize,
    pub input_cut: u64,
    pub pedals_held: bool,
    pub note_off_owed: usize,
    pub intent_slots: usize,
    pub lives: usize,
    pub held: usize,
    pub journal: usize,
    pub emergency: usize,
    pub manifest: usize,
    pub faults: u32,
    pub epoch: u64,
    pub sequence: u64,
    pub transfer_cut: u64,
    pub baseline_cut: Option<u64>,
    pub seal: Option<u64>,
    pub complete_through: i64,
}
pub const STORAGE_FAULT: u32 = 1;
pub const OUTPUT_FAULT: u32 = 2;
pub const CLOCK_FAULT: u32 = 4;
pub const INPUT_FAULT: u32 = 8;
pub const REFERENCE_FAULT: u32 = 16;
pub const CLOSED: u64 = 2;
pub const OPEN: u64 = 0;
pub const BUSY: u64 = 1;
pub const FENCED: u64 = 3;

#[derive(Clone, Copy)]
struct Pending {
    channel: channel::Cell,
    serial: u64,
    event: Event,
    life: u16,
    input: i64,
    generation: u64,
    staged: bool,
    cleanup_queued: bool,
    cleanup_next: u16,
    disposition: bool,
    work_head: u16,
    work_tail: u16,
    work_count: u8,
    work_remaining: u8,
    selected: u16,
    inline_done: bool,
}
#[derive(Clone, Copy)]
struct ReleaseIndex {
    parent: u16,
    work: u16,
}
#[derive(Clone, Copy)]
struct Life {
    serial: u64,
    id: i32,
    channel: u8,
    key: u8,
    input: i64,
    refs: u32,
    active: bool,
    reserved: bool,
    sounded: bool,
    canceled: bool,
    shift: Option<i64>,
    terminal: Option<(u64, i64, bool)>,
    release: Option<ReleaseIndex>,
    generation: u64,
    midi: bool,
    note_off_owed: bool,
    sound_off_refs: u16,
}
#[derive(Clone, Copy)]
struct Permit {
    position: usize,
    serial: u64,
    credit: bool,
    gate: bool,
    emergency: bool,
}
#[derive(Clone, Copy)]
struct Release {
    life: u16,
    staged: bool,
    accepted: Option<OutputDelta>,
}
#[derive(Clone, Copy)]
struct Manifest {
    transaction: u64,
    position: usize,
    serial: u64,
    work: u16,
}

#[derive(Clone, Copy)]
struct PendingBaseline {
    id: u64,
    cut: u64,
    /// The original callback's actual coverage, independent of later output
    /// held behind this snapshot. Its progress report must survive retirement.
    coverage: Coverage,
}

pub struct Source {
    pub shared: Arc<setup::Shared>,
    pub offer: Option<SourceOffer>,
    direct: Option<Arc<SessionControl>>,
    pub state: State,
    pending: Indexed<Pending, PENDING_EVENTS>,
    work: work::Work,
    cleanup_head: u16,
    cleanup_tail: u16,
    lives: Box<[Option<Life>]>,
    free_lives: Vec<u16>,
    active: [u16; 64],
    reserved: [u16; 64],
    owed_note_off: [u16; 64],
    emergency: [Option<Release>; 64],
    channel_reset: [u8; 16],
    emergency_output: Queue<OutputDelta, 128>,
    journal: Queue<OutputDelta, OUTCOME_JOURNAL>,
    sent: usize,
    emergency_sent: usize,
    transfer_cut: u64,
    pub sequence: u64,
    next_lifetime: u64,
    next_event: u64,
    next_disposition: u64,
    attempt: u64,
    permit: Option<Permit>,
    pub clock: Clock,
    rate: f64,
    max_frames: u32,
    callback: Option<api::Callback>,
    visits: usize,
    cancel_cursor: Option<usize>,
    pub faults: u32,
    pub participating: bool,
    generation: u64,
    epoch: u64,
    baseline: Option<PendingBaseline>,
    next_baseline: u64,
    baseline_needed: bool,
    baseline_acked: bool,
    adopt_sent: bool,
    coverage: Option<Coverage>,
    last_progress: Option<(i64, u64)>,
    acknowledged: u64,
    complete_through: i64,
    sealed_ack: Option<u64>,
    wave_shift: i64,
    stopping: bool,
    transport_playing: bool,
    producer_joined: bool,
    joined_published: bool,
    joined_unknown_wire: bool,
    stops: stop::Stops,
    setup_pending: [Option<super::slots::Retained<setup::Update>>; 2],
    manifest: Queue<Manifest, 64>,
    input_complete: bool,
    input_reported: Option<i64>,
    old_pending: usize,
    obligations: usize,
    cancel_cut: u64,
    setup_started: u64,
    detaching: bool,
    attachment_attempted: bool,
    sealed: bool,
    sealed_generation: u64,
    transition_seen: u64,
    direct_generation: u64,
    service_revision: u64,
    channels: channel::Channels,
}

impl Source {
    #[cfg(all(test, not(feature = "tuning-probe")))]
    pub fn test_snapshot(&self) -> Snapshot {
        Snapshot {
            pending: self.pending.len(),
            references: self.work.len(),
            obligations: self.obligations,
            old_obligations: self.old_pending,
            reference_high_water: self.work.high_water,
            input_cut: self.next_event,
            pedals_held: self.state.pedals_held(),
            note_off_owed: self.owed_note_off.iter().filter(|index| **index != NONE).count(),
            intent_slots: self
                .offer
                .as_ref()
                .map_or(INTENT_RING, |offer| offer.endpoints.intents.slots()),
            lives: LIFETIMES - self.free_lives.len(),
            held: self.held(),
            journal: self.journal.len(),
            emergency: self.emergency_output.len(),
            manifest: self.manifest.len(),
            faults: self.faults,
            epoch: self.epoch,
            sequence: self.sequence,
            transfer_cut: self.transfer_cut,
            baseline_cut: self.baseline.map(|baseline| baseline.cut),
            seal: self.sealed.then_some(self.sealed_generation),
            complete_through: self.complete_through,
        }
    }
    #[cfg(all(test, not(feature = "tuning-probe")))]
    pub fn test_cell_sizes() -> [usize; 5] {
        [
            std::mem::size_of::<Pending>(),
            std::mem::size_of::<Life>(),
            std::mem::size_of::<OutputDelta>(),
            std::mem::size_of::<Manifest>(),
            std::mem::size_of::<Self>(),
        ]
    }
    pub fn new(shared: Arc<setup::Shared>) -> Box<Self> {
        Box::new(Self {
            shared,
            offer: None,
            direct: None,
            state: State::default(),
            pending: Indexed::default(),
            work: work::Work::default(),
            cleanup_head: NONE,
            cleanup_tail: NONE,
            lives: vec![None; LIFETIMES].into_boxed_slice(),
            free_lives: (0..LIFETIMES as u16).rev().collect(),
            active: [NONE; 64],
            reserved: [NONE; 64],
            owed_note_off: [NONE; 64],
            emergency: [None; 64],
            channel_reset: [0; 16],
            emergency_output: Queue::default(),
            journal: Queue::default(),
            sent: 0,
            emergency_sent: 0,
            transfer_cut: 0,
            sequence: 0,
            next_lifetime: 0,
            next_event: 0,
            next_disposition: 0,
            attempt: 0,
            permit: None,
            clock: Clock::new(Calibration::default(), 0.0, 0),
            rate: 0.0,
            max_frames: 0,
            callback: None,
            visits: 0,
            cancel_cursor: None,
            faults: 0,
            participating: true,
            generation: 1,
            epoch: 0,
            baseline: None,
            next_baseline: 0,
            baseline_needed: true,
            baseline_acked: false,
            adopt_sent: false,
            coverage: None,
            last_progress: None,
            acknowledged: 0,
            complete_through: i64::MIN,
            sealed_ack: None,
            wave_shift: 0,
            stopping: false,
            transport_playing: false,
            producer_joined: false,
            joined_published: false,
            joined_unknown_wire: false,
            stops: stop::Stops::default(),
            setup_pending: [None, None],
            manifest: Queue::default(),
            input_complete: false,
            input_reported: None,
            old_pending: 0,
            obligations: 0,
            cancel_cut: 0,
            setup_started: 0,
            detaching: false,
            attachment_attempted: false,
            sealed: false,
            sealed_generation: 0,
            transition_seen: 0,
            direct_generation: 1,
            service_revision: 0,
            channels: channel::Channels::default(),
        })
    }
    pub fn activate(&mut self, rate: f64, max_frames: u32) {
        let first = self.rate == 0.0;
        self.rate = rate;
        self.max_frames = max_frames;
        if !first {
            // Reactivation cannot install accepted setup over a still-owned lease.
            self.clock.valid = false;
            self.shared.publish_clock(&self.clock);
            self.stop();
            return;
        }
        self.apply_setup();
        self.clock = Clock::new(self.shared.value().routing.calibration(), rate, max_frames);
        self.shared.publish_clock(&self.clock);
        self.coverage = None;
        self.baseline_needed = true;
    }
    pub fn reset_idle_clock(&mut self, epoch: u64) {
        assert!(self.settled());
        self.epoch = epoch;
        self.clock = Clock::new(self.clock.calibration, self.rate, self.max_frames);
        self.shared.publish_clock(&self.clock);
        self.coverage = None;
        self.complete_through = i64::MIN;
        self.sealed_ack = None;
        self.sealed = false;
        self.transition_seen = 0;
    }
    pub fn attach_direct(&mut self, session: Arc<SessionControl>) {
        assert!(self.direct.is_none());
        self.epoch = session.epoch.load(Ordering::Acquire);
        self.direct = Some(session);
        self.direct_generation = self.generation;
        self.old_pending = self.obligations;
        self.baseline_acked = true;
    }
    fn session(&self) -> Option<&SessionControl> {
        self.offer.as_ref().map(|o| &*o.session).or(self.direct.as_deref())
    }
    fn source_id(&self) -> SourceId {
        self.offer.as_ref().map_or(SourceId::DIRECT, |o| o.lease.source)
    }
    fn incarnation(&self) -> u64 {
        self.offer.as_ref().map_or(0, |o| o.lease.incarnation)
    }
    fn lease_generation(&self) -> Option<u64> {
        self.offer
            .as_ref()
            .map(|offer| offer.generation)
            .or(self.direct.as_ref().map(|_| self.direct_generation))
    }
    pub fn held(&self) -> usize {
        self.reserved.iter().filter(|v| **v != NONE).count()
    }
    /// Only the main-thread destroy path may make this immutable no-more-wire
    /// assertion. Retirement never invokes a host or increments sequence.
    pub fn join_producer(&mut self) {
        assert!(!self.producer_joined);
        self.joined_unknown_wire = self.unknown_joined_wire_state();
        self.producer_joined = true;
    }
    pub fn joined_cut(&self) -> Option<u64> {
        self.producer_joined.then_some(self.sequence)
    }
    pub fn unknown_joined_wire_state(&self) -> bool {
        self.state.count() != 0
            || self.state.pedals_held()
            || self.owed_note_off != [NONE; 64]
            || self.emergency.iter().flatten().any(|release| release.accepted.is_none())
            || self.channel_reset != [0; 16]
    }
    pub fn settled(&self) -> bool {
        self.held() == 0
            && !self.state.pedals_held()
            && self.owed_note_off == [NONE; 64]
            && self.journal.len() == 0
            && self.emergency_output.len() == 0
            && self.pending.len() == 0
            && self.permit.is_none()
            && self.emergency.iter().all(Option::is_none)
            && self.channel_reset == [0; 16]
            && self.baseline.is_none()
            && self.manifest.len() == 0
    }
    fn lease_settled(&self) -> bool {
        self.held() == 0
            && !self.state.pedals_held()
            && self.owed_note_off == [NONE; 64]
            && self.journal.len() == 0
            && self.emergency_output.len() == 0
            && self.old_pending == 0
            && self.permit.is_none()
            && self.manifest.len() == 0
            && self.emergency.iter().all(Option::is_none)
            && self.channel_reset == [0; 16]
            && self.baseline.is_none()
    }

    /// Reserve the return/ack slot BEFORE moving an endpoint-bearing offer.
    /// Exactly one attach OR detach attempt occurs at an enclosing boundary.
    fn attachment(&mut self) {
        if self.attachment_attempted {
            return;
        }
        self.attachment_attempted = true;
        let Some(bridge) = self.shared.source.as_ref() else {
            return;
        };
        if let Some(current) = self.offer.as_ref() {
            let row = &current.session.rows[usize::from(current.lease.slot - 1)];
            if row.withdrawn.load(Ordering::Acquire) && self.lease_settled() {
                if !row.hub_detached.load(Ordering::Acquire) {
                    if !self.detaching
                        && row
                            .to_hub
                            .publish(Control::Detach {
                                incarnation: current.lease.incarnation,
                                epoch: self.epoch,
                                cut: self.sequence,
                            })
                            .is_ok()
                    {
                        self.detaching = true;
                    }
                    return;
                }
                if let Some(return_slot) = bridge.returns.reserve() {
                    row.source_detached.store(true, Ordering::Release);
                    return_slot.publish(SourceReturn::Returned(self.offer.take().unwrap()));
                    self.sequence = 0;
                    self.acknowledged = 0;
                    self.transfer_cut = 0;
                    self.complete_through = i64::MIN;
                    self.sealed_ack = None;
                    self.next_baseline = 0;
                    self.detaching = false;
                    self.sealed = false;
                    self.shared.request_main();
                }
            }
            return;
        }
        let Some(return_slot) = bridge.returns.reserve() else {
            return;
        };
        let Some(offer) = bridge.offers.take() else {
            return;
        };
        #[cfg(test)]
        self.shared.after_offer_take.reach();
        let row = &offer.session.rows[usize::from(offer.lease.slot - 1)];
        let claimed = row
            .emission_gate
            .compare_exchange(CLOSED, BUSY, Ordering::AcqRel, Ordering::Acquire)
            .is_ok();
        if offer.generation != bridge.generation.load(Ordering::Acquire)
            || row.withdrawn.load(Ordering::Acquire)
            || !offer.session.alive.load(Ordering::Acquire)
            || offer.session.closing.load(Ordering::Acquire) != 0
            || !claimed
            || offer.lease.incarnation != row.expected_incarnation.load(Ordering::Acquire)
        {
            if claimed {
                row.emission_gate.store(CLOSED, Ordering::Release);
            }
            return_slot.publish(SourceReturn::Returned(offer));
        } else {
            row.source_detached.store(false, Ordering::Release);
            row.emission_gate.store(CLOSED, Ordering::Release);
            self.epoch = offer.session.epoch.load(Ordering::Acquire);
            self.generation = offer.generation;
            self.old_pending = self.obligations;
            self.baseline_needed = true;
            self.baseline_acked = false;
            self.adopt_sent = false;
            self.coverage = None;
            self.clock.coverage = None;
            self.last_progress = None;
            return_slot.publish(SourceReturn::Adopted {
                generation: offer.generation,
                lease: offer.lease,
            });
            self.offer = Some(offer);
        }
        self.shared.request_main();
    }

    pub fn begin(&mut self, callback: api::Callback) {
        self.callback = Some(callback);
        self.stops.emergency_start = 0;
        self.visits = 0;
        self.input_complete = false;
        self.attachment_attempted = false;
        if self.offer.is_none() {
            self.attachment();
        }
        if !self.detaching {
            self.receive();
        }
        self.drain_finished();
        if let Some(session) = self.session() {
            let faults = session.faults.load(Ordering::Acquire)
                | self.offer.as_ref().map_or(0, |o| {
                    session.rows[usize::from(o.lease.slot - 1)].faults.load(Ordering::Acquire)
                });
            if faults != 0 {
                self.fault(faults);
            }
        }
        if callback.input_status != api::InputStatus::Complete {
            self.fault(INPUT_FAULT);
        }
        let valid = self.clock.valid;
        let coverage = self.clock.begin(callback.steady_time, callback.frames);
        if self.clock.valid != valid {
            self.shared.publish_clock(&self.clock);
        }
        if self.clock.calibration.validated && coverage.is_none() {
            self.fault(CLOCK_FAULT);
            if let Some(session) = self.session() {
                session.faults.fetch_or(CLOCK_FAULT, Ordering::AcqRel);
            }
        }
        self.coverage = coverage;
        if self.session().is_some_and(|s| s.epoch.load(Ordering::Acquire) != self.epoch) {
            self.fault(CLOCK_FAULT);
        }
        self.cancel_slice();
    }

    pub fn input(&mut self, input: OwnedInput) -> api::Consumption {
        if let InputValue::Parameter { value, modulation: false, .. } = input.value {
            // Tune has exactly one parameter; its original retained value is the
            // authority even if the generic parameter atomic has run ahead.
            if self.shared.source.is_some() {
                self.participating = value >= 0.5;
                self.baseline_needed = true;
            }
            return api::Consumption::Consumed;
        }
        if let InputValue::Transport(transport) = input.value {
            // The wrapper retains enclosing transport in the same input pool.
            // Observe it only here, in original order: a newer callback's raw
            // flag cannot move the cancellation cut ahead of retained input.
            let playing = transport.flags & (1 << 4) != 0;
            if self.transport_playing && !playing {
                let Some(sample) = input.sample else {
                    self.fault(INPUT_FAULT);
                    return api::Consumption::Consumed;
                };
                if self.capture_stop(sample) == api::Consumption::Pending {
                    return api::Consumption::Pending;
                }
            }
            self.transport_playing = playing;
            return api::Consumption::Consumed;
        }
        let Some(event) = Event::from_input(input.value) else {
            return api::Consumption::Consumed;
        };
        let Some(raw) = input.sample else {
            self.fault(INPUT_FAULT);
            return api::Consumption::Consumed;
        };
        let mut targets = [NONE; 64];
        let mut count = 0;
        let attack = event.attack();
        let channel = event.channel_control();
        let addressed = event.release()
            || matches!(event, Event::Expression { .. })
            || matches!(event, Event::Midi { data, .. } if data[0] & 0xf0 == 0xa0);
        for index in self.active.into_iter().filter(|index| *index != NONE) {
            let life = self.lives[usize::from(index)].unwrap();
            if channel.is_some_and(|channel| life.channel == channel)
                || attack
                    .is_some_and(|(_, channel, key, _)| life.channel == channel && life.key == key)
                || addressed && event.matches(life.id, life.channel, life.key)
            {
                targets[count] = index;
                count += 1;
            }
        }
        // Reserve the original envelope AND all derived references before any
        // input binding, lifetime pin, or input-cut mutation. Exhaustion leaves
        // the original value owned by the wrapper's retained input cursor.
        if self.pending.free() == 0 || self.next_event == u64::MAX {
            self.fault(STORAGE_FAULT);
            return api::Consumption::Pending;
        }
        let references = if addressed && count <= 1 { 0 } else { count };
        if self.work.free() < references {
            self.fault(REFERENCE_FAULT);
            return api::Consumption::Pending;
        }
        if !self.charge(references) {
            return api::Consumption::Pending;
        }
        let active_slot = if let Some((_, channel, key, _)) = attack {
            let slot = self.active.iter().position(|index| {
                *index == NONE
                    || self.lives[usize::from(*index)]
                        .is_some_and(|life| life.channel == channel && life.key == key)
            });
            if slot.is_none() || self.free_lives.is_empty() || self.next_lifetime == u64::MAX {
                self.fault(STORAGE_FAULT);
                return api::Consumption::Pending;
            }
            slot
        } else {
            None
        };
        let life = if let Some((id, channel, key, _)) = attack {
            let index = self.free_lives.pop().unwrap();
            self.next_lifetime += 1;
            self.lives[usize::from(index)] = Some(Life {
                serial: self.next_lifetime,
                id,
                channel,
                key,
                input: raw,
                refs: 0,
                active: true,
                reserved: false,
                sounded: false,
                canceled: false,
                shift: None,
                terminal: None,
                release: None,
                generation: self.generation,
                midi: matches!(event, Event::Midi { .. }),
                note_off_owed: false,
                sound_off_refs: 0,
            });
            index
        } else if addressed && count == 1 {
            targets[0]
        } else {
            NONE
        };
        let position = self.enqueue_cell(event, life, raw, addressed && count != 1);
        for target in targets[..count].iter().copied() {
            let previous = self.lives[usize::from(target)].unwrap();
            let operation = if channel.is_some() {
                work::CHANNEL
            } else if attack.is_some() {
                if previous.note_off_owed || previous.sound_off_refs != 0 {
                    work::NOTE_OFF
                } else {
                    work::CHOKE
                }
            } else {
                work::TARGET
            };
            if references != 0 {
                self.capture_work(position, target, operation);
            } else if event.release() {
                self.lives[usize::from(target)].as_mut().unwrap().release =
                    Some(ReleaseIndex { parent: position as u16, work: NONE });
            }
            if attack.is_some() || event.release() || event.channel_termination() == Some(false) {
                self.lives[usize::from(target)].as_mut().unwrap().active = false;
                if let Some(slot) = self.active.iter_mut().find(|index| **index == target) {
                    *slot = NONE;
                }
            }
        }
        if let Some(slot) = active_slot {
            self.active[slot] = life;
        }
        if channel.is_some() {
            self.capture_channel(position);
        } else {
            self.link_channel(position);
        }
        self.remove_finished(position);
        api::Consumption::Consumed
    }

    fn enqueue_cell(&mut self, event: Event, life: u16, input: i64, addressed: bool) -> usize {
        self.next_event += 1;
        let generation = if life == NONE {
            self.generation
        } else {
            self.lives[usize::from(life)].unwrap().generation
        };
        self.pending
            .push(Pending {
                channel: channel::Cell::default(),
                serial: self.next_event,
                event,
                life,
                input,
                generation,
                staged: false,
                cleanup_queued: false,
                cleanup_next: NONE,
                disposition: false,
                work_head: NONE,
                work_tail: NONE,
                work_count: 0,
                work_remaining: 0,
                selected: NONE,
                inline_done: addressed,
            })
            .unwrap_or_else(|_| unreachable!("reserved original input envelope"));
        if !addressed {
            self.add_obligation(generation);
        }
        let position = self.pending.back_position().unwrap();
        if life != NONE {
            self.lives[usize::from(life)].as_mut().unwrap().refs += 1;
        }
        position
    }

    pub fn apply_setup(&mut self) {
        self.apply_setup_with_clock(true);
    }

    pub fn apply_setup_with_clock(&mut self, allow_clock: bool) -> Option<setup::Update> {
        self.input_complete = true;
        self.fence_transition();
        // Candidate changes have the same original-input cut as an explicit
        // pairing edit, even when there is no GUI/state restore transaction.
        let desired = self
            .shared
            .source
            .as_ref()
            .map_or(self.generation, |b| b.generation.load(Ordering::Acquire));
        if desired > self.generation {
            self.generation = desired;
            // A new pairing generation classifies future input; it is not
            // authority to dispose requests already owned by the old lease.
        }
        // Called only after the wrapper's retained input cursor reaches the
        // captured callback boundary. Already retained events keep generation.
        for cell in &mut self.setup_pending {
            if cell.is_none() {
                *cell = self.shared.updates.retain();
            }
        }
        if self.setup_pending[1].as_ref().is_some_and(|b| {
            self.setup_pending[0].as_ref().is_none_or(|a| b.value.generation < a.value.generation)
        }) {
            self.setup_pending.swap(0, 1);
        }
        for index in 0..2 {
            let Some(update) = self.setup_pending[index].as_ref().map(|slot| slot.value) else {
                continue;
            };
            if update.generation <= self.shared.applied.load(Ordering::Acquire) {
                self.setup_pending[index] = None;
                continue;
            }
            if self.setup_started < update.generation {
                self.generation = self.generation.max(update.pairing_generation);
                if let Some(value) = update.participating {
                    self.participating = value;
                    self.baseline_needed = true;
                }
                if update.reset {
                    self.stop();
                }
                self.setup_started = update.generation;
            }
            let changes_clock =
                update.reset || update.routing.calibration() != self.clock.calibration;
            if changes_clock && !allow_clock {
                return Some(update);
            }
            if changes_clock
                && (self.held() != 0 || self.journal.len() != 0 || self.emergency_output.len() != 0)
            {
                // A new clock cannot reinterpret outstanding accepted history.
                self.shared.status.store(CLOCK_FAULT, Ordering::Release);
                break;
            }
            if changes_clock
                && self
                    .offer
                    .as_ref()
                    .is_some_and(|offer| offer.generation < update.pairing_generation)
            {
                break;
            }
            if update.reset && (!self.lease_settled() || !self.local_cancel_cut_settled()) {
                // Explicit recovery clears local inhibition only after the
                // old cancellation/release obligations have really settled.
                // Ordinary offer adoption cannot provide that authority.
                break;
            }
            if changes_clock {
                self.clock = Clock::new(update.routing.calibration(), self.rate, self.max_frames);
                self.coverage = None;
                self.baseline_needed = true;
            }
            if update.reset {
                self.faults = 0;
                self.shared.status.store(0, Ordering::Release);
            }
            self.shared.applied.store(update.generation, Ordering::Release);
            self.shared.publish_clock(&self.clock);
            self.setup_pending[index] = None;
        }
        None
    }

    pub fn fence_transition(&mut self) {
        let transition =
            self.session().map_or(0, |session| session.closing.load(Ordering::Acquire));
        if transition != 0 && transition != self.transition_seen {
            self.transition_seen = transition;
            if let Some(next) = self.generation.checked_add(1) {
                self.generation = next;
                if let Some(bridge) = &self.shared.source {
                    bridge.generation.fetch_max(next, Ordering::AcqRel);
                }
            } else {
                self.fault(STORAGE_FAULT);
            }
            self.stop();
        }
    }

    pub fn transition_settled(&self) -> bool {
        self.sealed && self.lease_settled()
    }

    pub fn commit_clock_setup(&mut self, update: setup::Update, epoch: u64) {
        assert!(self.direct.is_some() && self.transition_settled());
        self.clock = Clock::new(update.routing.calibration(), self.rate, self.max_frames);
        self.coverage = None;
        self.epoch = epoch;
        self.complete_through = i64::MIN;
        self.sealed_ack = None;
        self.direct_generation = self.generation;
        self.old_pending = self.obligations;
        self.sealed = false;
        self.transition_seen = 0;
        self.faults = 0;
        self.shared.status.store(0, Ordering::Release);
        self.shared.applied.store(update.generation, Ordering::Release);
        self.shared.publish_clock(&self.clock);
        for slot in &mut self.setup_pending {
            if slot.as_ref().is_some_and(|slot| slot.value.generation == update.generation) {
                *slot = None;
            }
        }
    }

    fn cancel_unsounded(&mut self) {
        self.cancel_unsounded_through(self.next_event);
    }
    fn cancel_unsounded_through(&mut self, cut: u64) {
        self.stopping = true;
        self.cancel_cursor = self.pending.front_position();
        self.cancel_cut = self.cancel_cut.max(cut);
    }
    fn local_cancel_cut_settled(&self) -> bool {
        self.pending
            .front_position()
            .and_then(|head| self.pending.at(head))
            .is_none_or(|pending| pending.serial > self.cancel_cut)
    }
    pub fn stop(&mut self) {
        self.cancel_unsounded();
        if self.held() != 0 || self.state.pedals_held() || self.owed_note_off != [NONE; 64] {
            self.fault(OUTPUT_FAULT);
        }
    }
    fn cancel_slice(&mut self) {
        if !self.stopping {
            return;
        }
        for _ in 0..256 {
            let Some(position) = self.cancel_cursor else {
                self.stopping = false;
                break;
            };
            if !self.charge(1) {
                break;
            }
            let pending = self.pending.at(position).unwrap();
            if pending.serial > self.cancel_cut {
                self.stopping = false;
                break;
            }
            if pending.staged {
                self.cancel_cursor = self.pending.next_position(position);
                continue;
            }
            let mut child = pending.work_head;
            let mut blocked = false;
            while child != NONE {
                if !self.charge(1) {
                    blocked = true;
                    break;
                }
                let cell = self.work.at(child);
                if cell.phase == 0 {
                    let life = self.lives[usize::from(cell.life)].unwrap();
                    if cell.operation == work::CHANNEL || !life.sounded || life.terminal.is_some() {
                        if !life.sounded {
                            self.lives[usize::from(cell.life)].as_mut().unwrap().canceled = true;
                        }
                        if !self.dispose_work(position, child) {
                            blocked = true;
                            break;
                        }
                    }
                }
                child = cell.next;
                if self.pending.at(position).is_none() {
                    break;
                }
            }
            if blocked {
                break;
            }
            if let Some(parent) = self.pending.at(position) {
                if !parent.inline_done
                    && !parent.disposition
                    && (parent.life == NONE
                        || self.lives[usize::from(parent.life)]
                            .is_some_and(|life| !life.sounded || life.terminal.is_some()))
                {
                    if parent.life != NONE {
                        self.lives[usize::from(parent.life)].as_mut().unwrap().canceled = true;
                    }
                    if !self.dispose_work(position, NONE) {
                        break;
                    }
                }
            }
            if self.cancel_cursor == Some(position) {
                self.cancel_cursor = self.pending.next_position(position);
            }
        }
    }

    pub fn fault(&mut self, fault: u32) {
        if self.faults & fault == fault {
            return;
        }
        if self.faults == 0 {
            self.cancel_unsounded();
        }
        self.faults |= fault;
        self.shared.status.store(self.faults, Ordering::Release);
        if let Some(offer) = &self.offer {
            offer.session.rows[usize::from(offer.lease.slot - 1)]
                .faults
                .fetch_or(fault, Ordering::AcqRel);
        }
        self.arm_release_debt();
    }

    fn arm_release_debt(&mut self) {
        // The immutable final cut already proves actual old-stream release
        // and neutralization. Outstanding credits are acknowledgement debt;
        // a stronger fault cannot create fresh output behind that seal.
        if self.sealed {
            return;
        }
        let mut used_channels = 0u16;
        for life in self.reserved.into_iter().chain(self.owed_note_off) {
            if life != NONE
                && self.lives[usize::from(life)]
                    .is_some_and(|l| l.sounded && (l.terminal.is_none() || l.note_off_owed))
            {
                used_channels |= 1 << self.lives[usize::from(life)].unwrap().channel;
                self.ensure_emergency(life);
            }
        }
        for channel in 0..16 {
            let state = &self.state.channels()[channel];
            if used_channels & (1 << channel) != 0 || state.controller_valid != [0, 0] {
                for (bit, controller) in [64usize, 66, 69].into_iter().enumerate() {
                    let known_neutral =
                        state.controller_valid[controller / 64] & (1 << (controller % 64)) != 0
                            && state.controllers[controller] < 64;
                    // A completed reset is already factual; another fault
                    // cannot recreate it. A staged reset still owns its debt.
                    if !known_neutral && self.channel_reset[channel] & (1 << (bit + 3)) == 0 {
                        self.channel_reset[channel] |= 1 << bit;
                    }
                }
            }
        }
        self.baseline_needed = true;
    }

    fn channel_has_release_debt(&self, channel: u8) -> bool {
        self.channel_reset[usize::from(channel)] != 0
            || self.emergency.iter().flatten().any(|release| {
                release.accepted.is_none()
                    && self.lives[usize::from(release.life)]
                        .is_some_and(|life| life.channel == channel)
            })
    }

    fn ensure_emergency(&mut self, life: u16) {
        if self.emergency.iter().flatten().any(|release| release.life == life) {
            return;
        }
        let slot = self
            .emergency
            .iter()
            .position(Option::is_none)
            .expect("at most64 held/owed wire lifetimes");
        self.lives[usize::from(life)].as_mut().unwrap().refs += 1;
        self.emergency[slot] = Some(Release { life, staged: false, accepted: None });
    }
    pub fn schedule(&mut self, block: api::Block, output: &mut api::Output<'_>) {
        let Some(start) = block.callback.steady_time.checked_add(i64::from(block.start)) else {
            self.fault(CLOCK_FAULT);
            return;
        };
        let Some(end) = start.checked_add(i64::from(block.frames)) else {
            self.fault(CLOCK_FAULT);
            return;
        };
        self.advance_stops(block.start.max(output.cursor()));
        self.schedule_emergency(output);
        // At most 64 indexed established releases; blocked unsounded attacks
        // cannot hide these behind the 8,192-event ordinary queue.
        for index in self.reserved {
            if self.visits == 2048 {
                break;
            }
            self.visits += 1;
            if index == NONE {
                continue;
            }
            let life = self.lives[usize::from(index)].unwrap();
            if life.sounded && life.terminal.is_none() {
                if let Some(position) = life.release {
                    self.stage_work(
                        usize::from(position.parent),
                        position.work,
                        start,
                        end,
                        output,
                    );
                }
            }
        }
        self.schedule_pending(start, end, output);
        self.compact();
    }

    fn schedule_pending(&mut self, start: i64, end: i64, output: &mut api::Output<'_>) {
        let mut next = self.pending.front_position();
        while self.visits < 2048 {
            let Some(position) = next else {
                break;
            };
            next = self.pending.next_position(position);
            self.visits += 1;
            let pending = self.pending.at(position).unwrap();
            if pending.cleanup_queued || pending.staged {
                continue;
            }
            if !self.stage_pending(position, start, end, output) {
                break;
            }
        }
    }

    fn charge(&mut self, count: usize) -> bool {
        if count > 2048 - self.visits {
            return false;
        }
        self.visits += count;
        true
    }
    fn stage_pending(
        &mut self,
        position: usize,
        start: i64,
        end: i64,
        output: &mut api::Output<'_>,
    ) -> bool {
        let Some(parent) = self.pending.at(position) else {
            return true;
        };
        if parent.event == Event::Stop {
            return false;
        }
        if parent.inline_done && parent.work_remaining == 0 {
            self.remove_finished(position);
            return true;
        }
        if parent.staged {
            return true;
        }
        if !matches!(parent.channel.role, channel::Role::Header { .. }) {
            let mut child = parent.work_head;
            while child != NONE {
                if !self.charge(1) {
                    return false;
                }
                let cell = self.work.at(child);
                if cell.phase == 0 {
                    return self.stage_work(position, child, start, end, output);
                }
                child = cell.next;
            }
            // A retrigger's old termination remains an obligation through its
            // cancellation acknowledgement, even after another child settles.
            if parent.work_remaining != 0 {
                return false;
            }
        }
        if parent.inline_done || parent.disposition {
            return true;
        }
        self.stage_work(position, NONE, start, end, output)
    }
    fn stage_work(
        &mut self,
        position: usize,
        child: u16,
        start: i64,
        end: i64,
        output: &mut api::Output<'_>,
    ) -> bool {
        let Some(parent) = self.pending.at(position) else {
            return true;
        };
        if parent.staged {
            return true;
        }
        if child != NONE && self.work.at(child).phase != 0 {
            return true;
        }
        let pending = self.resolved(position, child);
        if pending.disposition || child == NONE && parent.inline_done {
            return true;
        }
        if (self.faults != 0 || pending.serial <= self.cancel_cut) && !pending.event.release() {
            return false;
        }
        if !pending.event.release()
            && pending.event.channel().is_some_and(|channel| self.channel_has_release_debt(channel))
        {
            return false;
        }
        if !self.channel_ready(pending) {
            return false;
        }
        let life = (pending.life != NONE).then(|| self.lives[usize::from(pending.life)].unwrap());
        if self.detaching
            || self.sealed
            || self.offer.as_ref().is_some_and(|offer| pending.generation > offer.generation)
            || self.direct.is_some()
                && self.transition_seen != 0
                && pending.generation > self.direct_generation
        {
            return false;
        }
        if life.is_some_and(|life| {
            life.canceled
                || life.terminal.is_some() && !(life.note_off_owed && pending.event.release())
        }) {
            self.dispose_work(position, child);
            return true;
        }
        if pending.event.attack().is_some() && !self.admitted(pending.generation) {
            return false;
        }
        let established = life.is_some_and(|life| life.sounded);
        let shift = if established { life.unwrap().shift.unwrap_or(0) } else { self.wave_shift };
        let Some(mut due) = pending.input.checked_add(shift) else {
            self.fault(CLOCK_FAULT);
            return false;
        };
        due = due.max(start);
        if due >= end || self.next_stop_sample().is_some_and(|stop| due >= stop) {
            return false;
        }
        let callback = self.callback.unwrap();
        let Some(offset) =
            due.checked_sub(callback.steady_time).and_then(|value| u32::try_from(value).ok())
        else {
            self.fault(CLOCK_FAULT);
            return false;
        };
        let time = offset.max(output.cursor());
        let Some(attempt) = self.attempt.checked_add(1) else {
            self.fault(STORAGE_FAULT);
            return false;
        };
        self.attempt = attempt;
        let token = api::Token([
            attempt,
            position as u64,
            pending.serial,
            if child == NONE { 0 } else { u64::from(child) + 3 },
        ]);
        let Ok(group) = api::Group::single(token, api::Lane::Normal, time, pending.event.input())
        else {
            self.fault(INPUT_FAULT);
            return false;
        };
        if output.stage(group).is_err() {
            return false;
        }
        if !established {
            if let Some(shift) = callback
                .steady_time
                .checked_add(i64::from(time))
                .and_then(|time| time.checked_sub(pending.input))
            {
                self.wave_shift = shift;
            }
        }
        let mut parent = self.pending.at(position).unwrap();
        parent.staged = true;
        parent.selected = child;
        self.pending.set(position, parent);
        true
    }

    fn admitted(&self, generation: u64) -> bool {
        let Some(session) = self.session() else {
            return false;
        };
        if !session.alive.load(Ordering::Acquire) || session.closing.load(Ordering::Acquire) != 0 {
            return false;
        }
        if let Some(offer) = &self.offer {
            let row = &session.rows[usize::from(offer.lease.slot - 1)];
            // Inputs captured before a first attachment are deliberately kept;
            // re-pair inputs wait until the old lease has fully detached.
            generation <= offer.generation
                && !row.withdrawn.load(Ordering::Acquire)
                && offer.lease.incarnation == row.expected_incarnation.load(Ordering::Acquire)
                && self.epoch == session.epoch.load(Ordering::Acquire)
                && row.emission_gate.load(Ordering::Acquire) == OPEN
        } else {
            true
        }
    }

    pub fn prepare(&mut self, group: api::Group) -> bool {
        assert!(self.permit.is_none());
        if matches!(group.token.0[3], 1 | 2) {
            return self.prepare_emergency(group);
        }
        let position = group.token.0[1] as usize;
        let child = if group.token.0[3] == 0 { NONE } else { (group.token.0[3] - 3) as u16 };
        let Some(parent) = self.pending.at(position).filter(|parent| {
            parent.serial == group.token.0[2] && parent.staged && parent.selected == child
        }) else {
            return false;
        };
        let pending = self.resolved(position, child);
        let actual = self.callback.unwrap().steady_time.checked_add(i64::from(group.time));
        if actual.is_none_or(|actual| self.next_stop_sample().is_some_and(|stop| actual >= stop)) {
            return false;
        }
        // Stop/Reset cancellation can remain acknowledgement-blocked after
        // its marker is gone. The cut itself inhibits every old non-release,
        // including a controller staged before the boundary was reached.
        if (self.faults != 0 || pending.serial <= self.cancel_cut) && !pending.event.release() {
            return false;
        }
        if !pending.event.release()
            && pending.event.channel().is_some_and(|channel| self.channel_has_release_debt(channel))
        {
            return false;
        }
        // Reserve all post-acceptance target visits, pin release and cell cleanup
        // before the host can accept a multi-terminal physical wire message.
        let completion_work = if matches!(parent.channel.role, channel::Role::Header { .. }) {
            4 * usize::from(parent.work_count)
        } else if parent.work_remaining <= 1 {
            usize::from(parent.work_count)
        } else {
            1
        };
        if !self.charge(completion_work) {
            return false;
        }
        if !self.clock.valid && !pending.event.release() {
            return false;
        }
        let report_cells = self.channel_report_cells(pending);
        // Preserve the entire reserved emergency allowance before every normal
        // host acceptance. Exhaustion must occur while terminations can still
        // receive unique factual sequence numbers; clearing a fault cannot wrap.
        if self.journal.free() < report_cells
            || self
                .sequence
                .checked_add(report_cells as u64)
                .and_then(|sequence| sequence.checked_add(api::EMERGENCY_OUTPUT_ATTEMPTS as u64))
                .is_none()
        {
            self.fault(STORAGE_FAULT);
            return false;
        }
        let mut permit = Permit {
            position,
            serial: pending.serial,
            credit: false,
            gate: false,
            emergency: false,
        };
        if !self.channel_ready(pending) || !self.channel_wire_bindings_available(pending) {
            return false;
        }
        if pending.event.attack().is_some() {
            if !self.admitted(pending.generation) {
                return false;
            }
            // A channel choke can end the logical reservation while the
            // original Note-Off remains a real wire obligation. Both kinds
            // share the fixed64 emergency voice owners without double counting.
            let unreserved_offs = self
                .owed_note_off
                .iter()
                .copied()
                .filter(|index| *index != NONE)
                .filter(|index| !self.lives[usize::from(*index)].unwrap().reserved)
                .count();
            if self.held() + unreserved_offs >= 64 {
                return false;
            }
            let Some(slot) = self.reserved.iter().position(|v| *v == NONE) else {
                return false;
            };
            if let Some(offer) = &self.offer {
                let row = &offer.session.rows[usize::from(offer.lease.slot - 1)];
                if row
                    .emission_gate
                    .compare_exchange(OPEN, BUSY, Ordering::AcqRel, Ordering::Acquire)
                    .is_err()
                {
                    return false;
                }
                permit.gate = true;
            }
            let session = self.session().unwrap();
            let credits = session.credits.load(Ordering::Acquire);
            if credits >= HELD_SESSION
                || session
                    .credits
                    .compare_exchange(credits, credits + 1, Ordering::AcqRel, Ordering::Acquire)
                    .is_err()
            {
                if permit.gate {
                    self.release_gate();
                }
                return false;
            }
            permit.credit = true;
            self.reserved[slot] = pending.life;
            self.lives[usize::from(pending.life)].as_mut().unwrap().reserved = true;
        } else if pending.life != NONE {
            let life = self.lives[usize::from(pending.life)].unwrap();
            if !life.sounded
                || life.terminal.is_some() && !(life.note_off_owed && pending.event.release())
            {
                return false;
            }
        }
        self.permit = Some(permit);
        true
    }

    pub fn complete(&mut self, completion: api::Completion, output: &mut api::Output<'_>) {
        if matches!(completion.group.token.0[3], 1 | 2) {
            self.complete_emergency(completion);
            self.schedule_emergency(output);
            if self.faults == 0 {
                // Accepted Stop debt can unblock post-Stop live input in this
                // same callback. Retention acknowledgement may arrive later.
                let callback = self.callback.unwrap();
                self.schedule_pending(
                    callback.steady_time + i64::from(completion.group.time),
                    callback.steady_time + i64::from(callback.frames),
                    output,
                );
            }
            return;
        }
        let position = completion.group.token.0[1] as usize;
        let child = if completion.group.token.0[3] == 0 {
            NONE
        } else {
            (completion.group.token.0[3] - 3) as u16
        };
        let Some(mut parent) = self.pending.at(position).filter(|parent| {
            parent.serial == completion.group.token.0[2]
                && parent.staged
                && parent.selected == child
        }) else {
            return;
        };
        let pending = self.resolved(position, child);
        parent.staged = false;
        self.pending.set(position, parent);
        let permit = self.permit.take();
        if completion.accepted & 1 != 0 {
            let permit = permit.expect("accepted output requires durable preparation");
            assert_eq!((permit.position, permit.serial), (position, parent.serial));
            let actual = self
                .callback
                .unwrap()
                .steady_time
                .checked_add(i64::from(completion.group.time))
                .unwrap();
            let delta = self.record(pending.event, pending.life, pending.input, actual);
            self.journal.push(delta).unwrap_or_else(|_| unreachable!("prepared journal credit"));
            self.record_channel_terminals(pending, delta.sequence, actual);
            let waiter = match parent.channel.role {
                channel::Role::Header { first_waiter, .. } => first_waiter,
                _ => NONE,
            };
            if pending.event.attack().is_some() {
                self.wave_shift = actual.checked_sub(pending.input).unwrap_or(0);
            }
            if matches!(parent.channel.role, channel::Role::Header { .. }) {
                let mut target = parent.work_head;
                while target != NONE {
                    let cell = self.work.at(target);
                    self.finish_work(position, target);
                    target = cell.next;
                }
            }
            self.finish_work(position, child);
            if permit.gate {
                self.release_gate();
            }
            self.wake_waiters(waiter, output);
            self.wake_channel(pending.event.channel(), output);
            if self.pending.at(position).is_some_and(|parent| parent.serial == pending.serial)
                && self.charge(1)
            {
                let callback = self.callback.unwrap();
                self.stage_pending(
                    position,
                    callback.steady_time,
                    callback.steady_time.saturating_add(i64::from(callback.frames)),
                    output,
                );
            }
        } else {
            if let Some(permit) = permit {
                if permit.credit {
                    self.return_credit(pending.life);
                }
                if permit.gate {
                    self.release_gate();
                }
            }
            if completion.attempted != 0
                || completion.disposition == api::Disposition::MissingOutput
            {
                self.fault(OUTPUT_FAULT);
            }
        }
        if parent.serial <= self.cancel_cut
            && self.pending.at(position).is_some_and(|value| value.serial == parent.serial)
        {
            // Stop may arrive while this child is staged. Complete accepted
            // facts first, then revisit the still-owned remainder on the next
            // bounded cancellation slice; skipping the staged parent is not an
            // authorization for its unsounded children to emit later.
            self.stopping = true;
            self.cancel_cursor = self.pending.front_position();
        }
        self.schedule_emergency(output);
    }

    fn record(&mut self, event: Event, life: u16, input: i64, actual: i64) -> OutputDelta {
        self.sequence += 1;
        let mapped = self.clock.valid
            && self.clock.calibration.map(input).is_some()
            && self.clock.calibration.map(actual).is_some();
        let mapped_input = if mapped { self.clock.calibration.map(input).unwrap() } else { input };
        let mapped_actual =
            if mapped { self.clock.calibration.map(actual).unwrap() } else { actual };
        let lifetime = if life == NONE { 0 } else { self.lives[usize::from(life)].unwrap().serial };
        let delta = OutputDelta {
            incarnation: self.incarnation(),
            sequence: self.sequence,
            lifetime,
            input: mapped_input,
            actual: mapped_actual,
            epoch: self.epoch,
            mapped,
            discontinuity_generation: if mapped { 0 } else { self.generation },
            event,
            outcome: Outcome::Wire,
        };
        let clock =
            ClockId { runtime_session: self.session().map_or(0, |s| s.runtime), epoch: self.epoch };
        let stamp = Stamp {
            source: self.source_id(),
            sequence: self.sequence,
            lifetime,
            time: mapped_actual as f64 / self.rate,
            input_time: mapped_input as f64 / self.rate,
            timing: mapped.then_some(EventTiming {
                clock,
                input: mapped_input,
                planned: None,
                sample: mapped_actual,
                sample_rate: self.rate,
            }),
            provenance: PitchProvenance::AcceptedOutput,
        };
        if mapped {
            self.state.apply(event, stamp);
        } else {
            assert!(self.state.apply_unmapped_terminal(event, lifetime));
        }
        if life != NONE {
            let value = self.lives[usize::from(life)].as_mut().unwrap();
            if event.attack().is_some() {
                value.sounded = true;
                value.shift = actual.checked_sub(input);
            }
            if event.release() {
                value.terminal = Some((self.sequence, mapped_actual, mapped));
                if value.note_off_owed {
                    let slot = self
                        .owed_note_off
                        .iter_mut()
                        .find(|index| **index == life)
                        .expect("owned Note-Off binding");
                    *slot = NONE;
                    value.refs -= 1;
                }
                value.note_off_owed = false;
                value.release = None;
                value.active = false;
                if let Some(slot) = self.active.iter_mut().find(|i| **i == life) {
                    *slot = NONE;
                }
            }
        }
        delta
    }
    fn release_gate(&self) {
        if let Some(offer) = &self.offer {
            let row = &offer.session.rows[usize::from(offer.lease.slot - 1)];
            assert_eq!(row.emission_gate.swap(OPEN, Ordering::Release), BUSY);
        }
    }
    fn return_credit(&mut self, index: u16) {
        let value = self.lives[usize::from(index)].as_mut().unwrap();
        assert!(value.reserved);
        value.reserved = false;
        let previous = self.session().unwrap().credits.fetch_sub(1, Ordering::AcqRel);
        assert!(previous != 0);
        *self.reserved.iter_mut().find(|v| **v == index).unwrap() = NONE;
        self.recycle(index);
    }
    fn recycle(&mut self, index: u16) {
        if self.lives[usize::from(index)].is_some_and(|l| !l.active && !l.reserved && l.refs == 0) {
            self.lives[usize::from(index)] = None;
            self.free_lives.push(index);
            if let Some(slot) = self.active.iter_mut().find(|i| **i == index) {
                *slot = NONE;
            }
        }
    }
    fn compact(&mut self) {
        self.drain_finished();
        if self.pending.len() == 0
            && self.state.count() == 0
            && self
                .state
                .channels()
                .iter()
                .all(|channel| [64, 66, 69].iter().all(|cc| channel.controllers[*cc] < 64))
        {
            self.wave_shift = 0;
        }
    }

    fn schedule_emergency(&mut self, output: &mut api::Output<'_>) {
        if self.sealed {
            return;
        }
        for index in 0..64 {
            let Some(mut release) = self.emergency[index] else {
                continue;
            };
            if release.staged || release.accepted.is_some() {
                continue;
            }
            let life = self.lives[usize::from(release.life)].unwrap();
            let token = api::Token([self.attempt, index as u64, life.serial, 1]);
            let event = if life.note_off_owed {
                Event::note_off(life.id, life.channel, life.key, life.midi)
            } else {
                Event::terminate(life.id, life.channel, life.key)
            };
            let Ok(group) = api::Group::single(
                token,
                api::Lane::Emergency,
                output.cursor().max(self.stops.emergency_start),
                event.input(),
            ) else {
                continue;
            };
            if output.stage(group).is_err() {
                break;
            }
            release.staged = true;
            self.emergency[index] = Some(release);
        }
        for channel in 0..16 {
            for (bit, controller) in [64, 66, 69].into_iter().enumerate() {
                if self.channel_reset[channel] & (1 << bit) == 0 {
                    continue;
                }
                let event =
                    Event::Midi { port: 0, data: [0xb0 | channel as u8, controller, 0], flags: 0 };
                let group = api::Group::single(
                    api::Token([0, channel as u64, bit as u64, 2]),
                    api::Lane::Emergency,
                    output.cursor().max(self.stops.emergency_start),
                    event.input(),
                )
                .unwrap();
                if output.stage(group).is_err() {
                    return;
                }
                self.channel_reset[channel] &= !(1 << bit);
                self.channel_reset[channel] |= 1 << (bit + 3);
            }
        }
    }
    fn prepare_emergency(&mut self, group: api::Group) -> bool {
        if self.sealed || self.emergency_output.free() == 0 || self.sequence == u64::MAX {
            return false;
        }
        self.permit = Some(Permit {
            position: group.token.0[1] as usize,
            serial: group.token.0[2],
            credit: false,
            gate: false,
            emergency: true,
        });
        true
    }
    fn complete_emergency(&mut self, completion: api::Completion) {
        let permit = self.permit.take();
        let index = completion.group.token.0[1] as usize;
        if completion.group.token.0[3] == 1 {
            let Some(mut release) = self.emergency[index] else {
                return;
            };
            release.staged = false;
            if completion.accepted & 1 != 0 {
                assert!(permit.is_some_and(|p| p.emergency));
                let life = self.lives[usize::from(release.life)].unwrap();
                let pending_release = life.release;
                let actual = self.callback.unwrap().steady_time + i64::from(completion.group.time);
                let event = Event::from_input(completion.group.event(0).unwrap()).unwrap();
                let delta = self.record(event, release.life, life.input, actual);
                self.emergency_output
                    .push(delta)
                    .unwrap_or_else(|_| unreachable!("prepared emergency cell"));
                release.accepted = Some(delta);
                if let Some(address) = pending_release {
                    let position = usize::from(address.parent);
                    if let Some(parent) = self.pending.at(position) {
                        let valid = if address.work == NONE {
                            parent.life == release.life
                                && !parent.inline_done
                                && !parent.disposition
                        } else {
                            let cell = self.work.at(address.work);
                            cell.life == release.life
                                && cell.serial == life.serial
                                && cell.phase == 0
                        };
                        if valid && !parent.staged {
                            self.finish_work(position, address.work);
                        }
                    }
                }
            }
            self.emergency[index] = Some(release);
        } else {
            let bit = completion.group.token.0[2] as u8;
            self.channel_reset[index] &= !(1 << (bit + 3));
            if completion.accepted & 1 != 0 {
                let event = Event::from_input(completion.group.event(0).unwrap()).unwrap();
                let actual = self.callback.unwrap().steady_time + i64::from(completion.group.time);
                let delta = self.record(event, NONE, actual, actual);
                self.emergency_output
                    .push(delta)
                    .unwrap_or_else(|_| unreachable!("prepared emergency cell"));
            } else {
                self.channel_reset[index] |= 1 << bit;
            }
        }
    }

    fn receive(&mut self) {
        for _ in 0..512 {
            let reply = self.offer.as_mut().and_then(|o| o.endpoints.replies.pop().ok());
            let Some(reply) = reply else {
                break;
            };
            self.service_revision = self.service_revision.wrapping_add(1);
            self.reply(reply);
        }
        for _ in 0..2 {
            let reply = self
                .offer
                .as_ref()
                .and_then(|o| o.session.rows[usize::from(o.lease.slot - 1)].to_source.take());
            let Some(reply) = reply else {
                break;
            };
            self.service_revision = self.service_revision.wrapping_add(1);
            self.reply(reply);
        }
        self.retire_acknowledged();
    }
    fn reply(&mut self, reply: Reply) {
        match reply {
            Reply::OutputRetained { incarnation, epoch, cut, complete_through }
                if incarnation == self.incarnation() && epoch == self.epoch =>
            {
                self.remember_ack(cut, complete_through)
            }
            Reply::SealedStreamRetained { incarnation, epoch, generation, cut }
                if incarnation == self.incarnation()
                    && epoch == self.epoch
                    && self.sealed
                    && generation == self.sealed_generation
                    && cut == self.sequence =>
            {
                self.sealed_ack = Some(cut);
                self.acknowledged = cut;
            }
            Reply::Baseline { incarnation, epoch, transaction, cut, start, .. }
                if incarnation == self.incarnation()
                    && epoch == self.epoch
                    && self.baseline.is_some_and(|baseline| {
                        baseline.id == transaction && baseline.cut == cut
                    }) =>
            {
                self.baseline = None;
                self.baseline_acked = true;
                if cut == 0 && self.coverage.is_some_and(|coverage| coverage.start < start) {
                    self.clock.coverage = None;
                    self.coverage = None;
                    self.last_progress = None;
                    self.baseline_needed = true;
                }
            }
            Reply::Disposition { incarnation, transaction, input_cut }
                if incarnation == self.incarnation() =>
            {
                if self
                    .manifest
                    .front()
                    .is_some_and(|m| m.transaction == transaction && m.serial == input_cut)
                {
                    let manifest = self.manifest.pop().unwrap();
                    if self
                        .pending
                        .at(manifest.position)
                        .is_some_and(|p| p.serial == manifest.serial)
                    {
                        self.finish_work(manifest.position, manifest.work);
                    }
                }
            }
            _ => {}
        }
    }
    pub fn acknowledge(&mut self, cut: u64, through: i64) {
        self.remember_ack(cut, through);
        self.retire_acknowledged();
    }
    /// Complete no-more-output proof, separate from continuous clock coverage.
    pub fn acknowledge_seal(&mut self) {
        if self.sealed {
            self.sealed_ack = Some(self.sequence);
            self.acknowledged = self.sequence;
            self.retire_acknowledged();
        }
    }
    fn remember_ack(&mut self, cut: u64, through: i64) {
        if cut < self.acknowledged || cut > self.sequence || through < self.complete_through {
            return;
        }
        self.acknowledged = cut;
        self.complete_through = through;
    }
    fn retire_acknowledged(&mut self) {
        let cut = self.acknowledged;
        let through = self.complete_through;
        for _ in 0..512 {
            if self.journal.front().is_none_or(|d| d.sequence > cut) {
                break;
            }
            self.journal.pop();
            self.sent = self.sent.saturating_sub(1);
        }
        for _ in 0..128 {
            if self.emergency_output.front().is_none_or(|d| d.sequence > cut) {
                break;
            }
            self.emergency_output.pop();
            self.emergency_sent = self.emergency_sent.saturating_sub(1);
        }
        for slot in 0..64 {
            if self.emergency[slot].is_some_and(|release| {
                release.accepted.is_some_and(|delta| {
                    delta.sequence <= cut
                        && (delta.mapped && delta.actual < through
                            || self.sealed_ack.is_some_and(|sealed| delta.sequence <= sealed))
                })
            }) {
                let release = self.emergency[slot].take().unwrap();
                self.lives[usize::from(release.life)].as_mut().unwrap().refs -= 1;
                self.recycle(release.life);
            }
        }
        for slot in 0..64 {
            let index = self.reserved[slot];
            if index != NONE
                && self.lives[usize::from(index)].is_some_and(|l| {
                    l.terminal.is_some_and(|(sequence, time, mapped)| {
                        sequence <= cut
                            && (mapped && time < through
                                || self.sealed_ack.is_some_and(|sealed| sequence <= sealed))
                    })
                })
            {
                self.return_credit(index);
            }
        }
    }

    pub fn end(&mut self, _callback: api::Callback) {
        #[cfg(test)]
        self.shared.before_transfer.reach();
        self.compact();
        if self.direct.is_some() {
            self.publish_seal();
            return;
        }
        if !self.detaching {
            self.transfer();
            self.publish_control();
            self.publish_seal();
        }
        if self.offer.is_some() {
            self.attachment();
        }
        if self.session().is_some_and(|session| !session.alive.load(Ordering::Acquire)) {
            self.shared.request_main();
        }
        self.shared.extra_delay.store(self.wave_shift.max(0) as u64, Ordering::Relaxed);
    }
    fn transfer(&mut self) {
        if self.offer.is_none() {
            return;
        }
        for _ in 0..512 {
            let ordinary = self.journal.get(self.sent);
            let emergency = self
                .emergency_output
                .get(self.emergency_sent)
                .filter(|d| d.sequence > self.acknowledged);
            // Emergency records remain in their dedicated cells through ack;
            // use a source-monotonic transport cut to avoid duplicate pushes.
            let last_sent = self.last_sent_sequence();
            let emergency = emergency.filter(|d| d.sequence > last_sent);
            let next = match (ordinary, emergency) {
                (Some(a), Some(b)) => {
                    if a.sequence < b.sequence {
                        a
                    } else {
                        b
                    }
                }
                (Some(a), None) | (None, Some(a)) => a,
                (None, None) => break,
            };
            // A complete current-state transaction is ordered between <=C and
            // >C history. Keep later actual deltas in their original journal
            // until the receiver has consumed that snapshot, not merely its
            // control-slot address.
            if self.baseline.is_some_and(|baseline| next.sequence > baseline.cut) {
                break;
            }
            let offer = self.offer.as_mut().unwrap();
            if offer.endpoints.outputs.push(next).is_err() {
                break;
            }
            if ordinary.is_some_and(|a| a.sequence == next.sequence) {
                self.sent += 1;
            } else {
                self.emergency_sent += 1;
            }
            self.transfer_cut = next.sequence;
        }
    }
    fn last_sent_sequence(&self) -> u64 {
        self.transfer_cut
    }

    fn dispose_work(&mut self, position: usize, child: u16) -> bool {
        let pending = self.resolved(position, child);
        if pending.disposition || child == NONE && pending.inline_done {
            return true;
        }
        if self.offer.is_none()
            || !self.adopt_sent
            || self.offer.as_ref().is_some_and(|offer| pending.generation > offer.generation)
        {
            self.finish_work(position, child);
            return true;
        }
        if self.manifest.free() == 0 || self.next_disposition == u64::MAX {
            return false;
        }
        let transaction = self.next_disposition + 1;
        let lifetime = if pending.life == NONE {
            0
        } else {
            self.lives[usize::from(pending.life)].unwrap().serial
        };
        let offer = self.offer.as_mut().unwrap();
        let message = Intent::Disposition {
            incarnation: offer.lease.incarnation,
            transaction,
            input_cut: pending.serial,
            total: 1,
            index: 0,
            lifetime,
            canceled: true,
        };
        if offer.endpoints.intents.push(message).is_err() {
            return false;
        }
        self.next_disposition = transaction;
        self.manifest
            .push(Manifest { transaction, position, serial: pending.serial, work: child })
            .unwrap_or_else(|_| unreachable!("reserved manifest cell"));
        if child == NONE {
            let mut parent = self.pending.at(position).unwrap();
            parent.disposition = true;
            self.pending.set(position, parent);
        } else {
            let mut cell = self.work.at(child);
            cell.phase |= work::DISPOSITION;
            self.work.set(child, cell);
        }
        true
    }

    /// Only after callback join. This is deliberately not begin/end: no new
    /// coverage, output attempt, attachment, or destroyed-instance wakeup.
    pub fn service_position(&self) -> ([usize; 4], [u64; 3], i64, [bool; 3]) {
        (
            [
                self.pending.len(),
                self.journal.len() + self.emergency_output.len(),
                self.manifest.len(),
                self.held(),
            ],
            [self.transfer_cut, self.acknowledged, self.service_revision],
            self.complete_through,
            [self.offer.is_some(), self.detaching, self.sealed],
        )
    }
    pub fn retired_pump(&mut self) -> bool {
        self.visits = 0;
        self.receive();
        self.cancel_slice();
        self.compact();
        if !self.detaching {
            self.transfer();
        }
        self.publish_seal();
        if self.producer_joined && !self.joined_published && self.transfer_cut == self.sequence {
            if let Some(offer) = &self.offer {
                if self.adopt_sent
                    && offer.session.rows[usize::from(offer.lease.slot - 1)]
                        .to_hub
                        .publish(Control::ProducerJoined {
                            incarnation: offer.lease.incarnation,
                            epoch: self.epoch,
                            cut: self.sequence,
                            unknown_wire: self.joined_unknown_wire,
                        })
                        .is_ok()
                {
                    self.joined_published = true;
                    self.service_revision = self.service_revision.wrapping_add(1);
                }
            }
        }
        self.publish_output_progress();
        if let Some(offer) = &self.offer {
            let row = &offer.session.rows[usize::from(offer.lease.slot - 1)];
            if self.settled() {
                if row.hub_detached.load(Ordering::Acquire) {
                    let Some(returned) = self.shared.source.as_ref().unwrap().returns.reserve()
                    else {
                        return true;
                    };
                    row.source_detached.store(true, Ordering::Release);
                    returned.publish(SourceReturn::Returned(self.offer.take().unwrap()));
                } else if !self.detaching
                    && row
                        .to_hub
                        .publish(Control::Detach {
                            incarnation: offer.lease.incarnation,
                            epoch: self.epoch,
                            cut: self.sequence,
                        })
                        .is_ok()
                {
                    self.detaching = true;
                }
            }
        }
        self.journal.front().is_some_and(|d| d.sequence <= self.acknowledged)
            || self.stopping
            || self.cleanup_head != NONE
    }
    fn publish_control(&mut self) {
        if self.sealed {
            return;
        }
        let Some(offer) = self.offer.as_mut() else {
            return;
        };
        let row = &offer.session.rows[usize::from(offer.lease.slot - 1)];
        let Some(coverage) = self.coverage else {
            return;
        };
        if self.input_complete
            && self.input_reported != Some(coverage.through)
            && offer
                .endpoints
                .intents
                .push(Intent::Coverage {
                    incarnation: offer.lease.incarnation,
                    epoch: self.epoch,
                    coverage,
                    input_cut: self.next_event,
                })
                .is_ok()
        {
            self.input_reported = Some(coverage.through);
        }
        if !self.adopt_sent
            && row
                .to_hub
                .publish(Control::Adopt {
                    lease: offer.lease,
                    epoch: self.epoch,
                    start: coverage.start,
                })
                .is_ok()
        {
            self.adopt_sent = true;
        }
        if self.baseline_needed && self.baseline.is_none() && !row.withdrawn.load(Ordering::Acquire)
        {
            if let Some(id) = self.next_baseline.checked_add(1) {
                if let Some(frame) = self.state.baseline(
                    offer.lease.source,
                    id,
                    self.sequence,
                    coverage.through.saturating_sub(1) as f64 / self.rate,
                    coverage.start as f64 / self.rate,
                    self.participating && self.faults == 0,
                ) {
                    if row
                        .baselines
                        .publish(Baseline {
                            incarnation: offer.lease.incarnation,
                            epoch: self.epoch,
                            frame,
                            start: coverage.start,
                        })
                        .is_ok()
                    {
                        self.next_baseline = id;
                        self.baseline = Some(PendingBaseline { id, cut: self.sequence, coverage });
                        self.baseline_needed = false;
                    }
                }
            }
        }
        self.publish_output_progress();
    }

    fn publish_output_progress(&mut self) {
        if self.detaching || !self.adopt_sent {
            return;
        }
        // Advertise completed coverage and its full accepted cut before
        // transfer fills both receiver windows. The Hub grants only the
        // received timestamp prefix until the complete cut arrives.
        // A pending snapshot keeps priority: later history cannot transfer
        // until the snapshot is acknowledged.
        let report = self
            .baseline
            .map(|baseline| (baseline.coverage, baseline.cut))
            .or_else(|| self.coverage.map(|coverage| (coverage, self.sequence)));
        let Some((coverage, cut)) = report else {
            return;
        };
        if self
            .last_progress
            .is_some_and(|(through, old_cut)| through >= coverage.through && old_cut >= cut)
        {
            return;
        }
        let Some(offer) = &self.offer else {
            return;
        };
        if offer.session.rows[usize::from(offer.lease.slot - 1)]
            .to_hub
            .publish(Control::Progress {
                incarnation: offer.lease.incarnation,
                epoch: self.epoch,
                coverage,
                output_cut: cut,
            })
            .is_ok()
        {
            self.last_progress = Some((coverage.through, cut));
            self.service_revision = self.service_revision.wrapping_add(1);
        }
    }

    fn publish_seal(&mut self) {
        if self.sealed
            || self.state.pedals_held()
            || self.owed_note_off != [NONE; 64]
            || self.old_pending != 0
            || self.state.count() != 0
            || self.channel_reset != [0; 16]
            || self.permit.is_some()
            || self.baseline.is_some()
            || self.emergency.iter().flatten().any(|release| release.accepted.is_none())
        {
            return;
        }
        if let Some(session) = &self.direct {
            if session.closing.load(Ordering::Acquire) != 0 {
                self.sealed_generation = session.closing.load(Ordering::Acquire);
                self.sealed = true;
            }
            return;
        }
        let Some(offer) = &self.offer else {
            return;
        };
        let row = &offer.session.rows[usize::from(offer.lease.slot - 1)];
        let generation = offer.session.closing.load(Ordering::Acquire);
        if row.withdrawn.load(Ordering::Acquire)
            && row
                .to_hub
                .publish(Control::Seal {
                    incarnation: offer.lease.incarnation,
                    epoch: self.epoch,
                    generation,
                    cut: self.sequence,
                })
                .is_ok()
        {
            self.sealed_generation = generation;
            self.sealed = true;
        }
    }
}

const _: () = assert!(Indexed::<Pending, PENDING_EVENTS>::BACKING_CELL_BYTES <= 128);
const _: () = assert!(std::mem::align_of::<Pending>() <= 8);
const _: () = assert!(std::mem::size_of::<Option<Life>>() <= 256);
const _: () = assert!(std::mem::align_of::<Option<Life>>() <= 8);
const _: () = assert!(std::mem::size_of::<Option<Manifest>>() <= 256);
const _: () = assert!(std::mem::align_of::<Option<Manifest>>() <= 8);
const _: () = assert!(std::mem::size_of::<Option<Release>>() <= 256);

#[cfg(all(test, not(feature = "tuning-probe")))]
impl Source {
    pub fn test_rebase_output_prefix(&mut self, prefix: u64) -> Lease {
        assert_eq!(self.journal.len(), 0);
        assert_eq!(self.emergency_output.len(), 0);
        assert!(self.baseline.is_none());
        assert_eq!(self.acknowledged, self.sequence);
        assert_eq!(self.transfer_cut, self.sequence);
        self.sequence = prefix;
        self.acknowledged = prefix;
        self.transfer_cut = prefix;
        self.last_progress = None;
        self.offer.as_ref().unwrap().lease
    }
    pub fn test_repeat_emergency_output(&mut self) {
        let delta = self.emergency_output.front().unwrap();
        self.offer.as_mut().unwrap().endpoints.outputs.push(delta).unwrap();
    }
    pub fn print_test_memory_layout(&self) {
        use std::mem::{size_of, size_of_val};
        println!(
            "LEDGER source indexed [option,linked,count,backing,free_u16_capacity] {:?}",
            self.pending.test_layout()
        );
        println!(
            "LEDGER source lifetime [option,count,backing,free_u16_capacity] {:?}",
            [
                size_of::<Option<Life>>(),
                self.lives.len(),
                size_of_val(&*self.lives),
                self.free_lives.capacity()
            ]
        );
        println!(
            "LEDGER source queues [cell,count,backing] journal={:?} emergency={:?} manifest={:?}",
            self.journal.test_layout(),
            self.emergency_output.test_layout(),
            self.manifest.test_layout()
        );
        println!(
            "LEDGER source inline [owner,state,release_option,channels,work_owner] {:?}",
            [
                size_of::<Self>(),
                size_of::<State>(),
                size_of::<Option<Release>>(),
                size_of::<channel::Channels>(),
                size_of::<work::Work>()
            ]
        );
    }
}
