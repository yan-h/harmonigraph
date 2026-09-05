//! Opt-in owned CLAP performance boundary. Musical retention, eligibility,
//! lifetime reservations and durable output journals belong to the plugin.
use super::configuration::InputValue;
use clap_sys::events::*;

pub const NORMAL_OUTPUT_ATTEMPTS: usize = 512;
pub const EMERGENCY_OUTPUT_ATTEMPTS: usize = 128;
pub const OUTPUT_CELLS: usize = NORMAL_OUTPUT_ATTEMPTS + EMERGENCY_OUTPUT_ATTEMPTS;

/// Exact transport value retained in the SAME input pool as note events.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Transport {
    pub event_flags: u32,
    pub flags: u32,
    pub song_pos_beats: i64,
    pub song_pos_seconds: i64,
    pub tempo: f64,
    pub tempo_inc: f64,
    pub loop_start_beats: i64,
    pub loop_end_beats: i64,
    pub loop_start_seconds: i64,
    pub loop_end_seconds: i64,
    pub bar_start: i64,
    pub bar_number: i32,
    pub tsig_num: u16,
    pub tsig_denom: u16,
}
impl From<clap_event_transport> for Transport {
    fn from(event: clap_event_transport) -> Self {
        Self {
            event_flags: event.header.flags,
            flags: event.flags,
            song_pos_beats: event.song_pos_beats,
            song_pos_seconds: event.song_pos_seconds,
            tempo: event.tempo,
            tempo_inc: event.tempo_inc,
            loop_start_beats: event.loop_start_beats,
            loop_end_beats: event.loop_end_beats,
            loop_start_seconds: event.loop_start_seconds,
            loop_end_seconds: event.loop_end_seconds,
            bar_start: event.bar_start,
            bar_number: event.bar_number,
            tsig_num: event.tsig_num,
            tsig_denom: event.tsig_denom,
        }
    }
}

impl Transport {
    pub(crate) fn event(self, time: u32) -> clap_event_transport {
        clap_event_transport {
            header: clap_event_header {
                size: std::mem::size_of::<clap_event_transport>() as u32,
                time,
                space_id: CLAP_CORE_EVENT_SPACE_ID,
                type_: CLAP_EVENT_TRANSPORT,
                flags: self.event_flags,
            },
            flags: self.flags,
            song_pos_beats: self.song_pos_beats,
            song_pos_seconds: self.song_pos_seconds,
            tempo: self.tempo,
            tempo_inc: self.tempo_inc,
            loop_start_beats: self.loop_start_beats,
            loop_end_beats: self.loop_end_beats,
            loop_start_seconds: self.loop_start_seconds,
            loop_end_seconds: self.loop_end_seconds,
            bar_start: self.bar_start,
            bar_number: self.bar_number,
            tsig_num: self.tsig_num,
            tsig_denom: self.tsig_denom,
        }
    }
}

/// Raw host observations, never framework-extrapolated song position.
#[derive(Clone, Copy, Debug)]
pub struct Callback {
    pub steady_time: i64,
    pub frames: u32,
    pub transport: Option<clap_event_transport>,
    pub input_status: InputStatus,
    pub output_available: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum InputStatus {
    Complete,
    Invalid,
    Full,
    Unsupported,
}

#[derive(Clone, Copy, Debug)]
pub struct Block {
    pub callback: Callback,
    pub start: u32,
    pub frames: u32,
    /// Latest raw observation and its original enclosing offset.
    pub transport: Option<clap_event_transport>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Consumption {
    Consumed,
    Pending,
}

/// Opaque caller identity. It must identify a unique staging attempt, including
/// retries, and contain no pointer or allocation whose lifetime ends on audio.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Token(pub [u64; 4]);

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Lane {
    Normal,
    Emergency,
}

#[derive(Clone, Copy, Debug, PartialEq)]
enum Events {
    Single(InputValue),
    Onset { note: InputValue, tuning: InputValue },
}

/// A single event or an onset plus same-address initial tuning. Construction
/// validates the value family; staging validates the enclosing output interval.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Group {
    pub token: Token,
    pub lane: Lane,
    pub time: u32,
    events: Events,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum StageError {
    Invalid,
    Full,
    Inhibited,
}

impl Group {
    pub fn single(
        token: Token,
        lane: Lane,
        time: u32,
        event: InputValue,
    ) -> Result<Self, StageError> {
        if !valid_event(event) {
            return Err(StageError::Invalid);
        }
        Ok(Self { token, lane, time, events: Events::Single(event) })
    }

