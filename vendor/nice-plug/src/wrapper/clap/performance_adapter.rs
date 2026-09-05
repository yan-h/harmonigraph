//! Serializes owned commands without holding any plugin, input, output or
//! configuration borrow across a host callback (including the tracing adapter).
use super::*;
use performance::{Completion, Disposition, Lane};

impl<P: ClapPlugin> Wrapper<P> {
    pub(super) fn begin_performance(&self, callback: performance::Callback) {
        self.performance_audio.store(true, Ordering::Release);
        self.legacy_send_misuse.store(false, Ordering::Release);
        self.begin_configuration_notifications();
        let mut scheduler = self.performance.lock();
        let scheduler = scheduler.as_mut().unwrap();
        scheduler
            .begin(callback.frames, callback.input_status != performance::InputStatus::Complete);
        self.plugin.lock().clap_performance_begin(callback, &mut scheduler.writer());
    }

    pub(super) unsafe fn finish_performance(
        &self,
        callback: performance::Callback,
        output: *const clap_output_events,
        status: clap_process_status,
    ) {
        {
            let mut scheduler = self.performance.lock();
            let scheduler = scheduler.as_mut().unwrap();
            if status == CLAP_PROCESS_ERROR {
                scheduler.inhibited = true;
            }
            self.plugin.lock().clap_performance_finalize(callback, status, &mut scheduler.writer());
        }
        unsafe {
            self.drain_performance(output, u32::MAX, status == CLAP_PROCESS_ERROR);
        }
        let summary = {
            let mut scheduler = self.performance.lock();
            let scheduler = scheduler.as_mut().unwrap();
            scheduler.summary.legacy_send_misuse = self.legacy_send_misuse.load(Ordering::Acquire);
            scheduler.summary
        };
        self.plugin.lock().clap_performance_end(callback, summary);
        self.performance_audio.store(false, Ordering::Release);
        if self.deferred_host_callback.swap(false, Ordering::AcqRel) {
            unsafe {
                (self.host_callback.request_callback.unwrap())(&*self.host_callback);
            }
        }
    }

