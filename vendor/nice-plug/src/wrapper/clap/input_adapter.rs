//! One owned host-input allocation shared by both opt-ins.
use super::*;
use crate::wrapper::clap::configuration::*;
use crate::wrapper::clap::performance::{Consumption, InputStatus};

pub(super) struct Runtime {
    pub storage: InputStorage,
    pub configured: usize,
    performed: usize,
    walked: usize,
    batch: u64,
    pending_status: InputStatus,
}
impl Default for Runtime {
    fn default() -> Self {
        Self {
            storage: InputStorage::default(),
            configured: 0,
            performed: 0,
            walked: 0,
            batch: 0,
            pending_status: InputStatus::Complete,
        }
    }
}
impl Runtime {
    pub fn get(&self, index: usize) -> Option<OwnedInput> {
        self.storage.get(self.configured + index)
    }
    pub fn len(&self) -> usize {
        self.storage.len() - self.configured
    }
    pub fn ack_configuration(&mut self, performance: bool) {
        if performance {
            self.configured += 1;
        } else {
            self.storage.pop();
        }
    }
    fn reclaim(&mut self) {
        let count = self.configured.min(self.performed).min(self.walked);
        for _ in 0..count {
            self.storage.pop();
        }
        self.configured -= count;
        self.performed -= count;
        self.walked -= count;
    }
    pub fn reset(&mut self) {
        self.storage.reset_timed();
        self.configured = 0;
        self.performed = 0;
        self.walked = 0;
    }
}

impl<P: ClapPlugin> Wrapper<P> {
    pub(super) unsafe fn capture_input(
        &self,
        host: *const clap_input_events,
        boundary: Option<(i64, u32)>,
        transport: Option<clap_event_transport>,
    ) -> InputStatus {
        let status = unsafe { self.capture_input_batch(host, boundary, transport) };
        self.latch_input_status(status);
        if boundary.is_none() && status != InputStatus::Complete && P::CLAP_CONFIGURATION {
            self.plugin.lock().clap_configuration_fault();
        }
        status
    }

    pub(super) fn latch_input_status(&self, status: InputStatus) {
        let mut guard = self.owned_input.lock();
        let input = guard.as_mut().unwrap();
        if input.pending_status == InputStatus::Complete {
            input.pending_status = status;
        }
    }

    /// Flush has no performance callback. Keep its first loss, including early
    /// capacity/validation failures, until a process boundary reports it. A
    /// successful capture or state reset cannot erase that unreported loss.
    pub(super) fn take_input_status(&self) -> InputStatus {
        let mut guard = self.owned_input.lock();
        std::mem::replace(&mut guard.as_mut().unwrap().pending_status, InputStatus::Complete)
    }

