//! Audio-owned collection and canonical publication. Musical retention never
//! waits for the display, file drainer, an editor, or registry bookkeeping.
use super::{
    clock::{Clock, Coverage},
    protocol::*,
    queue::{Queue, Window},
    registry::HubOffer,
    setup,
    source::{Source, BUSY, CLOSED, FENCED, OPEN},
    state::{Stamp, State},
};
use crate::configuration::Owner;
use harmonigraph_core::canonical::{ClockId, EventTiming};
use harmonigraph_core::confirmed::{ConfirmedPitch, PitchProvenance};
use harmonigraph_core::VoiceKey;
use harmonigraph_record::{publication::PublishError, Recorder};
use nice_plug::wrapper::clap::performance as api;
use std::sync::atomic::Ordering;
use std::sync::Arc;

#[derive(Clone, Copy)]
struct ChannelWitness {
    sequence: u64,
    input: i64,
    actual: i64,
    channel: u8,
    controller: u8,
    derived: u8,
}
struct Row {
    captures: super::capture::Permissions,
    channel_witness: Option<ChannelWitness>,
    lease: Option<Lease>,
    epoch: u64,
    state: State,
    output: Queue<OutputDelta, 2048>,
    received: u64,
    received_actual: Option<i64>,
    actual_order: bool,
    applied: u64,
    report: Option<(Coverage, u64)>,
    coverage: Option<Coverage>,
    baseline: Option<Baseline>,
    member: bool,
    participating: bool,
    repair: bool,
    baseline_id: u64,
    detach: Option<u64>,
    last_ack: Option<(u64, i64)>,
    ingress: Window<Intent, 1024>,
    ingress_cursor: Option<usize>,
    ingress_left: usize,
    last_disposition: Option<u64>,
    input_coverage: Option<(Coverage, u64)>,
    seal: Option<u64>,
    producer_joined: Option<u64>,
    joined_unknown_wire: bool,
    seal_generation: u64,
    last_sealed_ack: Option<(u64, u64)>,
    joining: Option<i64>,
}
impl Default for Row {
    fn default() -> Self {
        Self {
            captures: super::capture::Permissions::default(),
            channel_witness: None,
            lease: None,
            epoch: 0,
            state: State::default(),
            output: Queue::default(),
            received: 0,
            received_actual: None,
            actual_order: true,
            applied: 0,
            report: None,
            coverage: None,
            baseline: None,
            member: false,
            participating: true,
            repair: false,
            baseline_id: 0,
            detach: None,
            last_ack: None,
            ingress: Window::default(),
            ingress_cursor: None,
            ingress_left: 0,
            last_disposition: None,
            input_coverage: None,
            seal: None,
            producer_joined: None,
            joined_unknown_wire: false,
            seal_generation: 0,
            last_sealed_ack: None,
            joining: None,
        }
    }
}
pub struct Hub {
    #[cfg(all(test, not(feature = "tuning-probe")))]
    pub test_capture_request: Option<i64>,
    #[cfg(all(test, not(feature = "tuning-probe")))]
    pub test_capture_result:
        Option<Result<harmonigraph_core::cohort::Progress, harmonigraph_core::cohort::Error>>,
    #[cfg(all(test, not(feature = "tuning-probe")))]
    pub test_capture_commit: bool,
    #[cfg(test)]
    pub window_report_seen: bool,
    pub shared: Arc<setup::Shared>,
    pub offer: Option<HubOffer>,
    pub direct: Box<Source>,
    rows: Box<[Row; TUNERS]>,
    clock: Clock,
    rate: f64,
    max_frames: u32,
    callback: Option<api::Callback>,
    anchor: Option<(i64, f64)>,
    rotation: usize,
    membership: u64,
    publication_through: Option<i64>,
    collected: usize,
    input_work: usize,
    merged: usize,
    publication_clock: ClockId,
    transition: Option<setup::Update>,
    invalidated: bool,
    clock_loss_pending: bool,
    retired_publication: Option<(Box<Owner>, Recorder, f64)>,
    retired_through: Option<i64>,
    service_revision: u64,
    capture_hold: bool,
    frozen_captures: super::capture::Frozen,
    direct_ingress: Window<Intent, INTENT_RING>,
    direct_captures: super::capture::Permissions,
    direct_capture_cursor: Option<usize>,
    direct_capture_left: usize,
}
// Charged owner upper bounds apply in production builds too, where the fixture
// freeze controls are absent. Larger backing cells have their own assertions.
const _: () = assert!(std::mem::size_of::<Hub>() <= 1104);
const _: () = assert!(std::mem::size_of::<Row>() <= 31032);
impl Hub {
    pub fn end(
        &mut self,
        callback: api::Callback,
        owner: &mut Owner,
        recorder: &mut Recorder,
        observation: f64,
    ) {
        self.direct.end(callback);
        if let Some(through) = self.publication_through {
            self.direct.acknowledge(self.direct.sequence, through);
        }
        if owner.direct.pending().is_none() {
            self.direct.acknowledge_seal();
        }
        self.commit_transition(owner, recorder, observation);
    }
    fn fence_rows(&self, generation: u64) {
        if let Some(offer) = &self.offer {
            offer.session.closing.store(generation, Ordering::Release);
            for row in &offer.session.rows {
                row.withdrawn.store(true, Ordering::Release);
                for state in [OPEN, CLOSED] {
                    let _ = row.emission_gate.compare_exchange(
                        state,
                        FENCED,
                        Ordering::AcqRel,
                        Ordering::Acquire,
                    );
                }
            }
        }
    }
    /// A host reset cannot erase a live session's old accepted history/maps.
    /// Explicit setup, sealed streams and settled termination permit recovery.
    pub fn force_reset(&mut self, owner: &mut Owner, allow_idle: bool) -> bool {
        if self.offer.is_none() {
            return false;
        }
        if allow_idle
            && !self.invalidated
            && self.direct.settled()
            && owner.direct.pending().is_none()
            && owner.direct.state.count() == 0
            && !owner.direct.state.pedals_held()
            && !owner.recording.owns_recording()
            && self.rows.iter().all(|row| row.lease.is_none())
        {
            self.fence_rows(u64::MAX);
            let session = &self.offer.as_ref().unwrap().session;
            if session.rows.iter().all(|row| {
                row.source_detached.load(Ordering::Acquire)
                    && row.hub_detached.load(Ordering::Acquire)
                    && row.emission_gate.load(Ordering::Acquire) != BUSY
            }) {
                // No musical or recording owner can refer to this clock. Keep
                // the existing empty host-reset behavior for configuration.
                return false;
            }
        }
        if !self.invalidated {
            self.invalidated = true;
            owner.frozen = true;
            self.clock.valid = false;
            self.direct.clock.valid = false;
            self.shared.publish_clock(&self.direct.clock);
            self.fence_rows(u64::MAX);
            self.direct.fence_transition();
            self.shared.status.store(super::source::CLOCK_FAULT, Ordering::Release);
            // Publish the explicit gap after available pre-reset history has
            // drained: the take's failure consumer closes its file on that gap.
            self.clock_loss_pending = true;
        }
        true
    }
    pub fn reset_idle_clock(&mut self, clock: ClockId) {
        let Some(offer) = &self.offer else {
            return;
        };
        self.direct.reset_idle_clock(clock.epoch);
        self.clock = Clock::new(self.clock.calibration, self.rate, self.max_frames);
        self.publication_clock = clock;
        self.publication_through = None;
        self.anchor = None;
        offer.session.epoch.store(clock.epoch, Ordering::Release);
        for row in &offer.session.rows {
            row.withdrawn.store(false, Ordering::Release);
            row.emission_gate.store(CLOSED, Ordering::Release);
        }
        offer.session.closing.store(0, Ordering::Release);
    }
    pub fn input_boundary(&mut self) {
        if let Some(update) = self.direct.apply_setup_with_clock(false) {
            if self.transition.is_none() {
                self.transition = Some(update);
                if !self.invalidated {
                    self.fence_rows(update.generation);
                }
                self.direct.fence_transition();
            }
        }
    }
    pub fn new() -> Box<Self> {
        let shared = setup::Shared::hub();
        let direct = Source::new(shared.clone());
        Box::new(Self {
            #[cfg(all(test, not(feature = "tuning-probe")))]
            test_capture_request: None,
            #[cfg(all(test, not(feature = "tuning-probe")))]
            test_capture_result: None,
            #[cfg(all(test, not(feature = "tuning-probe")))]
            test_capture_commit: false,
            #[cfg(test)]
            window_report_seen: false,
            shared,
            offer: None,
            direct,
            rows: (0..TUNERS)
                .map(|_| Row::default())
                .collect::<Vec<_>>()
                .into_boxed_slice()
                .try_into()
                .ok()
                .unwrap(),
            clock: Clock::new(Default::default(), 0.0, 0),
            rate: 0.0,
            max_frames: 0,
            callback: None,
            anchor: None,
            rotation: 0,
            membership: 0,
            publication_through: None,
            collected: 0,
            input_work: 0,
            merged: 0,
            publication_clock: ClockId::default(),
            transition: None,
            invalidated: false,
            clock_loss_pending: false,
            retired_publication: None,
            retired_through: None,
            service_revision: 0,
            capture_hold: false,
            frozen_captures: super::capture::Frozen::default(),
            direct_ingress: Window::default(),
            direct_captures: super::capture::Permissions::default(),
            direct_capture_cursor: None,
            direct_capture_left: 0,
        })
    }
    pub fn activate(&mut self, rate: f64, frames: u32) {
        let first = self.rate == 0.0;
        self.rate = rate;
        self.max_frames = frames;
        if !first {
            self.clock.valid = false;
            self.direct.activate(rate, frames);
            return;
        }
        self.clock = Clock::new(self.shared.value().routing.calibration(), rate, frames);
        self.direct.activate(rate, frames);
        self.anchor = None;
    }
    fn attach(&mut self, owner: &mut Owner) {
        if self.offer.is_some() {
            return;
        }
        let bridge = self.shared.hub.as_ref().unwrap();
        let Some(return_slot) = bridge.returns.reserve() else {
            return;
        };
        let Some(offer) = bridge.offers.take() else {
            return;
        };
        owner.recording.clock.runtime_session = offer.session.runtime;
        offer.session.epoch.store(owner.recording.clock.epoch, Ordering::Release);
        self.direct.attach_direct(offer.session.clone());
        return_slot.publish(offer.session.runtime);
        self.offer = Some(offer);
        self.shared.request_main();
    }
    pub fn begin(&mut self, callback: api::Callback, owner: &mut Owner, presentation: f64) {
        self.attach(owner);
        self.callback = Some(callback);
        self.collected = 0;
        self.input_work = 0;
        self.merged = 0;
        let offset = self.clock.calibration.offset;
        owner.recording.hub_offset = offset;
        self.publication_clock = owner.recording.clock;
        owner.direct.offset = offset;
        if let Some(sample) = callback.steady_time.checked_add(offset) {
            if self.anchor.is_none() {
                self.anchor = Some((sample, presentation));
            }
        }
        let coverage = self.clock.begin(callback.steady_time, callback.frames);
        if self.clock.calibration.validated && coverage.is_none()
            || self.offer.as_ref().is_some_and(|offer| {
                offer.session.faults.load(Ordering::Acquire) & super::source::CLOCK_FAULT != 0
            })
        {
            self.force_reset(owner, false);
        }
        self.direct.begin(callback);
        self.collect_direct_captures();
        self.collect();
        #[cfg(all(test, not(feature = "tuning-probe")))]
        self.test_capture_tick();
    }

