//! Hub-owned live recovery. Old capture permissions, configuration bindings and
//! queued messages remain owned while independent emission cuts settle.
use super::*;
use crate::performance::slots::ReadGuard;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct Cuts {
    input: u64,
    output: u64,
    attempt: u64,
}

#[derive(Default)]
struct Participant {
    fence: Option<Fence>,
    sent: bool,
    cuts: Option<Cuts>,
    chunk: Option<ReadGuard<InventoryChunk, 1>>,
    cursor: usize,
    chunks: u32,
    total: Option<u32>,
    seen: u32,
    lifetime_cut: Option<u64>,
    inventory_acked: bool,
    resumed: bool,
}

#[derive(Default, Clone, Copy, Debug, PartialEq, Eq)]
enum Phase {
    #[default]
    Prepare,
    Inventory,
    Preserve,
    Status,
    Rebuild,
    Replay,
    Resume,
}

#[derive(Default)]
pub(super) struct Recovery {
    pub active: bool,
    transaction: u64,
    from: u64,
    pending_from: Option<u64>,
    ceiling: u64,
    horizon: Option<i64>,
    phase: Phase,
    participants: [Participant; TUNERS],
    plan_cursor: u32,
    plan_left: usize,
    source: usize,
    preserve: usize,
    pub work: usize,
    boundary: i64,
    context_cuts: [u64; TUNERS + 1],
    context_clock: ClockId,
    context_membership: u64,
    context_revision: u64,
    starts: [Option<usize>; TUNERS + 1],
    lengths: [usize; TUNERS + 1],
    heads: [Option<usize>; TUNERS + 1],
    left: [usize; TUNERS + 1],
    waiting: [bool; TUNERS + 1],
    status_cuts: [u64; TUNERS + 1],
}

impl Recovery {
    pub(super) fn lease(&self, source: usize) -> Option<Lease> {
        self.participants[source].fence.map(|fence| fence.lease)
    }
    pub(super) fn generation(&self, source: usize) -> Option<u64> {
        self.participants[source].fence?.generation.checked_add(4)
    }
    pub(super) fn old_decision(&self, decision: u64) -> bool {
        decision <= self.ceiling
    }
    pub(super) fn acknowledge(
        &mut self,
        source: usize,
        fence: Fence,
        input_cut: u64,
        output_cut: u64,
        settled_attempt: u64,
    ) {
        if !self.active {
            return;
        }
        let row = &mut self.participants[source];
        let cuts = Cuts { input: input_cut, output: output_cut, attempt: settled_attempt };
        if row.fence == Some(fence) && row.sent && row.cuts.is_none_or(|old| old == cuts) {
            row.cuts = Some(cuts);
        }
    }
}

