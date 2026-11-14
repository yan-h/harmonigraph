//! The fold the live heatmap used before the slab pyramid (#1758), kept as
//! the tests' independent oracle: one grid per slab width, folded column by
//! column, with no ring, no shared silence and no fast path. The pyramid's
//! levels are held to it byte for byte.

use super::*;

/// Completed slabs stay byte-sized. Only the active slab retains sums/counts;
/// continuing it after a display read never averages its quantized output.
///
/// One shared [`SpectrogramSlab`] per slab rather than one flat byte run, so a
/// frame's snapshot is a list of pointers and the renderer can tell the slabs
/// that moved from the ones that did not without reading either. The contract
/// that makes that exact is the one [`SlabGrid::finish`] keeps: a slab is
/// written only through `Arc::make_mut`, so one a snapshot still holds is
/// copied rather than changed under it.
#[derive(Default, Clone)]
pub(super) struct SlabGrid {
    mean: PowerMean,
    dirty: bool,
    pub(super) centers: Vec<f64>,
    pub(super) power: Vec<SpectrogramSlab>,
    cur_key: Option<i64>,
}

/// A fresh slab of silence, which is what every slab starts as.
fn silent_slab() -> SpectrogramSlab {
    SpectrogramSlab::from([0; SPECTRUM_BINS])
}

impl SlabGrid {
    /// Fold one column (columns arrive oldest-first) into the grid, appending
    /// slabs and adding the column to the current power mean. Returns `false` iff
    /// the column ran BACKWARDS in time relative to the current slab — batch
    /// ignores the result (it just starts a fresh row, as before), while the
    /// incremental aggregator rebuilds if a backward column reaches this fold.
    /// Its admission search assumes ordered history, so this is not a general
    /// recovery path for unordered input.
    /// `min_key` is an explicit live retention bound; the batch driver passes
    /// `None`. One older seed may survive until an admitted column arrives.
    pub(super) fn fold(
        &mut self,
        time: f64,
        power: &[f32],
        count: u64,
        bucket: f64,
        min_key: Option<i64>,
    ) -> bool {
        let key = (time / bucket).floor() as i64;
        if self.cur_key != Some(key) {
            self.finish();
            self.mean.clear();
        }
        let forward = match self.cur_key {
            Some(k) if k == key => true,
            // A slab with no columns in it STILL gets a row, so the grid stays
            // one row per slab of elapsed time. The time axis is uniform —
            // `slab_at` maps time linearly across `w * bucket` — so skipping an
            // empty slab makes the rows either side of it neighbouring slabs,
            // and the quad then stretches that pair over
            // the whole silent stretch: one flat color as wide as the silence.
            // Analysis stalls do happen (switching the FFT window empties the
            // ring for a window's worth of samples), and that band was the
            // result. Silence is what the analyzer actually had.
            Some(k) if key > k => {
                self.fill_gap(k, key, bucket, min_key);
                self.centers.push((key as f64 + 0.5) * bucket);
                self.power.push(silent_slab());
                self.cur_key = Some(key);
                true
            }
            // First column (None), or time running backwards (Some, key < k):
            // start a fresh row rather than fill a negative gap. Only the
            // backward case breaks the incremental invariant.
            //
            // Not expected from today's callers, and `push_history` asserts
            // as much in debug builds. If a backward column reaches this fold,
            // `SpectrogramAgg::serve` rebuilds on the `false`; its admission
            // search can skip an out-of-order column before it gets here.
            other => {
                self.cur_key = Some(key);
                self.centers.push((key as f64 + 0.5) * bucket);
                self.power.push(silent_slab());
                other.is_none()
            }
        };
        self.mean.add(power, count);
        self.dirty = true;
        if let Some(min_key) = min_key {
            self.retain_from(min_key, bucket);
        }
        forward
    }

    /// The rows for the empty slabs strictly between finished slab `from` and
    /// slab `to`.
    fn fill_gap(&mut self, from: i64, to: i64, bucket: f64, min_key: Option<i64>) {
        let empty = to - from - 1;
        // The live owner admits only its retained interval. Keep the
        // original gap length for the hold decision: clipping a long
        // gap down to one empty slab must not turn silence into a hold.
        for slot in (from + 1).max(min_key.unwrap_or(i64::MIN))..to {
            self.centers.push((slot as f64 + 0.5) * bucket);
            if empty <= JITTER_SLABS {
                // Hold the previous column: a gap this short is a seam
                // in the sample stream (see `JITTER_SLABS`), and
                // painting it silent would leave a stripe of false
                // silence scrolling across the display for the rest
                // of the window. The held slab is already final, so
                // the copy shares it.
                let held = self.power.last().expect("a current slab").clone();
                self.power.push(held);
            } else {
                self.power.push(silent_slab());
            }
        }
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

    /// Keep one predecessor when the whole grid is older than the interval.
    /// It seeds a clipped jitter hold and establishes a real black gap; the
    /// next admitted column removes it after any copies have been made.
    pub(super) fn retain_from(&mut self, min_key: i64, bucket: f64) {
        let drop = self
            .centers
            .partition_point(|&c| ((c / bucket).floor() as i64) < min_key)
            .min(self.centers.len().saturating_sub(1));
        self.centers.drain(..drop);
        self.power.drain(..drop);
    }
}
