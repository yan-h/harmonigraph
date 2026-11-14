//! The live heatmap's history: one shared slab pyramid, a level per rung of
//! [`live_slab`]'s ladder, that folds every column into every level as it
//! arrives and never re-reads one (#1758).
//!
//! A surface draws a slice of one level's ring, so every surface at a rung
//! draws the same slabs, and nothing is ever refolded: a rung change, a late
//! open or the Video preview reads what was folded on arrival. What is given
//! up for that is a redraw of old history under a new encoding, which nothing
//! changes today.

use std::collections::VecDeque;
use std::sync::Arc;

use harmonigraph_core::spectrum::SPECTRUM_BINS;
use harmonigraph_render::SpectrogramSlab;

use super::{live_slab, ring_slots, PowerMean, JITTER_SLABS, LIVE_SLAB_CAP};
use crate::spectrum::SpectrumBuckets;

/// Seconds one slab of `level` spans: the ladder's floor rung, doubled
/// `level` times.
pub(crate) fn width(level: usize) -> f64 {
    live_slab(0.0, 1) * f64::from(1u32 << level)
}

/// Levels the pyramid keeps: down to the rung a Span as long as the roll
/// offers ([`ROLL_SECONDS_MAX`](crate::ROLL_SECONDS_MAX)) takes on a pane
/// two slabs deep, the coarsest any surface asks for. Sixteen today, the
/// coarsest 524 s.
pub(crate) const LEVELS: usize = {
    let floor = crate::AudioSpectrum::FFT_INTERVAL * super::LADDER_FLOOR_COLUMNS;
    let mut level = 0;
    while floor * ((1u64 << level) as f64) * 2.0 < crate::ROLL_SECONDS_MAX as f64 {
        level += 1;
    }
    level + 1
};

/// The level a slab width names, saturating at the coarsest: [`live_slab`]
/// only ever answers a rung, and a window past the roll's own bound reads
/// the coarsest level the pyramid has.
pub(crate) fn level_of(bucket: f64) -> usize {
    (0..LEVELS).find(|&level| width(level) >= bucket).unwrap_or(LEVELS - 1)
}

/// Slabs `level` keeps: the widest ring a surface reads at it, or the
/// history's reach and three more, whichever is fewer. Three, so the far
/// edge one slab past a full window still lands on a slab the ring holds.
fn retention(level: usize) -> usize {
    let reach = (crate::AudioSpectrum::HISTORY_MAX_SECONDS / width(level)).ceil() as usize + 3;
    ring_slots(LIVE_SLAB_CAP as usize).min(reach)
}

/// What the guard at the pyramid's one entry makes of a column dated `time`
/// against the newest it has folded. See [`admit`].
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Admit {
    /// Newer than the newest: fold it.
    Fold,
    /// At or up to [`RESET_BEHIND`] before the newest, or undated: drop it,
    /// counted. No shipped feeder does this (see [`admit`]).
    Drop,
    /// Further back than that: the fold cannot trust its own state, so the
    /// history resets and the column starts it again, counted.
    Reset,
}

/// How far behind the newest folded column a column may be dated before the
/// history resets rather than dropping it.
///
/// Above the largest step back a correct analyzer change could make —
/// Precise's whole lag, about 188 ms — and below what reads as a hang: the
/// decisive case is a column dated an hour ahead, after which every correct
/// column is behind it, and dropping them would draw no new sound for an hour.
pub(crate) const RESET_BEHIND: f64 = 1.0;

/// The guard's verdict on a column dated `time` against `newest`, the newest
/// folded (negative infinity while the pyramid is empty). Only a strictly
/// newer column folds, so a NaN or infinite date is dropped.
///
/// No correct feeder dates backward: a resolution switch waits at least three
/// times its lag growth, the plugin's clock offset and presentation seconds
/// only grow, an input switch starts a later epoch, a Tape audition is dated
/// on the presentation clock, and offline builds a fresh state per render
/// with its frames ascending. So a drop or a reset is a bug elsewhere, which
/// the overlay counts and `SpectrumStore::take` asserts against.
pub(crate) fn admit(newest: f64, time: f64) -> Admit {
    if !time.is_finite() {
        Admit::Drop
    } else if time > newest {
        Admit::Fold
    } else if time >= newest - RESET_BEHIND {
        Admit::Drop
    } else {
        Admit::Reset
    }
}

/// One rung's store: a ring of slabs keyed `floor(t / width)`, the newest
/// one open and still averaging its columns.
struct Level {
    width: f64,
    keep: usize,
    /// The key of `slabs[0]`.
    first_key: i64,
    slabs: VecDeque<SpectrogramSlab>,
    /// The open slab's sums. All +0 while `silent`.
    mean: PowerMean,
    /// The open slab's bytes are behind its sums.
    dirty: bool,
    /// The open slab has seen only all-zero columns, so it is still the
    /// shared silent slab and its sums are still all +0.
    silent: bool,
}

