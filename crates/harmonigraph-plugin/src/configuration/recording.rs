//! Which recording pass and time origin own an output sample. Configuration is
//! block-granular, so a pass records the value adopted at each of its segments
//! rather than a history keyed on the sample an edit was observed at.
use harmonigraph_core::canonical::ClockId;
use harmonigraph_core::configuration::ResolvedConfig;
use harmonigraph_record::{
    configuration::{RecordAddress, RECORD_PASSES},
    Recorder,
};

// A segment lives only while output inside it is still unpublished, and
// contiguous callbacks on one pass and rate merge into one. Distinct spans
// therefore need a transport discontinuity or a pass change each; exhaustion
// is an explicit configuration failure, not growth on the audio thread.
const SEGMENTS: usize = 64;

#[derive(Clone, Copy)]
struct Segment {
    clock: ClockId,
    start: i64,
    end: i64,
    address: Option<RecordAddress>,
    origin: f64,
    rate: f64,
}
#[derive(Clone, Copy)]
struct Pass {
    address: RecordAddress,
    end: Option<i64>,
    /// The last configuration written to this pass, so a stable block writes
    /// nothing and a changed one writes once.
    last: Option<ResolvedConfig>,
    configuration_complete: bool,
    source_complete: bool,
}
pub(crate) struct Recording {
    pub clock: ClockId,
    /// Continuous presentation clock for publication controls and loss markers.
    pub observation_time: f64,
    source_prefix: Option<i64>,
    registered_through: i64,
    segments: [Option<Segment>; SEGMENTS],
    passes: [Option<Pass>; RECORD_PASSES],
    current: Option<RecordAddress>,
    pub captured_intent: u64,
    pub prefix: i64,
    pub block_start: i64,
    pub block_frames: u32,
    /// Terminal wrapper ownership, separate from successful configuration and
    /// source prefixes. Original recording routes remain until actual join cuts.
    pub retired_configuration: Option<bool>,
    pub retirement_finished: bool,
}
impl Default for Recording {
    fn default() -> Self {
        Self {
            clock: ClockId::default(),
            observation_time: 0.0,
            source_prefix: None,
            registered_through: i64::MIN,
            segments: [None; SEGMENTS],
            passes: [None; RECORD_PASSES],
            current: None,
            captured_intent: 0,
            prefix: 0,
            block_start: 0,
            block_frames: 0,
            retired_configuration: None,
            retirement_finished: false,
        }
    }
}
impl Recording {
    pub fn dispose_retired_configuration(&mut self, recorder: &mut Recorder) {
        let unfinished = self.retired_configuration.expect("joined configuration producer");
        // Retain every already-applied change at its original recording route.
        self.finish(recorder);
        if unfinished
            && self
                .segments
                .iter()
                .flatten()
                .any(|segment| segment.address.is_some() && segment.end > self.prefix)
        {
            recorder.fail_configuration();
        }
    }
    /// All original source producers have joined/sealed and every final actual
    /// cut has been published or explicitly lost. No synthetic prefix or seed.
    pub fn finish_retired_publication(&mut self, recorder: &mut Recorder, unknown_held: bool) {
        assert!(self.retired_configuration.is_some());
        if unknown_held
            || self.passes.iter().flatten().any(|pass| !pass.configuration_complete)
            || self
                .segments
                .iter()
                .flatten()
                .any(|segment| segment.address.is_some() && segment.end > self.prefix)
        {
            recorder.fail_configuration();
        }
        self.segments.fill(None);
        self.passes.fill(None);
        self.current = None;
        self.retirement_finished = true;
        recorder.retired_publication_complete();
    }
    pub fn reset(&mut self, recorder: &Recorder) {
        if self.segments.iter().flatten().any(|s| {
            s.address.is_some()
                && (s.end > self.prefix || self.source_prefix.is_none_or(|through| s.end > through))
        }) {
            recorder.fail_configuration();
        }
        let Some(epoch) = self.clock.epoch.checked_add(1) else {
            recorder.fail_configuration();
            return;
        };
        let clock = ClockId { epoch, ..self.clock };
        *self = Self { clock, ..Self::default() };
    }

