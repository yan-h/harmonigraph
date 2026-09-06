//! Emission quiescence is separate from original capture or output retirement.
use super::super::slots::OwnedReservation;
use super::*;

#[derive(Clone, Copy)]
struct PendingFence {
    command: Fence,
    input_cut: u64,
    lifetime_cut: u64,
    output_cut: u64,
    acknowledged: bool,
    inventory_acked: bool,
}

#[derive(Default)]
pub(super) struct Recovery {
    pending: Option<PendingFence>,
    completed: u64,
    minimum_emission: u64,
    pub(super) boundary: Option<i64>,
    cursor: usize,
    counting: bool,
    total: u32,
    emitted: u32,
    chunks: u32,
    writer: Option<OwnedReservation<InventoryChunk, 1>>,
    work: usize,
    cleanup: Option<usize>,
}

impl Recovery {
    pub(super) fn begin(&mut self) {
        self.work = 0;
    }
    #[cfg(all(test, not(feature = "tuning-probe")))]
    pub(super) fn active(&self) -> bool {
        self.pending.is_some()
    }
    pub(super) fn needs_ack(&self) -> bool {
        self.pending.is_some_and(|pending| !pending.acknowledged)
    }
    pub(super) fn holds(&self, serial: u64) -> bool {
        self.pending.is_some_and(|pending| serial <= pending.input_cut)
    }
    pub(super) fn holds_life(&self, serial: u64) -> bool {
        self.pending.is_some_and(|pending| serial <= pending.lifetime_cut)
    }
    pub(super) fn settled(&self) -> bool {
        self.pending.is_none() && self.cleanup.is_none()
    }
    pub(super) fn can_observe(&self, fence: Fence) -> bool {
        // A delayed completed command is consumed as stale. It cannot occupy
        // the one repair cell in front of the live transaction's ACK.
        fence.transaction <= self.completed
            || self.pending.is_none_or(|pending| {
                pending.command == fence || fence.transaction < pending.command.transaction
            })
    }
    pub(super) fn inhibits(&self, assignment: Assignment) -> bool {
        self.pending.is_some_and(|pending| {
            assignment.decision == 0 || assignment.decision >= pending.command.from_decision
        })
    }
    pub(super) fn rejects(&self, assignment: Assignment) -> bool {
        assignment.emission < self.minimum_emission
            || self.pending.is_some_and(|pending| {
                assignment.decision >= pending.command.from_decision
                    && assignment.emission <= pending.command.generation
            })
    }
}

impl Source {
    #[cfg(all(test, not(feature = "tuning-probe")))]
    pub(in crate::performance) fn test_recovery_state(&self) -> (bool, u32, u32, usize) {
        (self.recovery.active(), self.recovery.total, self.recovery.chunks, self.recovery.work)
    }
    pub(super) fn observe_fence(&mut self, command: Fence) {
        if self.capture_lease() != Some(command.lease)
            || self.epoch != command.epoch
            || command.transaction <= self.recovery.completed
            || command.transaction == 0
            || command.generation & GATE_FLAGS != 0
            || command.from_decision == 0
        {
            return;
        }
        if self.recovery.pending.is_some() {
            return;
        }
        let row = &self.offer.as_ref().unwrap().session.rows[usize::from(command.lease.slot - 1)];
        if row.emission_gate.load(Ordering::Acquire) & !BUSY != (command.generation | CLOSED) {
            return;
        }
        if command.terminal {
            // Reaffirm the exact terminal cut even if the original local fault
            // predated this command. This is once per fence, never per retry.
            self.cancel_unsounded();
            self.arm_release_debt();
        }
        self.recovery.pending = Some(PendingFence {
            command,
            input_cut: self.next_event,
            lifetime_cut: self.next_lifetime,
            output_cut: 0,
            acknowledged: false,
            inventory_acked: false,
        });
        // Pin every original through this cut, including a completed header
        // with an unsounded child. Removal cannot invalidate either finite scan.
        self.recovery.cursor = 0;
        self.recovery.counting = true;
        self.recovery.total = 0;
        self.recovery.emitted = 0;
        self.recovery.chunks = 0;
        assert!(self.recovery.writer.is_none());
    }

