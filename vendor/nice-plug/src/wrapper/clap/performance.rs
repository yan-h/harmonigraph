//! Opt-in owned CLAP performance boundary. Musical retention, eligibility and
//! every decision about what reaches the host belong to the plugin; what the
//! wrapper supplies is the exact value, its provenance, and the host's answer.
use super::configuration::InputValue;
use clap_sys::events::*;

/// How many parameter values and configuration notifications the wrapper will
/// push in one enclosing callback. The plugin's own output is not counted and
/// is not capped: it is bounded by whatever the plugin retains.
///
/// This is the bound the deleted scheduler's normal-lane allowance was also
/// serving. The GUI ring alone would cap a callback at one admitted prefix
/// (`INPUT_SCAN`), which is four times this and is not a number chosen for how
/// long an audio callback should spend calling the host back.
pub const PARAMETER_OUTPUT_ATTEMPTS: usize = 512;

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

/// What the wrapper itself observed over one enclosing callback. Host acceptance
/// of the plugin's own output is not in here: [`Output::push`] returns it per
/// event, to the caller that chose the event.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Summary {
    pub legacy_send_misuse: bool,
}

/// A borrow of the host's output list for the duration of one plugin callback.
pub struct Output<'a> {
    list: Option<&'a clap_output_events>,
}

impl<'a> Output<'a> {
    /// A `writable` false, a null list and a host that supplied no `try_push`
    /// all become the same thing: every push fails, so a caller that retains on
    /// failure keeps the value for a callback that can carry it.
    ///
    /// # Safety
    /// `list` must be null or a valid host output list for all of `'a`.
    pub(crate) unsafe fn new(list: *const clap_output_events, writable: bool) -> Self {
        let list = unsafe { list.as_ref() };
        Self { list: list.filter(|list| writable && list.try_push.is_some()) }
    }

    /// Push one event at `time` and return the host's answer. False also covers
    /// an unavailable output list and a value with no output encoding, so the
    /// caller never has to tell a refusal apart from a value it cannot send.
    ///
    /// **The chronological floor is the caller's contract.** A CLAP output event
    /// list is sorted by time, and nothing here checks that any more: the times
    /// one callback pushes must not decrease, and each must fall inside the
    /// sub-block the caller was handed. A delay line emitting in due order,
    /// floored at the sub-block's start, satisfies both.
    ///
    /// The wrapper's own parameter and configuration output is what that floor
    /// used to have to negotiate with through a shared cursor, and it no longer
    /// does. The wrapper pins every event it pushes itself to the LAST sample of
    /// the sub-block it is draining, after the caller's events for that
    /// sub-block, so the two halves cannot cross without either side learning
    /// the other's times: a parameter at `start + frames - 1` is at or after
    /// every note in `[start, start + frames)` and before every note in the
    /// next sub-block. Of the two samples the wrapper could prove correct, it
    /// takes the later one, because a value observed during a callback shaped
    /// none of that callback's notes -- the configuration in force was frozen at
    /// `clap_configuration_adopt` before the first of them.
    pub fn push(&mut self, value: InputValue, time: u32) -> bool {
        self.list.is_some_and(|list| unsafe { push_value(list, value, time) })
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
        // Parameter, transport and unsupported values have no output encoding
        // here. Staging used to reject them before they could be retained at
        // all; refusing them at the wire is the same answer one step later, and
        // the caller reads it the same way it reads a host refusal.
        InputValue::Parameter { .. } | InputValue::Transport(_) | InputValue::Other => false,
    }
}