    unsafe fn capture_input_batch(
        &self,
        host: *const clap_input_events,
        boundary: Option<(i64, u32)>,
        transport: Option<clap_event_transport>,
    ) -> InputStatus {
        let (cut, batch, original_len) = {
            let mut guard = self.owned_input.lock();
            let input = guard.as_mut().unwrap();
            let Some(cut) = self.prepare_configuration_capture(input, boundary) else {
                return InputStatus::Invalid;
            };
            let Some(batch) = input.batch.checked_add(1) else {
                return InputStatus::Invalid;
            };
            input.batch = batch;
            (cut, batch, input.storage.len())
        };
        if boundary.is_some_and(|(start, frames)| {
            start < 0 || frames == 0 || start.checked_add(i64::from(frames)).is_none()
        }) {
            return InputStatus::Invalid;
        }
        // Host callbacks run with no plugin/runtime/event storage lock held.
        let count = if host.is_null() {
            0
        } else {
            let host = unsafe { &*host };
            let (Some(size), Some(_)) = (host.size, host.get) else {
                return InputStatus::Invalid;
            };
            (unsafe { size(host) }) as usize
        };
        let marker = usize::from(P::CLAP_PERFORMANCE && transport.is_some());
        if count > INPUT_SCAN
            || count + marker > self.owned_input.lock().as_ref().unwrap().storage.available()
        {
            return InputStatus::Full;
        }
        let make = |value, index, offset| OwnedInput {
            sample: boundary.and_then(|(start, _)| start.checked_add(i64::from(offset))),
            event_index: index,
            offset,
            enclosing_start: boundary.map(|b| b.0),
            enclosing_frames: boundary.map_or(0, |b| b.1),
            flush: boundary.is_none(),
            command_cut: cut,
            command_sample: boundary.map(|b| b.0),
            batch,
            value,
        };
        if P::CLAP_PERFORMANCE {
            if let Some(transport) = transport {
                self.owned_input
                    .lock()
                    .as_mut()
                    .unwrap()
                    .storage
                    .push(make(InputValue::Transport(transport.into()), u32::MAX, 0))
                    .unwrap();
            }
        }
        let mut status = InputStatus::Complete;
        let mut previous_time = 0;
        for index in 0..count {
            let pointer = unsafe { ((*host).get.unwrap())(host, index as u32) };
            let Some(header) = (unsafe { pointer.as_ref() }) else {
                status = InputStatus::Invalid;
                break;
            };
            if boundary.is_some_and(|(_, frames)| {
                header.time >= frames || index > 0 && header.time < previous_time
            }) {
                status = InputStatus::Invalid;
                break;
            }
            previous_time = header.time;
            let value = match unsafe { decode(pointer, P::CLAP_PERFORMANCE) } {
                Ok(value) => value,
                Err(error) => {
                    status = error;
                    break;
                }
            };
            self.owned_input
                .lock()
                .as_mut()
                .unwrap()
                .storage
                .push(make(value, index as u32, header.time))
                .unwrap();
        }
        {
            let mut guard = self.owned_input.lock();
            let input = guard.as_mut().unwrap();
            if status != InputStatus::Complete {
                input.storage.truncate(original_len);
            }
        }
        status
    }

    pub(super) fn bind_performance_input(&self, start: i64) {
        if !P::CLAP_CONFIGURATION {
            let mut guard = self.owned_input.lock();
            let input = guard.as_mut().unwrap();
            input.storage.bind_untimed(start);
            input.configured = input.storage.len();
        }
    }

    pub(super) fn deliver_performance_input(&self) {
        let mut guard = self.owned_input.lock();
        let input = guard.as_mut().unwrap();
        let mut plugin = self.plugin.lock();
        for _ in input.performed..input.configured {
            let event = input.storage.get(input.performed).unwrap();
            if plugin.clap_performance_input(event) == Consumption::Pending {
                break;
            }
            input.performed += 1;
        }
        input.reclaim();
    }
    /// Wrapper consumption is a third short cursor in the SAME pool. Retain
    /// cells until configuration, performance and this walker have all finished;
    /// no second host scan, pointer cache or converted input deque is needed.
    pub(super) fn walk_owned_input(
        &self,
        start: u32,
        frames: u32,
        transport: &mut Option<clap_event_transport>,
    ) -> u32 {
        loop {
            let event = {
                let guard = self.owned_input.lock();
                let input = guard.as_ref().unwrap();
                input.storage.get(input.walked)
            };
            let Some(event) = event else {
                return frames;
            };
            let time = if event.flush { 0 } else { event.offset };
            let split = matches!(event.value, InputValue::Transport(_))
                || P::SAMPLE_ACCURATE_AUTOMATION
                    && matches!(event.value, InputValue::Parameter { .. });
            if time > start && split {
                return time;
            }
            match event.value {
                InputValue::Transport(value) => *transport = Some(value.event(time)),
                InputValue::Parameter { id, value, modulation } if !self.configuration_owns(id) => {
                    self.update_plain_value_by_hash(
                        id,
                        if modulation {
                            ClapParamUpdate::PlainValueMod(value)
                        } else {
                            ClapParamUpdate::PlainValueSet(value)
                        },
                        self.current_buffer_config.load().map(|c| c.sample_rate),
                    );
                }
                _ => {}
            }
            let mut guard = self.owned_input.lock();
            let input = guard.as_mut().unwrap();
            input.walked += 1;
            input.reclaim();
        }
    }
    pub(super) fn finish_owned_walk(&self) {
        let mut guard = self.owned_input.lock();
        let input = guard.as_mut().unwrap();
        input.walked = input.storage.len();
        input.reclaim();
    }
}

