//! Whether a plugin has note ports.

/// Which note events a plugin reads or writes. Any level above `None` gives
/// it a note port that speaks the CLAP and MIDI dialects; the levels differ
/// only in what the plugin itself does with the events.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum MidiConfig {
    /// No note port.
    None,
    /// Notes, note expressions and their MIDI equivalents.
    Basic,
    /// Also MIDI controllers and pitch bend.
    MidiCCs,
}
