//! The fold the live heatmap used before the slab pyramid (#1758), kept as
//! the tests' independent oracle: one grid per slab width, folded column by
//! column, with no ring, no shared silence and no fast path. The pyramid's
//! levels are held to it byte for byte.

use super::*;

/// One slab per slab width of elapsed time, oldest first. Finished slabs are
/// bytes; only the open one keeps sums, so continuing it after a read never
/// averages its quantized output. A slab is written only through
/// `Arc::make_mut`, as the pyramid's are.
#[derive(Default, Clone)]
pub(super) struct SlabGrid {
    mean: PowerMean,
    dirty: bool,
    pub(super) centers: Vec<f64>,
    pub(super) power: Vec<SpectrogramSlab>,
    cur_key: Option<i64>,
}

impl SlabGrid {
    /// Fold one column, in time order, into the slab of `bucket` seconds it
    /// lands in.
    pub(super) fn fold(&mut self, time: f64, power: &[f32], bucket: f64) {
        let key = (time / bucket).floor() as i64;
        if self.cur_key != Some(key) {
            self.finish();
            self.mean.clear();
            // A slab with no columns in it STILL gets a row, of silence, so
            // the grid stays one row per slab of elapsed time: the time axis
            // is uniform, so skipping one would stretch its neighbours over
            // it. Silence is what the analyzer had.
            let from = self.cur_key.map_or(key, |k| {
                assert!(key > k, "the oracle folds columns in time order");
                k + 1
            });
            for slot in from..=key {
                self.centers.push((slot as f64 + 0.5) * bucket);
                self.power.push(SpectrogramSlab::from([0; SPECTRUM_BINS]));
            }
            self.cur_key = Some(key);
        }
        self.mean.add(power);
        self.dirty = true;
    }

    pub(super) fn finish(&mut self) {
        if self.dirty {
            // Through `make_mut`: the last frame's snapshot may hold this slab,
            // and it must go on reading what it was handed.
            let open = self.power.last_mut().expect("a dirty grid has a current slab");
            self.mean.write(Arc::make_mut(open));
            self.dirty = false;
        }
    }
}
