//! Fitting the Spectral pane's pitch range to what a piece fills, for an
//! export's [`crate::PitchRangeRender::Fit`].
//!
//! Only a render can answer it, as only a render knows its own length for
//! Video history's Fit video: the whole soundtrack has to have been heard
//! before the first frame is drawn. So the offline renderer analyzes the
//! soundtrack once ahead of the frames, hands every column and every note's
//! pitch to a [`PitchFit`], and draws the whole video at the range it returns.

use crate::panes::spectral::axes;
use crate::{AnalyzedColumn, SpectrumConfig};
use harmonigraph_core::spectrum::{
    bucket_pitch, SPECTRUM_BINS, SPECTRUM_MAX_MIDI, SPECTRUM_MIN_MIDI,
};

/// How far up the volume colours' window a bucket has to draw to count as
/// lit: a fifth of the way from its floor to its ceiling. The colours rather
/// than the curve, because the heatmap is the picture a dark band shows in.
///
/// Over the floor alone is not enough. Window leakage and rumble draw the
/// bottom octaves faintly in pieces that have nothing there: on three takes of
/// 2026-11-19 the band under 60 Hz drew over the floor in up to 2.6% of its
/// columns, and over a fifth of the way up in 0.2%. The curve's window is no
/// stricter: at the fresh look a fifth of the way up it is the colours' floor,
/// and fitting on it took those takes down to 36-52 Hz.
const LIT_LEVEL: f32 = 0.2;

/// How long in all a bucket has to be lit to count, so a single transient's
/// broadband smear does not hold the whole axis open.
const LIT_SECONDS: f64 = 0.5;

/// Room left past the outermost lit pitch at each end, in semitones, so what
/// is drawn there fades out inside the frame rather than against its edge.
const MARGIN: f32 = 1.0;

/// Seconds each bucket was lit, and the pitches the notes reached, over a
/// render; see the module doc.
pub struct PitchFit {
    /// Per bucket, the power at and over which it counts as lit: [`LIT_LEVEL`]
    /// of the way up the volume colours at its pitch.
    threshold: Vec<f32>,
    lit: Vec<f64>,
    /// The lowest and highest pitch a note on the axis reached, as MIDI.
    notes: Option<(f32, f32)>,
}

impl PitchFit {
    /// A fit read through `config`'s volume colours and tilt. Its Frequency
    /// range plays no part: Fit piece frames the whole axis, as Fit video
    /// spans the whole render whatever History duration says, and Dialled is
    /// the way to crop by hand (Yan, #1976).
    pub fn new(config: &SpectrumConfig) -> Self {
        let threshold = (0..SPECTRUM_BINS)
            .map(|bucket| {
                let midi = bucket_pitch(bucket);
                let db = axes::coloured_db(config, LIT_LEVEL, midi);
                10f32.powf(db / 10.0)
            })
            .collect();
        PitchFit { threshold, lit: vec![0.0; SPECTRUM_BINS], notes: None }
    }

    /// Count one analyzed column of the soundtrack.
    pub fn column(&mut self, column: &AnalyzedColumn) {
        for ((lit, &power), &threshold) in
            self.lit.iter_mut().zip(&column.power).zip(&self.threshold)
        {
            if power >= threshold {
                *lit += column.step;
            }
        }
    }

    /// Count a pitch a note sounded at, as MIDI, so the roll's ribbons stay in
    /// the range however quietly they sounded. One off the axis draws nowhere,
    /// so it counts for nothing.
    pub fn note(&mut self, midi: f32) {
        if !(SPECTRUM_MIN_MIDI..=SPECTRUM_MAX_MIDI).contains(&midi) {
            return;
        }
        let (low, high) = self.notes.get_or_insert((midi, midi));
        (*low, *high) = (low.min(midi), high.max(midi));
    }