impl Level {
    fn new(level: usize) -> Self {
        Level {
            width: width(level),
            keep: retention(level),
            first_key: 0,
            slabs: VecDeque::new(),
            mean: PowerMean::default(),
            dirty: false,
            silent: true,
        }
    }

    fn newest_key(&self) -> Option<i64> {
        (!self.slabs.is_empty()).then(|| self.first_key + self.slabs.len() as i64 - 1)
    }

    /// Fold one sanitized column, `zero` if every bucket of it is +0 or -0.
    fn fold(&mut self, time: f64, power: &SpectrumBuckets, zero: bool, silence: &SpectrogramSlab) {
        let key = (time / self.width).floor() as i64;
        match self.newest_key() {
            // The guard admits only newer columns, so a key is never behind
            // the open slab's.
            Some(open) if key <= open => {}
            Some(open) => {
                self.finish();
                let empty = key - open - 1;
                let mut start = (open + 1).max(key - self.keep as i64 + 1);
                if (key - open) as f64 * self.width > crate::AudioSpectrum::HISTORY_MAX_SECONDS {
                    // A gap longer than the history reaches starts it again,
                    // as a fresh history: nothing before it is drawn, and
                    // the strip waits for a second slab as it does at
                    // startup, rather than drawing ten minutes of silence.
                    start = key;
                }
                if start > open + 1 {
                    // Everything held is older than the ring keeps.
                    self.slabs.clear();
                    self.first_key = start;
                }
                for _ in start..key {
                    // A gap this short is a seam in the sample stream rather
                    // than silence (see `JITTER_SLABS`): hold the previous
                    // slab, which is final, by sharing it.
                    let held = match self.slabs.back() {
                        Some(previous) if empty <= JITTER_SLABS => previous.clone(),
                        _ => silence.clone(),
                    };
                    self.slabs.push_back(held);
                }
                self.open(silence);
            }
            None => {
                self.first_key = key;
                self.open(silence);
            }
        }
        if zero {
            // Adding +0 to sums that start at +0 and only take non-negative
            // powers changes no bit, so only the count moves; and the bytes
            // of a slab that has seen nothing but zeros cannot move at all.
            self.mean.count += 1;
            self.dirty |= !self.silent;
        } else {
            self.mean.add(power, 1);
            self.silent = false;
            self.dirty = true;
        }
        while self.slabs.len() > self.keep {
            self.slabs.pop_front();
            self.first_key += 1;
        }
    }

    /// Open a new slab as the shared silent one, with sums at zero: a slab
    /// that closed silent left them there already.
    fn open(&mut self, silence: &SpectrogramSlab) {
        if !self.silent {
            self.mean.clear();
        }
        self.mean.count = 0;
        self.silent = true;
        self.dirty = false;
        self.slabs.push_back(silence.clone());
    }

    /// Bring the open slab's bytes up to its sums. Through `make_mut`: a
    /// snapshot may hold the slab, and the shared silent slab is always held
    /// by the pyramid too, so either is copied rather than written under its
    /// other owners.
    fn finish(&mut self) {
        if self.dirty {
            let open = self.slabs.back_mut().expect("a dirty level has an open slab");
            self.mean.write(Arc::make_mut(open));
            self.dirty = false;
        }
    }

    fn clear(&mut self) {
        self.slabs.clear();
        self.mean.clear();
        self.dirty = false;
        self.silent = true;
    }
}

/// The run a surface draws: `len` slabs from `first_key` to the newest.
pub(crate) struct Run<'a> {
    pub(crate) first_key: i64,
    pub(crate) slabs: std::collections::vec_deque::Iter<'a, SpectrogramSlab>,
}

impl Run<'_> {
    pub(crate) fn len(&self) -> usize {
        self.slabs.len()
    }
}

/// Every level, and the guard's newest date.
pub(crate) struct SlabPyramid {
    levels: Vec<Level>,
    /// The newest column folded, negative infinity while empty.
    newest: f64,
    /// The one silent slab every gap and every silent open slab shares. The
    /// pyramid's own handle keeps its count of owners at two or more, so a
    /// slab that has to change is always copied off it.
    silence: SpectrogramSlab,
    /// The sanitized column being folded, kept to save an allocation a
    /// column.
    scratch: Box<SpectrumBuckets>,
}

impl Default for SlabPyramid {
    fn default() -> Self {
        SlabPyramid {
            levels: (0..LEVELS).map(Level::new).collect(),
            newest: f64::NEG_INFINITY,
            silence: SpectrogramSlab::from([0; SPECTRUM_BINS]),
            scratch: Box::new([0.0; SPECTRUM_BINS]),
        }
    }
}

#[cfg(test)]
thread_local! {
    /// The level whose fold panics, for the test of what a crash in the fold
    /// does to the history.
    pub(crate) static FAIL_AT_LEVEL: std::cell::Cell<Option<usize>> =
        const { std::cell::Cell::new(None) };
}