impl Hub {
    #[cfg(all(test, not(feature = "tuning-probe")))]
    pub(in crate::performance) fn test_recovery_progress(&self) -> String {
        format!("active={} phase={:?} work={} heads={:?} waiting={:?} cuts={:?} plans={} decision={} frozen={}/{}/{}",
            self.sequencer.recovery.active, self.sequencer.recovery.phase, self.sequencer.recovery.work,
            self.sequencer.recovery.heads, self.sequencer.recovery.waiting, self.sequencer.recovery.status_cuts,
            self.sequencer.plan_count, self.sequencer.decision, self.frozen_captures.active,
            self.frozen_captures.sample, self.frozen_captures.len())
    }
    pub(in crate::performance::hub) fn request_recovery(&mut self, from: u64) {
        let from = from.max(1);
        if self.sequencer.recovery.active {
            let pending = &mut self.sequencer.recovery.pending_from;
            *pending = Some(pending.map_or(from, |old| old.min(from)));
            return;
        }
        let Some(transaction) = self.sequencer.recovery.transaction.checked_add(1) else {
            self.configuration_exhausted();
            return;
        };
        let Some(offer) = &self.offer else { return };
        let mut participants: [Participant; TUNERS] =
            std::array::from_fn(|_| Participant::default());
        for (index, row) in self.rows.iter().enumerate() {
            let Some(lease) = row.lease else { continue };
            let shared = &offer.session.rows[index];
            if shared.source_detached.load(Ordering::Acquire)
                && shared.hub_detached.load(Ordering::Acquire)
                && shared.emission_gate.load(Ordering::Acquire) & BUSY == 0
                && row.output.len() == 0
                && row.ingress.len() == 0
                && row.captures.empty()
                && row.baseline.is_none()
                && row.status_query.is_none()
                && row.state.complete
                && row.state.count() == 0
                && !row.state.pedals_held()
            {
                continue;
            }
            let generation =
                offer.session.rows[index].emission_gate.fetch_or(CLOSED, Ordering::AcqRel)
                    & !super::super::super::source::GATE_FLAGS;
            participants[index].fence = Some(Fence {
                lease,
                epoch: row.epoch,
                transaction,
                generation,
                from_decision: from,
            });
        }
        self.sequencer.recovery = Recovery {
            active: true,
            transaction,
            from,
            pending_from: None,
            ceiling: self.sequencer.decision,
            horizon: (self.frozen_captures.id.0 != 0).then_some(self.frozen_captures.sample),
            phase: Phase::Prepare,
            participants,
            plan_cursor: self.sequencer.plan_head,
            plan_left: self.sequencer.plan_count,
            source: 0,
            preserve: 0,
            work: self.sequencer.recovery.work,
            boundary: 0,
            context_cuts: [0; TUNERS + 1],
            context_clock: self.clock_id(),
            context_membership: self.membership,
            context_revision: self.sequencer.actual_revision,
            starts: [None; TUNERS + 1],
            lengths: [0; TUNERS + 1],
            heads: [None; TUNERS + 1],
            left: [0; TUNERS + 1],
            waiting: [false; TUNERS + 1],
            status_cuts: [0; TUNERS + 1],
        };
    }

    pub(in crate::performance::hub) fn service_recovery(&mut self, owner: &mut Owner) {
        if !self.sequencer.recovery.active {
            return;
        }
        if self.sequencer.retired
            && matches!(
                self.sequencer.recovery.phase,
                Phase::Status | Phase::Rebuild | Phase::Replay
            )
        {
            // Callback join ended every traversal reader. Inventory ownership
            // still settles normally; a retired Hub never computes new policy.
            self.frozen_captures.abandon();
            self.sequencer.cohort_unsent = 0;
            self.sequencer.cohort_recipients = 0;
            self.sequencer.committing = false;
            self.sequencer.config = None;
            self.sequencer.membership = None;
            self.sequencer.recovery.phase = Phase::Resume;
            self.service_revision = self.service_revision.wrapping_add(1);
        }
        // The repair cell is independent of a full Capture window. Ordinary
        // collection stops issuing new status queries until these fences pass.
        for source in 0..TUNERS {
            let participant = &mut self.sequencer.recovery.participants[source];
            let Some(fence) = participant.fence.filter(|_| !participant.sent) else { continue };
            if self.rows[source].lease != Some(fence.lease) {
                continue;
            }
            let Some(slot) =
                self.offer.as_ref().unwrap().session.rows[source].to_source.reserve_repair()
            else {
                continue;
            };
            slot.publish(Reply::Fence(fence));
            participant.sent = true;
            self.service_revision = self.service_revision.wrapping_add(1);
        }
        while self.sequencer.recovery.work < 256 {
            match self.sequencer.recovery.phase {
                Phase::Prepare => {
                    if self.sequencer.recovery.plan_left == 0 {
                        self.sequencer.recovery.phase = Phase::Inventory;
                        self.service_revision = self.service_revision.wrapping_add(1);
                        continue;
                    }
                    let index = self.sequencer.recovery.plan_cursor as usize;
                    let plan = self.sequencer.plans[index].as_mut().unwrap();
                    self.sequencer.recovery.plan_cursor = plan.next;
                    self.sequencer.recovery.plan_left -= 1;
                    self.sequencer.recovery.work += 1;
                    self.service_revision = self.service_revision.wrapping_add(1);
                    plan.inventoried = false;
                    plan.replay = if plan.binding.decision != 0
                        && plan.binding.decision < self.sequencer.recovery.from
                    {
                        PREFIX
                    } else {
                        0
                    };
                }
                Phase::Inventory => {
                    if !self.collect_inventory() {
                        return;
                    }
                    self.sequencer.recovery.phase = Phase::Preserve;
                    self.service_revision = self.service_revision.wrapping_add(1);
                }
                Phase::Preserve => {
                    if !self.preserve_cohort_configuration() {
                        return;
                    }
                    if self.sequencer.retired {
                        self.sequencer.recovery.phase = Phase::Resume;
                    } else {
                        self.begin_replay_status();
                        self.sequencer.recovery.phase = Phase::Status;
                    }
                    self.service_revision = self.service_revision.wrapping_add(1);
                }
                Phase::Status => {
                    if !self.collect_replay_status() {
                        return;
                    }
                    self.sequencer.recovery.phase = Phase::Rebuild;
                }
                Phase::Rebuild => {
                    if !self.rebuild_context(owner) {
                        return;
                    }
                    self.sequencer.recovery.phase = Phase::Replay;
                }
                Phase::Replay => {
                    if !self.replay_inputs() {
                        return;
                    }
                    self.sequencer.recovery.phase = Phase::Resume;
                }
                Phase::Resume => {
                    self.resume_emission();
                    return;
                }
            }
        }
    }

