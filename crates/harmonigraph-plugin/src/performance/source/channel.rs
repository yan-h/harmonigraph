//! A channel input owns one envelope and a separate captured reference list.
//! One physical CC acceptance can justify several explicitly linked terminal
//! facts, never several fake host attempts.
use super::*;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) struct Reference {
    pub index: u16,
    pub serial: u64,
}
impl Default for Reference {
    fn default() -> Self {
        Self { index: NONE, serial: 0 }
    }
}
#[derive(Clone, Copy, Default)]
pub(super) enum Role {
    #[default]
    Wire,
    Stop {
        previous: u16,
        next: u16,
    },
    Header {
        first_waiter: u16,
        last_waiter: u16,
        previous_header: u16,
        next_header: u16,
    },
}
#[derive(Clone, Copy, Default)]
pub(super) struct Cell {
    pub role: Role,
    pub dependency: Reference,
    pub next_waiter: Option<u16>,
    pub previous_waiter: Option<u16>,
}
#[derive(Default)]
pub(super) struct Channels {
    pub head: [Option<Reference>; 16],
    pub tail: [Option<Reference>; 16],
}

impl Source {
    pub(super) fn capture_channel(&mut self, head: usize) {
        let mut header = self.pending.at(head).unwrap();
        let channel = usize::from(header.event.channel_control().unwrap());
        let previous = self.channels.tail[channel].filter(|tail| self.reference_pending(*tail));
        header.channel.role = Role::Header {
            first_waiter: NONE,
            last_waiter: NONE,
            previous_header: previous.map_or(NONE, |reference| reference.index),
            next_header: NONE,
        };
        self.pending.set(head, header);
        self.link_channel(head);
        let reference = Reference { index: head as u16, serial: header.serial };
        if let Some(tail) = previous {
            let mut prior = self.pending.at(usize::from(tail.index)).unwrap();
            let Role::Header { ref mut next_header, .. } = prior.channel.role else {
                unreachable!()
            };
            *next_header = head as u16;
            self.pending.set(usize::from(tail.index), prior);
        } else {
            self.channels.head[channel] = Some(reference);
        }
        self.channels.tail[channel] = Some(reference);
    }
    fn reference_pending(&self, reference: Reference) -> bool {
        self.pending
            .at(usize::from(reference.index))
            .is_some_and(|pending| pending.serial == reference.serial)
    }
    pub(super) fn link_channel(&mut self, position: usize) {
        let mut pending = self.pending.at(position).unwrap();
        if pending.event.release() {
            return;
        } // established releases remain responsive
        let Some(channel) = pending.event.channel().map(usize::from) else {
            return;
        };
        let Some(reference) =
            self.channels.tail[channel].filter(|reference| self.reference_pending(*reference))
        else {
            return;
        };
        pending.channel.dependency = reference;
        let mut header = self.pending.at(usize::from(reference.index)).unwrap();
        let Role::Header { ref mut first_waiter, ref mut last_waiter, .. } = header.channel.role
        else {
            unreachable!()
        };
        pending.channel.previous_waiter = (*last_waiter != NONE).then_some(*last_waiter);
        if *last_waiter != NONE {
            let mut previous = self.pending.at(usize::from(*last_waiter)).unwrap();
            previous.channel.next_waiter = Some(position as u16);
            self.pending.set(usize::from(*last_waiter), previous);
        } else {
            *first_waiter = position as u16;
        }
        *last_waiter = position as u16;
        self.pending.set(usize::from(reference.index), header);
        self.pending.set(position, pending);
    }
    pub(super) fn channel_ready(&mut self, pending: Pending) -> bool {
        if self.reference_pending(pending.channel.dependency) {
            return false;
        }
        let Role::Header { .. } = pending.channel.role else {
            return true;
        };
        if !self.charge(usize::from(pending.work_count)) {
            return false;
        }
        let mut targets = pending.work_head;
        for _ in 0..64 {
            if targets == NONE {
                return true;
            }
            let target = self.work.at(targets);
            if target.phase & work::DONE == 0 {
                let life = self.lives[usize::from(target.life)].unwrap();
                assert_eq!(life.serial, target.serial);
                if !life.sounded && !life.canceled {
                    return false;
                }
            }
            targets = target.next;
        }
        targets == NONE
    }
    pub(super) fn channel_wire_bindings_available(&self, pending: Pending) -> bool {
        if pending.event.channel_termination() != Some(true) {
            return true;
        }
        let Role::Header { .. } = pending.channel.role else {
            return true;
        };
        self.owed_note_off.iter().filter(|index| **index == NONE).count()
            >= self.channel_terminal_count(pending)
    }
    fn channel_terminal_count(&self, pending: Pending) -> usize {
        let mut targets = pending.work_head;
        let mut required = 0;
        for _ in 0..64 {
            if targets == NONE {
                break;
            }
            let target = self.work.at(targets);
            if target.phase & work::DONE == 0
                && self.lives[usize::from(target.life)]
                    .is_some_and(|life| life.sounded && life.terminal.is_none())
            {
                required += 1;
            }
            targets = target.next;
        }
        required
    }
    pub(super) fn channel_report_cells(&self, pending: Pending) -> usize {
        match pending.channel.role {
            Role::Header { .. } if pending.event.channel_termination().is_some() => {
                1 + self.channel_terminal_count(pending)
            }
            _ => 1,
        }
    }
    pub(super) fn wake_channel(&mut self, channel: Option<u8>, output: &mut api::Output<'_>) {
        let Some(reference) = channel.and_then(|channel| self.channels.head[usize::from(channel)])
        else {
            return;
        };
        if !self.reference_pending(reference) || self.visits >= 2048 {
            return;
        }
        let callback = self.callback.unwrap();
        self.visits += 1;
        self.stage_pending(
            usize::from(reference.index),
            callback.steady_time,
            callback.steady_time.saturating_add(i64::from(callback.frames)),
            output,
        );
    }
    pub(super) fn wake_waiters(&mut self, mut waiter: u16, output: &mut api::Output<'_>) {
        let callback = self.callback.unwrap();
        while waiter != NONE && self.visits < 2048 {
            let Some(pending) = self.pending.at(usize::from(waiter)) else {
                break;
            };
            self.visits += 1;
            self.stage_pending(
                usize::from(waiter),
                callback.steady_time,
                callback.steady_time.saturating_add(i64::from(callback.frames)),
                output,
            );
            waiter = pending.channel.next_waiter.unwrap_or(NONE);
        }
    }
    pub(super) fn record_channel_terminals(
        &mut self,
        pending: Pending,
        wire_sequence: u64,
        actual: i64,
    ) {
        let Some(choke) = pending.event.channel_termination() else {
            return;
        };
        let Role::Header { .. } = pending.channel.role else {
            return;
        };
        let mut targets = pending.work_head;
        for _ in 0..64 {
            if targets == NONE {
                break;
            }
            let target = self.work.at(targets);
            if target.phase & work::DONE != 0 {
                targets = target.next;
                continue;
            }
            let life = self.lives[usize::from(target.life)].unwrap();
            assert_eq!(life.serial, target.serial);
            if life.sounded && life.terminal.is_none() {
                let terminal = if choke {
                    Event::terminate(life.id, life.channel, life.key)
                } else {
                    Event::note_off(life.id, life.channel, life.key, life.midi)
                };
                let active_slot = self.active.iter().position(|index| *index == target.life);
                let mut fact = self.record(terminal, target.life, pending.input, actual);
                fact.outcome = Outcome::ChannelTerminal {
                    wire_sequence,
                    controller: if choke { 120 } else { 123 },
                };
                self.journal
                    .push(fact)
                    .unwrap_or_else(|_| unreachable!("prepared whole channel outcome group"));
                if choke {
                    let current = self.lives[usize::from(target.life)].as_mut().unwrap();
                    let slot = self
                        .owed_note_off
                        .iter_mut()
                        .find(|index| **index == NONE)
                        .expect("prepared Note-Off binding capacity");
                    *slot = target.life;
                    current.refs += 1;
                    current.note_off_owed = true;
                    current.active = life.active;
                    if let Some(slot) = active_slot {
                        self.active[slot] = target.life;
                    }
                }
            }
            targets = target.next;
        }
    }
    pub(super) fn channel_done(&mut self, position: usize, pending: Pending) {
        if let Role::Stop { previous, next } = pending.channel.role {
            self.unlink_stop(previous, next);
            return;
        }
        // Unlink a canceled/accepted waiter before its envelope can be reused.
        if self.reference_pending(pending.channel.dependency) {
            let dependency = pending.channel.dependency;
            let mut header = self.pending.at(usize::from(dependency.index)).unwrap();
            let Role::Header { ref mut first_waiter, ref mut last_waiter, .. } =
                header.channel.role
            else {
                unreachable!()
            };
            if let Some(previous) = pending.channel.previous_waiter {
                let mut value = self.pending.at(usize::from(previous)).unwrap();
                value.channel.next_waiter = pending.channel.next_waiter;
                self.pending.set(usize::from(previous), value);
            } else {
                *first_waiter = pending.channel.next_waiter.unwrap_or(NONE);
            }
            if let Some(next) = pending.channel.next_waiter {
                let mut value = self.pending.at(usize::from(next)).unwrap();
                value.channel.previous_waiter = pending.channel.previous_waiter;
                self.pending.set(usize::from(next), value);
            } else {
                *last_waiter = pending.channel.previous_waiter.unwrap_or(NONE);
            }
            self.pending.set(usize::from(dependency.index), header);
        }
        let Role::Header { previous_header, next_header, .. } = pending.channel.role else {
            return;
        };
        let channel = usize::from(pending.event.channel_control().unwrap());
        if previous_header != NONE {
            let mut previous = self.pending.at(usize::from(previous_header)).unwrap();
            let Role::Header { ref mut next_header, .. } = previous.channel.role else {
                unreachable!()
            };
            *next_header = match pending.channel.role {
                Role::Header { next_header, .. } => next_header,
                _ => NONE,
            };
            self.pending.set(usize::from(previous_header), previous);
        } else {
            self.channels.head[channel] = self
                .pending
                .at(usize::from(next_header))
                .map(|next| Reference { index: next_header, serial: next.serial });
        }
        if next_header != NONE {
            let mut next = self.pending.at(usize::from(next_header)).unwrap();
            let Role::Header { ref mut previous_header, .. } = next.channel.role else {
                unreachable!()
            };
            *previous_header = match pending.channel.role {
                Role::Header { previous_header, .. } => previous_header,
                _ => NONE,
            };
            self.pending.set(usize::from(next_header), next);
        } else {
            self.channels.tail[channel] = self
                .pending
                .at(usize::from(previous_header))
                .map(|previous| Reference { index: previous_header, serial: previous.serial });
        }
        let _ = position;
    }
}
