//! One accepted translation per channel. Input setup and retained raw history
//! have different owners from State's actual receiver facts.
use super::*;
use harmonigraph_core::canonical::ChannelBaseline;

pub(super) const SETUP_TOKEN: u64 = u64::MAX;
const REGISTERS: u16 = 130;

/// A deferred association owns one channel pin on an exact retained Stop cell.
/// Full Pending serial identity remains fixed while that pin prevents reuse.
#[derive(Clone, Copy)]
pub(super) struct Prefix(u16);
impl Default for Prefix {
    fn default() -> Self {
        Self(128)
    }
}
impl Prefix {
    pub fn known(value: u8) -> Self {
        Self(u16::from(value))
    }
    pub fn stop(index: u16) -> Self {
        Self(256 + index)
    }
    pub fn stop_index(self) -> Option<usize> {
        (self.0 >= 256).then(|| usize::from(self.0 - 256))
    }
}

pub(super) struct Wave {
    pub shift: Option<i64>,
    pub first: u16,
    pub last: u16,
    pub wire: channel::Reference,
    pub delivered: u64,
    pub base: ChannelBaseline,
    /// Generic NRPN arithmetic is receiver-defined. Keep the original prelude,
    /// including intervening bank/program/controller order, in Pending instead.
    pub prelude: u64,
    pub setup: channel::Reference,
    setup_cursor: u16,
    setup_next: channel::Reference,
    setup_attempt: u64,
    setup_staged: bool,
    reset_cut: u64,
    neutralized: u8,
    input_prefix: Prefix,
    prefix_cut: u64,
    repaired_stop: u64,
}

impl Default for Wave {
    fn default() -> Self {
        Self {
            shift: None,
            first: NONE,
            last: NONE,
            wire: channel::Reference::default(),
            delivered: 0,
            base: ChannelBaseline::default(),
            prelude: 0,
            setup: channel::Reference::default(),
            setup_cursor: 0,
            setup_next: channel::Reference::default(),
            setup_attempt: 0,
            setup_staged: false,
            reset_cut: 0,
            neutralized: 0,
            input_prefix: Prefix::default(),
            prefix_cut: 0,
            repaired_stop: 0,
        }
    }
}

pub(super) fn transaction(event: Event) -> bool {
    matches!(event, Event::Midi { data: [status, cc, _], .. }
        if status & 0xf0 == 0xc0 || status & 0xf0 == 0xb0
            && matches!(cc, 0 | 6 | 32 | 38 | 96..=101 | 121..=122 | 124..=127))
}

impl Source {
    pub(super) fn capture_velocity_prefix(&mut self, position: usize) {
        let mut pending = self.pending.at(position).unwrap();
        let Event::Midi { data: [status, a, b], .. } = pending.event else { return };
        let channel = usize::from(status & 15);
        if status & 0xf0 == 0xb0 && a == 88 {
            self.drop_prefix(self.channels.waves[channel].input_prefix, channel);
        }
        let wave = &mut self.channels.waves[channel];
        match status & 0xf0 {
            0x80 | 0x90 => {
                pending.channel.velocity_prefix = wave.input_prefix;
                wave.input_prefix = Prefix::known(0);
                wave.prefix_cut = pending.serial;
                self.pending.set(position, pending);
            }
            0xb0 if a == 88 => {
                wave.input_prefix = Prefix::known(b);
                wave.prefix_cut = pending.serial;
            }
            _ => {}
        }
    }

    pub(super) fn prefix_reconciliation(&self, pending: Pending) -> Option<Event> {
        let value = self.prefix_value(
            pending.channel.velocity_prefix,
            usize::from(pending.event.channel()?),
        )??;
        let Event::Midi { port, data: [status, _, _], .. } = pending.event else { return None };
        if !matches!(status & 0xf0, 0x80 | 0x90) {
            return None;
        }
        let channel = usize::from(status & 15);
        let state = &self.state.channels()[channel];
        let wire = self.channels.waves[channel].wire;
        let earlier_wire = wire.index != NONE && wire.serial < pending.serial;
        (earlier_wire
            || state.controller_valid[1] & (1 << 24) == 0
            || state.controllers[88] != value)
            .then_some(Event::Midi { port, data: [0xb0 | channel as u8, 88, value], flags: 0 })
    }