    fn collect_inventory(&mut self) -> bool {
        // Each turn advances one chunk from each available Source. A missing
        // callback holds its own participant without preventing other inventories.
        for _ in 0..TUNERS {
            let source = self.sequencer.recovery.source;
            self.sequencer.recovery.source = (source + 1) % TUNERS;
            let participant = &mut self.sequencer.recovery.participants[source];
            if participant.inventory_acked || participant.fence.is_none() {
                continue;
            }
            let Some(cuts) = participant.cuts else { continue };
            let fence = participant.fence.unwrap();
            if participant.chunk.is_none() {
                participant.chunk =
                    self.offer.as_ref().unwrap().session.rows[source].inventory.read_owned();
                participant.cursor = 0;
                if participant.chunk.is_some() {
                    self.service_revision = self.service_revision.wrapping_add(1);
                }
            }
            let Some(chunk) = &participant.chunk else { continue };
            let frame = chunk.value();
            let valid = frame.fence == fence
                && frame.input_cut == cuts.input
                && frame.output_cut == cuts.output
                && frame.sequence == participant.chunks
                && frame.first == participant.seen
                && frame.count <= 64
                && frame
                    .first
                    .checked_add(u32::from(frame.count))
                    .is_some_and(|end| end <= frame.total)
                && (frame.count == 64 || frame.first + u32::from(frame.count) == frame.total)
                && participant.total.is_none_or(|total| total == frame.total)
                && participant.lifetime_cut.is_none_or(|cut| cut == frame.lifetime_cut);
            if !valid {
                // A stale/invalid frame relinquishes its actual slot, never a
                // successful inventory cut or new capture/coverage authority.
                participant.chunk = None;
                self.configuration_exhausted();
                return false;
            }
            participant.total = Some(frame.total);
            participant.lifetime_cut = Some(frame.lifetime_cut);
            while self.sequencer.recovery.work < 256 {
                let participant = &self.sequencer.recovery.participants[source];
                let frame = participant.chunk.as_ref().unwrap().value();
                if participant.cursor == usize::from(frame.count) {
                    break;
                }
                let Some(record) = frame.records[participant.cursor] else {
                    self.configuration_exhausted();
                    return false;
                };
                let arena = frame.arena;
                let lifetime_cut = frame.lifetime_cut;
                self.sequencer.recovery.work += 1;
                self.service_revision = self.service_revision.wrapping_add(1);
                if usize::from(record.life) >= LIFETIMES
                    || record.lifetime == 0
                    || record.lifetime > lifetime_cut
                    || record.on_serial == 0
                    || record.on_serial > cuts.input
                    || !self.inventory_request(source, fence, arena, record)
                {
                    self.configuration_exhausted();
                    return false;
                }
                self.sequencer.recovery.participants[source].cursor += 1;
            }
            let participant = &mut self.sequencer.recovery.participants[source];
            let frame = participant.chunk.as_ref().unwrap().value();
            if participant.cursor != usize::from(frame.count) {
                continue;
            }
            let finished = participant.seen + u32::from(frame.count) == frame.total;
            let ack = if finished {
                let Some(ack) =
                    self.offer.as_ref().unwrap().session.rows[source].to_source.reserve_repair()
                else {
                    continue;
                };
                Some(ack)
            } else {
                None
            };
            participant.seen += u32::from(frame.count);
            participant.chunks += 1;
            participant.chunk = None;
            self.service_revision = self.service_revision.wrapping_add(1);
            if let Some(ack) = ack {
                ack.publish(Reply::InventoryComplete {
                    fence,
                    input_cut: cuts.input,
                    total: participant.seen,
                    chunks: participant.chunks,
                });
                participant.inventory_acked = true;
            }
        }
        self.sequencer
            .recovery
            .participants
            .iter()
            .all(|participant| participant.fence.is_none() || participant.inventory_acked)
    }

