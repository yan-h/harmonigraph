//! The tape's spectrogram (#1719): columns of what the tape holds, on a grid
//! of song seconds, and the pyramid of merged slabs the Analyzer's Tape mode
//! draws from.
//!
//! The columns are analyzed from the tape's own audio rather than kept from
//! the live analyzer, so they follow the recording exactly: a pass that wins
//! a stretch replaces its columns by being analyzed there, a seam is analyzed
//! through the crossfade an audition plays, and a changed analysis window
//! re-analyzes the whole tape rather than leaving a seam where it changed.
//!
//! **Bytes, not powers.** A column is stored as the heatmap's own dB bytes,
//! and level `n` of the pyramid holds slabs of `2^n` columns, each the power
//! mean of the level-0 bytes it covers, read back through [`db_of`]. Merging
//! from bytes rather than from bfloat16 powers can move a merged cell by one
//! [`DB_STEP`](harmonigraph_core::spectrogram::DB_STEP), never more, because
//! every level is merged from level 0 in one step rather than level from
//! level. It halves the memory a power store would take, and no zoom ever
//! refolds: every slab the pane can ask for is already here.

use std::collections::{BTreeMap, BTreeSet};
use std::sync::Arc;

pub use harmonigraph_analysis::AnalyzerConfig;
use harmonigraph_analysis::ChannelBank;
use harmonigraph_core::spectrogram::{db_of, quantize, BucketDb};
use harmonigraph_core::spectrum::SPECTRUM_BINS;
use harmonigraph_render::SpectrogramSlab;

/// Song seconds between columns: level 0's slab width.
pub const COLUMN_SECONDS: f64 = 0.032;

/// Pyramid levels. The widest slab merges `2^(LEVELS - 1)` columns, 8.2 s,
/// so even a two-hour window fits the pane in under a thousand slabs.
pub const LEVELS: usize = 9;

/// Memory per song second: the columns, and the pyramid above them, which
/// holds about as much again. The tape's cap counts this beside its audio.
pub const BYTES_PER_SECOND: f64 = 2.0 * SPECTRUM_BINS as f64 / COLUMN_SECONDS;

/// The song seconds column `k` is centred on.
pub fn column_seconds(k: i64) -> f64 {
    k as f64 * COLUMN_SECONDS
}

/// The slabs of every level, by key: slab `j` of level `n` covers columns
/// `j * 2^n .. (j + 1) * 2^n`. A key with no entry has nothing recorded under
/// it, and draws as silence.
#[derive(Clone, Default)]
pub struct TapeSpectrogram {
    levels: [BTreeMap<i64, SpectrogramSlab>; LEVELS],
    /// Columns set since [`settle`](Self::settle) last rebuilt the levels
    /// above them.
    unsettled: BTreeSet<i64>,
}

impl std::fmt::Debug for TapeSpectrogram {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("TapeSpectrogram").field("columns", &self.columns()).finish()
    }
}

impl TapeSpectrogram {
    pub fn slab(&self, level: usize, key: i64) -> Option<&SpectrogramSlab> {
        self.levels.get(level)?.get(&key)
    }

    pub fn is_empty(&self) -> bool {
        self.levels[0].is_empty()
    }

    pub fn columns(&self) -> usize {
        self.levels[0].len()
    }

    pub fn clear(&mut self) {
        *self = TapeSpectrogram::default();
    }

    /// Set slab `key` of `level` as another copy holds it: `None` removes it.
    pub fn put(&mut self, level: usize, key: i64, slab: Option<SpectrogramSlab>) {
        match slab {
            Some(slab) => self.levels[level].insert(key, slab),
            None => self.levels[level].remove(&key),
        };
    }

    /// Set column `k`, or remove it with `None`. The levels above it follow
    /// at the next [`settle`](Self::settle).
    pub fn set_column(&mut self, k: i64, column: Option<SpectrogramSlab>) {
        self.put(0, k, column);
        self.unsettled.insert(k);
    }

    /// Rebuild every slab above a column set since the last call, and report
    /// each slab of every level that may have changed, columns included.
    pub fn settle(&mut self, changed: &mut BTreeSet<(usize, i64)>) {
        let columns = std::mem::take(&mut self.unsettled);
        changed.extend(columns.iter().map(|&k| (0, k)));
        let mut power = vec![0.0f32; SPECTRUM_BINS];
        for level in 1..LEVELS {
            let mut last = None;
            for &k in &columns {
                let key = k >> level;
                if last == Some(key) {
                    continue;
                }
                last = Some(key);
                let width = 1i64 << level;
                let first = key * width;
                power.fill(0.0);
                let mut any = false;
                let powers = powers();
                for (_, column) in self.levels[0].range(first..first + width) {
                    any = true;
                    for (sum, &byte) in power.iter_mut().zip(column.iter()) {
                        *sum += powers[usize::from(byte)];
                    }
                }
                let slab = any.then(|| {
                    let scale = 1.0 / width as f32;
                    power.iter().map(|&p| quantize(p * scale)).collect::<SpectrogramSlab>()
                });
                self.put(level, key, slab);
                changed.insert((level, key));
            }
        }
    }
}

