//! Serializes wrapper-owned parameter notifications without holding plugin,
//! input, output or configuration borrows across their host callbacks.
//! Plugin performance hooks themselves run under the plugin lock, including
//! when they push musical output to the host.
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
        let writable = callback.input_status == performance::InputStatus::Complete;
        let mut output =
            unsafe { performance::Output::new(output, &self.output_high_water, writable) };
        self.plugin.lock().clap_performance_begin(callback, &mut output);
    }

    pub(super) unsafe fn finish_performance(
        &self,
        callback: performance::Callback,
        output: *const clap_output_events,
        status: clap_process_status,
    ) {
        // A callback the host has already lost does not get the plugin's final
        // flush: an uncapturable input or a process error makes this output
        // blind, so the plugin's `push` fails and it retains what it was about
        // to emit for a callback the host will read.
        let writable = status != CLAP_PROCESS_ERROR
            && callback.input_status == performance::InputStatus::Complete;
        {
            let mut writer =
                unsafe { performance::Output::new(output, &self.output_high_water, writable) };
            self.plugin.lock().clap_performance_finalize(callback, status, &mut writer);
        }
        // The parameter half drains with the real list even on an error exit:
        // a host that discards the audio still keeps its parameter cache.
        unsafe {
            self.drain_performance(output);
        }
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

    /// The wrapper's own parameter output for one sub-block: the GUI's edits,
    /// each floored at the high-water mark of what this callback has already
    /// put on the wire, since a GUI edit has no source sample of its own.
    /// [`performance::Output::push`] holds the whole ordering argument.
    ///
    /// [`performance::PARAMETER_OUTPUT_ATTEMPTS`] bounds what one callback
    /// spends here across all of its sub-block drains. The loop would terminate
    /// without it — the GUI half is a prefix admitted once per callback, at
    /// capture — but it would terminate at `INPUT_SCAN`.
    pub(super) unsafe fn drain_performance(&self, output: *const clap_output_events) {
        let Some(output) = unsafe { output.as_ref() }.filter(|o| o.try_push.is_some()) else {
            return;
        };
        while self.parameter_attempts.load(Ordering::Acquire)
            < performance::PARAMETER_OUTPUT_ATTEMPTS
        {
            let floor = self.output_high_water.load(Ordering::Acquire);
            // Copy before host callbacks. Only the captured/admitted prefix can
            // notify; a GUI arrival during this callback waits for its next one.
            let Some(change) = self.output_parameter_events.borrow_mut().notification() else {
                break;
            };
            self.parameter_attempts.fetch_add(1, Ordering::AcqRel);
            if !unsafe { self.push_parameter(output, change, floor) } {
                // A refused GUI value ends the drain, so no later value
                // overtakes the one the host just declined.
                break;
            }
            self.output_parameter_events.borrow_mut().accept();
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
