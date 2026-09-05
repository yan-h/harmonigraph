//! Opt-in, fixed-value configuration ingress. The plugin owns musical reduction;
//! this module owns off-thread transactions, prepared restore slots and input
//! retention. It carries no assignment policy or performance output emitter.

use parking_lot::Mutex;
use std::cell::UnsafeCell;
use std::sync::atomic::{AtomicBool, AtomicU64, AtomicU8, Ordering};
use std::sync::Arc;

pub const CONFIG_COMMANDS: usize = 128;
pub const CONFIG_SLOTS: usize = 2;
pub const INPUT_SCAN: usize = 2048;
pub const CONFIG_PARAMETERS: usize = 5;
pub const PAYLOAD_WORDS: usize = 16;

/// Plugin-defined fixed semantic payload plus one atomic parameter-write batch.
/// Values are ordinary unmodulated plugin units, not CLAP's normalized units.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct ConfigurationEdit {
    pub values: [Option<f32>; CONFIG_PARAMETERS],
    pub payload: [i32; PAYLOAD_WORDS],
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ConfigurationOrigin {
    Ui,
    Restore,
    Automation,
    Flush,
    Learning,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ConfigurationCommand {
    pub id: u64,
    pub origin: ConfigurationOrigin,
    pub edit: ConfigurationEdit,
}

/// A coherent applied value. The two numeric forms have distinct consumers:
/// host save uses unmodulated plain; get_value uses modulated normalized values.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct ConfigurationSnapshot {
    pub applied_id: u64,
    pub revision: u64,
    pub effective_sample: i64,
    pub raw: [f32; CONFIG_PARAMETERS],
    pub unmodulated: [f32; CONFIG_PARAMETERS],
    pub normalized: [f32; CONFIG_PARAMETERS],
    /// Exact transient host offset; never infer it by subtracting clamped values.
    pub modulation: [f32; CONFIG_PARAMETERS],
    pub payload: [i32; PAYLOAD_WORDS],
    pub status: u32,
}

#[derive(Clone, Copy, Debug)]
pub struct ConfigurationBoundary {
    pub steady_time: i64,
    pub frames: u32,
    pub sample_rate: f32,
    pub transport_seconds: Option<f64>,
    pub playing: bool,
}

#[derive(Clone, Copy, Debug)]
pub struct ConfigurationCommit {
    pub sample: i64,
    pub modulation: [f32; CONFIG_PARAMETERS],
    pub raw: [f32; CONFIG_PARAMETERS],
    pub unmodulated: [f32; CONFIG_PARAMETERS],
    pub normalized: [f32; CONFIG_PARAMETERS],
}

// Atomic payload words, not a racing ordinary/volatile struct copy. Only the
// serialized audio owner publishes. Non-RT readers may retry; audio never does.
pub struct PublishedConfiguration {
    sequence: AtomicU64,
    words: [AtomicU64; 40],
}
impl Default for PublishedConfiguration {
    fn default() -> Self {
        Self { sequence: AtomicU64::new(0), words: std::array::from_fn(|_| AtomicU64::new(0)) }
    }
}
impl PublishedConfiguration {
    pub fn publish(&self, value: ConfigurationSnapshot) {
        let sequence = self.sequence.load(Ordering::Relaxed);
        // Counter exhaustion is a visible protocol failure; never wrap a reader's
        // validation sequence. This requires reinitialization after ~2^63 writes.
        if sequence > u64::MAX - 2 {
            self.words[39].fetch_or(2, Ordering::SeqCst);
            return;
        }
        self.sequence.store(sequence + 1, Ordering::SeqCst);
        let mut words = [0; 40];
        words[0] = value.applied_id;
        words[1] = value.revision;
        words[2] = value.effective_sample as u64;
        for (i, v) in value
            .raw
            .into_iter()
            .chain(value.unmodulated)
            .chain(value.normalized)
            .chain(value.modulation)
            .enumerate()
        {
            words[3 + i] = u64::from(v.to_bits());
        }
        for (i, v) in value.payload.into_iter().enumerate() {
            words[23 + i] = v as u32 as u64;
        }
        words[39] = u64::from(value.status);
        for (cell, word) in self.words.iter().zip(words) {
            cell.store(word, Ordering::SeqCst);
        }
        self.sequence.store(sequence + 2, Ordering::SeqCst);
    }

