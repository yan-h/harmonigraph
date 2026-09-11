//! Read-only publication of the sequencer's next-attack context. No UI state
//! feeds the musical owner, and the expensive winner calculation runs in UI.
use harmonigraph_core::{
    configuration::PolicyConfig,
    policy::{self, reach::Snapshot, ContextPitch, MusicalConfig},
    LatticePos, Tempered,
};
const HEADER: usize = 18;
const WORDS: usize = HEADER + (policy::MAX_CONTEXT + policy::MAX_MEMORY) * 6;
#[derive(Default)]
pub(super) struct Published(super::diagnostics::Snapshot<WORDS>);
impl Published {
    pub fn publish(&self, config: MusicalConfig, reference: i64, context: &[ContextPitch]) {
        let mut words = [0; WORDS];
        words[0] = reference;
        words[1] = context.len() as i64;
        words[2] = i64::from(config.c_offset);
        words[3..6].copy_from_slice(&config.axes.map(i64::from));
        words[6] =
            i64::from(config.tempered.syntonic) | i64::from(config.tempered.septimal_kleisma) << 1;
        words[7..17].copy_from_slice(&config.policy.words().map(i64::from));
        for (v, w) in context.iter().zip(words[HEADER..].chunks_exact_mut(6)) {
            w[0] = v.pitch;
            w[1] = v.weight.to_bits() as i64;
            if let Some(n) = v.node {
                w[2] = 1;
                w[3] = i64::from(n.threes);
                w[4] = i64::from(n.fives);
                w[5] = i64::from(n.sevens);
            }
        }
        self.0.publish(words);
    }
    pub fn read(&self) -> Option<Snapshot> {
        let w = self.0.read()?;
        let config = MusicalConfig {
            c_offset: w[2] as i32,
            axes: [w[3] as i32, w[4] as i32, w[5] as i32],
            tempered: Tempered { syntonic: w[6] & 1 != 0, septimal_kleisma: w[6] & 2 != 0 },
            policy: PolicyConfig::from_words(std::array::from_fn(|i| w[7 + i] as i32)),
        };
        let context = w[HEADER..]
            .chunks_exact(6)
            .take(w[1] as usize)
            .map(|v| ContextPitch {
                pitch: v[0],
                weight: f64::from_bits(v[1] as u64),
                node: (v[2] != 0).then(|| LatticePos::new(v[3] as i32, v[4] as i32, v[5] as i32)),
            })
            .collect();
        Some(Snapshot { config, reference: w[0], context })
    }
}
