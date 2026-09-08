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
use nice_plug::wrapper::hash_param_id;
use std::sync::atomic::Ordering;
use std::sync::Arc;

pub(super) mod channel;
#[cfg(all(test, debug_assertions))]
mod replay_tests;
mod stop;
mod wave;
pub(super) mod work;

pub(super) const NONE: u16 = u16::MAX;
/// Records one input event can address: at most every held note plus the
/// controller or onset itself. Copies are emitted one per addressed target.
pub(super) const CAPTURE_GROUP: usize = 68;
/// The per-channel wire state a reset may owe: sustain, sostenuto and soft
/// neutralized, and the pitch bend recentered. `channel_reset` holds one
/// pending bit and one staged bit for each, so this is half of a `u8`.
pub(super) const CHANNEL_RESETS: usize = 4;
/// The recentering slot. Unlike the three pedals it is never armed by
/// `arm_release_debt`, because ending a phrase does not change who owns
/// pitch; only a participation toggle does.
const PITCH_RESET: usize = 3;
/// The 14-bit MIDI pitch bend that means no bend.
const BEND_CENTER: u16 = 0x2000;
#[cfg(test)]
#[derive(Debug, PartialEq)]
pub struct Snapshot {
    pub captures: usize,
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
    /// This request has already been counted once against the deadline gauge.
    pub(super) timing_reported: bool,
}
/// What one staging visit produced: nothing to do, a refusal to retry, or the
/// group it would emit. A replacement plans both of its groups before either
/// is admitted, which is why building one is separate from staging it.
// A `Group` is what every staging path already moves by value on the audio
// thread, and boxing the variant to even the enum out would allocate there.
#[allow(clippy::large_enum_variant)]
enum Plan {
    Skip,
    Refused,
    Ready(api::Group),
}
#[derive(Clone, Copy)]
struct Permit {
    position: usize,
    serial: u64,
    credit: bool,
    gate: bool,
    emergency: bool,
    /// The silent note whose reservation this onset took over instead of
    /// claiming a fresh one, so an unaccepted onset can hand it straight back.
    inherited: u16,
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
    /// Largest accepted onset shift seen since the delay setting was last
    /// applied, reported as this Tune's worst lateness above D. A gauge, not a
    /// schedule: nothing reads it back to decide when anything emits.
    late_shift: i64,
    /// Notes that missed their deadline over the same interval, each counted
    /// exactly once however many times it is seen to be late.
    missed: u64,
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
    /// `multiplier x max_frames`, adopted whole at activation and fixed for
    /// it. The host is told this number and compensates playback by it.
    delay: i64,
    callback: Option<api::Callback>,
    visits: usize,
    intent_pushed: usize,
    cancel_cursor: Option<usize>,
    pub faults: u32,
    reset_armed: bool,
    /// A participation toggle's recenter, retired with its marker while the
    /// lease it belonged to was already sealed. Held rather than armed: a seal
    /// refuses fresh output, and the bend it would neutralize is on the wire
    /// rather than in the lease.
    pitch_center_owed: bool,
    pub participating: bool,
    timing_failed: bool,
    generation: u64,
    epoch: u64,
    /// The join request is published; the Hub has not enrolled this row yet.
    adopt_sent: bool,
    /// Enrolled. Ordinary output may reach the Hub's session from here.
    joined: bool,
    /// The coverage start this row's join request carried, so a join floor
    /// beyond it is recognized as a refusal and not as an enrollment.
    adopt_start: i64,
    /// This withdrawal's reset has already run. Cleared at the next adoption.
    withdrawal_reset: bool,
    coverage: Option<Coverage>,
    last_progress: Option<(i64, u64)>,
    acknowledged: u64,
    complete_through: i64,
    sealed_ack: Option<u64>,
    stopping: bool,
    transport_playing: bool,
    producer_joined: bool,
    joined_published: bool,
    joined_unknown_wire: bool,
    stops: stop::Stops,
    setup_pending: [Option<super::slots::Retained<setup::Update>>; 2],
    manifest: Queue<Manifest, 64>,
    committed_assignment: u64,
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
    /// This callback's normal output allowance is spent, so no later staging
    /// attempt in it can succeed and the walk stops rather than scanning on.
    stage_full: bool,
    /// The refusal `stage_work` just returned was one note's own wait, not the
    /// track's: rule one's two waits, and an established note whose shifted
    /// release is due in a later callback. Set per `stage_pending` call and
    /// read only by `schedule_pending`.
    note_wait: bool,
    capture_cursor: Option<usize>,
    /// Copies of the input event at `capture_group_position`, waiting for room
    /// in the intent ring. They are self-contained, so nothing remote depends
    /// on the envelope they came from.
    capture_group: Queue<Capture, CAPTURE_GROUP>,
    capture_group_position: usize,
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

