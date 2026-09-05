//! Ordered configuration ownership sharing the acknowledged owned input pool
//! with the independently opted-in performance boundary.
use super::*;
use crate::wrapper::clap::configuration::*;

pub(super) struct Runtime {
    pub mailbox: Arc<ConfigurationMailbox>,
    commands: rtrb::Consumer<QueuedConfiguration>,
    hashes: [Option<u32>; CONFIG_PARAMETERS],
    base: [f32; CONFIG_PARAMETERS],
    modulation: [f32; CONFIG_PARAMETERS],
    applied_id: u64,
    group: Option<Group>,
    notifications: [Option<Notification>; 16],
    fault: bool,
    reset_generation: u64,
    pending_learning: Option<(i64, ConfigurationEdit)>,
    // One first-observation cell per queued command, independent of input drain.
    observed_samples: [Option<(u64, Option<i64>)>; CONFIG_COMMANDS],
    observed_through: u64,
    output_boundary: Option<(i64, u32)>,
    gesture_ends: [bool; CONFIG_PARAMETERS],
    callback_cut: u64,
    notification_sequence: u64,
    notification_blocked: [bool; CONFIG_PARAMETERS],
}
struct Group {
    sample: i64,
    count: usize,
    parameter_cursor: usize,
    note_cursor: usize,
    learned: Option<ConfigurationEdit>,
    learned_ready: bool,
}
#[derive(Clone, Copy)]
struct Notification {
    id: u64,
    sequence: u64,
    sample: i64,
    values: [Option<f32>; CONFIG_PARAMETERS],
    parameter: usize,
    phase: u8,
}

impl<P: ClapPlugin> Wrapper<P> {
    /// Off-thread binding for a structured configuration UI or embedding host.
    pub fn configuration_handle(&self) -> Option<Arc<ConfigurationMailbox>> {
        self.configuration_mailbox.get().cloned()
    }

    pub(super) fn install_configuration(self: &Arc<Self>) {
        assert!(P::CLAP_CONFIGURATION_PARAMS.len() <= CONFIG_PARAMETERS);
        let weak = Arc::downgrade(self);
        let (mailbox, commands) = ConfigurationMailbox::new(Box::new(move || {
            if let Some(wrapper) = weak.upgrade() {
                // Off-thread submission only, after dropping the producer lock.
                let host_params =
                    wrapper.host_params.borrow().as_ref().map(|p| &**p as *const clap_host_params);
                if let Some(host_params) = host_params {
                    unsafe_clap_call! { host_params=>request_flush(&*wrapper.host_callback) };
                }
                wrapper.configuration_request_main();
            }
        }));
        let hashes = std::array::from_fn(|i| {
            P::CLAP_CONFIGURATION_PARAMS.get(i).map(|id| self.param_id_to_hash[*id])
        });
        let base = hashes.map(|hash| {
            hash.map_or(0.0, |hash| unsafe {
                self.param_by_hash[&hash].unmodulated_normalized_value()
            })
        });
        self.configuration_mailbox
            .set(mailbox.clone())
            .unwrap_or_else(|_| panic!("configuration installed twice"));
        self.plugin.lock().clap_configuration_install(mailbox.clone());
        *self.configuration.lock() = Some(Runtime {
            mailbox,
            commands,
            hashes,
            base,
            modulation: [0.0; CONFIG_PARAMETERS],
            applied_id: 0,
            group: None,
            notifications: [None; 16],
            fault: false,
            reset_generation: 0,
            pending_learning: None,
            observed_samples: [None; CONFIG_COMMANDS],
            observed_through: 0,
            output_boundary: None,
            gesture_ends: [false; CONFIG_PARAMETERS],
            callback_cut: 0,
            notification_sequence: 0,
            notification_blocked: [false; CONFIG_PARAMETERS],
        });
    }