    fn commit_transition(&mut self, owner: &mut Owner, recorder: &mut Recorder, observation: f64) {
        let Some(update) = self.transition else {
            return;
        };
        if !update.routing.calibration().matches(self.rate, self.max_frames)
            || !self.direct.transition_settled()
            || owner.direct.pending().is_some()
        {
            return;
        }
        let Some(offer) = &self.offer else {
            return;
        };
        if offer.session.rows.iter().any(|row| {
            row.emission_gate.load(Ordering::Acquire) == BUSY
                || !row.source_detached.load(Ordering::Acquire)
                || !row.hub_detached.load(Ordering::Acquire)
        }) || self.rows.iter().any(|row| {
            row.output.len() != 0
                || row.ingress.len() != 0
                || row.baseline.is_some()
                || row.state.count() != 0
        }) || offer.session.credits.load(Ordering::Acquire) != 0
        {
            return;
        }
        // End-of-callback source acknowledgements still use the old clock.
        // Finish every old recording route before changing either offset.
        owner.finish_recording_publication(recorder, observation);
        if self.invalidated {
            owner.recording.close_invalidated(recorder);
        }
        let Some(epoch) = self.publication_clock.epoch.checked_add(1) else {
            return;
        };
        let clock = ClockId { epoch, ..self.publication_clock };
        if !owner.recording.commit_clock(clock, update.routing.calibration().offset) {
            return;
        }
        self.direct.commit_clock_setup(update, epoch);
        owner.resume_clock(update.routing.calibration().offset, self.invalidated);
        self.invalidated = false;
        self.clock = Clock::new(update.routing.calibration(), self.rate, self.max_frames);
        self.anchor = None;
        self.publication_through = None;
        self.publication_clock = clock;
        self.transition = None;
        // Rematching cannot reopen a row until the complete committed clock is
        // visible. Old returned/still-Ready offers remain withdrawn and fenced.
        offer.session.faults.store(0, Ordering::Release);
        offer.session.epoch.store(epoch, Ordering::Release);
        offer.session.closing.store(0, Ordering::Release);
        self.shared.request_main();
    }
    fn clock_id(&self) -> ClockId {
        self.publication_clock
    }
    fn presentation(&self, sample: i64) -> f64 {
        let (anchor, time) = self.anchor.unwrap_or((0, 0.0));
        time + (sample as f64 - anchor as f64) / self.rate
    }
    fn collect(&mut self) {
        let Some(offer) = &mut self.offer else {
            return;
        };
        for step in 0..TUNERS {
            let index = (self.rotation + step) % TUNERS;
            let shared = &offer.session.rows[index];
            let row = &mut self.rows[index];
            if row.lease.is_some_and(|lease| {
                lease.incarnation == shared.expected_incarnation.load(Ordering::Acquire)
            }) && shared.hub_detached.load(Ordering::Acquire)
            {
                continue;
            }
            if row.lease.is_some_and(|lease| {
                lease.incarnation != shared.expected_incarnation.load(Ordering::Acquire)
            }) && shared.hub_detached.load(Ordering::Acquire)
                && row.output.len() == 0
                && row.state.count() == 0
                && row.ingress.len() == 0
                && row.captures.empty()
                && row.baseline.is_none()
            {
                row.lease = None;
                row.last_disposition = None;
                row.channel_witness = None;
                row.epoch = 0;
                row.state = State::default();
                row.received = 0;
                row.received_actual = None;
                row.actual_order = true;
                row.applied = 0;
                row.report = None;
                row.coverage = None;
                row.member = false;
                row.participating = true;
                row.repair = false;
                row.baseline_id = 0;
                row.detach = None;
                row.last_ack = None;
                row.input_coverage = None;
                row.seal = None;
                row.producer_joined = None;
                row.joined_unknown_wire = false;
                row.last_sealed_ack = None;
                row.joining = None;
            }
            for _ in 0..2 {
                // Keep the earliest not-yet-retained cut. A later report owns
                // its typed slot until this one can establish its interval.
                if row.report.is_some() {
                    break;
                }
                let Some(control) = shared.to_hub.take() else {
                    break;
                };
                self.service_revision = self.service_revision.wrapping_add(1);
                match control {
                    Control::Adopt { lease, epoch, start }
                        if lease.session == offer.session.runtime
                            && lease.incarnation
                                == shared.expected_incarnation.load(Ordering::Acquire)
                            && epoch == offer.session.epoch.load(Ordering::Acquire) =>
                    {
                        if row.lease.is_none() {
                            row.lease = Some(lease);
                            row.epoch = epoch;
                            row.coverage = Some(Coverage { start, through: start });
                            shared.hub_detached.store(false, Ordering::Release);
                        }
                    }
                    Control::Progress { incarnation, epoch, coverage, output_cut }
                        if row.lease.is_some_and(|l| l.incarnation == incarnation)
                            && epoch == row.epoch =>
                    {
                        if row.joining.is_some_and(|start| coverage.start >= start) && !row.member {
                            row.coverage =
                                Some(Coverage { start: coverage.start, through: coverage.start });
                        }
                        if row.report.is_none_or(|(old, cut)| {
                            coverage.start == old.start
                                && coverage.through >= old.through
                                && output_cut >= cut
                        }) {
                            row.report = Some((coverage, output_cut));
                        }
                    }
                    Control::Seal { incarnation, epoch, generation, cut }
                        if row.lease.is_some_and(|l| l.incarnation == incarnation)
                            && epoch == row.epoch
                            && shared.withdrawn.load(Ordering::Acquire) =>
                    {
                        row.seal = Some(cut);
                        row.seal_generation = generation;
                        row.joining = None;
                    }
                    Control::ProducerJoined { incarnation, epoch, cut, unknown_wire }
                        if row.lease.is_some_and(|lease| lease.incarnation == incarnation)
                            && epoch == row.epoch
                            && shared.withdrawn.load(Ordering::Acquire)
                            && row.received <= cut
                            && row.producer_joined.is_none_or(|old| old == cut) =>
                    {
                        row.producer_joined = Some(cut);
                        row.joined_unknown_wire = unknown_wire;
                        row.joining = None;
                    }
                    Control::Detach { incarnation, epoch, cut }
                        if row.lease.is_some_and(|l| l.incarnation == incarnation)
                            && epoch == row.epoch =>
                    {
                        row.detach = Some(cut)
                    }
                    _ => {}
                }
            }
            if row.baseline.is_none() {
                if let Some(baseline) = shared.baselines.take() {
                    self.service_revision = self.service_revision.wrapping_add(1);
                    if row.lease.is_some_and(|l| l.incarnation == baseline.incarnation)
                        && row.epoch == baseline.epoch
                    {
                        row.baseline = Some(baseline);
                    }
                }
            }
            for _ in 0..256 {
                if self.input_work == 4096 || row.ingress.free() == 0 {
                    break;
                }
                let units = match offer.bank.rows[index].intents.peek() {
                    Ok(Intent::Capture(token)) => token.units(),
                    Ok(_) => 1,
                    Err(_) => break,
                };
                if self.input_work + units > 4096 {
                    break;
                }
                let Ok(intent) = offer.bank.rows[index].intents.pop() else {
                    break;
                };
                self.input_work += units;
                self.service_revision = self.service_revision.wrapping_add(1);
                if let Intent::Disposition {
                    incarnation,
                    transaction,
                    total: 1,
                    index: 0,
                    canceled: true,
                    ..
                } = &intent
                {
                    // Seed in FIFO receipt order, never first sweep encounter:
                    // this Source's monotonic counter can predate this lease.
                    if *transaction != 0
                        && row.last_disposition.is_none()
                        && row.lease.is_some_and(|lease| lease.incarnation == *incarnation)
                    {
                        row.last_disposition = Some(transaction - 1);
                    }
                }
                let intent = if let Intent::Capture(token) = intent {
                    if row.lease == Some(token.key.lease) && row.epoch == token.key.epoch {
                        row.captures.accept(&token, token.key.lease, row.epoch);
                        Intent::Capture(token)
                    } else {
                        Intent::CaptureRetirement(token.retire_unread())
                    }
                } else {
                    intent
                };
                row.ingress.push(intent).unwrap_or_else(|_| unreachable!("checked ingress window"));
            }
            // A sweep starts at the oldest retained cell and visits only the
            // cells present now. Later appends cannot prolong it: even with 64
            // arrivals per callback, blocked retirement gets another visit.
            if row.ingress_left == 0 {
                row.ingress_cursor = row.ingress.front_position();
                row.ingress_left = row.ingress.len();
            }
            for _ in 0..64 {
                let Some(position) = row.ingress_cursor else {
                    break;
                };
                let units = match row.ingress.at_ref(position) {
                    Some(Intent::Capture(token))
                        if !self.capture_hold && token.frozen.is_none() =>
                    {
                        token.units()
                    }
                    _ => 1,
                };
                if self.input_work + units > 4096 {
                    break;
                }
                self.input_work += units;
                row.ingress_left -= 1;
                row.ingress_cursor =
                    if row.ingress_left == 0 { None } else { row.ingress.next_position(position) };
                if let Some(Intent::Capture(token)) = row.ingress.at_ref(position) {
                    if self.capture_hold || token.frozen.is_some() {
                        continue;
                    }
                    row.ingress.map_at(position, |intent| {
                        let Intent::Capture(token) = intent else { unreachable!() };
                        Intent::CaptureRetirement(row.captures.retire(token))
                    });
                }
                let Some(intent) = row.ingress.at_ref(position) else {
                    unreachable!();
                };
                let reply = match intent {
                    Intent::CaptureRetirement(retirement) => {
                        Some(Reply::CaptureRetired(retirement.key))
                    }
                    Intent::Coverage { incarnation, epoch, coverage, input_cut }
                        if row.lease.is_some_and(|lease| lease.incarnation == *incarnation)
                            && *epoch == row.epoch =>
                    {
                        if row.input_coverage.is_none_or(|(old, cut)| {
                            old.start == coverage.start
                                && old.through <= coverage.through
                                && cut <= *input_cut
                        }) {
                            row.input_coverage = Some((*coverage, *input_cut));
                        }
                        None
                    }
                    Intent::Disposition {
                        incarnation,
                        transaction,
                        input_cut,
                        total: 1,
                        index: 0,
                        lifetime: _,
                        canceled: true,
                    } if *transaction != 0
                        && row.lease.is_some_and(|lease| lease.incarnation == *incarnation) =>
                    {
                        let last = row.last_disposition.unwrap();
                        if *transaction > last && last.checked_add(1) != Some(*transaction) {
                            // A failed older reply still owns manifest.front at
                            // Source. Keep this younger message through sweeps.
                            continue;
                        }
                        if *transaction <= last {
                            row.ingress.remove(position);
                            self.service_revision = self.service_revision.wrapping_add(1);
                            continue;
                        }
                        Some(Reply::Disposition {
                            incarnation: *incarnation,
                            transaction: *transaction,
                            input_cut: *input_cut,
                        })
                    }
                    _ => None,
                };
                if reply.is_some_and(|reply| offer.bank.rows[index].replies.push(reply).is_err()) {
                    continue;
                }
                if let Some(Reply::Disposition { transaction, .. }) = reply {
                    row.last_disposition = Some(transaction);
                }
                row.ingress.remove(position);
                self.service_revision = self.service_revision.wrapping_add(1);
            }
            for _ in 0..256 {
                if self.collected == 4096 || row.output.free() == 0 {
                    break;
                }
                let Ok(delta) = offer.bank.rows[index].outputs.pop() else {
                    break;
                };
                self.collected += 1;
                self.service_revision = self.service_revision.wrapping_add(1);
                if delta.epoch != row.epoch
                    || row.lease.is_none_or(|lease| lease.incarnation != delta.incarnation)
                {
                    continue;
                }
                if delta.sequence <= row.received {
                    continue;
                }
                if delta.sequence != row.received + 1 {
                    shared.faults.fetch_or(super::source::STORAGE_FAULT, Ordering::AcqRel);
                    row.state.complete = false;
                    row.repair = true;
                    break;
                }
                if delta.mapped {
                    if row.received_actual.is_some_and(|actual| delta.actual < actual) {
                        row.actual_order = false;
                        row.state.complete = false;
                        row.repair = true;
                        shared.faults.fetch_or(super::source::CLOCK_FAULT, Ordering::AcqRel);
                    }
                    row.received_actual = Some(delta.actual);
                } else {
                    row.actual_order = false;
                    row.received_actual = None;
                }
                row.output
                    .push(delta)
                    .unwrap_or_else(|_| unreachable!("checked owned output window"));
                row.received = delta.sequence;
            }
            if let Some((coverage, cut)) = row.report {
                // Sequence-complete accepted output is monotonically timed by
                // the wrapper cursor and continuous calibrated callbacks. If a
                // report spans more than the retained row window, the last
                // received timestamp proves only its EXCLUSIVE earlier prefix;
                // same-sample records still wait for the rest of their group.
                // This does not claim coverage from a lone delta: the retained
                // report independently supplies the continuous interval/start.
                let through = if cut <= row.received {
                    Some(coverage.through)
                } else {
                    row.received_actual.map(|actual| actual.min(coverage.through))
                };
                if row.actual_order {
                    if let Some(through) = through.filter(|through| {
                        row.coverage.is_some_and(|old| {
                            old.start == coverage.start && old.through <= *through
                        })
                    }) {
                        #[cfg(test)]
                        if cut > row.received
                            && cut.saturating_sub(row.applied) > OUTPUT_RING as u64
                            && row.coverage.is_some_and(|old| old.through < through)
                        {
                            self.window_report_seen = true;
                        }
                        row.coverage = Some(Coverage { start: coverage.start, through });
                    }
                }
                if cut <= row.received {
                    row.report = None;
                }
            }
            // Empty initial baseline may enroll without registry acknowledgement.
            // Its complete coverage is nevertheless required, including silence.
            if !row.member {
                if let Some(baseline) = row.baseline.as_ref().filter(|b| b.frame.output_cut == 0) {
                    let floor = row.joining.or(self.publication_through).unwrap_or(baseline.start);
                    if baseline.start < floor {
                        if let Some(ack) = shared.to_source.reserve() {
                            // The old snapshot is complete but cannot authorize
                            // historical enrollment behind a published frontier.
                            // Reserve a future join boundary and ask for fresh
                            // coverage; musical admission remains closed.
                            ack.publish(Reply::Baseline {
                                incarnation: baseline.incarnation,
                                epoch: baseline.epoch,
                                transaction: baseline.frame.id,
                                cut: 0,
                                membership: self.membership,
                                start: floor,
                            });
                            row.baseline = None;
                            row.joining = Some(floor);
                            row.report = None;
                            row.coverage = Some(Coverage { start: floor, through: floor });
                        }
                    }
                }
            }
            if !row.member
                && row.baseline.as_ref().is_some_and(|b| b.frame.output_cut == 0)
                && row.coverage.is_some_and(|c| c.through > c.start)
                && self.clock.valid
                && offer.session.alive.load(Ordering::Acquire)
            {
                row.member = true;
                self.membership = self.membership.saturating_add(1);
                row.joining = None;
                if !shared.withdrawn.load(Ordering::Acquire) {
                    let _ = shared.emission_gate.compare_exchange(
                        CLOSED,
                        OPEN,
                        Ordering::AcqRel,
                        Ordering::Acquire,
                    );
                }
            }
            if shared.withdrawn.load(Ordering::Acquire) {
                let _ = shared.emission_gate.compare_exchange(
                    OPEN,
                    CLOSED,
                    Ordering::AcqRel,
                    Ordering::Acquire,
                );
            }
        }
        self.rotation = (self.rotation + 1) % TUNERS;
    }

