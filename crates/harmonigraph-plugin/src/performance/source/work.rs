//! Internal lifetime work has its own capacity. One original input owns one
//! Pending slot, regardless of how many captured targets it addresses.
//!
//! This storage is Tune-local and plain. Nothing outside the owning Source
//! reads a cell, so there is no packed immutable region and no free-list
//! ownership protocol left to defend.
use super::NONE;

pub(in crate::performance) const CAPACITY: usize = 32768;
pub(super) const TARGET: u8 = 0;
pub(super) const CHOKE: u8 = 1;
pub(super) const NOTE_OFF: u8 = 2;
pub(super) const CHANNEL: u8 = 3;
pub(super) const DONE: u8 = 1;
pub(super) const DISPOSITION: u8 = 2;
const UNLINKED: u8 = 4;
const INLINE: u16 = CAPACITY as u16;

pub(super) fn ready_reference(position: usize, child: u16) -> u16 {
    if child == NONE {
        INLINE | position as u16
    } else {
        child
    }
}

pub(super) fn inline_position(reference: u16) -> Option<usize> {
    (reference != NONE && reference >= INLINE).then_some(usize::from(reference & !INLINE))
}

#[derive(Clone, Copy)]
#[repr(C)]
pub(super) struct Cell {
    pub serial: u64,
    pub life: u16,
    pub parent: u16,
    pub next: u16,
    pub ready_next: u16,
    pub operation: u8,
    pub phase: u8,
}
pub(in crate::performance) struct Work {
    cells: Box<[Option<Cell>]>,
    free: Vec<u16>,
    pub high_water: usize,
}
impl Default for Work {
    fn default() -> Self {
        Self {
            cells: (0..CAPACITY).map(|_| None).collect(),
            free: (0..CAPACITY as u16).rev().collect(),
            high_water: 0,
        }
    }
}
impl Work {
    #[cfg(test)]
    pub(super) fn len(&self) -> usize {
        CAPACITY - self.free.len()
    }
    pub(super) fn free(&self) -> usize {
        self.free.len()
    }
    pub(super) fn at(&self, index: u16) -> Cell {
        self.cells[index as usize].expect("live work reference")
    }
    pub(super) fn set(&mut self, index: u16, cell: Cell) {
        let old = self.cells[index as usize].expect("live work reference");
        assert_eq!(
            (old.serial, old.life, old.parent, old.next, old.operation),
            (cell.serial, cell.life, cell.parent, cell.next, cell.operation)
        );
        assert!(matches!(cell.phase, 0 | DISPOSITION | DONE | 5), "invalid Source Work phase");
        self.cells[index as usize] = Some(cell);
    }
    pub(super) fn link_building(&mut self, index: u16, next: u16) {
        self.cells[index as usize].as_mut().expect("building work reference").next = next;
    }
    pub(super) fn push(&mut self, cell: Cell) -> u16 {
        assert!(cell.operation < 4);
        let index = self.free.pop().expect("whole reference group reserved at capture");
        self.cells[index as usize] = Some(cell);
        self.high_water = self.high_water.max(CAPACITY - self.free.len());
        index
    }
    pub(super) fn remove(&mut self, index: u16) {
        assert!(self.cells[index as usize].take().is_some());
        self.free.push(index);
    }
    #[cfg(test)]
    pub(super) fn test_layout(&self) -> [usize; 3] {
        [
            std::mem::size_of::<Option<Cell>>(),
            CAPACITY,
            std::mem::size_of_val(&*self.cells) + self.free.capacity() * 2,
        ]
    }
}
const _: () = assert!(std::mem::size_of::<Option<Cell>>() <= 32);
const _: () = assert!(PENDING_EVENTS <= INLINE as usize && LIFETIMES <= CAPACITY);

use super::*;
impl Source {
    fn append_ready(&mut self, life: u16, reference: u16) {
        let value = self.lives.local_mut(life).unwrap();
        if value.ready_tail == NONE {
            value.ready_head = reference;
        } else if let Some(position) = inline_position(value.ready_tail) {
            let mut tail = self.pending.at(position).unwrap();
            tail.work_tail = reference;
            self.pending.set(position, tail);
        } else {
            let mut tail = self.work.at(value.ready_tail);
            tail.ready_next = reference;
            self.work.set(value.ready_tail, tail);
        }
        value.ready_tail = reference;
    }

