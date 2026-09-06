//! The Hub's owned original-input cursor, separate from actual-output progress.
use super::*;
use harmonigraph_core::cohort::{self, EventPhase, TargetAccess};
use harmonigraph_core::configuration::ResolvedConfig;
#[cfg(all(test, not(feature = "tuning-probe")))]
mod actual_lookup_tests;
mod recovery;

#[derive(Clone, Copy)]
struct Plan {
    key: super::super::capture::Key,
    lifetime: u64,
    binding: Assignment,
    input: i64,
    sent: bool,
    terminal: bool,
    inventoried: bool,
    accepted: bool,
    bound: bool,
    replay: u8,
    next: u32,
    previous: u32,
}
const NO_PLAN: u32 = u32::MAX;
const NO_VOICE: u16 = u16::MAX;
const PREFIX: u8 = 1;

/// Physical factual slots are independent of request/Plan ownership and of
/// State's packed voice array. This directory is only a bounded lookup hint.
const ACTUAL_KEYS_PER_SOURCE: usize = 16 * 128;
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct ActualKey {
    lease: Lease,
    epoch: u64,
    lifetime: u64,
}
fn actual_address(lease: Lease, channel: u8, key: u8) -> usize {
    usize::from(lease.slot) * ACTUAL_KEYS_PER_SOURCE + usize::from(channel) * 128 + usize::from(key)
}

pub(super) struct ActualLookup {
    key: ActualKey,
    index: u16,
    address: Option<usize>,
}

#[derive(Clone, Copy, PartialEq)]
struct Voice {
    source: u8,
    lifetime: u64,
    correction: i64,
    player: f64,
    key: u8,
}

/// Frozen membership belongs to the same pass as its capture/config bindings.
/// A newly observed lease cannot remove a missing member from this proof.
#[derive(Clone, Copy)]
struct Membership {
    clock: ClockId,
    leases: [Option<Lease>; TUNERS],
    intervals: [Option<(Coverage, u64)>; TUNERS],
    through: i64,
    floor: i64,
}