    pub fn onset(
        token: Token,
        time: u32,
        note: InputValue,
        tuning: InputValue,
    ) -> Result<Self, StageError> {
        let matching = match (note, tuning) {
            (
                InputValue::Note { kind: CLAP_EVENT_NOTE_ON, note_id, port, channel, key, .. },
                InputValue::Expression {
                    expression: CLAP_NOTE_EXPRESSION_TUNING,
                    note_id: tid,
                    port: tp,
                    channel: tc,
                    key: tk,
                    ..
                },
            ) => (note_id, port, channel, key) == (tid, tp, tc, tk),
            _ => false,
        };
        if !matching || !valid_event(note) || !valid_event(tuning) {
            return Err(StageError::Invalid);
        }
        Ok(Self { token, lane: Lane::Normal, time, events: Events::Onset { note, tuning } })
    }

    pub fn event_count(&self) -> usize {
        match self.events {
            Events::Single(_) => 1,
            Events::Onset { .. } => 2,
        }
    }

    pub fn event(&self, index: usize) -> Option<InputValue> {
        match (self.events, index) {
            (Events::Single(e), 0)
            | (Events::Onset { note: e, .. }, 0)
            | (Events::Onset { tuning: e, .. }, 1) => Some(e),
            _ => None,
        }
    }
}

fn valid_event(event: InputValue) -> bool {
    match event {
        InputValue::Note { kind, velocity, .. } => {
            matches!(
                kind,
                CLAP_EVENT_NOTE_ON
                    | CLAP_EVENT_NOTE_OFF
                    | CLAP_EVENT_NOTE_CHOKE
                    | CLAP_EVENT_NOTE_END
            ) && velocity.is_finite()
        }
        InputValue::Expression { value, .. } => value.is_finite(),
        InputValue::Midi { .. } => true,
        _ => false,
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Disposition {
    Settled,
    Ineligible,
    Inhibited,
    MissingOutput,
    ProcessError,
}

/// Bit zero is the single/onset event, bit one the initial tuning. A rejected
/// onset leaves tuning unattempted. Accepted onset always attempts tuning under
/// the SAME caller permit, even if a fence closes inside the first host call.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Completion {
    pub group: Group,
    pub attempted: u8,
    pub accepted: u8,
    pub unattempted: u8,
    pub disposition: Disposition,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Summary {
    pub normal_attempts: usize,
    pub emergency_attempts: usize,
    pub completed_groups: usize,
    pub rejected: bool,
    pub legacy_send_misuse: bool,
    pub cursor: u32,
}

/// All storage is allocated once during wrapper construction. Each admitted
/// group keeps its cell through durable completion; pairs charge two event AND
/// completion credits atomically. Unattempted reservations are not recycled in
/// this callback, so repeatedly rejected claims cannot create unbounded work.
pub(crate) struct Scheduler {
    cells: Box<[Option<Group>; OUTPUT_CELLS]>,
    ready: [u16; OUTPUT_CELLS],
    ready_len: usize,
    used_cells: usize,
    normal_reserved: usize,
    emergency_reserved: usize,
    pub summary: Summary,
    pub frames: u32,
    pub inhibited: bool,
}

impl Default for Scheduler {
    fn default() -> Self {
        Self {
            cells: vec![None; OUTPUT_CELLS].into_boxed_slice().try_into().unwrap(),
            ready: [0; OUTPUT_CELLS],
            ready_len: 0,
            used_cells: 0,
            normal_reserved: 0,
            emergency_reserved: 0,
            summary: Summary::default(),
            frames: 0,
            inhibited: false,
        }
    }
}

impl Scheduler {
    pub fn begin(&mut self, frames: u32, inhibited: bool) {
        debug_assert!(self.cells.iter().all(Option::is_none));
        self.ready_len = 0;
        self.used_cells = 0;
        self.normal_reserved = 0;
        self.emergency_reserved = 0;
        self.summary = Summary::default();
        self.frames = frames;
        self.inhibited = inhibited;
    }

    pub fn writer(&mut self) -> Output<'_> {
        Output { scheduler: self }
    }

    pub fn next(&self, through: u32) -> Option<(usize, Group)> {
        if self.ready_len == 0 {
            return None;
        }
        let index = usize::from(self.ready[0]);
        let group = self.cells[index].unwrap();
        (group.time <= through).then_some((index, group))
    }

    fn key(&self, heap_index: usize) -> (u32, u16) {
        let cell = self.ready[heap_index];
        (self.cells[usize::from(cell)].unwrap().time, cell)
    }

    fn insert(&mut self, group: Group) {
        let cell = self.used_cells;
        self.used_cells += 1;
        self.cells[cell] = Some(group);
        let mut position = self.ready_len;
        self.ready_len += 1;
        self.ready[position] = cell as u16;
        while position > 0 {
            let parent = (position - 1) / 2;
            if self.key(parent) <= self.key(position) {
                break;
            }
            self.ready.swap(parent, position);
            position = parent;
        }
    }

    /// Remove from the ready heap before a completion can add earlier emergency
    /// work, but keep its event/completion cell reserved through that callback.
    pub fn begin_group(&mut self, index: usize) {
        debug_assert_eq!(usize::from(self.ready[0]), index);
        self.ready_len -= 1;
        if self.ready_len == 0 {
            return;
        }
        self.ready[0] = self.ready[self.ready_len];
        let mut position = 0;
        loop {
            let left = 2 * position + 1;
            if left >= self.ready_len {
                break;
            }
            let right = left + 1;
            let child = if right < self.ready_len && self.key(right) < self.key(left) {
                right
            } else {
                left
            };
            if self.key(position) <= self.key(child) {
                break;
            }
            self.ready.swap(position, child);
            position = child;
        }
    }

    pub fn complete(&mut self, index: usize) {
        self.cells[index] = None;
        self.summary.completed_groups += 1;
    }

    /// Parameters share ordinary reservation and attempt credits. They cannot
    /// consume the independent emergency allowance.
    pub fn reserve_parameter(&mut self) -> bool {
        if self.normal_reserved == NORMAL_OUTPUT_ATTEMPTS {
            return false;
        }
        self.normal_reserved += 1;
        true
    }

    pub fn attempted(&mut self, lane: Lane, time: u32, accepted: bool) {
        self.summary.cursor = time;
        match lane {
            Lane::Normal => self.summary.normal_attempts += 1,
            Lane::Emergency => self.summary.emergency_attempts += 1,
        }
        if !accepted {
            self.summary.rejected = true;
        }
    }
}

/// A short staging borrow, dropped before host calls. Rejected admission leaves
/// all caller retention/credits untouched. Caller must stage chronologically
/// within a sub-block's horizon; future sub-block output is retained locally.
pub struct Output<'a> {
    scheduler: &'a mut Scheduler,
}

impl Output<'_> {
    pub fn cursor(&self) -> u32 {
        self.scheduler.summary.cursor
    }