    // Outer None means the original Stop boundary/repair is still pending;
    // inner None means its actual receiver prefix was unknown.
    fn prefix_value(&self, prefix: Prefix, channel: usize) -> Option<Option<u8>> {
        if let Some(index) = prefix.stop_index() {
            let stop = self.pending.at(index).expect("owned Stop association");
            let channel::Role::ReachedStop { known, waiting, owners } = stop.channel.role else {
                return None;
            };
            assert_ne!(owners & (1 << channel), 0);
            if waiting & (1 << channel) != 0
                && self.channels.waves[channel].repaired_stop < stop.serial
            {
                return None;
            }
            Some((known & (1 << channel) != 0).then_some(0))
        } else {
            Some((prefix.0 < 128).then_some(prefix.0 as u8))
        }
    }

    pub(super) fn prefix_ready(&self, pending: Pending) -> bool {
        pending.event.channel().is_none_or(|channel| {
            self.prefix_value(pending.channel.velocity_prefix, usize::from(channel)).is_some()
        })
    }

    pub(super) fn drop_prefix(&mut self, prefix: Prefix, channel: usize) {
        let Some(index) = prefix.stop_index() else { return };
        let mut stop = self.pending.at(index).expect("owned Stop association");
        let (channel::Role::Stop { ref mut owners, .. }
        | channel::Role::ReachedStop { ref mut owners, .. }) = stop.channel.role
        else {
            unreachable!()
        };
        assert_ne!(*owners & (1 << channel), 0);
        *owners &= !(1 << channel);
        self.pending.set(index, stop);
        self.remove_finished(index);
    }

    pub(super) fn settle_wave_prefix(&mut self, channel: usize) {
        let prefix = self.channels.waves[channel].input_prefix;
        if prefix.stop_index().is_none() {
            return;
        }
        if let Some(value) = self.prefix_value(prefix, channel) {
            self.channels.waves[channel].input_prefix =
                value.map_or_else(Prefix::default, Prefix::known);
            self.drop_prefix(prefix, channel);
        }
    }

    pub(super) fn capture_stop_prefixes(&mut self, position: usize) {
        let serial = self.pending.at(position).unwrap().serial;
        for channel in 0..16 {
            self.drop_prefix(self.channels.waves[channel].input_prefix, channel);
            let wave = &mut self.channels.waves[channel];
            wave.input_prefix = Prefix::stop(position as u16);
            wave.prefix_cut = serial;
        }
    }

    pub(super) fn discard_wave_prefix_pins(&mut self) {
        for channel in 0..16 {
            let prefix = self.channels.waves[channel].input_prefix;
            let actual = &self.state.channels()[channel];
            // A reached cancellation discards every old input association.
            // Known nonzero receiver state still owns actual repair debt;
            // a consumer captured while that repair rejects must bind its
            // required neutral result, not resurrect the old receiver value.
            // State itself changes only when the host accepts output.
            self.channels.waves[channel].input_prefix =
                if actual.controller_valid[1] & (1 << 24) != 0 {
                    Prefix::known(0)
                } else {
                    Prefix::default()
                };
            self.drop_prefix(prefix, channel);
        }
    }

    pub(super) fn established_channel(&self, channel: usize) -> bool {
        self.state.voices().any(|voice| usize::from(voice.channel) == channel)
            || [64, 66, 69]
                .into_iter()
                .any(|cc| self.state.channels()[channel].controllers[cc] >= 64)
            || self.owed_note_off.into_iter().any(|index| {
                index != NONE && usize::from(self.lives.at(index).unwrap().channel) == channel
            })
    }
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

    pub(super) fn channel_history_needed(&self, pending: Pending) -> bool {
        let Some(channel) = pending.event.channel_control().map(usize::from) else {
            return false;
        };
        if self.producer_joined || pending.serial <= self.cancel_cut && !pending.channel.accepted {
            return false;
        }
        let wave = &self.channels.waves[channel];
        wave.setup.index != NONE
            || wave.prelude != 0 && pending.serial >= wave.prelude
            || wave.wire.index != NONE && pending.serial >= wave.wire.serial
            || self
                .pending
                .at(usize::from(wave.first))
                .is_some_and(|onset| onset.serial < pending.serial)
    }