pub(super) struct Sequencer {
    pub retired: bool,
    pub terminal_session: bool,
    pub terminal_sources: u16,
    pub heads: [Option<usize>; TUNERS + 1],
    pub captured: [u64; TUNERS + 1],
    membership: Option<Membership>,
    config: Option<ResolvedConfig>,
    binding_sample: i64,
    assembling: usize,
    decision: u64,
    cohort_floor: u64,
    cohort_unsent: usize,
    cohort_recipients: u16,
    committing: bool,
    pub finalized: Option<i64>,
    pub copied: Option<i64>,
    plans: Box<[Option<Plan>]>,
    plan_head: u32,
    plan_tail: u32,
    plan_cursor: u32,
    plan_left: usize,
    plan_count: usize,
    plan_work: usize,
    context: Box<[Option<Voice>]>,
    actual: Box<[Option<Voice>]>,
    actual_keys: Box<[Option<ActualKey>]>,
    actual_index: Box<[u16]>,
    actual_free: Vec<u16>,
    actual_revision: u64,
    pub work: usize,
    pub extra_delay: u64,
    recovery: recovery::Recovery,
}
impl Default for Sequencer {
    fn default() -> Self {
        Self {
            retired: false,
            terminal_session: false,
            terminal_sources: 0,
            heads: [None; TUNERS + 1],
            captured: [0; TUNERS + 1],
            membership: None,
            config: None,
            binding_sample: 0,
            assembling: 0,
            decision: 0,
            cohort_floor: 0,
            cohort_unsent: 0,
            cohort_recipients: 0,
            committing: false,
            finalized: None,
            copied: None,
            // DIRECT has no plan row. Birth indices are already separately
            // reserved at Source; they are never authority without the key.
            plans: vec![None; TUNERS * LIFETIMES].into_boxed_slice(),
            plan_head: NO_PLAN,
            plan_tail: NO_PLAN,
            plan_cursor: NO_PLAN,
            plan_left: 0,
            plan_count: 0,
            plan_work: 0,
            context: vec![None; HELD_SESSION].into_boxed_slice(),
            actual: vec![None; HELD_SESSION].into_boxed_slice(),
            actual_keys: vec![None; HELD_SESSION].into_boxed_slice(),
            actual_index: vec![NO_VOICE; (TUNERS + 1) * ACTUAL_KEYS_PER_SOURCE].into_boxed_slice(),
            actual_free: (0..HELD_SESSION as u16).rev().collect(),
            actual_revision: 0,
            work: 0,
            extra_delay: 0,
            recovery: recovery::Recovery::default(),
        }
    }
}
impl Sequencer {
    pub(super) fn can_reset_clock_context(&self) -> bool {
        self.actual_revision.checked_add(1).is_some()
    }
    pub(super) fn clear_clock_context(&mut self) {
        let revision = self.actual_revision.checked_add(1).expect("preflighted clock boundary");
        self.actual_free.clear();
        for index in 0..HELD_SESSION {
            self.context[index] = None;
            self.actual[index] = None;
            self.actual_keys[index] = None;
            self.actual_free.push((HELD_SESSION - 1 - index) as u16);
        }
        self.actual_revision = revision;
    }
    pub(super) fn reset_clock_context(&mut self, lease: Lease, epoch: u64, direct: &State) {
        self.clear_clock_context();
        // A discontinuous reset has already cleared this observed-input State.
        // A healthy reanchor preserves it, independently of forwarding's paid
        // termination. Rebind those same factual lifetimes to the new clock.
        for voice in direct.voices() {
            let index = self.actual_free.pop().expect("at most 64 DIRECT voices");
            let value = Voice {
                source: 0,
                lifetime: voice.lifetime,
                correction: voice.frozen_offset_microcents,
                player: voice.player_tuning,
                key: voice.note,
            };
            self.actual[usize::from(index)] = Some(value);
            self.context[usize::from(index)] = Some(value);
            self.actual_keys[usize::from(index)] =
                Some(ActualKey { lease, epoch, lifetime: voice.lifetime });
            self.actual_index[actual_address(lease, voice.channel, voice.note)] = index;
        }
        // Absent identities invalidate old hints without scanning the 34816
        // address directory. No physical debt may reach this committed cut.
    }
    pub(super) fn recovering(&self) -> bool {
        self.recovery.active
    }
    pub(super) fn revoke_ack(
        &mut self,
        source: usize,
        fence: Fence,
        input_cut: u64,
        output_cut: u64,
        settled_attempt: u64,
    ) {
        self.recovery.acknowledge(source, fence, input_cut, output_cut, settled_attempt);
    }
    fn insert_plan(&mut self, index: usize, mut plan: Plan) {
        assert!(self.plans[index].is_none());
        plan.previous = self.plan_tail;
        plan.next = NO_PLAN;
        if self.plan_tail == NO_PLAN {
            self.plan_head = index as u32;
        } else {
            self.plans[self.plan_tail as usize].as_mut().unwrap().next = index as u32;
        }
        self.plan_tail = index as u32;
        self.plan_count += 1;
        self.plans[index] = Some(plan);
    }
    fn remove_plan(&mut self, index: usize) {
        let plan = self.plans[index].take().unwrap();
        if plan.previous == NO_PLAN {
            self.plan_head = plan.next;
        } else {
            self.plans[plan.previous as usize].as_mut().unwrap().next = plan.next;
        }
        if plan.next == NO_PLAN {
            self.plan_tail = plan.previous;
        } else {
            self.plans[plan.next as usize].as_mut().unwrap().previous = plan.previous;
        }
        self.plan_count -= 1;
    }
    pub(super) fn consume_terminal_original(
        &mut self,
        source: usize,
        token: &super::super::capture::Token,
        permissions: &super::super::capture::Permissions,
    ) {
        let Some((life, lifetime)) = permissions.original_on(token) else { return };
        if let Some(plan) = self.plans[source * LIFETIMES + usize::from(life)].as_mut() {
            if plan.terminal
                && plan.lifetime == lifetime
                && plan.key.lease == token.key.lease
                && plan.key.epoch == token.key.epoch
                && plan.key.serial == token.key.serial
                && (plan.key.arena == 0 || plan.key.arena == token.key.arena)
            {
                plan.key = token.key;
                plan.bound = true;
            }
        }
    }

    pub(super) fn cancel(
        &mut self,
        source: usize,
        lease: Lease,
        epoch: u64,
        serial: u64,
        request: u16,
        lifetime: u64,
    ) {
        if serial == 0 || lifetime == 0 || usize::from(request) >= LIFETIMES {
            return;
        }
        let index = source * LIFETIMES + usize::from(request);
        if let Some(plan) = self.plans[index].as_mut() {
            if plan.key.lease == lease
                && plan.key.epoch == epoch
                && plan.key.serial == serial
                && plan.lifetime == lifetime
            {
                plan.terminal = true;
            }
        } else {
            // A copied Retained inventory record may predate this cancellation.
            // Keep its exact original-On identity until the complete fixed
            // inventory passes; no capture permission is claimed by this key.
            self.insert_plan(
                index,
                Plan {
                    key: super::super::capture::Key {
                        lease,
                        epoch,
                        serial,
                        arena: 0,
                        position: u16::MAX,
                    },
                    lifetime,
                    binding: Assignment::default(),
                    input: 0,
                    sent: false,
                    terminal: true,
                    inventoried: false,
                    accepted: false,
                    bound: false,
                    replay: 0,
                    next: NO_PLAN,
                    previous: NO_PLAN,
                },
            );
        }
    }
}