    pub(super) fn configuration_owns(&self, hash: u32) -> bool {
        P::CLAP_CONFIGURATION
            && P::CLAP_CONFIGURATION_PARAMS.iter().any(|id| self.param_id_to_hash[*id] == hash)
    }

    fn configuration_values(
        &self,
        runtime: &Runtime,
        base: [f32; CONFIG_PARAMETERS],
        modulation: [f32; CONFIG_PARAMETERS],
        sample: i64,
    ) -> ConfigurationCommit {
        let unmodulated = std::array::from_fn(|i| {
            runtime.hashes[i]
                .map_or(0.0, |hash| unsafe { self.param_by_hash[&hash].preview_plain(base[i]) })
        });
        let normalized = std::array::from_fn(|i| (base[i] + modulation[i]).clamp(0.0, 1.0));
        let raw = std::array::from_fn(|i| {
            runtime.hashes[i].map_or(0.0, |hash| unsafe {
                self.param_by_hash[&hash].preview_plain(normalized[i])
            })
        });
        ConfigurationCommit { sample, raw, unmodulated, normalized, modulation }
    }

    fn apply_configuration(
        &self,
        runtime: &mut Runtime,
        plugin: &mut P,
        command: ConfigurationCommand,
        sample: i64,
        modulation: Option<(usize, f32)>,
    ) -> bool {
        if matches!(command.origin, ConfigurationOrigin::Ui | ConfigurationOrigin::Learning)
            && runtime.notifications.iter().all(Option::is_some)
        {
            return false;
        }
        if matches!(command.origin, ConfigurationOrigin::Ui | ConfigurationOrigin::Learning)
            && runtime.notification_sequence == u64::MAX
        {
            runtime.fault = true;
            plugin.clap_configuration_fault();
            return false;
        }
        let mut base = runtime.base;
        let mut offsets = runtime.modulation;
        for (i, value) in command.edit.values.iter().enumerate() {
            if let (Some(value), Some(hash)) = (value, runtime.hashes[i]) {
                base[i] =
                    unsafe { self.param_by_hash[&hash].preview_normalized(*value) }.clamp(0.0, 1.0);
            }
        }
        if let Some((index, offset)) = modulation {
            offsets[index] = offset;
        }
        let commit = self.configuration_values(runtime, base, offsets, sample);
        let Some(mut snapshot) = plugin.clap_configuration_apply(command, commit) else {
            return false;
        };
        runtime.base = base;
        runtime.modulation = offsets;
        if command.id != 0 {
            runtime.applied_id = command.id;
        }
        snapshot.applied_id = runtime.applied_id;
        snapshot.raw = commit.raw;
        snapshot.unmodulated = commit.unmodulated;
        snapshot.normalized = commit.normalized;
        snapshot.modulation = offsets;
        // Mirrors only. Owned parameters never enter the legacy input path.
        // Host readback/save use the coherent published or accepted snapshot.
        for (i, hash) in runtime.hashes.iter().enumerate() {
            if let Some(hash) = hash {
                let ptr = self.param_by_hash[hash];
                unsafe {
                    ptr._internal_set_normalized_value(base[i]);
                    ptr._internal_modulate_value(offsets[i]);
                }
            }
        }
        runtime.mailbox.published.publish(snapshot);
        if matches!(command.origin, ConfigurationOrigin::Ui | ConfigurationOrigin::Learning) {
            // One cell per applied command, bounded by the plugin's 16-work slice.
            if let Some(cell) = runtime.notifications.iter_mut().find(|cell| cell.is_none()) {
                let values = std::array::from_fn(|i| command.edit.values[i].map(|_| base[i]));
                runtime.notification_sequence += 1;
                *cell = Some(Notification {
                    id: runtime.applied_id,
                    sequence: runtime.notification_sequence,
                    sample,
                    values,
                    parameter: 0,
                    phase: 0,
                });
            } else {
                runtime.fault = true;
                plugin.clap_configuration_fault();
            }
            runtime.mailbox.dirty.store(true, Ordering::Release);
        }
        true
    }