    pub fn publish(&mut self, owner: &mut Owner, recorder: &mut Recorder, observation: f64) {
        let Some(callback) = self.callback else {
            return;
        };
        let Some(raw_end) =
            owner.recording.block_start.checked_add(i64::from(owner.recording.block_frames))
        else {
            recorder.fail_configuration();
            return;
        };
        let Some(mut through) =
            raw_end.min(owner.recording.prefix).checked_add(owner.recording.hub_offset)
        else {
            recorder.fail_configuration();
            return;
        };
        if let Some(sealed) = self.retired_through {
            through = through.max(sealed);
        }
        if self.invalidated
            && self
                .rows
                .iter()
                .all(|row| row.lease.is_none() || row.seal.is_some_and(|cut| row.received >= cut))
        {
            // Complete closed streams can be disposed beyond the frozen HUB
            // map. This is not new recording coverage; missing routes fail.
            for row in &*self.rows {
                if let Some(last) =
                    row.output.get(row.output.len().saturating_sub(1)).filter(|d| d.mapped)
                {
                    through = through.max(last.actual.saturating_add(1));
                }
            }
        }
        // Copy membership ONCE for this collection/merge pass. A new arrival or
        // detach acknowledgement cannot retrospectively fill missing coverage.
        let members: [bool; TUNERS] = std::array::from_fn(|i| self.rows[i].member);
        for (index, member) in members.iter().copied().enumerate() {
            if let Some(start) = self.rows[index].joining {
                through = through.min(start);
            }
            if member {
                if self.rows[index]
                    .seal
                    .or(self.rows[index].producer_joined)
                    .is_some_and(|cut| self.rows[index].received >= cut)
                {
                    continue;
                }
                let Some(coverage) = self.rows[index].coverage else {
                    return;
                };
                through = through.min(coverage.through);
            }
        }
        if recorder.take_resync_request() {
            owner.direct.recovery = true;
            for row in self.rows.iter_mut().filter(|r| r.lease.is_some()) {
                row.repair = true;
            }
        }
        let clock = self.clock_id();
        while self.merged < 1024 {
            // A timestamp cannot order an unplaceable terminal. Consume only
            // a source FIFO head, after its earlier baseline/history; the gap
            // records the missing clock provenance independently of this merge.
            if let Some(index) = self.rows.iter().position(|row| {
                row.output.front().is_some_and(|delta| {
                    !delta.mapped
                        && row
                            .baseline
                            .as_ref()
                            .is_none_or(|baseline| delta.sequence <= baseline.frame.output_cut)
                })
            }) {
                let row = &mut self.rows[index];
                let value = row.output.pop().unwrap();
                row.channel_witness = None;
                if !matches!(value.outcome, Outcome::Wire)
                    || value.discontinuity_generation == 0
                    || !row.state.apply_unmapped_terminal(value.event, value.lifetime)
                {
                    row.state.complete = false;
                    recorder.fail_configuration();
                }
                self.clock_loss_pending = true;
                row.repair = true;
                row.applied = value.sequence;
                Self::confirm(row, &mut owner.confirmed);
                self.merged += 1;
                continue;
            }
            let direct = owner.direct.pending();
            let mut next = direct.and_then(|d| d.timing).map(|t| (t.sample, 0usize));
            for index in 0..TUNERS {
                if let Some(baseline) = self.rows[index]
                    .baseline
                    .as_ref()
                    .filter(|b| self.rows[index].applied >= b.frame.output_cut)
                {
                    let sample = (baseline.frame.time * self.rate).round() as i64;
                    if next.is_none_or(|old| (sample, TUNERS + 1 + index) < old) {
                        next = Some((sample, TUNERS + 1 + index));
                    }
                }
                if let Some(delta) = self.rows[index].output.front() {
                    if self.rows[index]
                        .baseline
                        .as_ref()
                        .is_some_and(|b| delta.sequence > b.frame.output_cut)
                    {
                        continue;
                    }
                    if !delta.mapped {
                        continue;
                    }
                    let sample = delta.actual;
                    if next.is_none_or(|old| (sample, index + 1) < old) {
                        next = Some((sample, index + 1));
                    }
                }
            }
            let Some((sample, index)) = next else {
                break;
            };
            if sample >= through {
                break;
            }
            self.merged += 1;
            if index == 0 {
                let delta = direct.unwrap();
                let route = owner
                    .recording_route(delta.timing.unwrap(), delta.event.time)
                    .unwrap_or_else(|_| {
                        recorder.fail_configuration();
                        Default::default()
                    });
                if recorder.publish_note(delta, observation, route).is_err() {
                    owner.direct.recovery = true;
                }
                owner.direct.published();
            } else if index > TUNERS {
                let row = index - TUNERS - 1;
                self.publish_baselines(
                    owner,
                    recorder,
                    observation,
                    sample.saturating_add(1),
                    Some(row),
                );
                if self.rows[row].baseline.is_some() {
                    break;
                }
            } else {
                let index = index - 1;
                let value = self.rows[index].output.pop().unwrap();
                if !Self::valid_outcome(&mut self.rows[index], value) {
                    self.rows[index].state.complete = false;
                    self.rows[index].repair = true;
                    self.rows[index].applied = value.sequence;
                    recorder.fail_configuration();
                    recorder.publication_lost(observation, Default::default());
                    continue;
                }
                let source = self.rows[index].lease.unwrap().source;
                let time = self.presentation(value.actual);
                let input_time = self.presentation(value.input);
                let timing = EventTiming {
                    clock,
                    input: value.input,
                    planned: None,
                    sample: value.actual,
                    sample_rate: self.rate,
                };
                let row = &mut self.rows[index];
                if let Some(delta) = row.state.apply(
                    value.event,
                    Stamp {
                        source,
                        sequence: value.sequence,
                        lifetime: value.lifetime,
                        time,
                        input_time,
                        timing: Some(timing),
                        provenance: PitchProvenance::AcceptedOutput,
                    },
                ) {
                    let route = owner.recording_route(timing, time).unwrap_or_else(|_| {
                        recorder.fail_configuration();
                        Default::default()
                    });
                    if recorder.publish_note(delta, observation, route).is_err() {
                        row.repair = true;
                    }
                }
                row.applied = value.sequence;
                Self::confirm(row, &mut owner.confirmed);
            }
        }
        let mut completed = through;
        if let Some(delta) = owner.direct.pending() {
            if let Some(timing) = delta.timing {
                completed = completed.min(timing.sample);
            }
        }
        for row in self.rows.iter() {
            if let Some(delta) = row.output.front() {
                // An undisposed unplaceable outcome freezes publication proof;
                // it is not assigned a guessed position in the sample domain.
                completed = completed.min(if delta.mapped {
                    delta.actual
                } else {
                    self.publication_through.unwrap_or(completed)
                });
            }
            if let Some(baseline) = row.baseline.as_ref() {
                completed = completed.min((baseline.frame.time * self.rate).round() as i64);
            }
        }
        self.publication_through =
            Some(self.publication_through.map_or(completed, |old| old.max(completed)));
        let completed = self.publication_through.unwrap();
        if owner.recording.source_frontier(clock, completed).is_err() {
            recorder.fail_configuration();
        }
        if self.clock_loss_pending
            && owner.direct.pending().is_none()
            && self.rows.iter().all(|row| {
                row.lease.is_none()
                    || row.seal.or(row.producer_joined) == Some(row.applied)
                        && row.output.len() == 0
                        && row.baseline.is_none()
            })
        {
            recorder.fail_configuration();
            recorder.publication_lost(observation, Default::default());
            self.clock_loss_pending = false;
            owner.direct.recovery = true;
            for row in self.rows.iter_mut().filter(|row| row.lease.is_some()) {
                row.repair = true;
            }
        }
        self.publish_baselines(owner, recorder, observation, completed, None);
        // Source journals may release only through this actual audio-owned
        // retention cut. GUI/file progress and baseline ack are absent here.
        self.acknowledge(completed);
        if self.offer.as_ref().is_some_and(|offer| offer.session.alive.load(Ordering::Acquire))
            && self.shared.hub.as_ref().unwrap().retired_pending.load(Ordering::Acquire)
        {
            self.shared.request_main();
        }
        // DIRECT repair is shared with the existing rich owner, after all of its
        // available earlier history has been merged and originally routed.
        #[cfg(test)]
        self.shared.before_direct_repair.reach();
        if owner.direct.pending().is_none() {
            owner.publish_direct(recorder, observation);
        }
        if let Some(offer) = &self.offer {
            if self.clock.valid && offer.session.alive.load(Ordering::Acquire) {
                offer.session.hub_through.store(
                    callback.steady_time
                        + i64::from(callback.frames)
                        + self.clock.calibration.offset,
                    Ordering::Release,
                );
            }
        }
    }
    fn valid_outcome(row: &mut Row, value: OutputDelta) -> bool {
        match value.outcome {
            Outcome::Wire => {
                row.channel_witness =
                    value.event.channel_termination().map(|choke| ChannelWitness {
                        sequence: value.sequence,
                        input: value.input,
                        actual: value.actual,
                        channel: value.event.channel().unwrap(),
                        controller: if choke { 120 } else { 123 },
                        derived: 0,
                    });
                true
            }
            Outcome::ChannelTerminal { wire_sequence, controller } => {
                let Some(witness) = &mut row.channel_witness else {
                    return false;
                };
                let valid = witness.sequence == wire_sequence
                    && witness.controller == controller
                    && witness.input == value.input
                    && witness.actual == value.actual
                    && value.event.channel() == Some(witness.channel)
                    && value.event.release()
                    && witness.derived < 64;
                if valid {
                    witness.derived += 1;
                }
                valid
            }
        }
    }
    fn confirm(row: &Row, confirmed: &mut harmonigraph_core::confirmed::ConfirmedPitches) {
        let Some(lease) = row.lease else {
            return;
        };
        let empty = ConfirmedPitch {
            key: VoiceKey { source: lease.source, channel: 0, note: 0 },
            lifetime: None,
            host_note_id: None,
            onset_sample: 0,
            pitch_microcents: 0,
            provenance: PitchProvenance::AcceptedOutput,
        };
        let mut rows = [empty; 64];
        let count = if row.participating && row.state.complete {
            row.state.confirmed(lease.source, &mut rows)
        } else {
            0
        };
        let _ = confirmed.replace_source(lease.source, &rows[..count]);
    }
    fn publish_baselines(
        &mut self,
        owner: &mut Owner,
        recorder: &mut Recorder,
        observation: f64,
        through: i64,
        selected: Option<usize>,
    ) {
        let clock = self.clock_id();
        let time_offset = self.presentation(0);
        let Some(offer) = &self.offer else {
            return;
        };
        for index in 0..TUNERS {
            if selected.is_some_and(|only| only != index) {
                continue;
            }
            let row = &mut self.rows[index];
            let Some(lease) = row.lease else {
                continue;
            };
            let shared = &offer.session.rows[index];
            if let Some(baseline) = row.baseline.as_ref() {
                let sample = (baseline.frame.time * self.rate).round() as i64;
                if row.applied == baseline.frame.output_cut && sample < through {
                    let Some(ack) = shared.to_source.reserve() else {
                        continue;
                    };
                    let transaction = baseline.frame.id;
                    let start = baseline.start;
                    let Some(id) = row.baseline_id.checked_add(1) else {
                        continue;
                    };
                    let mut frame = baseline.frame;
                    frame.id = id;
                    frame.translate(time_offset);
                    let timing = EventTiming {
                        clock,
                        input: sample,
                        planned: None,
                        sample,
                        sample_rate: self.rate,
                    };
                    let route = owner.recording_route(timing, frame.time).unwrap_or_else(|_| {
                        recorder.fail_configuration();
                        Default::default()
                    });
                    let result = recorder.publish_baseline(index + 1, &frame, observation, route);
                    if result == Err(PublishError::BaselineBusy) {
                        // This incoming historical cut is being settled, not
                        // retained for retry. Declare that reporting loss at
                        // its actual route before allowing subsequent output.
                        recorder.discard_publication(frame.time, route);
                    }
                    row.repair |= result.is_err();
                    row.state.replace(&frame);
                    row.participating = frame.participating;
                    row.baseline_id = row.baseline_id.max(frame.id);
                    Self::confirm(row, &mut owner.confirmed);
                    ack.publish(Reply::Baseline {
                        incarnation: lease.incarnation,
                        epoch: row.epoch,
                        transaction,
                        cut: frame.output_cut,
                        membership: self.membership,
                        start,
                    });
                    row.baseline = None;
                }
            }
            if selected.is_none()
                && !self.clock_loss_pending
                && row.repair
                && row.output.len() == 0
                && row.baseline.is_none()
                && recorder.publication_free() >= 2
            {
                let Some(id) = row.baseline_id.checked_add(1) else {
                    continue;
                };
                let sample = through.saturating_sub(1);
                let time = sample as f64 / self.rate + time_offset;
                let start =
                    row.coverage.map_or(sample, |c| c.start) as f64 / self.rate + time_offset;
                let Some(frame) = row.state.baseline(
                    lease.source,
                    id,
                    row.applied,
                    time,
                    start.min(time),
                    row.participating,
                ) else {
                    continue;
                };
                let timing = EventTiming {
                    clock,
                    input: sample,
                    planned: None,
                    sample,
                    sample_rate: self.rate,
                };
                let route = owner.recording_route(timing, time).unwrap_or_default();
                if recorder.publish_baseline(index + 1, &frame, observation, route).is_ok() {
                    row.baseline_id = id;
                    row.repair = false;
                }
            }
        }
    }
    fn acknowledge(&mut self, through: i64) {
        let Some(offer) = &mut self.offer else {
            return;
        };
        for (index, row) in self.rows.iter_mut().enumerate() {
            let Some(lease) = row.lease else {
                continue;
            };
            if offer.session.rows[index].hub_detached.load(Ordering::Acquire) {
                continue;
            }
            let sealed = (row.seal == Some(row.applied)
                && row.applied == row.received
                && row.output.len() == 0
                && row.baseline.is_none())
            .then_some(row.seal_generation);
            if row.last_ack != Some((row.received, through)) {
                let reply = Reply::OutputRetained {
                    incarnation: lease.incarnation,
                    epoch: row.epoch,
                    cut: row.received,
                    complete_through: through,
                };
                if offer.bank.rows[index].replies.push(reply).is_ok() {
                    row.last_ack = Some((row.received, through));
                    self.service_revision = self.service_revision.wrapping_add(1);
                }
            }
            if let Some(generation) = sealed {
                if row.last_sealed_ack != Some((row.applied, generation)) {
                    let reply = Reply::SealedStreamRetained {
                        incarnation: lease.incarnation,
                        epoch: row.epoch,
                        generation,
                        cut: row.applied,
                    };
                    if offer.bank.rows[index].replies.push(reply).is_ok() {
                        row.last_sealed_ack = Some((row.applied, generation));
                        self.service_revision = self.service_revision.wrapping_add(1);
                    }
                }
            }
            if row.detach.is_some_and(|cut| {
                row.seal == Some(cut)
                    && row.applied == cut
                    && row.output.len() == 0
                    && row.state.count() == 0
                    && row.ingress.len() == 0
                    && row.captures.empty()
                    && row.baseline.is_none()
            }) {
                row.member = false;
                if !offer.session.rows[index].hub_detached.swap(true, Ordering::AcqRel) {
                    self.service_revision = self.service_revision.wrapping_add(1);
                }
                // The source returns its actual endpoints after observing this.
                // Reuse is only registry-owned after the other user's ack.
            }
        }
    }
    pub fn service_position(&self) -> ([u64; TUNERS], [u64; TUNERS], [usize; 3], bool, u64) {
        (
            std::array::from_fn(|index| self.rows[index].received),
            std::array::from_fn(|index| self.rows[index].applied),
            [
                self.direct_ingress.len()
                    + self
                        .rows
                        .iter()
                        .map(|row| row.ingress.len() + row.output.len())
                        .sum::<usize>(),
                self.rows.iter().filter(|row| row.baseline.is_some()).count(),
                self.rows.iter().filter(|row| row.seal.is_some()).count(),
            ],
            self.retired_publication
                .as_ref()
                .is_some_and(|(owner, _, _)| owner.recording.publication_debt()),
            self.service_revision,
        )
    }
    pub fn retired_pump(&mut self) {
        self.release_captures(true);
        self.direct.retired_pump();
        self.collected = 0;
        self.input_work = 0;
        self.merged = 0;
        self.collect();
        self.collect_direct_captures();
        // Drain retained payloads without waiting for the final cut to fit in
        // this bounded window. publish still clamps to actual source coverage;
        // only retired_streams_published proves final publication ownership.
        let mut through = self.publication_through.unwrap_or(i64::MIN);
        for row in &*self.rows {
            if let Some(last) =
                row.output.get(row.output.len().saturating_sub(1)).filter(|d| d.mapped)
            {
                through = through.max(last.actual.saturating_add(1));
            }
            if let Some(baseline) = &row.baseline {
                let sample = (baseline.frame.time * self.rate).round() as i64;
                through = through.max(sample.saturating_add(1));
            }
        }
        self.retired_through = Some(through);
        if let Some((mut owner, mut recorder, observation)) = self.retired_publication.take() {
            if !owner.recording.retirement_finished {
                if let (Some(through), Some(end)) =
                    (self.retired_through.as_mut(), owner.direct.pending_end())
                {
                    *through = (*through).max(end);
                }
                self.publish(&mut owner, &mut recorder, observation);
                owner.finish_recording_publication(&mut recorder, observation);
                if owner.direct.pending().is_none() {
                    self.direct.acknowledge_seal();
                }
                if self.retired_streams_published(&owner) {
                    let unknown_held = self.direct.unknown_joined_wire_state()
                        || self.rows.iter().any(|row| {
                            row.joined_unknown_wire
                                || row.state.count() != 0
                                || row.state.pedals_held()
                        });
                    owner.recording.finish_retired_publication(&mut recorder, unknown_held);
                }
            }
            self.retired_publication = Some((owner, recorder, observation));
        }
        if let Some(through) = self.publication_through {
            self.acknowledge(through);
        }
        if let Some(through) = self.publication_through {
            self.direct.acknowledge(self.direct.sequence, through);
        }
        let Some(offer) = &self.offer else {
            return;
        };
        for (index, row) in self.rows.iter_mut().enumerate() {
            if let Some(baseline) =
                row.baseline.as_ref().filter(|b| b.frame.output_cut == 0 && row.received == 0)
            {
                let Some(ack) = offer.session.rows[index].to_source.reserve() else {
                    continue;
                };
                ack.publish(Reply::Baseline {
                    incarnation: baseline.incarnation,
                    epoch: baseline.epoch,
                    transaction: baseline.frame.id,
                    cut: 0,
                    membership: self.membership,
                    start: baseline.start,
                });
                row.baseline = None;
            }
            // A musical seal does not close input publication. The live
            // producer can still capture post-cut input before its enclosing
            // detach boundary; only Detach certifies that transfer has ended.
            if row.detach.is_some_and(|cut| row.seal == Some(cut) && row.applied == cut)
                && row.output.len() == 0
                && row.state.count() == 0
                && row.baseline.is_none()
                && row.ingress.len() == 0
                && row.captures.empty()
                && !offer.session.rows[index].hub_detached.swap(true, Ordering::AcqRel)
            {
                self.service_revision = self.service_revision.wrapping_add(1);
            }
        }
    }
    pub fn retired_settled(&self) -> bool {
        if self.direct_ingress.len() != 0 || !self.direct_captures.empty() {
            return false;
        }
        self.direct.settled()
            && self.rows.iter().all(|r| {
                r.output.len() == 0
                    && r.baseline.is_none()
                    && r.state.count() == 0
                    && r.ingress.len() == 0
                    && r.captures.empty()
                    && (r.lease.is_none() || r.seal == Some(r.applied))
            })
            && self.offer.as_ref().is_none_or(|offer| {
                offer.session.credits.load(Ordering::Acquire) == 0
                    && offer.session.rows.iter().all(|row| {
                        row.source_detached.load(Ordering::Acquire)
                            && row.hub_detached.load(Ordering::Acquire)
                            && row.emission_gate.load(Ordering::Acquire) != BUSY
                    })
            })
            && self.retired_publication.as_ref().is_none_or(|(owner, _, _)| {
                owner.direct.pending().is_none() && !owner.recording.publication_debt()
            })
    }