const _: () = assert!(std::mem::size_of::<Option<Plan>>() <= 256);
const _: () = assert!(std::mem::size_of::<Option<ActualKey>>() <= 56);
const _: () = assert!(std::mem::align_of::<Option<ActualKey>>() <= 8);
const _: () = assert!(
    std::mem::size_of::<Option<Plan>>() - std::mem::size_of::<ResolvedConfig>() + 128 <= 256
);
const _: () = assert!(std::mem::size_of::<Option<Voice>>() <= 256);
// ConfirmedPitches and its rich factual companion share one confirmed budget;
// the other half remains exclusively prospective. Keep full future config room.
const _: () = assert!(
    std::mem::size_of::<Option<Voice>>()
        + std::mem::size_of::<Option<harmonigraph_core::confirmed::ConfirmedPitch>>()
        + 128
        + 16
        + 8
        <= 256
);
const _: () = assert!(std::mem::size_of::<Option<Voice>>() + 128 + 16 + 8 <= 256);

#[cfg(all(test, not(feature = "tuning-probe")))]
impl Sequencer {
    pub(super) fn print_test_memory_layout(&self) {
        println!(
            "LEDGER factual lookup [key_cell,key_backing,directory_backing] {:?}",
            [
                std::mem::size_of::<Option<ActualKey>>(),
                std::mem::size_of_val(&*self.actual_keys),
                std::mem::size_of_val(&*self.actual_index)
            ]
        );
        println!(
            "LEDGER sequencer actual [backing,free_capacity,free_metadata,recovery_owner] {:?}",
            [
                std::mem::size_of_val(&*self.actual),
                self.actual_free.capacity() * std::mem::size_of::<u16>(),
                std::mem::size_of_val(&self.actual_free),
                std::mem::size_of_val(&self.recovery),
            ]
        );
        println!(
            "LEDGER sequencer [owner,plan_option,plan_backing,voice_option,voice_backing] {:?}",
            [
                std::mem::size_of::<Self>(),
                std::mem::size_of::<Option<Plan>>(),
                std::mem::size_of_val(&*self.plans),
                std::mem::size_of::<Option<Voice>>(),
                std::mem::size_of_val(&*self.context)
            ]
        );
    }
}

impl Hub {
    #[cfg(all(test, not(feature = "tuning-probe")))]
    pub(in crate::performance) fn test_actual_voice(
        &self,
        source: u8,
        lifetime: u64,
    ) -> Option<(usize, i64, f64)> {
        self.sequencer.actual.iter().enumerate().find_map(|(index, cell)| {
            cell.filter(|voice| voice.source == source && voice.lifetime == lifetime)
                .map(|voice| (index, voice.correction, voice.player))
        })
    }
    #[cfg(all(test, not(feature = "tuning-probe")))]
    pub(in crate::performance) fn test_service_terminal_plans(&mut self) {
        self.plan_callback();
        self.service_plans();
    }
    #[cfg(all(test, not(feature = "tuning-probe")))]
    pub(in crate::performance) fn test_terminal_scope(&self) -> (bool, u16) {
        (self.sequencer.terminal_session, self.sequencer.terminal_sources)
    }
    #[cfg(all(test, not(feature = "tuning-probe")))]
    pub(in crate::performance) fn test_request_recovery(&mut self, from: u64) {
        self.request_recovery(from);
    }
    #[cfg(all(test, not(feature = "tuning-probe")))]
    pub(in crate::performance) fn test_plan_binding(
        &self,
        source: usize,
        life: u16,
    ) -> Option<Assignment> {
        self.sequencer.plans[source * LIFETIMES + usize::from(life)].map(|plan| plan.binding)
    }
    #[cfg(all(test, not(feature = "tuning-probe")))]
    pub(in crate::performance) fn test_plan_state(
        &self,
        source: usize,
        life: u16,
    ) -> Option<(bool, bool, bool)> {
        self.sequencer.plans[source * LIFETIMES + usize::from(life)]
            .map(|plan| (plan.terminal, plan.bound, self.sequencer.recovering()))
    }
    #[cfg(all(test, not(feature = "tuning-probe")))]
    pub(in crate::performance) fn test_cohort_delivery(&self) -> (u64, usize, u16, bool, usize) {
        (
            self.sequencer.decision,
            self.sequencer.cohort_unsent,
            self.sequencer.cohort_recipients,
            self.sequencer.committing,
            self.sequencer.plan_work,
        )
    }

    pub(super) fn sequences_inputs(&self) -> bool {
        #[cfg(all(test, not(feature = "tuning-probe")))]
        if self.test_aggregation {
            return false;
        }
        !self.sequencer.retired && !self.direct.initial_direct()
    }

    fn input_window(&self, source: usize) -> &Window<Intent, INTENT_RING> {
        if source == 0 {
            &self.direct_ingress
        } else {
            &self.rows[source - 1].ingress
        }
    }