    pub fn stage(&mut self, group: Group) -> Result<(), StageError> {
        let s = &mut self.scheduler;
        if group.time < s.summary.cursor || group.time >= s.frames {
            return Err(StageError::Invalid);
        }
        let (reserved, limit) = match group.lane {
            Lane::Normal => {
                if s.inhibited {
                    return Err(StageError::Inhibited);
                }
                (&mut s.normal_reserved, NORMAL_OUTPUT_ATTEMPTS)
            }
            Lane::Emergency => (&mut s.emergency_reserved, EMERGENCY_OUTPUT_ATTEMPTS),
        };
        if *reserved + group.event_count() > limit {
            return Err(StageError::Full);
        }
        if s.used_cells == OUTPUT_CELLS {
            return Err(StageError::Full);
        }
        *reserved += group.event_count();
        s.insert(group);
        Ok(())
    }
}

pub(crate) unsafe fn push_value(output: &clap_output_events, value: InputValue, time: u32) -> bool {
    let header = |size, kind, flags| clap_event_header {
        size,
        time,
        space_id: CLAP_CORE_EVENT_SPACE_ID,
        type_: kind,
        flags,
    };
    match value {
        InputValue::Note { kind, note_id, port, channel, key, velocity, flags } => {
            let event = clap_event_note {
                header: header(std::mem::size_of::<clap_event_note>() as u32, kind, flags),
                note_id,
                port_index: port,
                channel,
                key,
                velocity,
            };
            unsafe { (output.try_push.unwrap())(output, &event.header) }
        }
        InputValue::Expression { expression, note_id, port, channel, key, value, flags } => {
            let event = clap_event_note_expression {
                header: header(
                    std::mem::size_of::<clap_event_note_expression>() as u32,
                    CLAP_EVENT_NOTE_EXPRESSION,
                    flags,
                ),
                expression_id: expression,
                note_id,
                port_index: port,
                channel,
                key,
                value,
            };
            unsafe { (output.try_push.unwrap())(output, &event.header) }
        }
        InputValue::Midi { port, data, flags } => {
            let event = clap_event_midi {
                header: header(
                    std::mem::size_of::<clap_event_midi>() as u32,
                    CLAP_EVENT_MIDI,
                    flags,
                ),
                port_index: port,
                data,
            };
            unsafe { (output.try_push.unwrap())(output, &event.header) }
        }
        _ => unreachable!("validated output group"),
    }
}

const _: () = assert!(std::mem::size_of::<Option<Group>>() <= 256);

const _: () = assert!(std::mem::size_of::<Completion>() <= 256);

const _: () = assert!(std::mem::size_of::<Option<Group>>() + std::mem::size_of::<u16>() <= 256);