    /// One note, one count. The same note reaches this twice by design -- once
    /// when its deadline passes with no assignment, again when it finally
    /// emits late -- and only the first arrival is the note. Its lateness is
    /// measured separately, in `late_shift`, where the emission that knows the
    /// number is; a note counted on the earlier arrival still contributes the
    /// worst lateness it eventually turns out to have.
    fn timing_failure(&mut self, life: u16) {
        let request = self.lives.local_mut(life).expect("retained timing request");
        if request.timing_reported {
            return;
        }
        request.timing_reported = true;
        self.missed += 1;
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

    /// The deadline gauge, to this Tune's own editor and to the row its Hub
    /// aggregates. Both numbers describe the interval since the delay setting
    /// was applied, so nothing here is cleared by a quiet passage.
    fn publish_deadline(&self) {
        let worst = self.late_shift.saturating_sub(self.delay()).max(0);
        self.shared.extra_delay.store(worst as u64, Ordering::Relaxed);
        self.shared.deadline_misses.store(self.missed, Ordering::Relaxed);
        if let Some(offer) = &self.offer {
            let row = &offer.session.rows[usize::from(offer.lease.slot - 1)];
            row.delay.store(self.delay(), Ordering::Release);
            row.worst_lateness.store(worst, Ordering::Relaxed);
            row.deadline_misses.store(self.missed, Ordering::Relaxed);
        }
    }

    fn delay(&self) -> i64 {
        #[cfg(test)]
        if self.test_aggregation {
            return 0;
        }
        if self.shared.source.is_some() {
            self.delay
        } else {
            0
        }
    }
    #[cfg(test)]
    pub fn test_snapshot(&self) -> Snapshot {
        Snapshot {
            captures: self.capture_group.len(),
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
        let (pending, lives, work) = super::capture::storage();
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
            late_shift: 0,
            missed: 0,
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
            delay: 0,
            callback: None,
            visits: 0,
            intent_pushed: 0,
            cancel_cursor: None,
            faults: 0,
            reset_armed: false,
            pitch_center_owed: false,
            participating: true,
            timing_failed: false,
            generation: 1,
            epoch: 0,
            adopt_sent: false,
            joined: false,
            adopt_start: i64::MIN,
            withdrawal_reset: false,
            coverage: None,
            last_progress: None,
            acknowledged: 0,
            complete_through: i64::MIN,
            sealed_ack: None,
            stopping: false,
            transport_playing: false,
            producer_joined: false,
            joined_published: false,
            joined_unknown_wire: false,
            stops: stop::Stops::default(),
            setup_pending: [None, None],
            manifest: Queue::default(),
            committed_assignment: 0,
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
            stage_full: false,
            note_wait: false,
            capture_cursor: None,
            capture_group: Queue::default(),
            capture_group_position: 0,
            settlement_cursor: None,
            settled_input: 0,
            settlement_sent: 0,
            capture_published: 0,
            membership: 0,
        })
    }
    /// The multiplier is the Tune's saved parameter and `max_frames` the
    /// format the host advertises for this activation; D is their product and
    /// nothing recomputes it until the next activation. Applying a new setting
    /// is what clears the deadline measurement, so the count and worst
    /// lateness on screen always describe the delay that is actually running.
    pub fn activate(&mut self, rate: f64, max_frames: u32, multiplier: u32) {
        let first = self.rate == 0.0;
        self.rate = rate;
        self.max_frames = max_frames;
        self.delay = i64::from(multiplier) * i64::from(max_frames);
        self.late_shift = 0;
        self.missed = 0;
        if !first {
            // Reactivation is a host-owned boundary, not a clock failure: no
            // callback is in flight, so cancel and release like Stop and then
            // adopt the new format outright. Publishing an invalid clock here
            // instead strands the release debt this Stop just armed, because
            // no later callback can repair a clock it never covers.
            self.stop();
            self.adopt_boundary_clock();
            return;
        }
        self.apply_setup();
        self.clock = Clock::new(self.shared.value().routing.calibration(), rate, max_frames);
        self.shared.publish_clock(&self.clock);
        self.coverage = None;
    }
    /// Reactivation takes the host's current format as adopted. The host
    /// skips enclosing time while it is stopped, so raw continuity starts
    /// over; the session's own coverage does not, because the lease it was
    /// reported under survives this boundary and the Hub grants only a
    /// contiguous accepted prefix.
    pub fn adopt_boundary_clock(&mut self) {
        let coverage = self.clock.coverage;
        self.clock = Clock::new(self.clock.calibration, self.rate, self.max_frames);
        self.clock.coverage = coverage;
        self.shared.publish_clock(&self.clock);
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
        self.joined = true;
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
        // A participation marker still standing in the queue is disposed after
        // this, by the retirement pump, and the recentre its disposal owes can
        // no longer reach output. Resolve it here, into the evidence this
        // publishes, rather than leaving it to arm debt nothing will clear:
        // the bend stays on the wire either way, and that is exactly what
        // unknown joined wire state means.
        self.joined_unknown_wire = self.unknown_joined_wire_state()
            || (self.pitch_center_owed || self.participation_marker_queued())
                && self.owes_pitch_center();
        self.producer_joined = true;
        if self.joined_unknown_wire {
            // Destruction removes the only possible owner of further physical
            // termination. Publish its exact row evidence before the joined
            // control message can wait behind a full mailbox; the Hub decides
            // local/session scope from its retained membership. Missing initial
            // controller state and factual ACK debt alone are not wire loss.
            self.fault(REFERENCE_FAULT);
        }
    }
    pub fn joined_cut(&self) -> Option<u64> {
        self.producer_joined.then_some(self.sequence)
    }
    /// What this Tune has already put on the wire and still owns: voices the
    /// receiver is sounding, pedals it is holding, and note-offs owed for
    /// terminals the host took. Every reset path asks this before it decides
    /// whether it owes releases, because forgetting ownership without
    /// terminating these is what strands a note in the instrument.
    fn forwarded_wire_state(&self) -> bool {
        self.state.count() != 0 || self.state.pedals_held() || self.owed_note_off != [NONE; 64]
    }
    pub fn unknown_joined_wire_state(&self) -> bool {
        self.forwarded_wire_state()
            || self.emergency.iter().flatten().any(|release| release.accepted.is_none())
            || self.channel_reset != [0; 16]
    }
    pub fn settled(&self) -> bool {
        self.output_settled() && self.pending.len() == 0 && self.capture_group.len() == 0
    }
    fn lease_settled(&self) -> bool {
        self.old_pending == 0 && self.capture_group.len() == 0 && self.output_settled()
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
    }

    /// Reserve the return/ack slot BEFORE moving an endpoint-bearing offer.
    /// Exactly one attach OR detach attempt occurs at an enclosing boundary.
    ///
    /// Both directions are a membership reset. Withdrawal cancels this Tune's
    /// pending attacks and arms the termination of every voice it has already
    /// forwarded, before its ownership of them is forgotten; adoption ends
    /// anything still sounding from before the pairing, so the Hub starts with
    /// nothing of this Tune's to be told about. That is what removed the
    /// held-note snapshot the Tune used to hand over at adoption: after the
    /// reset there is never one to send. Obsolete replies die with the lease
    /// incarnation and session epoch they were minted under.
    fn attachment(&mut self) {
        if self.attachment_attempted {
            return;
        }
        self.attachment_attempted = true;
        if self.shared.source.is_none() {
            return;
        }
        if self.offer.as_ref().is_some_and(|offer| {
            offer.session.rows[usize::from(offer.lease.slot - 1)].withdrawn.load(Ordering::Acquire)
        }) {
            if !self.withdrawal_reset {
                // Once per withdrawal, and the reason the reset is what a
                // withdrawal wants: its cancel is also what lets the lease
                // settle instead of waiting for input the new pairing will
                // never deliver.
                self.withdrawal_reset = true;
                self.stop();
            }
        } else {
            // A row can be un-withdrawn without ever being detached — a Hub
            // clock boundary fences every row and then reopens them. Arm the
            // next withdrawal from the observed flag rather than from the
            // adoption that may never come.
            self.withdrawal_reset = false;
        }
        let mut adopted = false;
        let bridge = self.shared.source.as_ref().unwrap();
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
            // The Hub reads this row's delay to report what each output was
            // planned for, and it can bind an onset from the first callback
            // this row is enrolled for. Publish D with the row, not only in
            // the end-of-callback gauge.
            row.delay.store(self.delay(), Ordering::Release);
            row.worst_lateness.store(0, Ordering::Relaxed);
            row.deadline_misses.store(0, Ordering::Relaxed);
            self.epoch = offer.session.epoch.load(Ordering::Acquire);
            self.generation = offer.generation;
            self.old_pending = self.obligations;
            self.adopt_sent = false;
            self.joined = false;
            self.adopt_start = i64::MIN;
            self.withdrawal_reset = false;
            adopted = true;
            self.committed_assignment = 0;
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
        // A Tune that sounded before it had a Hub — or before this one —
        // arrives holding voices the new session has no record of. End them
        // here rather than importing them: the interruption is accepted, a
        // silent divergence between what sounds and what the Hub believes is
        // not.
        if adopted && self.forwarded_wire_state() {
            self.arm_release_debt();
        }
    }

