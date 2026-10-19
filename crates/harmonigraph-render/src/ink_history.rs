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
            // The shipped ink after Opacity, which is exactly the glow target
            // `NodeMotion::step` hands `GlowFade`; against the carried level
            // that is the test the CPU makes, so colour and brightness take
            // the same attack or release.
            let target = instance.params.into_iter().fold(0.0, f32::max).clamp(0.0, 1.0);
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
