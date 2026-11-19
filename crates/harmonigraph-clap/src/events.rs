//! The host's events as the plugin reads them, and the output list it writes.
//!
//! The shell validates a callback's whole input list once, before anything
//! is applied: every event present, timed inside the block and in order, and
//! long enough for the type it claims. A list that fails loses the callback's
//! input whole, as a list did when it failed to copy into the pool this
//! replaced. A valid list is read lazily, by index, as often as the plugin
//! walks it: nothing is copied.
//!
//! An event the plugin does not read is skipped rather than failing the
//! callback (#1814 Q3): SysEx, MIDI 2, events outside the core space, and a
//! parameter value or modulation addressed to a note, port, channel or key
//! (no parameter declares per-note support). Failing the callback lost its
//! Note-Offs; a skipped event is applied to no voice, so nothing turns
//! monophonic either.

use clap_sys::events::*;
use clap_sys::fixedpoint::{CLAP_BEATTIME_FACTOR, CLAP_SECTIME_FACTOR};
use std::mem::size_of;

/// The host's transport, as the callback carries it and as an event in the
/// list carries it: every field, flags and fixed-point timelines included.
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

impl Transport {
    fn has(&self, flag: u32) -> bool {
        self.flags & flag != 0
    }

    pub fn playing(&self) -> bool {
        self.has(CLAP_TRANSPORT_IS_PLAYING)
    }

    /// The song position in seconds, if the host gives a seconds timeline.
    pub fn seconds(&self) -> Option<f64> {
        self.has(CLAP_TRANSPORT_HAS_SECONDS_TIMELINE)
            .then(|| self.song_pos_seconds as f64 / CLAP_SECTIME_FACTOR as f64)
    }

    /// The song position in quarter notes, if the host gives a beats
    /// timeline.
    pub fn beats(&self) -> Option<f64> {
        self.has(CLAP_TRANSPORT_HAS_BEATS_TIMELINE).then(|| beats(self.song_pos_beats))
    }

    /// Quarter notes per minute, if the host gives a tempo.
    pub fn tempo(&self) -> Option<f64> {
        self.has(CLAP_TRANSPORT_HAS_TEMPO).then_some(self.tempo)
    }

    /// The time signature's numerator and denominator, if the host gives one.
    pub fn time_signature(&self) -> Option<(u16, u16)> {
        self.has(CLAP_TRANSPORT_HAS_TIME_SIGNATURE).then_some((self.tsig_num, self.tsig_denom))
    }

    /// The current bar, from 0 at the song's first, and where it starts in
    /// quarter notes. CLAP puts these behind no flag, so a host that fills
    /// in none of them gives bar 0 starting at beat 0.
    pub fn bar(&self) -> (i32, f64) {
        (self.bar_number, beats(self.bar_start))
    }
}

/// CLAP's fixed-point beats as quarter notes.
fn beats(value: i64) -> f64 {
    value as f64 / CLAP_BEATTIME_FACTOR as f64
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

/// One host event's value, decoded from its CLAP struct with nothing lost:
/// the raw note id, the signed wildcard addresses, the `f64` value and the
/// header flags.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum InputValue {
    /// A parameter value or mono modulation addressed to every note, in host
    /// units. [`crate::params::Table::decode`] turns it into a row's value.
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
    Transport(Transport),
}

/// One input at its sample on the host's steady clock.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Input {
    pub sample: i64,
    pub value: InputValue,
}

/// A list that passed validation, and what validating it found.
#[derive(Clone, Copy)]
pub(crate) struct Checked<'a> {
    pub list: List<'a>,
    /// The list holds a transport event. Bitwig sends none (#1814): it
    /// supplies the transport only as the callback's own.
    pub transport: bool,
}

/// The number of bytes a core event the plugin reads must hold, or `None`
/// for an event it skips.
fn required(header: &clap_event_header) -> Option<usize> {
    if header.space_id != CLAP_CORE_EVENT_SPACE_ID {
        return None;
    }
    Some(match header.type_ {
        CLAP_EVENT_PARAM_VALUE => size_of::<clap_event_param_value>(),
        CLAP_EVENT_PARAM_MOD => size_of::<clap_event_param_mod>(),
        CLAP_EVENT_NOTE_ON | CLAP_EVENT_NOTE_OFF | CLAP_EVENT_NOTE_CHOKE | CLAP_EVENT_NOTE_END => {
            size_of::<clap_event_note>()
        }
        CLAP_EVENT_NOTE_EXPRESSION => size_of::<clap_event_note_expression>(),
        CLAP_EVENT_MIDI => size_of::<clap_event_midi>(),
        CLAP_EVENT_TRANSPORT => size_of::<clap_event_transport>(),
        _ => return None,
    })
}

