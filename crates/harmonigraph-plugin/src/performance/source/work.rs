//! Internal lifetime work has its own capacity. One original input owns one
//! Pending slot, regardless of how many captured targets it addresses.
use super::NONE;

pub(super) const CAPACITY: usize = 32768;
pub(super) const TARGET: u8 = 0;
pub(super) const CHOKE: u8 = 1;
pub(super) const NOTE_OFF: u8 = 2;
pub(super) const CHANNEL: u8 = 3;
pub(super) const DONE: u8 = 1;
pub(super) const DISPOSITION: u8 = 2;

#[derive(Clone, Copy)]
#[repr(C)]
pub(super) struct Cell {
    pub serial: u64,
    pub life: u16,
    pub parent: u16,
    pub next: u16,
    pub operation: u8,
    pub phase: u8,
}
pub(super) struct Work {
    cells: Box<[Cell]>,
    free: u16,
    len: usize,
    pub high_water: usize,
}
impl Default for Work {
    fn default() -> Self {
        let cells = (0..CAPACITY)
            .map(|index| Cell {
                serial: 0,
                life: NONE,
                parent: NONE,
                next: if index + 1 == CAPACITY { NONE } else { (index + 1) as u16 },
                operation: TARGET,
                phase: 0,
            })
            .collect::<Vec<_>>()
            .into_boxed_slice();
        Self { cells, free: 0, len: 0, high_water: 0 }
    }
}
impl Work {
    #[cfg(all(test, not(feature = "tuning-probe")))]
    pub fn len(&self) -> usize {
        self.len
    }
    pub fn free(&self) -> usize {
        CAPACITY - self.len
    }
    pub fn at(&self, index: u16) -> Cell {
        let cell = self.cells[usize::from(index)];
        assert_ne!(cell.parent, NONE);
        cell
    }
    pub fn set(&mut self, index: u16, cell: Cell) {
        assert_ne!(self.cells[usize::from(index)].parent, NONE);
        self.cells[usize::from(index)] = cell;
    }
    pub fn push(&mut self, cell: Cell) -> u16 {
        assert_ne!(self.free, NONE, "whole reference group reserved at capture");
        let index = self.free;
        self.free = self.cells[usize::from(index)].next;
        self.cells[usize::from(index)] = cell;
        self.len += 1;
        self.high_water = self.high_water.max(self.len);
        index
    }
    pub fn remove(&mut self, index: u16) {
        let cell = &mut self.cells[usize::from(index)];
        assert_ne!(cell.parent, NONE);
        cell.parent = NONE;
        cell.next = self.free;
        self.free = index;
        self.len -= 1;
    }
}
const _: () = assert!(std::mem::size_of::<Cell>() == 16);
const _: () = assert!(std::mem::align_of::<Cell>() <= 8);

use super::*;
impl Source {
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
        let value = self.lives[usize::from(life)].as_mut().unwrap();
        value.refs += 1;
        let serial = value.serial;
        let generation = value.generation;
        let mut pending = self.pending.at(position).unwrap();
        if operation == CHANNEL && pending.event.channel_termination() == Some(true) {
            value.sound_off_refs += 1;
        }
        let index = self.work.push(Cell {
            serial,
            life,
            parent: position as u16,
            next: NONE,
            operation,
            phase: 0,
        });
        if pending.work_head == NONE {
            pending.work_head = index;
        } else {
            let mut tail = self.work.at(pending.work_tail);
            tail.next = index;
            self.work.set(pending.work_tail, tail);
        }
        pending.work_tail = index;
        pending.work_count += 1;
        pending.work_remaining += 1;
        self.pending.set(position, pending);
        if operation == CHOKE
            || operation == NOTE_OFF
            || operation == TARGET && pending.event.release()
        {
            self.lives[usize::from(life)].as_mut().unwrap().release =
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
            let life = self.lives[usize::from(cell.life)].unwrap();
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
            let life = self.lives[usize::from(pending.life)].unwrap();
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
                let life = self.lives[usize::from(parent.life)].as_mut().unwrap();
                life.refs -= 1;
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
            let life = self.lives[usize::from(cell.life)].as_mut().unwrap();
            assert_eq!(life.serial, cell.serial);
            let generation = life.generation;
            life.refs -= 1;
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
            self.recycle(cell.life);
        }
        self.service_revision = self.service_revision.wrapping_add(1);
        self.pending.set(position, parent);
        self.remove_finished(position);
    }
    pub(super) fn remove_finished(&mut self, position: usize) {
        let mut parent = self.pending.at(position).unwrap();
        if !parent.inline_done
            || parent.work_remaining != 0
            || parent.staged
            || parent.cleanup_queued
        {
            return;
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
        self.drain_finished();
    }
    pub(super) fn drain_finished(&mut self) {
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
            self.channel_done(position, parent);
            if self.cancel_cursor == Some(position) {
                self.cancel_cursor = self.pending.next_position(position);
            }
            let mut child = parent.work_head;
            while child != NONE {
                let cell = self.work.at(child);
                assert_eq!(cell.phase & DONE, DONE);
                self.work.remove(child);
                child = cell.next;
            }
            self.pending.remove(position).unwrap();
        }
    }
}