/// The power each stored byte stands for, with the floor as silence: a merge
/// that takes in a gap or the floor dims toward the floor rather than lifting
/// off it. Taken once per merge rather than per byte, so the inner loop is a
/// plain table read.
fn powers() -> &'static [f32; 256] {
    static POWERS: std::sync::LazyLock<[f32; 256]> = std::sync::LazyLock::new(|| {
        std::array::from_fn(|b| if b == 0 { 0.0 } else { 10f32.powf(db_of(b as BucketDb) / 10.0) })
    });
    &POWERS
}

/// Analyzes one column at a time from audio read by song frame, through the
/// same estimator as the live analyzer.
#[derive(Default)]
pub struct ColumnAnalyzer {
    bank: Option<(ChannelBank, AnalyzerConfig, usize)>,
    scratch: Vec<f32>,
}

impl ColumnAnalyzer {
    /// The frame and lag `config` reads with, in frames: a column centred on
    /// frame `c` reads `c + lag - frame .. c + lag`.
    pub fn reach(config: AnalyzerConfig) -> (i64, i64) {
        let lag = (config.column_lag() * f64::from(config.sample_rate)).round() as i64;
        (config.fft_size as i64, lag)
    }

    /// The column centred on song frame `centre`, from `read`, which fills
    /// interleaved frames from a song frame on, silence where nothing is held.
    pub fn column(
        &mut self,
        config: AnalyzerConfig,
        channels: usize,
        centre: i64,
        read: impl FnOnce(i64, &mut [f32]),
    ) -> SpectrogramSlab {
        let fresh = self.bank.as_ref().is_none_or(|(_, held, held_channels)| {
            held.fft_size != config.fft_size
                || held.sample_rate != config.sample_rate
                || *held_channels != channels
        });
        if fresh {
            self.bank = Some((ChannelBank::new(config, channels), config, channels));
        }
        let (bank, _, _) = self.bank.as_mut().expect("built above");
        let (frame, lag) = Self::reach(config);
        self.scratch.clear();
        self.scratch.resize(frame as usize * channels, 0.0);
        read(centre + lag - frame, &mut self.scratch);
        bank.restart();
        bank.push_frames(&self.scratch);
        let power = bank.power_sum().expect("a full frame was pushed");
        power.iter().map(|&p| quantize(p)).collect()
    }
}

/// A slab of silence.
pub fn silent_slab() -> SpectrogramSlab {
    static SILENT: std::sync::LazyLock<SpectrogramSlab> =
        std::sync::LazyLock::new(|| Arc::from([0; SPECTRUM_BINS]));
    SILENT.clone()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn flat(byte: BucketDb) -> SpectrogramSlab {
        Arc::from([byte; SPECTRUM_BINS])
    }

    #[test]
    fn a_level_is_the_power_mean_of_its_columns_and_a_gap_counts_as_silence() {
        let mut tape = TapeSpectrogram::default();
        // -60 dB in columns 0 and 1: level 1 slab 0 holds both, -60 dB.
        let byte = quantize(1e-6);
        tape.set_column(0, Some(flat(byte)));
        tape.set_column(1, Some(flat(byte)));
        // Column 2 alone in level 1 slab 1: half its power, 3 dB down.
        tape.set_column(2, Some(flat(byte)));
        let mut changed = BTreeSet::new();
        tape.settle(&mut changed);
        assert_eq!(tape.slab(1, 0).unwrap()[100], byte);
        assert_eq!(tape.slab(1, 1).unwrap()[100], quantize(0.5e-6));
        // Level 2 slab 0 covers all four: three quarters.
        assert_eq!(tape.slab(2, 0).unwrap()[100], quantize(0.75e-6));
        assert!(changed.contains(&(0, 2)) && changed.contains(&(8, 0)));

        // Removing the columns removes every slab above them.
        for k in 0..3 {
            tape.set_column(k, None);
        }
        tape.settle(&mut changed);
        assert!((0..LEVELS).all(|level| tape.slab(level, 0).is_none()));
    }

    #[test]
    fn a_column_reads_the_frame_its_lag_places_around_its_centre() {
        let config = AnalyzerConfig { sample_rate: 48_000.0, fft_size: 4096 };
        let (frame, lag) = ColumnAnalyzer::reach(config);
        let mut analyzer = ColumnAnalyzer::default();
        let mut asked = None;
        let column = analyzer.column(config, 2, 10_000, |from, out| {
            asked = Some((from, out.len()));
            // A full-scale 1 kHz sine in both channels.
            for (i, frame) in out.chunks_mut(2).enumerate() {
                let s = (2.0 * std::f32::consts::PI * 1000.0 * i as f32 / 48_000.0).sin();
                frame.fill(s);
            }
        });
        assert_eq!(asked, Some((10_000 + lag - frame, frame as usize * 2)));
        let peak = column.iter().copied().max().unwrap();
        assert!(db_of(peak) > -6.0, "a full-scale sine reads near 0 dB, read {}", db_of(peak));
    }
}
