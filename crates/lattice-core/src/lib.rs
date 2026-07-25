//! Pure lattice/tuning/note-tracking/analysis logic. No I/O, no GUI, and
//! no dependencies at all — everything here is unit-testable and shared
//! verbatim between the standalone dev harness and the plugin.
//!
//! What lives where:
//! - [`coords`] — integer lattice positions and note spelling.
//! - [`notes`] — MIDI voice tracking: which pitches are sounding, and the
//!   envelopes that outlive their note-offs.
//! - [`history`] — what has already been played, kept once a voice's fade
//!   completes, so the scene can mark where the music has been.
//! - [`tuning`] — pitch classes in integer microcents, the tuning of each
//!   prime axis, and learning a tuning from what's played.
//! - [`spectrum`] — a hand-rolled FFT and the analyzer that buckets it
//!   onto a MIDI-pitch axis for the Spectral pane.
//! - [`spectrogram`] — those spectra kept over time, quantized and
//!   age-tiered so a long history stays affordable.
//!
//! Convention: types are re-exported at the crate root (below), while free
//! functions and constants are reached through their module
//! (`lattice_core::tuning::microcents`, `lattice_core::coords::positions_within`).

pub mod align;
pub mod coords;
pub mod history;
pub mod notes;
pub mod roll;
pub mod spectrogram;
pub mod spectrum;
pub mod tuning;
pub mod wav;

pub use coords::{positions_within, LatticePos, NoteName};
pub use history::{NoteHistory, Visit};
pub use notes::{ChannelRole, NoteEvent, NoteEventKind, NoteTracker, Time, Voice, VoiceState};
pub use roll::{NoteRoll, RollNote};
pub use spectrogram::{SpectrogramColumn, SpectrumHistory};
pub use tuning::{learn_tuning, LearnedTuning, PitchClass, PitchClassDistance, Tuning};
