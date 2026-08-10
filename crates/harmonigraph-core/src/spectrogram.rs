//! The spectrogram's stored history: what the analyzer measured, kept over
//! time in the form the heatmap reads it back.
//!
//! [`spectrum`](crate::spectrum) answers "what is sounding now"; this answers
//! "what has been sounding", which is a memory question — one column is a whole
//! pitch spectrum, and they arrive at the analyzer's rate for as long as the
//! plugin is open. Two decisions do the work, and both come from what the
//! display can actually show:
//!
//! - **A bucket is stored as a byte of dB, not a float of power.** The heatmap
//!   maps a bucket through `10*log10` into a colour ramp, so dB is the domain it
//!   is read in, and a byte resolves it to half a dB.
//!
//!   The encoding therefore owes the readers two things rather than one. It is
//!   MONOTONE, so the aggregation along TIME — a MAX, here and in the display's
//!   slabs alike — is order-preserving and needs no decode. And it is AFFINE in
//!   dB, so the aggregation across PITCH, which is a power mean and does have
//!   to decode, can do it from a 256-entry table indexed by a difference of
//!   bytes rather than by reaching for a logarithm per bucket. Neither reader
//!   ever converts a byte back to a float of power.
//!
//!   Half a dB was checked by eye before it was settled on: the store was
//!   briefly sixteen bits with a toggle that drew either grid, and the two
//!   pictures were indistinguishable, so the byte stays and the toggle went.
//! - **Old columns are merged as they age.** The heatmap draws a window ending
//!   at `now`, so a column of age `a` is only ever on screen when the window is
//!   at least `a` long — and the window is cut into at most a thousand-odd time
//!   slabs, which puts a floor of roughly `a / 1024` on the slab a column of
//!   that age can land in. Keeping 8 ms columns from ten minutes ago is therefore
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

/// One stored bucket: power in dB, quantized to a byte. See [`quantize`].
pub type BucketDb = u8;

/// One stored column's worth of buckets.
pub type ColumnDb = [BucketDb; SPECTRUM_BINS];

/// The dB a stored `0` stands for, and the step between stored values.
///
/// The pair covers -120 dB to +7.5 dB. The floor is exactly where the display's
/// own mapping bottoms out (`loudness` clamps power at 1e-12) and exactly where
/// the heatmap's range bar stops, so the quietest cell the UI can ask to see is
/// the quietest value there is — the encoding adds no floor of its own, and
/// nothing fades out early against one. The ceiling sits above a full-scale
/// sine's 0 dB, which is already saturated white at any range the bars allow,
/// so nothing visible clips against it either.
pub const DB_FLOOR: f32 = -120.0;
pub const DB_STEP: f32 = 0.5;

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
    /// MAX, not a mean, for the same reason the display aggregates ALONG TIME by
    /// MAX — a spectrogram cell answers "was there anything here", and averaging
    /// a bright thin partial with its quiet neighbour answers "not much". This
    /// merges two columns, so it is that axis and not the pitch one, where the
    /// display takes a power mean instead over a run of INDEPENDENT buckets. The
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
///
/// What they do have to know is that a column older than the finest tier is not
/// the one they read last time: a merge REWRITES the pair it replaces, at their
/// midpoint time and carrying both their energies
/// ([`absorb`](SpectrogramColumn::absorb)). Re-reading the old stretch therefore
/// answers a slightly different question each time it ages a tier, so work
/// derived from it is worth keeping rather than recomputing — see
/// `SpectrogramAgg` in harmonigraph-ui, which folds each column once as it arrives
/// and never re-reads it.
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
    ///
    /// Must be at least TWICE [`COARSE_COLUMNS`](Self::COARSE_COLUMNS): tier 1
    /// stores two analysis intervals apart, so the fine tier has to reach an age
    /// where a slab is that wide before the merging starts.
    pub const FINE_COLUMNS: usize = 2048;
    /// Columns per coarser tier. Each tier doubles the reach of the one before
    /// it for this many more columns, which is what makes the reach/memory
    /// trade logarithmic — a longer span costs a tier, not a proportion.
    ///
    /// Must be at least the display's slab cap
    /// (`harmonigraph_ui::spectrogram::LIVE_SLAB_CAP`). Every tier doubles its
    /// spacing but only ages the store forward by `COARSE_COLUMNS` of that
    /// spacing, so a tier narrower than the cap falls behind what the finest
    /// window reaching it can resolve — one tier per doubling only keeps up
    /// while these two match. The UI test
    /// `stored_columns_stay_finer_than_the_slabs_they_are_drawn_into` is what
    /// says so; the two constants move together or not at all.
    pub const COARSE_COLUMNS: usize = 1024;
    /// Total tiers, the fine one included.
    pub const TIERS: usize = 7;
    /// The most columns ever held. At 8 ms per column this reaches ~17 minutes
    /// (see [`reach`](Self::reach)) for about 30 MB.
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

    /// Half of what the byte encoding owes its readers: it must be monotone in
    /// power, so a MAX over stored bytes is a MAX over the powers they stand
    /// for. Anything else would let the time-axis aggregation pick the quieter
    /// of two buckets. (The other half is that it is affine in dB, which is what
    /// lets the pitch axis take a power mean off a table — the display's own
    /// `the_curve_and_the_heatmap_read_a_run_of_buckets_alike` holds that end.)
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