    fn drain_configuration_commands(
        &self,
        runtime: &mut Runtime,
        plugin: &mut P,
        cut: u64,
        sample: i64,
    ) -> bool {
        if !self.apply_configuration_cut(runtime, plugin, cut, sample) {
            return false;
        }
        true
    }

    fn apply_configuration_cut(
        &self,
        runtime: &mut Runtime,
        plugin: &mut P,
        cut: u64,
        _sample: i64,
    ) -> bool {
        while let Ok(&queued) = runtime.commands.peek() {
            let command = runtime.mailbox.command(queued);
            if command.id > cut {
                break;
            }
            let index = command.id as usize % CONFIG_COMMANDS;
            let Some((id, Some(sample))) = runtime.observed_samples[index] else {
                runtime.fault = true;
                plugin.clap_configuration_fault();
                return false;
            };
            if id != command.id {
                runtime.fault = true;
                plugin.clap_configuration_fault();
                return false;
            }
            if !self.apply_configuration(runtime, plugin, command, sample, None) {
                return false;
            }
            runtime.commands.pop().expect("peeked configuration command");
            runtime.mailbox.retained(queued);
            runtime.observed_samples[index] = None;
        }
        true
    }

    pub(super) fn reset_configuration_walk(&self) {
        if let Some(runtime) = self.configuration.lock().as_mut() {
            runtime.group = None;
            runtime.pending_learning = None;
            runtime.fault = false;
            for (_, sample) in runtime.observed_samples.iter_mut().flatten() {
                *sample = None;
            }
        }
    }

    pub(super) fn prepare_configuration_capture(
        &self,
        input: &mut input_adapter::Runtime,
        boundary: Option<(i64, u32)>,
    ) -> Option<u64> {
        let mut guard = self.configuration.lock();
        let Some(runtime) = guard.as_mut() else {
            return Some(0);
        };
        let reset = runtime.mailbox.reset_generation.load(Ordering::Acquire);
        if reset != runtime.reset_generation {
            input.reset();
            runtime.group = None;
            runtime.pending_learning = None;
            for (_, sample) in runtime.observed_samples.iter_mut().flatten() {
                *sample = None;
            }
            runtime.fault = false;
            runtime.reset_generation = reset;
        }
        let cut = runtime.mailbox.accepted_command.load(Ordering::Acquire);
        runtime.callback_cut = cut;
        runtime.output_boundary = boundary;
        if cut > runtime.observed_through {
            for id in runtime.observed_through + 1..=cut {
                let cell = &mut runtime.observed_samples[id as usize % CONFIG_COMMANDS];
                if cell.is_some() {
                    runtime.fault = true;
                    self.plugin.lock().clap_configuration_fault();
                    return None;
                }
                *cell = Some((id, boundary.map(|b| b.0)));
            }
            runtime.observed_through = cut;
        }
        Some(cut)
    }