    fn inventory_request(
        &mut self,
        source: usize,
        fence: Fence,
        arena: usize,
        record: RequestInventory,
    ) -> bool {
        let index = source * LIFETIMES + usize::from(record.life);
        if let Some(plan) = self.sequencer.plans[index].as_mut() {
            if plan.key.lease != fence.lease
                || plan.key.epoch != fence.epoch
                || plan.lifetime != record.lifetime
                || plan.key.serial != record.on_serial
                || plan.key.arena != 0 && plan.key.arena != arena
                || plan.inventoried
                || plan.bound
                    && record.decision != 0
                    && (record.decision > plan.binding.decision
                        || record.configuration != plan.binding.configuration)
            {
                return false;
            }
            if plan.key.arena == 0 && plan.terminal {
                plan.key.arena = arena;
                plan.binding.decision = record.decision;
                plan.binding.configuration = record.configuration;
                plan.input = record.input;
            }
            plan.inventoried = true;
            plan.accepted |=
                matches!(record.outcome, RequestOutcome::Accepted | RequestOutcome::Partial);
            plan.terminal |= record.outcome == RequestOutcome::Canceled;
        } else if record.outcome == RequestOutcome::Retained {
            if record.decision != 0 {
                return false;
            }
            // This placeholder is an original-On identity only. No parent
            // position can satisfy a CaptureArena view; a later permitted
            // On Capture must attach its original key after matching Birth.
            self.sequencer.insert_plan(
                index,
                Plan {
                    key: super::super::super::capture::Key {
                        lease: fence.lease,
                        epoch: fence.epoch,
                        serial: record.on_serial,
                        arena,
                        position: u16::MAX,
                    },
                    lifetime: record.lifetime,
                    binding: Assignment::default(),
                    input: record.input,
                    sent: false,
                    terminal: false,
                    inventoried: true,
                    accepted: false,
                    bound: false,
                    replay: 0,
                    actual: NO_VOICE,
                    next: NO_PLAN,
                    previous: NO_PLAN,
                },
            );
        }
        // A historically accepted/partial terminal Life may outlive a correctly
        // retired Plan. It is factual and must never recreate an onset request.
        true
    }

