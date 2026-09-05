//! Resolved configuration boundaries. Replay consumes these values verbatim;
//! it never reruns detection or learning at a video frame boundary.
use harmonigraph_core::configuration::{PolicyConfig, ResolvedConfig, TuningModes};
use harmonigraph_core::{Tempered, Tuning};
use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct PolicyRecord {
    pub version: u32,
    pub domain: [u16; 3],
    pub candidate_radius: u32,
    pub context_radius: u32,
    pub context_weight: u16,
    pub history_weight: u16,
    pub origin_weight: u16,
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
            policy: PolicyRecord {
                version: p.version,
                domain: p.domain,
                candidate_radius: p.candidate_radius,
                context_radius: p.context_radius,
                context_weight: p.context_weight,
                history_weight: p.history_weight,
                origin_weight: p.origin_weight,
            },
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
            policy: PolicyConfig {
                version: p.version,
                domain: p.domain,
                candidate_radius: p.candidate_radius,
                context_radius: p.context_radius,
                context_weight: p.context_weight,
                history_weight: p.history_weight,
                origin_weight: p.origin_weight,
            },
        }
    }
}