    pub(super) fn process_configuration(&self, boundary: ConfigurationBoundary) {
        let mut input_guard = self.owned_input.lock();
        let input = input_guard.as_mut().unwrap();
        let mut guard = self.configuration.lock();
        let Some(runtime) = guard.as_mut() else {
            return;
        };
        let mut plugin = self.plugin.lock();
        let reset = runtime.mailbox.reset_generation.load(Ordering::Acquire);
        if reset != runtime.reset_generation {
            input.reset();
            runtime.group = None;
            runtime.pending_learning = None;
            for (_, sample) in runtime.observed_samples.iter_mut().flatten() {
                *sample = None;
            }
            runtime.fault = false;
            runtime.reset_generation = reset;
        }
        for (_, sample) in runtime.observed_samples.iter_mut().flatten() {
            if sample.is_none() {
                *sample = Some(boundary.steady_time);
            }
        }
        plugin.clap_configuration_begin(boundary);
        if let Some((sample, edit)) = runtime.pending_learning {
            if !self.apply_configuration(
                runtime,
                &mut plugin,
                ConfigurationCommand { id: 0, origin: ConfigurationOrigin::Learning, edit },
                sample,
                None,
            ) {
                return;
            }
            runtime.pending_learning = None;
        }
        if boundary.steady_time < 0 || runtime.fault {
            plugin.clap_configuration_fault();
            return;
        }
        input.storage.bind_untimed(boundary.steady_time);
        loop {
            let Some(first) = input.get(0) else {
                break;
            };
            if !self.drain_configuration_commands(
                runtime,
                &mut plugin,
                first.command_cut,
                first.command_sample.unwrap(),
            ) {
                return;
            }
            if runtime.group.is_none() {
                let count = (0..input.len())
                    .take_while(|&i| {
                        input.get(i).is_some_and(|input| {
                            input.sample == first.sample && input.batch == first.batch
                        })
                    })
                    .count();
                runtime.group = Some(Group {
                    sample: first.sample.unwrap(),
                    count,
                    parameter_cursor: 0,
                    note_cursor: 0,
                    learned: None,
                    learned_ready: false,
                });
            }
            // Every same-sample parameter retains its original order. All apply
            // before the cohort's notes, whose own lifecycle order is unchanged.
            while runtime.group.as_ref().unwrap().parameter_cursor
                < runtime.group.as_ref().unwrap().count
            {
                let group = runtime.group.as_ref().unwrap();
                let input = input.get(group.parameter_cursor).unwrap();
                if let InputValue::Parameter { id, value, modulation } = input.value {
                    if let Some(index) = runtime.hashes.iter().position(|hash| *hash == Some(id)) {
                        if !value.is_finite() {
                            runtime.fault = true;
                            plugin.clap_configuration_fault();
                            return;
                        }
                        let mut edit = ConfigurationEdit::default();
                        let mut offset = None;
                        if modulation {
                            offset = Some((index, value as f32));
                        } else {
                            edit.values[index] = Some(unsafe {
                                self.param_by_hash[&id]
                                    .preview_plain((value as f32).clamp(0.0, 1.0))
                            });
                        }
                        let command = ConfigurationCommand {
                            id: 0,
                            origin: if input.flush {
                                ConfigurationOrigin::Flush
                            } else {
                                ConfigurationOrigin::Automation
                            },
                            edit,
                        };
                        if !self.apply_configuration(
                            runtime,
                            &mut plugin,
                            command,
                            input.sample.unwrap(),
                            offset,
                        ) {
                            return;
                        }
                    }
                }
                runtime.group.as_mut().unwrap().parameter_cursor += 1;
            }
            while runtime.group.as_ref().unwrap().note_cursor
                < runtime.group.as_ref().unwrap().count
            {
                let cursor = runtime.group.as_ref().unwrap().note_cursor;
                plugin.clap_configuration_observe(input.get(cursor).unwrap());
                runtime.group.as_mut().unwrap().note_cursor += 1;
            }
            let group = runtime.group.as_mut().unwrap();
            if !group.learned_ready {
                group.learned = plugin.clap_configuration_group_end(group.sample);
                group.learned_ready = true;
            }
            if let Some(edit) = group.learned {
                let sample = group.sample;
                let command =
                    ConfigurationCommand { id: 0, origin: ConfigurationOrigin::Learning, edit };
                if !self.apply_configuration(runtime, &mut plugin, command, sample, None) {
                    return;
                }
            }
            let count = runtime.group.take().unwrap().count;
            for _ in 0..count {
                input.ack_configuration(P::CLAP_PERFORMANCE);
            }
        }
        // A silent callback still adopts UI/restore commands. Untimed flush never
        // invents sample zero, and nothing waits for a GUI/background acknowledgement.
        let cut = runtime.callback_cut;
        if self.drain_configuration_commands(runtime, &mut plugin, cut, boundary.steady_time) {
            if let Some(edit) = plugin.clap_configuration_group_end(boundary.steady_time) {
                if !self.apply_configuration(
                    runtime,
                    &mut plugin,
                    ConfigurationCommand { id: 0, origin: ConfigurationOrigin::Learning, edit },
                    boundary.steady_time,
                    None,
                ) {
                    runtime.pending_learning = Some((boundary.steady_time, edit));
                }
            }
        }
    }