/// Check every event of `list`: present, long enough for the type it claims
/// and, given the block's `frames`, inside the block and in time order. A
/// flush passes `None`, since its events have no time. A null list is empty.
/// What passes is the list the plugin reads.
///
/// # Safety
/// `list` must be null or a valid host input list for `'a`.
pub(crate) unsafe fn validate<'a>(
    list: *const clap_input_events,
    frames: Option<u32>,
) -> Result<Checked<'a>, ()> {
    let Some(list) = (unsafe { list.as_ref() }) else {
        return Ok(Checked { list: List::EMPTY, transport: false });
    };
    let (Some(size), Some(get)) = (list.size, list.get) else { return Err(()) };
    let count = unsafe { size(list) };
    let mut checked = Checked { list: List { list: Some(list), count }, transport: false };
    let mut previous = 0;
    for index in 0..count {
        // Only the header is borrowed here; the event is read later from the
        // host's own pointer, whose reach covers all of it.
        let Some(header) = (unsafe { get(list, index).as_ref() }) else { return Err(()) };
        if let Some(frames) = frames {
            if header.time >= frames || header.time < previous {
                return Err(());
            }
            previous = header.time;
        }
        if required(header).is_some_and(|bytes| (header.size as usize) < bytes) {
            return Err(());
        }
        checked.transport |=
            header.space_id == CLAP_CORE_EVENT_SPACE_ID && header.type_ == CLAP_EVENT_TRANSPORT;
    }
    Ok(checked)
}

/// The value of one validated event, or `None` for one the plugin skips.
/// The header is borrowed only to read its kind; the event itself is read
/// through the host's pointer, so it keeps the reach of the whole event.
///
/// # Safety
/// `pointer` must be the host's pointer to an event [`validate`] accepted.
unsafe fn decode(pointer: *const clap_event_header) -> Option<InputValue> {
    let (kind, flags) = {
        let header = unsafe { &*pointer };
        required(header)?;
        (header.type_, header.flags)
    };
    let wildcard = |note_id, port, channel, key| (note_id, port, channel, key) == (-1, -1, -1, -1);
    Some(match kind {
        CLAP_EVENT_PARAM_VALUE => {
            let e = unsafe { &*pointer.cast::<clap_event_param_value>() };
            if !wildcard(e.note_id, e.port_index, e.channel, e.key) {
                return None;
            }
            InputValue::Parameter { id: e.param_id, value: e.value, modulation: false }
        }
        CLAP_EVENT_PARAM_MOD => {
            let e = unsafe { &*pointer.cast::<clap_event_param_mod>() };
            if !wildcard(e.note_id, e.port_index, e.channel, e.key) {
                return None;
            }
            InputValue::Parameter { id: e.param_id, value: e.amount, modulation: true }
        }
        CLAP_EVENT_NOTE_EXPRESSION => {
            let e = unsafe { &*pointer.cast::<clap_event_note_expression>() };
            InputValue::Expression {
                expression: e.expression_id,
                note_id: e.note_id,
                port: e.port_index,
                channel: e.channel,
                key: e.key,
                value: e.value,
                flags,
            }
        }
        CLAP_EVENT_MIDI => {
            let e = unsafe { &*pointer.cast::<clap_event_midi>() };
            InputValue::Midi { port: e.port_index, data: e.data, flags }
        }
        CLAP_EVENT_TRANSPORT => {
            InputValue::Transport(unsafe { *pointer.cast::<clap_event_transport>() }.into())
        }
        kind => {
            let e = unsafe { &*pointer.cast::<clap_event_note>() };
            InputValue::Note {
                kind,
                note_id: e.note_id,
                port: e.port_index,
                channel: e.channel,
                key: e.key,
                velocity: e.velocity,
                flags,
            }
        }
    })
}

/// A validated input list, read by index.
#[derive(Clone, Copy)]
pub(crate) struct List<'a> {
    list: Option<&'a clap_input_events>,
    count: u32,
}

impl<'a> List<'a> {
    /// No events: a lost input, or a callback the host gave none.
    pub(crate) const EMPTY: List<'static> = List { list: None, count: 0 };

    /// Every event the plugin reads, with its block offset.
    pub(crate) fn values(self) -> impl Iterator<Item = (u32, InputValue)> + use<'a> {
        let list = self.list;
        (0..self.count).filter_map(move |index| {
            let list = list?;
            // SAFETY: validated, so `get` exists and the event is present.
            let pointer = unsafe { (list.get.unwrap())(list, index) };
            let time = unsafe { (*pointer).time };
            Some((time, unsafe { decode(pointer) }?))
        })
    }
}

