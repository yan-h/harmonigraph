//! The spectrogram's stored history: what the analyzer measured, kept over
//! time in the form the heatmap reads it back.
//!
//! [`spectrum`](crate::spectrum) answers "what is sounding now"; this answers
//! "what has been sounding", which is a memory question — one column is a whole
//! pitch spectrum, and they arrive at the analyzer's rate for as long as the
//! plugin is open. Two decisions do the work, and both come from what the
//! display can actually show:
//!
//! - **A bucket is stored as quantized dB, not a float of power.** The heatmap
//!   maps a bucket through `10*log10` into a colour ramp, so dB is the domain it
//!   is read in, and a grid over it costs a quarter of what a float does at a
//!   byte per bucket. MAX (which is all the aggregation ever does) is
//!   order-preserving under a monotone encoding, so nothing downstream has to
//!   decode first. Whether a byte's half a dB is fine enough to be invisible is
//!   what [`coarsen`] and the pane's Fine levels toggle are for; until that is
//!   settled the store is sixteen bits and the toggle picks the picture.
//! - **Old columns are merged as they age.** The heatmap draws a window ending
//!   at `now`, so a column of age `a` is only ever on screen when the window is
//!   at least `a` long — and the window is cut into at most a few hundred time
//!   slabs, which puts a floor of roughly `a / 512` on the slab a column of that
//!   age can land in. Keeping 20 ms columns from ten minutes ago is therefore
//!   paying to store detail that no window can ever resolve. [`SpectrumHistory`]
//!   keeps the recent stretch at full rate and MAX-merges pairs as they fall
//!   past each tier, so resolution decays with age exactly as fast as the
//!   display's ability to show it.
//!
//! Together they turn the reach/memory trade from linear into logarithmic:
//! every extra tier doubles how far back the heatmap reaches for a fixed
//! [`SpectrumHistory::COARSE_COLUMNS`] more columns.

use std::collections::VecDeque;

use crate::spectrum::SPECTRUM_BINS;

/// One stored bucket: power in dB, quantized onto a fixed grid. See
/// [`quantize`].
///
/// Sixteen bits while the [`coarsen`] A/B is in place — the question it exists
/// to answer is whether the eight-bit grid's half a dB is visible as banding,
/// and that can only be judged against the finer picture. Once it is answered
/// this goes back to `u8` (and [`coarsen`] goes away), or stays and the toggle
/// does.
pub type BucketDb = u16;

/// One stored column's worth of buckets.
pub type ColumnDb = [BucketDb; SPECTRUM_BINS];

/// The dB a stored `0` stands for, and the step between stored values.
///
/// The pair covers -120 dB to +11 dB. The floor is exactly where the display's
/// own mapping bottoms out (`loudness` clamps power at 1e-12) and exactly where
/// the heatmap's range bar stops, so the quietest cell the UI can ask to see is
/// the quietest value there is — the encoding adds no floor of its own, and
/// nothing fades out early against one. The ceiling sits above a full-scale
/// sine's 0 dB, which is already saturated white at any range the bars allow,
/// so nothing visible clips against it either.
pub const DB_FLOOR: f32 = -120.0;
pub const DB_STEP: f32 = 0.002;

/// The step an eight-bit store would have had, and what [`coarsen`] rounds to.
///
/// An exact multiple of [`DB_STEP`], so coarsening lands on grid points and
/// reproduces the byte-per-bucket picture exactly rather than approximating it.
pub const COARSE_DB_STEP: f32 = 0.5;

/// Round a stored value onto the eight-bit grid — the A/B's coarse side.
///
/// Applied when the heatmap DRAWS, not when a column is stored, so flipping the
/// toggle re-renders the whole accumulated history at the chosen depth instead
/// of only what arrives next. Comparing two pictures of the same audio is the
/// entire point; comparing a picture against the seam where the setting changed
/// would answer nothing.
pub fn coarsen(bucket: BucketDb) -> BucketDb {
    let per_step = (COARSE_DB_STEP / DB_STEP).round(); // 250
    ((bucket as f32 / per_step).round() * per_step) as BucketDb
}

/// Power at or below [`DB_FLOOR`] — the many empty buckets of a typical
/// spectrum, which [`quantize`] answers without reaching for a `log10`.
const POWER_FLOOR: f32 = 1e-12;

/// Store a bucket's absolute power (as [`crate::spectrum::SpectrumAnalyzer`]
/// reports it) on the dB grid.
pub fn quantize(power: f32) -> BucketDb {
    if power <= POWER_FLOOR {
        return 0;
    }
    let db = 10.0 * power.log10();
    ((db - DB_FLOOR) / DB_STEP).round().clamp(0.0, BucketDb::MAX as f32) as BucketDb
}