    pub(super) fn publish_configuration_prefix(&self, start: i64, frames: u32) {
        let input_guard = self.owned_input.lock();
        let input = input_guard.as_ref().unwrap();
        let guard = self.configuration.lock();
        let Some(runtime) = guard.as_ref() else {
            return;
        };
        let mut through = start.saturating_add(i64::from(frames));
        if let Some(input) = input.get(0) {
            through = through.min(input.sample.unwrap_or(start));
        }
        for (_, sample) in runtime.observed_samples.iter().flatten() {
            through = through.min(sample.unwrap_or(start));
        }
        if let Some((sample, _)) = runtime.pending_learning {
            through = through.min(sample);
        }
        self.plugin.lock().clap_configuration_prefix(through);
    }

    /// Merge configuration values before performance events at this legal output
    /// offset. A retained older value is necessarily a late host notification at
    /// offset zero; its effective configuration/take sample remains unchanged.
    pub(super) unsafe fn notify_configuration(&self, output: &clap_output_events, through: u32) {
        let (notifications, hashes, mut ends) = {
            let mut guard = self.configuration.lock();
            let Some(runtime) = guard.as_mut() else {
                return;
            };
            let mut ready = [None; 16];
            for (index, cell) in runtime.notifications.iter_mut().enumerate() {
                if let Some(notification) = cell {
                    let time = runtime.output_boundary.map_or(0, |(start, frames)| {
                        (notification.sample.saturating_sub(start).max(0) as u64)
                            .min(u64::from(frames.saturating_sub(1))) as u32
                    });
                    if time <= through {
                        ready[index] = Some((*notification, time));
                        *cell = None;
                    }
                }
            }
            (ready, runtime.hashes, runtime.gesture_ends)
        };
        let mailbox = self.configuration_mailbox.get().unwrap();
        let gesture = |hash, kind, time| clap_event_param_gesture {
            header: clap_event_header {
                size: mem::size_of::<clap_event_param_gesture>() as u32,
                time,
                space_id: CLAP_CORE_EVENT_SPACE_ID,
                type_: kind,
                flags: CLAP_EVENT_IS_LIVE,
            },
            param_id: hash,
        };
        let reject = || {
            mailbox.notification_rejected.store(true, Ordering::Release);
            mailbox.dirty.store(true, Ordering::Release);
        };
        // A rejected end owns one bounded debt per parameter. No later begin
        // may cross it; a failed begin creates no value/end transaction at all.
        // `through` may be later than a queued learning notification. Close at
        // the earliest ready output before emitting that notification's begin.
        let closing_time =
            notifications.iter().flatten().map(|(_, time)| *time).min().unwrap_or(through);
        for (i, debt) in ends.iter_mut().enumerate() {
            if *debt {
                let end = gesture(hashes[i].unwrap(), CLAP_EVENT_PARAM_GESTURE_END, closing_time);
                *debt = !unsafe {
                    clap_call! { output=>try_push(output, &end.header) }
                };
                if *debt {
                    reject();
                }
            }
        }
        for (notification, time) in notifications.into_iter().flatten() {
            if notification.id < mailbox.accepted_restore.load(Ordering::Acquire) {
                continue;
            }
            for (i, value) in notification.values.into_iter().enumerate() {
                let (Some(value), Some(hash)) = (value, hashes[i]) else {
                    continue;
                };
                if ends[i] {
                    reject();
                    continue;
                }
                let begin = gesture(hash, CLAP_EVENT_PARAM_GESTURE_BEGIN, time);
                if !unsafe {
                    clap_call! { output=>try_push(output, &begin.header) }
                } {
                    reject();
                    continue;
                }
                let event = clap_event_param_value {
                    header: clap_event_header {
                        size: mem::size_of::<clap_event_param_value>() as u32,
                        time,
                        space_id: CLAP_CORE_EVENT_SPACE_ID,
                        type_: CLAP_EVENT_PARAM_VALUE,
                        flags: CLAP_EVENT_IS_LIVE,
                    },
                    param_id: hash,
                    cookie: std::ptr::null_mut(),
                    note_id: -1,
                    port_index: -1,
                    channel: -1,
                    key: -1,
                    value: f64::from(value),
                };
                if !unsafe {
                    clap_call! { output=>try_push(output, &event.header) }
                } {
                    reject();
                }
                let end = gesture(hash, CLAP_EVENT_PARAM_GESTURE_END, time);
                ends[i] = !unsafe {
                    clap_call! { output=>try_push(output, &end.header) }
                };
                if ends[i] {
                    reject();
                }
            }
            // A successful old value can race restore acceptance AND its first
            // rescan while the host call is in flight. Retain a later rescan even
            // on success; a restore after this check sets the latch itself.
            if notification.id < mailbox.accepted_restore.load(Ordering::Acquire) {
                mailbox.dirty.store(true, Ordering::Release);
            }
        }
        self.configuration.lock().as_mut().unwrap().gesture_ends = ends;
        mailbox.gesture_debt.store(ends.into_iter().any(|end| end), Ordering::Release);
    }

