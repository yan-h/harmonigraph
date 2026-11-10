//! CPU bookkeeping owned by the GPU strip. Deriving or discarding a callback
//! cannot consume a seed or advance the clock of pixels that have not been drawn.

use harmonigraph_scene::GlowTiming;

use crate::GpuInstance;

#[derive(Clone, Copy, Default)]
struct Row {
    owner: u64,
    level: f32,
    seen: u64,
}

pub(super) struct InkHistory {
    at: Option<f64>,
    frame: u64,
    rows: Vec<Row>,
    /// Each shipped instance's coefficient from the last [`encode`](Self::encode),
    /// in instance order: what replaces its third glow value on the way to the
    /// GPU. Kept rather than a patched copy of every instance, which is the
    /// same frame again at forty times the size.
    mixes: Vec<f32>,
}

impl InkHistory {
    pub fn new(rows: u32) -> Self {
        Self { at: None, frame: 0, rows: vec![Row::default(); rows as usize], mixes: Vec::new() }
    }

    pub fn clear(&mut self) {
        if self.at.take().is_some() {
            self.frame = 0;
            self.rows.fill(Row::default());
        }
    }

    /// Called once for an ink pass that will be encoded. Absent rows are
    /// cleared by that pass, so returning after an absence must seed as well.
    /// Returns each instance's coefficient, its own third glow value where it
    /// carries no light.
    pub fn encode(
        &mut self,
        timing: GlowTiming,
        instances: &[GpuInstance],
        owners: &[u64],
    ) -> &[f32] {
        assert_eq!(instances.len(), owners.len());
        let (up, down) = timing.coefficients(self.at);
        self.at = Some(timing.now);
        self.frame += 1;
        self.mixes.clear();
        for (instance, &owner) in instances.iter().zip(owners) {
            if instance.glow[0] <= 0.0 {
                self.mixes.push(instance.glow[2]);
                continue;
            }
            let row = &mut self.rows[instance.glow[1] as usize];
            // The glow's own target (`GlowStep::target`), not the presence in
            // `params.x`, which holds through a release: against the carried
            // level that is the test `GlowFade` makes, so colour and
            // brightness take the same attack or release.
            let target = instance.params[3];
            self.mixes.push(if row.owner != owner || row.seen + 1 != self.frame || row.seen == 0 {
                1.0
            } else if target > row.level {
                up
            } else {
                down
            });
            *row = Row { owner, level: instance.glow[0], seen: self.frame };
        }
        &self.mixes
    }

    /// The coefficients the last encoded pass shipped.
    #[cfg(test)]
    pub fn mixes(&self) -> &[f32] {
        &self.mixes
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A released note's colour leaves at once while its presence holds for
    /// the slices still to retract. The strip mixes at the release rate,
    /// as `GlowFade` takes the brightness down, rather than at the attack
    /// rate because the presence is still above the carried light.
    #[test]
    fn a_release_under_a_held_presence_mixes_at_the_release_rate() {
        let lit = |presence, ink, carried| GpuInstance {
            params: [presence, 0.0, 0.0, ink],
            glow: [carried, 0.0, 1.0, 1.0],
            ..bytemuck::Zeroable::zeroed()
        };
        let timing = |now| GlowTiming { now, attack: 0.1, release: 1.0 };
        let mut history = InkHistory::new(1);
        // Already on its way out: the light carried below the held presence.
        history.encode(timing(0.0), &[lit(1.0, 0.6, 0.95)], &[7]);
        let (up, down) = timing(0.1).coefficients(Some(0.0));
        assert_ne!(up, down);
        let mixes = history.encode(timing(0.1), &[lit(1.0, 0.5, 0.9)], &[7]);
        assert_eq!(mixes[0], down, "the colour took the attack rate on its way out");
    }
}