    fn input_snapshot(&self, owner: &Owner) -> Option<Membership> {
        let (direct, cut) = self.direct.completed_input()?;
        if cut > self.sequencer.captured[0] {
            return None;
        }
        let cap = owner
            .recording
            .configuration_seed_frontier()
            .checked_add(self.clock.calibration.offset)?;
        let mut snapshot = Membership {
            clock: self.clock_id(),
            leases: [None; TUNERS],
            intervals: [None; TUNERS],
            through: direct.through.min(cap),
            floor: direct.start,
        };
        for (index, row) in self.rows.iter().enumerate() {
            let Some(lease) = row.lease else { continue };
            // Enrollment is an acknowledged new boundary; no old interval is
            // inferred from a large output endpoint or an absent callback.
            if !row.member {
                if row.terminal_cut.is_some() {
                    continue;
                }
                return None;
            }
            if row.acknowledged_membership == 0
                || row.input_membership != row.acknowledged_membership
            {
                return None;
            }
            let interval = row.input_coverage?;
            if interval.1 > self.sequencer.captured[index + 1] {
                return None;
            }
            snapshot.leases[index] = Some(lease);
            snapshot.intervals[index] = Some(interval);
            snapshot.floor = snapshot.floor.max(interval.0.start);
            snapshot.through = snapshot.through.min(interval.0.through);
        }
        (snapshot.through > snapshot.floor).then_some(snapshot)
    }

    fn next_input_sample(&mut self) -> Option<i64> {
        let mut sample = None;
        for source in 0..=TUNERS {
            if source != 0 && self.rows[source - 1].terminal_cut.is_some() {
                continue;
            }
            while let Some(index) = self.sequencer.heads[source] {
                if self.input_work == 4096 {
                    return None;
                }
                self.input_work += 1;
                let window = self.input_window(source);
                if let Some(Intent::Capture(token)) = window.at_ref(index) {
                    if token.frozen.is_none() {
                        sample =
                            Some(sample.map_or(token.sample, |old: i64| old.min(token.sample)));
                        break;
                    }
                }
                self.sequencer.heads[source] = window.next_position(index);
            }
        }
        sample
    }

    pub(super) fn sequence_inputs(&mut self, owner: &mut Owner, recorder: &mut Recorder) {
        self.observe_terminal_faults();
        if self.sequencer.recovering() {
            self.service_recovery(owner);
            self.service_plans();
            return;
        }
        if self.sequencer.terminal_session {
            self.service_plans();
            return;
        }
        if !self.sequences_inputs() || !self.clock.valid || self.invalidated || self.capture_hold {
            return;
        }
        if self.sequencer.committing {
            self.publish_cohort();
        }
        self.service_plans();
        if self.sequencer.committing && !self.publish_cohort() {
            return;
        }
        while self.sequencer.work < 1024 && self.input_work < 4096 {
            if !self.frozen_captures.active {
                let Some(membership) = self.input_snapshot(owner) else { return };
                let sample = self.next_input_sample();
                if self.input_work == 4096 {
                    return;
                }
                let boundary =
                    sample.map_or(membership.through, |sample| sample.max(membership.floor));
                let finalized = boundary.min(membership.through);
                if owner.finalize_input(membership.clock, finalized, finalized, recorder).is_err() {
                    return;
                }
                self.sequencer.finalized = Some(finalized);
                self.sequencer.copied = Some(finalized);
                let Some(sample) = sample.filter(|_| boundary < membership.through) else { return };
                let Ok(config) = owner.bind_input_cohort(membership.clock, boundary) else {
                    return;
                };
                if self.frozen_captures.begin(sample, 0).is_err() {
                    self.configuration_exhausted();
                    return;
                }
                self.sequencer.membership = Some(membership);
                self.sequencer.binding_sample = boundary;
                self.sequencer.config = Some(config);
                self.sequencer.assembling = 0;
                self.sequencer.cohort_floor = self.sequencer.decision;
                self.sequencer.cohort_unsent = 0;
                self.sequencer.cohort_recipients = 0;
            }
            if !self.assemble_inputs() {
                return;
            }
            // Graph work and event applications each retain their cursor. The
            // one shared structural grant is never refreshed by a subblock.
            let units = 4096 - self.input_work;
            let progress = self.advance_captures(units);
            self.input_work += self.frozen_captures.spent;
            match progress {
                Ok(cohort::Progress::Pending) => return,
                Ok(cohort::Progress::Complete { .. }) => {
                    self.frozen_captures.end(false);
                    self.sequencer.committing = true;
                    if !self.publish_cohort() {
                        return;
                    }
                }
                Ok(cohort::Progress::Event(selected)) => {
                    if !self.assign_selected(selected) {
                        return;
                    }
                    self.commit_capture().expect("same retained offered phase");
                    self.sequencer.work += 1;
                }
                Err(_) => {
                    self.configuration_exhausted();
                    return;
                }
            }
        }
    }

