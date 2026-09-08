//! One channel's ordered onset list and its live wire cursor. Controller
//! history is no longer retained here: a late note meets the receiver's
//! current controller state, and shared channel controls keep their own
//! input+D schedule.
use super::*;

pub(super) struct Wave {
    pub first: u16,
    pub last: u16,
    pub wire: channel::Reference,
    pub delivered: u64,
}

impl Default for Wave {
    fn default() -> Self {
        Self { first: NONE, last: NONE, wire: channel::Reference::default(), delivered: 0 }
    }
}

impl Source {
    pub(super) fn pending_reference(&self, index: u16) -> channel::Reference {
        self.pending.at(usize::from(index)).map_or_else(channel::Reference::default, |value| {
            channel::Reference { index, serial: value.serial }
        })
    }

    pub(super) fn capture_onset(&mut self, position: usize) {
        let mut pending = self.pending.at(position).unwrap();
        let channel = usize::from(pending.event.channel().unwrap());
        let wave = &mut self.channels.waves[channel];
        pending.channel.role = channel::Role::Onset { previous: wave.last, next: NONE };
        if wave.last != NONE {
            let mut previous = self.pending.at(usize::from(wave.last)).unwrap();
            let channel::Role::Onset { ref mut next, .. } = previous.channel.role else {
                unreachable!()
            };
            *next = position as u16;
            self.pending.set(usize::from(wave.last), previous);
        } else {
            wave.first = position as u16;
        }
        wave.last = position as u16;
        self.pending.set(position, pending);
    }

    pub(super) fn unlink_onset(&mut self, pending: Pending, previous: u16, next: u16) {
        let channel = usize::from(pending.event.channel().unwrap());
        if previous == NONE {
            self.channels.waves[channel].first = next;
        } else {
            let mut value = self.pending.at(usize::from(previous)).unwrap();
            let channel::Role::Onset { next: ref mut link, .. } = value.channel.role else {
                unreachable!()
            };
            *link = next;
            self.pending.set(usize::from(previous), value);
        }
        if next == NONE {
            self.channels.waves[channel].last = previous;
        } else {
            let mut value = self.pending.at(usize::from(next)).unwrap();
            let channel::Role::Onset { previous: ref mut link, .. } = value.channel.role else {
                unreachable!()
            };
            *link = previous;
            self.pending.set(usize::from(next), value);
        }
    }

    pub(super) fn accept_channel_wire(&mut self, position: usize, pending: Pending) {
        let channel = usize::from(pending.event.channel_control().unwrap());
        let channel::Role::Header { next_header, .. } = pending.channel.role else {
            unreachable!()
        };
        let next = self.pending_reference(next_header);
        let wave = &mut self.channels.waves[channel];
        wave.delivered = pending.serial;
        wave.wire = next;
        let mut parent = self.pending.at(position).unwrap();
        parent.channel.accepted = true;
        self.pending.set(position, parent);
    }

    pub(super) fn schedule_channels(&mut self, start: i64, end: i64, output: &mut api::Output<'_>) {
        for channel in 0..16 {
            if !self.charge(1) {
                break;
            }
            let reference = self.channels.waves[channel].wire;
            if reference.index != NONE {
                self.stage_pending(usize::from(reference.index), start, end, output);
            }
            self.wake_onset(Some(channel as u8), start, output);
        }
    }

    /// The supplementary out-of-order wake, and the only thing its refusal
    /// below still decides is which ready event gets the scarce remaining
    /// output allowance of THIS callback: `schedule_pending` steps over an
    /// attack without an assignment and over an event addressed to a note
    /// that has not sounded, so it reaches this onset on its own and rule one
    /// no longer depends on the wake happening here.
    pub(super) fn wake_onset(
        &mut self,
        channel: Option<u8>,
        start: i64,
        output: &mut api::Output<'_>,
    ) {
        let Some(channel) = channel else {
            return;
        };
        let first = self.channels.waves[usize::from(channel)].first;
        if first != NONE
            && self.pending_cursor.and_then(|position| self.pending.at(position)).is_some_and(
                |earlier| {
                    earlier.serial < self.pending.at(usize::from(first)).unwrap().serial
                        && earlier.event.attack().is_none()
                        && earlier.event.channel_control().is_none()
                        && !earlier.event.marker()
                        && (!earlier.inline_done || earlier.work_remaining != 0)
                },
            )
        {
            return;
        }
        if first != NONE && self.charge(1) {
            let callback = self.callback.unwrap();
            self.stage_pending(
                usize::from(first),
                start,
                callback.steady_time + i64::from(callback.frames),
                output,
            );
        }
    }

    pub(super) fn wake_life(&mut self, index: u16, start: i64, output: &mut api::Output<'_>) {
        let Some(life) = self.lives.at(index) else {
            return;
        };
        if !life.sounded || life.ready_head == NONE || !self.charge(1) {
            return;
        }
        let callback = self.callback.unwrap();
        let end = callback.steady_time + i64::from(callback.frames);
        if let Some(position) = work::inline_position(life.ready_head) {
            self.stage_work(position, NONE, start, end, output);
            return;
        }
        let cell = self.work.at(life.ready_head);
        assert_eq!(cell.serial, life.serial);
        if cell.phase != 0 {
            return;
        }
        if cell.operation == work::CHANNEL {
            self.stage_pending(usize::from(cell.parent), start, end, output);
        } else {
            self.stage_work(usize::from(cell.parent), life.ready_head, start, end, output);
        }
    }

    pub(super) fn schedule_ready(&mut self, start: i64, _end: i64, output: &mut api::Output<'_>) {
        self.drain_ready_work();
        for index in self.reserved {
            if index != NONE {
                self.wake_life(index, start, output);
            }
        }
    }
}
