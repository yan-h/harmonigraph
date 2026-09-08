//! Input capture precedes output sub-blocks. A Stop therefore retains its own
//! bounded marker until output reaches that sample, rather than terminating
//! already-held voices while earlier events are still waiting to be scheduled.
use super::*;

pub(super) struct Stops {
    head: u16,
    tail: u16,
    pub emergency_start: u32,
}
impl Default for Stops {
    fn default() -> Self {
        Self { head: NONE, tail: NONE, emergency_start: 0 }
    }
}
impl Source {
    /// A participation toggle is a reset of this Tune in either direction, and
    /// it takes the same boundary a transport Stop does: the marker stands in
    /// the input queue at the toggle's own sample, and the reset runs where
    /// output reaches it. Everything captured before it is cancelled or
    /// terminated there; everything captured after it is already in the new
    /// mode, because `participating` moves here, in input order.
    ///
    /// Deferring the reset to the marker is what keeps it off the input path:
    /// no allocation, no wait, and the emergency releases land at the toggle's
    /// own offset instead of at the head of whatever callback carried it.
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
            self.capture_marker(Event::Participation(value), sample);
        }
        self.participating = value;
        true
    }

    pub(super) fn capture_stop(&mut self, sample: i64) {
        if self.pending.free() == 0 || self.next_event == u64::MAX {
            self.fault(STORAGE_FAULT);
            return;
        }
        self.capture_marker(Event::Stop, sample);
    }

    /// The caller has already proved the queue has room for this cell.
    fn capture_marker(&mut self, event: Event, sample: i64) {
        let position = self.enqueue_cell(event, NONE, sample, false);
        let mut pending = self.pending.at(position).unwrap();
        pending.channel.role = channel::Role::Stop { previous: self.stops.tail, next: NONE };
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
        // Later original input cannot address a lifetime from before the
        // marker. This changes input bindings, not actual sounding state or
        // output debt.
        for index in &mut self.active {
            if *index != NONE {
                self.lives.local_mut(*index).unwrap().active = false;
                *index = NONE;
            }
        }
        self.pending.seal(position);
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
            self.retire_marker(pending);
            let mut reached = self.pending.at(position).unwrap();
            reached.channel.role = channel::Role::ReachedStop;
            self.pending.set(position, reached);
            self.arm_release_debt();
            self.finish_work(position, NONE);
            self.cancel_slice();
        }
    }

    /// A marker cell leaves the queue exactly two ways: output reaches it, or
    /// a stronger reset disposes it where it stands. Both come through here,
    /// because the pitch obligation a participation toggle carries belongs to
    /// the toggle and not to whichever of the two wins.
    ///
    /// The losing case is real: `apply_setup` captures the marker for a
    /// restored participation value and, when that same restore also changes
    /// the routing, immediately calls `stop()`, whose cut covers the marker's
    /// own serial. The local mode has already moved, so nothing else would
    /// ever recentre the wire and every adaptive note afterwards would sound
    /// at the bend the Off phrase left behind.
    pub(super) fn retire_marker(&mut self, pending: Pending) {
        let channel::Role::Stop { previous, next } = pending.channel.role else { unreachable!() };
        self.unlink_stop(previous, next);
        if matches!(pending.event, Event::Participation(_)) {
            self.arm_pitch_center();
        }
    }

    /// A participation toggle output has not reached. Whichever way its cell
    /// leaves the queue, `retire_marker` will owe the wire a recentre for it,
    /// so a teardown that can no longer send one has to account for it before
    /// it publishes what it left behind.
    pub(super) fn participation_marker_queued(&self) -> bool {
        let mut index = self.stops.head;
        while let Some(pending) = self.pending.at(usize::from(index)) {
            if matches!(pending.event, Event::Participation(_)) {
                return true;
            }
            let channel::Role::Stop { next, .. } = pending.channel.role else { unreachable!() };
            index = next;
        }
        false
    }

    #[cfg(test)]
    pub(in crate::performance) fn test_marker_queued(&self) -> bool {
        self.stops.head != NONE
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