    pub(super) fn fold_channel_header(&mut self, pending: Pending) {
        if !pending.channel.accepted {
            return;
        }
        let Event::Midi { data: [status, a, b], .. } = pending.event else { return };
        let wave = &mut self.channels.waves[usize::from(status & 15)];
        let b = if status & 0xf0 == 0xb0
            && pending.serial <= wave.reset_cut
            && [64, 66, 69]
                .iter()
                .position(|cc| *cc == a)
                .is_some_and(|bit| wave.neutralized & (1 << bit) != 0)
        {
            0
        } else {
            b
        };
        let base = &mut wave.base;
        match status & 0xf0 {
            0xb0 if !matches!(a, 88 | 120 | 123) => {
                base.controllers[usize::from(a)] = b;
                base.controller_valid[usize::from(a / 64)] |= 1 << (a % 64);
            }
            0xd0 => base.pressure = Some(a),
            0xe0 => base.pitch_bend = Some(u16::from(a) | u16::from(b) << 7),
            _ => {}
        }
    }

    pub(super) fn cancel_wave_history(&mut self, cut: u64) {
        for wave in &mut self.channels.waves {
            wave.reset_cut = cut;
            // A later cut cannot undo an actual earlier pedal repair while
            // retained raw transaction history still contains the old Down.
            if wave.setup.index != NONE && wave.setup.serial <= cut {
                wave.setup = channel::Reference::default();
                wave.setup_staged = false;
            }
        }
    }

    pub(super) fn onset_wave_ready(
        &mut self,
        position: usize,
        start: i64,
        output: &mut api::Output<'_>,
    ) -> bool {
        let pending = self.pending.at(position).unwrap();
        let channel = usize::from(pending.event.channel().unwrap());
        let wave = &self.channels.waves[channel];
        if wave.first != position as u16 {
            return false;
        }
        let callback = self.callback.unwrap();
        let actual = start
            .max(callback.steady_time + i64::from(output.cursor()))
            .max(self.recovery.boundary.unwrap_or(i64::MIN));
        let Some(proposed) = actual
            .checked_sub(pending.input)
            .map(|shift| shift.max(self.wave_shift).max(self.delay()))
        else {
            self.fault(CLOCK_FAULT);
            return false;
        };
        if wave.setup.index == NONE {
            if wave.shift.is_none_or(|shift| shift == proposed) {
                return true;
            }
            if self
                .session()
                .is_none_or(|session| session.credits.load(Ordering::Acquire) >= HELD_SESSION)
            {
                return false;
            }
            // Accepted Off can precede the retention acknowledgement, but a
            // staged Off, held pedal, owed physical Off or old channel event
            // cannot authorize another translation.
            let state = &self.state.channels()[channel];
            let neutral = [64, 66, 69].into_iter().all(|cc| {
                state.controller_valid[cc / 64] & (1 << (cc % 64)) != 0
                    && state.controllers[cc] < 64
            });
            if !neutral
                || wave.wire.index != NONE
                    && (wave.wire.serial < pending.serial
                        || self
                            .pending
                            .at(usize::from(wave.wire.index))
                            .is_some_and(|header| header.staged))
                || self.channel_has_release_debt(channel as u8)
                || self.reserved.into_iter().chain(self.owed_note_off).any(|index| {
                    index != NONE
                        && self.lives.at(index).is_some_and(|life| {
                            usize::from(life.channel) == channel
                                && (life.terminal.is_none() || life.note_off_owed)
                        })
                })
            {
                return false;
            }
            let head = self.channels.head[channel].unwrap_or_default();
            let wave = &mut self.channels.waves[channel];
            wave.setup = channel::Reference { index: position as u16, serial: pending.serial };
            wave.setup_cursor = 0;
            wave.setup_next = head;
        }
        self.stage_wave_setup(channel, output)
    }