    fn preserve_cohort_configuration(&mut self) -> bool {
        if !self.frozen_captures.active {
            return true;
        }
        // The original complete input proof predates the fence. Finish only its
        // interrupted metadata copy, so even not-yet-selected On bindings retain
        // that cohort's configuration before the traversal is abandoned.
        if !self.assemble_inputs() {
            return false;
        }
        while self.sequencer.recovery.work < 256
            && self.sequencer.recovery.preserve < self.frozen_captures.len()
        {
            let index = self.sequencer.recovery.preserve;
            self.sequencer.recovery.preserve += 1;
            self.sequencer.recovery.work += 1;
            self.service_revision = self.service_revision.wrapping_add(1);
            let event = self.frozen_captures.event(index).unwrap();
            if event.id.source == 0 || event.kind != cohort::Kind::Onset {
                continue;
            }
            let targets = Self::capture_targets(
                &self.rows,
                &self.direct_ingress,
                &self.direct_captures,
                self.direct.capture_lease(),
                self.publication_clock.epoch,
                self.frozen_captures.id,
            );
            let view = targets.rows[event.id.source as usize].as_ref().unwrap();
            let position = (event.targets.first >> 16) as usize;
            let (key, _) = view.original(position).unwrap();
            let (life, birth) = view.onset(position).unwrap();
            let plan_index = (event.id.source as usize - 1) * LIFETIMES + usize::from(life);
            let Some(plan) = self.sequencer.plans[plan_index].as_mut() else { continue };
            if plan.lifetime != birth.serial {
                self.configuration_exhausted();
                return false;
            }
            if !plan.bound {
                plan.key = key;
                plan.binding.configuration = self.sequencer.config.unwrap();
                plan.input = event.sample;
                plan.bound = true;
            }
        }
        self.sequencer.recovery.preserve == self.frozen_captures.len()
    }

    fn rebuild_context(&mut self, owner: &mut Owner) -> bool {
        // Reserve the whole physical cache copy. A multi-callback scan of live
        // factual state could combine states that never coexisted.
        if self.sequencer.recovery.work != 0
            || self.input_work + TUNERS + 1 > 4096
            || self.clock_id() != self.sequencer.recovery.context_clock
            || !owner.confirmed.is_complete()
            || owner.direct.pending().is_some()
            || !owner.direct.state.complete
        {
            return false;
        }
        if owner.direct.sequence < self.sequencer.recovery.status_cuts[0] {
            return false;
        }
        for (index, participant) in self.sequencer.recovery.participants.iter().enumerate() {
            if participant.fence.is_none() {
                continue;
            }
            let Some(cuts) = participant.cuts else { return false };
            if self.rows[index].applied
                < cuts.output.max(self.sequencer.recovery.status_cuts[index + 1])
                || self.rows[index].baseline.is_some()
                || !self.rows[index].state.complete
            {
                return false;
            }
        }
        if self.sequencer.config.is_some()
            && owner.abandon_input_cohort(self.clock_id(), self.sequencer.binding_sample).is_err()
        {
            return false;
        }
        self.input_work += TUNERS + 1;
        self.sequencer.context.copy_from_slice(&self.sequencer.actual);
        self.sequencer.recovery.work = 256;
        self.sequencer.recovery.context_cuts[0] = owner.direct.sequence;
        for source in 0..TUNERS {
            self.sequencer.recovery.context_cuts[source + 1] = self.rows[source].applied;
        }
        self.sequencer.recovery.context_membership = self.membership;
        self.sequencer.recovery.context_revision = self.sequencer.actual_revision;
        let Some(boundary) = self
            .callback
            .and_then(|callback| callback.steady_time.checked_add(i64::from(callback.frames)))
            .and_then(|raw| raw.checked_add(self.clock.calibration.offset))
        else {
            self.configuration_exhausted();
            return false;
        };
        self.sequencer.recovery.boundary = boundary;
        // All fixed old-generation request/output cuts and copied bindings are
        // now owned. The transaction resolves its old unsent/marker obligations;
        // queued old replies cannot authorize decisions above the frozen ceiling.
        self.frozen_captures.abandon();
        self.sequencer.cohort_unsent = 0;
        self.sequencer.cohort_recipients = 0;
        self.sequencer.committing = false;
        self.sequencer.membership = None;
        self.sequencer.config = None;
        self.sequencer.recovery.heads = self.sequencer.recovery.starts;
        self.sequencer.recovery.left = self.sequencer.recovery.lengths;
        self.service_revision = self.service_revision.wrapping_add(1);
        true
    }