    fn retired_streams_published(&self, owner: &Owner) -> bool {
        self.direct.joined_cut().is_some()
            && !self.clock_loss_pending
            && owner.direct.pending().is_none()
            && self.rows.iter().enumerate().all(|(index, row)| {
                if row.lease.is_none() {
                    return self.offer.as_ref().is_none_or(|offer| {
                        offer.session.rows[index].source_detached.load(Ordering::Acquire)
                    });
                }
                row.seal.or(row.producer_joined).is_some_and(|cut| row.applied == cut)
                    && row.output.len() == 0
                    && row.baseline.is_none()
            })
    }
    pub fn retire_publication(
        &mut self,
        mut owner: Box<Owner>,
        mut recorder: Recorder,
        observation: f64,
    ) {
        assert!(self.retired_publication.is_none());
        self.direct.join_producer();
        recorder.hold_retired_publication();
        owner.recording.dispose_retired_configuration(&mut recorder, &owner.timeline);
        if self.shared.registration().is_none() {
            assert!(self.offer.is_none());
            owner.publish_retired_direct(&mut recorder, observation);
            owner
                .recording
                .finish_retired_publication(&mut recorder, self.direct.unknown_joined_wire_state());
            return;
        }
        self.retired_publication = Some((owner, recorder, observation));
    }
}

