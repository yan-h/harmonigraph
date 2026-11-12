//! The tape (#1719): an always-on recording of the main input keyed on song
//! position, played back from Bitwig's playhead while the transport is stopped.
//!
//! - **Capture** reuses the analyzer's ring. While the transport rolls, each
//!   block's descriptor carries its song position, and the input worker
//!   appends those blocks to the [`store`], with the notes and tuning that
//!   arrived beside them ([`worker`]).
//! - **Audition** starts when a stopped playhead moves and then holds still
//!   for a block. The audio thread ([`deck`]) publishes the cue; the input
//!   worker assembles audio from the store into a ring; the audio thread adds
//!   that ring to the main buffer. The audio thread stays trivial and
//!   real-time safe, at the cost of a few tens of ms after a click.
//!
//! The tape never records itself: it records only rolling blocks, and sounds
//! only while stopped. Nothing here is saved with the project.

use std::sync::atomic::{AtomicBool, AtomicU64};
use std::sync::Arc;

use harmonigraph_core::canonical::{NoteDelta, PublicationGap, SourceBaseline};
use harmonigraph_core::configuration::ResolvedConfig;
use harmonigraph_ui::params::ParamKey;

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

/// What the audio thread publishes for the worker.
#[derive(Default)]
pub(crate) struct Shared {
    /// The cue serial; odd while a cue wants audio.
    cue: AtomicU64,
    /// Song seconds the cue starts at, as f64 bits.
    cue_position: AtomicU64,
    /// Process calls so far, and whether the last was stopped, and how many
    /// seconds of all-zero input had run by then (f64 bits): the
    /// processing-while-stopped measurement.
    blocks: AtomicU64,
    stopped: AtomicBool,
    quiet: AtomicU64,
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
    /// Another click.
    Click,
    /// Audition was switched off, or the analysis input left Main.
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
            Reason::Click => "another click",
            Reason::Off => "Audition off",
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
    /// The playhead moved after the transport stopped.
    Returned {
        from: f64,
        to: f64,
        blocks: u32,
        seconds: f64,
    },
    NoReturn {
        from: f64,
        seconds: f64,
    },
    /// A run of stopped playhead moves.
    Moves {
        count: u32,
        from: f64,
        to: f64,
        seconds: f64,
    },
}

/// The tuning a stretch of tape was recorded under: what the live display
/// drew it with. Sampled by the worker once a step, so it lands up to a poll
/// late, and further during a faster-than-realtime export.
#[derive(Clone, Debug, Default, PartialEq)]
pub(crate) struct Tuning {
    /// `None` until the CLAP configuration extension has been installed.
    pub resolved: Option<ResolvedConfig>,
    /// The values the take records with each block (`ParamKey::ALL`).
    pub params: [f32; ParamKey::ALL.len()],
}

/// A note or tuning fact stored beside the audio.
#[derive(Clone, Debug)]
#[expect(
    dead_code,
    reason = "recorded now, read once the lattice and roll follow the tape (#1719 slice 3)"
)]
pub(crate) enum Mark {
    Note(NoteDelta),
    Gap(PublicationGap),
    Baseline(Box<SourceBaseline>),
    Tuning(Box<Tuning>),
}

/// The two halves: one for `process`, one for the input worker.
pub(crate) fn channel(params: Arc<crate::HarmonigraphParams>) -> (Deck, Worker) {
    let shared = Arc::new(Shared::default());
    let (sample_tx, sample_rx) = rtrb::RingBuffer::new(PLAYBACK_SAMPLES);
    let (piece_tx, piece_rx) = rtrb::RingBuffer::new(PIECES);
    let (event_tx, event_rx) = rtrb::RingBuffer::new(EVENTS);
    (
        Deck::new(shared.clone(), sample_rx, piece_rx, event_tx),
        Worker::new(shared, params, sample_tx, piece_tx, event_rx),
    )
}
