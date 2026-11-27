//! The tape's spectrogram, kept by the input worker: which columns a change
//! to the recording or to the analysis window reaches, analyzed a few at a
//! time between the step's two holds, and copied into the Tape view in the
//! first.
//!
//! The worker's [`TapeSpectrogram`] is the one that is settled; the view's is
//! a copy of the same shared slabs, brought up to date by the slabs that
//! changed. Settling outside the lock keeps a re-analysis, which rebuilds a
//! few thousand merged slabs a second, from holding up the frame.

use std::collections::BTreeSet;
use std::time::{Duration, Instant};

use harmonigraph_ui::tape::spectrogram::{
    column_seconds, AnalyzerConfig, ColumnAnalyzer, TapeSpectrogram, COLUMN_SECONDS,
};
use harmonigraph_ui::SpectrumConfig;

use super::store::{fade_frames, Store};

/// How long a step may spend analyzing, beyond its first column.
///
/// A column took about 0.08, 0.15 and 0.30 ms through the Fast, Balanced and
/// Precise windows back to back, and two to three times that in a step woken
/// from the worker's sleep (release build, stereo at 48 kHz, 2026-11-22), so
/// keeping up with a recording, a column per 32 ms of song, costs 0.3% to 3%
/// of a core, well inside this. What the budget bounds is a re-analysis of
/// the whole tape after the window changes: measured with each step's
/// overrunning last column and the levels' rebuilds included, about a fifth of
/// a core with the editor open (a step every
/// [`POLL`](crate::background::POLL) plus its work) and a sixteenth with it
/// shut. Five minutes of song caught up in 4 to 9 s through Fast, 8 to 25 s
/// through Balanced and 14 to 38 s through Precise with the editor open, the
/// spread being how slow a woken step's columns ran on a loaded machine, and
/// in about a minute through Balanced with it shut. The step that sees the
/// switch also queues the whole tape outside this budget, about 2.3 ms for
/// 25 minutes.
pub(crate) const STEP_BUDGET: Duration = Duration::from_millis(1);

/// How often the levels above the columns are rebuilt while columns are still
/// waiting. A rebuild re-merges a slab of every level over the columns a step
/// set, about half a millisecond whether the step set one column or thirty,
/// so a re-analysis that rebuilt every step would spend half as much again as
/// its budget on slabs the next step rebuilds. At this rate the picture still
/// fills in at 20 Hz; once nothing waits, every step that sets a column
/// rebuilds.
const SETTLE_EVERY: Duration = Duration::from_millis(50);

#[derive(Default)]
pub(crate) struct Spectrum {
    analyzer: ColumnAnalyzer,
    /// The settings the analysis window comes from, as Tape mode last showed
    /// them.
    settings: Option<SpectrumConfig>,
    /// The frame, at the store's rate, that the columns queued and analyzed
    /// so far were scheduled under: when the settings' differs,
    /// [`adopt`](Self::adopt) queues the whole tape again.
    scheduled: Option<usize>,
    tape: TapeSpectrogram,
    /// Columns a change reached and that have not been analyzed since.
    pending: BTreeSet<i64>,
    /// Slabs the view's copy has not been given yet.
    changed: BTreeSet<(usize, i64)>,
    /// The view's copy must be emptied before it is given anything.
    cleared: bool,
    /// When the levels were last rebuilt.
    settled: Option<Instant>,
}

impl Spectrum {
    fn config(&self, store: &Store) -> Option<AnalyzerConfig> {
        Some(self.settings?.analyzer(store.rate))
    }

    /// Take the Analyzer's settings: a copy, since it runs under the editor
    /// lock. A different analysis window is acted on by the next
    /// [`analyze`](Self::analyze), outside it.
    pub(crate) fn configure(&mut self, settings: &SpectrumConfig) {
        self.settings = Some(*settings);
    }

    /// A different analysis window re-analyzes everything recorded, so no seam
    /// is left where it changed; so do the first settings, if anything was
    /// recorded before them. Never under the lock: on a long tape this queues
    /// tens of thousands of columns, about 2 ms for 25 minutes.
    fn adopt(&mut self, store: Option<&Store>) {
        let Some(store) = store else { return };
        let Some(config) = self.config(store) else { return };
        if self.scheduled == Some(config.fft_size) {
            return;
        }
        self.scheduled = Some(config.fft_size);
        let all: Vec<(i64, i64)> = store.chunks().map(|c| (c.start, c.end)).collect();
        self.invalidate(store, &all);
    }

    /// The store is gone or new: nothing it held is drawn.
    pub(crate) fn clear(&mut self) {
        self.tape.clear();
        self.scheduled = None;
        self.pending.clear();
        self.changed.clear();
        self.cleared = true;
    }