    pub(super) unsafe fn drain_performance(
        &self,
        output: *const clap_output_events,
        through: u32,
        process_error: bool,
    ) {
        let output = unsafe { output.as_ref() }.filter(|o| o.try_push.is_some());
        // Every iteration retires a reserved group or spends a parameter credit.
        // A denied parameter admission cannot block already-reserved emergency.
        let mut parameters_blocked = output.is_none();
        loop {
            let (next, cursor) = {
                let scheduler = self.performance.lock();
                let scheduler = scheduler.as_ref().unwrap();
                (scheduler.next(through), scheduler.summary.cursor)
            };
            let notification = if parameters_blocked {
                None
            } else {
                self.next_configuration_notification(through, cursor)
            };
            let ordinary_ready = !parameters_blocked && {
                let mut pending = self.pending_parameter.lock();
                if pending.is_none() {
                    *pending = self.output_parameter_events.pop();
                }
                pending.is_some()
            };
            let parameter_time =
                if ordinary_ready { Some(cursor) } else { notification.map(|n| n.time) };
            if parameter_time.is_some_and(|time| next.is_none_or(|(_, g)| time <= g.time)) {
                let reserved = self.performance.lock().as_mut().unwrap().reserve_parameter();
                if !reserved {
                    parameters_blocked = true;
                    continue;
                }
                let time = parameter_time.unwrap();
                let accepted = if ordinary_ready {
                    let change = self.pending_parameter.lock().as_ref().copied().unwrap();
                    unsafe { self.push_parameter(output.unwrap(), change, time) }
                } else {
                    unsafe { notification.unwrap().push(output.unwrap()) }
                };
                self.performance.lock().as_mut().unwrap().attempted(Lane::Normal, time, accepted);
                if ordinary_ready {
                    if accepted {
                        self.pending_parameter.lock().take();
                    } else {
                        parameters_blocked = true;
                    }
                } else {
                    self.complete_configuration_notification(notification.unwrap(), accepted);
                }
                continue;
            }
            let Some((index, group)) = next else {
                break;
            };
            self.performance.lock().as_mut().unwrap().begin_group(index);
            let inhibited = self.performance.lock().as_ref().unwrap().inhibited;
            let disposition = if output.is_none() {
                Disposition::MissingOutput
            } else if group.lane == Lane::Normal && process_error {
                Disposition::ProcessError
            } else if group.lane == Lane::Normal && inhibited {
                Disposition::Inhibited
            } else {
                Disposition::Settled
            };
            let mut completion = Completion {
                group,
                attempted: 0,
                accepted: 0,
                unattempted: (1 << group.event_count()) - 1,
                disposition,
            };
            if disposition == Disposition::Settled {
                // This borrow owns the caller's final eligibility/permit claim.
                // It ends before BOTH host calls and before trace takes its lock.
                let claimed = self.plugin.lock().clap_performance_prepare(group);
                if claimed {
                    for event_index in 0..group.event_count() {
                        let bit = 1 << event_index;
                        let accepted = unsafe {
                            performance::push_value(
                                output.unwrap(),
                                group.event(event_index).unwrap(),
                                group.time,
                            )
                        };
                        completion.attempted |= bit;
                        completion.unattempted &= !bit;
                        if accepted {
                            completion.accepted |= bit;
                        }
                        self.performance
                            .lock()
                            .as_mut()
                            .unwrap()
                            .attempted(group.lane, group.time, accepted);
                        if !accepted {
                            if group.lane == Lane::Normal {
                                self.performance.lock().as_mut().unwrap().inhibited = true;
                            }
                            break;
                        }
                    }
                } else {
                    completion.disposition = Disposition::Ineligible;
                }
            }
            // Completion still owns its reserved cell until the owner records the
            // durable result and clears BUSY. The callback may reserve emergency.
            {
                let mut scheduler = self.performance.lock();
                let scheduler = scheduler.as_mut().unwrap();
                self.plugin.lock().clap_performance_complete(completion, &mut scheduler.writer());
                scheduler.complete(index);
            }
        }
    }

    unsafe fn push_parameter(
        &self,
        output: &clap_output_events,
        change: OutputParamEvent,
        time: u32,
    ) -> bool {
        let (hash, kind) = match change {
            OutputParamEvent::BeginGesture { param_hash } => {
                (param_hash, CLAP_EVENT_PARAM_GESTURE_BEGIN)
            }
            OutputParamEvent::EndGesture { param_hash } => {
                (param_hash, CLAP_EVENT_PARAM_GESTURE_END)
            }
            OutputParamEvent::SetValue { param_hash, clap_plain_value } => {
                self.update_plain_value_by_hash(
                    param_hash,
                    ClapParamUpdate::PlainValueSet(clap_plain_value),
                    self.current_buffer_config.load().map(|c| c.sample_rate),
                );
                let event = clap_event_param_value {
                    header: clap_event_header {
                        size: mem::size_of::<clap_event_param_value>() as u32,
                        time,
                        space_id: CLAP_CORE_EVENT_SPACE_ID,
                        type_: CLAP_EVENT_PARAM_VALUE,
                        flags: CLAP_EVENT_IS_LIVE,
                    },
                    param_id: param_hash,
                    cookie: std::ptr::null_mut(),
                    note_id: -1,
                    port_index: -1,
                    channel: -1,
                    key: -1,
                    value: clap_plain_value,
                };
                return unsafe { (output.try_push.unwrap())(output, &event.header) };
            }
        };
        let event = clap_event_param_gesture {
            header: clap_event_header {
                size: mem::size_of::<clap_event_param_gesture>() as u32,
                time,
                space_id: CLAP_CORE_EVENT_SPACE_ID,
                type_: kind,
                flags: CLAP_EVENT_IS_LIVE,
            },
            param_id: hash,
        };
        unsafe { (output.try_push.unwrap())(output, &event.header) }
    }
}