    /// Single-target input already pins its life in the inline envelope. Its
    /// otherwise unused Work tail stores the ready link; no Work cell is needed.
    pub(super) fn capture_inline_ready(&mut self, position: usize, life: u16) {
        let mut pending = self.pending.at(position).unwrap();
        assert_eq!(pending.work_count, 0);
        pending.work_linked = 1;
        self.pending.set(position, pending);
        self.append_ready(life, ready_reference(position, NONE));
    }

    fn unlink_inline_ready(&mut self, position: usize) {
        let mut pending = self.pending.at(position).unwrap();
        let life = self.lives.local_mut(pending.life).unwrap();
        assert_eq!(life.ready_head, ready_reference(position, NONE));
        assert!(pending.inline_done && pending.work_linked == 1);
        life.ready_head = pending.work_tail;
        if life.ready_head == NONE {
            life.ready_tail = NONE;
        }
        // The original capture keeps its Birth pin through remote retirement.
        pending.work_linked = 0;
        pending.work_tail = NONE;
        self.pending.set(position, pending);
        self.recycle(pending.life);
    }
    pub(super) fn add_obligation(&mut self, generation: u64) {
        self.obligations += 1;
        if self.lease_generation().is_some_and(|lease| generation <= lease) {
            self.old_pending += 1;
        }
    }
    pub(super) fn settle_obligation(&mut self, generation: u64) {
        self.obligations -= 1;
        if self.lease_generation().is_some_and(|lease| generation <= lease) {
            self.old_pending -= 1;
        }
    }
    pub(super) fn capture_work(&mut self, position: usize, life: u16, operation: u8) {
        let birth = self.lives.at(life).unwrap();
        let serial = birth.serial;
        let generation = birth.generation;
        let value = self.lives.local_mut(life).unwrap();
        value.refs += 1;
        let mut pending = self.pending.at(position).unwrap();
        if operation == CHANNEL && pending.event.channel_termination() == Some(true) {
            value.sound_off_refs += 1;
        }
        let index = self.work.push(Cell {
            serial,
            life,
            parent: position as u16,
            next: NONE,
            ready_next: NONE,
            operation,
            phase: 0,
        });
        self.append_ready(life, index);
        if pending.work_head == NONE {
            pending.work_head = index;
        } else {
            self.work.link_building(pending.work_tail, index);
        }
        pending.work_tail = index;
        pending.work_count += 1;
        pending.work_remaining += 1;
        pending.work_linked += 1;
        self.pending.set(position, pending);
        if operation == CHOKE
            || operation == NOTE_OFF
            || operation == TARGET && pending.event.release()
        {
            self.lives.local_mut(life).unwrap().release =
                Some(ReleaseIndex { parent: position as u16, work: index });
        }
        self.add_obligation(generation);
    }
    /// Ephemeral resolved value; the exact input envelope is never rewritten.
    pub(super) fn resolved(&self, position: usize, child: u16) -> Pending {
        let mut pending = self.pending.at(position).unwrap();
        if child != NONE {
            let cell = self.work.at(child);
            assert_eq!(usize::from(cell.parent), position);
            assert_eq!(cell.phase & DONE, 0);
            let life = self.lives.at(cell.life).unwrap();
            assert_eq!(cell.serial, life.serial);
            pending.life = cell.life;
            pending.generation = life.generation;
            pending.event = match cell.operation {
                CHOKE => Event::terminate(life.id, life.channel, life.key),
                NOTE_OFF => Event::note_off(life.id, life.channel, life.key, life.midi),
                _ => pending.event.for_voice(life.id, life.channel, life.key),
            };
            pending.disposition = cell.phase & DISPOSITION != 0;
            pending.selected = child;
        } else if pending.life != NONE && pending.event.attack().is_none() {
            let life = self.lives.at(pending.life).unwrap();
            pending.event = pending.event.for_voice(life.id, life.channel, life.key);
        }
        pending
    }
    pub(super) fn finish_work(&mut self, position: usize, child: u16) {
        let mut parent = self.pending.at(position).unwrap();
        if child == NONE {
            if parent.inline_done {
                return;
            }
            parent.inline_done = true;
            parent.disposition = false;
            self.settle_obligation(parent.generation);
            if parent.life != NONE {
                let life = self.lives.local_mut(parent.life).unwrap();
                if life.canceled {
                    life.active = false;
                }
                if life.release.is_some_and(|release| {
                    release.parent as usize == position && release.work == NONE
                }) {
                    life.release = None;
                }
                self.recycle(parent.life);
            }
        } else {
            let mut cell = self.work.at(child);
            assert_eq!(usize::from(cell.parent), position);
            if cell.phase & DONE != 0 {
                return;
            }
            let birth = self.lives.at(cell.life).unwrap();
            assert_eq!(birth.serial, cell.serial);
            let generation = birth.generation;
            let life = self.lives.local_mut(cell.life).unwrap();
            if life.canceled {
                life.active = false;
            }
            if life
                .release
                .is_some_and(|release| release.parent as usize == position && release.work == child)
            {
                life.release = None;
            }
            if cell.operation == CHANNEL && parent.event.channel_termination() == Some(true) {
                life.sound_off_refs -= 1;
            }
            cell.phase = DONE;
            self.work.set(child, cell);
            parent.work_remaining -= 1;
            self.settle_obligation(generation);
        }
        self.service_revision = self.service_revision.wrapping_add(1);
        self.pending.set(position, parent);
        if child == NONE && parent.work_count == 0 && parent.work_linked != 0 {
            if self.lives.at(parent.life).unwrap().ready_head == ready_reference(position, NONE) {
                // The common accepted inline head has constant unlink work,
                // covered by its normal envelope cleanup visit.
                self.unlink_inline_ready(position);
                if self.lives.at(parent.life).is_some_and(|life| {
                    life.ready_head != NONE
                        && inline_position(life.ready_head).map_or_else(
                            || self.work.at(life.ready_head).phase & DONE != 0,
                            |position| self.pending.at(position).unwrap().inline_done,
                        )
                }) {
                    self.queue_ready_cleanup(parent.life);
                    self.drain_ready_work();
                }
            } else {
                self.queue_ready_cleanup(parent.life);
            }
        } else if child != NONE {
            self.queue_ready_cleanup(self.work.at(child).life);
            self.drain_ready_work();
        }
        if self.pending.at(position).is_some() {
            self.remove_finished(position);
        }
    }
    pub(super) fn remove_finished(&mut self, position: usize) {
        let mut parent = self.pending.at(position).unwrap();
        if !parent.inline_done
            || parent.work_remaining != 0
            || parent.work_linked != 0
            || parent.staged
            || parent.cleanup_queued
        {
            return;
        }
        if self.pending.local_done(position) {
            if self.capture_retained(position) {
                return;
            }
        } else if let Some(channel) = parent.event.channel_control().map(usize::from) {
            if self.channels.head[channel].is_some_and(|head| usize::from(head.index) != position)
                && (parent.channel.accepted || parent.serial > self.cancel_cut)
            {
                return;
            }
        }
        parent.cleanup_queued = true;
        if self.cleanup_tail != NONE {
            let mut tail = self.pending.at(usize::from(self.cleanup_tail)).unwrap();
            tail.cleanup_next = position as u16;
            self.pending.set(usize::from(self.cleanup_tail), tail);
        } else {
            self.cleanup_head = position as u16;
        }
        self.cleanup_tail = position as u16;
        self.pending.set(position, parent);
        if !self.draining_finished {
            self.drain_finished();
        }
    }
    pub(super) fn drain_finished(&mut self) {
        self.draining_finished = true;
        while self.cleanup_head != NONE {
            let position = usize::from(self.cleanup_head);
            let parent = self.pending.at(position).unwrap();
            if !self.charge(1 + usize::from(parent.work_count)) {
                break;
            }
            self.cleanup_head = parent.cleanup_next;
            if self.cleanup_head == NONE {
                self.cleanup_tail = NONE;
            }
            if !self.pending.local_done(position) {
                self.channel_done(position, parent);
                self.pending.finish_local(position);
            }
            let mut updated = self.pending.at(position).unwrap();
            updated.cleanup_queued = false;
            updated.cleanup_next = NONE;
            self.pending.set(position, updated);
            if let Some(channel) = parent.event.channel().map(usize::from) {
                if let Some(head) = self.channels.head[channel] {
                    if head.index as usize != position {
                        self.remove_finished(head.index as usize);
                    }
                }
            }
            if self.capture_retained(position) {
                continue;
            }
            if self.cancel_cursor == Some(position) {
                self.cancel_cursor = self.pending.next_position(position);
            }
            if self.pending_cursor == Some(position) {
                self.pending_cursor = self.pending.next_position(position);
            }
            if self.capture_cursor == Some(position) {
                self.capture_cursor = self.pending.next_position(position);
            }
            if self.settlement_cursor == Some(position) {
                self.settlement_cursor = self.pending.next_position(position);
            }
            let mut child = parent.work_head;
            while child != NONE {
                let cell = self.work.at(child);
                assert_eq!(cell.phase & DONE, DONE);
                self.work.remove(child);
                self.lives.local_mut(cell.life).unwrap().refs -= 1;
                self.recycle(cell.life);
                child = cell.next;
            }
            if parent.life != NONE {
                self.lives.local_mut(parent.life).unwrap().refs -= 1;
                self.recycle(parent.life);
            }
            self.pending.remove(position).unwrap();
            if let Some(channel) = parent.event.channel().map(usize::from) {
                if let Some(head) = self.channels.head[channel] {
                    self.remove_finished(usize::from(head.index));
                }
            }
        }
        self.draining_finished = false;
    }