    fn next_setup_event(&mut self, channel: usize) -> Option<(Event, i64)> {
        let target = self.channels.waves[channel].setup;
        let input = self.pending.at(usize::from(target.index))?.input;
        while self.channels.waves[channel].setup_cursor < REGISTERS {
            if !self.charge(1) {
                return None;
            }
            let wave = &mut self.channels.waves[channel];
            let cursor = wave.setup_cursor;
            let data = match cursor {
                cc @ 0..=127
                    if wave.base.controller_valid[usize::from(cc / 64)] & (1 << (cc % 64)) != 0 =>
                {
                    Some([0xb0 | channel as u8, cc as u8, wave.base.controllers[usize::from(cc)]])
                }
                128 => wave.base.pressure.map(|value| [0xd0 | channel as u8, value, 0]),
                129 => wave
                    .base
                    .pitch_bend
                    .map(|value| [0xe0 | channel as u8, (value & 127) as u8, (value >> 7) as u8]),
                _ => None,
            };
            if let Some(data) = data {
                return Some((Event::Midi { port: 0, data, flags: 0 }, input));
            }
            wave.setup_cursor += 1;
        }
        loop {
            let reference = self.channels.waves[channel].setup_next;
            if reference.index == NONE || reference.serial >= target.serial {
                return None;
            }
            if !self.charge(1) {
                return None;
            }
            let pending = self.pending.at(usize::from(reference.index)).unwrap();
            assert_eq!(pending.serial, reference.serial);
            if pending.event.channel_termination().is_none()
                && !matches!(pending.event, Event::Midi { data: [status, 88, _], .. } if status & 0xf0 == 0xb0)
                && (pending.serial > self.cancel_cut || pending.channel.accepted)
            {
                if let Event::Midi { data: [status, cc @ (64 | 66 | 69), _], port, flags } =
                    pending.event
                {
                    let wave = &self.channels.waves[channel];
                    if status & 0xf0 == 0xb0
                        && pending.serial <= wave.reset_cut
                        && [64, 66, 69]
                            .iter()
                            .position(|value| *value == cc)
                            .is_some_and(|bit| wave.neutralized & (1 << bit) != 0)
                    {
                        return Some((
                            Event::Midi { data: [status, cc, 0], port, flags },
                            pending.input,
                        ));
                    }
                }
                return Some((pending.event, pending.input));
            }
            self.advance_setup_history(channel);
        }
    }

    fn advance_setup_history(&mut self, channel: usize) {
        let reference = self.channels.waves[channel].setup_next;
        let pending = self.pending.at(usize::from(reference.index)).unwrap();
        let channel::Role::Header { next_header, .. } = pending.channel.role else {
            unreachable!()
        };
        self.channels.waves[channel].setup_next = self.pending_reference(next_header);
    }

    fn setup_complete(&self, channel: usize) -> bool {
        let wave = &self.channels.waves[channel];
        wave.setup_cursor == REGISTERS
            && (wave.setup_next.index == NONE || wave.setup_next.serial >= wave.setup.serial)
    }

    fn stage_wave_setup(&mut self, channel: usize, output: &mut api::Output<'_>) -> bool {
        if self.channels.waves[channel].setup_staged {
            return false;
        }
        let Some((event, _)) = self.next_setup_event(channel) else {
            return self.setup_complete(channel);
        };
        let Some(attempt) = self.attempt.checked_add(1) else {
            self.fault(STORAGE_FAULT);
            return false;
        };
        let target = self.channels.waves[channel].setup;
        let group = api::Group::single(
            api::Token([attempt, channel as u64, target.serial, SETUP_TOKEN]),
            api::Lane::Normal,
            output.cursor(),
            event.input(),
        )
        .unwrap();
        if output.stage(group).is_err() {
            return false;
        }
        self.attempt = attempt;
        let wave = &mut self.channels.waves[channel];
        wave.setup_attempt = attempt;
        wave.setup_staged = true;
        false
    }

    pub(super) fn prepare_wave_setup(&mut self, group: api::Group) -> bool {
        #[cfg(all(test, not(feature = "tuning-probe")))]
        super::super::tests::close_setup_lease(true);
        let channel = group.token.0[1] as usize;
        let Some(wave) = self.channels.waves.get(channel) else {
            return false;
        };
        if !wave.setup_staged
            || wave.setup_attempt != group.token.0[0]
            || wave.setup.serial != group.token.0[2]
            || self.faults != 0
            || wave.setup.serial <= self.cancel_cut
            || !self.output_clock_valid()
        {
            return false;
        }
        let Some(pending) = self
            .pending
            .at(usize::from(wave.setup.index))
            .filter(|pending| pending.serial == wave.setup.serial)
        else {
            return false;
        };
        if !self.ordinary_stream_ready()
            || !self.admitted(pending.generation)
            || !self.assignment_ready(pending.life)
        {
            return false;
        }
        if self.journal.free() == 0
            || self.sequence.checked_add(1 + api::EMERGENCY_OUTPUT_ATTEMPTS as u64).is_none()
        {
            self.fault(STORAGE_FAULT);
            return false;
        }
        if self
            .callback
            .unwrap()
            .steady_time
            .checked_add(i64::from(group.time))
            .is_none_or(|actual| self.next_stop_sample().is_some_and(|stop| actual >= stop))
        {
            return false;
        }
        let gate = if let Some(offer) = &self.offer {
            let row = &offer.session.rows[usize::from(offer.lease.slot - 1)];
            let emission = self.emission(pending, row);
            if row
                .emission_gate
                .compare_exchange(emission, emission | BUSY, Ordering::AcqRel, Ordering::Acquire)
                .is_err()
            {
                return false;
            }
            true
        } else {
            false
        };
        self.permit = Some(Permit {
            position: usize::from(self.channels.waves[channel].setup.index),
            serial: pending.serial,
            credit: false,
            gate,
            emergency: false,
        });
        true
    }

