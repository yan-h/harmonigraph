//! What "the transport is rolling" means, for every reader of the host's
//! position: the take's lifecycle and the plugin's always-on tape (#1719).
//!
//! The host's `playing` flag alone is not it. An export reports
//! `playing = false` while its position climbs block by block, and both readers
//! must treat that as playback. So a block also rolls when it advanced from the
//! previous one by about that block's length.

/// How far behind the previous block a playing host's position may land
/// before it counts as a jump back: playback's jitter, not a loop wrap.
pub const JITTER_SECONDS: f64 = 0.05;

/// How one block's position relates to the previous block's.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Step {
    /// Behind the previous block: by more than playback's 50 ms jitter while
    /// playing, by anything while stopped. It rolls only if the host says it
    /// is playing, as a loop wrap does.
    Back { rolling: bool },
    /// Anything else, including the first block with no previous one.
    Ahead { rolling: bool },
}

impl Step {
    pub fn rolling(self) -> bool {
        match self {
            Step::Back { rolling } | Step::Ahead { rolling } => rolling,
        }
    }
}

/// Classify a block at `position` against the `previous` block's
/// `(position, duration)`.
///
/// Continuity uses the PREVIOUS callback's size, inclusive +/-50% (#839): a
/// tiny scrub inside that interval reads as one rolled block, exactly as a
/// stopped export does.
pub fn step(previous: Option<(f64, f64)>, position: f64, playing: bool) -> Step {
    match previous {
        Some((last, _)) if position < last - if playing { JITTER_SECONDS } else { 0.0 } => {
            Step::Back { rolling: playing }
        }
        Some((last, previous_duration)) => {
            let continuous = previous_duration.is_finite()
                && previous_duration > 0.0
                && position >= last + previous_duration * 0.5
                && position <= last + previous_duration * 1.5;
            Step::Ahead { rolling: playing || continuous }
        }
        None => Step::Ahead { rolling: playing },
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const BLOCK: f64 = 64.0 / 48_000.0;

    #[test]
    fn a_stopped_position_rolls_only_when_it_advanced_by_about_one_block() {
        let previous = Some((10.0, BLOCK));
        assert_eq!(step(previous, 10.0 + BLOCK, false), Step::Ahead { rolling: true });
        assert_eq!(step(previous, 10.0 + BLOCK * 0.5, false), Step::Ahead { rolling: true });
        assert_eq!(step(previous, 10.0 + BLOCK * 1.5, false), Step::Ahead { rolling: true });
        // A parked playhead, and a click forward, are not playback.
        assert_eq!(step(previous, 10.0, false), Step::Ahead { rolling: false });
        assert_eq!(step(previous, 12.0, false), Step::Ahead { rolling: false });
        assert_eq!(step(previous, 9.0, false), Step::Back { rolling: false });
        assert_eq!(step(None, 10.0, false), Step::Ahead { rolling: false });
    }

    #[test]
    fn playing_rolls_everywhere_and_tolerates_jitter_before_calling_it_a_jump_back() {
        let previous = Some((10.0, BLOCK));
        assert_eq!(step(previous, 9.96, true), Step::Ahead { rolling: true });
        assert_eq!(step(previous, 2.0, true), Step::Back { rolling: true });
        assert_eq!(step(None, 0.0, true), Step::Ahead { rolling: true });
    }
}