#[cfg(all(test, not(feature = "tuning-probe")))]
impl Hub {
    pub fn test_row_receiver(&self, slot: usize, channel: usize) -> (Option<u8>, usize, u64, u64) {
        let row = &self.rows[slot];
        let state = &row.state.channels()[channel];
        (
            (state.controller_valid[1] & (1 << 24) != 0).then_some(state.controllers[88]),
            row.state.count(),
            row.received,
            row.applied,
        )
    }
    pub fn test_row_retirement(&self, slot: usize) -> (u64, u64, usize, Option<(u64, i64)>) {
        let row = &self.rows[slot];
        (
            row.received,
            row.applied,
            row.output.len(),
            row.baseline.as_ref().map(|baseline| {
                (baseline.frame.output_cut, (baseline.frame.time * self.rate).round() as i64)
            }),
        )
    }
    pub fn test_joined_rows(&self) -> [(Option<Lease>, Option<u64>, bool, u64); TUNERS] {
        std::array::from_fn(|index| {
            let row = &self.rows[index];
            (row.lease, row.producer_joined, row.joined_unknown_wire, row.applied)
        })
    }
    pub fn test_rebase_output_prefix(&mut self, lease: Lease, prefix: u64) {
        let row = self.rows.iter_mut().find(|row| row.lease == Some(lease)).unwrap();
        assert_eq!(row.output.len(), 0);
        assert_eq!(row.applied, row.received);
        assert!(row.baseline.is_none() && row.report.is_none());
        row.received = prefix;
        row.applied = prefix;
        row.last_ack = Some((prefix, row.coverage.unwrap().through));
    }
    pub fn print_test_memory_layout(&self) {
        use std::mem::{size_of, size_of_val};
        println!(
            "LEDGER hub [owner,row,row_backing,state,baseline_option] {:?}",
            [
                size_of::<Self>(),
                size_of::<Row>(),
                size_of_val(&*self.rows),
                size_of::<State>(),
                size_of::<Option<Baseline>>()
            ]
        );
        println!(
            "LEDGER hub row queues [cell,count,backing] output={:?} ingress={:?}",
            self.rows[0].output.test_layout(),
            self.rows[0].ingress.test_layout()
        );
        println!(
            "LEDGER DIRECT capture ingress [cell,count,backing] {:?}",
            self.direct_ingress.test_layout()
        );
    }
}