    /// Off-thread only. All payload accesses are atomic even on failed reads.
    pub fn load(&self) -> ConfigurationSnapshot {
        loop {
            let before = self.sequence.load(Ordering::SeqCst);
            if before & 1 != 0 {
                std::hint::spin_loop();
                continue;
            }
            let words = self.words.each_ref().map(|word| word.load(Ordering::SeqCst));
            if before != self.sequence.load(Ordering::SeqCst) {
                continue;
            }
            return ConfigurationSnapshot {
                applied_id: words[0],
                revision: words[1],
                effective_sample: words[2] as i64,
                raw: std::array::from_fn(|i| f32::from_bits(words[3 + i] as u32)),
                unmodulated: std::array::from_fn(|i| f32::from_bits(words[8 + i] as u32)),
                normalized: std::array::from_fn(|i| f32::from_bits(words[13 + i] as u32)),
                modulation: std::array::from_fn(|i| f32::from_bits(words[18 + i] as u32)),
                payload: std::array::from_fn(|i| words[23 + i] as u32 as i32),
                status: words[39] as u32,
            };
        }
    }
}

const EMPTY: u8 = 0;
const WRITING: u8 = 1;
const READY: u8 = 2;
const READING: u8 = 3;
const DONE: u8 = 4;

struct RestoreSlot {
    state: AtomicU8,
    value: UnsafeCell<ConfigurationCommand>,
}
// The one producer writes only WRITING after acquiring EMPTY/DONE. The one
// consumer copies only READING after acquiring READY. Release transitions
// publish the payload and exclude reuse until the consumer has finished copying.
unsafe impl Sync for RestoreSlot {}
impl RestoreSlot {
    fn new() -> Self {
        Self {
            state: AtomicU8::new(EMPTY),
            value: UnsafeCell::new(ConfigurationCommand {
                id: 0,
                origin: ConfigurationOrigin::Restore,
                edit: ConfigurationEdit::default(),
            }),
        }
    }
}

#[derive(Clone, Copy, Debug)]
pub(crate) enum QueuedConfiguration {
    Edit(ConfigurationCommand),
    Restore { slot: u8, id: u64 },
}

struct Producer {
    queue: rtrb::Producer<QueuedConfiguration>,
    next_id: u64,
    /// Accepted host restore intent, not another resolver. Retired only after an
    /// applied owner publication includes this restore, never on queue transfer.
    shadow: Option<ConfigurationSnapshot>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SubmitError {
    Full,
    Invalid,
    CounterExhausted,
}

pub struct ConfigurationMailbox {
    producer: Mutex<Producer>,
    slots: [RestoreSlot; CONFIG_SLOTS],
    pub published: PublishedConfiguration,
    pub dirty: AtomicBool,
    pub notification_rejected: AtomicBool,
    pub gesture_debt: AtomicBool,
    submission_rejected: AtomicBool,
    pub reset_generation: AtomicU64,
    /// Unsent older UI value notifications may not overwrite a newer accepted
    /// restore in the host cache. Its shadow also overlays readback and save.
    pub accepted_restore: AtomicU64,
    pub accepted_command: AtomicU64,
    notify: Box<dyn Fn() + Send + Sync>,
}

impl ConfigurationMailbox {
    pub(crate) fn new(
        notify: Box<dyn Fn() + Send + Sync>,
    ) -> (Arc<Self>, rtrb::Consumer<QueuedConfiguration>) {
        let (queue, consumer) = rtrb::RingBuffer::new(CONFIG_COMMANDS);
        (
            Arc::new(Self {
                producer: Mutex::new(Producer { queue, next_id: 1, shadow: None }),
                slots: std::array::from_fn(|_| RestoreSlot::new()),
                published: PublishedConfiguration::default(),
                dirty: AtomicBool::new(false),
                notification_rejected: AtomicBool::new(false),
                gesture_debt: AtomicBool::new(false),
                submission_rejected: AtomicBool::new(false),
                reset_generation: AtomicU64::new(0),
                accepted_restore: AtomicU64::new(0),
                accepted_command: AtomicU64::new(0),
                notify,
            }),
            consumer,
        )
    }

