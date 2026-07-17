//! Pure lattice/tuning/note-tracking logic. No I/O, no GUI, no plugin
//! dependencies — everything here is unit-testable and shared verbatim
//! between the standalone dev harness and the plugin.

pub mod coords;
pub mod notes;
pub mod tuning;

pub use coords::LatticePos;
pub use notes::{NoteEvent, NoteEventKind, NoteTracker, Voice, VoiceState};
pub use tuning::{PitchClass, PitchClassDistance, Tuning};