    fn replayable(&self, source: usize, token: &super::super::super::capture::Token) -> bool {
        token.frozen.is_some()
            && self.sequencer.recovery.horizon.is_some_and(|horizon| token.sample <= horizon)
            && if source == 0 {
                self.direct.capture_lease() == Some(token.key.lease)
            } else {
                self.sequencer.recovery.participants[source - 1].fence.is_some_and(|fence| {
                    token.key.lease == fence.lease
                        && token.key.epoch == fence.epoch
                        && token.key.serial
                            <= self.sequencer.recovery.participants[source - 1].cuts.unwrap().input
                })
            }
    }

    fn begin_replay_status(&mut self) {
        // Only old bound/frozen originals participate. The normal binding proof
        // required the complete cohort's capture cut to be owned before policy;
        // later unbound input cannot extend this finite pre-open replay.
        for source in 0..=TUNERS {
            self.sequencer.recovery.starts[source] = self.input_window(source).front_position();
            self.sequencer.recovery.lengths[source] = self.input_window(source).len();
        }
        self.sequencer.recovery.heads = self.sequencer.recovery.starts;
        self.sequencer.recovery.left = self.sequencer.recovery.lengths;
    }

    fn advance_replay_head(&mut self, source: usize, position: usize) {
        self.sequencer.recovery.left[source] -= 1;
        self.sequencer.recovery.heads[source] = if self.sequencer.recovery.left[source] == 0 {
            None
        } else {
            self.input_window(source).next_position(position)
        };
        self.service_revision = self.service_revision.wrapping_add(1);
    }

    fn collect_replay_status(&mut self) -> bool {
        for source in 0..=TUNERS {
            while self.sequencer.recovery.work < 256 && self.input_work < 4096 {
                let Some(position) = self.sequencer.recovery.heads[source] else { break };
                self.sequencer.recovery.work += 1;
                self.input_work += 1;
                let Some(Intent::Capture(token)) = self.input_window(source).at_ref(position)
                else {
                    self.advance_replay_head(source, position);
                    continue;
                };
                if !self.replayable(source, token) {
                    self.advance_replay_head(source, position);
                    continue;
                }
                let key = token.key;
                if source != 0 {
                    let row = &self.rows[source - 1];
                    let view = super::super::super::capture::View {
                        ingress: &row.ingress,
                        permissions: &row.captures,
                        lease: row.lease.unwrap(),
                        epoch: row.epoch,
                        frozen: token.frozen.unwrap(),
                    };
                    if let Some((life, birth)) = view.onset(position) {
                        if let Some(plan) = self.sequencer.plans
                            [(source - 1) * LIFETIMES + usize::from(life)]
                        .as_mut()
                        .filter(|plan| {
                            plan.key.lease == key.lease
                                && plan.key.epoch == key.epoch
                                && plan.key.serial == key.serial
                                && plan.lifetime == birth.serial
                        }) {
                            plan.replay |= REPLAY_READER;
                        }
                    }
                }
                if source == 0 {
                    let Some(status) = self.direct.copy_capture_status(key) else { break };
                    let Some(status) = status else {
                        self.configuration_exhausted();
                        return false;
                    };
                    let Some(Intent::Capture(token)) = self.direct_ingress.at_mut(position) else {
                        unreachable!()
                    };
                    token.retain_status(status);
                } else {
                    let row = &mut self.rows[source - 1];
                    if row.status_query.is_some() {
                        break;
                    }
                    if !self.sequencer.recovery.waiting[source] {
                        let Some(query) = self.offer.as_ref().unwrap().session.rows[source - 1]
                            .to_source
                            .reserve_repair()
                        else {
                            break;
                        };
                        query.publish(Reply::CaptureStatusQuery(key));
                        row.status_query = Some(position);
                        self.sequencer.recovery.waiting[source] = true;
                        self.service_revision = self.service_revision.wrapping_add(1);
                        break;
                    }
                    self.sequencer.recovery.waiting[source] = false;
                }
                let Some(Intent::Capture(token)) = self.input_window(source).at_ref(position)
                else {
                    unreachable!()
                };
                let Some(status) = token.status else {
                    self.configuration_exhausted();
                    return false;
                };
                self.sequencer.recovery.status_cuts[source] =
                    self.sequencer.recovery.status_cuts[source].max(status.output_cut);
                self.advance_replay_head(source, position);
            }
        }
        self.sequencer.recovery.heads.iter().all(Option::is_none)
    }