    /// Main/editor thread only; a single producer mutex serializes UI, restore
    /// and future nonparameter edits. No audio caller takes this mutex.
    pub fn submit(&self, edit: ConfigurationEdit) -> Result<u64, SubmitError> {
        if edit.values.iter().flatten().any(|v| !v.is_finite()) {
            return Err(SubmitError::Invalid);
        }
        let mut producer = self.producer.lock();
        let id = producer.next_id;
        let next = id.checked_add(1).ok_or(SubmitError::CounterExhausted)?;
        producer
            .queue
            .push(QueuedConfiguration::Edit(ConfigurationCommand {
                id,
                origin: ConfigurationOrigin::Ui,
                edit,
            }))
            .map_err(|_| {
                self.submission_rejected.store(true, Ordering::Release);
                SubmitError::Full
            })?;
        self.submission_rejected.store(false, Ordering::Release);
        producer.next_id = next;
        self.accepted_command.store(id, Ordering::Release);
        drop(producer);
        (self.notify)();
        Ok(id)
    }

    /// Preparation and generic visual restore happen off audio in `apply_visual`.
    /// Reserve both resources first; failure changes no accepted musical state.
    pub(crate) fn restore(
        &self,
        prepare_and_apply: impl FnOnce() -> Result<
            (ConfigurationEdit, ConfigurationSnapshot),
            SubmitError,
        >,
    ) -> Result<u64, SubmitError> {
        let mut producer = self.producer.lock();
        if producer.queue.slots() == 0 {
            return Err(SubmitError::Full);
        }
        let id = producer.next_id;
        let next = id.checked_add(1).ok_or(SubmitError::CounterExhausted)?;
        let slot_index = self
            .slots
            .iter()
            .position(|slot| {
                let state = slot.state.load(Ordering::Acquire);
                (state == EMPTY || state == DONE)
                    && slot
                        .state
                        .compare_exchange(state, WRITING, Ordering::Acquire, Ordering::Relaxed)
                        .is_ok()
            })
            .ok_or(SubmitError::Full)?;
        let slot = &self.slots[slot_index];
        let (edit, mut shadow) = match prepare_and_apply() {
            Ok(value) => value,
            Err(error) => {
                slot.state.store(EMPTY, Ordering::Release);
                return Err(error);
            }
        };
        let command = ConfigurationCommand { id, origin: ConfigurationOrigin::Restore, edit };
        // SAFETY: producer owns WRITING; the consumer cannot access this payload.
        unsafe {
            *slot.value.get() = command;
        }
        shadow.applied_id = id;
        producer.shadow = Some(shadow);
        self.accepted_restore.store(id, Ordering::Release);
        self.dirty.store(true, Ordering::Release);
        slot.state.store(READY, Ordering::Release);
        // Reserved under the sole producer lock. A consumer can only free space.
        producer
            .queue
            .push(QueuedConfiguration::Restore { slot: slot_index as u8, id })
            .expect("reserved configuration command slot");
        producer.next_id = next;
        self.accepted_command.store(id, Ordering::Release);
        drop(producer);
        (self.notify)();
        Ok(id)
    }

    pub fn visible(&self) -> (ConfigurationSnapshot, bool) {
        let mut producer = self.producer.lock();
        let mut applied = self.published.load();
        if self.notification_rejected.load(Ordering::Acquire) {
            applied.status |= 4;
        }
        if self.submission_rejected.load(Ordering::Acquire) {
            applied.status |= 8;
        }
        if let Some(mut shadow) = producer.shadow {
            if applied.applied_id < shadow.applied_id {
                shadow.status |= applied.status;
                return (shadow, true);
            }
            producer.shadow = None;
        }
        (applied, self.accepted_command.load(Ordering::Acquire) > applied.applied_id)
    }

    pub(crate) fn command(&self, queued: QueuedConfiguration) -> ConfigurationCommand {
        match queued {
            QueuedConfiguration::Edit(command) => command,
            QueuedConfiguration::Restore { slot, id } => {
                let slot = &self.slots[usize::from(slot)];
                if slot.state.load(Ordering::Relaxed) == READY {
                    slot.state
                        .compare_exchange(READY, READING, Ordering::Acquire, Ordering::Relaxed)
                        .expect("single restore consumer");
                }
                // SAFETY: this consumer owns READING until `retained` is called.
                let command = unsafe { *slot.value.get() };
                assert_eq!(command.id, id);
                command
            }
        }
    }

