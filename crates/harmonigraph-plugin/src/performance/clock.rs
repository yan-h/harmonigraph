//! Immutable adopted calibration. Raw enclosing time and mapped session time
//! are separate domains; a coverage endpoint is exclusive, never a heartbeat.
use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Calibration {
    pub offset: i64,
    pub sample_rate: f64,
    pub max_frames: u32,
    pub validated: bool,
}

impl Calibration {
    pub fn matches(self, rate: f64, frames: u32) -> bool {
        self.validated
            && rate.is_finite()
            && rate > 0.0
            && self.sample_rate == rate
            && self.max_frames == frames
            && frames != 0
    }
    pub fn map(self, raw: i64) -> Option<i64> {
        raw.checked_add(self.offset)
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Coverage {
    pub start: i64,
    pub through: i64,
}

pub struct Clock {
    pub calibration: Calibration,
    pub coverage: Option<Coverage>,
    pub valid: bool,
    raw_through: Option<i64>,
}
impl Clock {
    pub fn new(calibration: Calibration, rate: f64, max_frames: u32) -> Self {
        Self {
            calibration,
            coverage: None,
            valid: calibration.matches(rate, max_frames),
            raw_through: None,
        }
    }
    pub fn begin(&mut self, raw: i64, frames: u32) -> Option<Coverage> {
        if !self.valid
            || raw < 0
            || frames == 0
            || frames > self.calibration.max_frames
            || self.raw_through.is_some_and(|end| raw != end)
        {
            self.valid = false;
            return None;
        }
        let Some(raw_end) = raw.checked_add(i64::from(frames)) else {
            self.valid = false;
            return None;
        };
        let (Some(start), Some(through)) =
            (self.calibration.map(raw), self.calibration.map(raw_end))
        else {
            self.valid = false;
            return None;
        };
        self.raw_through = Some(raw_end);
        let coverage = Coverage { start: self.coverage.map_or(start, |old| old.start), through };
        self.coverage = Some(coverage);
        Some(coverage)
    }
}