    fn next_replay_sample(&mut self) -> Option<i64> {
        let mut sample = None;
        for source in 0..=TUNERS {
            while let Some(position) = self.sequencer.recovery.heads[source] {
                if self.input_work == 4096 {
                    return None;
                }
                self.input_work += 1;
                if let Some(Intent::Capture(token)) = self.input_window(source).at_ref(position) {
                    if self.replayable(source, token) {
                        sample =
                            Some(sample.map_or(token.sample, |old: i64| old.min(token.sample)));
                        break;
                    }
                }
                self.advance_replay_head(source, position);
            }
        }
        sample
    }

    fn assemble_replay(&mut self) -> bool {
        while self.sequencer.assembling <= TUNERS {
            let source = self.sequencer.assembling;
            let Some(position) = self.sequencer.recovery.heads[source] else {
                self.sequencer.assembling += 1;
                continue;
            };
            if self.input_work == 4096 {
                return false;
            }
            self.input_work += 1;
            let Some(Intent::Capture(token)) = self.input_window(source).at_ref(position) else {
                self.advance_replay_head(source, position);
                continue;
            };
            if !self.replayable(source, token) {
                self.advance_replay_head(source, position);
                continue;
            }
            if token.sample != self.frozen_captures.sample {
                self.sequencer.assembling += 1;
                continue;
            }
            if self.frozen_captures.len() == cohort::COHORT_EVENTS {
                self.configuration_exhausted();
                return false;
            }
            let id = self.frozen_captures.id;
            let (window, permissions, lease, epoch) = if source == 0 {
                (
                    &mut self.direct_ingress,
                    &self.direct_captures,
                    self.direct.capture_lease().unwrap(),
                    self.publication_clock.epoch,
                )
            } else {
                let row = &mut self.rows[source - 1];
                (&mut row.ingress, &row.captures, row.lease.unwrap(), row.epoch)
            };
            let Some(Intent::Capture(token)) = window.at_mut(position) else { unreachable!() };
            token.frozen = Some(id);
            let view = super::super::super::capture::View {
                ingress: window,
                permissions,
                lease,
                epoch,
                frozen: id,
            };
            self.frozen_captures.push(view.metadata(position).unwrap());
            self.advance_replay_head(source, position);
        }
        true
    }

