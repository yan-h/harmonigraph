//! Adaptive tuning transport: the Tune's delay line and the Hub's sequencer.
//!
//! The shape is the fault cut of #786. A Tune copies each input to the Hub and
//! emits it D samples later, with the correction that came back or without one;
//! nothing waits on anyone. The only lifecycle event is the cut, and a fault is
//! a status bit rather than a gate. The musical policy it calls is unchanged.
pub(crate) mod diagnostics;
pub mod event;
pub(crate) mod hub;
pub(crate) mod instances;
#[cfg(target_os = "macos")]
mod native;
mod neighbourhood;
pub(crate) mod plugin;
mod queue;
pub(crate) mod session;
pub(crate) mod setup;
mod state;
pub(crate) mod tune;

#[cfg(test)]
mod tests;

/// Tune rows one Hub sequences. The Hub's own input is a seventeenth source
/// with a reserved index, so a paired Tune never collides with it.
pub const TUNERS: usize = 16;
pub const DIRECT: u8 = TUNERS as u8;
/// Voices one source may hold, and across the whole session.
pub const HELD_PER_SOURCE: usize = 64;
pub const HELD_SESSION: usize = 256;
/// Steps on the Tuning delay parameter. Sixteen still reaches the 512 samples
/// this delay used to be fixed at even from a 32-frame callback.
pub const DELAY_MULTIPLIER_MAX: i32 = 16;
/// Events one Tune may hold in its delay line. One input takes one cell,
/// whether or not it addresses a note.
pub const PENDING_EVENTS: usize = 8192;
/// Copied records in flight from one Tune to the Hub, and corrections coming
/// back. A full ring drops the copy and that note sounds uncorrected.
pub const CAPTURE_RING: usize = 1024;
pub const REPLY_RING: usize = 1024;
/// Records the Hub orders in one pass across every source.
pub const BATCH_EVENTS: usize = 2048;
/// Note-offs and pedal neutralisations one cut may owe: every held voice, plus
/// sustain, sostenuto and legato on each of sixteen channels. A cut does not
/// recentre the bend, because a Tune does not track one.
pub const CUT_EVENTS: usize = HELD_PER_SOURCE + 16 * 3;