/// The host's output list for one callback.
///
/// A CLAP output list is sorted by time, so the times one callback pushes
/// must not decrease. The plugin's own emission keeps them in order (a delay
/// line emitting in due order), and the shell's own reports all go at offset
/// 0, before the plugin's first push. The `debug_assert` is the only check:
/// the release path is a `try_push` and nothing else.
pub struct Output<'a> {
    list: Option<&'a clap_output_events>,
    last: u32,
}

impl<'a> Output<'a> {
    /// A list that cannot be written: a null one, one with no `try_push`, and
    /// a callback whose input was lost all become one where every push fails,
    /// so a caller that retains on failure keeps the value for a callback
    /// that can carry it.
    ///
    /// # Safety
    /// `list` must be null or a valid host output list for all of `'a`.
    pub(crate) unsafe fn new(list: *const clap_output_events, writable: bool) -> Self {
        let list = unsafe { list.as_ref() };
        Self { list: list.filter(|list| writable && list.try_push.is_some()), last: 0 }
    }

    /// Push `value` at `time` and return the host's answer. A parameter
    /// value goes out as an edit the user made live; a modulation, a
    /// transport and a non-finite magnitude have no output encoding. False
    /// also covers those and an output that cannot be written, so the caller
    /// never has to tell a refusal from a value it cannot send.
    pub fn push(&mut self, value: InputValue, time: u32) -> bool {
        self.push_with(time, |list| unsafe { push_value(list, value, time) })
    }

    /// Push the begin (`true`) or end of a gesture on parameter `id`.
    pub fn push_gesture(&mut self, id: u32, begin: bool, time: u32) -> bool {
        let kind =
            if begin { CLAP_EVENT_PARAM_GESTURE_BEGIN } else { CLAP_EVENT_PARAM_GESTURE_END };
        self.push_with(time, |list| {
            let event = clap_event_param_gesture {
                header: header::<clap_event_param_gesture>(time, kind, CLAP_EVENT_IS_LIVE),
                param_id: id,
            };
            unsafe { (list.try_push.unwrap())(list, whole(&event)) }
        })
    }

    fn push_with(&mut self, time: u32, push: impl FnOnce(&clap_output_events) -> bool) -> bool {
        debug_assert!(time >= self.last, "CLAP output must be sorted by time");
        let Some(list) = self.list else { return false };
        let pushed = push(list);
        if pushed {
            self.last = time;
        }
        pushed
    }
}

/// A pointer to all of `event`, typed as its header, as CLAP passes events:
/// a pointer to the header field alone could not reach the rest (Miri).
fn whole<T>(event: &T) -> *const clap_event_header {
    (event as *const T).cast()
}

/// The header of an output event of type `T`.
fn header<T>(time: u32, kind: u16, flags: u32) -> clap_event_header {
    clap_event_header {
        size: size_of::<T>() as u32,
        time,
        space_id: CLAP_CORE_EVENT_SPACE_ID,
        type_: kind,
        flags,
    }
}

unsafe fn push_value(output: &clap_output_events, value: InputValue, time: u32) -> bool {
    let push = output.try_push.unwrap();
    match value {
        InputValue::Note { kind, note_id, port, channel, key, velocity, flags }
            if matches!(
                kind,
                CLAP_EVENT_NOTE_ON
                    | CLAP_EVENT_NOTE_OFF
                    | CLAP_EVENT_NOTE_CHOKE
                    | CLAP_EVENT_NOTE_END
            ) && velocity.is_finite() =>
        {
            let event = clap_event_note {
                header: header::<clap_event_note>(time, kind, flags),
                note_id,
                port_index: port,
                channel,
                key,
                velocity,
            };
            unsafe { push(output, whole(&event)) }
        }
        InputValue::Expression { expression, note_id, port, channel, key, value, flags }
            if value.is_finite() =>
        {
            let event = clap_event_note_expression {
                header: header::<clap_event_note_expression>(
                    time,
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
            unsafe { push(output, whole(&event)) }
        }
        InputValue::Midi { port, data, flags } => {
            let event = clap_event_midi {
                header: header::<clap_event_midi>(time, CLAP_EVENT_MIDI, flags),
                port_index: port,
                data,
            };
            unsafe { push(output, whole(&event)) }
        }
        InputValue::Parameter { id, value, modulation: false } if value.is_finite() => {
            let event = clap_event_param_value {
                header: header::<clap_event_param_value>(
                    time,
                    CLAP_EVENT_PARAM_VALUE,
                    CLAP_EVENT_IS_LIVE,
                ),
                param_id: id,
                cookie: std::ptr::null_mut(),
                note_id: -1,
                port_index: -1,
                channel: -1,
                key: -1,
                value,
            };
            unsafe { push(output, whole(&event)) }
        }
        // No output encoding. A caller that would retain on false owes itself
        // a check that the value can ever be sent, or it retains it forever.
        _ => false,
    }
}
