//! The pyramid held to the oracle fold it replaced (`spectrogram::oracle`),
//! level by level.

use super::*;
use crate::spectrogram::oracle::SlabGrid;

type Column = (f64, Box<SpectrumBuckets>);

/// A deterministic stream: xorshift powers over every bucket, from -110 to
/// -10 dB so a slab's mean lands anywhere on the byte grid, and a few loud
/// lines.
struct Stream(u64);

impl Stream {
    fn next(&mut self) -> u64 {
        self.0 ^= self.0 << 13;
        self.0 ^= self.0 >> 7;
        self.0 ^= self.0 << 17;
        self.0
    }
    fn unit(&mut self) -> f32 {
        (self.next() >> 40) as f32 / (1u64 << 24) as f32
    }
    fn power(&mut self) -> Box<SpectrumBuckets> {
        let mut power = Box::new([0.0f32; SPECTRUM_BINS]);
        for p in power.iter_mut() {
            *p = 10f32.powf(-11.0 + 10.0 * self.unit());
        }
        for _ in 0..8 {
            let bin = (self.next() % SPECTRUM_BINS as u64) as usize;
            power[bin] = 0.1 + self.unit();
        }
        power
    }
}

/// What the pyramid folds a column as: non-finite and negative powers as 0.
fn sanitized(power: &SpectrumBuckets) -> Box<SpectrumBuckets> {
    Box::new(power.map(|p| if p.is_finite() { p.max(0.0) } else { 0.0 }))
}

/// Hold every level of `pyramid` to the oracle folded over `columns` (all
/// fed to the pyramid, in order): over the slabs both hold, byte for byte.
/// The oracle folds from one slab before the level's first, so it allocates
/// no long run of silence. Once a level has evicted, it must hold exactly its retention.
fn matches_the_oracle(pyramid: &mut SlabPyramid, columns: &[Column], at: &str) {
    for n in 0..LEVELS {
        let (first_key, slabs) = pyramid.level(n);
        let w = width(n);
        let from = columns.partition_point(|(t, _)| ((t / w).floor() as i64) < first_key - 1);
        let mut grid = SlabGrid::default();
        for (time, power) in &columns[from..] {
            grid.fold(*time, &sanitized(power)[..], w);
        }
        grid.finish();
        let oracle_first = (grid.centers[0] / w).floor() as i64;
        let newest = first_key + slabs.len() as i64 - 1;
        assert_eq!(
            oracle_first + grid.power.len() as i64 - 1,
            newest,
            "level {n} ends at another slab {at}"
        );
        for key in first_key.max(oracle_first)..=newest {
            let (ours, theirs) =
                (&slabs[(key - first_key) as usize], &grid.power[(key - oracle_first) as usize]);
            if ours[..] != theirs[..] {
                let differ = ours.iter().zip(theirs.iter()).filter(|(a, b)| a != b).count();
                panic!("level {n} slab {key} differs in {differ} buckets {at}");
            }
        }
        let first_held = (columns[0].0 / w).floor() as i64;
        if first_key > first_held {
            assert_eq!(slabs.len(), retention(n), "level {n} after eviction {at}");
        }
    }
}

/// Every level folds what the oracle folds, as far back as it keeps, through
/// eviction at all sixteen: a stream long enough for the coarsest level's
/// five slabs to turn over, at the column rate for the finest levels' whole
/// rings and at eight times that after, with gaps of a slab, of several, and
/// longer than the finest level keeps.
#[test]
fn every_level_folds_what_the_oracle_folds_over_what_it_keeps() {
    let coarsest = width(LEVELS - 1) * retention(LEVELS - 1) as f64;
    let fine = width(0) * retention(0) as f64;
    let mut stream = Stream(0x9E37_79B9_7F4A_7C15);
    let origin = 90.012_345_678;
    let mut columns: Vec<Column> = Vec::new();
    let mut t = origin;
    while t < origin + coarsest + 2.0 * width(LEVELS - 1) {
        let rel = t - origin;
        let gap = [(3.0, 0.017), (7.31, 0.03), (21.0, 1.3), (40.0, 20.0)]
            .iter()
            .any(|&(at, gap)| rel >= at && rel < at + gap);
        if !gap {
            columns.push((t, stream.power()));
        }
        t += if rel < fine + 1.0 { 0.008 } else { 0.064 };
    }
    let mut pyramid = SlabPyramid::default();
    let checkpoints = [columns.len() / 7, columns.len() / 2, columns.len() - 1];
    for (i, (time, power)) in columns.iter().enumerate() {
        pyramid.push(*time, power);
        if checkpoints.contains(&i) {
            matches_the_oracle(&mut pyramid, &columns[..=i], &format!("at column {i}"));
        }
    }
    let (first_key, slabs) = pyramid.level(LEVELS - 1);
    assert!(
        first_key > (origin / width(LEVELS - 1)).floor() as i64 && slabs.len() == 5,
        "the fixture never turned the coarsest level over",
    );
}

