//! Fixed performance values. These cannot contain full transport, a host
//! pointer, or an allocation.
use harmonigraph_core::Expression;
use nice_plug::wrapper::clap::configuration::InputValue;

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Event {
    Note { kind: u16, id: i32, port: i16, channel: i16, key: i16, velocity: f64, flags: u32 },
    Expression { kind: i32, id: i32, port: i16, channel: i16, key: i16, value: f64, flags: u32 },
    Midi { port: u16, data: [u8; 3], flags: u32 },
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

    /// Can reach a CLAP output list as itself. This is what the deleted
    /// `nice_plug` staging validator checked before it would retain a value:
    /// `Output::push` now encodes whatever it is handed, so a kind CLAP has no
    /// note event for, or a magnitude it cannot carry, has to be refused here
    /// instead. MIDI is already narrowed by [`Self::from_input`].
    pub fn emittable(self) -> bool {
        match self {
            Self::Note { kind: 0..=3, velocity, .. } => velocity.is_finite(),
            Self::Note { .. } => false,
            Self::Expression { value, .. } => value.is_finite(),
            Self::Midi { .. } => true,
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

    /// Start a reverse scan of one source's same-sample prefix. Only finite
    /// tuning expressions initialize onsets; addressing is shared with output.
    pub fn initial_tuning(self) -> Option<InitialTuning> {
        let Self::Expression { kind: 2, value, .. } = self else { return None };
        value.is_finite().then_some(InitialTuning { expression: self, value, seen: [0; 16] })
    }

    /// The pressure, gain or timbre this carries, by CLAP's volume (0),
    /// brightness (5) and pressure (6) note expressions, in range.
    pub fn expression(self) -> Option<(Expression, f32)> {
        let Self::Expression { kind, value, .. } = self else { return None };
        let expression = match kind {
            0 => Expression::Gain,
            5 => Expression::Timbre,
            6 => Expression::Pressure,
            _ => return None,
        };
        Some((expression, expression.accept(value as f32)?))
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
}

/// A wildcard can initialize several keys, but only the newest identity at
/// each channel/key. This stack-local scan marks replacements before matching,
/// so an expression naming an obsolete id cannot reach through its replacement.
/// Expressions before an onset cannot initialize it; repeated expressions
/// overwrite every addressed onset with their latest value.
pub struct InitialTuning {
    expression: Event,
    value: f64,
    seen: [u128; 16],
}

impl InitialTuning {
    pub fn bind(&mut self, event: Event) -> Option<f64> {
        let (id, channel, key, _) = event.attack()?;
        let seen = &mut self.seen[usize::from(channel)];
        let bit = 1u128 << key;
        let replaced = *seen & bit != 0;
        *seen |= bit;
        (!replaced && self.expression.matches(id, channel, key)).then_some(self.value)
    }
}

const _: () = assert!(std::mem::size_of::<Event>() <= 48);