    /// Queue every column whose frame reaches into a stretch of `store` that
    /// changed, seams' crossfades included.
    pub(crate) fn invalidate(&mut self, store: &Store, ranges: &[(i64, i64)]) {
        let Some(config) = self.config(store) else { return };
        let (frame, lag) = (config.fft_size as i64, ColumnAnalyzer::lag(config));
        let margin = fade_frames(f64::from(store.rate)) as i64;
        let hop = COLUMN_SECONDS * f64::from(store.rate);
        for &(a, b) in ranges {
            // Column k reads `c + lag - frame .. c + lag` around its centre c.
            let first = ((a - margin - lag) as f64 / hop).floor().max(0.0) as i64;
            let last = ((b + margin + frame - lag) as f64 / hop).ceil() as i64;
            self.pending.extend(first..=last);
        }
    }

    #[cfg(test)]
    pub(crate) fn pending(&self) -> usize {
        self.pending.len()
    }

    /// Analyze waiting columns until `budget` has passed, at least one, in
    /// song order, first queueing the whole tape if the window changed.
    pub(crate) fn analyze(&mut self, store: Option<&Store>, budget: Duration) {
        self.adopt(store);
        let (Some(store), Some(_)) = (store, self.pending.first()) else { return };
        let Some(config) = self.config(store) else { return };
        let started = Instant::now();
        let rate = f64::from(store.rate);
        while let Some(k) = self.pending.pop_first() {
            let centre = (column_seconds(k) * rate).round() as i64;
            if store.holding(centre).is_none() {
                if self.tape.slab(0, k).is_some() {
                    self.tape.set_column(k, None);
                }
            } else {
                let column = self.analyzer.column(config, store.channels, centre, |from, out| {
                    store.read(from, out);
                });
                self.tape.set_column(k, Some(column));
            }
            if started.elapsed() >= budget {
                break;
            }
        }
        if self.pending.is_empty() || self.settled.is_none_or(|at| at.elapsed() >= SETTLE_EVERY) {
            self.tape.settle(&mut self.changed);
            self.settled = Some(Instant::now());
        }
    }

