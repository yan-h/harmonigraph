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

/// Opaque caller identity. It must identify a unique staging attempt, including
/// retries, and contain no pointer or allocation whose lifetime ends on audio.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Token(pub [u64; 4]);

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Lane {
    Normal,
    Emergency,
}

/// The output-only subset of [`InputValue`]. Keeping the retained form narrow
/// lets one preparation own a short sequence without paying for three copies
/// of the much larger transport-capable input enum in every scheduler cell.
#[derive(Clone, Copy, Debug, PartialEq)]
enum OutputValue {
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
}

impl OutputValue {
    fn new(event: InputValue) -> Result<Self, StageError> {
        Ok(match event {
            InputValue::Note { kind, note_id, port, channel, key, velocity, flags }
                if matches!(
                    kind,
                    CLAP_EVENT_NOTE_ON
                        | CLAP_EVENT_NOTE_OFF
                        | CLAP_EVENT_NOTE_CHOKE
                        | CLAP_EVENT_NOTE_END
                ) && velocity.is_finite() =>
            {
                Self::Note { kind, note_id, port, channel, key, velocity, flags }
            }
            InputValue::Expression {
                expression,
                note_id,
                port,
                channel,
                key,
                value,
                flags,
            } if value.is_finite() => {
                Self::Expression { expression, note_id, port, channel, key, value, flags }
            }
            InputValue::Midi { port, data, flags } => Self::Midi { port, data, flags },
            _ => return Err(StageError::Invalid),
        })
    }

