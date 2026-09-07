//! Input capture precedes output sub-blocks. A Stop therefore retains its own
//! bounded marker until output reaches that sample, rather than terminating
//! already-held voices while earlier events are still waiting to be scheduled.
use super::*;

pub(super) struct Stops {
    head: u16,
    tail: u16,
    pub emergency_start: u32,
    pub reached: u64,
}
impl Default for Stops {
    fn default() -> Self {
        Self { head: NONE, tail: NONE, emergency_start: 0, reached: 0 }
    }
}
impl Source {
    pub(super) fn capture_participation(&mut self, value: bool, sample: Option<i64>) -> bool {
        if value == self.participating {
            return true;
        }
        // Activation may seed the setting before a callback clock exists.
        // Runtime edits reserve their Original before advancing the local flag.
        if let Some(sample) = sample {
            if self.pending.free() == 0 || self.next_event == u64::MAX {
                self.fault(STORAGE_FAULT);
                return false;
            }
            if !self.charge(1) {
                return false;
            }
            let position = self.enqueue_cell(Event::Participation(value), NONE, sample, false);
            self.pending.seal(position);
            self.finish_work(position, NONE);
        }
        self.participating = value;
        self.baseline_needed = true;
        true
    }

    pub(super) fn capture_stop(&mut self, sample: i64) -> api::Consumption {
        if self.pending.free() == 0 || self.next_event == u64::MAX {
            self.fault(STORAGE_FAULT);
            return api::Consumption::Pending;
        }
        if !self.charge(64) {
            return api::Consumption::Pending;
        }
        let position = self.enqueue_cell(Event::Stop, NONE, sample, false);
        let mut pending = self.pending.at(position).unwrap();
        pending.channel.role =
            channel::Role::Stop { previous: self.stops.tail, next: NONE };
        self.pending.set(position, pending);
        if self.stops.tail == NONE {
            self.stops.head = position as u16;
        } else {
            let mut tail = self.pending.at(usize::from(self.stops.tail)).unwrap();
            let channel::Role::Stop { ref mut next, .. } = tail.channel.role else {
                unreachable!()
            };
            *next = position as u16;
            self.pending.set(usize::from(self.stops.tail), tail);
        }
        self.stops.tail = position as u16;
        // Later original input cannot address a lifetime from before Stop.
        // This changes input bindings, not actual sounding state or output debt.
        for index in &mut self.active {
            if *index != NONE {
                self.lives.local_mut(*index).unwrap().active = false;
                *index = NONE;
            }
        }
        self.pending.seal(position);
        api::Consumption::Consumed
    }

    pub(super) fn next_stop_sample(&self) -> Option<i64> {
        self.pending.at(usize::from(self.stops.head)).map(|pending| pending.input)
    }

    pub(super) fn advance_stops(&mut self, offset: u32) {
        let callback = self.callback.unwrap();
        let Some(sample) = callback.steady_time.checked_add(i64::from(offset)) else {
            return;
        };
        while let Some(pending) = self.pending.at(usize::from(self.stops.head)) {
            if pending.input > sample
                || pending.disposition
                || pending.inline_done
                || !self.charge(192)
            {
                break;
            }
            let position = usize::from(self.stops.head);
            self.stops.emergency_start = self.stops.emergency_start.max(offset);
            self.cancel_unsounded_through(pending.serial);
            self.stops.reached = pending.serial;
            let channel::Role::Stop { previous, next } = pending.channel.role else {
                unreachable!()
            };
            self.unlink_stop(previous, next);
            let mut reached = self.pending.at(position).unwrap();
            reached.channel.role = channel::Role::ReachedStop;
            self.pending.set(position, reached);
            self.arm_release_debt();
            self.finish_work(position, NONE);
            self.cancel_slice();
        }
    }

    pub(super) fn unlink_stop(&mut self, previous: u16, next: u16) {
        if previous == NONE {
            self.stops.head = next;
        } else {
            let mut value = self.pending.at(usize::from(previous)).unwrap();
            let channel::Role::Stop { next: ref mut link, .. } = value.channel.role else {
                unreachable!()
            };
            *link = next;
            self.pending.set(usize::from(previous), value);
        }
        if next == NONE {
            self.stops.tail = previous;
        } else {
            let mut value = self.pending.at(usize::from(next)).unwrap();
            let channel::Role::Stop { previous: ref mut link, .. } = value.channel.role else {
                unreachable!()
            };
            *link = previous;
            self.pending.set(usize::from(next), value);
        }
    }
}
