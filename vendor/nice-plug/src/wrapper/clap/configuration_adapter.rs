//! Block-granular configuration ownership sharing the acknowledged owned input
//! pool with the independently opted-in performance boundary. Every accepted
//! edit reduces in arrival order; the plugin decides when a reduced value
//! becomes effective, which is the next callback boundary.
use super::*;
use crate::wrapper::clap::configuration::*;

pub(super) struct Runtime {
    pub mailbox: Arc<ConfigurationMailbox>,
    commands: rtrb::Consumer<ConfigurationCommand>,
    hashes: [Option<u32>; CONFIG_PARAMETERS],
    base: [f32; CONFIG_PARAMETERS],
    modulation: [f32; CONFIG_PARAMETERS],
    applied_id: u64,
    notifications: [Option<Notification>; 16],
    fault: bool,
    reset_generation: u64,
    output_boundary: Option<(i64, u32)>,
    gesture_ends: [bool; CONFIG_PARAMETERS],
    notification_sequence: u64,
    notification_blocked: [bool; CONFIG_PARAMETERS],
    notification_blocked_at: [u32; CONFIG_PARAMETERS],
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

    pub(super) fn retire_configuration(&self) {
        let runtime = self.configuration.lock().take();
        if let Some(runtime) = runtime {
            let unfinished = !runtime.commands.is_empty()
                || self.owned_input.lock().as_ref().is_some_and(|input| input.get(0).is_some());
            // All callbacks have joined. Dropping these original owners is an
            // explicit disposal, with no guessed application or completion cut.
            self.plugin.lock().clap_configuration_retire(unfinished);
            drop(runtime);
        }
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
            notifications: [None; 16],
            fault: false,
            reset_generation: 0,
            output_boundary: None,
            gesture_ends: [false; CONFIG_PARAMETERS],
            notification_sequence: 0,
            notification_blocked: [false; CONFIG_PARAMETERS],
            notification_blocked_at: [0; CONFIG_PARAMETERS],
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

    /// Every command accepted before this callback, applied at its boundary.
    /// There is no per-command observation sample: an edit that arrives while a
    /// block is in flight is simply drained by the next one.
    fn drain_configuration_commands(
        &self,
        runtime: &mut Runtime,
        plugin: &mut P,
        sample: i64,
    ) -> bool {
        while let Ok(&command) = runtime.commands.peek() {
            if !self.apply_configuration(runtime, plugin, command, sample, None) {
                return false;
            }
            runtime.commands.pop().expect("peeked configuration command");
        }
        true
    }

    pub(super) fn reset_configuration_walk(&self) {
        if let Some(runtime) = self.configuration.lock().as_mut() {
            runtime.fault = false;
        }
    }

    pub(super) fn prepare_configuration_capture(
        &self,
        input: &mut input_adapter::Runtime,
        boundary: Option<(i64, u32)>,
    ) {
        let mut guard = self.configuration.lock();
        let Some(runtime) = guard.as_mut() else {
            return;
        };
        let reset = runtime.mailbox.reset_generation.load(Ordering::Acquire);
        if reset != runtime.reset_generation {
            input.reset();
            runtime.fault = false;
            runtime.reset_generation = reset;
        }
        runtime.output_boundary = boundary;
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
            runtime.fault = false;
            runtime.reset_generation = reset;
        }
        plugin.clap_configuration_begin(boundary);
        if boundary.steady_time < 0 || runtime.fault {
            plugin.clap_configuration_fault();
            return;
        }
        if !self.drain_configuration_commands(runtime, &mut plugin, boundary.steady_time) {
            return;
        }
        // THE configuration boundary, once every command accepted before this
        // callback has reduced: one value for every group this block starts.
        plugin.clap_configuration_adopt();
        input.storage.bind_untimed(boundary.steady_time);
        // One walk, in input order, with each event's sample-precise timestamp
        // preserved. Automation reduces where it arrives and becomes effective
        // at the next boundary, exactly like a queued UI edit.
        while let Some(event) = input.get(0) {
            if let InputValue::Parameter { id, value, modulation } = event.value {
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
                            self.param_by_hash[&id].preview_plain((value as f32).clamp(0.0, 1.0))
                        });
                    }
                    let command = ConfigurationCommand {
                        id: 0,
                        origin: if event.flush {
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
                        event.sample.unwrap(),
                        offset,
                    ) {
                        return;
                    }
                }
            }
            plugin.clap_configuration_observe(event);
            input.ack_configuration(P::CLAP_PERFORMANCE);
        }
        // Learning reads the whole block's evidence and lands at the next
        // boundary with everything else.
        if let Some(edit) = plugin.clap_configuration_group_end(boundary.steady_time) {
            self.apply_configuration(
                runtime,
                &mut plugin,
                ConfigurationCommand { id: 0, origin: ConfigurationOrigin::Learning, edit },
                boundary.steady_time,
                None,
            );
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
        // A rejected closure may retry at the next distinct timed value for
        // that parameter. Blocking it for the whole performance callback lets
        // a later note advance the shared cursor past that value. One retry per
        // original timestamp remains bounded by the shared output allowance.
        for parameter in 0..CONFIG_PARAMETERS {
            if r.notification_blocked[parameter] && r.notifications.iter().flatten().any(|n| {
                if n.parameter != parameter { return false; }
                let time = r.output_boundary.map_or(0, |(start, frames)| n.sample.saturating_sub(start)
                    .max(0).min(i64::from(frames.saturating_sub(1))) as u32).max(cursor);
                time > r.notification_blocked_at[parameter] && time <= through
            }) { r.notification_blocked[parameter] = false; }
        }
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
            r.notification_blocked_at[attempt.parameter] = attempt.time;
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