    fn assemble_inputs(&mut self) -> bool {
        while self.sequencer.assembling <= TUNERS {
            let source = self.sequencer.assembling;
            let Some(position) = self.sequencer.heads[source] else {
                self.sequencer.assembling += 1;
                continue;
            };
            if self.input_work == 4096 {
                return false;
            }
            self.input_work += 1;
            let window = self.input_window(source);
            let Some(Intent::Capture(token)) = window.at_ref(position) else {
                self.sequencer.heads[source] = window.next_position(position);
                continue;
            };
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
            let view = super::super::capture::View {
                ingress: window,
                permissions,
                lease,
                epoch,
                frozen: id,
            };
            self.frozen_captures.push(view.metadata(position).expect("installed immutable input"));
            self.sequencer.heads[source] = window.next_position(position);
        }
        true
    }

    fn assign_selected(&mut self, selected: cohort::Selected) -> bool {
        let event = self.frozen_captures.event(selected.event_index).unwrap();
        let targets = Self::capture_targets(
            &self.rows,
            &self.direct_ingress,
            &self.direct_captures,
            self.direct.capture_lease(),
            self.publication_clock.epoch,
            self.frozen_captures.id,
        );
        let span = if selected.phase == EventPhase::ReplacedRelease {
            event.replaced
        } else {
            event.targets
        };
        let mut next = span.first;
        if event.kind == cohort::Kind::Onset && selected.phase == EventPhase::Original {
            let target = targets.get(event.id.source, next).unwrap().target;
            let source = event.id.source as usize;
            let view = targets.rows[source].as_ref().unwrap();
            let position = (next >> 16) as usize;
            let (key, _) = view.original(position).unwrap();
            let (life, birth) = view.onset(position).unwrap();
            let replay = self.sequencer.recovering();
            let done = replay && view.effect_done(next, 0).expect("copied replay status");
            let prior = (source != 0)
                .then(|| self.sequencer.plans[(source - 1) * LIFETIMES + usize::from(life)])
                .flatten();
            if let Some(prior) = prior {
                if prior.lifetime != birth.serial
                    || prior.key.lease != key.lease
                    || prior.key.epoch != key.epoch
                    || prior.key.arena != 0 && prior.key.arena != key.arena
                    || prior.key.serial != key.serial
                    || prior.key.position != u16::MAX && prior.key != key
                {
                    return false;
                }
                if prior.terminal {
                    let plan = self.sequencer.plans[(source - 1) * LIFETIMES + usize::from(life)]
                        .as_mut()
                        .unwrap();
                    plan.key = key;
                    if !plan.bound {
                        plan.binding.configuration =
                            self.sequencer.config.unwrap_or(plan.binding.configuration);
                    }
                    plan.bound = true;
                    return true;
                }
                if replay && prior.accepted {
                    return true;
                }
                if !replay && prior.bound {
                    return false;
                }
            } else if replay && source != 0 && !done {
                return false;
            }
            if done {
                return true;
            }
            let Some(decision) = self.sequencer.decision.checked_add(1) else { return false };
            // Deliberately artificial: the sum includes each prior canonical
            // assignment, including predecessors from other Sources.
            let correction = if replay && prior.is_some_and(|plan| plan.replay & PREFIX != 0) {
                prior.unwrap().binding.correction
            } else if source != 0 && birth.adaptive {
                (1_000_000
                    + self
                        .sequencer
                        .context
                        .iter()
                        .flatten()
                        .map(|voice| voice.correction)
                        .sum::<i64>())
                    % 49_000_000
            } else {
                0
            };
            let player = if replay && prior.is_some_and(|plan| plan.replay & PREFIX != 0) {
                prior.unwrap().binding.initial_player
            } else {
                selected.initial_tuning.map_or(0.0, |initial| f64::from_bits(initial.value_bits))
            };
            let Some(slot) = self.sequencer.context.iter().position(Option::is_none) else {
                self.configuration_exhausted();
                return false;
            };
            if source != 0 {
                let index = (source - 1) * LIFETIMES + usize::from(life);
                let configuration = prior
                    .filter(|plan| plan.bound)
                    .map(|plan| plan.binding.configuration)
                    .or(self.sequencer.config)
                    .expect("owned original cohort configuration");
                let emission = if replay {
                    self.sequencer.recovery.generation(source - 1).unwrap()
                } else {
                    self.offer.as_ref().unwrap().session.rows[source - 1]
                        .emission_gate
                        .load(Ordering::Acquire)
                        & !super::super::source::GATE_FLAGS
                };
                let binding = Assignment {
                    configuration,
                    decision,
                    emission,
                    correction,
                    initial_player: player,
                };
                if let Some(plan) = self.sequencer.plans[index].as_mut() {
                    plan.key = key;
                    plan.binding = binding;
                    plan.sent = false;
                    plan.bound = true;
                } else {
                    self.sequencer.insert_plan(
                        index,
                        Plan {
                            key,
                            lifetime: birth.serial,
                            binding,
                            input: event.sample,
                            sent: false,
                            terminal: false,
                            inventoried: false,
                            accepted: false,
                            bound: true,
                            replay: 0,
                            next: NO_PLAN,
                            previous: NO_PLAN,
                        },
                    );
                }
                self.sequencer.cohort_unsent += 1;
                self.sequencer.cohort_recipients |= 1 << (source - 1);
                let reply = Reply::Assignment { key, life, lifetime: birth.serial, binding };
                if self.offer.as_mut().unwrap().bank.rows[source - 1].replies.push(reply).is_ok() {
                    self.sequencer.plans[index].as_mut().unwrap().sent = true;
                    self.sequencer.cohort_unsent -= 1;
                }
            }
            self.sequencer.context[slot] = Some(Voice {
                source: event.id.source,
                lifetime: target.lifetime,
                correction,
                player,
                key: target.key,
            });
            self.sequencer.decision = decision;
        } else {
            for ordinal in 0..span.len {
                let address = next;
                let link = targets.get(event.id.source, next).unwrap();
                next = link.next;
                if self.sequencer.recovering()
                    && targets.rows[event.id.source as usize]
                        .as_ref()
                        .unwrap()
                        .effect_done(address, u16::from(ordinal))
                        .expect("copied replay status")
                {
                    continue;
                }
                if let Some(cell) = self.sequencer.context.iter_mut().find(|cell| {
                    cell.is_some_and(|voice| {
                        voice.source == event.id.source && voice.lifetime == link.target.lifetime
                    })
                }) {
                    match (selected.phase, event.kind) {
                        (EventPhase::ReplacedRelease, _)
                        | (
                            _,
                            cohort::Kind::Terminal | cohort::Kind::Channel { terminal: true, .. },
                        ) => *cell = None,
                        (_, cohort::Kind::Tuning { value_bits }) => {
                            cell.as_mut().unwrap().player = f64::from_bits(value_bits)
                        }
                        _ => {}
                    }
                }
            }
        }
        true
    }

