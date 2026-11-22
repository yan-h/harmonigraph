//! What the take needs from the audio thread: which stretches recorded, where
//! each sits in the take's audio, and the configuration in force. Every record
//! is timed where it sounds, and the take's reader places it by that time
//! (#1760).
use crate::presentation::Mapping;
use harmonigraph_record::{configuration::RecordIntent, Recorder};

pub(crate) struct Recording {
    /// The Hub's mapping for this callback (`presentation`). A run states
    /// where it opens on it, and every delta is timed on it.
    pub presentation: Mapping,
    registered_through: i64,
    /// The intent captured at this callback's boundary.
    pub captured_intent: RecordIntent,
    pub block_start: i64,
    pub block_frames: u32,
}
impl Default for Recording {
    fn default() -> Self {
        Self {
            presentation: Mapping::default(),
            registered_through: i64::MIN,
            captured_intent: RecordIntent::default(),
            block_start: 0,
            block_frames: 0,
        }
    }
}
impl Recording {
    /// This block is registered through `end`.
    pub fn registered(&mut self, end: i64) {
        self.registered_through = end;
    }

    /// A joined boundary with the take disarmed: no callback can still close
    /// its run, so it closes here. For a host that stops calling the audio
    /// thread after a Stop.
    pub fn close_stopped(&mut self, recorder: &mut Recorder) {
        if !recorder.capture_recording_intent().is_armed() {
            self.end_run(recorder);
        }
    }

    /// Close the open run where this owner last registered: a run is open
    /// only after a block that recorded, which registered.
    fn end_run(&self, recorder: &mut Recorder) {
        if recorder.recording() {
            recorder.run_end(self.presentation.at(self.registered_through));
        }
    }

    /// A host reset cuts the take. A run open across it ends here, and the
    /// next one opens at or after that end on the same clock, even where the
    /// host restarts its steady counter, which the shell runs on across
    /// (`Boundary::steady_time`).
    pub fn reset(&mut self, recorder: &mut Recorder) {
        self.end_run(recorder);
        *self = Self::default();
    }
}

#[cfg(test)]
pub(super) mod tests;