    /// Only an enclosing completion or an established callback join calls this.
    /// Every staged group has unconditional completion, so attempt is a settled
    /// cut as well as a staging counter. No release/history ownership is freed.
    pub(super) fn publish_revoke_ack(&mut self) {
        let Some(pending) = self.recovery.pending.filter(|pending| !pending.acknowledged) else {
            return;
        };
        if self.permit.is_some() {
            return;
        }
        let Some(offer) = &self.offer else { return };
        let row = &offer.session.rows[usize::from(offer.lease.slot - 1)];
        if row.emission_gate.load(Ordering::Acquire) != (pending.command.generation | CLOSED) {
            return;
        }
        let Some(cell) = row.to_hub.reserve_repair() else { return };
        cell.publish(Control::RevokeAck {
            fence: pending.command,
            input_cut: pending.input_cut,
            output_cut: self.sequence,
            settled_attempt: self.attempt,
        });
        let pending = self.recovery.pending.as_mut().unwrap();
        pending.acknowledged = true;
        pending.output_cut = self.sequence;
        self.service_revision = self.service_revision.wrapping_add(1);
    }

    pub(super) fn publish_inventory(&mut self) {
        let Some(pending) = self
            .recovery
            .pending
            .filter(|pending| pending.acknowledged && !pending.inventory_acked)
        else {
            return;
        };
        // Terminal inventory is the cancellation-cut proof: every nonessential
        // pre-cut effect has its exact disposition ACK before classification.
        // Unaccepted physical releases retain their independent original owners.
        if pending.command.terminal && !self.local_cancel_cut_settled() {
            return;
        }
        // Requests outlive their original On captures. Enumerate the fixed
        // lifetime set independently, including a held/terminal accepted On
        // whose original capture was already retired before this fence.
        while self.recovery.counting && self.recovery.work < 256 {
            if self.recovery.cursor == LIFETIMES {
                self.recovery.counting = false;
                self.recovery.cursor = 0;
                break;
            }
            self.recovery.work += 1;
            self.service_revision = self.service_revision.wrapping_add(1);
            let life = self.lives.at(self.recovery.cursor as u16);
            self.recovery.cursor += 1;
            if life.is_some_and(|life| life.serial <= pending.lifetime_cut) {
                self.recovery.total += 1;
            }
        }
        if self.recovery.counting
            || self.recovery.emitted == self.recovery.total && self.recovery.chunks != 0
        {
            return;
        }
        if self.recovery.writer.is_none() {
            if self.recovery.work + 64 > 256 {
                return;
            }
            let Some(offer) = &self.offer else { return };
            let row = &offer.session.rows[usize::from(offer.lease.slot - 1)];
            let Some(mut writer) = row.inventory.reserve_owned() else { return };
            self.recovery.work += 64;
            self.service_revision = self.service_revision.wrapping_add(1);
            writer.initialize(InventoryChunk {
                fence: pending.command,
                arena: self.pending.arena_identity(),
                input_cut: pending.input_cut,
                lifetime_cut: pending.lifetime_cut,
                output_cut: pending.output_cut,
                sequence: self.recovery.chunks,
                total: self.recovery.total,
                first: self.recovery.emitted,
                count: 0,
                records: [None; 64],
            });
            self.recovery.writer = Some(writer);
        }
        while self.recovery.work < 256 {
            let count = self.recovery.writer.as_mut().unwrap().value_mut().count;
            if count == 64 || self.recovery.emitted + u32::from(count) == self.recovery.total {
                break;
            }
            assert!(self.recovery.cursor < LIFETIMES, "complete pinned lifetime inventory");
            let index = self.recovery.cursor as u16;
            self.recovery.work += 1;
            self.service_revision = self.service_revision.wrapping_add(1);
            self.recovery.cursor += 1;
            let Some(life) =
                self.lives.at(index).filter(|life| life.serial <= pending.lifetime_cut)
            else {
                continue;
            };
            let local = self.lives.local_mut(index).unwrap();
            let outcome = if local.flags & PARTIAL_ON != 0 {
                RequestOutcome::Partial
            } else if life.sounded {
                RequestOutcome::Accepted
            } else if life.canceled {
                RequestOutcome::Canceled
            } else {
                RequestOutcome::Retained
            };
            let chunk = self.recovery.writer.as_mut().unwrap().value_mut();
            chunk.records[usize::from(chunk.count)] = Some(RequestInventory {
                lifetime: life.serial,
                on_serial: life.on_serial,
                decision: life.assignment.decision,
                configuration: life.assignment.configuration,
                input: life.input,
                life: index,
                outcome,
            });
            chunk.count += 1;
        }
        let chunk = self.recovery.writer.as_mut().unwrap().value_mut();
        if chunk.count == 64
            || self.recovery.emitted + u32::from(chunk.count) == self.recovery.total
        {
            self.recovery.emitted += u32::from(chunk.count);
            self.recovery.chunks += 1;
            self.recovery.writer.take().unwrap().publish_initialized();
            self.service_revision = self.service_revision.wrapping_add(1);
        }
    }