impl Hub {
    fn collect_direct_captures(&mut self) {
        let Some(lease) = self.direct.capture_lease() else {
            return;
        };
        for _ in 0..256 {
            if self.direct_ingress.free() == 0 {
                break;
            }
            let Some(units) = self.direct.direct_capture_units() else {
                break;
            };
            if self.input_work + units > 4096 {
                break;
            }
            self.input_work += units;
            let Some(token) = self.direct.take_direct_capture() else {
                break;
            };
            self.direct_captures.accept(&token, lease, token.key.epoch);
            self.direct_ingress
                .push(Intent::Capture(token))
                .unwrap_or_else(|_| unreachable!("checked DIRECT ingress capacity"));
            self.service_revision = self.service_revision.wrapping_add(1);
        }
        if self.direct_capture_left == 0 {
            self.direct_capture_cursor = self.direct_ingress.front_position();
            self.direct_capture_left = self.direct_ingress.len();
        }
        for _ in 0..64 {
            let Some(position) = self.direct_capture_cursor else {
                break;
            };
            let units = match self.direct_ingress.at_ref(position) {
                Some(Intent::Capture(token)) if !self.capture_hold && token.frozen.is_none() => {
                    token.units()
                }
                _ => 1,
            };
            if self.input_work + units > 4096 {
                break;
            }
            self.input_work += units;
            self.direct_capture_left -= 1;
            self.direct_capture_cursor = if self.direct_capture_left == 0 {
                None
            } else {
                self.direct_ingress.next_position(position)
            };
            let Some(Intent::Capture(token)) = self.direct_ingress.at_ref(position) else {
                unreachable!();
            };
            if self.capture_hold || token.frozen.is_some() {
                continue;
            }
            let Intent::Capture(token) = self.direct_ingress.remove(position).unwrap() else {
                unreachable!();
            };
            let retirement = self.direct_captures.retire(token);
            self.direct.retire_direct_capture(retirement);
            self.service_revision = self.service_revision.wrapping_add(1);
        }
    }