    /// ONLY the canonical audio owner may supply this proof: every actual
    /// output strictly before `through` has acquired an immutable recording
    /// route or an explicit publication-failure disposition. Source receipt,
    /// held baseline and configuration progress are insufficient.
    ///
    /// A sample here is the Hub's own steady timeline, the same one the
    /// segments below are cut on, so there is no offset between them to apply.
    pub fn source_frontier(&mut self, clock: ClockId, through: i64) -> Result<(), ()> {
        if clock != self.clock {
            return Err(());
        }
        if self.source_prefix.is_some_and(|old| through < old) {
            return Err(());
        }
        self.source_prefix = Some(through);
        Ok(())
    }

    pub fn route(
        &self,
        clock: ClockId,
        sample: i64,
        presentation_time: f64,
    ) -> Result<harmonigraph_record::publication::Route, ()> {
        if clock != self.clock || !presentation_time.is_finite() {
            return Err(());
        }
        let segment = self
            .segments
            .iter()
            .flatten()
            .find(|s| s.clock == clock && s.start <= sample && sample < s.end)
            .ok_or(())?;
        let t = segment.origin + (sample - segment.start) as f64 / segment.rate;
        Ok(harmonigraph_record::publication::Route {
            address: segment.address,
            time_offset: t - presentation_time,
        })
    }
    pub fn segment(
        &mut self,
        recorder: &mut Recorder,
        origin: Option<f64>,
        rate: f64,
        config: ResolvedConfig,
    ) {
        let (start, end) = (self.block_start, self.block_start + i64::from(self.block_frames));
        let address = origin.and_then(|_| recorder.configuration_address());
        let next_current = if origin.is_some() {
            address
        } else if recorder.configuration_address().is_none() {
            None
        } else {
            self.current
        };
        if next_current != self.current {
            if let Some(old) = self.current {
                if let Some(pass) = self.passes.iter_mut().flatten().find(|p| p.address == old) {
                    pass.end = Some(start);
                } else {
                    recorder.fail_configuration();
                }
            }
            if let Some(address) = next_current {
                if let Some(cell) = self.passes.iter_mut().find(|p| p.is_none()) {
                    *cell = Some(Pass {
                        address,
                        end: None,
                        last: None,
                        configuration_complete: false,
                        source_complete: false,
                    });
                } else {
                    recorder.fail_configuration();
                }
            }
            self.current = next_current;
        }
        // Include explicit non-recording provenance. A later arm can never claim
        // input first observed while disarmed, even when that work drains late.
        let recorded_address = address
            .filter(|a| a.epoch == self.captured_intent >> 1 && self.captured_intent & 1 != 0);
        let segment = Segment {
            clock: self.clock,
            start,
            end,
            address: recorded_address,
            origin: origin.unwrap_or(0.0),
            rate,
        };
        if let Some(previous) = self.segments.iter_mut().flatten().find(|s| {
            s.clock == segment.clock
                && s.end == start
                && s.address == segment.address
                && s.rate == rate
                && (s.address.is_none()
                    || s.origin + (start - s.start) as f64 / rate == segment.origin)
        }) {
            previous.end = end;
        } else if let Some(cell) = self.segments.iter_mut().find(|s| s.is_none()) {
            *cell = Some(segment);
        } else {
            recorder.fail_configuration();
            return;
        }

        self.registered_through = end;
        // The block's own adopted configuration, written once per pass per
        // change. There is no historical lookup: the value in force over this
        // segment IS the value the Hub used for every group it started here.
        if let Some(address) = recorded_address {
            if let Some(pass) = self.passes.iter_mut().flatten().find(|p| p.address == address) {
                if pass.last != Some(config) {
                    recorder.configuration_at(address, segment.origin, config);
                    pass.last = Some(config);
                }
            } else {
                recorder.fail_configuration();
            }
        }
        self.finish(recorder);
    }