    pub fn begin(&mut self, callback: api::Callback) {
        self.intent_pushed = 0;
        self.stage_full = false;
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
        // A recenter the seal refused, now that the lease it waited on has
        // returned. Placed after the drain because that is where a cancelled
        // marker is disposed, so an obligation raised this callback is armed
        // in it rather than in the next one.
        if self.pitch_center_owed {
            self.arm_pitch_center();
        }
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
        }
        // Registry offers precede the Hub's first audio callback. Its initial
        // epoch is provisional until that callback publishes actual progress.
        // Nothing from an unsent join request may cross that clock boundary.
        let initial_wait = !self.adopt_sent
            && self
                .offer
                .as_ref()
                .is_some_and(|offer| offer.session.hub_through.load(Ordering::Acquire) == i64::MIN);
        if !self.adopt_sent && !initial_wait {
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

    /// Delivered once and owned from here. The wrapper retains nothing for a
    /// later callback, so every refusal below is a latched fault rather than a
    /// request to be offered this value again.
    pub fn input(&mut self, input: OwnedInput) {
        if let InputValue::Parameter { id, value, modulation: false } = input.value {
            // Participation is the only parameter this class reads, and the id
            // is the only thing that says which one arrived: a stepped
            // parameter carries its step index, so the tuning delay's 1x is a
            // zero here and would read as Off. The delay is the wrapper's to
            // store and the activation's to have adopted already.
            //
            // For participation the original retained value is the authority
            // even if the generic parameter atomic has run ahead.
            if self.shared.source.is_some() && id == hash_param_id(setup::PARTICIPATING) {
                let Some(sample) = input.sample else {
                    self.fault(INPUT_FAULT);
                    return;
                };
                self.capture_participation(value >= 0.5, Some(sample));
            }
            return;
        }
        if let InputValue::Transport(transport) = input.value {
            // The wrapper retains enclosing transport in the same input pool.
            // Observe it only here, in original order: a newer callback's raw
            // flag cannot move the cancellation cut ahead of retained input.
            let playing = transport.flags & (1 << 4) != 0;
            if self.transport_playing && !playing {
                let Some(sample) = input.sample else {
                    self.fault(INPUT_FAULT);
                    return;
                };
                self.capture_stop(sample);
            }
            self.transport_playing = playing;
            return;
        }
        let Some(mut event) = Event::from_input(input.value) else {
            return;
        };
        if self.participating && self.delay() != 0 {
            // A PARTICIPATING musical Tune owns pitch. Normalize before capture
            // so both prospective scoring and factual output see the same
            // pitch; DIRECT observation still forwards its original input.
            //
            // Off owns nothing, so its bend and its per-note tuning are the
            // player's and go out unchanged. `participating` moves at this
            // event's own place in the input order, which is what makes the
            // note after the toggle the first one in the new mode.
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
            return;
        }
        let Some(raw) = input.sample else {
            self.trace.input(event);
            self.fault(INPUT_FAULT);
            return;
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
        // input binding, lifetime pin, or input-cut mutation. Exhaustion is an
        // explicit bounded failure now that nothing retains the value for a
        // later callback: the latch is the whole report. Ingestion spends no
        // visit budget — its own storage is what bounds it, and the budget
        // belongs to the staging walkers.
        if self.pending.free() == 0 || self.next_event == u64::MAX {
            self.fault(STORAGE_FAULT);
            return;
        }
        let references = if addressed && count <= 1 { 0 } else { count };
        if self.work.free() < references {
            self.fault(REFERENCE_FAULT);
            return;
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
                return;
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
                    timing_reported: false,
                },
            );
            index
        } else if addressed && count == 1 {
            targets[0]
        } else {
            NONE
        };
        let position = self.enqueue_cell(event, life, raw, addressed && count != 1);
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
        self.cancel_unsounded_through(self.next_event);
    }
    fn cancel_unsounded_through(&mut self, cut: u64) {
        self.cancel_cursor = self.pending.front_position();
        self.cancel_cut = self.cancel_cut.max(cut);
        self.stopping = self.cancel_cursor.is_some_and(|position| {
            self.pending.at(position).is_some_and(|pending| pending.serial <= self.cancel_cut)
        });
    }
    fn local_cancel_cut_settled(&self) -> bool {
        !self.stopping
            && self.manifest.front().is_none_or(|manifest| manifest.serial > self.cancel_cut)
            && self.work_cleanup_head == NONE
    }
    /// The one reset a Tune has: cancel every unsounded attack at the current
    /// input cut, then terminate what has already been forwarded, before its
    /// ownership is forgotten. `cancel_cut` is also what rejects the replies
    /// those cancelled attacks are still owed.
    ///
    /// It does NOT latch. Input past the cut is admitted again on the next
    /// event, once the channel's release debt has been accepted. Every caller
    /// takes this same whole-queue scope and differs only in what it does
    /// afterwards, and in what the restart therefore needs:
    ///
    /// - transport Stop and host Reset resume by themselves.
    /// - a host reactivation adopts the new format next; the lease, epoch and
    ///   generation deliberately survive that boundary.
    /// - a membership withdrawal resumes at the next adoption, and the old
    ///   incarnation and epoch are what kill its replies.
    /// - an explicit setup reset and a Hub clock fence are the only callers
    ///   that also arm `reset_armed`, which is what lets `begin` clear a
    ///   latched fault. A bare Reset without one stays terminal.
    /// - destruction never resumes.
    ///
    /// A terminal fault is these same two steps plus that latch, and lives in
    /// `fault` rather than here because it owes releases whatever the wire
    /// state and refuses new attacks until an explicit Reset. A missed
    /// deadline is neither: lateness goes to the late-playback path and never
    /// reaches this function.
    pub fn stop(&mut self) {
        self.cancel_unsounded();
        // Credits can outlive an accepted Off until its factual ACK arrives.
        // Only actual wire state may create fresh release debt at Stop.
        if self.forwarded_wire_state() {
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
            if used_channels & (1 << channel) != 0 || state.controller_valid != [0, 0] {
                for (bit, controller) in [64usize, 66, 69].into_iter().enumerate() {
                    let known_neutral =
                        state.controller_valid[controller / 64] & (1 << (controller % 64)) != 0
                            && state.controllers[controller] < 64;
                    // A completed reset is already factual; another fault
                    // cannot recreate it. A staged reset still owns its debt.
                    if !known_neutral
                        && self.channel_reset[channel] & (1 << (bit + CHANNEL_RESETS)) == 0
                    {
                        self.channel_reset[channel] |= 1 << bit;
                    }
                }
            }
        }
    }