/// The dB a stored value stands for — the inverse of [`quantize`], to within
/// half a step.
pub fn db_of(bucket: BucketDb) -> f32 {
    DB_FLOOR + bucket as f32 * DB_STEP
}

/// One column of the spectrogram: the raw spectrum at a moment, on the shell
/// clock, so it can be placed on the roll's time axis.
///
/// Stored as quantized dB (see the module docs), which is also the domain the
/// heatmap colours from — so a column is read straight through without a decode
/// step, and a merge is an element-wise MAX.
pub struct SpectrogramColumn {
    pub time: f64,
    pub db: Box<ColumnDb>,
}

impl SpectrogramColumn {
    /// Quantize a freshly analyzed power spectrum into a stored column.
    pub fn from_power(time: f64, power: &[f32; SPECTRUM_BINS]) -> SpectrogramColumn {
        let mut db = Box::new([0; SPECTRUM_BINS]);
        for (out, &p) in db.iter_mut().zip(power.iter()) {
            *out = quantize(p);
        }
        SpectrogramColumn { time, db }
    }

    /// Absorb an OLDER column: the pair becomes one column standing for the
    /// whole interval, holding the loudest each bucket reached across it.
    ///
    /// MAX, not a mean, for the same reason the display aggregates by MAX — a
    /// spectrogram cell answers "was there anything here", and averaging a
    /// bright thin partial with its quiet neighbour answers "not much". The
    /// timestamp lands at the midpoint, the least any peak inside can be moved.
    fn absorb(&mut self, older: &SpectrogramColumn) {
        for (mine, &theirs) in self.db.iter_mut().zip(older.db.iter()) {
            *mine = (*mine).max(theirs);
        }
        self.time = 0.5 * (self.time + older.time);
    }
}

/// The spectrogram's history: every column ever pushed, oldest first, at a
/// resolution that decays with age.
///
/// Columns enter tier 0 as they are analyzed. When a tier is full its two
/// oldest columns are MAX-merged into one and handed to the next tier, so tier
/// `k` holds columns spaced `2^k` analysis intervals apart and the whole
/// structure reaches back geometrically far for a linear number of columns. The
/// last tier's overflow is simply forgotten.
///
/// Consumers see one flat, time-ordered sequence — [`iter`](Self::iter),
/// [`get`](Self::get), [`partition_point`](Self::partition_point) — and never
/// have to know which tier a column came from.
#[derive(Default)]
pub struct SpectrumHistory {
    /// Finest first: `tiers[0]` holds columns exactly as they were pushed.
    /// Time runs oldest-first WITHIN a tier and oldest-tier-first ACROSS them,
    /// so the flat order is `tiers[TIERS-1] .. tiers[0]`.
    tiers: [VecDeque<SpectrogramColumn>; SpectrumHistory::TIERS],
    len: usize,
}

impl SpectrumHistory {
    /// Columns kept at the analyzer's own rate — the stretch the display can
    /// still ask for at full time resolution (a window this short is cut into
    /// slabs no finer than the analysis interval anyway).
    pub const FINE_COLUMNS: usize = 2048;
    /// Columns per coarser tier. Each tier doubles the reach of the one before
    /// it for this many more columns, which is what makes the reach/memory
    /// trade logarithmic — a longer span costs a tier, not a proportion.
    pub const COARSE_COLUMNS: usize = 512;
    /// Total tiers, the fine one included.
    pub const TIERS: usize = 6;
    /// The most columns ever held. At 20 ms per column this reaches ~11 minutes
    /// (see [`reach`](Self::reach)) — about 18 MB a byte per bucket, double
    /// that while the [`coarsen`] A/B keeps the store sixteen bits wide.
    pub const MAX_COLUMNS: usize = Self::FINE_COLUMNS + (Self::TIERS - 1) * Self::COARSE_COLUMNS;

    /// How many columns tier `k` holds before it merges into the next.
    const fn cap(k: usize) -> usize {
        if k == 0 {
            Self::FINE_COLUMNS
        } else {
            Self::COARSE_COLUMNS
        }
    }

    /// Seconds of history a full structure spans, for a source producing a
    /// column every `interval` seconds.
    pub fn reach(interval: f64) -> f64 {
        let mut span = Self::FINE_COLUMNS as f64 * interval;
        let mut step = interval;
        for _ in 1..Self::TIERS {
            step *= 2.0;
            span += Self::COARSE_COLUMNS as f64 * step;
        }
        span
    }