impl SlabPyramid {
    /// The newest column folded, `None` while empty.
    pub(crate) fn newest(&self) -> Option<f64> {
        self.newest.is_finite().then_some(self.newest)
    }

    /// The newest column folded, negative infinity while empty: what
    /// [`admit`] weighs a column against.
    pub(crate) fn newest_or_never(&self) -> f64 {
        self.newest
    }

    /// Fold a column the guard admitted into every level. Its power is
    /// sanitized first: a non-finite bucket folds as 0, and a negative one as
    /// 0 too, as the curve takes them.
    pub(crate) fn fold(&mut self, time: f64, power: &SpectrumBuckets) {
        let mut zero = true;
        for (out, &p) in self.scratch.iter_mut().zip(power) {
            *out = if p.is_finite() { p.max(0.0) } else { 0.0 };
            zero &= *out == 0.0;
        }
        self.newest = time;
        for (n, level) in self.levels.iter_mut().enumerate() {
            #[cfg(test)]
            if FAIL_AT_LEVEL.get() == Some(n) {
                panic!("a fault injected into level {n}'s fold");
            }
            #[cfg(not(test))]
            let _ = n;
            level.fold(time, &self.scratch, zero, &self.silence);
        }
    }

    /// Empty every level.
    pub(crate) fn clear(&mut self) {
        for level in &mut self.levels {
            level.clear();
        }
        self.newest = f64::NEG_INFINITY;
    }

    /// The run a surface drawing `level` reads for a window whose far edge is
    /// at `from`: from one slab past the slab the edge is in, or the oldest
    /// held, to the newest, with the open slab's bytes brought up to date.
    /// `None` while the level holds under two slabs of it.
    ///
    /// One slab past the edge rather than the last column before it, so the
    /// strip reaches the edge however the slabs before it were filled: the
    /// extra slab is off the pane, and the outermost half slab of pixels
    /// reads its true neighbour.
    pub(crate) fn run(&mut self, level: usize, from: f64) -> Option<Run<'_>> {
        let level = &mut self.levels[level.min(LEVELS - 1)];
        let newest = level.newest_key()?;
        level.finish();
        let edge = (from / level.width).floor();
        let first = (edge as i64).saturating_sub(1).max(level.first_key).min(newest);
        if newest - first < 1 {
            return None;
        }
        let at = (first - level.first_key) as usize;
        Some(Run { first_key: first, slabs: level.slabs.range(at..) })
    }

    /// Bytes the pyramid holds in its slabs and sums, counting each slab once
    /// however many levels and snapshots share it.
    #[cfg(test)]
    pub(crate) fn resident_bytes(&self) -> usize {
        let mut seen = std::collections::HashSet::new();
        let mut bytes = 0;
        for level in &self.levels {
            bytes += SPECTRUM_BINS * size_of::<f64>();
            for slab in &level.slabs {
                // Its bytes and the `Arc`'s two counts, to the 512-byte
                // quantum macOS allocates blocks of this size in.
                if seen.insert(Arc::as_ptr(slab) as *const u8) {
                    bytes += (slab.len() + 2 * size_of::<usize>()).next_multiple_of(512);
                }
            }
        }
        bytes
    }

    /// A level's slabs, oldest first, with the key of the first.
    #[cfg(test)]
    pub(crate) fn level(&mut self, level: usize) -> (i64, Vec<SpectrogramSlab>) {
        let level = &mut self.levels[level];
        level.finish();
        (level.first_key, level.slabs.iter().cloned().collect())
    }

    /// Whether `level`'s open slab is marked behind its sums.
    #[cfg(test)]
    pub(crate) fn open_is_dirty(&self, level: usize) -> bool {
        self.levels[level].dirty
    }

    /// Columns `level`'s open slab has averaged.
    #[cfg(test)]
    pub(crate) fn open_count(&self, level: usize) -> u64 {
        self.levels[level].mean.count
    }

    /// The shared silent slab.
    #[cfg(test)]
    pub(crate) fn silence(&self) -> &SpectrogramSlab {
        &self.silence
    }
}

/// One level of `width` seconds keeping `keep` slabs, fed `columns` in order:
/// its first key and its slabs, for the tests that hold a level to the
/// oracle at widths off the ladder.
#[cfg(test)]
pub(crate) fn fold_one_level(
    width: f64,
    keep: usize,
    columns: &[(f64, Box<SpectrumBuckets>)],
) -> (i64, Vec<SpectrogramSlab>) {
    let mut level = Level { width, keep, ..Level::new(0) };
    let silence = SpectrogramSlab::from([0; SPECTRUM_BINS]);
    for (time, power) in columns {
        let zero = power.iter().all(|&p| p == 0.0);
        level.fold(*time, power, zero, &silence);
    }
    level.finish();
    (level.first_key, level.slabs.into_iter().collect())
}

#[cfg(test)]
mod tests;
