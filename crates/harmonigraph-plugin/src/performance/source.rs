//! Serialized ordinary performance owner. Storage is allocated before activation;
//! only actual host completions establish output facts or settle reservations.
use super::{
    clock::{Calibration, Clock, Coverage},
    event::Event,
    protocol::*,
    queue::Queue,
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

pub(super) mod channel;
mod recovery;
#[cfg(all(test, debug_assertions))]
mod replay_tests;
mod stop;
mod wave;
pub(super) mod work;

pub(super) const NONE: u16 = u16::MAX;
pub(super) const READY_QUEUED: u8 = 1;
pub(super) const ASSIGNMENT_HELD: u8 = 2;
pub(super) const TIMING_REPORTED: u8 = 4;
pub(super) const PARTIAL_ON: u8 = 8;
pub(super) const SHIFT_VALID: u8 = 16;
#[cfg(test)]
#[derive(Debug, PartialEq)]
pub struct Snapshot {
    pub captures: usize,
    pub velocity_prefix: [Option<u8>; 16],
    pub unmapped_reports: usize,
    pub pending: usize,
    pub local_pending: usize,
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
    pub acknowledged: u64,
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
pub use super::setup::TIMING_FAILURE;
pub const CLOSED: u64 = 2;
pub const OPEN: u64 = 0;
pub const BUSY: u64 = 1;
pub const GATE_FLAGS: u64 = CLOSED | BUSY;

#[derive(Clone, Copy)]
pub(super) struct Pending {
    pub(super) channel: channel::Cell,
    pub(super) serial: u64,
    pub(super) event: Event,
    pub(super) life: u16,
    pub(super) input: i64,
    pub(super) generation: u64,
    pub(super) staged: bool,
    pub(super) cleanup_queued: bool,
    pub(super) cleanup_next: u16,
    pub(super) disposition: bool,
    pub(super) work_head: u16,
    pub(super) work_tail: u16,
    pub(super) work_count: u8,
    pub(super) work_remaining: u8,
    pub(super) work_linked: u8,
    pub(super) selected: u16,
    pub(super) inline_done: bool,
}
#[derive(Clone, Copy)]
pub(super) struct ReleaseIndex {
    pub(super) parent: u16,
    pub(super) work: u16,
}
#[derive(Clone, Copy)]
pub(super) struct Life {
    pub(super) serial: u64,
    pub(super) on_serial: u64,
    pub(super) id: i32,
    pub(super) channel: u8,
    pub(super) key: u8,
    pub(super) input: i64,
    pub(super) refs: u32,
    pub(super) active: bool,
    pub(super) reserved: bool,
    pub(super) sounded: bool,
    pub(super) canceled: bool,
    pub(super) shift: Option<i64>,
    pub(super) terminal: Option<(u64, i64, bool)>,
    pub(super) release: Option<ReleaseIndex>,
    pub(super) generation: u64,
    pub(super) midi: bool,
    pub(super) adaptive: bool,
    pub(super) assignment: Assignment,
    pub(super) assignment_held: bool,
    pub(super) note_off_owed: bool,
    pub(super) sound_off_refs: u16,
    pub(super) ready_head: u16,
    pub(super) ready_tail: u16,
    pub(super) cleanup_next: u16,
    pub(super) ready_queued: bool,
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

#[derive(Clone, Copy, PartialEq, Eq)]
#[repr(u8)]
enum Adoption {
    Pending,
    Sent,
    Joined,
}

impl Adoption {
    fn sent(self) -> bool {
        self != Self::Pending
    }
}

pub struct Source {
    trace: Box<super::diagnostics::Counts>,
    #[cfg(test)]
    pub test_aggregation: bool,
    pub shared: Arc<setup::Shared>,
    pub offer: Option<SourceOffer>,
    direct: Option<Arc<SessionControl>>,
    pub state: State,
    pending: super::capture::PendingStore,
    work: work::Work,
    cleanup_head: u16,
    cleanup_tail: u16,
    lives: super::capture::Lives,
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
    intent_pushed: usize,
    cancel_cursor: Option<usize>,
    pub faults: u32,
    reset_armed: bool,
    pub participating: bool,
    timing_failed: bool,
    generation: u64,
    epoch: u64,
    baseline: Option<PendingBaseline>,
    next_baseline: u64,
    baseline_needed: bool,
    baseline_acked: bool,
    adoption: Adoption,
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
    status_query: Option<super::capture::Key>,
    committed_assignment: u64,
    recovery: recovery::Recovery,
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
    work_cleanup_head: u16,
    work_cleanup_tail: u16,
    draining_finished: bool,
    pending_cursor: Option<usize>,
    capture_cursor: Option<usize>,
    capture_offer: Option<super::capture::Token>,
    captures_outstanding: usize,
    settlement_cursor: Option<usize>,
    settled_input: u64,
    settlement_sent: u64,
    capture_published: u64,
    membership: u64,
}

impl Source {
    pub(super) fn diagnostics(&self) -> u32 {
        self.faults | if self.timing_failed { TIMING_FAILURE } else { 0 }
    }

    fn timing_failure(&mut self, life: u16) {
        let request = self.lives.local_mut(life).expect("retained timing request");
        if request.flags & TIMING_REPORTED != 0 {
            return;
        }
        request.flags |= TIMING_REPORTED;
        self.timing_failed = true;
        // Diagnostic publication is independent of the emergency fault path:
        // this original request remains eligible for one valid late assignment.
        if let Some(offer) = &self.offer {
            offer.session.rows[usize::from(offer.lease.slot - 1)]
                .faults
                .fetch_or(TIMING_FAILURE, Ordering::AcqRel);
        }
        self.shared.status.store(self.diagnostics(), Ordering::Release);
    }

    fn delay(&self) -> i64 {
        #[cfg(test)]
        if self.test_aggregation {
            return 0;
        }
        if self.shared.source.is_some() {
            DELAY
        } else {
            0
        }
    }
    #[cfg(test)]
    pub fn test_snapshot(&self) -> Snapshot {
        Snapshot {
            captures: self.captures_outstanding,
            velocity_prefix: std::array::from_fn(|channel| {
                let state = &self.state.channels()[channel];
                (state.controller_valid[1] & (1 << 24) != 0).then_some(state.controllers[88])
            }),
            unmapped_reports: (0..self.journal.len())
                .filter(|index| !self.journal.get(*index).unwrap().mapped)
                .count()
                + (0..self.emergency_output.len())
                    .filter(|index| !self.emergency_output.get(*index).unwrap().mapped)
                    .count(),
            pending: self.pending.len(),
            local_pending: (0..PENDING_EVENTS)
                .filter(|index| {
                    self.pending.at(*index).is_some() && !self.pending.local_done(*index)
                })
                .count(),
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
            acknowledged: self.acknowledged,
            transfer_cut: self.transfer_cut,
            baseline_cut: self.baseline.map(|baseline| baseline.cut),
            seal: self.sealed.then_some(self.sealed_generation),
            complete_through: self.complete_through,
        }
    }
    #[cfg(test)]
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
        let (pending, lives, work) = super::capture::storage(&shared);
        Box::new(Self {
            trace: Box::default(),
            #[cfg(test)]
            test_aggregation: false,
            shared,
            offer: None,
            direct: None,
            state: State::default(),
            pending,
            work,
            cleanup_head: NONE,
            cleanup_tail: NONE,
            lives,
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
            intent_pushed: 0,
            cancel_cursor: None,
            faults: 0,
            reset_armed: false,
            participating: true,
            timing_failed: false,
            generation: 1,
            epoch: 0,
            baseline: None,
            next_baseline: 0,
            baseline_needed: true,
            baseline_acked: false,
            adoption: Adoption::Pending,
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
            status_query: None,
            committed_assignment: 0,
            recovery: recovery::Recovery::default(),
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
            work_cleanup_head: NONE,
            work_cleanup_tail: NONE,
            draining_finished: false,
            pending_cursor: None,
            capture_cursor: None,
            capture_offer: None,
            captures_outstanding: 0,
            settlement_cursor: None,
            settled_input: 0,
            settlement_sent: 0,
            capture_published: 0,
            membership: 0,
        })
    }
    pub fn activate(&mut self, rate: f64, max_frames: u32) {
        let first = self.rate == 0.0;
        self.rate = rate;
        self.max_frames = max_frames;
        if !first {
            // Reactivation cannot install accepted setup over a still-owned lease.
            self.clock.sample_rate = rate;
            self.clock.max_frames = max_frames;
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
    pub(super) fn capture_lease(&self) -> Option<Lease> {
        self.offer.as_ref().map(|offer| offer.lease).or_else(|| {
            self.direct.as_ref().map(|session| Lease {
                session: session.runtime,
                source: SourceId::DIRECT,
                incarnation: 0,
                slot: 0,
            })
        })
    }
    fn output_clock_valid(&self) -> bool {
        self.clock.valid
    }
    pub(super) fn completed_input(&self) -> Option<(Coverage, u64)> {
        if !self.input_complete {
            return None;
        }
        let coverage = self.coverage?;
        let next = self.capture_cursor.and_then(|index| self.pending.at(index));
        // Match the tuner's exclusive prefix proof: the first untransferred
        // sample is not complete, even if earlier events at that sample moved.
        let through = if let Some(next) = next {
            self.clock.calibration.map(next.input)?.min(coverage.through)
        } else {
            coverage.through
        };
        let cut = next.map_or(self.next_event, |_| self.capture_published);
        (through > coverage.start).then_some((Coverage { start: coverage.start, through }, cut))
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
        if self.joined_unknown_wire {
            // Destruction removes the only possible owner of further physical
            // termination. Publish its exact row evidence before the joined
            // control message can wait behind a full mailbox; the Hub decides
            // local/session scope from its retained membership. Missing initial
            // controller state and factual ACK debt alone are not wire loss.
            self.fault(REFERENCE_FAULT);
        }
        self.discard_wave_prefix_pins();
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
            && self.status_query.is_none()
            && self.recovery.settled()
    }
    fn lease_settled(&self) -> bool {
        self.old_pending == 0
            && self.captures_outstanding == 0
            && self.output_settled()
            && self.status_query.is_none()
            && self.recovery.settled()
    }
    fn output_settled(&self) -> bool {
        self.held() == 0
            && !self.state.pedals_held()
            && self.owed_note_off == [NONE; 64]
            && self.journal.len() == 0
            && self.emergency_output.len() == 0
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
        let closed = row.emission_gate.load(Ordering::Acquire);
        let claimed = closed & GATE_FLAGS == CLOSED
            && row
                .emission_gate
                .compare_exchange(closed, closed | BUSY, Ordering::AcqRel, Ordering::Acquire)
                .is_ok();
        if offer.generation != bridge.generation.load(Ordering::Acquire)
            || row.withdrawn.load(Ordering::Acquire)
            || !offer.session.alive.load(Ordering::Acquire)
            || offer.session.closing.load(Ordering::Acquire) != 0
            || !claimed
            || offer.lease.incarnation != row.expected_incarnation.load(Ordering::Acquire)
        {
            if claimed {
                row.emission_gate.fetch_and(!BUSY, Ordering::Release);
            }
            return_slot.publish(SourceReturn::Returned(offer));
        } else {
            row.source_detached.store(false, Ordering::Release);
            row.emission_gate.fetch_and(!BUSY, Ordering::Release);
            self.epoch = offer.session.epoch.load(Ordering::Acquire);
            self.generation = offer.generation;
            self.old_pending = self.obligations;
            self.baseline_needed = true;
            self.baseline_acked = false;
            self.adoption = Adoption::Pending;
            self.committed_assignment = 0;
            self.recovery = recovery::Recovery::default();
            self.coverage = None;
            self.clock.coverage = None;
            self.last_progress = None;
            self.membership = 0;
            self.input_reported = None;
            let input_start_cut = self
                .capture_cursor
                .and_then(|position| self.pending.at(position))
                .map_or(self.next_event, |pending| pending.serial.saturating_sub(1));
            self.capture_published = input_start_cut;
            self.settled_input = input_start_cut;
            self.settlement_sent = input_start_cut;
            self.settlement_cursor = self.pending.front_position();
            return_slot.publish(SourceReturn::Adopted {
                generation: offer.generation,
                lease: offer.lease,
            });
            self.offer = Some(offer);
        }
        self.shared.request_main();
    }

    pub fn begin(&mut self, callback: api::Callback) {
        self.recovery.begin();
        self.intent_pushed = 0;
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
        self.drain_ready_work();
        if let Some(session) = self.session() {
            let faults = session.faults.load(Ordering::Acquire)
                | self.offer.as_ref().map_or(0, |o| {
                    session.rows[usize::from(o.lease.slot - 1)].faults.load(Ordering::Acquire)
                });
            if faults & !TIMING_FAILURE & !self.faults != 0 {
                self.fault(faults & !TIMING_FAILURE);
            }
        }
        if self.session().is_some_and(|session| !session.alive.load(Ordering::Acquire)) {
            self.fault(REFERENCE_FAULT);
        }
        if callback.input_status != api::InputStatus::Complete {
            self.fault(INPUT_FAULT);
        }
        let valid = self.clock.valid;
        let coverage = self.clock.begin(callback.steady_time, callback.frames);
        if self.clock.valid != valid {
            self.shared.publish_clock(&self.clock);
        }
        if coverage.is_none() {
            self.fault(CLOCK_FAULT);
            if let Some(session) = self.session() {
                session.faults.fetch_or(CLOCK_FAULT, Ordering::AcqRel);
            }
        }
        self.coverage = coverage;
        if self.reset_armed
            && callback.input_status == api::InputStatus::Complete
            && coverage.is_some()
            && self.recovery.settled()
            && self.output_settled()
            && self.local_cancel_cut_settled()
            && self.session().is_none_or(|session| {
                session.alive.load(Ordering::Acquire)
                    && session.epoch.load(Ordering::Acquire) == self.epoch
                    && session.faults.load(Ordering::Acquire) & !TIMING_FAILURE == 0
            })
        {
            self.reset_armed = false;
            self.faults = 0;
            self.timing_failed = false;
            self.shared.status.store(self.diagnostics(), Ordering::Release);
            self.baseline_needed = true;
        }
        // Registry offers precede the Hub's first audio callback. Its initial
        // epoch is provisional until that callback publishes actual progress.
        // Nothing from a Pending adoption may cross that clock boundary.
        let initial_wait = !self.adoption.sent()
            && self
                .offer
                .as_ref()
                .is_some_and(|offer| offer.session.hub_through.load(Ordering::Acquire) == i64::MIN);
        if !self.adoption.sent() && !initial_wait {
            if let Some(offer) = &self.offer {
                if offer.session.alive.load(Ordering::Acquire)
                    && offer.session.closing.load(Ordering::Acquire) == 0
                    && !offer.session.rows[usize::from(offer.lease.slot - 1)]
                        .withdrawn
                        .load(Ordering::Acquire)
                {
                    self.epoch = offer.session.epoch.load(Ordering::Acquire);
                }
            }
        }
        if !initial_wait
            && self.session().is_some_and(|s| s.epoch.load(Ordering::Acquire) != self.epoch)
        {
            self.fault(CLOCK_FAULT);
        }
        self.cancel_slice();
    }

    pub fn input(&mut self, input: OwnedInput) -> api::Consumption {
        if let InputValue::Parameter { value, modulation: false, .. } = input.value {
            // Tune has exactly one parameter; its original retained value is the
            // authority even if the generic parameter atomic has run ahead.
            if self.shared.source.is_some() {
                let Some(sample) = input.sample else {
                    self.fault(INPUT_FAULT);
                    return api::Consumption::Consumed;
                };
                if !self.capture_participation(value >= 0.5, Some(sample)) {
                    return api::Consumption::Pending;
                }
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
        let Some(mut event) = Event::from_input(input.value) else {
            return api::Consumption::Consumed;
        };
        if self.delay() != 0 {
            // The musical Tune owns pitch. Normalize before capture so both
            // prospective scoring and factual output see the same pitch;
            // DIRECT observation still forwards its original input.
            match &mut event {
                Event::Expression { kind: 2, value, .. } => *value = 0.0,
                Event::Midi { data, .. } if data[0] & 0xf0 == 0xe0 => {
                    data[1] = 0;
                    data[2] = 64;
                }
                _ => {}
            }
        }
        // A terminal latch rejects new performance. Essential original releases
        // can still discharge an existing physical lifetime.
        if self.faults != 0 && !event.release() {
            self.trace.input(event);
            return api::Consumption::Consumed;
        }
        let Some(raw) = input.sample else {
            self.trace.input(event);
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
            let life = self.lives.at(index).unwrap();
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
                    || self
                        .lives
                        .at(*index)
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
            self.lives.insert(
                index,
                Life {
                    serial: self.next_lifetime,
                    on_serial: self.next_event + 1,
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
                    adaptive: self.participating && self.delay() != 0,
                    assignment: Assignment::default(),
                    assignment_held: false,
                    note_off_owed: false,
                    sound_off_refs: 0,
                    ready_head: NONE,
                    ready_tail: NONE,
                    cleanup_next: NONE,
                    ready_queued: false,
                },
            );
            index
        } else if addressed && count == 1 {
            targets[0]
        } else {
            NONE
        };
        let position = self.enqueue_cell(event, life, raw, addressed && count != 1);
        self.capture_velocity_prefix(position);
        if addressed && count == 1 {
            self.capture_inline_ready(position, life);
        }
        for target in targets[..count].iter().copied() {
            let previous = self.lives.at(target).unwrap();
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
                self.lives.local_mut(target).unwrap().release =
                    Some(ReleaseIndex { parent: position as u16, work: NONE });
            }
            if attack.is_some() || event.release() || event.channel_termination() == Some(false) {
                self.lives.local_mut(target).unwrap().active = false;
                if let Some(slot) = self.active.iter_mut().find(|index| **index == target) {
                    *slot = NONE;
                }
            }
        }
        if let Some(slot) = active_slot {
            self.active[slot] = life;
            self.capture_onset(position);
        }
        if channel.is_some() {
            self.capture_channel(position);
        } else {
            self.link_channel(position);
        }
        self.pending.seal(position);
        self.remove_finished(position);
        self.trace.input(event);
        api::Consumption::Consumed
    }

    fn enqueue_cell(&mut self, event: Event, life: u16, input: i64, addressed: bool) -> usize {
        self.next_event += 1;
        let generation =
            if life == NONE { self.generation } else { self.lives.at(life).unwrap().generation };
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
                work_linked: 0,
                selected: NONE,
                inline_done: addressed,
            })
            .unwrap_or_else(|_| unreachable!("reserved original input envelope"));
        if !addressed {
            self.add_obligation(generation);
        }
        let position = self.pending.back_position().unwrap();
        if self.pending_cursor.is_none() {
            self.pending_cursor = Some(position);
        }
        if self.capture_cursor.is_none() {
            self.capture_cursor = Some(position);
        }
        if life != NONE {
            self.lives.local_mut(life).unwrap().refs += 1;
        }
        position
    }

    pub fn apply_setup(&mut self) {
        self.apply_setup_with_clock(true);
    }

    pub fn apply_setup_with_clock(&mut self, allow_clock: bool) -> Option<setup::Update> {
        self.trace.setup_wait = 0;
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
                    let sample = self.callback.and_then(|callback| {
                        callback.steady_time.checked_add(i64::from(callback.frames))
                    });
                    if !self.capture_participation(value, sample) {
                        self.trace.setup_wait = 1;
                        break;
                    }
                }
                if update.reset {
                    self.stop();
                }
                self.setup_started = update.generation;
            }
            let changes_clock =
                update.reset || update.routing.calibration() != self.clock.calibration;
            if changes_clock && !allow_clock {
                self.trace.setup_wait = 2;
                return Some(update);
            }
            if changes_clock
                && (self.held() != 0 || self.journal.len() != 0 || self.emergency_output.len() != 0)
            {
                // A new clock cannot reinterpret outstanding accepted history.
                self.shared.status.store(self.diagnostics() | CLOCK_FAULT, Ordering::Release);
                self.trace.setup_wait = 3;
                break;
            }
            if changes_clock
                && self
                    .offer
                    .as_ref()
                    .is_some_and(|offer| offer.generation < update.pairing_generation)
            {
                self.trace.setup_wait = 4;
                break;
            }
            if update.reset
                && (!update.routing.calibration().matches(self.rate, self.max_frames)
                    || !self.recovery.settled()
                    || !self.output_settled()
                    || !self.local_cancel_cut_settled())
            {
                // Explicit recovery clears local inhibition only after the
                // old cancellation/release obligations have really settled.
                // The generation guard above waits for the old lease. A new
                // lease counts post-cut input in old_pending again; requiring
                // that input to finish before clearing this fault deadlocks it.
                self.trace.setup_wait =
                    if !update.routing.calibration().matches(self.rate, self.max_frames) {
                        5
                    } else if !self.recovery.settled() {
                        6
                    } else if !self.output_settled() {
                        7
                    } else {
                        8
                    };
                break;
            }
            if changes_clock {
                self.clock = Clock::new(update.routing.calibration(), self.rate, self.max_frames);
                self.coverage = None;
                self.baseline_needed = true;
            }
            if update.reset {
                // Applying a valid calibration is not yet observed fresh
                // callback coverage. Keep the visible latch until begin proves it.
                self.reset_armed = true;
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
        self.reset_armed = update.reset;
        self.shared.status.store(self.diagnostics(), Ordering::Release);
        self.shared.applied.store(update.generation, Ordering::Release);
        self.shared.publish_clock(&self.clock);
        for slot in &mut self.setup_pending {
            if slot.as_ref().is_some_and(|slot| slot.value.generation == update.generation) {
                *slot = None;
            }
        }
    }

    fn cancel_unsounded(&mut self) {
        self.discard_wave_prefix_pins();
        self.cancel_unsounded_through(self.next_event);
    }
    fn cancel_unsounded_through(&mut self, cut: u64) {
        self.cancel_cursor = self.pending.front_position();
        self.cancel_cut = self.cancel_cut.max(cut);
        self.stopping = self.cancel_cursor.is_some_and(|position| {
            self.pending.at(position).is_some_and(|pending| pending.serial <= self.cancel_cut)
        });
        self.cancel_wave_history(cut);
    }
    fn local_cancel_cut_settled(&self) -> bool {
        !self.stopping
            && self.manifest.front().is_none_or(|manifest| manifest.serial > self.cancel_cut)
            && self.work_cleanup_head == NONE
    }
    pub fn stop(&mut self) {
        self.cancel_unsounded();
        // Credits can outlive an accepted Off until its factual ACK arrives.
        // Only actual wire state may create fresh release debt at Stop.
        if self.state.count() != 0
            || self.state.pedals_held()
            || self.owed_note_off != [NONE; 64]
            || self.state.channels().iter().any(|channel| {
                channel.controller_valid[1] & (1 << 24) != 0 && channel.controllers[88] != 0
            })
        {
            self.arm_release_debt();
        }
    }
    fn cancel_slice(&mut self) {
        if !self.stopping {
            return;
        }
        let before = (self.cancel_cursor, self.stopping);
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
                    let life = self.lives.at(cell.life).unwrap();
                    if cell.operation == work::CHANNEL
                        || !life.sounded
                        || life.terminal.is_some()
                        || pending.event.attack().is_none() && !pending.event.release()
                    {
                        if !life.sounded {
                            self.lives.local_mut(cell.life).unwrap().canceled = true;
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
                        || parent.event.attack().is_none() && !parent.event.release()
                        || self
                            .lives
                            .at(parent.life)
                            .is_some_and(|life| !life.sounded || life.terminal.is_some()))
                {
                    if parent.life != NONE && !self.lives.at(parent.life).unwrap().sounded {
                        self.lives.local_mut(parent.life).unwrap().canceled = true;
                    }
                    if !self.dispose_work(position, NONE) {
                        break;
                    }
                }
            }
            if self.pending.at(position).is_some_and(|parent| parent.serial == pending.serial) {
                self.remove_finished(position);
            }
            if self.cancel_cursor == Some(position) {
                self.cancel_cursor = self.pending.next_position(position);
            }
        }
        if (self.cancel_cursor, self.stopping) != before {
            self.service_revision = self.service_revision.wrapping_add(1);
        }
    }

    pub fn fault(&mut self, fault: u32) {
        // A newly observed failure consumes Reset authorization even when its
        // class matches the old latch. Re-reading published bits in begin does
        // not call this method unless they add evidence to this local latch.
        self.reset_armed = false;
        if let Some(offer) = &self.offer {
            offer.session.rows[usize::from(offer.lease.slot - 1)]
                .faults
                .fetch_or(fault, Ordering::AcqRel);
        } else if let Some(session) = &self.direct {
            session.faults.fetch_or(fault, Ordering::AcqRel);
        }
        if self.faults & fault == fault {
            return;
        }
        if self.faults == 0 {
            self.cancel_unsounded();
        }
        self.faults |= fault;
        self.shared.status.store(self.diagnostics(), Ordering::Release);
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
                && self
                    .lives
                    .at(life)
                    .is_some_and(|l| l.sounded && (l.terminal.is_none() || l.note_off_owed))
            {
                used_channels |= 1 << self.lives.at(life).unwrap().channel;
                self.ensure_emergency(life);
            }
        }
        for channel in 0..16 {
            let state = &self.state.channels()[channel];
            if state.controller_valid[1] & (1 << 24) != 0
                && state.controllers[88] != 0
                && self.channel_reset[channel] & 0x80 == 0
            {
                self.channel_reset[channel] |= 0x40;
            }
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
                    && self.lives.at(release.life).is_some_and(|life| life.channel == channel)
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
        self.lives.local_mut(life).unwrap().refs += 1;
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
            let life = self.lives.at(index).unwrap();
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
        // CC120 can retire the musical credit before the original physical
        // Note-Off arrives. Its independent binding is still an indexed release
        // owner even though it no longer appears in reserved[].
        for index in self.owed_note_off {
            if index == NONE {
                continue;
            }
            let life = self.lives.at(index).unwrap();
            if life.reserved {
                continue;
            }
            if !self.charge(1) {
                break;
            }
            if let Some(position) = life.release {
                self.stage_work(usize::from(position.parent), position.work, start, end, output);
            }
        }
        self.schedule_pending(start, end, output);
        self.schedule_channels(start, end, output);
        self.schedule_ready(start, end, output);
        self.compact();
    }

    fn schedule_pending(&mut self, start: i64, end: i64, output: &mut api::Output<'_>) {
        let mut next = self.pending_cursor;
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
            self.pending_cursor = next;
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
            return matches!(parent.channel.role, channel::Role::ReachedStop { .. });
        }
        if matches!(parent.channel.role, channel::Role::Header { .. }) {
            let channel = usize::from(parent.event.channel_control().unwrap());
            if self.channels.waves[channel].wire.serial != parent.serial {
                self.remove_finished(position);
                return true;
            }
            return self.stage_work(position, NONE, start, end, output);
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
        if pending.disposition
            || child == NONE
                && parent.inline_done
                && !matches!(parent.channel.role, channel::Role::Header { .. })
        {
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
        let life = (pending.life != NONE).then(|| self.lives.at(pending.life).unwrap());
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
        if pending.event.attack().is_some() && !self.assignment_ready(pending.life) {
            if pending.input.checked_add(self.delay()).is_some_and(|deadline| deadline < end) {
                self.timing_failure(pending.life);
            }
            return false;
        }
        let established = life.is_some_and(|life| life.sounded);
        if life.is_some() && !established && pending.event.attack().is_none() {
            return false;
        }
        if pending.event.attack().is_none()
            && !pending.event.release()
            && life.is_some_and(|life| life.ready_head != work::ready_reference(position, child))
        {
            return false;
        }
        if pending.event.attack().is_some() && !self.onset_wave_ready(position, start, output) {
            return false;
        }
        let shift = if established {
            life.unwrap().shift.unwrap_or(0)
        } else if let Some(channel) = pending.event.channel_control() {
            self.channels.waves[usize::from(channel)]
                .shift
                .unwrap_or(self.wave_shift.max(self.delay()))
        } else {
            self.wave_shift.max(self.delay())
        };
        let Some(mut due) = pending.input.checked_add(shift) else {
            self.fault(CLOCK_FAULT);
            return false;
        };
        due = due.max(start);
        if !established {
            if let Some(boundary) = self.recovery.boundary {
                due = due.max(boundary);
            }
        }
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
        if pending.event.attack().is_some()
            && life.is_some_and(|life| life.adaptive)
            && callback
                .steady_time
                .checked_add(i64::from(time))
                .zip(pending.input.checked_add(self.delay()))
                .is_some_and(|(actual, planned)| actual > planned)
        {
            self.timing_failure(pending.life);
        }
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
        let wire = self.assigned_event(pending);
        let prefix = self.prefix_reconciliation(pending).map(Event::input);
        let group = if pending.event.attack().is_some() && self.delay() != 0 {
            let life = life.unwrap();
            let tuning = Event::Expression {
                kind: 2,
                id: life.id,
                port: 0,
                channel: i16::from(life.channel),
                key: i16::from(life.key),
                value: life.assignment.initial_player
                    + life.assignment.correction as f64 / 100_000_000.0,
                flags: 0,
            };
            api::Group::tuned_onset(token, time, prefix, wire.input(), tuning.input())
        } else if let Some(prefix) = prefix {
            api::Group::velocity_note(token, time, prefix, wire.input())
        } else {
            api::Group::single(token, api::Lane::Normal, time, wire.input())
        };
        let Ok(group) = group else {
            self.fault(INPUT_FAULT);
            return false;
        };
        if output.stage(group).is_err() {
            return false;
        }
        let mut parent = self.pending.at(position).unwrap();
        parent.staged = true;
        parent.selected = child;
        self.pending.set(position, parent);
        true
    }

    fn assigned_event(&self, pending: Pending) -> Event {
        let mut event = pending.event;
        if let Event::Expression { kind: 2, value, .. } = &mut event {
            if let Some(life) =
                (pending.life != NONE).then(|| self.lives.at(pending.life)).flatten()
            {
                *value += life.assignment.correction as f64 / 100_000_000.0;
            }
        }
        event
    }

    fn assignment_ready(&self, life: u16) -> bool {
        self.delay() == 0
            || self.lives.at(life).is_some_and(|life| {
                life.assignment.decision != 0
                    && life.assignment.decision <= self.committed_assignment
                    && !self.recovery.inhibits(life.assignment)
                    && !self.recovery.rejects(life.assignment)
            })
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
                && row.emission_gate.load(Ordering::Acquire) & GATE_FLAGS == OPEN
        } else {
            true
        }
    }

    fn ordinary_stream_ready(&mut self) -> bool {
        // A newly adopted stream must reach its actual Hub join before controls
        // create its first accepted output. Admission can precede snapshot ack
        // when calibration places the snapshot ahead of the Hub's playhead;
        // the pending baseline still fences transfer of later accepted history.
        // Retain that fact through withdrawal so established controls remain
        // responsive. A new offer resets it; fresh attacks/setup still claim
        // current admission separately. Clock replacement also waits for the
        // next validated callback.
        if self.coverage.is_none() {
            return false;
        }
        let Some(offer) = &self.offer else { return true };
        let row = &offer.session.rows[usize::from(offer.lease.slot - 1)];
        if row.expected_incarnation.load(Ordering::Acquire) != offer.lease.incarnation
            || offer.session.epoch.load(Ordering::Acquire) != self.epoch
        {
            return false;
        }
        if self.adoption == Adoption::Sent
            && row.emission_gate.load(Ordering::Acquire) & GATE_FLAGS == OPEN
        {
            self.adoption = Adoption::Joined;
        }
        self.adoption == Adoption::Joined
    }

    pub fn prepare(&mut self, group: api::Group) -> bool {
        assert!(self.permit.is_none());
        if group.token.0[3] == wave::SETUP_TOKEN {
            return self.prepare_wave_setup(group);
        }
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
        if !self.prefix_ready(pending) {
            return false;
        }
        if group.velocity_prefix().is_none() && self.prefix_reconciliation(pending).is_some() {
            return false;
        }
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
        if (!self.output_clock_valid() || !self.ordinary_stream_ready()) && !pending.event.release()
        {
            return false;
        }
        let report_cells = self.channel_report_cells(pending)
            + usize::from(group.velocity_prefix().is_some())
            + usize::from(group.initial_tuning().is_some());
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
            if !self.admitted(pending.generation) || !self.assignment_ready(pending.life) {
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
                .filter(|index| !self.lives.at(*index).unwrap().reserved)
                .count();
            if self.held() + unreserved_offs >= 64 {
                return false;
            }
            let Some(slot) = self.reserved.iter().position(|v| *v == NONE) else {
                return false;
            };
            if let Some(offer) = &self.offer {
                let row = &offer.session.rows[usize::from(offer.lease.slot - 1)];
                let emission = self.emission(pending, row);
                if row
                    .emission_gate
                    .compare_exchange(
                        emission,
                        emission | BUSY,
                        Ordering::AcqRel,
                        Ordering::Acquire,
                    )
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
            self.lives.local_mut(pending.life).unwrap().reserved = true;
        } else if pending.life != NONE {
            let life = self.lives.at(pending.life).unwrap();
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
        if completion.group.token.0[3] == wave::SETUP_TOKEN {
            self.complete_wave_setup(completion, output);
            return;
        }
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
        let prefix = completion.group.velocity_prefix();
        if let Some(prefix) = prefix.filter(|_| completion.accepted & 1 != 0) {
            assert!(permit.is_some_and(
                |permit| permit.position == position && permit.serial == parent.serial
            ));
            let event = Event::from_input(prefix).unwrap();
            let actual = self.callback.unwrap().steady_time + i64::from(completion.group.time);
            let delta = self.record(event, NONE, pending.input, actual);
            self.journal
                .push(delta)
                .unwrap_or_else(|_| unreachable!("prepared prefix journal credit"));
        }
        let note_bit = if prefix.is_some() { 2 } else { 1 };
        if completion.accepted & note_bit != 0 {
            let permit = permit.expect("accepted output requires durable preparation");
            assert_eq!((permit.position, permit.serial), (position, parent.serial));
            let actual = self
                .callback
                .unwrap()
                .steady_time
                .checked_add(i64::from(completion.group.time))
                .unwrap();
            let player = match pending.event {
                Event::Expression { kind: 2, value, .. } => value,
                _ => 0.0,
            };
            let mut delta = self.record_player(
                self.assigned_event(pending),
                pending.life,
                pending.input,
                actual,
                player,
            );
            let partial = completion.group.initial_tuning().is_some()
                && completion.accepted & (note_bit << 1) == 0;
            delta.outcome =
                Outcome::wire(pending.life, OutputOrigin { parent: position as u16 }, partial);
            if partial {
                self.state.partial(delta.lifetime);
                self.lives.local_mut(pending.life).unwrap().flags |= PARTIAL_ON;
            }
            self.journal.push(delta).unwrap_or_else(|_| unreachable!("prepared journal credit"));
            if let Some(tuning) = completion.group.initial_tuning() {
                if completion.accepted & (note_bit << 1) != 0 {
                    let tuning = Event::from_input(tuning).unwrap();
                    let player = self.lives.at(pending.life).unwrap().assignment.initial_player;
                    let delta =
                        self.record_player(tuning, pending.life, pending.input, actual, player);
                    self.journal
                        .push(delta)
                        .unwrap_or_else(|_| unreachable!("prepared tuning journal credit"));
                } else {
                    // The accepted onset is factual even when its intended
                    // initial tuning fails. Its independent termination owner
                    // remains reserved; no tuning repair retunes that voice.
                    self.fault(OUTPUT_FAULT);
                }
            }
            self.record_channel_terminals(position as u16, pending, delta.sequence, actual);
            let waiter = match parent.channel.role {
                channel::Role::Header { first_waiter, .. } => first_waiter,
                _ => NONE,
            };
            if pending.event.attack().is_some() {
                self.accept_onset_wave(pending, actual);
            }
            if self.pending_cursor == Some(position) {
                self.pending_cursor = self.pending.next_position(position);
            }
            if matches!(parent.channel.role, channel::Role::Header { .. }) {
                let mut target = parent.work_head;
                while target != NONE {
                    let cell = self.work.at(target);
                    if self.lives.at(cell.life).is_some_and(|life| life.sounded) {
                        self.finish_work(position, target);
                    }
                    target = cell.next;
                }
                self.accept_channel_wire(position, pending);
                // Queue final replay cleanup before the completion's remaining
                // visit budget can run out. History retention remains separate.
                self.remove_finished(position);
            }
            if !parent.inline_done || child != NONE {
                self.finish_work(position, child);
            }
            if permit.gate {
                self.release_gate();
            }
            self.wake_waiters(waiter, output);
            self.wake_channel(pending.event.channel(), output);
            if pending.life != NONE {
                self.wake_life(pending.life, actual, output);
            }
            self.wake_onset(pending.event.channel(), actual, output);
            if pending.event.channel_control().is_some() {
                let callback = self.callback.unwrap();
                self.schedule_ready(
                    actual,
                    callback.steady_time + i64::from(callback.frames),
                    output,
                );
            }
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
                if prefix.is_some() && completion.accepted & 1 != 0 {
                    // Every newly accepted prefix without its consumer owns
                    // repair, even when an earlier output fault is latched.
                    self.arm_release_debt();
                }
            } else if prefix.is_none()
                && self.prefix_reconciliation(pending).is_some()
                && self.charge(1)
            {
                let callback = self.callback.unwrap();
                self.stage_work(
                    position,
                    child,
                    callback.steady_time,
                    callback.steady_time + i64::from(callback.frames),
                    output,
                );
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
        let player = match event {
            Event::Expression { kind: 2, value, .. } => value,
            _ => 0.0,
        };
        self.record_player(event, life, input, actual, player)
    }
    fn record_player(
        &mut self,
        event: Event,
        life: u16,
        input: i64,
        actual: i64,
        player: f64,
    ) -> OutputDelta {
        self.sequence += 1;
        let offset = self.clock.calibration.offset;
        let mapped = self.clock.valid
            && input.checked_add(offset).is_some()
            && actual.checked_add(offset).is_some();
        let mapped_input = if mapped { input.checked_add(offset).unwrap() } else { input };
        let mapped_actual = if mapped { actual.checked_add(offset).unwrap() } else { actual };
        let lifetime = if life == NONE { 0 } else { self.lives.at(life).unwrap().serial };
        let delta = OutputDelta {
            decision: if life == NONE {
                0
            } else {
                self.lives.at(life).unwrap().assignment.decision
            },
            player,
            incarnation: self.incarnation(),
            sequence: self.sequence,
            lifetime,
            input: mapped_input,
            actual: mapped_actual,
            epoch: self.epoch,
            mapped,
            discontinuity_generation: if mapped { 0 } else { self.generation },
            event,
            outcome: Outcome::wire(life, OutputOrigin::NONE, false),
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
                planned: (life != NONE).then(|| mapped_input.checked_add(self.delay())).flatten(),
                sample: mapped_actual,
                sample_rate: self.rate,
            }),
            provenance: PitchProvenance::AcceptedOutput,
        };
        if mapped {
            self.state.apply(event, stamp);
            if matches!(event, Event::Expression { kind: 2, .. }) && life != NONE {
                self.state.assignment(lifetime, self.lives.at(life).unwrap().assignment, player);
            }
        } else {
            assert!(self.state.apply_unmapped_terminal(event, lifetime));
        }
        if life != NONE {
            let value = self.lives.local_mut(life).unwrap();
            if event.attack().is_some() {
                value.sounded = true;
                let shift = actual.checked_sub(input);
                value.shift = shift.unwrap_or_default();
                value.flags =
                    (value.flags & !SHIFT_VALID) | (u8::from(shift.is_some()) * SHIFT_VALID);
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
        let pitch = self.state.voice(lifetime).map(|voice| {
            (
                voice.note,
                voice.pitch_microcents,
                voice.player_tuning,
                voice.frozen_offset_microcents,
            )
        });
        self.trace.output(event, pitch, self.source_id().0);
        delta
    }
    fn release_gate(&self) {
        if let Some(offer) = &self.offer {
            let row = &offer.session.rows[usize::from(offer.lease.slot - 1)];
            // A Hub close can race any permitted group. All its actual facts
            // are durable before BUSY is released; CLOSED and generation stay.
            assert_ne!(row.emission_gate.fetch_and(!BUSY, Ordering::Release) & BUSY, 0);
        }
    }
    fn emission(&self, pending: Pending, row: &SourceControl) -> u64 {
        if self.delay() != 0
            && pending.life != NONE
            && self.lives.at(pending.life).is_some_and(|life| !life.sounded)
        {
            self.lives.at(pending.life).unwrap().assignment.emission
        } else {
            row.emission_gate.load(Ordering::Acquire) & !GATE_FLAGS
        }
    }
    fn return_credit(&mut self, index: u16) {
        let value = self.lives.local_mut(index).unwrap();
        assert!(value.reserved);
        value.reserved = false;
        let previous = self.session().unwrap().credits.fetch_sub(1, Ordering::AcqRel);
        assert!(previous != 0);
        *self.reserved.iter_mut().find(|v| **v == index).unwrap() = NONE;
        self.recycle(index);
    }
    fn recycle(&mut self, index: u16) {
        if self.lives.at(index).is_some_and(|l| {
            !l.active
                && !l.reserved
                && l.refs == 0
                && !l.ready_queued
                && !l.assignment_held
                && !self.recovery.holds_life(l.serial)
        }) {
            self.lives.remove(index);
            self.free_lives.push(index);
            if let Some(slot) = self.active.iter_mut().find(|i| **i == index) {
                *slot = NONE;
            }
        }
    }
    fn compact(&mut self) {
        self.drain_finished();
        if self.obligations == 0
            && self.state.count() == 0
            && self.held() == 0
            && self.journal.len() == 0
            && self.emergency_output.len() == 0
            && self.manifest.len() == 0
            && self.permit.is_none()
            && self.baseline.is_none()
            && self.baseline_acked
            && self.owed_note_off == [NONE; 64]
            && self.channel_reset == [0; 16]
            && self.state.channels().iter().zip(&self.channels.waves).all(|(channel, wave)| {
                wave.shift.is_none()
                    || [64, 66, 69].into_iter().all(|cc| {
                        channel.controller_valid[cc / 64] & (1 << (cc % 64)) != 0
                            && channel.controllers[cc] < 64
                    })
            })
        {
            self.wave_shift = 0;
            for wave in &mut self.channels.waves {
                wave.shift = None;
            }
        }
    }

    fn schedule_emergency(&mut self, output: &mut api::Output<'_>) {
        if self.sealed {
            return;
        }
        // A failed consumer can leave its accepted CC88 waiting at the receiver.
        // Repair precedes any emergency raw MIDI Off that could consume it.
        // 64 voice +48 pedal +16 prefix attempts fit the reserved128 exactly.
        for channel in 0..16 {
            if self.channel_reset[channel] & 0x40 == 0 {
                continue;
            }
            let event = Event::Midi { port: 0, data: [0xb0 | channel as u8, 88, 0], flags: 0 };
            let group = api::Group::single(
                api::Token([0, channel as u64, 3, 2]),
                api::Lane::Emergency,
                output.cursor().max(self.stops.emergency_start),
                event.input(),
            )
            .unwrap();
            if output.stage(group).is_err() {
                return;
            }
            self.channel_reset[channel] = (self.channel_reset[channel] & !0x40) | 0x80;
        }
        for index in 0..64 {
            let Some(mut release) = self.emergency[index] else {
                continue;
            };
            if release.staged || release.accepted.is_some() {
                continue;
            }
            let life = self.lives.at(release.life).unwrap();
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
        if matches!(group.event(0), Some(InputValue::Midi { data: [status, _, _], .. })
            if matches!(status & 0xf0, 0x80 | 0x90) && self.channel_reset[usize::from(status & 15)] & 0xc0 != 0)
        {
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
        if completion.accepted == 0
            && (completion.attempted != 0
                || completion.disposition == api::Disposition::MissingOutput)
        {
            self.fault(OUTPUT_FAULT);
        }
        let permit = self.permit.take();
        let index = completion.group.token.0[1] as usize;
        if completion.group.token.0[3] == 1 {
            let Some(mut release) = self.emergency[index] else {
                return;
            };
            release.staged = false;
            if completion.accepted & 1 != 0 {
                assert!(permit.is_some_and(|p| p.emergency));
                let life = self.lives.at(release.life).unwrap();
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
            let (pending_bit, staged_bit) =
                if bit == 3 { (0x40, 0x80) } else { (1 << bit, 1 << (bit + 3)) };
            self.channel_reset[index] &= !staged_bit;
            if completion.accepted & 1 != 0 {
                let event = Event::from_input(completion.group.event(0).unwrap()).unwrap();
                let actual = self.callback.unwrap().steady_time + i64::from(completion.group.time);
                let delta = self.record(event, NONE, actual, actual);
                if bit == 3 {
                    self.accept_prefix_neutralization(index);
                } else {
                    self.accept_wave_neutralization(index, bit);
                }
                self.emergency_output
                    .push(delta)
                    .unwrap_or_else(|_| unreachable!("prepared emergency cell"));
            } else {
                self.channel_reset[index] |= pending_bit;
            }
        }
    }

    fn receive(&mut self) {
        let reply = self.offer.as_ref().and_then(|offer| {
            offer.session.rows[usize::from(offer.lease.slot - 1)].to_source.take_repair_if(
                |reply| match reply {
                    Reply::CaptureStatusQuery(_) => self.status_query.is_none(),
                    Reply::Fence(fence) => self.recovery.can_observe(fence),
                    _ => true,
                },
            )
        });
        if let Some(reply) = reply {
            self.service_revision = self.service_revision.wrapping_add(1);
            self.reply(reply);
        }
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
        self.publish_capture_status();
    }
    /// The pending owner stores only a key. Taking it frees the query cell even
    /// when a crossed disposition owns the response cell; its ACK can now pass.
    fn publish_capture_status(&mut self) {
        if self.recovery.needs_ack() {
            return;
        }
        let Some(key) = self.status_query else { return };
        let Some(offer) = &self.offer else { return };
        let session = offer.session.clone();
        let row = &session.rows[usize::from(offer.lease.slot - 1)];
        let Some(reply) = row.to_hub.reserve_repair() else { return };
        let Some(status) = self.copy_capture_status(key) else { return };
        reply.publish(Control::CaptureStatus { key, status });
        self.status_query = None;
        self.service_revision = self.service_revision.wrapping_add(1);
    }
    /// Outer None means this callback's work grant is spent; inner None is an
    /// invalid permission. DIRECT uses the same Source-owned snapshot operation.
    pub(super) fn copy_capture_status(
        &mut self,
        key: super::capture::Key,
    ) -> Option<Option<CaptureStatus>> {
        let valid = self
            .capture_lease()
            .is_some_and(|lease| self.pending.owns_publication(key, lease, self.epoch));
        Some(if valid {
            let parent = self.pending.at(usize::from(key.position)).unwrap();
            if !self.charge(1 + usize::from(parent.work_count)) {
                return None;
            }
            let mut work_done = 0;
            let mut next = parent.work_head;
            for bit in 0..parent.work_count {
                let work = self.work.at(next);
                assert_eq!(work.parent, key.position);
                if work.phase & work::DONE != 0 {
                    work_done |= 1 << bit;
                }
                next = work.next;
            }
            assert_eq!(next, NONE);
            Some(CaptureStatus {
                output_cut: self.sequence,
                work_done,
                inline_done: parent.inline_done && !parent.staged,
            })
        } else {
            None
        })
    }
    fn reply(&mut self, reply: Reply) {
        match reply {
            Reply::Fence(fence) => self.observe_fence(fence),
            Reply::RecoveryComplete { fence, generation, boundary } => {
                self.complete_recovery(fence, generation, boundary)
            }
            Reply::InventoryComplete { fence, input_cut, total, chunks } => {
                self.acknowledge_inventory(fence, input_cut, total, chunks)
            }
            Reply::CohortCommitted { lease, epoch, through }
                if self.capture_lease() == Some(lease) && self.epoch == epoch =>
            {
                self.committed_assignment = self.committed_assignment.max(through);
            }
            Reply::CaptureStatusQuery(key) if self.status_query.is_none() => {
                self.status_query = Some(key);
            }
            Reply::PlanRetired { incarnation, epoch, life, lifetime, decision }
                if incarnation == self.incarnation()
                    && epoch == self.epoch
                    && life < LIFETIMES as u16 =>
            {
                if self.lives.at(life).is_some_and(|request| {
                    request.serial == lifetime
                        && request.assignment.decision == decision
                        && request.assignment_held
                }) {
                    self.lives.local_mut(life).unwrap().flags &= !ASSIGNMENT_HELD;
                    self.recycle(life);
                }
            }
            Reply::Assignment { key, life, lifetime, binding }
                if self.capture_lease() == Some(key.lease)
                    && self.epoch == key.epoch
                    && self.pending.owns_publication(key, key.lease, self.epoch)
                    && !self.recovery.rejects(binding)
                    && binding.decision != 0 =>
            {
                if let Some(original) = self.pending.at(usize::from(key.position)).filter(|p| {
                    p.serial == key.serial
                        && p.life == life
                        && p.event.attack().is_some()
                        && !p.staged
                }) {
                    if self.lives.at(life).is_some_and(|request| {
                        request.serial == lifetime
                            && !request.sounded
                            && !request.canceled
                            && original.serial > self.cancel_cut
                            && request.assignment.decision < binding.decision
                    }) {
                        self.lives.local_mut(life).unwrap().assignment = binding;
                        self.lives.local_mut(life).unwrap().flags |= ASSIGNMENT_HELD;
                        self.trace.assignments = self.trace.assignments.saturating_add(1);
                        self.trace.decision = binding.decision;
                        self.trace.correction = binding.correction;
                    }
                }
            }
            Reply::CaptureRetired(key) => {
                if let Some(lease) = self.capture_lease() {
                    if let Some(position) = self.pending.retire(key, lease, self.epoch) {
                        self.captures_outstanding -= 1;
                        self.remove_finished(position);
                    }
                }
            }
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
            Reply::Baseline { incarnation, epoch, transaction, cut, start, membership }
                if incarnation == self.incarnation()
                    && epoch == self.epoch
                    && self.baseline.is_some_and(|baseline| {
                        baseline.id == transaction && baseline.cut == cut
                    }) =>
            {
                let retry = cut == 0 && self.baseline.unwrap().coverage.start < start;
                self.baseline = None;
                // The Hub can acknowledge an obsolete empty snapshot solely
                // to move its join floor. That reply does not admit this stream.
                self.baseline_acked = !retry;
                if !retry {
                    self.adoption = Adoption::Joined;
                    if self.membership != membership {
                        self.input_reported = None;
                    }
                    self.membership = membership;
                }
                if retry {
                    self.clock.coverage = None;
                    self.coverage = None;
                    self.last_progress = None;
                    self.input_reported = None;
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
                self.lives.local_mut(release.life).unwrap().refs -= 1;
                self.recycle(release.life);
            }
        }
        for slot in 0..64 {
            let index = self.reserved[slot];
            if index != NONE
                && self.lives.at(index).is_some_and(|l| {
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

    pub fn end(&mut self, callback: api::Callback) {
        #[cfg(test)]
        self.shared.before_transfer.reach();
        self.compact();
        self.publish_revoke_ack();
        self.publish_inventory();
        self.cleanup_recovery();
        self.shared.status.store(self.diagnostics(), Ordering::Release);
        self.shared
            .extra_delay
            .store(self.wave_shift.saturating_sub(self.delay()).max(0) as u64, Ordering::Relaxed);
        if self.direct.is_some() {
            self.publish_seal();
            return;
        }
        if !self.detaching {
            self.transfer();
            // A fresh Progress can occupy the only ordinary control cell on
            // every callback. Once eligible, the final Seal and Detach must
            // precede replaceable coverage or a valid Reset never settles.
            self.publish_seal();
        }
        if self.offer.is_some() {
            self.attachment();
        }
        if !self.detaching {
            self.publish_control();
        }
        if self.session().is_some_and(|session| !session.alive.load(Ordering::Acquire)) {
            self.shared.request_main();
        }
        if self.shared.source.is_some() && self.trace.due(callback.frames, self.rate) {
            self.publish_diagnostics(callback);
            self.shared.request_main();
        }
    }
    pub(super) fn publish_diagnostics(&self, callback: api::Callback) {
        let lease = self.capture_lease();
        let session = self.session();
        let row =
            self.offer.as_ref().map(|offer| &offer.session.rows[usize::from(offer.lease.slot - 1)]);
        let position = self.pending.front_position();
        let retained = position.is_some_and(|position| self.pending.local_done(position));
        let head = position.filter(|_| !retained).and_then(|position| self.pending.at(position));
        // Independent gate facts for the oldest retained input, not permission
        // to bypass any gate and not a claim that later inputs share its wait.
        // Completed envelopes can remain for a remote ACK after channel/prefix
        // owners were released. Only inspect wire gates on still-local work.
        let head_wait = head.map_or(i64::from(retained) << 7, |pending| {
            i64::from(!self.output_clock_valid())
                | (i64::from(!self.admitted(pending.generation)) << 1)
                | (i64::from(pending.life != NONE && !self.assignment_ready(pending.life)) << 2)
                | (i64::from(pending.serial <= self.cancel_cut) << 3)
                | (i64::from(self.prefix_reconciliation(pending).is_some()) << 4)
                | (i64::from(!self.prefix_ready(pending)) << 5)
                | (i64::from(self.faults != 0) << 6)
        });
        self.shared.diagnostics.source.publish([
            session.map_or(0, |s| s.runtime) as i64,
            self.source_id().0 as i64,
            lease.map_or(-1, |lease| i64::from(lease.slot)),
            self.incarnation() as i64,
            self.epoch as i64,
            self.setup_started as i64,
            self.trace.setup_wait,
            i64::from(self.output_clock_valid()),
            match self.adoption {
                Adoption::Pending => 0,
                Adoption::Sent => 1,
                Adoption::Joined => 2,
            },
            row.map_or(-1, |row| row.emission_gate.load(Ordering::Acquire) as i64),
            row.map_or(0, |row| i64::from(row.withdrawn.load(Ordering::Acquire))),
            session.map_or(0, |s| s.closing.load(Ordering::Acquire) as i64),
            i64::from(self.participating),
            self.held() as i64,
            self.pending.len() as i64,
            self.old_pending as i64,
            self.captures_outstanding as i64,
            self.journal.len() as i64,
            self.emergency_output.len() as i64,
            self.baseline.map_or(-1, |b| b.cut as i64),
            i64::from(self.baseline_acked),
            self.recovery.diagnostic_state(),
            i64::from(self.output_settled()),
            i64::from(self.local_cancel_cut_settled()),
            i64::from(self.diagnostics()),
            self.wave_shift.saturating_sub(self.delay()).max(0),
            self.trace.input_on as i64,
            self.trace.input_off as i64,
            self.trace.last_input_key.map_or(-1, i64::from),
            self.trace.output_on as i64,
            self.trace.output_off as i64,
            self.trace.last_output_key.map_or(-1, i64::from),
            self.trace.last_output_pitch,
            self.trace.assignments as i64,
            self.trace.decision as i64,
            i64::from(self.trace.correction),
            self.committed_assignment as i64,
            self.sequence as i64,
            self.transfer_cut as i64,
            self.acknowledged as i64,
            callback.steady_time.saturating_add(i64::from(callback.frames)),
            self.coverage.map_or(i64::MIN, |c| c.through),
            self.complete_through,
            self.next_event as i64,
            self.cancel_cut as i64,
            head_wait,
            i64::from(self.held() != 0)
                | (i64::from(self.state.pedals_held()) << 1)
                | (i64::from(self.owed_note_off != [NONE; 64]) << 2)
                | (i64::from(self.journal.len() != 0) << 3)
                | (i64::from(self.emergency_output.len() != 0) << 4)
                | (i64::from(self.permit.is_some()) << 5)
                | (i64::from(self.manifest.len() != 0) << 6)
                | (i64::from(self.emergency.iter().any(Option::is_some)) << 7)
                | (i64::from(self.channel_reset != [0; 16]) << 8)
                | (i64::from(self.baseline.is_some()) << 9),
            i64::from(self.status_query.is_some()),
            self.trace.last_output_player,
            self.trace.last_output_correction,
        ]);
    }
    fn transfer(&mut self) {
        if self.offer.is_none() {
            return;
        }
        // A previous full 512-push slice may have left its strict input prefix
        // unpublished. Represent that boundary before filling another window.
        self.publish_input_prefix();
        self.transfer_captures();
        self.transfer_input_settlement();
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
    fn next_capture(&mut self) -> Option<super::capture::Token> {
        if let Some(token) = self.capture_offer.take() {
            return Some(token);
        }
        let position = self.capture_cursor?;
        let lease = self.capture_lease()?;
        // Post-reset input belongs to the next lease. Publishing it into the
        // closing session pins captures that cannot be sequenced or detached.
        if self.session().is_some_and(|session| session.closing.load(Ordering::Acquire) != 0)
            && self.pending.at(position)?.generation > self.lease_generation()?
        {
            return None;
        }
        let offset = self.clock.calibration.offset;
        let Some(token) = self.pending.offer(position, lease, self.epoch, offset) else {
            self.fault(CLOCK_FAULT);
            return None;
        };
        self.captures_outstanding += 1;
        Some(token)
    }
    fn transfer_captures(&mut self) {
        if !self.adoption.sent() {
            return;
        }
        for _ in 0..512 {
            if self.intent_pushed == 512 {
                break;
            }
            let Some(token) = self.next_capture() else {
                break;
            };
            let position = token.key.position as usize;
            let serial = token.key.serial;
            match self.push_intent(Intent::Capture(token)) {
                Ok(()) => {
                    self.capture_published = serial;
                    self.capture_cursor = self.pending.next_position(position);
                    self.service_revision = self.service_revision.wrapping_add(1);
                }
                Err(Intent::Capture(token)) => {
                    self.capture_offer = Some(token);
                    break;
                }
                Err(_) => unreachable!(),
            }
        }
    }
    fn transfer_input_settlement(&mut self) {
        if self.delay() == 0 {
            return;
        }
        if self.settlement_cursor.is_none() {
            self.settlement_cursor = self.pending.front_position();
        }
        for _ in 0..512 {
            let Some(position) = self.settlement_cursor else { break };
            if !self.charge(1) {
                break;
            }
            let pending = self.pending.at(position).unwrap();
            if pending.serial > self.settled_input {
                if !pending.inline_done
                    || pending.work_remaining != 0
                    || pending.work_linked != 0
                    || pending.staged
                {
                    break;
                }
                self.settled_input = pending.serial;
            }
            self.settlement_cursor = self.pending.next_position(position);
        }
        // New-session input can wait at the capture cursor while the old
        // session still needs this finite completed prefix to release readers.
        if self.settled_input == self.settlement_sent
            || self.capture_cursor.is_some_and(|position| {
                self.pending
                    .at(position)
                    .is_some_and(|pending| pending.serial <= self.settled_input)
            })
            || self.capture_offer.is_some()
        {
            return;
        }
        if self
            .push_intent(Intent::InputSettled {
                incarnation: self.incarnation(),
                epoch: self.epoch,
                input_cut: self.settled_input,
                output_cut: self.sequence,
            })
            .is_ok()
        {
            self.settlement_sent = self.settled_input;
        }
    }
    pub(super) fn take_direct_capture(&mut self) -> Option<super::capture::Token> {
        assert!(self.direct.is_some());
        let token = self.next_capture()?;
        self.capture_published = token.key.serial;
        self.capture_cursor = self.pending.next_position(token.key.position as usize);
        self.service_revision = self.service_revision.wrapping_add(1);
        Some(token)
    }
    pub(super) fn direct_capture_units(&self) -> Option<usize> {
        let position = self.capture_cursor?;
        Some(1 + usize::from(self.pending.at(position)?.work_count))
    }
    pub(super) fn retire_direct_capture(&mut self, retirement: super::capture::Retirement) {
        assert!(self.direct.is_some());
        self.reply(Reply::CaptureRetired(retirement.key));
    }
    fn last_sent_sequence(&self) -> u64 {
        self.transfer_cut
    }

    /// All intent kinds spend the same enclosing-callback grant. A refusal
    /// returns the exact caller-owned value without advancing its cursor.
    fn push_intent(&mut self, intent: Intent) -> Result<(), Intent> {
        if self.intent_pushed == 512 {
            return Err(intent);
        }
        let Some(offer) = &mut self.offer else {
            return Err(intent);
        };
        match offer.endpoints.intents.push(intent) {
            Ok(()) => {
                self.intent_pushed += 1;
                Ok(())
            }
            Err(rtrb::PushError::Full(intent)) => Err(intent),
        }
    }

    fn dispose_work(&mut self, position: usize, child: u16) -> bool {
        let pending = self.resolved(position, child);
        if pending.disposition || child == NONE && pending.inline_done {
            return true;
        }
        if self.offer.is_none()
            || !self.adoption.sent()
            || self.offer.as_ref().is_some_and(|offer| pending.generation > offer.generation)
        {
            self.finish_work(position, child);
            return true;
        }
        if self.manifest.free() == 0
            || self.next_disposition == u64::MAX
            || self.intent_pushed == 512
        {
            return false;
        }
        let transaction = self.next_disposition + 1;
        let lifetime =
            if pending.life == NONE { 0 } else { self.lives.at(pending.life).unwrap().serial };
        let offer = self.offer.as_mut().unwrap();
        let message = Control::Disposition {
            incarnation: offer.lease.incarnation,
            epoch: self.epoch,
            transaction,
            input_cut: pending.serial,
            lifetime,
            request: pending.life,
            original_on: child == NONE && pending.event.attack().is_some(),
        };
        let Some(cell) =
            offer.session.rows[usize::from(offer.lease.slot - 1)].to_hub.reserve_repair()
        else {
            return false;
        };
        cell.publish(message);
        self.intent_pushed += 1;
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
        self.recovery.begin();
        self.intent_pushed = 0;
        self.visits = 0;
        self.receive();
        self.cancel_slice();
        self.drain_ready_work();
        self.compact();
        if self.producer_joined {
            self.publish_revoke_ack();
            self.publish_inventory();
        }
        self.cleanup_recovery();
        if !self.detaching {
            self.transfer();
        }
        self.publish_seal();
        if self.producer_joined && !self.joined_published && self.transfer_cut == self.sequence {
            if let Some(offer) = &self.offer {
                if self.adoption.sent()
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
            || !self.recovery.settled()
    }
    fn initial_enrollment_ready(&self) -> bool {
        self.adoption.sent()
            || self.offer.as_ref().is_none_or(|offer| {
                // Recheck at publication: the Hub can finish its first callback
                // between our begin and end. A newly ready epoch waits for begin.
                offer.session.hub_through.load(Ordering::Acquire) != i64::MIN
                    && offer.session.epoch.load(Ordering::Acquire) == self.epoch
                    && offer.session.alive.load(Ordering::Acquire)
                    && offer.session.closing.load(Ordering::Acquire) == 0
                    && !offer.session.rows[usize::from(offer.lease.slot - 1)]
                        .withdrawn
                        .load(Ordering::Acquire)
            })
    }
    fn publish_control(&mut self) {
        if self.sealed || !self.initial_enrollment_ready() {
            return;
        }
        let Some(lease) = self.offer.as_ref().map(|offer| offer.lease) else {
            return;
        };
        let Some(coverage) = self.coverage else {
            return;
        };
        self.publish_input_prefix();
        let (report, output_cut) = self.output_report().unwrap();
        let input_start_cut = self
            .capture_cursor
            .and_then(|index| self.pending.at(index))
            .map_or(self.next_event, |pending| pending.serial - 1);
        let offer = self.offer.as_mut().unwrap();
        let row = &offer.session.rows[usize::from(lease.slot - 1)];
        if !self.adoption.sent()
            && row
                .to_hub
                .publish(Control::Adopt {
                    lease: offer.lease,
                    epoch: self.epoch,
                    coverage: report,
                    output_cut,
                    input_start_cut,
                })
                .is_ok()
        {
            self.adoption = Adoption::Sent;
        }
        if self.adoption.sent()
            && self.baseline_needed
            && self.baseline.is_none()
            && !row.withdrawn.load(Ordering::Acquire)
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
    fn publish_input_prefix(&mut self) {
        if !self.input_complete || !self.initial_enrollment_ready() {
            return;
        }
        let Some(coverage) = self.coverage else { return };
        let Some(lease) = self.capture_lease().filter(|lease| lease.slot != 0) else { return };
        let next = self.capture_cursor.and_then(|index| self.pending.at(index));
        let through = if self.delay() == 0 && next.is_some() {
            return;
        } else if let Some(next) = next {
            let Some(sample) = self.clock.calibration.map(next.input) else { return };
            sample.min(coverage.through)
        } else {
            coverage.through
        };
        if through <= coverage.start || self.input_reported.is_some_and(|old| old >= through) {
            return;
        }
        let input_cut = next.map_or(self.next_event, |_| self.capture_published);
        if self
            .push_intent(Intent::Coverage {
                incarnation: lease.incarnation,
                epoch: self.epoch,
                membership: self.membership,
                coverage: Coverage { start: coverage.start, through },
                input_cut,
            })
            .is_ok()
        {
            self.input_reported = Some(through);
        }
    }

    fn publish_output_progress(&mut self) {
        if self.detaching || !self.adoption.sent() {
            return;
        }
        // Advertise completed coverage and its full accepted cut before
        // transfer fills both receiver windows. The Hub grants only the
        // received timestamp prefix until the complete cut arrives.
        // A pending snapshot keeps priority: later history cannot transfer
        // until the snapshot is acknowledged.
        let Some((coverage, cut)) = self.output_report() else {
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
    fn output_report(&self) -> Option<(Coverage, u64)> {
        self.baseline
            .map(|baseline| (baseline.coverage, baseline.cut))
            .or_else(|| self.coverage.map(|coverage| (coverage, self.sequence)))
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
            && self.coverage.is_some()
            && self.last_progress.is_none_or(|(_, cut)| cut < self.sequence)
        {
            // The receiver may need the final accepted cut's continuous
            // timestamp prefix to drain a full output window before Seal.
            // Publish this finite cut once; later coverage alone must not
            // regain priority and starve the final seal on every callback.
            self.publish_output_progress();
            return;
        }
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

const _: () = assert!(super::capture::PendingStore::BACKING_CELL_BYTES <= 128);
const _: () = assert!(std::mem::align_of::<Pending>() <= 8);
const _: () = assert!(std::mem::size_of::<Option<Life>>() <= 256);
const _: () = assert!(std::mem::align_of::<Option<Life>>() <= 8);
const _: () = assert!(std::mem::size_of::<Option<Manifest>>() <= 256);
const _: () = assert!(std::mem::align_of::<Option<Manifest>>() <= 8);
const _: () = assert!(std::mem::size_of::<Option<Release>>() <= 256);
// The ledger charges this measured owner including test-support padding.
const _: () = assert!(std::mem::size_of::<Source>() <= 31288 + 64 * 8);

#[cfg(test)]
impl Source {
    pub fn test_stream_status(&self) -> (Option<Lease>, bool, bool, Option<Coverage>) {
        (
            self.offer.as_ref().map(|offer| offer.lease),
            self.adoption.sent(),
            self.baseline_acked,
            self.coverage,
        )
    }
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
        use std::mem::size_of;
        println!(
            "LEDGER source indexed [option,linked,count,backing,free_u16_capacity] {:?}",
            self.pending.test_layout()
        );
        println!(
            "LEDGER source lifetime [option,count,backing,free_u16_capacity] {:?}",
            [
                self.lives.test_layout()[0],
                self.lives.test_layout()[1],
                self.lives.test_layout()[2],
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
        println!(
            "LEDGER wave [wave,stops,prefix] {:?}",
            [size_of::<wave::Wave>(), size_of::<stop::Stops>(), size_of::<wave::Prefix>()]
        );
    }
}

#[cfg(test)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) struct CaptureSnapshot {
    pub adaptive: bool,
    pub position: u16,
    pub life: u16,
    pub life_serial: u64,
    pub work_head: u16,
    pub work_count: u8,
    pub local_done: bool,
    pub remote_pending: bool,
}
#[cfg(test)]
impl Source {
    pub(super) fn test_assignment(&self, life: u16) -> Option<Assignment> {
        self.lives.at(life).map(|life| life.assignment)
    }
    pub(super) fn test_capture(&self, serial: u64) -> Option<CaptureSnapshot> {
        let mut position = self.pending.front_position();
        while let Some(index) = position {
            let pending = self.pending.at(index).unwrap();
            position = self.pending.next_position(index);
            if pending.serial != serial {
                continue;
            }
            return Some(CaptureSnapshot {
                adaptive: self.lives.at(pending.life).is_some_and(|life| life.adaptive),
                position: index as u16,
                life: pending.life,
                life_serial: self.lives.at(pending.life).map_or(0, |life| life.serial),
                work_head: pending.work_head,
                work_count: pending.work_count,
                local_done: pending.inline_done
                    && pending.work_remaining == 0
                    && pending.work_linked == 0,
                remote_pending: self.pending.remote_pending(index),
            });
        }
        None
    }
    pub(super) fn test_cancel_before_receive(&mut self) {
        self.visits = 0;
        self.stop();
        self.cancel_slice();
    }
    pub(super) fn test_reset_progress(&self) -> String {
        format!("armed={} pending={:?} generation={} applied={} setup={} offer={:?} detaching={} settled={} recovery={} lease={} old={} captures={} query={:?} sealed={}", self.reset_armed,
            self.setup_pending.each_ref().map(|v| v.as_ref().map(|v| v.value)), self.generation,
            self.shared.applied.load(Ordering::Acquire), self.setup_started,
            self.offer.as_ref().map(|o| (o.generation, o.lease)), self.detaching,
            self.output_settled(), self.recovery.settled(), self.lease_settled(), self.old_pending, self.captures_outstanding, self.status_query, self.sealed)
    }
    pub(super) fn test_reset_armed(&self) -> bool {
        self.reset_armed
    }
}