    /// Bytes the columns themselves occupy when full (the per-column bookkeeping
    /// on top is a timestamp and a pointer).
    pub const fn max_bytes() -> usize {
        Self::MAX_COLUMNS * SPECTRUM_BINS * std::mem::size_of::<BucketDb>()
    }

    /// Append a column (callers push in time order).
    pub fn push(&mut self, column: SpectrogramColumn) {
        self.tiers[0].push_back(column);
        self.len += 1;
        self.cascade();
    }

    /// Push every full tier's overflow down: two columns out, one merged column
    /// into the next tier, and out of the structure entirely at the last one.
    ///
    /// Amortized O(1) per push — a column is merged at most once per tier over
    /// its whole life, and a merge is a byte-wise MAX over one spectrum.
    fn cascade(&mut self) {
        for k in 0..Self::TIERS {
            while self.tiers[k].len() > Self::cap(k) {
                // A cap is never below 2, so passing it leaves at least two to
                // take; the `else` cannot fire, and returning if it somehow did
                // just leaves the tier one column over.
                let (Some(older), Some(mut newer)) =
                    (self.tiers[k].pop_front(), self.tiers[k].pop_front())
                else {
                    return;
                };
                self.len -= 2;
                if k + 1 < Self::TIERS {
                    newer.absorb(&older);
                    self.tiers[k + 1].push_back(newer);
                    self.len += 1;
                }
            }
        }
    }

    /// Forget everything older than `cutoff`, oldest tier first.
    pub fn trim_older_than(&mut self, cutoff: f64) {
        for k in (0..Self::TIERS).rev() {
            while self.tiers[k].front().is_some_and(|c| c.time < cutoff) {
                self.tiers[k].pop_front();
                self.len -= 1;
            }
            // A surviving front means everything newer survives too.
            if !self.tiers[k].is_empty() {
                break;
            }
        }
    }

    pub fn clear(&mut self) {
        for tier in &mut self.tiers {
            tier.clear();
        }
        self.len = 0;
    }

    pub fn len(&self) -> usize {
        self.len
    }

    pub fn is_empty(&self) -> bool {
        self.len == 0
    }

    /// The oldest column held.
    pub fn front(&self) -> Option<&SpectrogramColumn> {
        self.tiers.iter().rev().find_map(|t| t.front())
    }

    /// The newest column held.
    pub fn back(&self) -> Option<&SpectrogramColumn> {
        self.tiers.iter().find_map(|t| t.back())
    }

    /// Which tier flat index `i` falls in, and where inside it.
    fn locate(&self, mut i: usize) -> Option<(usize, usize)> {
        for k in (0..Self::TIERS).rev() {
            let n = self.tiers[k].len();
            if i < n {
                return Some((k, i));
            }
            i -= n;
        }
        None
    }

    /// The `i`th column, oldest first.
    pub fn get(&self, i: usize) -> Option<&SpectrogramColumn> {
        let (k, off) = self.locate(i)?;
        self.tiers[k].get(off)
    }