    pub(super) fn complete_wave_setup(
        &mut self,
        completion: api::Completion,
        output: &mut api::Output<'_>,
    ) {
        let channel = completion.group.token.0[1] as usize;
        let Some(wave) = self.channels.waves.get(channel) else {
            return;
        };
        if !wave.setup_staged
            || wave.setup_attempt != completion.group.token.0[0]
            || wave.setup.serial != completion.group.token.0[2]
        {
            return;
        }
        let target = wave.setup;
        let input = if wave.setup_cursor < REGISTERS {
            self.pending.at(usize::from(target.index)).unwrap().input
        } else {
            self.pending.at(usize::from(wave.setup_next.index)).unwrap().input
        };
        self.channels.waves[channel].setup_staged = false;
        let permit = self.permit.take();
        if completion.accepted & 1 != 0 {
            assert!(permit.is_some_and(|permit| permit.position == usize::from(target.index)
                && permit.serial == target.serial));
            let event = Event::from_input(completion.group.event(0).unwrap()).unwrap();
            let actual = self.callback.unwrap().steady_time + i64::from(completion.group.time);
            let delta = self.record(event, NONE, input, actual);
            self.journal
                .push(delta)
                .unwrap_or_else(|_| unreachable!("prepared setup journal cell"));
            if self.channels.waves[channel].setup_cursor < REGISTERS {
                self.channels.waves[channel].setup_cursor += 1;
            } else {
                self.advance_setup_history(channel);
            }
            // Keep the claim through durable factual completion, then require
            // a fresh claim for the next separate setup group.
            if permit.is_some_and(|permit| permit.gate) {
                self.release_gate();
            }
            if self.stage_wave_setup(channel, output) && self.charge(1) {
                let callback = self.callback.unwrap();
                self.stage_pending(
                    usize::from(target.index),
                    actual,
                    callback.steady_time + i64::from(callback.frames),
                    output,
                );
            }
        } else {
            if completion.attempted != 0
                || completion.disposition == api::Disposition::MissingOutput
            {
                self.fault(OUTPUT_FAULT);
            }
            if permit.is_some_and(|permit| permit.gate) {
                self.release_gate();
            }
        }
        self.schedule_emergency(output);
    }

    pub(super) fn accept_onset_wave(&mut self, pending: Pending, actual: i64) {
        let channel = usize::from(pending.event.channel().unwrap());
        let wave = &mut self.channels.waves[channel];
        let shift = actual.checked_sub(pending.input).unwrap();
        if wave.setup.index != NONE {
            assert_eq!(wave.setup.serial, pending.serial);
            wave.wire = wave.setup_next;
            wave.delivered = pending.serial;
            wave.setup = channel::Reference::default();
        }
        wave.shift = Some(shift);
        self.wave_shift = self.wave_shift.max(shift);
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

    pub(super) fn accept_wave_neutralization(&mut self, channel: usize, bit: u8) {
        let wave = &mut self.channels.waves[channel];
        wave.neutralized |= 1 << bit;
        let cc = [64, 66, 69][usize::from(bit)];
        wave.base.controller_valid[cc / 64] |= 1 << (cc % 64);
        wave.base.controllers[cc] = 0;
    }

    pub(super) fn accept_prefix_neutralization(&mut self, channel: usize) {
        let wave = &mut self.channels.waves[channel];
        wave.repaired_stop = self.stops.reached;
        if wave.prefix_cut <= self.cancel_cut && wave.input_prefix.stop_index().is_none() {
            wave.input_prefix = Prefix::known(0);
        }
        self.settle_wave_prefix(channel);
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
                        && earlier.event != Event::Stop
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