    /// The range to draw: [`MARGIN`] past the outermost lit bucket and note,
    /// widened about its centre to the least span the range bar allows, all on
    /// the axis. `None` when nothing was lit or played, which keeps the dialled
    /// range.
    pub fn range(&self) -> Option<(f32, f32)> {
        let lit =
            (0..SPECTRUM_BINS).filter(|&bucket| self.lit[bucket] >= LIT_SECONDS).map(bucket_pitch);
        let notes = self.notes.into_iter().flat_map(|(low, high)| [low, high]);
        let (low, high) = lit.chain(notes).fold(None, |range: Option<(f32, f32)>, midi| {
            Some(range.map_or((midi, midi), |(low, high)| (low.min(midi), high.max(midi))))
        })?;
        let short = (crate::PITCH_RANGE_MIN_SPAN - (high - low) - 2.0 * MARGIN).max(0.0) / 2.0;
        Some(harmonigraph_scene::clamp_span(
            low - MARGIN - short,
            high + MARGIN + short,
            (SPECTRUM_MIN_MIDI, SPECTRUM_MAX_MIDI),
            crate::PITCH_RANGE_MIN_SPAN,
        ))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use harmonigraph_core::spectrum::BINS_PER_SEMITONE;

    /// A column `step` seconds long with `power` in every bucket from `low`
    /// to `high` MIDI, and silence elsewhere.
    fn column(low: f32, high: f32, power: f32, step: f64) -> AnalyzedColumn {
        let mut buckets = [0.0; SPECTRUM_BINS];
        for (bucket, value) in buckets.iter_mut().enumerate() {
            if (low..=high).contains(&bucket_pitch(bucket)) {
                *value = power;
            }
        }
        AnalyzedColumn { time: 0.0, power: buckets, step }
    }

    /// A look whose reading the tests set, rather than the fresh one's, with a
    /// Frequency range narrower than what it plays, which the fit ignores.
    fn look() -> SpectrumConfig {
        SpectrumConfig {
            volume_floor_db: -68.0,
            volume_ceiling_db: -35.0,
            tilt: -4.5,
            low_midi: 60.0,
            high_midi: 84.0,
            ..Default::default()
        }
    }

    /// Both ends go to what the piece fills, past the dialled range and a note
    /// under the audio included; a sound over the lit level for less than half
    /// a second in all, one too faint at its pitch, and a note off the axis
    /// hold nothing open.
    #[test]
    fn the_fit_frames_what_is_lit_and_played_on_the_whole_axis() {
        let config = look();
        let loud = 1.0; // 0 dB, over the window's ceiling at every pitch.
        let mut fit = PitchFit::new(&config);
        for _ in 0..100 {
            fit.column(&column(48.0, 96.0, loud, 0.01));
        }
        // 0.4 s in all, at 30 MIDI: under LIT_SECONDS.
        for _ in 0..40 {
            fit.column(&column(29.5, 30.5, loud, 0.01));
        }
        // Drawn everywhere, but each bucket too faint at its own pitch to count.
        let mut faint = column(0.0, 0.0, 0.0, 0.01);
        for (bucket, power) in faint.power.iter_mut().enumerate() {
            *power = 10f32
                .powf(axes::coloured_db(&config, LIT_LEVEL / 2.0, bucket_pitch(bucket)) / 10.0);
        }
        for _ in 0..100 {
            fit.column(&faint);
        }
        fit.note(43.0);
        fit.note(SPECTRUM_MAX_MIDI + 5.0);

        let (low, high) = fit.range().unwrap();
        // Bucket centres sit off the whole semitones, so the outermost lit one
        // is up to a bucket inside the band's edge.
        let bucket = 1.0 / BINS_PER_SEMITONE as f32;
        assert!((low - (43.0 - MARGIN)).abs() < 1e-4, "low end {low}");
        assert!((high - (96.0 + MARGIN)).abs() <= bucket, "high end {high}");
    }

    /// A piece narrower than the range bar's least span opens at that span,
    /// centred on it, and one at an end of the axis is held on it; one that
    /// fills nothing keeps the dialled range.
    #[test]
    fn a_narrow_piece_opens_at_the_least_span_and_an_empty_one_at_the_dial() {
        let mut fit = PitchFit::new(&look());
        assert_eq!(fit.range(), None);
        fit.note(60.0);
        let (low, high) = fit.range().unwrap();
        assert!((high - low - crate::PITCH_RANGE_MIN_SPAN).abs() < 1e-4, "{low}..{high}");
        assert!(((low + high) / 2.0 - 60.0).abs() < 1e-4, "{low}..{high}");
        let mut fit = PitchFit::new(&look());
        fit.note(SPECTRUM_MIN_MIDI);
        assert_eq!(fit.range().unwrap().0, SPECTRUM_MIN_MIDI);
    }
}
