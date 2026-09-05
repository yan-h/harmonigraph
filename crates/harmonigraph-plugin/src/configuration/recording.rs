//! Original sample-to-pass ownership for deferred configuration. This is a take
//! publication fence, independent of the later session's timeline retirement.
use harmonigraph_core::canonical::ClockId;
use harmonigraph_core::configuration::{timeline::ConfigTimeline, ResolvedConfig};
use harmonigraph_record::{
    configuration::{RecordAddress, RECORD_PASSES},
    Recorder,
};

// Every unconsumed input or command may keep a different segment alive. Include
// the current callback and a silent learning boundary; no heap growth on audio.
const SEGMENTS: usize = 2048 + 128 + 2;
const CHANGES: usize = 128;

#[derive(Clone, Copy)]
struct Segment {
    clock: ClockId,
    start: i64,
    end: i64,
    address: Option<RecordAddress>,
    origin: f64,
    rate: f64,
    seeded: bool,
}
#[derive(Clone, Copy)]
struct Pass {
    address: RecordAddress,
    end: Option<i64>,
    seeded: bool,
    last: Option<(i64, ResolvedConfig)>,
    configuration_complete: bool,
    source_complete: bool,
}
pub(super) struct Recording {
    pub clock: ClockId,
    pub hub_offset: i64,
    /// Continuous presentation clock for publication controls and loss markers.
    pub observation_time: f64,
    source_prefix: Option<i64>,
    segments: [Option<Segment>; SEGMENTS],
    changes: [Option<(i64, ResolvedConfig)>; CHANGES],
    passes: [Option<Pass>; RECORD_PASSES],
    current: Option<RecordAddress>,
    pub captured_intent: u64,
    pub prefix: i64,
    pub block_start: i64,
    pub block_frames: u32,
}
impl Default for Recording {
    fn default() -> Self {
        Self {
            clock: ClockId::default(),
            hub_offset: 0,
            observation_time: 0.0,
            source_prefix: None,
            segments: [None; SEGMENTS],
            changes: [None; CHANGES],
            passes: [None; RECORD_PASSES],
            current: None,
            captured_intent: 0,
            prefix: 0,
            block_start: 0,
            block_frames: 0,
        }
    }
}
impl Recording {
    pub fn reset(&mut self, recorder: &Recorder) {
        if self.segments.iter().flatten().any(|s| {
            s.address.is_some()
                && (s.end > self.prefix || self.source_prefix.is_none_or(|through| s.end > through))
        }) || self.changes.iter().any(Option::is_some)
        {
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
    /// output strictly before mapped_through has acquired an immutable recording
    /// route or an explicit publication-failure disposition. Source receipt,
    /// held baseline and configuration progress are insufficient.
    pub fn source_frontier(&mut self, clock: ClockId, mapped_through: i64) -> Result<(), ()> {
        if clock != self.clock {
            return Err(());
        }
        let raw = mapped_through.checked_sub(self.hub_offset).ok_or(())?;
        if self.source_prefix.is_some_and(|old| raw < old) {
            return Err(());
        }
        self.source_prefix = Some(raw);
        Ok(())
    }

    pub fn route(
        &self,
        clock: ClockId,
        mapped_actual: i64,
        presentation_time: f64,
    ) -> Result<harmonigraph_record::publication::Route, ()> {
        if clock != self.clock || !presentation_time.is_finite() {
            return Err(());
        }
        let sample = mapped_actual.checked_sub(self.hub_offset).ok_or(())?;
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
    pub fn change(&mut self, sample: i64, config: ResolvedConfig, recorder: &Recorder) {
        if let Some(cell) = self.changes.iter_mut().find(|c| c.is_none()) {
            *cell = Some((sample, config));
        } else {
            recorder.fail_configuration();
        }
    }
    pub fn segment(
        &mut self,
        recorder: &mut Recorder,
        timeline: &ConfigTimeline,
        origin: Option<f64>,
        rate: f64,
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
                        seeded: false,
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
            seeded: false,
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
        }

        self.finish(recorder, timeline);
    }

    pub fn finish(&mut self, recorder: &mut Recorder, timeline: &ConfigTimeline) {
        // Seed the first segment and any resumed recording from the value at
        // its actual start. Stable continuous callbacks write no repeated state.
        for segment in self.segments.iter_mut().flatten() {
            let Some(address) = segment.address else {
                continue;
            };
            if !segment.seeded && self.prefix > segment.start {
                let Some(pass) = self.passes.iter_mut().flatten().find(|p| p.address == address)
                else {
                    recorder.fail_configuration();
                    continue;
                };
                match timeline.configuration_at(segment.start) {
                    Ok(config) => {
                        if pass
                            .last
                            .is_none_or(|(sample, old)| sample > segment.start || old != config)
                            || !pass.seeded
                        {
                            recorder.configuration_at(address, segment.origin, config);
                        }
                        if pass.last.is_none_or(|(sample, _)| sample <= segment.start) {
                            pass.last = Some((segment.start, config));
                        }
                        segment.seeded = true;
                        pass.seeded = true;
                    }
                    Err(_) => recorder.fail_configuration(),
                }
            }
        }

        // Route every change by the segment that originally owned its sample,
        // including changes applied now for an earlier callback/pass.
        for cell in &mut self.changes {
            let Some((sample, config)) = *cell else {
                continue;
            };
            if let Some(segment) =
                self.segments.iter().flatten().find(|s| s.start <= sample && sample < s.end)
            {
                if let Some(address) = segment.address {
                    let t = segment.origin + (sample - segment.start) as f64 / segment.rate;
                    recorder.configuration_at(address, t, config);
                    if let Some(pass) =
                        self.passes.iter_mut().flatten().find(|p| p.address == address)
                    {
                        if pass.last.is_none_or(|(old, _)| old <= sample) {
                            pass.last = Some((sample, config));
                        }
                    } else {
                        recorder.fail_configuration();
                    }
                }
                *cell = None;
            }
        }
        for cell in &mut self.passes {
            let Some(pass) = cell.as_mut() else {
                continue;
            };
            if pass.seeded
                && pass.end.is_some_and(|end| self.prefix >= end)
                && !self.changes.iter().flatten().any(|(sample, _)| *sample < pass.end.unwrap())
                && !self
                    .segments
                    .iter()
                    .flatten()
                    .any(|s| s.address == Some(pass.address) && !s.seeded)
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
        for cell in &mut self.segments {
            if cell.is_some_and(|s| {
                s.end <= self.prefix
                    && self.source_prefix.is_some_and(|through| s.end <= through)
                    && (s.address.is_none() || s.seeded)
            }) && !self
                .changes
                .iter()
                .flatten()
                .any(|(sample, _)| cell.is_some_and(|s| s.start <= *sample && *sample < s.end))
            {
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

    #[test]
    fn mapped_output_frontier_and_recording_routes_use_the_adopted_hub_clock_once() {
        let (mut recorder, capture) = harmonigraph_record::testing::channel();
        recorder.enable_configuration();
        recorder.enable_canonical();
        capture.arm();
        recorder.is_armed();
        recorder.observe_transport(20.0, true);
        let clock = ClockId { runtime_session: 7, epoch: 3 };
        let mut recording = Recording {
            clock,
            hub_offset: 32,
            captured_intent: 3,
            prefix: 1040,
            block_start: 1000,
            block_frames: 40,
            ..Default::default()
        };
        let timeline = ConfigTimeline::new(ConfigReducer::default());
        recording.segment(&mut recorder, &timeline, Some(20.0), 48000.0);
        recording.source_frontier(clock, 1056).unwrap();
        recording.finish(&mut recorder, &timeline);
        assert_eq!(recording.source_prefix, Some(1024));
        assert_eq!(
            recording.segments.iter().flatten().count(),
            1,
            "configuration prefix1040 cannot retire source prefix1024"
        );
        // source raw1000 +source offset64 = mapped1064; subtract HUB offset32
        // once to find raw1032 in the original hub recording segment.
        let route = recording.route(clock, 1064, 1064.0 / 48000.0).unwrap();
        assert_eq!(route.address, Some(RecordAddress { epoch: 1, pass: 1 }));
        assert!((1064.0 / 48000.0 + route.time_offset - (20.0 + 32.0 / 48000.0)).abs() < 1e-12);
        assert!(recording.route(clock, 1071, 0.0).unwrap().address.is_some());
        // Explicit disarmed span stays distinct from a later recording segment.
        capture.stop();
        recorder.is_armed();
        recording.captured_intent = 2;
        recording.block_start = 1040;
        recording.block_frames = 40;
        recording.segment(&mut recorder, &timeline, None, 48000.0);
        assert_eq!(recording.route(clock, 1072, 0.0).unwrap().address, None);
        capture.arm();
        recorder.is_armed();
        recording.captured_intent = recorder.capture_recording_intent();
        recording.block_start = 1080;
        recording.segment(&mut recorder, &timeline, Some(5.0), 48000.0);
        assert_eq!(recording.route(clock, 1072, 0.0).unwrap().address, None);
        let new_clock = ClockId { epoch: 4, ..clock };
        assert!(recording.route(new_clock, 1064, 0.0).is_err());
        assert!(recording.source_frontier(new_clock, 2000).is_err());
        assert_eq!(recording.source_prefix, Some(1024));
        recording.reset(&recorder);
        recording.block_start = 0;
        recording.block_frames = 64;
        recording.segment(&mut recorder, &timeline, None, 48000.0);
        assert!(
            recording.route(clock, 32, 0.0).is_err(),
            "old epoch raw32 must not match new epoch [0,64)"
        );
    }
}