    pub(super) fn acknowledge_inventory(
        &mut self,
        fence: Fence,
        input_cut: u64,
        total: u32,
        chunks: u32,
    ) {
        if self.recovery.pending.is_some_and(|pending| {
            pending.command == fence && pending.acknowledged && pending.input_cut == input_cut
        }) && !self.recovery.counting
            && self.recovery.writer.is_none()
            && self.recovery.emitted == total
            && self.recovery.total == total
            && self.recovery.chunks == chunks
            && chunks != 0
        {
            self.recovery.pending.as_mut().unwrap().inventory_acked = true;
        }
    }

    pub(super) fn complete_recovery(&mut self, command: Fence, generation: u64, boundary: i64) {
        if self.capture_lease() != Some(command.lease)
            || self.epoch != command.epoch
            || command.generation.checked_add(4) != Some(generation)
            || !self.recovery.pending.is_some_and(|pending| {
                pending.command == command && pending.acknowledged && pending.inventory_acked
            })
        {
            return;
        }
        let row = &self.offer.as_ref().unwrap().session.rows[usize::from(command.lease.slot - 1)];
        // A subsequent Hub invalidation may already have closed this generation
        // while the prior completion still owns the repair cell.
        if row.emission_gate.load(Ordering::Acquire) & !GATE_FLAGS != generation {
            return;
        }
        let boundary = if command.terminal {
            None
        } else {
            let Some(boundary) = boundary.checked_sub(self.clock.calibration.offset) else {
                self.fault(CLOCK_FAULT);
                return;
            };
            Some(boundary)
        };
        self.recovery.completed = command.transaction;
        self.recovery.minimum_emission = generation;
        self.recovery.boundary = boundary;
        self.recovery.pending = None;
        self.recovery.cleanup = Some(0);
        // The existing bounded pending scan revisits newly unpinned originals.
        self.pending_cursor = self.pending.front_position();
        if self.producer_joined {
            // Joined owners never call the live output scheduler that consumes
            // pending_cursor. Revisit the existing explicit cancellation cut
            // through its bounded off-audio scan so completed pinned Originals
            // can release their final Life references after the reader ends.
            self.cancel_unsounded_through(self.cancel_cut);
        }
        // Explicit local Stop/emergency cuts and their latches remain owned by
        // their existing settlement path; a Hub reopen cannot erase them.
    }

    pub(super) fn cleanup_recovery(&mut self) {
        while self.recovery.work < 256 {
            let Some(index) = self.recovery.cleanup else { break };
            self.recovery.work += 1;
            self.recovery.cleanup = (index + 1 < LIFETIMES).then_some(index + 1);
            self.recycle(index as u16);
            self.service_revision = self.service_revision.wrapping_add(1);
        }
    }
}

// The one inventory window and existing cancellation ACK window share the
// prepaid manifest allocation, including the future full 128-byte configuration.
const _: () = assert!(
    std::mem::size_of::<super::super::slots::Slots<InventoryChunk, 1>>()
    + 2 * std::mem::size_of::<usize>() // Arc allocation header
    + 64 * (128 - std::mem::size_of::<harmonigraph_core::configuration::ResolvedConfig>())
    + std::mem::size_of::<Option<Manifest>>() * 64
        <= 64 * 256
);
