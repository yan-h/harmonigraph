//! Audio-owned collection and canonical publication. Musical retention never
//! waits for the display, file drainer, an editor, or registry bookkeeping.
use super::{
    clock::{Clock, Coverage},
    protocol::*,
    queue::Queue,
    registry::HubOffer,
    setup,
    source::{Source, BUSY, CLOSED, OPEN},
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

mod sequencing;

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
    /// Copied input records this row has received and not yet sequenced. The
    /// Hub owns every one of them outright.
    inputs: Queue<Capture, CAPTURES_PER_SOURCE>,
    last_disposition: Option<u64>,
    input_coverage: Option<(Coverage, u64)>,
    input_settled: (u64, u64),
    terminal_cut: Option<u64>,
    input_membership: u64,
    acknowledged_membership: u64,
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
            inputs: Queue::default(),
            last_disposition: None,
            input_coverage: None,
            input_settled: (0, 0),
            terminal_cut: None,
            input_membership: 0,
            acknowledged_membership: 0,
            seal: None,
            producer_joined: None,
            joined_unknown_wire: false,
            seal_generation: 0,
            last_sealed_ack: None,
            joining: None,
        }
    }
}
impl Row {
    fn input_progress(&mut self, coverage: Coverage, input_cut: u64, membership: u64) {
        let new_segment = self.coverage.is_some_and(|output| output.start == coverage.start)
            && self.input_coverage.is_none_or(|(old, _)| coverage.start >= old.through);
        if self.input_coverage.is_none_or(|(old, cut)| {
            input_cut >= cut
                && ((old.start == coverage.start && old.through <= coverage.through) || new_segment)
        }) {
            self.input_coverage = Some((coverage, input_cut));
            self.input_membership = membership;
        }
    }
}
pub struct Hub {
    trace: Box<super::diagnostics::Counts>,
    #[cfg(test)]
    pub test_aggregation: bool,
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
    batch: Box<super::capture::Batch>,
    direct_inputs: Queue<Capture, CAPTURES_PER_SOURCE>,
    sequencer: Box<sequencing::Sequencer>,
}
// Charged owner upper bounds apply in production builds too, where the fixture
// freeze controls are absent. Larger backing cells have their own assertions.
const _: () = assert!(std::mem::size_of::<Hub>() <= 1136);
const _: () = assert!(std::mem::size_of::<Row>() <= 31200 + 2 * 64 * 8);
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
        let mut diagnostics = self.direct.diagnostics();
        if let Some(offer) = &self.offer {
            diagnostics |= offer.session.faults.load(Ordering::Acquire);
            for row in &offer.session.rows {
                diagnostics |= row.faults.load(Ordering::Acquire);
            }
        }
        self.shared.status.store(diagnostics, Ordering::Release);
        self.shared.extra_delay.fetch_max(self.sequencer.extra_delay, Ordering::Relaxed);
        if self.trace.due(callback.frames, self.rate) {
            self.publish_diagnostics(callback, owner);
            self.shared.request_main();
        }
    }
    fn publish_diagnostics(&self, callback: api::Callback, owner: &Owner) {
        self.direct.publish_diagnostics(callback);
        let config = owner.timeline.reducer().resolved();
        self.shared.diagnostics.hub.as_ref().unwrap().publish([
            self.offer.as_ref().map_or(0, |offer| offer.session.runtime) as i64,
            self.publication_clock.epoch as i64,
            self.transition.map_or(0, |update| update.generation) as i64,
            self.trace.setup_wait,
            i64::from(self.clock.valid),
            i64::from(self.invalidated),
            self.sequencer.decision as i64,
            self.publication_through.unwrap_or(i64::MIN),
            callback.steady_time.saturating_add(i64::from(callback.frames)),
            self.trace.output_on as i64,
            self.trace.output_off as i64,
            self.trace.last_source as i64,
            self.trace.last_output_key.map_or(-1, i64::from),
            self.trace.last_output_pitch,
            config.revision as i64,
            i64::from(config.tuning.c_offset),
            i64::from(config.tuning.three),
            i64::from(config.tuning.five),
            i64::from(config.tuning.seven),
            i64::from(config.modes.tempered.syntonic),
            i64::from(config.modes.tempered.septimal_kleisma),
            i64::from(config.modes.auto[0]),
            i64::from(config.modes.auto[1]),
            i64::from(config.modes.learning),
            self.trace.input_wait,
            self.trace.input_source as i64,
            self.trace.publication_wait,
            self.trace.publication_source as i64,
            self.offer.as_ref().map_or(0, |offer| offer.session.credits.load(Ordering::Acquire))
                as i64,
        ]);
        let Some(offer) = &self.offer else { return };
        for (index, (row, snapshot)) in
            self.rows.iter().zip(self.shared.diagnostics.rows.as_ref().unwrap().iter()).enumerate()
        {
            let shared = &offer.session.rows[index];
            snapshot.publish([
                row.lease.map_or(0, |lease| lease.source.0) as i64,
                shared.expected_incarnation.load(Ordering::Acquire) as i64,
                i64::from(row.member),
                row.joining.unwrap_or(i64::MIN),
                row.coverage.map_or(i64::MIN, |coverage| coverage.through),
                row.input_coverage.map_or(i64::MIN, |(coverage, _)| coverage.through),
                row.received as i64,
                row.applied as i64,
                row.output.len() as i64,
                row.inputs.len() as i64,
                row.baseline.as_ref().map_or(-1, |baseline| baseline.frame.output_cut as i64),
                row.state.count() as i64,
                row.acknowledged_membership as i64,
                row.input_membership as i64,
                shared.emission_gate.load(Ordering::Acquire) as i64,
                i64::from(shared.withdrawn.load(Ordering::Acquire)),
                i64::from(shared.source_detached.load(Ordering::Acquire)),
                i64::from(shared.hub_detached.load(Ordering::Acquire)),
                row.seal.map_or(-1, |cut| cut as i64),
                row.producer_joined.map_or(-1, |cut| cut as i64),
                i64::from(shared.faults.load(Ordering::Acquire)),
            ]);
        }
    }
    fn fence_rows(&self, generation: u64) {
        if let Some(offer) = &self.offer {
            offer.session.closing.store(generation, Ordering::Release);
            for row in &offer.session.rows {
                row.withdrawn.store(true, Ordering::Release);
                row.emission_gate.fetch_or(CLOSED, Ordering::AcqRel);
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
                    && row.emission_gate.load(Ordering::Acquire) & BUSY == 0
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
        // Plugin::reset has cleared observed input. With an offer, force_reset
        // also proved idle ownership; without one, clearing this observation
        // cache must leave forwarding's independent pending/debt state intact.
        // Host Reset is its own synchronous boundary, not the previous audio
        // callback's exhausted output grant.
        if !self.sequencer.can_reset_clock_context() {
            self.configuration_exhausted();
            return;
        }
        self.sequencer.clear_clock_context();
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
            row.emission_gate.fetch_or(CLOSED, Ordering::AcqRel);
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
        if self.sequences_inputs() {
            self.collect_direct_captures();
        }
    }
    pub fn configuration_exhausted(&mut self) {
        if let Some(offer) = &self.offer {
            offer.session.faults.fetch_or(super::source::STORAGE_FAULT, Ordering::AcqRel);
            for row in &offer.session.rows {
                row.emission_gate.fetch_or(CLOSED, Ordering::AcqRel);
            }
        }
        self.direct.fault(super::source::STORAGE_FAULT);
    }
    pub fn new() -> Box<Self> {
        let shared = setup::Shared::hub();
        let direct = Source::new(shared.clone());
        Box::new(Self {
            trace: Box::default(),
            #[cfg(test)]
            test_aggregation: false,
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
            batch: Box::default(),
            direct_inputs: Queue::default(),
            sequencer: Box::default(),
        })
    }
    pub fn activate(&mut self, rate: f64, frames: u32) {
        let first = self.rate == 0.0;
        self.rate = rate;
        self.max_frames = frames;
        if !first {
            self.clock.sample_rate = rate;
            self.clock.max_frames = frames;
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
        self.sequencer.work = 0;
        self.plan_callback();
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
        if coverage.is_none()
            || self.offer.as_ref().is_some_and(|offer| {
                offer.session.faults.load(Ordering::Acquire) & super::source::CLOCK_FAULT != 0
            })
        {
            self.force_reset(owner, false);
        }
        self.direct.begin(callback);
        if !self.sequences_inputs() {
            self.collect_direct_captures();
        }
        self.collect();
        self.observe_terminal_faults();
    }

    fn commit_transition(&mut self, owner: &mut Owner, recorder: &mut Recorder, observation: f64) {
        self.trace.setup_wait = 0;
        let Some(update) = self.transition else {
            return;
        };
        if self.sequencer.terminal_session && !update.reset {
            self.trace.setup_wait = 1;
            return;
        }
        if !self.sequencer.can_reset_clock_context() {
            self.trace.setup_wait = 2;
            self.configuration_exhausted();
            return;
        }
        if !update.routing.calibration().matches(self.rate, self.max_frames)
            || !self.direct.transition_settled()
            || owner.direct.pending().is_some()
        {
            self.trace.setup_wait =
                if !update.routing.calibration().matches(self.rate, self.max_frames) {
                    3
                } else if !self.direct.transition_settled() {
                    4
                } else {
                    5
                };
            return;
        }
        let Some(offer) = &self.offer else {
            self.trace.setup_wait = 6;
            return;
        };
        if offer.session.rows.iter().any(|row| {
            row.emission_gate.load(Ordering::Acquire) & BUSY != 0
                || !row.source_detached.load(Ordering::Acquire)
                || !row.hub_detached.load(Ordering::Acquire)
        }) || self.rows.iter().any(|row| {
            row.output.len() != 0
                || row.inputs.len() != 0
                || row.baseline.is_some()
                || row.state.count() != 0
        }) || offer.session.credits.load(Ordering::Acquire) != 0
        {
            self.trace.setup_wait = 7;
            return;
        }
        // Both factual identities and prospective context cross this boundary
        // together, after all old ownership settles. Budget the clear/reseed
        // before committing anything; the directory itself needs no scan.
        if self.collected + HELD_SESSION + 64 > 4096 {
            self.trace.setup_wait = 8;
            return;
        }
        self.collected += HELD_SESSION + 64;
        // End-of-callback source acknowledgements still use the old clock.
        // Finish every old recording route before changing either offset.
        owner.finish_recording_publication(recorder, observation);
        if self.invalidated {
            owner.recording.close_invalidated(recorder);
        }
        let Some(epoch) = self.publication_clock.epoch.checked_add(1) else {
            self.trace.setup_wait = 9;
            return;
        };
        let clock = ClockId { epoch, ..self.publication_clock };
        let offset = update.routing.calibration().offset;
        if !owner.recording.commit_clock(clock, offset) {
            self.trace.setup_wait = 10;
            return;
        }
        self.direct.commit_clock_setup(update, epoch);
        owner.resume_clock(offset, self.invalidated);
        self.invalidated = false;
        self.clock = Clock::new(update.routing.calibration(), self.rate, self.max_frames);
        self.anchor = None;
        self.publication_through = None;
        self.publication_clock = clock;
        self.transition = None;
        self.sequencer.terminal_session = false;
        self.sequencer.terminal_sources = 0;
        self.sequencer.reset_clock_context(
            self.direct.capture_lease().unwrap(),
            epoch,
            &owner.direct.state,
        );
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
        let sequencing = self.sequences_inputs();
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
                && row.inputs.len() == 0
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
                self.sequencer.participating[index + 1] = true;
                self.sequencer.participation_serial[index + 1] = 0;
                self.sequencer.history.clear(index + 1, self.sequencer.decision);
                row.repair = false;
                row.baseline_id = 0;
                row.detach = None;
                row.last_ack = None;
                row.input_coverage = None;
                row.input_settled = (0, 0);
                row.terminal_cut = None;
                self.sequencer.terminal_sources &= !(1 << index);
                row.input_membership = 0;
                row.acknowledged_membership = 0;
                self.sequencer.captured[index + 1] = 0;
                row.seal = None;
                row.producer_joined = None;
                row.joined_unknown_wire = false;
                row.last_sealed_ack = None;
                row.joining = None;
            }
            // This lane never waits for Capture ingress or an older Progress
            // report. Reserve cancellation ACK before consuming its authority.
            if self.input_work < 4096 {
                let ack = shared.to_source.reserve_repair();
                let repair = shared.to_hub.take_repair_if(|control| match control {
                    Control::Disposition { incarnation, epoch, transaction, .. }
                        if row.lease.is_some_and(|lease| lease.incarnation == incarnation)
                            && row.epoch == epoch
                            && transaction != 0 =>
                    {
                        ack.is_some()
                            && row.last_disposition.is_none_or(|last| {
                                transaction <= last || last.checked_add(1) == Some(transaction)
                            })
                    }
                    _ => true,
                });
                if let Some(control) = repair {
                    self.input_work += 1;
                    self.service_revision = self.service_revision.wrapping_add(1);
                    match control {
                        Control::Disposition { incarnation, epoch, transaction, input_cut, lifetime, request, original_on }
                            if row.lease.is_some_and(|lease| lease.incarnation == incarnation)
                                && row.epoch == epoch && transaction != 0 => {
                            ack.unwrap().publish(Reply::Disposition { incarnation, transaction, input_cut });
                            row.last_disposition = Some(row.last_disposition.map_or(transaction, |old| old.max(transaction)));
                            if original_on && (sequencing || self.sequencer.retired)
                                && !self.sequencer.cancel(index, row.lease.unwrap(), epoch, input_cut, request, lifetime) {
                                shared.faults.fetch_or(super::source::STORAGE_FAULT, Ordering::AcqRel);
                            }
                        }
                        _ => {}
                    }
                }
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
                    Control::Adopt { lease, epoch, coverage, output_cut, input_start_cut }
                        if lease.session == offer.session.runtime
                            && lease.incarnation
                                == shared.expected_incarnation.load(Ordering::Acquire)
                            && epoch == offer.session.epoch.load(Ordering::Acquire) =>
                    {
                        if row.lease.is_none() {
                            row.lease = Some(lease);
                            row.epoch = epoch;
                            row.coverage =
                                Some(Coverage { start: coverage.start, through: coverage.start });
                            // The first callback's actual progress travels with
                            // Adopt in its one normal cell. A second message is
                            // not needed to preserve the initial join boundary.
                            row.report = Some((coverage, output_cut));
                            self.sequencer.captured[index + 1] = input_start_cut;
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
                if self.input_work == 4096 {
                    break;
                }
                match offer.bank.rows[index].intents.peek() {
                    Ok(Intent::Capture(_)) if row.inputs.free() == 0 => break,
                    Ok(_) => {}
                    Err(_) => break,
                }
                let Ok(intent) = offer.bank.rows[index].intents.pop() else {
                    break;
                };
                self.input_work += 1;
                self.service_revision = self.service_revision.wrapping_add(1);
                match intent {
                    Intent::Coverage { incarnation, epoch, coverage, input_cut, membership } => {
                        if row.lease.is_some_and(|lease| lease.incarnation == incarnation)
                            && row.epoch == epoch
                        {
                            row.input_progress(coverage, input_cut, membership);
                        }
                    }
                    Intent::InputSettled { incarnation, epoch, input_cut, output_cut } => {
                        if row.lease.is_some_and(|lease| lease.incarnation == incarnation)
                            && row.epoch == epoch
                            && input_cut >= row.input_settled.0
                            && output_cut >= row.input_settled.1
                        {
                            row.input_settled = (input_cut, output_cut);
                        }
                    }
                    // A record whose lease or epoch has moved on belongs to a
                    // session this row no longer has; the copy is simply
                    // dropped, and no retirement is owed for it.
                    Intent::Capture(record) => {
                        if row.lease == Some(record.lease) && row.epoch == record.epoch {
                            self.sequencer.captured[index + 1] = record.serial;
                            row.inputs
                                .push(record)
                                .unwrap_or_else(|_| unreachable!("checked input capacity"));
                        }
                    }
                }
            }
            // The Hub owns every copied record it holds. A terminated row's
            // records are dropped: its stream is settled where the fault
            // latched, and nothing is left to sequence them against.
            if row.terminal_cut.is_some() && row.inputs.len() != 0 {
                row.inputs.clear();
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
            if !row.member && row.terminal_cut.is_none() {
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
                && shared.faults.load(Ordering::Acquire) & !super::source::TIMING_FAILURE == 0
                && offer.session.faults.load(Ordering::Acquire) & !super::source::TIMING_FAILURE
                    == 0
            {
                row.member = true;
                self.membership = self.membership.saturating_add(1);
                row.joining = None;
                if !shared.withdrawn.load(Ordering::Acquire)
                    && shared.faults.load(Ordering::Acquire) & !super::source::TIMING_FAILURE == 0
                    && offer.session.faults.load(Ordering::Acquire) & !super::source::TIMING_FAILURE
                        == 0
                {
                    let _ = shared.emission_gate.compare_exchange(
                        CLOSED,
                        OPEN,
                        Ordering::AcqRel,
                        Ordering::Acquire,
                    );
                }
            }
            if shared.withdrawn.load(Ordering::Acquire) {
                shared.emission_gate.fetch_or(CLOSED, Ordering::AcqRel);
            }
        }
        self.rotation = (self.rotation + 1) % TUNERS;
    }

    pub fn publish(&mut self, owner: &mut Owner, recorder: &mut Recorder, observation: f64) {
        // Apply newly known accepted output before any dependent policy call.
        // Incomplete publication may return early; sequencing still gets its
        // independent bounded turn to advance input/recovery ownership.
        self.publish_output(owner, recorder, observation);
        self.sequence_inputs(owner, recorder);
    }

    fn publish_output(&mut self, owner: &mut Owner, recorder: &mut Recorder, observation: f64) {
        self.trace.publication_wait = 0;
        self.trace.publication_source = 0;
        let Some(callback) = self.callback else {
            self.trace.publication_wait = 1;
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
                if start <= through {
                    self.trace.publication_wait = 2;
                    self.trace.publication_source = index + 1;
                }
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
                    self.trace.publication_wait = 3;
                    self.trace.publication_source = index + 1;
                    return;
                };
                if coverage.through <= through {
                    self.trace.publication_wait = 4;
                    self.trace.publication_source = index + 1;
                }
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
                let Some(lookup) = self.sequencer.lookup_output(
                    self.rows[index].lease.unwrap(),
                    self.rows[index].output.front().unwrap(),
                    &mut self.collected,
                ) else {
                    break;
                };
                let row = &mut self.rows[index];
                let value = row.output.pop().unwrap();
                row.channel_witness = None;
                if !value.outcome.is_wire()
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
                let actual_voice = row.state.voice(value.lifetime).copied();
                if !self.sequencer.actual_output(lookup, actual_voice) {
                    self.configuration_exhausted();
                }
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
            let lookup = if index == 0 {
                self.sequencer.lookup_direct(direct.unwrap(), &mut self.collected)
            } else if index <= TUNERS {
                self.sequencer.lookup_output(
                    self.rows[index - 1].lease.unwrap(),
                    self.rows[index - 1].output.front().unwrap(),
                    &mut self.collected,
                )
            } else {
                None
            };
            if index <= TUNERS && lookup.is_none() {
                self.trace.publication_wait = 5;
                self.trace.publication_source = index;
                break;
            }
            self.merged += 1;
            if index == 0 {
                let delta = direct.unwrap();
                if !self.sequencer.actual_direct(delta, lookup.unwrap()) {
                    self.configuration_exhausted();
                }
                let route = owner
                    .recording_route(delta.timing.unwrap(), delta.event.time)
                    .unwrap_or_else(|_| {
                        recorder.fail_configuration();
                        Default::default()
                    });
                if recorder.publish_note(delta, observation, route).is_err() {
                    owner.direct.recovery = true;
                } else {
                    self.trace.published(delta);
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
                let assignment = self.output_assignment(index, value);
                let time = self.presentation(value.actual);
                let input_time = self.presentation(value.input);
                let timing = EventTiming {
                    clock,
                    input: value.input,
                    planned: assignment.map(|(_, planned)| planned),
                    sample: value.actual,
                    sample_rate: self.rate,
                };
                let row = &mut self.rows[index];
                let delta = row.state.apply(
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
                );
                if let Some((binding, _)) = assignment.filter(|_| {
                    !value.outcome.partial()
                        && (value.event.attack().is_some()
                            || matches!(
                                value.event,
                                super::event::Event::Expression { kind: 2, .. }
                            ))
                }) {
                    let player = if value.event.attack().is_some() {
                        binding.initial_player
                    } else {
                        value.player
                    };
                    row.state.assignment(value.lifetime, binding, player);
                }
                if let Some(mut delta) = delta {
                    delta.assignment = row
                        .state
                        .voice(value.lifetime)
                        .and_then(harmonigraph_core::canonical::VoiceBaseline::metadata);
                    delta.partial_output = value.outcome.partial();
                    let route = owner.recording_route(timing, time).unwrap_or_else(|_| {
                        recorder.fail_configuration();
                        Default::default()
                    });
                    if recorder.publish_note(delta, observation, route).is_err() {
                        row.repair = true;
                    } else {
                        self.trace.published(delta);
                    }
                }
                row.applied = value.sequence;
                if value.outcome.partial() {
                    row.state.partial(value.lifetime);
                }
                Self::confirm(row, &mut owner.confirmed);
                let actual_voice = row.state.voice(value.lifetime).copied();
                if !self.sequencer.actual_output(lookup.unwrap(), actual_voice) {
                    self.configuration_exhausted();
                }
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
        if value.outcome.is_wire() {
            row.channel_witness = value.event.channel_termination().map(|choke| ChannelWitness {
                sequence: value.sequence,
                input: value.input,
                actual: value.actual,
                channel: value.event.channel().unwrap(),
                controller: if choke { 120 } else { 123 },
                derived: 0,
            });
            true
        } else if let Some((wire_sequence, controller)) = value.outcome.channel_terminal() {
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
        } else {
            false
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
        let sequenced = self.sequences_inputs();
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
                // Adopt and its first Progress can occupy different callbacks.
                // Keep the initial snapshot until collection has enrolled the
                // row; acknowledging it earlier loses the only join proof.
                if baseline.frame.output_cut == 0 && !row.member && row.terminal_cut.is_none() {
                    continue;
                }
                let sample = (baseline.frame.time * self.rate).round() as i64;
                if row.applied == baseline.frame.output_cut && sample < through {
                    // The factual companion is one atomic source replacement:
                    // clear at most256 entries and copy at most64 held entries.
                    if self.collected + HELD_SESSION + 64 > 4096 {
                        continue;
                    }
                    let Some(ack) = shared.to_source.reserve() else {
                        continue;
                    };
                    let transaction = baseline.frame.id;
                    let start = baseline.start;
                    let Some(id) = row.baseline_id.checked_add(1) else {
                        continue;
                    };
                    let mut frame = baseline.frame;
                    if sequenced {
                        if row.baseline_id == 0
                            && self.sequencer.participation_serial[index + 1] == 0
                        {
                            self.sequencer.participating[index + 1] = frame.participating;
                        }
                        // After enrollment only the ordered Original changes
                        // eligibility. A baseline can arrive before that marker
                        // or after a newer one; neither reverses its authority.
                        frame.participating = self.sequencer.participating[index + 1];
                    }
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
                        recorder.publication_lost(frame.time, route);
                    }
                    row.repair |= result.is_err();
                    row.state.replace(&frame);
                    self.collected += HELD_SESSION + 64;
                    if !self.sequencer.actual_baseline(lease, row.epoch, &frame) {
                        shared.faults.fetch_or(super::source::STORAGE_FAULT, Ordering::AcqRel);
                    }
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
                    row.acknowledged_membership = self.membership;
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
                    && row.inputs.len() == 0
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
                self.direct_inputs.len()
                    + self
                        .rows
                        .iter()
                        .map(|row| row.inputs.len() + row.output.len())
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
        self.batch.end();
        self.plan_callback();
        self.sequencer.work = 0;
        self.direct.retired_pump();
        self.collected = 0;
        self.input_work = 0;
        self.merged = 0;
        self.collect();
        self.service_plans();
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
                && row.inputs.len() == 0
                && !offer.session.rows[index].hub_detached.swap(true, Ordering::AcqRel)
            {
                self.service_revision = self.service_revision.wrapping_add(1);
            }
        }
    }
    pub fn retired_settled(&self) -> bool {
        if self.direct_inputs.len() != 0 {
            return false;
        }
        self.direct.settled()
            && self.rows.iter().all(|r| {
                r.output.len() == 0
                    && r.baseline.is_none()
                    && r.state.count() == 0
                    && r.inputs.len() == 0
                    && (r.lease.is_none() || r.seal == Some(r.applied))
            })
            && self.offer.as_ref().is_none_or(|offer| {
                offer.session.credits.load(Ordering::Acquire) == 0
                    && offer.session.rows.iter().all(|row| {
                        row.source_detached.load(Ordering::Acquire)
                            && row.hub_detached.load(Ordering::Acquire)
                            && row.emission_gate.load(Ordering::Acquire) & BUSY == 0
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
        self.sequencer.retired = true;
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

#[cfg(test)]
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
        self.sequencer.print_test_memory_layout();
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
            "LEDGER hub row queues [cell,count,backing] output={:?} inputs={:?}",
            self.rows[0].output.test_layout(),
            self.rows[0].inputs.test_layout()
        );
        println!(
            "LEDGER DIRECT copied inputs [cell,count,backing] {:?}",
            self.direct_inputs.test_layout()
        );
    }
}

impl Hub {
    /// The Hub's own MIDI reaches row zero as the same copied records the
    /// Tunes send, minus the ring: one owner hands them to another.
    fn collect_direct_captures(&mut self) {
        if self.direct.capture_lease().is_none() {
            return;
        }
        for _ in 0..256 {
            if self.direct_inputs.free() == 0 || self.input_work == 4096 {
                break;
            }
            let Some(record) = self.direct.take_direct_capture() else {
                break;
            };
            self.input_work += 1;
            self.sequencer.captured[0] = record.serial;
            self.direct_inputs
                .push(record)
                .unwrap_or_else(|_| unreachable!("checked DIRECT input capacity"));
            self.service_revision = self.service_revision.wrapping_add(1);
        }
        if self.sequencer.terminal_session && self.direct_inputs.len() != 0 {
            self.direct_inputs.clear();
        }
    }
}

#[cfg(test)]
impl Hub {
    pub fn test_input_sequence_progress(
        &self,
    ) -> (Option<(Coverage, u64)>, usize, bool, Option<i64>) {
        (self.rows[0].input_coverage, self.batch.len(), self.batch.active, self.sequencer.finalized)
    }
    #[allow(clippy::type_complexity)] // A compact snapshot of six independent row proofs.
    pub fn test_input_row(
        &self,
        source: usize,
    ) -> (bool, u64, u64, Option<(Coverage, u64)>, Option<Coverage>, Option<(Coverage, u64)>) {
        let row = &self.rows[source];
        (
            row.member,
            row.acknowledged_membership,
            row.input_membership,
            row.input_coverage,
            row.coverage,
            row.report,
        )
    }
    pub fn test_voice(
        &self,
        source: usize,
        lifetime: u64,
    ) -> Option<harmonigraph_core::canonical::VoiceBaseline> {
        self.rows[source].state.voice(lifetime).copied()
    }
    /// Copied records this row is holding, oldest first.
    pub fn test_inputs(&self, source: usize) -> Vec<Capture> {
        let queue = if source == 0 { &self.direct_inputs } else { &self.rows[source - 1].inputs };
        (0..queue.len()).filter_map(|offset| queue.get(offset)).collect()
    }
}