    /// Participating means the Tune owns pitch again, so the wire cannot be
    /// left holding a bend an Off phrase passed through: every note the Tune
    /// tunes afterwards would sound at that offset. Recenter the channels
    /// this Tune has actually bent -- one it never bent, or already left at
    /// center, owes nothing and costs no event.
    ///
    /// Only a participation toggle arms this. Stop and a terminal fault end a
    /// phrase without changing who owns pitch, and the recenter would be an
    /// event no reset before this one sent.
    fn arm_pitch_center(&mut self) {
        // Past the producer join there is no far side to wait for: the
        // retirement pump invokes no host output and never reaches `begin`, so
        // a bit armed here would hold `output_settled` false for the life of
        // the process and the registry would keep this Source for good.
        // `join_producer` has already put the bend into the teardown evidence.
        if self.producer_joined {
            return;
        }
        // Behind the final cut `schedule_emergency` stages nothing, so a bit
        // armed here would never leave and `output_settled` would wait on it
        // for good. The obligation outlives the lease, though -- the wire keeps
        // the bend across the seal -- so it waits rather than being dropped,
        // and `begin` arms it on the far side.
        if self.sealed {
            self.pitch_center_owed = true;
            return;
        }
        self.pitch_center_owed = false;
        for channel in 0..16 {
            if self.owes_pitch_center_on(channel) {
                self.channel_reset[channel] |= 1 << PITCH_RESET;
            }
        }
    }

    /// A channel this Tune has actually bent and has not already recentred.
    /// One it never bent, or already left at center, owes nothing.
    fn owes_pitch_center_on(&self, channel: usize) -> bool {
        self.state.channels()[channel].pitch_bend.is_some_and(|value| value != BEND_CENTER)
            && self.channel_reset[channel] & (1 << (PITCH_RESET + CHANNEL_RESETS)) == 0
    }
    fn owes_pitch_center(&self) -> bool {
        (0..16).any(|channel| self.owes_pitch_center_on(channel))
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
        self.drain_finished();
    }