    /// The future scheduler supplies the complete cut. This stage provides only
    /// persistent metadata/target ownership; calling this proves no frontier.
    #[allow(dead_code)] // Persistent ownership API; scheduler integration follows.
    pub(super) fn freeze_captures(
        &mut self,
        sample: i64,
    ) -> Result<harmonigraph_core::cohort::FrozenInputId, harmonigraph_core::cohort::Error> {
        use super::capture::View;
        let windows =
            std::iter::once(&self.direct_ingress).chain(self.rows.iter().map(|row| &row.ingress));
        let count = windows.map(|window| {
            let mut position = window.front_position();
            let mut count = 0;
            while let Some(index) = position {
                if matches!(window.at_ref(index), Some(Intent::Capture(token)) if token.sample == sample) { count += 1; }
                position = window.next_position(index);
            }
            count
        }).sum();
        let id = self.frozen_captures.begin(sample, count)?;
        let direct = (&mut self.direct_ingress, &self.direct_captures, self.direct.capture_lease());
        let rows = std::iter::once(direct)
            .chain(self.rows.iter_mut().map(|row| (&mut row.ingress, &row.captures, row.lease)));
        for (window, permissions, lease) in rows {
            let Some(lease) = lease else {
                continue;
            };
            let mut position = window.front_position();
            while let Some(index) = position {
                position = window.next_position(index);
                let epoch = match window.at_mut(index) {
                    Some(Intent::Capture(token)) if token.sample == sample => {
                        token.frozen = Some(id);
                        token.key.epoch
                    }
                    _ => continue,
                };
                let view = View { ingress: window, permissions, lease, epoch, frozen: id };
                self.frozen_captures.push(view.metadata(index).unwrap());
            }
        }
        self.capture_hold = false;
        Ok(id)
    }
    #[allow(dead_code)] // Persistent ownership API; scheduler integration follows.
    fn capture_targets<'a>(
        rows: &'a [Row; TUNERS],
        direct: &'a Window<Intent, INTENT_RING>,
        direct_permissions: &'a super::capture::Permissions,
        direct_lease: Option<Lease>,
        epoch: u64,
        id: harmonigraph_core::cohort::FrozenInputId,
    ) -> super::capture::Targets<'a> {
        super::capture::Targets {
            rows: std::array::from_fn(|index| {
                if index == 0 {
                    direct_lease.map(|lease| super::capture::View {
                        ingress: direct,
                        permissions: direct_permissions,
                        lease,
                        epoch,
                        frozen: id,
                    })
                } else {
                    let row = &rows[index - 1];
                    row.lease.map(|lease| super::capture::View {
                        ingress: &row.ingress,
                        permissions: &row.captures,
                        lease,
                        epoch: row.epoch,
                        frozen: id,
                    })
                }
            }),
        }
    }
    #[allow(dead_code)] // Persistent ownership API; scheduler integration follows.
    pub(super) fn advance_captures(
        &mut self,
        units: usize,
    ) -> Result<harmonigraph_core::cohort::Progress, harmonigraph_core::cohort::Error> {
        let targets = Self::capture_targets(
            &self.rows,
            &self.direct_ingress,
            &self.direct_captures,
            self.direct.capture_lease(),
            self.publication_clock.epoch,
            self.frozen_captures.id,
        );
        self.frozen_captures.advance(&targets, units)
    }
    #[allow(dead_code)] // Persistent ownership API; scheduler integration follows.
    pub(super) fn commit_capture(&mut self) -> Result<(), harmonigraph_core::cohort::Error> {
        let targets = Self::capture_targets(
            &self.rows,
            &self.direct_ingress,
            &self.direct_captures,
            self.direct.capture_lease(),
            self.publication_clock.epoch,
            self.frozen_captures.id,
        );
        self.frozen_captures.commit(&targets)
    }
    pub(super) fn release_captures(&mut self, joined: bool) {
        if !self.frozen_captures.active && !self.capture_hold {
            return;
        }
        self.frozen_captures.end(joined);
        for window in std::iter::once(&mut self.direct_ingress)
            .chain(self.rows.iter_mut().map(|row| &mut row.ingress))
        {
            let mut position = window.front_position();
            while let Some(index) = position {
                position = window.next_position(index);
                if let Some(Intent::Capture(token)) = window.at_mut(index) {
                    token.frozen = None;
                }
            }
        }
        self.capture_hold = false;
    }
}

