//! Resolved configuration boundaries. Replay consumes these values verbatim;
//! it never reruns detection or learning at a video frame boundary.
use harmonigraph_core::configuration::{PolicyConfig, ResolvedConfig, TuningModes};
use harmonigraph_core::{Tempered, Tuning};
use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct PolicyRecord {
    pub version: u32,
    pub radius: u8,
    pub axes: u8,
    pub memory: u8,
    pub harmonic: u16,
    pub pitch_scale: u16,
    pub released: u16,
    pub recency: u16,
    pub register_floor: u16,
    pub register_falloff: u16,
    pub tolerance: u32,
    pub silence_ms: u32,
    pub reset_stop: bool,
    pub reset_loop: bool,
}
impl Default for PolicyRecord {
    fn default() -> Self {
        PolicyConfig::default().into()
    }
}
impl From<PolicyConfig> for PolicyRecord {
    fn from(p: PolicyConfig) -> Self {
        Self {
            version: p.version,
            radius: p.radius,
            axes: p.axes,
            memory: p.memory,
            harmonic: p.harmonic,
            pitch_scale: p.pitch_scale,
            released: p.released,
            recency: p.recency,
            register_floor: p.register_floor,
            register_falloff: p.register_falloff,
            tolerance: p.tolerance,
            silence_ms: p.silence_ms,
            reset_stop: p.reset_stop,
            reset_loop: p.reset_loop,
        }
    }
}
impl From<PolicyRecord> for PolicyConfig {
    fn from(p: PolicyRecord) -> Self {
        Self {
            version: p.version,
            radius: p.radius,
            axes: p.axes,
            memory: p.memory,
            harmonic: p.harmonic,
            pitch_scale: p.pitch_scale,
            released: p.released,
            recency: p.recency,
            register_floor: p.register_floor,
            register_falloff: p.register_falloff,
            tolerance: p.tolerance,
            silence_ms: p.silence_ms,
            reset_stop: p.reset_stop,
            reset_loop: p.reset_loop,
        }
        .sanitize()
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct ConfigurationRecord {
    pub t: f64,
    pub revision: u64,
    pub axes: [i32; 5],
    pub tempered: [bool; 2],
    pub auto: [bool; 2],
    pub learning: bool,
    pub policy: PolicyRecord,
}
impl Default for ConfigurationRecord {
    fn default() -> Self {
        Self::new(0.0, harmonigraph_core::configuration::ConfigReducer::default().resolved())
    }
}
impl ConfigurationRecord {
    pub fn new(t: f64, config: ResolvedConfig) -> Self {
        let p = config.policy;
        Self {
            t,
            revision: config.revision,
            axes: [
                config.tuning.c_offset,
                config.tuning.three,
                config.tuning.five,
                config.tuning.seven,
                config.tuning.tolerance,
            ],
            tempered: [config.modes.tempered.syntonic, config.modes.tempered.septimal_kleisma],
            auto: config.modes.auto,
            learning: config.modes.learning,
            policy: p.into(),
        }
    }
    pub fn resolved(self) -> ResolvedConfig {
        let p = self.policy;
        ResolvedConfig {
            revision: self.revision,
            tuning: Tuning {
                c_offset: self.axes[0],
                three: self.axes[1],
                five: self.axes[2],
                seven: self.axes[3],
                tolerance: self.axes[4],
            },
            modes: TuningModes {
                tempered: Tempered {
                    syntonic: self.tempered[0],
                    septimal_kleisma: self.tempered[1],
                },
                auto: self.auto,
                learning: self.learning,
            },
            policy: p.into(),
        }
    }
}
