//! Serializes owned commands without holding any plugin, input, output or
//! configuration borrow across a host callback (including the tracing adapter).
use super::*;

impl<P: ClapPlugin> Wrapper<P> {
    pub(super) unsafe fn begin_performance(
        &self,
        callback: performance::Callback,
        output: *const clap_output_events,
    ) {
        self.performance_audio.store(true, Ordering::Release);
        self.legacy_send_misuse.store(false, Ordering::Release);
        self.parameter_attempts.store(0, Ordering::Release);
        self.output_high_water.store(0, Ordering::Release);
        self.begin_configuration_notifications();
        let writable = callback.input_status == performance::InputStatus::Complete;
        let mut output =
            unsafe { performance::Output::new(output, &self.output_high_water, writable) };
        self.plugin.lock().clap_performance_begin(callback, &mut output);
    }

    /// `output` is the wrapper's own route, which the process trace observes.
    /// `plugin_output` is the host's original list, which the plugin writes to
    /// directly — the trace hook takes the plugin lock, and the plugin holds it.
    pub(super) unsafe fn finish_performance(
        &self,
        callback: performance::Callback,
        output: *const clap_output_events,
        plugin_output: *const clap_output_events,
        status: clap_process_status,
    ) {
        // A callback the host has already lost does not get the plugin's final
        // flush: an uncapturable input or a process error makes this output
        // blind, so the plugin's `push` fails and it retains what it was about
        // to emit for a callback the host will read.
        let writable = status != CLAP_PROCESS_ERROR
            && callback.input_status == performance::InputStatus::Complete;
        {
            let mut writer = unsafe {
                performance::Output::new(plugin_output, &self.output_high_water, writable)
            };
            self.plugin.lock().clap_performance_finalize(callback, status, &mut writer);
        }
        // The parameter half drains with the real list even on an error exit:
        // a host that discards the audio still keeps its parameter cache, and
        // the racing-restore rescan depends on that value reaching it.
        unsafe {
            self.drain_performance(output, callback.frames.saturating_sub(1));
        }
        // A restore can service its rescan while an older host value attempt is
        // in flight. Completion relatches dirty; every exit must wake that work.
        self.configuration_request_main();
        let summary = performance::Summary {
            legacy_send_misuse: self.legacy_send_misuse.load(Ordering::Acquire),
        };
        self.plugin.lock().clap_performance_end(callback, summary);
        self.performance_audio.store(false, Ordering::Release);
        if self.deferred_host_callback.swap(false, Ordering::AcqRel) {
            unsafe {
                (self.host_callback.request_callback.unwrap())(&*self.host_callback);
            }
        }
    }

    /// The wrapper's own parameter and configuration output for one sub-block,
    /// bounded by `through` — its last sample — and floored at the high-water
    /// mark of what this callback has already put on the wire.
    /// [`performance::Output::push`] holds the whole ordering argument.
    ///
    /// The mark is what the notification cursor carries, so a value keeps its
    /// own mapped offset unless something later is already on the wire, in which
    /// case it lands just after that. Pinning to `through` instead would be one
    /// line shorter and would cost every notification its sample: without
    /// `SAMPLE_ACCURATE_AUTOMATION` a steady-transport callback is a single
    /// sub-block, so `through` is the whole buffer's last sample and distinct
    /// source samples would collapse onto it. What the plugin plays is not the
    /// only thing at stake in an offset; what the host RECORDS is the other,
    /// and it is the one this half exists for.
    ///
    /// The mark cannot exceed `through`, because the plugin's events for this
    /// sub-block are all inside it — so a notification whose offset maps past
    /// `through` still fails the `time <= through` filter and waits for the
    /// sub-block that contains it, exactly as before. A caller that broke that
    /// contract would starve its own notifications rather than unsort the list.
    ///
    /// [`performance::PARAMETER_OUTPUT_ATTEMPTS`] bounds what one callback
    /// spends here across all of its sub-block drains. The loop would terminate
    /// without it — the GUI half is a prefix admitted once per callback, at
    /// capture, and the notification bank is a fixed number of cells whose
    /// phases only advance — but it would terminate at `INPUT_SCAN`.
    pub(super) unsafe fn drain_performance(
        &self,
        output: *const clap_output_events,
        through: u32,
    ) {
        let Some(output) = unsafe { output.as_ref() }.filter(|o| o.try_push.is_some()) else {
            return;
        };
        while self.parameter_attempts.load(Ordering::Acquire)
            < performance::PARAMETER_OUTPUT_ATTEMPTS
        {
            let floor = self.output_high_water.load(Ordering::Acquire);
            let notification = self.next_configuration_notification(through, floor);
            // Copy before host callbacks. Only the captured/admitted prefix can
            // notify; a GUI arrival during this callback waits for its next one.
            let ordinary = self.output_parameter_events.borrow_mut().notification();
            if ordinary.is_none() && notification.is_none() {
                break;
            }
            self.parameter_attempts.fetch_add(1, Ordering::AcqRel);
            // A GUI edit has no source sample of its own, so it takes the floor.
            let time = if ordinary.is_some() { floor } else { notification.unwrap().time };
            let accepted = if let Some(change) = ordinary {
                let accepted = unsafe { self.push_parameter(output, change, time) };
                if accepted {
                    self.output_parameter_events.borrow_mut().accept();
                }
                accepted
            } else {
                let notification = notification.unwrap();
                let accepted = unsafe { notification.push(output) };
                self.complete_configuration_notification(notification, accepted);
                accepted
            };
            if accepted {
                // Raise the floor for the next iteration, or two values with
                // distinct offsets could leave in the wrong order.
                self.output_high_water.fetch_max(time, Ordering::AcqRel);
            } else if ordinary.is_some() {
                // A refused GUI value ends the drain rather than letting a
                // notification overtake the value the host just declined.
                break;
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