#[cfg(all(test, not(feature = "tuning-probe")))]
impl Hub {
    pub fn test_pause_captures(&mut self) {
        self.capture_hold = true;
    }
    pub fn test_hold_captures(&mut self, sample: i64) {
        self.capture_hold = true;
        self.test_capture_request = Some(sample);
    }
    fn test_capture_tick(&mut self) {
        if let Some(sample) = self.test_capture_request.take() {
            self.freeze_captures(sample).unwrap();
        }
        if self.frozen_captures.active {
            if std::mem::take(&mut self.test_capture_commit) {
                self.commit_capture().unwrap();
            }
            self.test_capture_result = Some(self.advance_captures(4096));
        }
    }
    pub fn test_capture_metadata(
        &self,
        source: u8,
        serial: u64,
    ) -> Option<(
        usize,
        super::capture::Key,
        harmonigraph_core::cohort::Event,
        [Option<harmonigraph_core::cohort::TargetLink>; 64],
    )> {
        use harmonigraph_core::cohort::TargetAccess;
        let targets = Self::capture_targets(
            &self.rows,
            &self.direct_ingress,
            &self.direct_captures,
            self.direct.capture_lease(),
            self.publication_clock.epoch,
            self.frozen_captures.id,
        );
        let view = targets.rows[source as usize].as_ref()?;
        let mut position = view.ingress.front_position();
        while let Some(index) = position {
            position = view.ingress.next_position(index);
            let Some(Intent::Capture(token)) = view.ingress.at_ref(index) else {
                continue;
            };
            if token.key.serial != serial {
                continue;
            }
            let metadata = view.metadata(index)?;
            let mut next = metadata.targets.first;
            let links = std::array::from_fn(|_| {
                let link = view.get(source, next);
                if let Some(link) = link {
                    next = link.next;
                }
                link
            });
            return Some((index, token.key, metadata, links));
        }
        None
    }
    pub fn test_capture_keys(&self, source: usize) -> Vec<super::capture::Key> {
        let window =
            if source == 0 { &self.direct_ingress } else { &self.rows[source - 1].ingress };
        let mut keys = Vec::new();
        let mut position = window.front_position();
        while let Some(index) = position {
            position = window.next_position(index);
            match window.at_ref(index).unwrap() {
                Intent::Capture(token) => keys.push(token.key),
                Intent::CaptureRetirement(retirement) => keys.push(retirement.key),
                _ => {}
            }
        }
        keys
    }
}

#[cfg(all(test, not(feature = "tuning-probe")))]
impl Hub {
    pub fn test_capture_phases(&self, source: usize) -> (usize, usize) {
        let window =
            if source == 0 { &self.direct_ingress } else { &self.rows[source - 1].ingress };
        let mut position = window.front_position();
        let mut counts = (0, 0);
        while let Some(index) = position {
            position = window.next_position(index);
            match window.at_ref(index).unwrap() {
                Intent::Capture(_) => counts.0 += 1,
                Intent::CaptureRetirement(_) => counts.1 += 1,
                _ => {}
            }
        }
        counts
    }
}

#[cfg(all(test, not(feature = "tuning-probe")))]
impl Hub {
    pub fn test_frozen_id(&self) -> harmonigraph_core::cohort::FrozenInputId {
        self.frozen_captures.id
    }
    pub fn test_capture_lookup(
        &self,
        source: u8,
        binding: harmonigraph_core::cohort::FrozenInputId,
        handle: u32,
    ) -> Option<harmonigraph_core::cohort::TargetLink> {
        use harmonigraph_core::cohort::TargetAccess;
        Self::capture_targets(
            &self.rows,
            &self.direct_ingress,
            &self.direct_captures,
            self.direct.capture_lease(),
            self.publication_clock.epoch,
            binding,
        )
        .get(source, handle)
    }
    pub fn test_fill_reply_with_old_ack(&mut self) {
        let row = &self.rows[0];
        let (cut, complete_through) = row.last_ack.unwrap();
        let reply = Reply::OutputRetained {
            incarnation: row.lease.unwrap().incarnation,
            epoch: row.epoch,
            cut,
            complete_through,
        };
        let replies = &mut self.offer.as_mut().unwrap().bank.rows[0].replies;
        while replies.push(reply).is_ok() {}
    }
    pub fn test_disposition_cursor(&self) -> (usize, Option<u64>) {
        let row = &self.rows[0];
        let mut count = 0;
        let mut next = row.ingress.front_position();
        while let Some(position) = next {
            if matches!(row.ingress.at_ref(position), Some(Intent::Disposition { .. })) {
                count += 1;
            }
            next = row.ingress.next_position(position);
        }
        let cursor = row.ingress_cursor.and_then(|position| match row.ingress.at_ref(position) {
            Some(Intent::Disposition { transaction, .. }) => Some(*transaction),
            _ => None,
        });
        (count, cursor)
    }
    pub fn test_repeat_capture_retirement(&mut self, key: super::capture::Key) {
        self.offer.as_mut().unwrap().bank.rows[key.lease.slot as usize - 1]
            .replies
            .push(Reply::CaptureRetired(key))
            .unwrap();
    }
}