    /// Every column, oldest first.
    pub fn iter(&self) -> impl Iterator<Item = &SpectrogramColumn> + '_ {
        self.tiers.iter().rev().flat_map(|t| t.iter())
    }

    /// Every column from flat index `i` on — `iter().skip(i)` without walking
    /// the skipped ones, which matters because the callers skip nearly the
    /// whole history every frame.
    pub fn iter_from(&self, i: usize) -> impl Iterator<Item = &SpectrogramColumn> + '_ {
        let (tier, off) = self.locate(i).unwrap_or((0, self.tiers[0].len()));
        self.tiers[..=tier]
            .iter()
            .rev()
            .enumerate()
            .flat_map(move |(n, t)| t.iter().skip(if n == 0 { off } else { 0 }))
    }

    /// The number of leading columns satisfying `pred`, which must be monotone
    /// over the (time-ordered) sequence — the flat equivalent of
    /// [`VecDeque::partition_point`].
    pub fn partition_point(&self, pred: impl Fn(&SpectrogramColumn) -> bool) -> usize {
        let mut n = 0;
        for k in (0..Self::TIERS).rev() {
            let tier = &self.tiers[k];
            let p = tier.partition_point(|c| pred(c));
            n += p;
            if p < tier.len() {
                break;
            }
        }
        n
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn col(time: f64, level: f32) -> SpectrogramColumn {
        let mut power = [0.0f32; SPECTRUM_BINS];
        power[0] = level;
        SpectrogramColumn::from_power(time, &power)
    }

    /// The whole point of the byte encoding: it must be monotone in power, so a
    /// MAX over stored bytes is a MAX over the powers they stand for. Anything
    /// else would let aggregation pick the quieter of two buckets.
    #[test]
    fn quantizing_preserves_order() {
        let powers = [0.0, 1e-11, 1e-9, 1e-6, 1e-3, 0.01, 0.1, 0.5, 1.0, 2.0, 10.0];
        for pair in powers.windows(2) {
            assert!(
                quantize(pair[0]) <= quantize(pair[1]),
                "{} -> {} but {} -> {}",
                pair[0],
                quantize(pair[0]),
                pair[1],
                quantize(pair[1]),
            );
        }
        // And it separates levels that matter: a dB apart is at least a step.
        assert!(quantize(0.1) < quantize(0.126), "1 dB apart must not collide");
    }

    /// Round-tripping a bucket must land within half a step of where it
    /// started, across the whole range the display can read.
    #[test]
    fn quantizing_round_trips_to_half_a_step() {
        // The whole span the display can be asked to show: its dB window stops
        // at -120 dB, and 0 dB (a full-scale sine) is the top of every range bar.
        for db in [-120.0f32, -110.0, -100.0, -90.0, -60.0, -30.0, -12.0, -0.5, 0.0, 6.0] {
            let power = 10.0f32.powf(db / 10.0);
            let back = db_of(quantize(power));
            assert!((back - db).abs() <= DB_STEP * 0.5 + 1e-3, "{db} dB came back as {back} dB");
        }
        // Out of range in both directions saturates rather than wrapping. The
        // top is well above a full-scale sine, and far above any ceiling the
        // bars offer, so what saturates there was already white.
        assert_eq!(quantize(0.0), 0);
        assert_eq!(quantize(1e-30), 0);
        assert_eq!(quantize(1e9), BucketDb::MAX);
        assert!(db_of(BucketDb::MAX) >= 6.0, "no headroom left above a full-scale sine");
    }

    /// The A/B's coarse side has to be exactly the picture a byte-per-bucket
    /// store would have given — the same set of representable dB values, hit
    /// by rounding, not by drifting near them. Otherwise the comparison it
    /// exists for is between the fine picture and something that is neither.
    #[test]
    fn coarsening_lands_on_the_eight_bit_grid() {
        let per_step = (COARSE_DB_STEP / DB_STEP).round() as u32;
        for raw in (0..=u32::from(BucketDb::MAX)).step_by(37) {
            let raw = raw as BucketDb;
            let coarse = coarsen(raw);
            assert_eq!(u32::from(coarse) % per_step, 0, "{raw} coarsened to an off-grid {coarse}");
            let moved = (db_of(coarse) - db_of(raw)).abs();
            assert!(
                moved <= COARSE_DB_STEP * 0.5 + 1e-3,
                "{raw} moved {moved} dB, over half a coarse step",
            );
        }
        // A value already on the grid must not move at all.
        for k in [0u32, 1, 100, 255] {
            let on_grid = (k * per_step) as BucketDb;
            assert_eq!(coarsen(on_grid), on_grid, "a grid point moved");
        }
    }

    /// The flat view has to behave exactly like the single queue it replaced:
    /// in time order, indexable, and searchable — whatever tier a column has
    /// aged into.
    #[test]
    fn the_flat_view_stays_in_time_order_across_tiers() {
        let mut history = SpectrumHistory::default();
        // Enough to push columns down several tiers.
        let pushed = SpectrumHistory::FINE_COLUMNS + 4 * SpectrumHistory::COARSE_COLUMNS;
        for i in 0..pushed {
            history.push(col(i as f64 * 0.02, 0.5));
        }
        assert!(history.len() > 1);
        let times: Vec<f64> = history.iter().map(|c| c.time).collect();
        assert_eq!(times.len(), history.len(), "iter must yield exactly len columns");
        for pair in times.windows(2) {
            assert!(pair[0] < pair[1], "history ran backwards: {pair:?}");
        }
        // get / iter_from / partition_point agree with that order.
        for i in [0, 1, times.len() / 3, times.len() - 1] {
            assert_eq!(history.get(i).unwrap().time, times[i], "get({i})");
            assert_eq!(history.iter_from(i).next().unwrap().time, times[i], "iter_from({i})");
            assert_eq!(history.iter_from(i).count(), times.len() - i);
        }
        let mid = times[times.len() / 2];
        assert_eq!(
            history.partition_point(|c| c.time < mid),
            times.iter().filter(|&&t| t < mid).count(),
        );
        assert_eq!(history.front().unwrap().time, times[0]);
        assert_eq!(history.back().unwrap().time, *times.last().unwrap());
    }

    /// Memory is bounded by the tier caps, not by how long the plugin has been
    /// open — that is the whole reason for the structure.
    #[test]
    fn the_column_count_is_bounded_however_long_it_runs() {
        const INTERVAL: f64 = 0.02;
        let mut history = SpectrumHistory::default();
        // Long enough to fill every tier (the deepest is only reached after a
        // whole `reach` of columns) and then run well past it.
        let pushes = (SpectrumHistory::reach(INTERVAL) / INTERVAL * 1.5) as usize;
        for i in 0..pushes {
            history.push(col(i as f64 * INTERVAL, 0.5));
            assert!(
                history.len() <= SpectrumHistory::MAX_COLUMNS,
                "held {} columns, cap is {}",
                history.len(),
                SpectrumHistory::MAX_COLUMNS,
            );
        }
        // And it really is full, rather than quietly dropping everything. Each
        // tier sits a column or two under its cap between merges, so the total
        // rides just below the bound instead of exactly on it.
        assert!(
            history.len() > SpectrumHistory::MAX_COLUMNS * 9 / 10,
            "only {} of {} columns held",
            history.len(),
            SpectrumHistory::MAX_COLUMNS,
        );
        // A full structure reaches back what `reach` advertises, which is the
        // number every retention decision above it is made against.
        let held = history.back().unwrap().time - history.front().unwrap().time;
        let want = SpectrumHistory::reach(INTERVAL);
        assert!(
            (held - want).abs() < want * 0.05,
            "reaches back {held} s, advertised {want} s",
        );
    }

    /// Merging is MAX, so a brief loud moment survives into the coarse tiers
    /// instead of being averaged away by the silence around it — the same
    /// promise the display's own slab aggregation makes.
    #[test]
    fn a_peak_survives_being_merged_into_the_coarse_tiers() {
        let mut history = SpectrumHistory::default();
        let loud_at = 7.0;
        for i in 0..(SpectrumHistory::FINE_COLUMNS * 4) {
            let t = i as f64 * 0.02;
            let level = if (t - loud_at).abs() < 1e-9 { 1.0 } else { 1e-6 };
            history.push(col(t, level));
        }
        // The loud column is long past the fine tier by now.
        assert!(history.front().unwrap().time < loud_at, "the peak should still be held");
        let peak = history
            .iter()
            .filter(|c| (c.time - loud_at).abs() < 1.0)
            .map(|c| c.db[0])
            .max()
            .expect("columns around the peak");
        assert_eq!(peak, quantize(1.0), "the peak was lost to merging");
    }

    /// A merged column stands for the interval it covers, so its timestamp must
    /// stay inside that interval — it is what places the energy on the time
    /// axis.
    #[test]
    fn a_merged_column_is_timestamped_inside_the_pair_it_replaced() {
        let mut history = SpectrumHistory::default();
        let n = SpectrumHistory::FINE_COLUMNS + 2 * SpectrumHistory::COARSE_COLUMNS;
        for i in 0..n {
            history.push(col(i as f64 * 0.02, 0.5));
        }
        let first = history.front().unwrap().time;
        let last = history.back().unwrap().time;
        assert!(first >= 0.0, "a merged timestamp fell before the first column");
        assert!(last <= (n - 1) as f64 * 0.02 + 1e-9, "and none after the last");
        // Spacing grows with age: the oldest columns are further apart than the
        // newest, which is the resolution decay the structure exists for.
        let times: Vec<f64> = history.iter().map(|c| c.time).collect();
        let oldest_gap = times[1] - times[0];
        let newest_gap = times[times.len() - 1] - times[times.len() - 2];
        assert!(
            oldest_gap > newest_gap,
            "old columns should be coarser: {oldest_gap} vs {newest_gap}",
        );
    }

    #[test]
    fn trimming_drops_the_oldest_first_and_clear_empties_it() {
        let mut history = SpectrumHistory::default();
        for i in 0..(SpectrumHistory::FINE_COLUMNS * 3) {
            history.push(col(i as f64 * 0.02, 0.5));
        }
        let newest = history.back().unwrap().time;
        history.trim_older_than(newest - 5.0);
        assert!(history.front().unwrap().time >= newest - 5.0, "old columns survived the trim");
        assert!(!history.is_empty(), "recent columns must not be trimmed");
        assert_eq!(history.iter().count(), history.len(), "len tracked the trim");
        history.clear();
        assert!(history.is_empty());
        assert_eq!(history.iter().count(), 0);
        assert!(history.front().is_none() && history.back().is_none());
    }
}