    pub(super) fn configuration_main_thread(&self) {
        let dirty = self
            .configuration_mailbox
            .get()
            .is_some_and(|mailbox| mailbox.dirty.swap(false, Ordering::AcqRel));
        if dirty {
            let host_params =
                self.host_params.borrow().as_ref().map(|p| &**p as *const clap_host_params);
            let host_state =
                self.host_state.borrow().as_ref().map(|p| &**p as *const clap_host_state);
            if let Some(host_params) = host_params {
                unsafe_clap_call! { host_params=>rescan(&*self.host_callback, CLAP_PARAM_RESCAN_VALUES) };
                if self
                    .configuration_mailbox
                    .get()
                    .is_some_and(|m| m.gesture_debt.load(Ordering::Acquire))
                {
                    unsafe_clap_call! { host_params=>request_flush(&*self.host_callback) };
                }
            }
            if let Some(host_state) = host_state {
                unsafe_clap_call! { host_state=>mark_dirty(&*self.host_callback) };
            }
        }
    }

    pub(super) fn configuration_request_main(&self) {
        let dirty = self
            .configuration_mailbox
            .get()
            .is_some_and(|mailbox| mailbox.dirty.load(Ordering::Acquire));
        if dirty {
            unsafe {
                (self.host_callback.request_callback.unwrap())(&*self.host_callback);
            }
        }
    }