    /// Rule one waits with the note an event is addressed to, and nothing else
    /// waits behind it. So a blocked entry — an attack still without its
    /// assignment, most often — pins the cursor but does not end the walk: the
    /// unaddressed events behind it, a raw MIDI clock among them, keep their
    /// own input+D schedule. The visit budget bounds the scan.
    fn schedule_pending(&mut self, start: i64, end: i64, output: &mut api::Output<'_>) {
        let mut next = self.pending_cursor;
        let mut blocked = false;
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
            self.note_wait = false;
            if !self.stage_pending(position, start, end, output) {
                blocked = true;
                // A refusal that belongs to one note holds that note and
                // nothing else, so the walk steps over it: rule one's two
                // waits, and an established note whose own shift puts its
                // release in a later callback than the events queued behind
                // it. An unaddressed raw MIDI event and a later onset on the
                // same channel each keep their own input+D schedule past all
                // three. Any other refusal is this callback's spent output
                // allowance, a transport boundary or a track-wide fence, and
                // stops the walk exactly as it always did. `stage_work` is
                // what knows which it was, and says so in `note_wait`.
                if !self.stage_full && self.note_wait {
                    continue;
                }
                break;
            }
            if !blocked {
                self.pending_cursor = next;
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
        if parent.event.marker() {
            return matches!(parent.channel.role, channel::Role::ReachedStop);
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
    /// Everything the replacement's own attack must clear before its
    /// predecessor may be choked for it. Timing is not here: both are due at
    /// the same sample, and `stage_work` refuses that for itself.
    fn replacement_ready(&mut self, parent: Pending) -> bool {
        if !self.assignment_ready(parent.life) {
            self.note_wait = true;
            return false;
        }
        !self.stage_full && self.admitted(parent.generation)
    }
    /// Everything one group owes before it can be admitted, ending in the
    /// group itself. It stages nothing: a replacement's choke and onset are
    /// planned separately and admitted together, so a plan that succeeds must
    /// still be discardable when its partner's does not.
    fn plan_work(
        &mut self,
        position: usize,
        child: u16,
        start: i64,
        end: i64,
        output: &mut api::Output<'_>,
    ) -> Plan {
        let Some(parent) = self.pending.at(position) else {
            return Plan::Skip;
        };
        if parent.staged {
            return Plan::Skip;
        }
        if child != NONE && self.work.at(child).phase != 0 {
            return Plan::Skip;
        }
        let pending = self.resolved(position, child);
        if pending.disposition
            || child == NONE
                && parent.inline_done
                && !matches!(parent.channel.role, channel::Role::Header { .. })
        {
            return Plan::Skip;
        }
        if (self.faults != 0 || pending.serial <= self.cancel_cut) && !pending.event.release() {
            return Plan::Refused;
        }
        if !pending.event.release()
            && pending.event.channel().is_some_and(|channel| self.channel_has_release_debt(channel))
        {
            return Plan::Refused;
        }
        if !self.channel_ready(pending) {
            return Plan::Refused;
        }
        let life = (pending.life != NONE).then(|| self.lives.at(pending.life).unwrap());
        if self.detaching
            || self.sealed
            || self.offer.as_ref().is_some_and(|offer| pending.generation > offer.generation)
            || self.direct.is_some()
                && self.transition_seen != 0
                && pending.generation > self.direct_generation
        {
            return Plan::Refused;
        }
        if life.is_some_and(|life| {
            life.canceled
                || life.terminal.is_some() && !(life.note_off_owed && pending.event.release())
        }) {
            self.dispose_work(position, child);
            return Plan::Skip;
        }
        // Rule two ties a replacement's forced release of its predecessor to the
        // moment the replacement is emitted, and an attack's children are
        // exactly that release. Resolved, the child IS a release, so it skips
        // every check an attack owes; without this the key would be silenced
        // while its replacement was still unassigned or unadmitted. Both
        // staging paths reach it -- the predecessor's own indexed release scan
        // stages this child too, and it is the one that gets here first.
        if child != NONE && parent.event.attack().is_some() && !self.replacement_ready(parent) {
            return Plan::Refused;
        }
        if pending.event.attack().is_some() && !self.admitted(pending.generation) {
            return Plan::Refused;
        }
        if pending.event.attack().is_some() && !self.assignment_ready(pending.life) {
            if pending.input.checked_add(self.delay()).is_some_and(|deadline| deadline < end) {
                self.timing_failure(pending.life);
            }
            self.note_wait = true;
            return Plan::Refused;
        }
        let established = life.is_some_and(|life| life.sounded);
        if life.is_some() && !established && pending.event.attack().is_none() {
            self.note_wait = true;
            return Plan::Refused;
        }
        if pending.event.attack().is_none()
            && !pending.event.release()
            && life.is_some_and(|life| life.ready_head != work::ready_reference(position, child))
        {
            return Plan::Refused;
        }
        // Rule: an established note keeps its own onset lateness for its own
        // later release and expression. Everything else — a fresh onset and
        // every shared channel control — is input time plus the fixed delay.
        // A replacement's forced release of its predecessor is the exception:
        // it happens when the REPLACEMENT emits, so it takes that fresh onset's
        // input+D rather than however late the note it displaces was.
        let replacement = child != NONE && parent.event.attack().is_some();
        let shift = if established && !replacement {
            life.unwrap().shift.unwrap_or(0)
        } else {
            self.delay()
        };
        let Some(mut due) = pending.input.checked_add(shift) else {
            self.fault(CLOCK_FAULT);
            return Plan::Refused;
        };
        due = due.max(start);
        if due >= end || self.next_stop_sample().is_some_and(|stop| due >= stop) {
            // Only an established note can be due AFTER something queued behind
            // it: its release rides its own onset lateness while everything
            // else is input+D, which is monotonic in input order. A stop
            // boundary blocks the whole track and is not this note's wait.
            self.note_wait |= established && due >= end;
            return Plan::Refused;
        }
        let callback = self.callback.unwrap();
        let Some(offset) =
            due.checked_sub(callback.steady_time).and_then(|value| u32::try_from(value).ok())
        else {
            self.fault(CLOCK_FAULT);
            return Plan::Refused;
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
            return Plan::Refused;
        };
        self.attempt = attempt;
        let token = api::Token([
            attempt,
            position as u64,
            pending.serial,
            if child == NONE { 0 } else { u64::from(child) + 3 },
        ]);
        let wire = self.assigned_event(pending);
        // Only an adaptive onset carries an initial tuning. An Off onset has
        // no assignment to state, and stating the default would be a zero
        // sent over whatever bend or per-note tuning the player is holding --
        // centering by another name, which is exactly what Off does not do.
        let group = if pending.event.attack().is_some() && life.is_some_and(|life| life.adaptive) {
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
            api::Group::onset(token, time, wire.input(), tuning.input())
        } else {
            api::Group::single(token, api::Lane::Normal, time, wire.input())
        };
        let Ok(group) = group else {
            self.fault(INPUT_FAULT);
            return Plan::Refused;
        };
        Plan::Ready(group)
    }

    fn stage_work(
        &mut self,
        position: usize,
        child: u16,
        start: i64,
        end: i64,
        output: &mut api::Output<'_>,
    ) -> bool {
        let group = match self.plan_work(position, child, start, end, output) {
            Plan::Skip => return true,
            Plan::Refused => return false,
            Plan::Ready(group) => group,
        };
        // Rule two ties the choke to the replacement's own emission, and the
        // two are separate host pushes: the choke, then a completion, then the
        // onset, with everything else the callback owes competing for the same
        // 512 credits in between. So they are admitted together or not at all.
        // Nothing is reserved across the round trip -- the onset is already in
        // the scheduler before the choke can be pushed -- which is what keeps a
        // callback boundary, a spent visit budget or another replacement from
        // coming between the two.
        let replacement = child != NONE
            && self.pending.at(position).is_some_and(|parent| parent.event.attack().is_some());
        let staged = if replacement {
            match self.plan_work(position, NONE, start, end, output) {
                Plan::Ready(onset) => output.stage_all(&[group, onset]),
                // The replacement will never emit, so nothing is choked for it:
                // the forced release retires with the onset instead of sounding
                // alone, which is also what keeps the envelope retirable.
                Plan::Skip => return self.dispose_work(position, child),
                Plan::Refused => return false,
            }
        } else {
            output.stage(group)
        };
        if staged.is_err() {
            self.stage_full = true;
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

    /// Whether this onset may be emitted at all yet. Only an ADAPTIVE onset
    /// waits: an Off Tune is a local forwarding path, so its onset has nothing
    /// to wait for and takes the same input+D every other event takes.
    /// DIRECT, whose whole delay is zero, is the same case.
    ///
    /// This relaxation is only sound because the Hub mints no plan for a
    /// non-adaptive record: an onset that emits without an assignment reports
    /// decision zero, which matches no plan, so a plan minted for one would
    /// never be retired.
    fn assignment_ready(&self, life: u16) -> bool {
        self.delay() == 0
            || self.lives.at(life).is_some_and(|life| {
                !life.adaptive
                    || life.assignment.decision != 0
                        && life.assignment.decision <= self.committed_assignment
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
        // create its first accepted output. An opened emission gate is that
        // join even before the enrollment reply lands. Retain the fact through
        // withdrawal so established controls remain responsive. A new offer
        // resets it; fresh attacks/setup still claim current admission
        // separately. Clock replacement also waits for the next validated
        // callback.
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
        if self.adopt_sent && row.emission_gate.load(Ordering::Acquire) & GATE_FLAGS == OPEN {
            self.joined = true;
        }
        self.joined
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
        if (!self.output_clock_valid() || !self.ordinary_stream_ready()) && !pending.event.release()
        {
            return false;
        }
        let report_cells =
            self.channel_report_cells(pending) + usize::from(group.initial_tuning().is_some());
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
            inherited: NONE,
        };
        if !self.channel_ready(pending) || !self.channel_wire_bindings_available(pending) {
            return false;
        }
        if pending.event.attack().is_some() {
            if !self.admitted(pending.generation) || !self.assignment_ready(pending.life) {
                return false;
            }
            // Rule two makes a replacement the same voice as the note it
            // displaces rather than a second one, so the pair holds one
            // reservation and not one each -- the same cell `State` already
            // reuses for a retrigger. Its predecessor is silent by the time
            // this onset is prepared, because an unaccepted choke takes the
            // onset down with it before either is permitted, so that
            // reservation IS the room this onset needs. Inheriting it retires
            // nothing early: the debt moves with the slot, and a successor's
            // terminal cut and time both dominate its predecessor's, so the
            // one credit comes back no sooner than the two would have.
            // Claiming a 65th instead is what left 64 held notes plus a
            // retrigger choked and silent until a later callback.
            let inherited = self.replaced_reservation(position);
            let slot = match inherited {
                Some(slot) => slot,
                None => {
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
                    slot
                }
            };
            if let Some(offer) = &self.offer {
                let row = &offer.session.rows[usize::from(offer.lease.slot - 1)];
                // Claiming the gate is a claim of OPEN. It used to be a claim
                // of whatever generation the planned `Assignment` recorded, but
                // the gate holds nothing but OPEN/BUSY/CLOSED, so the writer
                // masked those flags off a value that never had anything else
                // in it and both ends read zero -- 3,720 mints and 50,719
                // claims across the suite, every one of them zero. Reverting
                // the mode choice above to `delay() != 0` killed no test, which
                // is what a field that is always zero looks like.
                if row
                    .emission_gate
                    .compare_exchange(OPEN, BUSY, Ordering::AcqRel, Ordering::Acquire)
                    .is_err()
                {
                    return false;
                }
                permit.gate = true;
            }
            if inherited.is_some() {
                permit.inherited = self.reserved[slot];
                self.lives.local_mut(permit.inherited).unwrap().reserved = false;
            } else {
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
            }
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
        // A replacement's onset was admitted in the same pass as its choke and
        // is still in the scheduler, so the envelope stays busy through it and
        // hands the completion filter over by clearing the selection the choke
        // matched. A choke that was not accepted takes its onset with it:
        // `staged` falls as it always did, the onset finds no match of its own,
        // and the pair is planned and admitted again whole.
        if child != NONE && parent.event.attack().is_some() && completion.accepted & 1 != 0 {
            parent.selected = NONE;
        } else {
            parent.staged = false;
        }
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
            let partial =
                completion.group.initial_tuning().is_some() && completion.accepted & 2 == 0;
            delta.outcome = Outcome::wire(pending.life, partial);
            if partial {
                self.state.partial(delta.lifetime);
            }
            self.journal.push(delta).unwrap_or_else(|_| unreachable!("prepared journal credit"));
            if let Some(tuning) = completion.group.initial_tuning() {
                if completion.accepted & 2 != 0 {
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
            self.record_channel_terminals(pending, delta.sequence, actual);
            let waiter = match parent.channel.role {
                channel::Role::Header { first_waiter, .. } => first_waiter,
                _ => NONE,
            };
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
                if permit.inherited != NONE {
                    // An onset the host never took leaves its predecessor
                    // exactly as it found it: the borrowed slot goes back, and
                    // with it the acknowledgement debt it still carries. A
                    // permit only reaches here after an attempted push, so the
                    // OUTPUT_FAULT below always accompanies this and no fixture
                    // can tell the two apart -- this is the symmetric undo of
                    // the acquisition, like the credit and the gate beside it,
                    // rather than a state anything downstream still reads.
                    self.lives.local_mut(pending.life).unwrap().reserved = false;
                    *self.reserved.iter_mut().find(|v| **v == pending.life).unwrap() =
                        permit.inherited;
                    self.lives.local_mut(permit.inherited).unwrap().reserved = true;
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
            outcome: Outcome::wire(life, false),
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
                value.shift = shift;
                self.late_shift = self.late_shift.max(shift.unwrap_or_default());
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
            // are durable before BUSY is released; CLOSED stays.
            assert_ne!(row.emission_gate.fetch_and(!BUSY, Ordering::Release) & BUSY, 0);
        }
    }
    /// The reservation a replacement takes over instead of claiming a fresh
    /// one: the slot still held by the note its own forced release has already
    /// silenced. That release is this attack's own work child, so this is rule
    /// two's pair and not merely the last note on the key -- an ordinary attack
    /// after an ordinary release chokes nothing and waits its turn like any
    /// other. `terminal` with no Note-Off owed is what `arm_release_debt` reads
    /// to decide a life needs no emergency release of its own, so a slot handed
    /// on here is one no fault could still have to spend.
    fn replaced_reservation(&self, position: usize) -> Option<usize> {
        let mut child = self.pending.at(position)?.work_head;
        while child != NONE {
            let cell = self.work.at(child);
            if matches!(cell.operation, work::CHOKE | work::NOTE_OFF)
                && self.lives.at(cell.life).is_some_and(|previous| {
                    previous.reserved && previous.terminal.is_some() && !previous.note_off_owed
                })
            {
                return self.reserved.iter().position(|index| *index == cell.life);
            }
            child = cell.next;
        }
        None
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
            !l.active && !l.reserved && l.refs == 0 && !l.ready_queued && !l.assignment_held
        }) {
            self.lives.remove(index);
            self.free_lives.push(index);
            if let Some(slot) = self.active.iter_mut().find(|i| **i == index) {
                *slot = NONE;
            }
        }
    }

    fn schedule_emergency(&mut self, output: &mut api::Output<'_>) {
        if self.sealed {
            return;
        }
        // 64 voice releases and 64 channel resets fit the reserved 128 exactly.
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
            for bit in 0..CHANNEL_RESETS {
                if self.channel_reset[channel] & (1 << bit) == 0 {
                    continue;
                }
                let event = Self::channel_reset_event(channel as u8, bit);
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
                self.channel_reset[channel] |= 1 << (bit + CHANNEL_RESETS);
            }
        }
    }
    /// The neutral wire value for one channel reset slot. The three pedals go
    /// out as their controller at zero; the recenter is the only one that is
    /// not a CC, and it carries the 14-bit center split the way MIDI does.
    fn channel_reset_event(channel: u8, bit: usize) -> Event {
        if bit == PITCH_RESET {
            return Event::Midi {
                port: 0,
                data: [0xe0 | channel, (BEND_CENTER & 0x7f) as u8, (BEND_CENTER >> 7) as u8],
                flags: 0,
            };
        }
        Event::Midi { port: 0, data: [0xb0 | channel, [64, 66, 69][bit], 0], flags: 0 }
    }
    fn prepare_emergency(&mut self, group: api::Group) -> bool {
        if self.sealed || self.emergency_output.free() == 0 || self.sequence == u64::MAX {
            return false;
        }
        // `position` and `serial` are the ordinary lane's slot-reuse guard --
        // `complete` asserts the pending cell it is about to write is still
        // the one the permit was prepared for. The emergency lane compares
        // neither, and cannot with what it carries: `complete_emergency`
        // derives both from `completion.group.token`, the same token this
        // prepared from, so an assertion here would compare a value to itself.
        // A real check wants a serial on `Release`, which is a mechanism and
        // an abort path rather than a deletion (#712).
        self.permit = Some(Permit {
            position: group.token.0[1] as usize,
            serial: group.token.0[2],
            credit: false,
            gate: false,
            emergency: true,
            inherited: NONE,
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
            let (pending_bit, staged_bit) = (1 << bit, 1 << (bit + CHANNEL_RESETS as u8));
            self.channel_reset[index] &= !staged_bit;
            if completion.accepted & 1 != 0 {
                let event = Event::from_input(completion.group.event(0).unwrap()).unwrap();
                let actual = self.callback.unwrap().steady_time + i64::from(completion.group.time);
                let delta = self.record(event, NONE, actual, actual);
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
            offer.session.rows[usize::from(offer.lease.slot - 1)].to_source.take_repair_if(|_| true)
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
    }
    fn reply(&mut self, reply: Reply) {
        match reply {
            Reply::CohortCommitted { lease, epoch, through }
                if self.capture_lease() == Some(lease) && self.epoch == epoch =>
            {
                self.committed_assignment = self.committed_assignment.max(through);
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
                    self.lives.local_mut(life).unwrap().assignment_held = false;
                    self.recycle(life);
                }
            }
            // A copied reply carries the whole identity it needs: the lease and
            // epoch reject an obsolete reset generation, and the request slot,
            // its birth serial and the onset's own input serial together reject
            // a slot that has since been reused.
            Reply::Assignment { request, binding }
                if self.capture_lease() == Some(request.lease)
                    && self.epoch == request.epoch
                    && binding.decision != 0
                    && usize::from(request.request) < LIFETIMES =>
            {
                let life = request.request;
                if self.lives.at(life).is_some_and(|held| {
                    held.serial == request.lifetime
                        && held.on_serial == request.serial
                        && !held.sounded
                        && !held.canceled
                        // Reserved means its onset already passed preparation
                        // with the values it will emit; a later assignment
                        // must not disagree with the wire event.
                        && !held.reserved
                        && request.serial > self.cancel_cut
                        && held.assignment.decision < binding.decision
                }) {
                    let held = self.lives.local_mut(life).unwrap();
                    held.assignment = binding;
                    held.assignment_held = true;
                    self.trace.assignments = self.trace.assignments.saturating_add(1);
                    self.trace.decision = binding.decision;
                    self.trace.correction = binding.correction;
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
            Reply::Enrolled { incarnation, epoch, membership, start }
                if incarnation == self.incarnation() && epoch == self.epoch && self.adopt_sent =>
            {
                // A floor beyond what the join request carried is a refusal:
                // the Hub has already published past that point, so this row
                // rejoins from there with a fresh request rather than being
                // admitted behind the frontier.
                if start > self.adopt_start {
                    self.adopt_sent = false;
                    self.joined = false;
                    self.adopt_start = i64::MIN;
                    self.clock.coverage = None;
                    self.coverage = None;
                    self.last_progress = None;
                    self.input_reported = None;
                } else {
                    self.joined = true;
                    if self.membership != membership {
                        self.input_reported = None;
                    }
                    self.membership = membership;
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
        self.drain_finished();
        self.shared.status.store(self.diagnostics(), Ordering::Release);
        self.publish_deadline();
        if self.direct.is_some() {
            self.publish_seal();
            return;
        }
        // Same reasoning as the retired pump below, for a Tune that is simply
        // unpaired and still playing: nothing can send it `OutputRetained`,
        // `transfer` will not even look at the journal without an offer, and a
        // journal that only grows latches STORAGE_FAULT after 4,096 accepted
        // events and keeps the Source from ever settling (#718). Its own
        // forwarding is the whole fact; pairing is a reset boundary, so a Hub
        // that arrives later is told the cut it starts from and never wants
        // this history.
        if self.session().is_none() {
            let through = self.coverage.map_or(self.complete_through, |c| c.through);
            self.acknowledge(self.sequence, through.max(self.complete_through));
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
            i64::from(self.adopt_sent) + i64::from(self.joined),
            row.map_or(-1, |row| row.emission_gate.load(Ordering::Acquire) as i64),
            row.map_or(0, |row| i64::from(row.withdrawn.load(Ordering::Acquire))),
            session.map_or(0, |s| s.closing.load(Ordering::Acquire) as i64),
            i64::from(self.participating),
            self.held() as i64,
            self.pending.len() as i64,
            self.old_pending as i64,
            self.capture_group.len() as i64,
            self.journal.len() as i64,
            self.emergency_output.len() as i64,
            self.adopt_start,
            i64::from(self.joined),
            i64::from(self.output_settled()),
            i64::from(self.local_cancel_cut_settled()),
            i64::from(self.diagnostics()),
            self.late_shift.saturating_sub(self.delay()).max(0),
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
                | (i64::from(self.channel_reset != [0; 16]) << 8),
            self.capture_published as i64,
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
    /// Copy one input event into as many self-contained records as it
    /// addresses targets, so the Hub never sees a fan-out and never reads a
    /// byte this Tune still owns. Fills `capture_group`; the caller drains it.
    fn build_capture_group(&mut self, position: usize) -> bool {
        let lease = match self.capture_lease() {
            Some(lease) => lease,
            None => return false,
        };
        let pending = match self.pending.at(position) {
            Some(pending) if self.pending.sealed(position) => pending,
            _ => return false,
        };
        // Post-reset input belongs to the next lease. Copying it into the
        // closing session sends records that cannot be sequenced or detached.
        if self.session().is_some_and(|session| session.closing.load(Ordering::Acquire) != 0)
            && self.lease_generation().is_none_or(|lease| pending.generation > lease)
        {
            return false;
        }
        let Some(sample) = pending.input.checked_add(self.clock.calibration.offset) else {
            self.fault(CLOCK_FAULT);
            return false;
        };
        let base = Capture {
            lease,
            epoch: self.epoch,
            serial: pending.serial,
            sample,
            kind: CaptureKind::Other,
            request: NO_REQUEST,
            lifetime: 0,
            channel: 0,
            key: 0,
            adaptive: false,
        };
        let addressed = |base: Capture, kind: CaptureKind, index: u16, life: Life| Capture {
            kind,
            request: index,
            lifetime: life.serial,
            channel: life.channel,
            key: life.key,
            adaptive: life.adaptive,
            ..base
        };
        // A same-key predecessor and every note a channel termination ends
        // become their own Terminal records. They share this input's serial,
        // and the ordering pass applies every terminal before any onset, so a
        // replacement cannot be scored against the note it just displaced.
        let terminals = pending.event.attack().is_some()
            || pending.event.channel_termination().is_some()
            || pending.event.release();
        let per_target = if terminals {
            Some(CaptureKind::Terminal)
        } else if matches!(pending.event, Event::Expression { kind: 2, value, .. } if value.is_finite())
        {
            let Event::Expression { value, .. } = pending.event else { unreachable!() };
            Some(CaptureKind::Tuning { value_bits: value.to_bits() })
        } else {
            None
        };
        let mut overflow = false;
        if let Some(kind) = per_target {
            if pending.life != NONE && pending.event.attack().is_none() {
                if let Some(life) = self.lives.at(pending.life) {
                    overflow |=
                        self.capture_group.push(addressed(base, kind, pending.life, life)).is_err();
                }
            }
            let mut child = pending.work_head;
            while child != NONE {
                let cell = self.work.at(child);
                if let Some(life) =
                    self.lives.at(cell.life).filter(|life| life.serial == cell.serial)
                {
                    overflow |=
                        self.capture_group.push(addressed(base, kind, cell.life, life)).is_err();
                }
                child = cell.next;
            }
        }
        let own = match pending.event {
            Event::Participation(value) => Some(CaptureKind::Participation(value)),
            Event::Stop => Some(CaptureKind::Stop),
            event if event.attack().is_some() => Some(CaptureKind::Onset),
            event if event.channel_control().is_some() => Some(CaptureKind::Channel),
            // Everything else already left as addressed records. An input that
            // addressed none still owes the Hub one record so the input cut
            // can pass it.
            _ if self.capture_group.len() != 0 => None,
            _ => Some(CaptureKind::Other),
        };
        if let Some(kind) = own {
            let record = match (kind, self.lives.at(pending.life)) {
                (CaptureKind::Onset, life) => {
                    addressed(base, kind, pending.life, life.expect("onset request"))
                }
                (CaptureKind::Channel, _) => {
                    Capture { kind, channel: pending.event.channel().unwrap_or_default(), ..base }
                }
                (_, Some(life)) => addressed(base, kind, pending.life, life),
                (_, None) => Capture { kind, ..base },
            };
            overflow |= self.capture_group.push(record).is_err();
        }
        if overflow {
            // More addressed targets than one input event can hold. Bounded
            // failure: latch rather than send the Hub a partial group.
            self.fault(REFERENCE_FAULT);
            self.capture_group.clear();
            return false;
        }
        true
    }
    /// The next record this Source will copy out, left where it stands.
    /// None when nothing can be copied right now, which is also what the
    /// caller sees from `next_capture`.
    fn fill_capture_group(&mut self) -> Option<Capture> {
        loop {
            if let Some(record) = self.capture_group.front() {
                return Some(record);
            }
            let position = self.capture_cursor?;
            if !self.build_capture_group(position) {
                return None;
            }
            self.capture_group_position = position;
        }
    }
    /// Drain order within one input event is the order the group was built in,
    /// and the Hub's merge keeps that order for records sharing a serial.
    fn next_capture(&mut self) -> Option<Capture> {
        self.fill_capture_group()?;
        let record = self.capture_group.pop().unwrap();
        if self.capture_group.len() == 0 {
            let position = self.capture_group_position;
            self.pending.publish(position);
            self.capture_published = record.serial;
            self.capture_cursor = self.pending.next_position(position);
            self.service_revision = self.service_revision.wrapping_add(1);
            // Publication is the last thing an envelope can be waiting on.
            // Nothing else revisits it, so settle it here.
            self.remove_finished(position);
        }
        Some(record)
    }
    fn transfer_captures(&mut self) {
        if !self.adopt_sent {
            return;
        }
        for _ in 0..512 {
            if self.intent_pushed == 512 {
                break;
            }
            if self.offer.as_ref().is_none_or(|offer| offer.endpoints.intents.slots() == 0) {
                break;
            }
            let Some(record) = self.next_capture() else {
                break;
            };
            self.push_intent(Intent::Capture(record))
                .unwrap_or_else(|_| unreachable!("checked intent ring capacity"));
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
            || self.capture_group.len() != 0
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
    /// The Hub's own MIDI reaches its row by the same copied records, minus
    /// the ring: the Hub owns both sides of this handoff.
    pub(super) fn take_direct_capture(&mut self) -> Option<Capture> {
        assert!(self.direct.is_some());
        self.next_capture()
    }
    /// What the next `take_direct_capture` would hand over, so the Hub can see
    /// whether a full DIRECT queue is a wait or an oversized same-sample group.
    pub(super) fn peek_direct_capture(&mut self) -> Option<Capture> {
        assert!(self.direct.is_some());
        self.fill_capture_group()
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
            || !self.adopt_sent
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
            // What this flag asks the Hub for is the retirement of the plan
            // this attack minted. An Off attack minted none, so saying yes
            // would have the Hub hold a placeholder for a request that is
            // never coming — one `LIFETIMES` slot per cancelled Off note.
            original_on: child == NONE
                && pending.event.attack().is_some()
                && self.lives.at(pending.life).is_some_and(|life| life.adaptive),
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
        self.intent_pushed = 0;
        self.visits = 0;
        // A Source with no session at all has no reader for its accepted
        // output: the facts are already in its own State, and no Hub exists to
        // send `OutputRetained`. Retiring the journal locally is what lets
        // destruction settle; without it the registry entry and the whole
        // Source leak for the life of the process. `end` does the same for a
        // live unpaired Tune, which is the other half of #718.
        if self.session().is_none() && self.acknowledged < self.sequence {
            self.acknowledge(self.sequence, self.complete_through);
        }
        self.receive();
        self.cancel_slice();
        self.drain_ready_work();
        self.drain_finished();
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
    fn initial_enrollment_ready(&self) -> bool {
        self.adopt_sent
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
        if self.coverage.is_none() {
            return;
        }
        self.publish_input_prefix();
        let (report, output_cut) = self.output_report().unwrap();
        let input_start_cut = self
            .capture_cursor
            .and_then(|index| self.pending.at(index))
            .map_or(self.next_event, |pending| pending.serial - 1);
        let offer = self.offer.as_mut().unwrap();
        let row = &offer.session.rows[usize::from(lease.slot - 1)];
        // The join request is the whole of what the Hub is told at adoption:
        // the reset above guarantees this Tune holds nothing for the new
        // session to import.
        if !self.adopt_sent
            && !row.withdrawn.load(Ordering::Acquire)
            && row
                .to_hub
                .publish(Control::Adopt {
                    lease: offer.lease,
                    epoch: self.epoch,
                    coverage: report,
                    output_cut,
                    input_start_cut,
                    participating: self.participating && self.faults == 0,
                })
                .is_ok()
        {
            self.adopt_sent = true;
            self.adopt_start = report.start;
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
        if self.detaching || !self.adopt_sent {
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
        self.coverage.map(|coverage| (coverage, self.sequence))
    }

    fn publish_seal(&mut self) {
        if self.sealed
            || self.state.pedals_held()
            || self.owed_note_off != [NONE; 64]
            || self.old_pending != 0
            || self.state.count() != 0
            || self.channel_reset != [0; 16]
            || self.permit.is_some()
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
        (self.offer.as_ref().map(|offer| offer.lease), self.adopt_sent, self.joined, self.coverage)
    }
    pub fn test_rebase_output_prefix(&mut self, prefix: u64) -> Lease {
        assert_eq!(self.journal.len(), 0);
        assert_eq!(self.emergency_output.len(), 0);
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
        println!("LEDGER source work [cell,count,backing] {:?}", self.work.test_layout());
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
            [size_of::<wave::Wave>(), size_of::<stop::Stops>()]
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
    pub published: bool,
}
#[cfg(test)]
impl Source {
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
                published: self.pending.published(index),
            });
        }
        None
    }
    pub(super) fn test_reset_progress(&self) -> String {
        format!("armed={} pending={:?} generation={} applied={} setup={} offer={:?} detaching={} settled={} lease={} old={} captures={} published={} sealed={}", self.reset_armed,
            self.setup_pending.each_ref().map(|v| v.as_ref().map(|v| v.value)), self.generation,
            self.shared.applied.load(Ordering::Acquire), self.setup_started,
            self.offer.as_ref().map(|o| (o.generation, o.lease)), self.detaching,
            self.output_settled(), self.lease_settled(), self.old_pending, self.capture_group.len(), self.capture_published, self.sealed)
    }
    pub(super) fn test_reset_armed(&self) -> bool {
        self.reset_armed
    }
}
