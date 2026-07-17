//! Pure lattice/tuning/note-tracking logic. No I/O, no GUI, no plugin
//! dependencies — everything here is unit-testable and shared verbatim
//! between the standalone dev harness and the plugin.

pub mod coords;
pub mod notes;
pub mod tuning;

pub use coords::{positions_within, LatticePos, NoteName};
pub use notes::{ChannelRole, NoteEvent, NoteEventKind, NoteTracker, Time, Voice, VoiceState};
pub use tuning::{learn_tuning, LearnedTuning, PitchClass, PitchClassDistance, Tuning};