    pub(crate) fn retained(&self, queued: QueuedConfiguration) {
        if let QueuedConfiguration::Restore { slot, .. } = queued {
            self.slots[usize::from(slot)].state.store(DONE, Ordering::Release);
        }
    }
}

/// Exact owned CLAP input values, shared with the later performance adapter.
/// Signed wildcard addresses, event flags and f64 tuning survive capture.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum InputValue {
    Parameter {
        id: u32,
        value: f64,
        modulation: bool,
    },
    Note {
        kind: u16,
        note_id: i32,
        port: i16,
        channel: i16,
        key: i16,
        velocity: f64,
        flags: u32,
    },
    Expression {
        expression: i32,
        note_id: i32,
        port: i16,
        channel: i16,
        key: i16,
        value: f64,
        flags: u32,
    },
    Midi {
        port: u16,
        data: [u8; 3],
        flags: u32,
    },
    /// Full raw transport payload, including flags and fixed-point timelines.
    Transport(super::performance::Transport),
    /// Unsupported header: never a claim that its payload was retained.
    Other,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct OwnedInput {
    /// None only for untimed params.flush. Bound once at the next process boundary.
    pub sample: Option<i64>,
    pub event_index: u32,
    /// Original enclosing offset; never rewritten when retained or bound.
    pub offset: u32,
    pub enclosing_start: Option<i64>,
    pub enclosing_frames: u32,
    pub flush: bool,
    pub command_cut: u64,
    pub command_sample: Option<i64>,
    pub batch: u64,
    pub value: InputValue,
}

/// The single budgeted persistent input allocation. It is not rebuilt on Rust
/// sub-blocks and never retains a borrowed host pointer across a callback.
pub(crate) struct InputStorage {
    cells: Box<[Option<OwnedInput>; INPUT_SCAN]>,
    head: usize,
    len: usize,
}
impl Default for InputStorage {
    fn default() -> Self {
        Self { cells: vec![None; INPUT_SCAN].into_boxed_slice().try_into().unwrap(), head: 0, len: 0 }
    }
}
impl InputStorage {
    pub fn len(&self) -> usize {
        self.len
    }
    pub fn available(&self) -> usize {
        INPUT_SCAN - self.len
    }
    pub fn push(&mut self, input: OwnedInput) -> Result<(), SubmitError> {
        if self.len == INPUT_SCAN {
            return Err(SubmitError::Full);
        }
        self.cells[(self.head + self.len) % INPUT_SCAN] = Some(input);
        self.len += 1;
        Ok(())
    }
    pub fn get(&self, index: usize) -> Option<OwnedInput> {
        if index >= self.len {
            return None;
        }
        self.cells[(self.head + index) % INPUT_SCAN]
    }
    pub fn pop(&mut self) -> Option<OwnedInput> {
        if self.len == 0 {
            return None;
        }
        let value = self.cells[self.head].take();
        self.head = (self.head + 1) % INPUT_SCAN;
        self.len -= 1;
        value
    }
    pub fn truncate(&mut self, len: usize) {
        while self.len > len {
            self.len -= 1;
            self.cells[(self.head + self.len) % INPUT_SCAN] = None;
        }
    }
    pub fn reset_timed(&mut self) {
        let count = self.len;
        for _ in 0..count {
            let input = self.pop().unwrap();
            if input.sample.is_none() {
                self.push(input).expect("retained untimed flush");
            }
        }
    }

    pub fn bind_untimed(&mut self, boundary: i64) {
        for i in 0..self.len {
            let input = self.cells[(self.head + i) % INPUT_SCAN].as_mut().unwrap();
            if input.sample.is_none() {
                input.sample = Some(boundary);
            }
            if input.command_sample.is_none() {
                input.command_sample = Some(boundary);
            }
        }
    }
}

const _: () = assert!(std::mem::size_of::<ConfigurationCommand>() <= 256);
const _: () = assert!(std::mem::size_of::<Option<OwnedInput>>() <= 192);
const _: () = assert!(std::mem::align_of::<OwnedInput>() <= 8);
