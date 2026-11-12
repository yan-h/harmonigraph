//! The tape (#1719): an always-on recording of the main input keyed on song
//! position, played back from wherever the Tape tab is clicked while the
//! transport is stopped.
//!
//! - **Capture** reuses the analyzer's ring. While the transport rolls, each
//!   block's descriptor carries its song position, and the input worker
//!   appends those blocks to the [`store`] ([`worker`]). Notes and tuning are
//!   not stored yet: nothing reads them before the lattice and roll follow the
//!   tape (#1719 slice 3), which needs an exact note chase this shape cannot
//!   give.
//! - **Audition** starts from a click in the Tape tab. Bitwig's own playhead
//!   cannot start it: a click in the arranger while stopped does not change
//!   the song position Bitwig sends a plugin (measured in #1721). The input
//!   worker passes the click on; the audio thread ([`deck`]) publishes the
//!   cue; the worker assembles audio from the store into a ring; the audio
//!   thread adds that ring to the main buffer. The audio thread stays trivial
//!   and real-time safe, at the cost of a few tens of ms after a click.
//!
//! The tape never records itself: it records only rolling blocks, and sounds
//! only while stopped. Nothing here is saved with the project.

use std::sync::atomic::{AtomicBool, AtomicU64};
use std::sync::Arc;

pub(crate) mod deck;
pub(crate) mod store;
pub(crate) mod worker;

#[cfg(test)]
mod tests;

pub(crate) use deck::Deck;
pub(crate) use worker::Worker;

/// Capacity of the playback ring, in interleaved samples: 0.68 s of stereo at
/// 96 kHz, well over the [`worker::LEAD_SECONDS`] the worker keeps queued at
/// any rate a host offers.
const PLAYBACK_SAMPLES: usize = 1 << 17;
/// Pieces in flight. The worker sends at most one a step per cue.
const PIECES: usize = 256;
/// Console events in flight between two worker steps.
const EVENTS: usize = 256;

/// The request slot's two values that are not a position: NaNs, which no
/// song position is.
const NO_REQUEST: u64 = 0x7ff8_0000_0000_0001;
const STOP: u64 = 0x7ff8_0000_0000_0002;

/// What the audio thread and the worker publish for each other.
pub(crate) struct Shared {
    /// The cue serial; odd while a cue wants audio.
    cue: AtomicU64,
    /// Song seconds the cue starts at, as f64 bits.
    cue_position: AtomicU64,
    /// The Tape tab's newest request, which the deck swaps out for
    /// [`NO_REQUEST`]: the song seconds to cue from as f64 bits, or [`STOP`].
    /// One slot, so a newer request replaces one not yet taken and a request
    /// is taken whole or not at all.
    request: AtomicU64,
    /// What the deck last saw, for the Tape tab: the song position (f64 bits,
    /// NaN for none), whether it rolled, whether the tape may sound, and the
    /// song seconds sounding (NaN while silent).
    position: AtomicU64,
    rolling: AtomicBool,
    enabled: AtomicBool,
    heard: AtomicU64,
}

impl Shared {
    fn new() -> Shared {
        Shared {
            cue: AtomicU64::new(0),
            cue_position: AtomicU64::new(0),
            request: AtomicU64::new(NO_REQUEST),
            position: AtomicU64::new(f64::NAN.to_bits()),
            rolling: AtomicBool::new(false),
            enabled: AtomicBool::new(false),
            heard: AtomicU64::new(f64::NAN.to_bits()),
        }
    }
}

/// A run of assembled frames in the playback ring, for one cue.
#[derive(Clone, Copy, Debug)]
pub(crate) struct Piece {
    serial: u64,
    frames: usize,
    channels: usize,
    rate: f32,
    /// Coverage ends after these frames: the cue is over.
    last: bool,
}

/// Why a cue ended.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Reason {
    /// Another cue.
    Cue,
    /// The Tape tab's Stop.
    Stop,
    /// The analysis input left Main.
    Off,
    /// The transport started rolling.
    Rolling,
    /// The plugin was reset.
    Reset,
    /// The end of what is recorded.
    End,
    /// The host stopped reporting a song position.
    NoPosition,
    /// No audio arrived from the worker in time.
    Starved,
}

impl Reason {
    fn describe(self) -> &'static str {
        match self {
            Reason::Cue => "another cue",
            Reason::Stop => "Stop",
            Reason::Off => "the analysis input leaving Main",
            Reason::Rolling => "the transport started",
            Reason::Reset => "a plugin reset",
            Reason::End => "the end of what is recorded",
            Reason::NoPosition => "the host reported no song position",
            Reason::Starved => "no audio from the input worker",
        }
    }
}

/// Console lines from the audio thread, formatted by the worker.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) enum Event {
    Started {
        position: f64,
    },
    Ended {
        position: f64,
        played: f64,
        reason: Reason,
    },
    Reset {
        sounding: bool,
    },
    /// The first two blocks after a reset.
    AfterReset {
        first: f64,
        first_playing: bool,
        second: f64,
        second_playing: bool,
        frames: u32,
    },
    /// A rolling block followed a block that carried tape.
    TapeBeforeRolling {
        position: f64,
        advanced: bool,
    },
}

/// The two halves: one for `process`, one for the input worker.
pub(crate) fn channel() -> (Deck, Worker) {
    let shared = Arc::new(Shared::new());
    let (sample_tx, sample_rx) = rtrb::RingBuffer::new(PLAYBACK_SAMPLES);
    let (piece_tx, piece_rx) = rtrb::RingBuffer::new(PIECES);
    let (event_tx, event_rx) = rtrb::RingBuffer::new(EVENTS);
    (
        Deck::new(shared.clone(), sample_rx, piece_rx, event_tx),
        Worker::new(shared, sample_tx, piece_tx, event_rx),
    )
}