    pub(super) fn restore_configuration(&self, state: &mut PluginState) -> bool {
        // Clone only the mailbox, off audio. Static parsing under its independent
        // producer lock fixes restore order before a later UI edit can overtake it.
        let mailbox = self.configuration_mailbox.get().unwrap().clone();
        mailbox
            .restore(|| {
                let edit = P::clap_configuration_prepare(state)?;
                if edit.values.iter().flatten().any(|v| !v.is_finite()) {
                    return Err(SubmitError::Invalid);
                }
                let modulation = mailbox.published.load().modulation;
                let mut shadow = ConfigurationSnapshot {
                    payload: edit.payload,
                    modulation,
                    ..Default::default()
                };
                for (i, id) in P::CLAP_CONFIGURATION_PARAMS.iter().enumerate() {
                    let ptr = self.param_by_hash[&self.param_id_to_hash[*id]];
                    let value = edit.values[i].ok_or(SubmitError::Invalid)?;
                    shadow.unmodulated[i] = value;
                    shadow.normalized[i] =
                        (unsafe { ptr.preview_normalized(value) } + modulation[i]).clamp(0.0, 1.0);
                    shadow.raw[i] = unsafe { ptr.preview_plain(shadow.normalized[i]) };
                }
                shadow = P::clap_configuration_preview(shadow);
                for id in P::CLAP_CONFIGURATION_PARAMS {
                    state.params.remove(*id);
                }
                for field in P::CLAP_CONFIGURATION_FIELDS {
                    state.fields.remove(*field);
                }
                let success = unsafe {
                    state::deserialize_object::<P>(
                        state,
                        self.params.clone(),
                        state::make_params_getter(&self.param_by_hash, &self.param_id_to_hash),
                        self.current_buffer_config.load().as_ref(),
                    )
                };
                if !success {
                    return Err(SubmitError::Invalid);
                }
                Ok((edit, shadow))
            })
            .is_ok()
    }

    pub(super) fn overlay_configuration_state(&self, state: &mut PluginState) {
        if !P::CLAP_CONFIGURATION {
            return;
        }
        let mailbox = self.configuration_mailbox.get().unwrap().clone();
        let (snapshot, _) = mailbox.visible();
        for (i, id) in P::CLAP_CONFIGURATION_PARAMS.iter().enumerate() {
            state.params.insert(
                (*id).to_owned(),
                nice_plug_core::plugin::ParamValue::F32(snapshot.unmodulated[i]),
            );
        }
        P::clap_configuration_save(snapshot, state);
    }

    pub(super) fn configuration_value(&self, hash: u32) -> Option<f64> {
        if !P::CLAP_CONFIGURATION {
            return None;
        }
        let mailbox = self.configuration_mailbox.get()?;
        let index = P::CLAP_CONFIGURATION_PARAMS
            .iter()
            .position(|id| self.param_id_to_hash[*id] == hash)?;
        Some(f64::from(mailbox.visible().0.normalized[index]))
    }
}