    /// Bring the view's copy up to date, returning any cleared spectrogram
    /// displaced from it so the caller can drop it outside the lock.
    pub(crate) fn sync(&mut self, view: &mut TapeSpectrogram) -> Option<TapeSpectrogram> {
        let displaced = std::mem::take(&mut self.cleared).then(|| view.take_cleared());
        for (level, key) in std::mem::take(&mut self.changed) {
            view.put(level, key, self.tape.slab(level, key).cloned());
        }
        displaced
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tape::store::{Chunk, Stamp};
    use harmonigraph_core::spectrum::{bucket_pitch, midi_to_bucket_x};
    use harmonigraph_ui::SpectrumWindow;

    const RATE: f32 = 48_000.0;

    /// Mono song frames `start..end` of a sine at MIDI `midi`, from pass
    /// `pass` started at song frame `pass_start`.
    fn sine(start: i64, end: i64, midi: f32, pass: u64, pass_start: i64) -> Chunk {
        let hz = 440.0 * 2f32.powf((midi - 69.0) / 12.0);
        let when = Instant::now() + Duration::from_secs(pass);
        Chunk {
            start,
            end,
            stamp: Stamp { pass, when, pass_start },
            data_start: start,
            samples: (start..end)
                .map(|f| 0.5 * (2.0 * std::f32::consts::PI * hz * f as f32 / RATE).sin())
                .collect(),
        }
    }

    fn insert(spectrum: &mut Spectrum, store: &mut Store, chunk: Chunk) {
        store.insert(chunk);
        let changed = store.take_changed();
        spectrum.invalidate(store, &changed);
        spectrum.analyze(Some(store), Duration::MAX);
        assert_eq!(spectrum.pending(), 0);
    }

    /// The bucket MIDI pitch `midi` falls in.
    fn bucket(midi: f32) -> usize {
        midi_to_bucket_x(midi) as usize
    }

    /// The MIDI pitch the view's copy of column `k` peaks at.
    fn peak(view: &TapeSpectrogram, k: i64) -> Option<f32> {
        let column = view.slab(0, k)?;
        let (bucket, _) = column.iter().enumerate().max_by_key(|&(_, &b)| b)?;
        Some(bucket_pitch(bucket))
    }

    #[test]
    fn columns_follow_the_pass_that_wins_and_a_new_window_reanalyzes_them() {
        let seconds = |s: f64| (s * f64::from(RATE)) as i64;
        let column = |s: f64| (s / COLUMN_SECONDS).round() as i64;
        let mut store = Store::new(RATE, 1);
        let mut spectrum = Spectrum::default();
        // Precise lags its columns about 170 ms, five columns, behind the
        // newest sample each reads.
        let mut settings = SpectrumConfig { window: SpectrumWindow::Precise, ..Default::default() };
        spectrum.configure(&settings);
        // A4 from the song's start through 4 s.
        insert(&mut spectrum, &mut store, sine(0, seconds(4.0), 69.0, 1, 0));
        let mut view = TapeSpectrogram::default();
        spectrum.sync(&mut view);
        let near = |got: Option<f32>, want: f32| got.is_some_and(|m| (m - want).abs() < 0.5);
        assert!(near(peak(&view, column(2.0)), 69.0), "{:?}", peak(&view, column(2.0)));
        assert!(view.slab(0, column(4.5)).is_none(), "nothing recorded there");
        assert!(view.slab(4, 0).is_some(), "the levels above the columns follow");

        // A cold pass over 1..3 s loses to the warm recording: nothing moves.
        insert(&mut spectrum, &mut store, sine(seconds(1.0), seconds(3.0), 76.0, 2, seconds(1.0)));
        spectrum.sync(&mut view);
        assert!(near(peak(&view, column(2.0)), 69.0));

        // A pass from the song's start that wins 1..2 s with E5 replaces the
        // columns there and only there, and reaches the one centred 0.1 s
        // before the seam through its frame: it hears the replay's first 70 ms.
        let early = column(0.9);
        let e5 = |view: &TapeSpectrogram| view.slab(0, early).unwrap()[bucket(76.0)];
        let before = e5(&view);
        insert(&mut spectrum, &mut store, sine(seconds(1.0), seconds(2.0), 76.0, 3, 0));
        spectrum.sync(&mut view);
        assert!(near(peak(&view, column(1.5)), 76.0), "{:?}", peak(&view, column(1.5)));
        assert!(near(peak(&view, column(3.0)), 69.0));
        assert!(e5(&view) >= before + 20, "E5 before the seam: {before} -> {}", e5(&view));

        // Another window re-analyzes every recorded column.
        let precise = view.slab(0, column(3.0)).cloned();
        settings.window = SpectrumWindow::Fast;
        spectrum.configure(&settings);
        assert_eq!(spectrum.pending(), 0, "the copy taken under the lock queues nothing");
        spectrum.analyze(Some(&store), Duration::ZERO);
        assert!(spectrum.pending() >= column(4.0) as usize - 1, "{}", spectrum.pending());
        spectrum.analyze(Some(&store), Duration::MAX);
        spectrum.sync(&mut view);
        assert!(near(peak(&view, column(3.0)), 69.0));
        assert_ne!(view.slab(0, column(3.0)).cloned(), precise, "the column was not re-analyzed");
    }

    #[test]
    fn sync_retires_cleared_spectrogram_for_off_lock_drop() {
        let mut view = TapeSpectrogram::default();
        let mut changed = BTreeSet::new();
        for k in 0..64i64 {
            let slab: harmonigraph_render::SpectrogramSlab = std::sync::Arc::from(
                [(k as u8).wrapping_add(1); harmonigraph_core::spectrum::SPECTRUM_BINS],
            );
            view.set_column(k, Some(slab));
        }
        view.settle(&mut changed);
        let column_probe = view.slab(0, 0).expect("column 0 populated").clone();
        let level_probe = view.slab(1, 0).expect("level 1 slab 0 populated").clone();
        assert_eq!(std::sync::Arc::strong_count(&column_probe), 2);
        assert_eq!(std::sync::Arc::strong_count(&level_probe), 2);

        let mut spectrum = Spectrum::default();
        spectrum.clear();
        let displaced = spectrum.sync(&mut view);

        assert!(view.is_empty());
        assert!(displaced.is_some());
        assert_eq!(
            std::sync::Arc::strong_count(&column_probe),
            2,
            "sync under EditorLock must return cleared column slabs without dropping them"
        );
        assert_eq!(
            std::sync::Arc::strong_count(&level_probe),
            2,
            "sync under EditorLock must return cleared pyramid slabs without dropping them"
        );

        drop(displaced);
        assert_eq!(
            std::sync::Arc::strong_count(&column_probe),
            1,
            "dropping displaced spectrogram off-lock must release column slabs"
        );
        assert_eq!(
            std::sync::Arc::strong_count(&level_probe),
            1,
            "dropping displaced spectrogram off-lock must release pyramid slabs"
        );
    }
}