    fn input(self) -> InputValue {
        match self {
            Self::Note { kind, note_id, port, channel, key, velocity, flags } => {
                InputValue::Note { kind, note_id, port, channel, key, velocity, flags }
            }
            Self::Expression { expression, note_id, port, channel, key, value, flags } => {
                InputValue::Expression {
                    expression,
                    note_id,
                    port,
                    channel,
                    key,
                    value,
                    flags,
                }
            }
            Self::Midi { port, data, flags } => InputValue::Midi { port, data, flags },
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
enum Events {
    Single(OutputValue),
    Pair {
        first: OutputValue,
        second: OutputValue,
    },
    Sequence {
        first: OutputValue,
        second_token: Token,
        second: OutputValue,
        third: Option<OutputValue>,
    },
}

/// One prepared output unit: a single event, a validated note-on/tuning pair,
/// or a short sequence whose second group must be secured before the first is
/// attempted. Staging validates the enclosing output interval.
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
    pub fn initial_tuning(&self) -> Option<InputValue> {
        match self.events {
            Events::Pair {
                second:
                    tuning @ OutputValue::Expression {
                        expression: CLAP_NOTE_EXPRESSION_TUNING,
                        ..
                    },
                ..
            }
            | Events::Sequence {
                third:
                    Some(
                        tuning @ OutputValue::Expression {
                            expression: CLAP_NOTE_EXPRESSION_TUNING,
                            ..
                        },
                    ),
                ..
            } => Some(tuning.input()),
            _ => None,
        }
    }

    pub fn single(
        token: Token,
        lane: Lane,
        time: u32,
        event: InputValue,
    ) -> Result<Self, StageError> {
        Ok(Self { token, lane, time, events: Events::Single(OutputValue::new(event)?) })
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
            (InputValue::Midi { port: 0, data: [status, key, velocity], .. },
             InputValue::Expression { expression: CLAP_NOTE_EXPRESSION_TUNING, note_id: -1,
                 port: 0, channel, key: tuning_key, .. }) =>
                status & 0xf0 == 0x90 && key < 128 && (1..128).contains(&velocity) && i16::from(status & 15) == channel
                    && i16::from(key) == tuning_key,
            _ => false,
        };
        if !matching {
            return Err(StageError::Invalid);
        }
        Ok(Self {
            token,
            lane: Lane::Normal,
            time,
            events: Events::Pair {
                first: OutputValue::new(note)?,
                second: OutputValue::new(tuning)?,
            },
        })
    }

    /// Make two adjacent normal groups one preparation and one completion.
    /// The first must be a single event and the second may be a single event
    /// or an onset pair. This is the shape required when accepting the first
    /// event without securing the second would make the wire state false.
    pub fn sequence(first: Self, second: Self) -> Result<Self, StageError> {
        if first.lane != Lane::Normal || second.lane != Lane::Normal || first.time != second.time {
            return Err(StageError::Invalid);
        }
        let Events::Single(first_event) = first.events else {
            return Err(StageError::Invalid);
        };
        let (second_event, third) = match second.events {
            Events::Single(event) => (event, None),
            Events::Pair { first, second } => (first, Some(second)),
            Events::Sequence { .. } => return Err(StageError::Invalid),
        };
        Ok(Self {
            token: first.token,
            lane: Lane::Normal,
            time: first.time,
            events: Events::Sequence {
                first: first_event,
                second_token: second.token,
                second: second_event,
                third,
            },
        })
    }

    pub fn sequence_parts(&self) -> Option<(Self, Self)> {
        let Events::Sequence { first, second_token, second, third } = self.events else {
            return None;
        };
        let first = Self {
            token: self.token,
            lane: self.lane,
            time: self.time,
            events: Events::Single(first),
        };
        let second = Self {
            token: second_token,
            lane: self.lane,
            time: self.time,
            events: match third {
                Some(third) => Events::Pair { first: second, second: third },
                None => Events::Single(second),
            },
        };
        Some((first, second))
    }

    pub fn event_count(&self) -> usize {
        match self.events {
            Events::Single(_) => 1,
            Events::Pair { .. } => 2,
            Events::Sequence { third, .. } => 2 + usize::from(third.is_some()),
        }
    }

    pub fn event(&self, index: usize) -> Option<InputValue> {
        match (self.events, index) {
            (Events::Single(e), 0)
            | (Events::Pair { first: e, .. }, 0)
            | (Events::Pair { second: e, .. }, 1)
            | (Events::Sequence { first: e, .. }, 0)
            | (Events::Sequence { second: e, .. }, 1)
            | (Events::Sequence { third: Some(e), .. }, 2) => Some(e.input()),
            _ => None,
        }
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

/// Bits follow event order. A rejection suppresses the remaining events.
/// Accepted prefixes continue under the SAME caller permit even if a fence
/// closes inside a host call.
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
/// group keeps its cell through durable completion and charges every retained
/// event and its completion atomically. Unattempted reservations are not
/// recycled in this callback, so repeatedly rejected claims cannot create
/// unbounded work.
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
        self.stage_all(std::slice::from_ref(&group))
    }

    /// Admit the whole run or none of it. A caller whose emissions must all
    /// land in this callback proves the run fits before the first group is
    /// admitted, so nothing staged between them -- output, parameters or
    /// notifications -- can take credits the rest of the run still needs. No
    /// reservation outlives the call, so there is nothing to leak or to
    /// misattribute across a host round trip. Equal times keep the run's own
    /// order, since the ready heap breaks that tie by admission order.
    pub fn stage_all(&mut self, groups: &[Group]) -> Result<(), StageError> {
        let s = &mut self.scheduler;
        let (mut normal, mut emergency) = (0, 0);
        for group in groups {
            if group.time < s.summary.cursor || group.time >= s.frames {
                return Err(StageError::Invalid);
            }
            match group.lane {
                Lane::Normal => {
                    if s.inhibited {
                        return Err(StageError::Inhibited);
                    }
                    normal += group.event_count();
                }
                Lane::Emergency => emergency += group.event_count(),
            }
        }
        if s.normal_reserved + normal > NORMAL_OUTPUT_ATTEMPTS
            || s.emergency_reserved + emergency > EMERGENCY_OUTPUT_ATTEMPTS
            || s.used_cells + groups.len() > OUTPUT_CELLS
        {
            return Err(StageError::Full);
        }
        s.normal_reserved += normal;
        s.emergency_reserved += emergency;
        for group in groups {
            self.scheduler.insert(*group);
        }
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
