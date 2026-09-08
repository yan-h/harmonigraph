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
}

impl InkHistory {
    pub fn new(rows: u32) -> Self {
        Self { at: None, frame: 0, rows: vec![Row::default(); rows as usize] }
    }

    pub fn clear(&mut self) {
        if self.at.take().is_some() {
            self.frame = 0;
            self.rows.fill(Row::default());
        }
    }

    /// Called once for an ink pass that will be encoded. Absent rows are
    /// cleared by that pass, so returning after an absence must seed as well.
    pub fn encode(&mut self, timing: GlowTiming, instances: &mut [GpuInstance], owners: &[u64]) {
        assert_eq!(instances.len(), owners.len());
        let (up, down) = timing.coefficients(self.at);
        self.at = Some(timing.now);
        self.frame += 1;
        for (instance, &owner) in instances.iter_mut().zip(owners) {
            if instance.glow[0] <= 0.0 {
                continue;
            }
            let row = &mut self.rows[instance.glow[1] as usize];
            let target = instance.params.into_iter().fold(0.0, f32::max).clamp(0.0, 1.0);
            instance.glow[2] = if row.owner != owner || row.seen + 1 != self.frame || row.seen == 0
            {
                1.0
            } else if target > row.level {
                up
            } else {
                down
            };
            *row = Row { owner, level: instance.glow[0], seen: self.frame };
        }
    }
}
