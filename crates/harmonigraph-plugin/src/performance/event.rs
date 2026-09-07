//! Fixed performance values and an internal Stop boundary marker. These cannot
//! contain full transport, a host pointer, or an allocation.
use nice_plug::wrapper::clap::configuration::InputValue;

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Event {
    /// Retained and consumed by Source at its original sample, never sent to a host.
    Stop,
    /// Ordered local participation boundary, never a downstream MIDI event.
    Participation(bool),
    Note {
        kind: u16,
        id: i32,
        port: i16,
        channel: i16,
        key: i16,
        velocity: f64,
        flags: u32,
    },
    Expression {
        kind: i32,
        id: i32,
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

impl Event {
    pub fn from_input(value: InputValue) -> Option<Self> {
        Some(match value {
            InputValue::Note { kind, note_id, port, channel, key, velocity, flags } => {
                Self::Note { kind, id: note_id, port, channel, key, velocity, flags }
            }
            InputValue::Expression { expression, note_id, port, channel, key, value, flags } => {
                Self::Expression { kind: expression, id: note_id, port, channel, key, value, flags }
            }
            InputValue::Midi { port, data, flags }
                if port == 0 && data[0] >= 0x80 && data[1] < 128 && data[2] < 128 =>
            {
                Self::Midi { port, data, flags }
            }
            _ => return None,
        })
    }

    pub fn input(self) -> InputValue {
        match self {
            Self::Stop | Self::Participation(_) => {
                unreachable!("local boundaries are not wire output")
            }
            Self::Note { kind, id, port, channel, key, velocity, flags } => {
                InputValue::Note { kind, note_id: id, port, channel, key, velocity, flags }
            }
            Self::Expression { kind, id, port, channel, key, value, flags } => {
                InputValue::Expression {
                    expression: kind,
                    note_id: id,
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

    pub fn channel_control(self) -> Option<u8> {
        match self {
            Self::Midi { port: 0, data, .. } if matches!(data[0] & 0xf0, 0xb0..=0xe0) => {
                Some(data[0] & 15)
            }
            _ => None,
        }
    }
    /// true=All Sound Off (choke), false=All Notes Off (logical note-off).
    pub fn channel_termination(self) -> Option<bool> {
        match self {
            Self::Midi { port: 0, data: [status, 120, _], .. } if status & 0xf0 == 0xb0 => {
                Some(true)
            }
            Self::Midi { port: 0, data: [status, 123, _], .. } if status & 0xf0 == 0xb0 => {
                Some(false)
            }
            _ => None,
        }
    }
    pub fn note_off(id: i32, channel: u8, key: u8, midi: bool) -> Self {
        if midi {
            Self::Midi { port: 0, data: [0x80 | channel, key, 0], flags: 0 }
        } else {
            Self::Note {
                kind: 1,
                id,
                port: 0,
                channel: i16::from(channel),
                key: i16::from(key),
                velocity: 0.0,
                flags: 0,
            }
        }
    }

    pub fn attack(self) -> Option<(i32, u8, u8, f32)> {
        match self {
            Self::Note {
                kind: 0,
                id,
                port: 0,
                channel: channel @ 0..=15,
                key: key @ 0..=127,
                velocity,
                ..
            } if velocity.is_finite() && (0.0..=1.0).contains(&velocity) => {
                Some((id, channel as u8, key as u8, velocity as f32))
            }
            Self::Midi { port: 0, data, .. } if data[0] & 0xf0 == 0x90 && data[2] != 0 => {
                Some((-1, data[0] & 15, data[1], f32::from(data[2]) / 127.0))
            }
            _ => None,
        }
    }

    pub fn release(self) -> bool {
        matches!(self, Self::Note { kind: 1 | 2, .. })
            || matches!(self, Self::Midi { data, .. }
                if data[0] & 0xf0 == 0x80 || data[0] & 0xf0 == 0x90 && data[2] == 0)
    }

    pub fn channel(self) -> Option<u8> {
        match self {
            Self::Note { channel: c @ 0..=15, .. }
            | Self::Expression { channel: c @ 0..=15, .. } => Some(c as u8),
            Self::Midi { data, .. } if data[0] < 0xf0 => Some(data[0] & 15),
            _ => None,
        }
    }

    pub fn matches(self, id: i32, channel: u8, key: u8) -> bool {
        match self {
            Self::Note { id: i, port, channel: c, key: k, .. }
            | Self::Expression { id: i, port, channel: c, key: k, .. } => {
                matches!(port, -1 | 0)
                    && (i == -1 || i == id)
                    && (c == -1 || c == i16::from(channel))
                    && (k == -1 || k == i16::from(key))
            }
            Self::Midi { port: 0, data, .. } => data[0] & 15 == channel && data[1] == key,
            _ => false,
        }
    }

    /// An input wildcard's captured target is emitted with its original ID but
    /// concrete key/channel so it cannot address a later translated lifetime.
    pub fn addressed(self, channel: u8, key: u8) -> Self {
        match self {
            Self::Note { kind, id, velocity, flags, .. } => Self::Note {
                kind,
                id,
                port: 0,
                channel: i16::from(channel),
                key: i16::from(key),
                velocity,
                flags,
            },
            Self::Expression { kind, id, value, flags, .. } => Self::Expression {
                kind,
                id,
                port: 0,
                channel: i16::from(channel),
                key: i16::from(key),
                value,
                flags,
            },
            value => value,
        }
    }

    pub fn for_voice(self, id: i32, channel: u8, key: u8) -> Self {
        let mut value = self.addressed(channel, key);
        match &mut value {
            Self::Note { id: target, .. } | Self::Expression { id: target, .. } => *target = id,
            _ => {}
        }
        value
    }

    pub fn terminate(id: i32, channel: u8, key: u8) -> Self {
        Self::Note {
            kind: 2,
            id,
            port: 0,
            channel: i16::from(channel),
            key: i16::from(key),
            velocity: 0.0,
            flags: 0,
        }
    }
}

const _: () = assert!(std::mem::size_of::<Event>() <= 48);