    pub(super) fn plan_callback(&mut self) {
        self.sequencer.plan_work = 0;
        self.sequencer.recovery.work = 0;
    }

    /// A traversal may finish across callbacks, and the real reply consumer may
    /// run concurrently with these pushes. The same FIFO's final marker makes
    /// every assignment in the completed cohort eligible together at Source.
    fn publish_cohort(&mut self) -> bool {
        if !self.sequencer.committing {
            return true;
        }
        if self.sequencer.cohort_unsent != 0 {
            return false;
        }
        for source in 0..TUNERS {
            let bit = 1 << source;
            if self.sequencer.cohort_recipients & bit == 0 {
                continue;
            }
            if self.sequencer.plan_work == 256 {
                return false;
            }
            self.sequencer.plan_work += 1;
            let lease = if self.sequencer.recovering() {
                self.sequencer.recovery.lease(source).unwrap()
            } else {
                self.sequencer.membership.unwrap().leases[source].unwrap()
            };
            if self.rows[source].lease != Some(lease) {
                return false;
            }
            let reply = Reply::CohortCommitted {
                lease,
                epoch: self.rows[source].epoch,
                through: self.sequencer.decision,
            };
            if self.offer.as_mut().unwrap().bank.rows[source].replies.push(reply).is_err() {
                continue;
            }
            self.sequencer.cohort_recipients &= !bit;
        }
        if self.sequencer.cohort_recipients != 0 {
            return false;
        }
        self.sequencer.committing = false;
        self.sequencer.membership = None;
        self.sequencer.config = None;
        true
    }

    pub(super) fn output_assignment(
        &mut self,
        source: usize,
        output: OutputDelta,
    ) -> Option<(Assignment, i64)> {
        let request = output.outcome.request;
        if usize::from(request) >= LIFETIMES {
            return None;
        }
        let plan = self.sequencer.plans[source * LIFETIMES + usize::from(request)].as_mut()?;
        if self.rows[source].lease != Some(plan.key.lease)
            || output.epoch != plan.key.epoch
            || output.lifetime != plan.lifetime
            || output.decision != plan.binding.decision
        {
            return None;
        }
        if output.event.release() {
            plan.terminal = true;
        }
        let planned = output.input.checked_add(DELAY)?;
        if output.event.attack().is_some() {
            plan.accepted = true;
            self.sequencer.extra_delay =
                self.sequencer.extra_delay.max(output.actual.saturating_sub(planned).max(0) as u64);
        }
        Some((plan.binding, planned))
    }