    /// Retain only an input whose copy has not reached the Hub yet: destroying
    /// it here would lose an input the Hub is owed. Once the copy is sent
    /// nothing remote refers to this envelope, so there is no retirement to
    /// wait for. After producer join, or for a cancellation that precedes the
    /// first adoption, no copy was sent and none will be — do not publish that
    /// completed Original later as a fresh onset with no cancellation.
    fn capture_retained(&self, position: usize) -> bool {
        let serial = self.pending.at(position).unwrap().serial;
        !self.pending.published(position)
            && self.session().is_some()
            && !self.producer_joined
            && (self.direct.is_some() || self.adopt_sent || serial > self.cancel_cut)
    }

    fn queue_ready_cleanup(&mut self, index: u16) {
        let life = self.lives.local_mut(index).unwrap();
        if life.ready_queued {
            return;
        }
        life.ready_queued = true;
        if self.work_cleanup_tail == NONE {
            self.work_cleanup_head = index;
        } else {
            self.lives.local_mut(self.work_cleanup_tail).unwrap().cleanup_next = index;
        }
        self.work_cleanup_tail = index;
    }

    pub(super) fn drain_ready_work(&mut self) {
        while self.work_cleanup_head != NONE {
            if !self.charge(1) {
                return;
            }
            let index = self.work_cleanup_head;
            let life = self.lives.at(index).unwrap();
            if let Some(position) = inline_position(life.ready_head) {
                if self.pending.at(position).unwrap().inline_done {
                    self.unlink_inline_ready(position);
                    self.remove_finished(position);
                    continue;
                }
            }
            if life.ready_head == NONE
                || inline_position(life.ready_head).is_some()
                || self.work.at(life.ready_head).phase & DONE == 0
            {
                self.work_cleanup_head = life.cleanup_next;
                if self.work_cleanup_head == NONE {
                    self.work_cleanup_tail = NONE;
                }
                let value = self.lives.local_mut(index).unwrap();
                value.ready_queued = false;
                value.cleanup_next = NONE;
                self.recycle(index);
                continue;
            }
            let mut cell = self.work.at(life.ready_head);
            assert_eq!(cell.serial, life.serial);
            cell.phase |= UNLINKED;
            self.work.set(life.ready_head, cell);
            let value = self.lives.local_mut(index).unwrap();
            value.ready_head = cell.ready_next;
            if value.ready_head == NONE {
                value.ready_tail = NONE;
            }
            // Birth remains pinned by the original parent until both owners retire.
            let mut parent = self.pending.at(usize::from(cell.parent)).unwrap();
            parent.work_linked -= 1;
            self.pending.set(usize::from(cell.parent), parent);
            self.remove_finished(usize::from(cell.parent));
        }
    }
}
