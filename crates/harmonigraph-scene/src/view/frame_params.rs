//! [`FrameParams`] — the per-frame mirror of the host-automatable appearance
//! parameters, which the persist blob deliberately does not hold.

// For the doc links alone, which name `ViewConfig` to say what this is NOT a
// part of; see the same note in `ring_stack`.
#[allow(unused_imports)]
use super::*;

/// Per-frame mirrors of the host-automatable appearance parameters. The
/// shell copies these from its param backend every frame (see root_ui).
/// Deliberately NOT part of [`ViewConfig`] or the persist blob: the param
/// system owns these values, and persisting a copy would create a second
/// source of truth that's dead on arrival at load time.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct FrameParams {
    /// Seconds a note takes to arrive, and — once it is released — to leave,
    /// for EVERY layer of the node: the audio ring, the octave glyphs, and the
    /// melody/bass marks.
    ///
    /// One time per DIRECTION, so the lattice answers the keys the way it
    /// lets go of them; a note that came up quickly and left slowly would
    /// read as two instruments. It costs a short note no brightness, only
    /// time at full, because the arrival lands before the departure starts
    /// (see [`ViewConfig::envelope`]).
    ///
    /// One time per LAYER too, so an arrival or a release reads as a single
    /// gesture instead of pieces of the node moving at different rates. The
    /// octave sectors and the melody/bass marks arrive on the same ramp as
    /// each other, because a mark and the sector it links back to
    /// belong to one note — [`ViewConfig::mark_delay`] moves a ring's ramp
    /// LATER without changing its rate, which is the one thing that may
    /// differ.
    pub fade_time: f32,
    /// Pitch (MIDI note) mapped to the darkest gradient color.
    pub darkest_pitch: f32,
    /// Pitch mapped to the brightest gradient color.
    pub brightest_pitch: f32,
}

impl Default for FrameParams {
    fn default() -> Self {
        FrameParams { fade_time: 1.0, darkest_pitch: 24.0, brightest_pitch: 108.0 }
    }
}