    pub(super) fn service_plans(&mut self) {
        if self.sequencer.plan_left == 0 {
            self.sequencer.plan_cursor = self.sequencer.plan_head;
            self.sequencer.plan_left = self.sequencer.plan_count;
        }
        while self.sequencer.plan_work < 256 && self.sequencer.plan_left != 0 {
            let index = self.sequencer.plan_cursor as usize;
            let plan = self.sequencer.plans[index].unwrap();
            self.sequencer.plan_cursor = plan.next;
            self.sequencer.plan_left -= 1;
            self.sequencer.plan_work += 1;
            let source = index / LIFETIMES;
            let life = (index % LIFETIMES) as u16;
            // Delivery accounting is independent of retained identity and a
            // successful terminal reply. Mark it paid before any reader hold.
            if plan.terminal
                && !self.sequencer.retired
                && !plan.sent
                && plan.binding.decision > self.sequencer.cohort_floor
            {
                assert_ne!(self.sequencer.cohort_unsent, 0);
                self.sequencer.cohort_unsent -= 1;
                self.sequencer.plans[index].as_mut().unwrap().sent = true;
            }
            // A canceled unbound Original still has to pass the input cursor.
            // Keep its exact identity until then so normal sequencing cannot
            // resurrect it after an earlier inventory frame was acknowledged.
            if plan.terminal
                && (self.sequencer.recovering() || !self.sequencer.retired && !plan.bound)
            {
                continue;
            }
            if !plan.terminal
                && (!plan.bound
                    || self.sequencer.recovering()
                        && self.sequencer.recovery.old_decision(plan.binding.decision))
            {
                continue;
            }
            let reply = if plan.terminal {
                Reply::PlanRetired {
                    incarnation: plan.key.lease.incarnation,
                    epoch: plan.key.epoch,
                    life,
                    lifetime: plan.lifetime,
                    decision: plan.binding.decision,
                }
            } else if !plan.sent && !self.sequencer.retired {
                Reply::Assignment {
                    key: plan.key,
                    life,
                    lifetime: plan.lifetime,
                    binding: plan.binding,
                }
            } else {
                continue;
            };
            if self.offer.as_mut().unwrap().bank.rows[source].replies.push(reply).is_err() {
                continue;
            }
            if !self.sequencer.retired
                && !plan.terminal
                && !plan.sent
                && plan.binding.decision > self.sequencer.cohort_floor
            {
                assert_ne!(self.sequencer.cohort_unsent, 0);
                self.sequencer.cohort_unsent -= 1;
            }
            if plan.terminal {
                self.sequencer.remove_plan(index);
            } else {
                self.sequencer.plans[index].as_mut().unwrap().sent = true;
            }
        }
    }
}

impl Sequencer {
    fn actual_hint(&self, key: ActualKey, hint: u16) -> bool {
        hint != NO_VOICE && self.actual_keys[usize::from(hint)] == Some(key)
    }

    /// Resolve once under the output grant, including unsuccessful fallback
    /// searches. The returned slot (or proven absence) is reused by application.
    fn lookup_actual(
        &self,
        key: ActualKey,
        address: Option<usize>,
        new_on: bool,
        work: &mut usize,
    ) -> Option<ActualLookup> {
        if key.lifetime == 0 {
            return Some(ActualLookup { key, index: NO_VOICE, address });
        }
        if *work == 4096 {
            return None;
        }
        *work += 1;
        let hint = address.map_or(NO_VOICE, |address| self.actual_index[address]);
        if self.actual_hint(key, hint) {
            return Some(ActualLookup { key, index: hint, address });
        }
        if new_on {
            return Some(ActualLookup { key, index: NO_VOICE, address });
        }
        if *work + HELD_SESSION > 4096 {
            return None;
        }
        *work += HELD_SESSION;
        let index = self.actual_keys.iter().position(|old| *old == Some(key));
        Some(ActualLookup { key, index: index.map_or(NO_VOICE, |index| index as u16), address })
    }

    pub(super) fn lookup_output(
        &self,
        lease: Lease,
        output: OutputDelta,
        work: &mut usize,
    ) -> Option<ActualLookup> {
        use super::super::event::Event;
        let address = match output.event {
            Event::Note { channel: channel @ 0..=15, key: key @ 0..=127, .. }
            | Event::Expression { channel: channel @ 0..=15, key: key @ 0..=127, .. } => {
                Some(actual_address(lease, channel as u8, key as u8))
            }
            Event::Midi { data: [status, key @ 0..=127, _], .. }
                if matches!(status & 0xf0, 0x80 | 0x90) =>
            {
                Some(actual_address(lease, status & 15, key))
            }
            _ => None,
        };
        self.lookup_actual(
            ActualKey { lease, epoch: output.epoch, lifetime: output.lifetime },
            address,
            output.event.attack().is_some(),
            work,
        )
    }