    pub fn finish(&mut self, recorder: &mut Recorder) {
        for cell in &mut self.passes {
            let Some(pass) = cell.as_mut() else {
                continue;
            };
            if pass.last.is_some()
                && pass.end.is_some_and(|end| self.prefix >= end)
                && !pass.configuration_complete
            {
                recorder.configuration_pass_complete(pass.address);
                pass.configuration_complete = true;
            }
            if pass.end.is_some_and(|end| self.source_prefix.is_some_and(|prefix| prefix >= end))
                && !pass.source_complete
            {
                recorder.source_pass_complete(pass.address, self.observation_time);
                pass.source_complete = true;
            }
            if pass.configuration_complete && pass.source_complete {
                *cell = None;
            }
        }
        // A segment is only a route for output that has not been published
        // yet; both frontiers past its end retire it.
        for cell in &mut self.segments {
            if cell.is_some_and(|s| {
                s.end <= self.prefix && self.source_prefix.is_some_and(|through| s.end <= through)
            }) {
                *cell = None;
            }
        }
        // A concurrent GUI stop cannot close an epoch whose armed callback
        // may still begin another recording segment later in this same block.
        let epoch = self.captured_intent >> 1;
        if self.captured_intent & 1 == 0
            && self.current.is_none()
            && !self.passes.iter().flatten().any(|p| p.address.epoch == epoch)
        {
            recorder.configuration_epoch_complete(epoch);
            recorder.source_epoch_complete(epoch, self.observation_time);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use harmonigraph_core::configuration::ConfigReducer;

    /// Every sample crossing this boundary is already on the Hub's own steady
    /// timeline, which is the timeline the segments are cut on: a route asks
    /// for the sample it means, with nothing subtracted on the way in.
    #[test]
    fn output_frontier_and_recording_routes_read_samples_on_the_adopted_hub_clock() {
        let (mut recorder, capture) = harmonigraph_record::testing::channel();
        recorder.enable_configuration();
        recorder.enable_canonical();
        capture.arm();
        recorder.is_armed();
        recorder.observe_transport(20.0, true, 64.0 / 48_000.0);
        let clock = ClockId { runtime_session: 7, epoch: 3 };
        let mut recording = Recording {
            clock,
            captured_intent: 3,
            prefix: 1040,
            block_start: 1000,
            block_frames: 40,
            ..Default::default()
        };
        let config = ConfigReducer::default().resolved();
        recording.segment(&mut recorder, Some(20.0), 48000.0, config);
        recording.source_frontier(clock, 1024).unwrap();
        recording.finish(&mut recorder);
        assert_eq!(recording.source_prefix, Some(1024));
        assert_eq!(
            recording.segments.iter().flatten().count(),
            1,
            "configuration prefix1040 cannot retire source prefix1024"
        );
        // Sample 1032 is offset 32 into the segment that starts at 1000, so it
        // is 32 samples past that segment's 20.0 s origin.
        let route = recording.route(clock, 1032, 1032.0 / 48000.0).unwrap();
        assert_eq!(route.address, Some(RecordAddress { epoch: 1, pass: 1 }));
        assert!((1032.0 / 48000.0 + route.time_offset - (20.0 + 32.0 / 48000.0)).abs() < 1e-12);
        assert!(recording.route(clock, 1039, 0.0).unwrap().address.is_some());
        // Explicit disarmed span stays distinct from a later recording segment.
        capture.stop();
        recorder.is_armed();
        recording.captured_intent = 2;
        recording.block_start = 1040;
        recording.block_frames = 40;
        recording.segment(&mut recorder, None, 48000.0, config);
        assert_eq!(recording.route(clock, 1040, 0.0).unwrap().address, None);
        capture.arm();
        recorder.is_armed();
        recording.captured_intent = recorder.capture_recording_intent();
        recording.block_start = 1080;
        recording.segment(&mut recorder, Some(5.0), 48000.0, config);
        assert_eq!(recording.route(clock, 1040, 0.0).unwrap().address, None);
        let new_clock = ClockId { epoch: 4, ..clock };
        assert!(recording.route(new_clock, 1032, 0.0).is_err());
        assert!(recording.source_frontier(new_clock, 2000).is_err());
        assert_eq!(recording.source_prefix, Some(1024));
        recording.reset(&recorder);
        recording.block_start = 0;
        recording.block_frames = 64;
        recording.segment(&mut recorder, None, 48000.0, config);
        assert!(
            recording.route(clock, 32, 0.0).is_err(),
            "old epoch raw32 must not match new epoch [0,64)"
        );
    }
}