/// One owned parameter attempt, with enough identity to finish the retained
/// transaction after dropping all runtime/plugin locks for the host callback.
#[derive(Clone, Copy)]
pub(super) struct NotificationAttempt {
    cell: Option<usize>,
    parameter: usize,
    phase: u8,
    hash: u32,
    value: f32,
    pub time: u32,
}
impl NotificationAttempt {
    pub unsafe fn push(self, output: &clap_output_events) -> bool {
        let header = |size, kind| clap_event_header {
            size,
            time: self.time,
            space_id: CLAP_CORE_EVENT_SPACE_ID,
            type_: kind,
            flags: CLAP_EVENT_IS_LIVE,
        };
        if self.phase == 1 {
            let event = clap_event_param_value {
                header: header(
                    mem::size_of::<clap_event_param_value>() as u32,
                    CLAP_EVENT_PARAM_VALUE,
                ),
                param_id: self.hash,
                cookie: std::ptr::null_mut(),
                note_id: -1,
                port_index: -1,
                channel: -1,
                key: -1,
                value: f64::from(self.value),
            };
            unsafe { (output.try_push.unwrap())(output, &event.header) }
        } else {
            let event = clap_event_param_gesture {
                header: header(
                    mem::size_of::<clap_event_param_gesture>() as u32,
                    if self.phase == 0 {
                        CLAP_EVENT_PARAM_GESTURE_BEGIN
                    } else {
                        CLAP_EVENT_PARAM_GESTURE_END
                    },
                ),
                param_id: self.hash,
            };
            unsafe { (output.try_push.unwrap())(output, &event.header) }
        }
    }
}
impl<P: ClapPlugin> Wrapper<P> {
    pub(super) fn begin_configuration_notifications(&self) {
        if let Some(runtime) = self.configuration.lock().as_mut() {
            runtime.notification_blocked.fill(false);
        }
    }
    pub(super) fn next_configuration_notification(
        &self,
        through: u32,
        cursor: u32,
    ) -> Option<NotificationAttempt> {
        let mut guard = self.configuration.lock();
        let r = guard.as_mut()?;
        // Finite cleanup of completed or superseded cells; a started gesture
        // still closes even if a newer accepted restore shadows its value.
        let restore = r.mailbox.accepted_restore.load(Ordering::Acquire);
        for cell in &mut r.notifications {
            if let Some(n) = cell {
                if n.id < restore {
                    if n.phase == 0 {
                        *cell = None;
                        continue;
                    }
                    n.phase = 2;
                }
                while n.parameter < CONFIG_PARAMETERS && n.values[n.parameter].is_none() {
                    n.parameter += 1;
                }
                if n.parameter == CONFIG_PARAMETERS {
                    *cell = None;
                }
            }
        }
        let ready = r
            .notifications
            .iter()
            .enumerate()
            .filter_map(|(i, n)| n.map(|n| (i, n)))
            .filter(|(_, n)| !r.notification_blocked[n.parameter])
            .filter(|(_, n)| {
                n.phase != 0
                    || !r
                        .notifications
                        .iter()
                        .flatten()
                        .any(|other| other.parameter == n.parameter && other.phase != 0)
            })
            .filter_map(|(i, n)| {
                let time = r
                    .output_boundary
                    .map_or(0, |(start, frames)| {
                        n.sample
                            .saturating_sub(start)
                            .max(0)
                            .min(i64::from(frames.saturating_sub(1))) as u32
                    })
                    .max(cursor);
                (time <= through).then_some((i, n, time))
            })
            .min_by_key(|(_, n, time)| (*time, n.sample, n.sequence));
        let closing_time = ready.map_or(cursor, |(_, _, time)| time);
        if let Some(i) =
            (0..CONFIG_PARAMETERS).find(|&i| r.gesture_ends[i] && !r.notification_blocked[i])
        {
            return Some(NotificationAttempt {
                cell: None,
                parameter: i,
                phase: 2,
                hash: r.hashes[i].unwrap(),
                value: 0.0,
                time: closing_time,
            });
        }
        let (i, n, time) = ready?;
        Some(NotificationAttempt {
            cell: Some(i),
            parameter: n.parameter,
            phase: n.phase,
            hash: r.hashes[n.parameter]?,
            value: n.values[n.parameter]?,
            time,
        })
    }
    pub(super) fn complete_configuration_notification(
        &self,
        attempt: NotificationAttempt,
        accepted: bool,
    ) {
        let mut guard = self.configuration.lock();
        let r = guard.as_mut().unwrap();
        if !accepted {
            r.mailbox.notification_rejected.store(true, Ordering::Release);
            r.mailbox.dirty.store(true, Ordering::Release);
        }
        if let Some(index) = attempt.cell {
            let n = r.notifications[index].as_mut().unwrap();
            match attempt.phase {
                0 if accepted => n.phase = 1,
                1 => n.phase = 2,
                _ => {
                    if attempt.phase == 2 && !accepted {
                        r.gesture_ends[attempt.parameter] = true;
                        r.notification_blocked[attempt.parameter] = true;
                    }
                    n.values[attempt.parameter] = None;
                    n.parameter += 1;
                    n.phase = 0;
                }
            }
            if n.id < r.mailbox.accepted_restore.load(Ordering::Acquire) {
                r.mailbox.dirty.store(true, Ordering::Release);
            }
        } else {
            r.gesture_ends[attempt.parameter] = !accepted;
            r.notification_blocked[attempt.parameter] = !accepted;
        }
        r.mailbox.gesture_debt.store(r.gesture_ends.iter().any(|e| *e), Ordering::Release);
    }
}