unsafe fn decode(
    pointer: *const clap_event_header,
    performance: bool,
) -> Result<InputValue, InputStatus> {
    let header = unsafe { &*pointer };
    let required = if header.space_id == CLAP_CORE_EVENT_SPACE_ID {
        match header.type_ {
            CLAP_EVENT_PARAM_VALUE => mem::size_of::<clap_event_param_value>(),
            CLAP_EVENT_PARAM_MOD => mem::size_of::<clap_event_param_mod>(),
            CLAP_EVENT_NOTE_ON
            | CLAP_EVENT_NOTE_OFF
            | CLAP_EVENT_NOTE_CHOKE
            | CLAP_EVENT_NOTE_END => mem::size_of::<clap_event_note>(),
            CLAP_EVENT_NOTE_EXPRESSION => mem::size_of::<clap_event_note_expression>(),
            CLAP_EVENT_MIDI => mem::size_of::<clap_event_midi>(),
            CLAP_EVENT_TRANSPORT => mem::size_of::<clap_event_transport>(),
            CLAP_EVENT_PARAM_GESTURE_BEGIN | CLAP_EVENT_PARAM_GESTURE_END => {
                mem::size_of::<clap_event_param_gesture>()
            }
            _ if performance => return Err(InputStatus::Unsupported),
            _ => mem::size_of::<clap_event_header>(),
        }
    } else if performance {
        return Err(InputStatus::Unsupported);
    } else {
        mem::size_of::<clap_event_header>()
    };
    if (header.size as usize) < required {
        return Err(InputStatus::Invalid);
    }
    if performance && header.space_id == CLAP_CORE_EVENT_SPACE_ID {
        // Per-note parameter automation/modulation is not part of this raw
        // note-expression/MIDI contract; never silently turn it monophonic.
        let addressing = match header.type_ {
            CLAP_EVENT_PARAM_VALUE => {
                let e = unsafe { &*pointer.cast::<clap_event_param_value>() };
                Some((e.note_id, e.port_index, e.channel, e.key))
            }
            CLAP_EVENT_PARAM_MOD => {
                let e = unsafe { &*pointer.cast::<clap_event_param_mod>() };
                Some((e.note_id, e.port_index, e.channel, e.key))
            }
            _ => None,
        };
        if addressing.is_some_and(|a| a != (-1, -1, -1, -1)) {
            return Err(InputStatus::Unsupported);
        }
    }
    let value = if header.space_id != CLAP_CORE_EVENT_SPACE_ID {
        InputValue::Other
    } else {
        match header.type_ {
            CLAP_EVENT_PARAM_VALUE => {
                let event = unsafe { &*pointer.cast::<clap_event_param_value>() };
                InputValue::Parameter { id: event.param_id, value: event.value, modulation: false }
            }
            CLAP_EVENT_PARAM_MOD => {
                let event = unsafe { &*pointer.cast::<clap_event_param_mod>() };
                if event.note_id == -1 {
                    InputValue::Parameter {
                        id: event.param_id,
                        value: event.amount,
                        modulation: true,
                    }
                } else {
                    InputValue::Other
                }
            }
            CLAP_EVENT_NOTE_ON
            | CLAP_EVENT_NOTE_OFF
            | CLAP_EVENT_NOTE_CHOKE
            | CLAP_EVENT_NOTE_END => {
                let event = unsafe { &*pointer.cast::<clap_event_note>() };
                InputValue::Note {
                    kind: header.type_,
                    note_id: event.note_id,
                    port: event.port_index,
                    channel: event.channel,
                    key: event.key,
                    velocity: event.velocity,
                    flags: header.flags,
                }
            }
            CLAP_EVENT_NOTE_EXPRESSION => {
                let event = unsafe { &*pointer.cast::<clap_event_note_expression>() };
                InputValue::Expression {
                    expression: event.expression_id,
                    note_id: event.note_id,
                    port: event.port_index,
                    channel: event.channel,
                    key: event.key,
                    value: event.value,
                    flags: header.flags,
                }
            }
            CLAP_EVENT_MIDI => {
                let event = unsafe { &*pointer.cast::<clap_event_midi>() };
                InputValue::Midi { port: event.port_index, data: event.data, flags: header.flags }
            }
            CLAP_EVENT_TRANSPORT => {
                InputValue::Transport(unsafe { *pointer.cast::<clap_event_transport>() }.into())
            }
            _ => InputValue::Other,
        }
    };
    Ok(value)
}