/// The zero-column fast path changes no byte the plain fold would write, at
/// any level, across every state it has: a slab opened by a zero column and
/// then given data, data and then zeros, zero runs across slab boundaries at
/// levels 0, 1 and 6, a one-slab gap after a slab that saw only zeros, and a NaN column and a -0.0 bucket part-way through a slab.
///
/// A slab that saw only zeros is the shared silent slab, and its open
/// predecessor is never marked behind its sums: its bytes cannot change.
#[test]
fn the_zero_column_fast_path_folds_what_the_oracle_folds() {
    let mut stream = Stream(11);
    let zero = || Box::new([0.0f32; SPECTRUM_BINS]);
    let (mut columns, mut t): (Vec<Column>, f64) = (Vec::new(), 1000.0 + 0.003);
    // Data or zeros, eight milliseconds apart; `None` skips a column.
    let mut plan: Vec<Option<bool>> = Vec::new();
    // Zeros straddling several level-0 and level-1 slabs, then data opening
    // into the slab the zeros left open, then zeros again.
    plan.extend([[Some(false); 9].as_slice(), &[Some(true); 5], &[Some(false); 9]].concat());
    // A one-slab gap at level 0 after a slab of zeros.
    plan.extend([[None; 2].as_slice(), &[Some(false); 4], &[Some(true); 2]].concat());
    for wanted in plan {
        if let Some(data) = wanted {
            columns.push((t, if data { stream.power() } else { zero() }));
        }
        t += 0.008;
    }
    // A NaN column and a -0.0 bucket inside a loud slab.
    let mut nan = stream.power();
    nan[17] = f32::NAN;
    nan[18] = -0.0;
    let mut negative = zero();
    negative[5] = -0.0;
    for power in [nan, negative, stream.power(), stream.power()] {
        columns.push((t, power));
        t += 0.008;
    }
    // A long zero run across level-6 boundaries (1.024 s), then data.
    for k in 0..303 {
        columns.push((t, if k < 300 { zero() } else { stream.power() }));
        t += 0.008;
    }

    let mut pyramid = SlabPyramid::default();
    // Per level: the open slab's key, and whether it has seen only zeros.
    let mut open: Vec<Option<(i64, bool)>> = vec![None; LEVELS];
    let mut silent_seen = [false; LEVELS];
    for (time, power) in &columns {
        pyramid.push(*time, power);
        let all_zero = sanitized(power).iter().all(|&p| p == 0.0);
        for (n, open) in open.iter_mut().enumerate() {
            let key = (time / width(n)).floor() as i64;
            let silent = match *open {
                Some((k, silent)) if k == key => silent && all_zero,
                _ => all_zero,
            };
            *open = Some((key, silent));
            if silent {
                silent_seen[n] = true;
                assert!(!pyramid.open_is_dirty(n), "a silent open slab at level {n} is dirty");
            }
        }
    }
    assert!(silent_seen.iter().all(|&s| s), "some level never opened a silent slab");
    matches_the_oracle(&mut pyramid, &columns, "on the mixed fixture");
    for n in [0, 1, 6] {
        let (_, slabs) = pyramid.level(n);
        assert!(
            slabs.iter().filter(|slab| slab.iter().all(|&b| b == 0)).count() > 1,
            "level {n} saw no zero run across a boundary",
        );
        for slab in &slabs {
            if slab.iter().all(|&b| b == 0) {
                assert!(Arc::ptr_eq(slab, pyramid.silence()), "a silent slab at level {n} copied");
            }
        }
    }
}

/// The whole pyramid, every level holding its full retention of distinct
/// slabs, stays under 31 MB: about 7,400 slabs of 4 KB, as the allocator
/// rounds them, and a level's sums each.
#[test]
fn the_full_retention_stays_under_31_mb() {
    let mut pyramid = SlabPyramid::default();
    let mut stream = Stream(3);
    // A column in every slab of every level's retention: going forward in
    // time, the spacing of the finest level that still reaches back here.
    let end = 5000.0;
    let mut t = end - width(LEVELS - 1) * (retention(LEVELS - 1) + 1) as f64;
    while t < end {
        pyramid.push(t, &stream.power());
        let behind = end - t;
        let level =
            (0..LEVELS).find(|&n| width(n) * retention(n) as f64 >= behind).unwrap_or(LEVELS - 1);
        t += width(level);
    }
    let mut slabs = 0;
    for n in 0..LEVELS {
        let (_, held) = pyramid.level(n);
        assert_eq!(held.len(), retention(n), "level {n} is not full");
        slabs += held.len();
    }
    let bytes = pyramid.resident_bytes();
    assert!(slabs > 7_000, "{slabs} slabs");
    assert!(bytes < 31_000_000, "the full pyramid holds {bytes} bytes");
}
