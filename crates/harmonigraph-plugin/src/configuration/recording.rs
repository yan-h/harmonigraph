//! Original sample-to-pass ownership for deferred configuration. This is a take
//! publication fence, independent of the later session's timeline retirement.
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
}
pub(super) struct Recording {
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
        if self.segments.iter().flatten().any(|s| s.address.is_some() && s.end > self.prefix)
            || self.changes.iter().any(Option::is_some)
        {
            recorder.fail_configuration();
        }
        *self = Self::default();
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
                    *cell = Some(Pass { address, end: None, seeded: false, last: None });
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
            start,
            end,
            address: recorded_address,
            origin: origin.unwrap_or(0.0),
            rate,
            seeded: false,
        };
        if let Some(previous) = self.segments.iter_mut().flatten().find(|s| {
            s.end == start
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
            {
                recorder.configuration_pass_complete(pass.address);
                *cell = None;
            }
        }
        for cell in &mut self.segments {
            if cell.is_some_and(|s| s.end <= self.prefix && (s.address.is_none() || s.seeded))
                && !self
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
        }
    }
}