    pub(super) fn lookup_direct(
        &self,
        delta: harmonigraph_core::canonical::NoteDelta,
        work: &mut usize,
    ) -> Option<ActualLookup> {
        let clock = delta.timing?.clock;
        let lease = Lease {
            session: clock.runtime_session,
            source: harmonigraph_core::SourceId::DIRECT,
            incarnation: 0,
            slot: 0,
        };
        self.lookup_actual(
            ActualKey { lease, epoch: clock.epoch, lifetime: delta.lifetime },
            Some(actual_address(lease, delta.event.channel, delta.event.note)),
            matches!(delta.event.kind, harmonigraph_core::NoteEventKind::On { .. }),
            work,
        )
    }

    fn store_actual(&mut self, lookup: ActualLookup, voice: Option<Voice>) -> Result<(), ()> {
        let ActualLookup { key, index, address } = lookup;
        if index != NO_VOICE {
            assert!(self.actual_hint(key, index));
            let slot = usize::from(index);
            if self.actual[slot] != voice {
                self.actual_revision = self.actual_revision.checked_add(1).ok_or(())?;
            }
            self.actual[slot] = voice;
            if voice.is_none() {
                self.actual_keys[slot] = None;
                self.actual_free.push(index);
                if let Some(address) =
                    address.filter(|address| self.actual_index[*address] == index)
                {
                    self.actual_index[address] = NO_VOICE;
                }
            } else if let Some(address) = address {
                self.actual_index[address] = index;
            }
        } else if let Some(voice) = voice {
            let revision = self.actual_revision.checked_add(1).ok_or(())?;
            let index = self.actual_free.pop().ok_or(())?;
            let slot = usize::from(index);
            assert!(self.actual[slot].is_none());
            self.actual[slot] = Some(voice);
            self.actual_keys[slot] = Some(key);
            if let Some(address) = address {
                self.actual_index[address] = index;
            }
            self.actual_revision = revision;
        }
        Ok(())
    }

    pub(super) fn actual_output(
        &mut self,
        lookup: ActualLookup,
        voice: Option<harmonigraph_core::canonical::VoiceBaseline>,
    ) -> bool {
        if lookup.key.lifetime == 0 {
            return true;
        }
        let voice = voice.map(|voice| Voice {
            source: lookup.key.lease.slot,
            lifetime: voice.lifetime,
            correction: voice.frozen_offset_microcents,
            player: voice.player_tuning,
            key: voice.note,
        });
        self.store_actual(lookup, voice).is_ok()
    }

    pub(super) fn actual_baseline(
        &mut self,
        lease: Lease,
        epoch: u64,
        frame: &harmonigraph_core::canonical::SourceBaseline,
    ) -> bool {
        let source = lease.slot;
        let mut replaced = false;
        for (index, cell) in self.actual.iter_mut().enumerate() {
            if cell.is_some_and(|voice| voice.source == source) {
                replaced = true;
                *cell = None;
                self.actual_keys[index] = None;
                self.actual_free.push(index as u16);
            }
        }
        // Stale directory entries are harmless: full lease/epoch/lifetime
        // equality is required even when replacement reuses the same cell.
        for voice in frame.voices() {
            replaced = true;
            let Some(index) = self.actual_free.pop() else { return false };
            let key = ActualKey { lease, epoch, lifetime: voice.lifetime };
            self.actual_keys[usize::from(index)] = Some(key);
            self.actual_index[actual_address(lease, voice.channel, voice.note)] = index;
            self.actual[usize::from(index)] = Some(Voice {
                source,
                lifetime: voice.lifetime,
                correction: voice.frozen_offset_microcents,
                player: voice.player_tuning,
                key: voice.note,
            });
        }
        if replaced {
            let Some(revision) = self.actual_revision.checked_add(1) else { return false };
            self.actual_revision = revision;
        }
        true
    }

    pub(super) fn actual_direct(
        &mut self,
        delta: harmonigraph_core::canonical::NoteDelta,
        lookup: ActualLookup,
    ) -> bool {
        use harmonigraph_core::NoteEventKind;
        if delta.lifetime == 0 {
            return true;
        }
        let voice = match delta.event.kind {
            NoteEventKind::On { .. } => Some(Voice {
                source: 0,
                lifetime: delta.lifetime,
                correction: 0,
                player: 0.0,
                key: delta.event.note,
            }),
            NoteEventKind::Off => None,
            NoteEventKind::Tuning { .. } => {
                if lookup.index == NO_VOICE {
                    return true;
                }
                let Some(mut voice) = self.actual[usize::from(lookup.index)] else {
                    return false;
                };
                let Some(pitch) = delta.pitch_microcents else { return false };
                voice.player = (pitch - i64::from(voice.key) * 100_000_000) as f64 / 100_000_000.0;
                Some(voice)
            }
            _ => return true,
        };
        self.store_actual(lookup, voice).is_ok()
    }
}
