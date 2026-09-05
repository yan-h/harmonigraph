//! Narrow CLAP adapter for ordered configuration ownership. Legacy performance
//! forwarding remains in the wrapper; this fixed input allocation is reused by
//! the later owned performance boundary, which has not been implemented here.
use super::*;
use crate::wrapper::clap::configuration::*;

pub(super) struct Runtime {
    pub mailbox: Arc<ConfigurationMailbox>,
    commands: rtrb::Consumer<QueuedConfiguration>,
    input: InputStorage,
    hashes: [Option<u32>; CONFIG_PARAMETERS],
    base: [f32; CONFIG_PARAMETERS],
    modulation: [f32; CONFIG_PARAMETERS],
    applied_id: u64,
    batch: u64,
    group: Option<Group>,
    notifications: [Option<Notification>; 16],
    fault: bool,
    reset_generation: u64,
    pending_learning: Option<(i64, ConfigurationEdit)>,
    observed_commands: Option<(u64, i64)>,
    callback_cut: u64,
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
    values: [Option<f32>; CONFIG_PARAMETERS],
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
                if let Some(host_params) = wrapper.host_params.borrow().as_ref() {
                    unsafe_clap_call! { host_params=>request_flush(&*wrapper.host_callback) };
                }
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
            input: InputStorage::default(),
            hashes,
            base,
            modulation: [0.0; CONFIG_PARAMETERS],
            applied_id: 0,
            batch: 0,
            group: None,
            notifications: [None; 16],
            fault: false,
            reset_generation: 0,
            pending_learning: None,
            observed_commands: None,
            callback_cut: 0,
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
                *cell = Some(Notification { id: runtime.applied_id, values });
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
        // Preserve a first-observed boundary across callback budget exhaustion.
        // New commands after this cut are observed at a subsequent boundary.
        if let Some((previous_cut, previous_sample)) = runtime.observed_commands {
            if !self.apply_configuration_cut(runtime, plugin, previous_cut, previous_sample) {
                return false;
            }
            runtime.observed_commands = None;
        }
        runtime.observed_commands = Some((cut, sample));
        if !self.apply_configuration_cut(runtime, plugin, cut, sample) {
            return false;
        }
        runtime.observed_commands = None;
        true
    }

    fn apply_configuration_cut(
        &self,
        runtime: &mut Runtime,
        plugin: &mut P,
        cut: u64,
        sample: i64,
    ) -> bool {
        while let Ok(&queued) = runtime.commands.peek() {
            let command = runtime.mailbox.command(queued);
            if command.id > cut {
                break;
            }
            if !self.apply_configuration(runtime, plugin, command, sample, None) {
                return false;
            }
            runtime.commands.pop().expect("peeked configuration command");
            runtime.mailbox.retained(queued);
        }
        true
    }

    /// Capture once per enclosing process/flush, before the legacy event walker
    /// can overwrite parameters. Count ALL host events before conversion/growth.
    pub(super) unsafe fn capture_configuration(
        &self,
        input: *const clap_input_events,
        boundary: Option<(i64, u32)>,
    ) -> bool {
        let mut guard = self.configuration.lock();
        let Some(runtime) = guard.as_mut() else {
            return true;
        };
        let reset = runtime.mailbox.reset_generation.load(Ordering::Acquire);
        if reset != runtime.reset_generation {
            runtime.input.reset_timed();
            runtime.group = None;
            runtime.pending_learning = None;
            runtime.observed_commands = None;
            runtime.fault = false;
            runtime.reset_generation = reset;
        }
        let count = if input.is_null() {
            0
        } else {
            unsafe {
                clap_call! { input=>size(input) }
            }
        } as usize;
        if count > INPUT_SCAN || count > runtime.input.available() {
            runtime.fault = true;
            self.plugin.lock().clap_configuration_fault();
            return false;
        }
        let Some(batch) = runtime.batch.checked_add(1) else {
            runtime.fault = true;
            return false;
        };
        runtime.batch = batch;
        let cut = runtime.mailbox.accepted_command.load(Ordering::Acquire);
        runtime.callback_cut = cut;
        let mut previous_time = 0;
        for index in 0..count {
            let pointer = unsafe {
                clap_call! { input=>get(input, index as u32) }
            };
            let header = unsafe { &*pointer };
            let sample = if let Some((start, frames)) = boundary {
                if header.time >= frames || (index > 0 && header.time < previous_time) {
                    runtime.fault = true;
                    self.plugin.lock().clap_configuration_fault();
                    return false;
                }
                let Some(sample) = start.checked_add(i64::from(header.time)) else {
                    runtime.fault = true;
                    return false;
                };
                Some(sample)
            } else {
                None
            };
            previous_time = header.time;
            let value = if header.space_id != CLAP_CORE_EVENT_SPACE_ID {
                InputValue::Other
            } else {
                match header.type_ {
                    CLAP_EVENT_PARAM_VALUE => {
                        let event = unsafe { &*pointer.cast::<clap_event_param_value>() };
                        InputValue::Parameter {
                            id: event.param_id,
                            value: event.value,
                            modulation: false,
                        }
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
                    CLAP_EVENT_NOTE_ON | CLAP_EVENT_NOTE_OFF | CLAP_EVENT_NOTE_CHOKE => {
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
                        InputValue::Midi {
                            port: event.port_index,
                            data: event.data,
                            flags: header.flags,
                        }
                    }
                    _ => InputValue::Other,
                }
            };
            runtime
                .input
                .push(OwnedInput {
                    sample,
                    event_index: index as u32,
                    flush: boundary.is_none(),
                    command_cut: cut,
                    command_sample: boundary.map(|b| b.0),
                    batch,
                    value,
                })
                .expect("reserved input capacity");
        }
        true
    }

    pub(super) fn process_configuration(&self, boundary: ConfigurationBoundary) {
        let mut guard = self.configuration.lock();
        let Some(runtime) = guard.as_mut() else {
            return;
        };
        let mut plugin = self.plugin.lock();
        let reset = runtime.mailbox.reset_generation.load(Ordering::Acquire);
        if reset != runtime.reset_generation {
            runtime.input.reset_timed();
            runtime.group = None;
            runtime.pending_learning = None;
            runtime.observed_commands = None;
            runtime.fault = false;
            runtime.reset_generation = reset;
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
        runtime.input.bind_untimed(boundary.steady_time);
        loop {
            let Some(first) = runtime.input.get(0) else {
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
                let count = (0..runtime.input.len())
                    .take_while(|&i| {
                        runtime.input.get(i).is_some_and(|input| {
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
                let input = runtime.input.get(group.parameter_cursor).unwrap();
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
                plugin.clap_configuration_observe(runtime.input.get(cursor).unwrap());
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
                runtime.input.pop();
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

    pub(super) unsafe fn notify_configuration(&self, output: &clap_output_events, time: u32) {
        // Own notification values by copy and drop runtime/plugin borrows before
        // host calls. Rejection does not undo an already applied local command.
        let (notifications, hashes, restore_cut) = {
            let mut guard = self.configuration.lock();
            let Some(runtime) = guard.as_mut() else {
                return;
            };
            (
                std::mem::replace(&mut runtime.notifications, [None; 16]),
                runtime.hashes,
                runtime.mailbox.accepted_restore.load(Ordering::Acquire),
            )
        };
        for notification in notifications.into_iter().flatten() {
            if notification.id < restore_cut {
                continue;
            }
            for (i, value) in notification.values.into_iter().enumerate() {
                let (Some(value), Some(hash)) = (value, hashes[i]) else {
                    continue;
                };
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
                let gesture = |kind| clap_event_param_gesture {
                    header: clap_event_header {
                        size: mem::size_of::<clap_event_param_gesture>() as u32,
                        type_: kind,
                        ..event.header
                    },
                    param_id: hash,
                };
                let begin = gesture(CLAP_EVENT_PARAM_GESTURE_BEGIN);
                let end = gesture(CLAP_EVENT_PARAM_GESTURE_END);
                let began = unsafe {
                    clap_call! { output=>try_push(output, &begin.header) }
                };
                let accepted = unsafe {
                    clap_call! { output=>try_push(output, &event.header) }
                };
                let ended = unsafe {
                    clap_call! { output=>try_push(output, &end.header) }
                };
                if !(began && accepted && ended) {
                    self.configuration_mailbox
                        .get()
                        .unwrap()
                        .notification_rejected
                        .store(true, Ordering::Release);
                    // Host cache rescan/dirty remains latched independently of its
                    // event sink. No same-value input can acknowledge this event.
                    self.configuration_mailbox.get().unwrap().dirty.store(true, Ordering::Release);
                }
            }
        }
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
                ((*self.host_callback).request_callback.unwrap())(&*self.host_callback);
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
