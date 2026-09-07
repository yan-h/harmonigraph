//! The Hub's owned original-input cursor, separate from actual-output progress.
use super::*;
use harmonigraph_core::configuration::ResolvedConfig;
use harmonigraph_core::{policy, LatticePos, PitchClass};
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
    plans: [Option<Box<[Option<Plan>]>>; TUNERS],
    plan_head: u32,
    plan_tail: u32,
    plan_cursor: u32,
    plan_left: usize,
    plan_count: usize,
    plan_work: usize,
    /// Every note the Hub believes is sounding, prospective and factual in one
    /// table: a copied onset record puts a voice here with the pitch it was
    /// assigned, and the accepted output that realizes it snaps that pitch to
    /// the exact wire value. A terminal record removes it. The Hub's own
    /// authoritative sounding-note facts stay where they always were, in each
    /// row's `State`; this is the policy's context, not a second copy of them.
    context: Box<[Option<Voice>]>,
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
    /// A clock boundary empties the Hub's belief about what is sounding.
    ///
    /// Nothing is reseeded across it. The copied input records own every cell
    /// here and number their notes in the Tune's own lifetime space, so a
    /// voice put here from anywhere else is a voice no later Terminal record
    /// can address — it would sit in the policy's context until the next
    /// clock boundary swept it out. The DIRECT observation this used to be
    /// seeded from keeps its own held notes in `Direct::state`, which is
    /// where display and recording read them; and a boundary is only
    /// committed once forwarding has settled, so nothing is still sounding
    /// for the seeded voices to have represented.
    pub(super) fn clear_clock_context(&mut self) {
        self.history.clear_all(self.decision);
        for cell in self.context.iter_mut() {
            *cell = None;
        }
    }
    /// A settled reset retires the cohort still in flight along with the
    /// session that owned it.
    ///
    /// A cohort stays `committing` whenever its marker cannot be published --
    /// a full reply ring or a spent plan budget is enough -- and a terminal
    /// fault then leaves it there, because the faulted path never reaches
    /// `publish_cohort`. Its recipients' leases die at this boundary, so the
    /// barrier could never be satisfied again: `publish_cohort` refuses on the
    /// lease mismatch and `sequence_inputs` returns on it before it looks at
    /// any fresh input. Epoch validation retires the obsolete REPLIES; nothing
    /// retired the coordinator work that was waiting for them.
    ///
    /// The floor moves with the debt. Every plan the old cohort minted has a
    /// decision at or below the current one, so clearing the debt without
    /// moving the floor would leave those plans claiming a delivery against a
    /// count of zero -- which `service_plans` reads as broken accounting and
    /// latches.
    pub(super) fn retire_cohort(&mut self) {
        self.committing = false;
        self.cohort_recipients = 0;
        self.cohort_unsent = 0;
        self.cohort_floor = self.decision;
        self.membership = None;
        self.config = None;
    }
    pub(super) fn install_plan_row(&mut self, row: usize, ledger: Box<[Option<Plan>]>) {
        self.plans[row] = Some(ledger);
    }
    #[cfg(test)]
    fn plan_ledger_bytes(&self) -> usize {
        self.plans.iter().flatten().map(|row| std::mem::size_of_val(&**row)).sum()
    }
    fn plan(&self, index: usize) -> Option<Plan> {
        self.plans[index / LIFETIMES].as_ref()?[index % LIFETIMES]
    }
    fn plan_mut(&mut self, index: usize) -> Option<&mut Plan> {
        self.plans[index / LIFETIMES].as_mut()?[index % LIFETIMES].as_mut()
    }
    /// None only for a row with no ledger, which is a row that was never
    /// paired. Every plan index reaching this comes from a leased row.
    fn plan_cell(&mut self, index: usize) -> Option<&mut Option<Plan>> {
        Some(&mut self.plans[index / LIFETIMES].as_mut()?[index % LIFETIMES])
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
            if plan.request != identity {
                return true;
            }
            plan.terminal = true;
            // An authoritative cancellation ends the note as surely as a
            // release does. Marking the plan alone would leave the voice this
            // onset prospectively inserted scoring every later note, because
            // a canceled attack never produces the output that would clear it.
            self.forget_voice(identity.lease.slot, lifetime);
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
const _: () = assert!(
    std::mem::size_of::<Option<Plan>>() - std::mem::size_of::<ResolvedConfig>() + 128 <= 256
);
const _: () = assert!(std::mem::size_of::<Option<Voice>>() <= 256);
// One context cell keeps full room for a future complete configuration beside
// the eight bytes its revision already occupies.
const _: () = assert!(std::mem::size_of::<Option<Voice>>() + 128 - 8 <= 256);

#[cfg(test)]
impl Sequencer {
    /// Every voice the policy would score a fresh onset against, as
    /// `(source slot, lifetime)`. Distinct from a row's factual `State`: this
    /// is what tuning reads, and a note the Hub no longer believes is sounding
    /// has to be gone from BOTH.
    pub(super) fn test_context(&self) -> Vec<(u8, u64)> {
        self.context.iter().flatten().map(|voice| (voice.source, voice.lifetime)).collect()
    }
    pub(super) fn print_test_memory_layout(&self) {
        println!(
            "LEDGER musical [history_cell,prospective] {:?}; policy [scratch,context] {:?}",
            self.history.layout(),
            [std::mem::size_of_val(&*self.policy), std::mem::size_of_val(&*self.policy_context)]
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
    /// The Hub's belief about one sounding note: its slot, its frozen adaptive
    /// correction and the player tuning last applied to it.
    #[cfg(test)]
    pub(in crate::performance) fn test_context_voice(
        &self,
        source: u8,
        lifetime: u64,
    ) -> Option<(usize, i64, f64)> {
        self.sequencer.context.iter().enumerate().find_map(|(index, cell)| {
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
        let mut faults =
            offer.session.faults.load(Ordering::Acquire) & !super::super::source::TIMING_FAILURE;
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
            // Collection and assembly spend one allowance, so a callback that
            // spent most of it collecting cannot be trusted to finish taking
            // this sample. Reserve the whole sample before removing any of it:
            // a partly consumed batch has nowhere to go, since `end` clears
            // what was popped and those records are already gone from their
            // queues. Too big for the pass at all is the bounded failure; too
            // big for what is left of this callback is a wait, and the next
            // callback starts the allowance over with room for any sample.
            let owed = self.same_sample_records(sample);
            if owed > BATCH_EVENTS {
                self.configuration_exhausted();
                return;
            }
            if self.input_work + owed > 4096 {
                return;
            }
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
            // copies, sort them once, then apply them in that order. The
            // records are out of their queues from here, so a step that fails
            // is a terminal fault and never a deferral — several of `apply`'s
            // own refusals latch nothing on their own.
            if !self.assemble_inputs() || !self.apply_batch(owner) {
                self.configuration_exhausted();
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

    /// What this sample owes across every source, leaving all of it in place.
    /// Records within a source are in input order, so each source's share is
    /// the run standing at its queue front. Counting stops one past the batch
    /// capacity: beyond that the answer is only "too many", and the scan has
    /// no reason to walk seventeen full queues to say so.
    fn same_sample_records(&mut self, sample: i64) -> usize {
        let mut owed = 0;
        for source in 0..=TUNERS {
            let mut offset = 0;
            while owed <= BATCH_EVENTS
                && self.inputs(source).get(offset).is_some_and(|record| record.sample == sample)
            {
                offset += 1;
                owed += 1;
            }
        }
        owed
    }

    /// Take every copied record standing at this sample, from every source,
    /// and put the batch in merge order. The caller reserved room for the
    /// whole sample in both the callback allowance and the batch, so false is
    /// a latched fault rather than the deferral it used to be.
    #[must_use]
    fn assemble_inputs(&mut self) -> bool {
        let sample = self.batch.sample;
        for source in 0..=TUNERS {
            while self.inputs(source).front().is_some_and(|record| record.sample == sample) {
                let record = self.inputs(source).pop().unwrap();
                self.input_work += 1;
                if !self.batch.push(record) {
                    // Reserved above, so unreachable; latching rather than
                    // asserting keeps a wrong reservation off the host's
                    // audio thread.
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
                // Release-first ordering puts a terminal ahead of the onset it
                // addresses whenever one lifetime is born and ends at a single
                // sample. Retain the identity so that onset does not leave a
                // live context entry for a note this sample already ended.
                (CaptureKind::Terminal, None) => {
                    self.batch.ended(record.lease.slot, record.lifetime)
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
        let request = Request {
            lease: record.lease,
            epoch: record.epoch,
            serial: record.serial,
            request: record.request,
            lifetime: record.lifetime,
        };
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
        let player = self.batch.initial_tuning(record.lease.slot, record.lifetime).unwrap_or(0.0);
        // A lifetime this sample already ended still gets its assignment — the
        // Tune is waiting for one — but never becomes context, because the
        // release that ended it applied before this onset existed.
        let slot = if self.batch.already_ended(record.lease.slot, record.lifetime) {
            None
        } else if let Some(slot) = self.sequencer.context.iter().position(Option::is_none) {
            Some(slot)
        } else {
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
            if self.row_replies(source - 1).is_some_and(|row| row.replies.push(reply).is_ok()) {
                self.sequencer.plan_mut(index).unwrap().sent = true;
                self.sequencer.cohort_unsent -= 1;
            }
        }
        if let Some(slot) = slot {
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
        }
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
            if self.row_replies(source).is_none_or(|row| row.replies.push(reply).is_err()) {
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
            if self.row_replies(source).is_none_or(|row| row.replies.push(reply).is_err()) {
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
    /// Accepted output realizing one planned voice: keep the exact wire pitch,
    /// boundary rounding included, so later assignments see what actually
    /// sounded rather than what was intended. Only when the accepted fact
    /// still agrees with the plan — a report overtaken by a newer bend or by
    /// a scheduled release must not move or resurrect a live context voice.
    ///
    /// One bounded scan of the 256 context cells, charged to nothing: the
    /// table it replaced paid a keyed directory hint plus, on a miss, a
    /// 256-cell fallback against the same per-callback budget the merge loop
    /// spends on output.
    /// A note the Hub no longer believes is sounding leaves no policy context
    /// behind it. The ordinary path is the addressed Terminal record in
    /// `apply`; this is for the endings that never become one.
    fn forget_voice(&mut self, source: u8, lifetime: u64) {
        if let Some(cell) = self.context.iter_mut().find(|cell| {
            cell.is_some_and(|voice| voice.source == source && voice.lifetime == lifetime)
        }) {
            *cell = None;
        }
    }

    pub(super) fn accepted_output(
        &mut self,
        source: u8,
        value: OutputDelta,
        voice: Option<&harmonigraph_core::canonical::VoiceBaseline>,
    ) {
        let Some(voice) = voice.filter(|voice| voice.lifetime != 0) else {
            // An accepted termination the Hub's factual row has already applied
            // away. A Stop or a membership withdrawal ends its forwarded voices
            // through emergency releases, which reach the Hub as output and
            // never as an addressed Terminal record, so this is the only place
            // that can take them out of the policy's context.
            if value.lifetime != 0
                && (value.event.release() || value.outcome.channel_terminal().is_some())
            {
                self.forget_voice(source, value.lifetime);
            }
            return;
        };
        let fact = Voice::factual(source, voice);
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
}
