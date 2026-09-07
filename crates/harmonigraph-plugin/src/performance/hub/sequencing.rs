//! The Hub's owned original-input cursor, separate from actual-output progress.
use super::*;
use harmonigraph_core::configuration::ResolvedConfig;
use harmonigraph_core::{policy, LatticePos, PitchClass};
#[cfg(test)]
mod actual_lookup_tests;
mod history;

#[derive(Clone, Copy)]
pub(super) struct Plan {
    request: Request,
    binding: Assignment,
    /// Expected input-to-output translation for this exact decision. Replayed
    /// plans start at their acknowledged boundary; accepted onsets freeze it.
    shift: i64,
    sent: bool,
    terminal: bool,
    accepted: bool,
    bound: bool,
    next: u32,
    previous: u32,
}
const NO_PLAN: u32 = u32::MAX;
const NO_VOICE: u16 = u16::MAX;

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
    channel: u8,
    pitch: i64,
    node: Option<LatticePos>,
    configuration_revision: u64,
    decision: u64,
}

impl Voice {
    fn factual(source: u8, voice: &harmonigraph_core::canonical::VoiceBaseline) -> Self {
        Self {
            source,
            lifetime: voice.lifetime,
            correction: voice.frozen_offset_microcents,
            player: voice.player_tuning,
            key: voice.note,
            channel: voice.channel,
            pitch: voice.pitch_microcents,
            node: voice.attack_node,
            configuration_revision: voice.assignment.map_or(0, |config| config.revision),
            decision: voice.decision,
        }
    }
    fn tune(&mut self, player: f64) {
        self.player = player;
        self.pitch = i64::from(self.key) * 100_000_000
            + self.correction
            + (player * 100_000_000.0).round() as i64;
    }
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
    pub captured: [u64; TUNERS + 1],
    membership: Option<Membership>,
    config: Option<ResolvedConfig>,
    binding_sample: i64,
    pub(super) decision: u64,
    cohort_floor: u64,
    cohort_unsent: usize,
    cohort_recipients: u16,
    committing: bool,
    pub finalized: Option<i64>,
    pub copied: Option<i64>,
    /// One LIFETIMES-long ledger per PAIRED row, and nothing at all for a row
    /// that has never been paired. The registry allocates a row's ledger on
    /// the main thread when it hands out that row's lease; the Hub only ever
    /// moves the box in. Plan indices stay flat — `row * LIFETIMES + request` —
    /// so the intrusive list still links freely across rows.
    plans: [Option<super::PlanRow>; TUNERS],
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
    pub(super) history: history::History,
    policy: Box<policy::PolicyScratch>,
    policy_context: Box<[policy::ContextPitch]>,
    pub(super) participating: [bool; TUNERS + 1],
    pub(super) participation_serial: [u64; TUNERS + 1],
    pub work: usize,
    pub extra_delay: u64,
    #[cfg(test)]
    policy_counts: [usize; 3],
}
impl Default for Sequencer {
    fn default() -> Self {
        Self {
            retired: false,
            terminal_session: false,
            terminal_sources: 0,
            captured: [0; TUNERS + 1],
            membership: None,
            config: None,
            binding_sample: 0,
            decision: 0,
            cohort_floor: 0,
            cohort_unsent: 0,
            cohort_recipients: 0,
            committing: false,
            finalized: None,
            copied: None,
            // DIRECT has no plan row. Birth indices are already separately
            // reserved at Source; they are never authority without the key.
            plans: std::array::from_fn(|_| None),
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
            history: history::History::default(),
            policy: Box::default(),
            policy_context: vec![
                policy::ContextPitch {
                    pitch: PitchClass::from_microcents(0),
                    node: None
                };
                HELD_SESSION
            ]
            .into_boxed_slice(),
            participating: [true; TUNERS + 1],
            participation_serial: [0; TUNERS + 1],
            work: 0,
            extra_delay: 0,
            #[cfg(test)]
            policy_counts: [0; 3],
        }
    }
}
impl Sequencer {
    pub(super) fn can_reset_clock_context(&self) -> bool {
        self.actual_revision.checked_add(1).is_some()
    }
    pub(super) fn clear_clock_context(&mut self) {
        self.history.clear_all(self.decision);
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
            let value = Voice::factual(0, voice);
            self.actual[usize::from(index)] = Some(value);
            self.context[usize::from(index)] = Some(value);
            self.actual_keys[usize::from(index)] =
                Some(ActualKey { lease, epoch, lifetime: voice.lifetime });
            self.actual_index[actual_address(lease, voice.channel, voice.note)] = index;
        }
        // Absent identities invalidate old hints without scanning the 34816
        // address directory. No physical debt may reach this committed cut.
    }
    pub(super) fn install_plan_row(&mut self, row: usize, ledger: super::PlanRow) {
        self.plans[row] = Some(ledger);
    }
    #[cfg(test)]
    fn plan_ledger_bytes(&self) -> usize {
        self.plans.iter().flatten().map(|row| std::mem::size_of_val(&*row.0)).sum()
    }
    fn plan(&self, index: usize) -> Option<Plan> {
        self.plans[index / LIFETIMES].as_ref()?.0[index % LIFETIMES]
    }
    fn plan_mut(&mut self, index: usize) -> Option<&mut Plan> {
        self.plans[index / LIFETIMES].as_mut()?.0[index % LIFETIMES].as_mut()
    }
    /// None only for a row with no ledger, which is a row that was never
    /// paired. Every plan index reaching this comes from a leased row.
    fn plan_cell(&mut self, index: usize) -> Option<&mut Option<Plan>> {
        Some(&mut self.plans[index / LIFETIMES].as_mut()?.0[index % LIFETIMES])
    }
    /// False when the slot is already live: overwriting it would strand the
    /// old plan's links in the intrusive list and overcount `plan_count`, so
    /// the caller latches a fault and abandons the insert instead. Also false
    /// for a row whose ledger the pairing boundary has not delivered.
    #[must_use]
    fn insert_plan(&mut self, index: usize, mut plan: Plan) -> bool {
        match self.plan_cell(index) {
            Some(cell) if cell.is_none() => {}
            _ => return false,
        }
        plan.previous = self.plan_tail;
        plan.next = NO_PLAN;
        if self.plan_tail == NO_PLAN {
            self.plan_head = index as u32;
        } else {
            self.plan_mut(self.plan_tail as usize).unwrap().next = index as u32;
        }
        self.plan_tail = index as u32;
        self.plan_count += 1;
        *self.plan_cell(index).unwrap() = Some(plan);
        true
    }
    fn remove_plan(&mut self, index: usize) {
        let plan = self.plan_cell(index).unwrap().take().unwrap();
        if plan.previous == NO_PLAN {
            self.plan_head = plan.next;
        } else {
            self.plan_mut(plan.previous as usize).unwrap().next = plan.next;
        }
        if plan.next == NO_PLAN {
            self.plan_tail = plan.previous;
        } else {
            self.plan_mut(plan.next as usize).unwrap().previous = plan.previous;
        }
        self.plan_count -= 1;
    }
    /// False only when plan accounting is already broken; a rejected argument
    /// is a normal no-op, not a fault.
    #[must_use]
    pub(super) fn cancel(
        &mut self,
        source: usize,
        lease: Lease,
        epoch: u64,
        serial: u64,
        request: u16,
        lifetime: u64,
    ) -> bool {
        if serial == 0 || lifetime == 0 || usize::from(request) >= LIFETIMES {
            return true;
        }
        let index = source * LIFETIMES + usize::from(request);
        let identity = Request { lease, epoch, serial, request, lifetime };
        if let Some(plan) = self.plan_mut(index) {
            if plan.request == identity {
                plan.terminal = true;
            }
            true
        } else {
            // A cancellation can overtake the onset's own copied record. Hold
            // its exact identity until that record passes the input cursor, so
            // ordinary sequencing cannot resurrect it as a fresh onset.
            self.insert_plan(
                index,
                Plan {
                    request: identity,
                    binding: Assignment::default(),
                    shift: DELAY,
                    sent: false,
                    terminal: true,
                    accepted: false,
                    bound: false,
                    next: NO_PLAN,
                    previous: NO_PLAN,
                },
            )
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
        // Node and decision are now populated in Voice. Its revision occupies
        // eight bytes of the prepaid complete configuration; reserve the rest.
        + (128 - std::mem::size_of::<u64>())
        <= 256
);
const _: () = assert!(std::mem::size_of::<Option<Voice>>() + 128 - 8 <= 256);

#[cfg(test)]
impl Sequencer {
    pub(super) fn print_test_memory_layout(&self) {
        println!(
            "LEDGER musical [history_cell,prospective] {:?}; policy [scratch,context] {:?}",
            self.history.layout(),
            [std::mem::size_of_val(&*self.policy), std::mem::size_of_val(&*self.policy_context)]
        );
        println!(
            "LEDGER factual lookup [key_cell,key_backing,directory_backing] {:?}",
            [
                std::mem::size_of::<Option<ActualKey>>(),
                std::mem::size_of_val(&*self.actual_keys),
                std::mem::size_of_val(&*self.actual_index)
            ]
        );
        println!(
            "LEDGER sequencer actual [backing,free_capacity,free_metadata] {:?}",
            [
                std::mem::size_of_val(&*self.actual),
                self.actual_free.capacity() * std::mem::size_of::<u16>(),
                std::mem::size_of_val(&self.actual_free),
            ]
        );
        println!(
            "LEDGER sequencer [owner,plan_option,paired_row_backing,plan_backing,voice_option,voice_backing] {:?}",
            [
                std::mem::size_of::<Self>(),
                std::mem::size_of::<Option<Plan>>(),
                Hub::test_plan_row_bytes(),
                self.plan_ledger_bytes(),
                std::mem::size_of::<Option<Voice>>(),
                std::mem::size_of_val(&*self.context)
            ]
        );
    }
}

impl Hub {
    /// What one paired row's ledger costs, so a fixture can say which rows are
    /// allocated rather than repeating a byte count that would drift.
    #[cfg(test)]
    pub(in crate::performance) fn test_plan_row_bytes() -> usize {
        std::mem::size_of::<Option<Plan>>() * LIFETIMES
    }
    #[cfg(test)]
    pub(in crate::performance) fn test_plan_ledger_bytes(&self) -> usize {
        self.sequencer.plan_ledger_bytes()
    }
    #[cfg(test)]
    pub(in crate::performance) fn test_policy_counts(&self) -> [usize; 3] {
        self.sequencer.policy_counts
    }
    #[cfg(test)]
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
    #[cfg(test)]
    pub(in crate::performance) fn test_terminal_scope(&self) -> (bool, u16) {
        (self.sequencer.terminal_session, self.sequencer.terminal_sources)
    }
    /// Drop the cohort's outstanding delivery count while the plans that owe
    /// it stay unsent — the accounting violation `service_plans` latches. No
    /// MIDI input reaches that state, so a test constructs it. Returns the
    /// debt cleared, so a fixture with nothing to break cannot read as one.
    #[cfg(test)]
    pub(in crate::performance) fn test_clear_cohort_unsent(&mut self) -> usize {
        std::mem::take(&mut self.sequencer.cohort_unsent)
    }
    #[cfg(test)]
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
        #[cfg(test)]
        if self.test_aggregation {
            return false;
        }
        !self.sequencer.retired
    }

    fn inputs(&mut self, source: usize) -> &mut Queue<Capture, CAPTURES_PER_SOURCE> {
        if source == 0 {
            &mut self.direct_inputs
        } else {
            &mut self.rows[source - 1].inputs
        }
    }

    fn input_snapshot(&mut self, owner: &Owner) -> Option<Membership> {
        self.trace.input_wait = 0;
        self.trace.input_source = 0;
        let Some((direct, cut)) = self.direct.completed_input() else {
            self.trace.input_wait = 1;
            return None;
        };
        if cut > self.sequencer.captured[0] {
            self.trace.input_wait = 2;
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
                self.trace.input_wait = 3;
                self.trace.input_source = index + 1;
                return None;
            }
            if row.acknowledged_membership == 0
                || row.input_membership != row.acknowledged_membership
            {
                self.trace.input_wait = 4;
                self.trace.input_source = index + 1;
                return None;
            }
            let Some(interval) = row.input_coverage else {
                self.trace.input_wait = 5;
                self.trace.input_source = index + 1;
                return None;
            };
            if interval.1 > self.sequencer.captured[index + 1] {
                self.trace.input_wait = 6;
                self.trace.input_source = index + 1;
                return None;
            }
            snapshot.leases[index] = Some(lease);
            snapshot.intervals[index] = Some(interval);
            snapshot.floor = snapshot.floor.max(interval.0.start);
            snapshot.through = snapshot.through.min(interval.0.through);
        }
        if snapshot.through <= snapshot.floor {
            self.trace.input_wait = 7;
            return None;
        }
        Some(snapshot)
    }

    /// The merge front: the earliest sample any source still owes. Records
    /// within a source are already in input order, so this is one look at
    /// seventeen queue fronts.
    fn next_input_sample(&mut self) -> Option<i64> {
        let mut sample = None;
        for source in 0..=TUNERS {
            if source != 0 && self.rows[source - 1].terminal_cut.is_some() {
                continue;
            }
            if let Some(record) = self.inputs(source).front() {
                sample = Some(sample.map_or(record.sample, |old: i64| old.min(record.sample)));
            }
        }
        sample
    }

    /// A resource or host-output fault is a latched shared reset, not a
    /// recovery pass: it closes every affected emission gate, faults the
    /// sources so their own Stop path cancels unsounded attacks and arms the
    /// downstream release debt, and latches `terminal_session` so no new
    /// attack is admitted until an explicit Reset clears it in
    /// `commit_transition`. Obsolete replies die with the epoch that latched.
    pub(super) fn observe_terminal_faults(&mut self) {
        if !self.sequences_inputs() && !self.sequencer.retired {
            return;
        }
        let Some(offer) = &self.offer else { return };
        let mut faults = offer.session.faults.load(Ordering::Acquire)
            & !super::super::source::TIMING_FAILURE;
        let mut local = 0u16;
        for (source, row) in self.rows.iter().enumerate() {
            let bits = offer.session.rows[source].faults.load(Ordering::Acquire)
                & !super::super::source::TIMING_FAILURE;
            if row.lease.is_none()
                || bits == 0
                || self.sequencer.terminal_sources & (1 << source) != 0
            {
                continue;
            }
            if row.member
                || self
                    .sequencer
                    .membership
                    .is_some_and(|membership| membership.leases[source].is_some())
            {
                faults |= bits;
            } else {
                local |= 1 << source;
            }
        }
        if faults != 0 && !self.sequencer.terminal_session {
            offer.session.faults.fetch_or(faults, Ordering::AcqRel);
            self.sequencer.terminal_session = true;
            local = u16::MAX;
            self.direct.fault(faults);
        }
        if local == 0 {
            return;
        }
        self.sequencer.terminal_sources |= local;
        for source in 0..TUNERS {
            if local & (1 << source) == 0 || self.rows[source].lease.is_none() {
                continue;
            }
            self.offer.as_ref().unwrap().session.rows[source]
                .emission_gate
                .fetch_or(super::super::source::CLOSED, Ordering::AcqRel);
            // The latch terminates this row's input stream where the Hub has
            // already settled it. Sequencing stops waiting for its coverage,
            // and its pending baseline may now be acknowledged so the Source's
            // own settlement can finish; explicit Reset clears the cut.
            if self.rows[source].terminal_cut.is_none() {
                self.rows[source].terminal_cut = Some(self.rows[source].input_settled.0);
                if !self.rows[source].member {
                    // A refused provisional join never contributed coverage,
                    // so its proposed floor cannot pin other Sources.
                    self.rows[source].joining = None;
                }
            }
        }
    }

    pub(super) fn sequence_inputs(&mut self, owner: &mut Owner, recorder: &mut Recorder) {
        self.observe_terminal_faults();
        if self.sequencer.terminal_session {
            self.service_plans();
            return;
        }
        if !self.sequences_inputs() || !self.clock.valid || self.invalidated {
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
            let Some(membership) = self.input_snapshot(owner) else { return };
            let sample = self.next_input_sample();
            let boundary = sample.map_or(membership.through, |sample| sample.max(membership.floor));
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
            self.batch.begin(sample);
            self.sequencer.membership = Some(membership);
            self.sequencer.binding_sample = boundary;
            self.sequencer.history.configuration(config.revision, self.sequencer.decision);
            self.sequencer.config = Some(config);
            self.sequencer.cohort_floor = self.sequencer.decision;
            self.sequencer.cohort_unsent = 0;
            self.sequencer.cohort_recipients = 0;
            // One sample, start to finish, inside this callback: assemble the
            // copies, sort them once, then apply them in that order. A step
            // that fails has already latched the shared reset, so the batch is
            // abandoned rather than resumed.
            if !self.assemble_inputs() || !self.apply_batch(owner) {
                self.batch.end();
                return;
            }
            self.batch.end();
            self.sequencer.committing = true;
            if !self.publish_cohort() {
                return;
            }
        }
    }

    /// Take every copied record standing at this sample, from every source,
    /// and put the batch in merge order.
    fn assemble_inputs(&mut self) -> bool {
        let sample = self.batch.sample;
        for source in 0..=TUNERS {
            loop {
                if self.input_work == 4096 {
                    return false;
                }
                match self.inputs(source).front() {
                    Some(record) if record.sample == sample => {}
                    _ => break,
                }
                let record = self.inputs(source).pop().unwrap();
                self.input_work += 1;
                if !self.batch.push(record) {
                    // More same-sample records than one pass can hold.
                    self.configuration_exhausted();
                    return false;
                }
            }
        }
        self.batch.order();
        true
    }

    /// Apply the ordered batch: every release and controller from every source
    /// first, then the onsets. Assignment order inside the second half is the
    /// key/channel/source tie-break the musical decision has always used.
    fn apply_batch(&mut self, owner: &mut Owner) -> bool {
        while let Some(record) = self.batch.next() {
            if !self.apply(record) {
                return false;
            }
            if matches!(record.kind, CaptureKind::Participation(_)) && record.lease.slot != 0 {
                Self::confirm(&self.rows[usize::from(record.lease.slot) - 1], &mut owner.confirmed);
            }
            self.sequencer.work += 1;
        }
        true
    }

    /// Apply one copied record. This is the whole of what a record does to the
    /// Hub's musical state; the ordering pass has already put them in the order
    /// that makes it correct.
    fn apply(&mut self, record: Capture) -> bool {
        let source = usize::from(record.lease.slot);
        if let CaptureKind::Participation(value) = record.kind {
            if record.serial > self.sequencer.participation_serial[source] {
                self.sequencer.participation_serial[source] = record.serial;
                self.sequencer.participating[source] = value;
                if !value {
                    self.sequencer.history.clear(source, self.sequencer.decision);
                }
                if source != 0 {
                    self.rows[source - 1].participating = value;
                    self.rows[source - 1].repair = true;
                }
            }
            return true;
        }
        if !record.onset() {
            let voice = self.sequencer.context.iter_mut().find(|cell| {
                cell.is_some_and(|voice| {
                    voice.source == record.lease.slot && voice.lifetime == record.lifetime
                })
            });
            match (record.kind, voice) {
                (CaptureKind::Terminal, Some(cell)) => *cell = None,
                (CaptureKind::Tuning { value_bits }, Some(cell)) => {
                    cell.as_mut().unwrap().tune(f64::from_bits(value_bits))
                }
                // A pitch expression that arrives at its own note's sample has
                // no voice to reach yet: it is the value that note starts from.
                (CaptureKind::Tuning { value_bits }, None) => {
                    if !self.batch.stash_tuning(record.lease.slot, record.lifetime, value_bits) {
                        self.configuration_exhausted();
                        return false;
                    }
                }
                _ => {}
            }
            return true;
        }
        let request =
            Request { lease: record.lease, epoch: record.epoch, serial: record.serial,
                request: record.request, lifetime: record.lifetime };
        let prior = (source != 0)
            .then(|| self.sequencer.plan((source - 1) * LIFETIMES + usize::from(record.request)))
            .flatten();
        if let Some(prior) = prior {
            if prior.request != request {
                return false;
            }
            if prior.terminal {
                // Its cancellation arrived before its own record. Binding the
                // plan here is what lets `service_plans` retire it, and keeps
                // ordinary sequencing from assigning it as a fresh onset.
                let config = self.sequencer.config;
                let plan = self
                    .sequencer
                    .plan_mut((source - 1) * LIFETIMES + usize::from(record.request))
                    .unwrap();
                if !plan.bound {
                    plan.binding.configuration = config.unwrap_or(plan.binding.configuration);
                }
                plan.bound = true;
                return true;
            }
            if prior.bound {
                return false;
            }
        }
        let Some(decision) = self.sequencer.decision.checked_add(1) else { return false };
        let configuration = prior
            .filter(|plan| plan.bound)
            .map(|plan| plan.binding.configuration)
            .or(self.sequencer.config)
            .expect("owned original cohort configuration");
        let (correction, selection) = if source != 0 && record.adaptive {
            let mut count = 0;
            for voice in self.sequencer.context.iter().flatten() {
                if self.sequencer.participating[usize::from(voice.source)] {
                    self.sequencer.policy_context[count] = policy::ContextPitch {
                        pitch: PitchClass::from_microcents(voice.pitch),
                        node: voice.node,
                    };
                    count += 1;
                }
            }
            #[cfg(test)]
            {
                self.sequencer.policy_counts[0] += 1;
                self.sequencer.policy_counts[1] += count;
                self.sequencer.policy_counts[2] = self.sequencer.policy_counts[2].max(count);
            }
            let history = self.sequencer.history.previous(
                record.lease,
                record.channel,
                record.key,
                configuration.revision,
            );
            let Ok(selection) = policy::assign_new_note(
                configuration.into(),
                &self.sequencer.policy_context[..count],
                history,
                policy::OrderedOnset { key: record.key },
                &mut self.sequencer.policy,
            ) else {
                self.configuration_exhausted();
                return false;
            };
            let node = match selection.assignment {
                policy::Assignment::Selected { node, .. } => Selection::Node([
                    i8::try_from(node.threes).expect("bounded canonical threes"),
                    i8::try_from(node.fives).expect("bounded canonical fives"),
                    i8::try_from(node.sevens).expect("bounded canonical sevens"),
                ]),
                policy::Assignment::NoCandidate => Selection::NoCandidate,
            };
            (selection.assignment.correction_microcents(), node)
        } else {
            (0, Selection::Unretuned)
        };
        let player =
            self.batch.initial_tuning(record.lease.slot, record.lifetime).unwrap_or(0.0);
        let Some(slot) = self.sequencer.context.iter().position(Option::is_none) else {
            self.configuration_exhausted();
            return false;
        };
        if source != 0 {
            let index = (source - 1) * LIFETIMES + usize::from(record.request);
            let emission = self.offer.as_ref().unwrap().session.rows[source - 1]
                .emission_gate
                .load(Ordering::Acquire)
                & !super::super::source::GATE_FLAGS;
            let binding = Assignment {
                configuration,
                decision,
                emission,
                correction,
                selection,
                initial_player: player,
            };
            if let Some(plan) = self.sequencer.plan_mut(index) {
                plan.request = request;
                plan.binding = binding;
                plan.sent = false;
                plan.bound = true;
                plan.shift = DELAY;
            } else if !self.sequencer.insert_plan(
                index,
                Plan {
                    request,
                    binding,
                    shift: DELAY,
                    sent: false,
                    terminal: false,
                    accepted: false,
                    bound: true,
                    next: NO_PLAN,
                    previous: NO_PLAN,
                },
            ) {
                self.configuration_exhausted();
                return false;
            }
            if record.adaptive && self.sequencer.participating[source] {
                self.sequencer.history.commit(record.lease, record.channel, record.key, binding);
            }
            self.sequencer.cohort_unsent += 1;
            self.sequencer.cohort_recipients |= 1 << (source - 1);
            let reply = Reply::Assignment { request, binding };
            if self.offer.as_mut().unwrap().bank.rows[source - 1].replies.push(reply).is_ok() {
                self.sequencer.plan_mut(index).unwrap().sent = true;
                self.sequencer.cohort_unsent -= 1;
            }
        }
        self.sequencer.context[slot] = Some(Voice {
            source: record.lease.slot,
            lifetime: record.lifetime,
            correction: i64::from(correction),
            player,
            key: record.key,
            channel: record.channel,
            pitch: i64::from(record.key) * 100_000_000
                + i64::from(correction)
                + (player * 100_000_000.0).round() as i64,
            node: selection.node(),
            configuration_revision: configuration.revision,
            decision,
        });
        self.sequencer.decision = decision;
        true
    }

    pub(super) fn plan_callback(&mut self) {
        self.sequencer.plan_work = 0;
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
            let lease = self.sequencer.membership.unwrap().leases[source].unwrap();
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
        let plan = self.sequencer.plan_mut(source * LIFETIMES + usize::from(request))?;
        if self.rows[source].lease != Some(plan.request.lease)
            || !plan.bound
            || output.epoch != plan.request.epoch
            || output.lifetime != plan.request.lifetime
            || output.decision != plan.binding.decision
        {
            return None;
        }
        if output.event.release() {
            plan.terminal = true;
        }
        let planned = output.input.checked_add(plan.shift)?;
        let mut accepted_shift = None;
        if output.event.attack().is_some() {
            plan.accepted = true;
            plan.shift = output.actual.checked_sub(output.input)?;
            accepted_shift = Some(plan.shift);
        }
        let binding = plan.binding;
        if let Some(shift) = accepted_shift {
            self.sequencer.extra_delay =
                self.sequencer.extra_delay.max(shift.saturating_sub(DELAY).max(0) as u64);
        }
        Some((binding, planned))
    }

    pub(super) fn service_plans(&mut self) {
        if self.sequencer.plan_left == 0 {
            self.sequencer.plan_cursor = self.sequencer.plan_head;
            self.sequencer.plan_left = self.sequencer.plan_count;
        }
        while self.sequencer.plan_work < 256 && self.sequencer.plan_left != 0 {
            let index = self.sequencer.plan_cursor as usize;
            let plan = self.sequencer.plan(index).unwrap();
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
                // A plan that still owes this cohort a reply while the unsent
                // count reads zero is broken accounting, not a full queue.
                // Latch it and abandon this plan for the pass; wrapping the
                // counter would corrupt every later cohort, and panicking on
                // the audio thread takes the host's engine down with it.
                let Some(remaining) = self.sequencer.cohort_unsent.checked_sub(1) else {
                    self.configuration_exhausted();
                    continue;
                };
                self.sequencer.cohort_unsent = remaining;
                self.sequencer.plan_mut(index).unwrap().sent = true;
            }
            // A canceled onset still has to pass the input cursor, so normal
            // sequencing cannot resurrect it. A terminated or retired row will
            // never deliver that record — its stream is settled where it
            // stopped and its emission gate is closed — so retire it there.
            if plan.terminal
                && !plan.bound
                && !self.sequencer.retired
                && self.rows[source].terminal_cut.is_none()
            {
                continue;
            }
            if !plan.terminal && !plan.bound {
                continue;
            }
            let reply = if plan.terminal {
                Reply::PlanRetired {
                    incarnation: plan.request.lease.incarnation,
                    epoch: plan.request.epoch,
                    life,
                    lifetime: plan.request.lifetime,
                    decision: plan.binding.decision,
                }
            } else if !plan.sent && !self.sequencer.retired {
                Reply::Assignment { request: plan.request, binding: plan.binding }
            } else {
                continue;
            };
            let owes_cohort = !self.sequencer.retired
                && !plan.terminal
                && !plan.sent
                && plan.binding.decision > self.sequencer.cohort_floor;
            // Same broken accounting as above, tested before the push so the
            // reply is abandoned along with the plan rather than leaving the
            // cohort a delivery it can no longer account for.
            if owes_cohort && self.sequencer.cohort_unsent == 0 {
                self.configuration_exhausted();
                continue;
            }
            if self.offer.as_mut().unwrap().bank.rows[source].replies.push(reply).is_err() {
                continue;
            }
            if owes_cohort {
                self.sequencer.cohort_unsent -= 1;
            }
            if plan.terminal {
                self.sequencer.remove_plan(index);
            } else {
                self.sequencer.plan_mut(index).unwrap().sent = true;
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
        if let Some(fact) = voice {
            // Do not overwrite a later scheduled bend or resurrect a scheduled
            // release. Once this same planned expression is accepted, retain
            // the exact wire pitch (including its boundary rounding).
            if let Some(planned) = self.context.iter_mut().flatten().find(|planned| {
                planned.source == fact.source
                    && planned.lifetime == fact.lifetime
                    && planned.decision == fact.decision
                    && planned.correction == fact.correction
                    && planned.player == fact.player
            }) {
                planned.pitch = fact.pitch;
            }
        }
        if index != NO_VOICE {
            // The resolved slot no longer carries this identity, so writing it
            // would overwrite an unrelated voice. Err is the caller's existing
            // exhaustion channel; it latches the fault and drops this store.
            if !self.actual_hint(key, index) {
                return Err(());
            }
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
            // The free list handed back an occupied slot. Its bookkeeping is
            // already inconsistent, so leak the slot rather than pushing it
            // back: returning it would re-offer the same corrupt entry.
            if self.actual[slot].is_some() {
                return Err(());
            }
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
        let voice = voice.map(|voice| Voice::factual(lookup.key.lease.slot, &voice));
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
            self.actual[usize::from(index)] = Some(Voice::factual(source, voice));
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
                channel: delta.event.channel,
                pitch: delta.pitch_microcents.unwrap_or(i64::from(delta.event.note) * 100_000_000),
                node: None,
                configuration_revision: 0,
                decision: 0,
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
                voice.pitch = pitch;
                Some(voice)
            }
            _ => return true,
        };
        self.store_actual(lookup, voice).is_ok()
    }
}