    fn replay_inputs(&mut self) -> bool {
        // Essential actual terminals can advance after the fence's ACK. A
        // divergent snapshot requires another closed transaction before any
        // replacement can emit; do not mix a newer cache with this prefix.
        if self.clock_id() != self.sequencer.recovery.context_clock
            || self.membership != self.sequencer.recovery.context_membership
            || self.sequencer.actual_revision != self.sequencer.recovery.context_revision
        {
            self.sequencer.recovery.pending_from = Some(1);
        }
        if self.sequencer.committing {
            self.publish_cohort();
        }
        self.service_plans();
        if self.sequencer.committing && !self.publish_cohort() {
            return false;
        }
        while self.sequencer.work < 1024 && self.input_work < 4096 {
            if !self.frozen_captures.active {
                let sample = self.next_replay_sample();
                if self.input_work == 4096 {
                    return false;
                }
                let Some(sample) = sample else { return true };
                if self.frozen_captures.begin(sample, 0).is_err() {
                    self.configuration_exhausted();
                    return false;
                }
                self.sequencer.assembling = 0;
                self.sequencer.cohort_floor = self.sequencer.decision;
                self.sequencer.cohort_unsent = 0;
                self.sequencer.cohort_recipients = 0;
            }
            if !self.assemble_replay() {
                return false;
            }
            let progress = self.advance_captures(4096 - self.input_work);
            self.input_work += self.frozen_captures.spent;
            match progress {
                Ok(cohort::Progress::Pending) => return false,
                Ok(cohort::Progress::Complete { .. }) => {
                    self.frozen_captures.end(false);
                    self.sequencer.committing = true;
                    if !self.publish_cohort() {
                        return false;
                    }
                }
                Ok(cohort::Progress::Event(selected)) => {
                    if !self.assign_selected(selected) {
                        return false;
                    }
                    self.commit_capture().expect("same retained replay phase");
                    self.sequencer.work += 1;
                }
                Err(_) => {
                    self.configuration_exhausted();
                    return false;
                }
            }
        }
        false
    }

    fn resume_emission(&mut self) {
        if !self.sequencer.retired
            && (self.clock_id() != self.sequencer.recovery.context_clock
                || self.membership != self.sequencer.recovery.context_membership
                || self.sequencer.actual_revision != self.sequencer.recovery.context_revision)
        {
            self.sequencer.recovery.pending_from = Some(1);
            // A prior callback may have delivered some completion frames. Close
            // those generations too before settling the remaining participants;
            // a racing permitted group remains factual in the next inventory.
            for (source, participant) in self.sequencer.recovery.participants.iter().enumerate() {
                if participant.resumed {
                    self.offer.as_ref().unwrap().session.rows[source]
                        .emission_gate
                        .fetch_or(CLOSED, Ordering::AcqRel);
                }
            }
        }
        for source in 0..TUNERS {
            let participant = &mut self.sequencer.recovery.participants[source];
            let Some(fence) = participant.fence.filter(|_| !participant.resumed) else { continue };
            let Some(generation) = fence.generation.checked_add(4) else {
                self.configuration_exhausted();
                return;
            };
            let shared = &self.offer.as_ref().unwrap().session.rows[source];
            let Some(reply) = shared.to_source.reserve_repair() else { continue };
            let next = generation
                | if self.sequencer.retired
                    || self.sequencer.recovery.pending_from.is_some()
                    || shared.withdrawn.load(Ordering::Acquire)
                {
                    CLOSED
                } else {
                    OPEN
                };
            if shared
                .emission_gate
                .compare_exchange(
                    fence.generation | CLOSED,
                    next,
                    Ordering::AcqRel,
                    Ordering::Acquire,
                )
                .is_err()
            {
                continue;
            }
            reply.publish(Reply::RecoveryComplete {
                fence,
                generation,
                boundary: self.sequencer.recovery.boundary,
            });
            participant.resumed = true;
            self.service_revision = self.service_revision.wrapping_add(1);
        }
        if self
            .sequencer
            .recovery
            .participants
            .iter()
            .any(|participant| participant.fence.is_some() && !participant.resumed)
        {
            return;
        }
        self.sequencer.recovery.active = false;
        self.service_revision = self.service_revision.wrapping_add(1);
        if let Some(from) = self.sequencer.recovery.pending_from.take() {
            self.request_recovery(from);
        }
    }
}
