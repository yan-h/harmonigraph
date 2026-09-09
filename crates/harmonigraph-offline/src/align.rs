//! Lining a replacement soundtrack up with the visualization.
//!
//! The problem this solves: live DAW playback can crackle, so the audio
//! the take recorded is not good enough to ship. You bounce a clean WAV
//! of the same performance offline and want *that* in the video — but a
//! naive swap can drift. The clean bounce may start at a different song
//! position than where recording armed, and plugin-delay compensation can
//! shift the master bus by a constant amount you'd have to discover by
//! ear.
//!
//! The fix leans on something the take already has. Its own recorded
//! audio is stamped to the same transport clock as the notes, so it is
//! *already aligned to the picture*. It is too crackly to be the
//! soundtrack, but it is a perfect timing reference. Cross-correlate the
//! clean bounce against it and the clean bounce inherits that alignment.
//!
//! Correlation is done on **onset-strength envelopes**, not raw samples:
//!
//! - it is immune to level differences, and to the two mixes not being
//!   bit-identical (live and offline renders rarely are);
//! - crackles are sparse transients that barely dent it;
//! - onsets give a sharp correlation peak where a raw energy envelope,
//!   flat across sustained passages, would give a mushy one;
//! - it is rate-independent — both files reduce to the same time grid, so
//!   a 44.1 kHz reference aligns a 48 kHz bounce without resampling.

use crate::wav::Audio;

/// Envelope frame length. 5 ms buys ~5 ms alignment — comfortably inside
/// one frame at 60 fps (16.7 ms), so the spectrum never visibly leads or
/// lags the picture.
const HOP: f64 = 0.005;

/// How much of the reference to match against the clean file. A window
/// rather than the whole thing: it is cheaper, and anchoring on the
/// reference's most energetic stretch makes the peak sharper and the
/// match more certain.
const TEMPLATE_SECONDS: f64 = 12.0;

/// Below this many envelope frames there is not enough to correlate.
const MIN_FRAMES: usize = 8;

/// The result of lining a soundtrack up against a reference.
pub struct Alignment {
    /// Take-time, in seconds, of the soundtrack's first sample — what the
    /// renderer needs to place both the spectrum and the muxed audio.
    pub start: f64,
    /// Peak normalized correlation, in `-1..=1`. Near 1 is a confident
    /// match; low means the two files may not be the same performance.
    pub confidence: f32,
}

/// RMS energy across the original channels per [`HOP`]-length frame.
/// Squaring before combining channels preserves opposite-phase attacks and
/// avoids allocating a whole-file mono signal just to reduce it again.
///
/// Frame boundaries are placed by *time*, not by a fixed sample count, so
/// frame `k` covers exactly `[k*HOP, (k+1)*HOP)` seconds whatever the rate
/// — 44.1 kHz rounds to 220 samples a frame, 48 kHz to 240, and a fixed
/// count would let their time grids drift apart over a few seconds and
/// smear the correlation. Placing boundaries by time keeps both files on
/// one grid, which is what lets a reference and a bounce at different
/// rates line up at all.
fn envelope(audio: &mut Audio) -> Result<Vec<f32>, String> {
    let sr = f64::from(audio.sample_rate);
    let channels = audio.channels;
    let frames = (audio.seconds() / HOP).floor() as usize;
    (0..frames)
        .map(|k| {
            let start = (k as f64 * HOP * sr) as usize;
            let end = (((k + 1) as f64 * HOP * sr) as usize).min(audio.frames()).max(start);
            let mut sum_sq = 0.0;
            audio.for_frames(start..end, |chunk| {
                // Preserve the original sample-by-sample accumulation order,
                // including across physical decoding boundaries.
                for &x in chunk {
                    sum_sq += f64::from(x) * f64::from(x);
                }
            })?;
            let count = (end - start) * channels;
            Ok(if count == 0 { 0.0 } else { (sum_sq / count as f64).sqrt() as f32 })
        })
        .collect()
}

/// Onset strength: the half-wave-rectified first difference of the
/// envelope. Emphasizes attacks, which is where two recordings of one
/// performance agree most sharply.
fn onset_strength(envelope: &[f32]) -> Vec<f32> {
    let mut out = Vec::with_capacity(envelope.len());
    out.push(0.0);
    for i in 1..envelope.len() {
        out.push((envelope[i] - envelope[i - 1]).max(0.0));
    }
    out
}

/// Start frame of the highest-energy window of length `width` in `signal`
/// — the stretch worth anchoring the correlation on.
fn strongest_window(signal: &[f32], width: usize) -> usize {
    if signal.len() <= width {
        return 0;
    }
    let mut prefix = vec![0.0f64; signal.len() + 1];
    for (i, &value) in signal.iter().enumerate() {
        prefix[i + 1] = prefix[i] + f64::from(value);
    }
    let mut best_start = 0;
    let mut best_sum = f64::MIN;
    for start in 0..=signal.len() - width {
        let sum = prefix[start + width] - prefix[start];
        if sum > best_sum {
            best_sum = sum;
            best_start = start;
        }
    }
    best_start
}

/// The lag (into `haystack`) at which the already-centered `template`
/// correlates best, and that peak's normalized value.
///
/// The template is mean-subtracted up front, so its own mean is zero and
/// the window-mean term of the normalized cross-correlation drops out of
/// the numerator — leaving a plain dot product. The denominator's
/// per-window mean and norm come from prefix sums, so only the dot
/// product is `O(width)` per lag.
fn best_lag(template: &[f32], template_norm: f64, haystack: &[f32]) -> (usize, f32) {
    let width = template.len();
    if haystack.len() < width {
        return (0, 0.0);
    }
    let n = haystack.len();
    let mut sum = vec![0.0f64; n + 1];
    let mut sum_sq = vec![0.0f64; n + 1];
    for (i, &value) in haystack.iter().enumerate() {
        let v = f64::from(value);
        sum[i + 1] = sum[i] + v;
        sum_sq[i + 1] = sum_sq[i] + v * v;
    }

    let mut best_lag = 0;
    let mut best = -2.0f64;
    for lag in 0..=n - width {
        let window_sum = sum[lag + width] - sum[lag];
        let mean = window_sum / width as f64;
        // ||window - mean||^2, via the prefix sums.
        let variance = (sum_sq[lag + width] - sum_sq[lag]) - width as f64 * mean * mean;
        if variance <= 1e-12 {
            continue;
        }
        let mut dot = 0.0f64;
        for i in 0..width {
            dot += f64::from(template[i]) * f64::from(haystack[lag + i]);
        }
        let correlation = dot / (template_norm * variance.sqrt());
        if correlation > best {
            best = correlation;
            best_lag = lag;
        }
    }
    (best_lag, best as f32)
}

/// Reduce a source once for every alignment attempt that needs its attacks.
/// The replacement path reuses this small sequence for recording and MIDI
/// references, so a failed recording match does not decode the source again.
pub fn audio_onsets(audio: &mut Audio) -> Result<Vec<f32>, String> {
    Ok(onset_strength(&envelope(audio)?))
}

/// Find where `clean` sits on the take's timeline by matching its onsets
/// against the MIDI **note-ons** directly — no reference recording needed.
/// Each onset is `(take-time seconds, velocity)`; `span` is the take's
/// duration.
///
/// This is the path for a take that recorded no scratch audio: the notes are
/// already on the take clock, so a bounce of the same performance lines its
/// transients up against them. Reliable when the material has clear attacks
/// (percussive, plucked); soft or legato material gives a low confidence and
/// is better nudged by eye.
pub fn align_to_notes(onsets: &[(f64, f32)], span: f64, clean_onsets: &[f32]) -> Option<Alignment> {
    let reference_onsets = note_onset_envelope(onsets, span);
    // The note train is on the take clock, so its frame 0 is take-time 0.
    align(&reference_onsets, 0.0, clean_onsets)
}

/// An onset-strength envelope synthesized from MIDI note-on times, on the same
/// [`HOP`] grid as [`envelope`], so it correlates against an audio onset
/// envelope directly. Each note-on is a short velocity-weighted attack bump.
fn note_onset_envelope(onsets: &[(f64, f32)], span: f64) -> Vec<f32> {
    let frames = (span.max(0.0) / HOP).ceil() as usize + 2;
    let mut env = vec![0.0f32; frames];
    for &(t, velocity) in onsets {
        if t < 0.0 {
            continue;
        }
        let f = (t / HOP) as usize;
        if f < frames {
            // A two-frame bump: an audio onset spike has a few ms of width, so
            // this tolerates sub-frame timing without smearing the peak.
            env[f] += velocity.max(0.0);
            if f + 1 < frames {
                env[f + 1] += 0.5 * velocity.max(0.0);
            }
        }
    }
    env
}

/// Correlate source onset sequences (from [`audio_onsets`]): slide the
/// reference's strongest window over the clean file's onsets and read off the
/// best lag. `reference_start` is the take-time of the reference's frame 0.
pub fn align(
    reference_onsets: &[f32],
    reference_start: f64,
    clean_onsets: &[f32],
) -> Option<Alignment> {
    if reference_onsets.len() < MIN_FRAMES || clean_onsets.len() < MIN_FRAMES {
        return None;
    }

    let width = ((TEMPLATE_SECONDS / HOP) as usize)
        .min(reference_onsets.len())
        .min(clean_onsets.len())
        .max(MIN_FRAMES);
    let anchor = strongest_window(reference_onsets, width);
    let template = &reference_onsets[anchor..anchor + width];

    let mean = template.iter().map(|&x| f64::from(x)).sum::<f64>() / width as f64;
    let centered: Vec<f32> = template.iter().map(|&x| (f64::from(x) - mean) as f32).collect();
    let norm = centered.iter().map(|&x| f64::from(x) * f64::from(x)).sum::<f64>().sqrt();
    if norm <= 1e-9 {
        // A silent (or eventless) template — nothing to lock onto.
        return None;
    }

    let (lag, confidence) = best_lag(&centered, norm, clean_onsets);

    // The template frame `anchor` and the clean frame `lag` are the same
    // moment, so their take-times are equal:
    //   reference_start + anchor*HOP  ==  clean_start + lag*HOP
    // which gives the clean file's start on the take's timeline.
    let start = reference_start + (anchor as f64 - lag as f64) * HOP;
    Some(Alignment { start, confidence })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn align(
        reference: &mut Audio,
        reference_start: f64,
        clean: &mut Audio,
    ) -> Result<Option<Alignment>, String> {
        Ok(super::align(&audio_onsets(reference)?, reference_start, &audio_onsets(clean)?))
    }

    fn align_to_notes(
        onsets: &[(f64, f32)],
        span: f64,
        clean: &mut Audio,
    ) -> Result<Option<Alignment>, String> {
        Ok(super::align_to_notes(onsets, span, &audio_onsets(clean)?))
    }

    /// A cheap deterministic pseudo-random stream, so "crackle" is
    /// reproducible without a dependency or the real RNG.
    struct Lcg(u64);
    impl Lcg {
        fn next_unit(&mut self) -> f32 {
            self.0 = self.0.wrapping_mul(6_364_136_223_846_793_005).wrapping_add(1);
            ((self.0 >> 33) as f32 / (1u64 << 31) as f32) - 1.0
        }
    }

    /// A signal of decaying clicks at the given times (seconds), over a
    /// `duration`-second buffer. Clicks are what cross-correlation locks
    /// onto, and they stand in for note attacks.
    fn clicks(times: &[f64], duration: f64, sample_rate: f32) -> Vec<f32> {
        let n = (duration * f64::from(sample_rate)) as usize;
        let mut samples = vec![0.0f32; n];
        let tail = (sample_rate / 40.0) as usize; // 25 ms
        for &t in times {
            let start = (t * f64::from(sample_rate)) as usize;
            for k in 0..tail {
                if start + k < n {
                    samples[start + k] += 0.8 * (1.0 - k as f32 / tail as f32);
                }
            }
        }
        samples
    }

    /// Regular-ish click times across `0..span` seconds.
    fn beat(span: f64) -> Vec<f64> {
        let mut times = Vec::new();
        let mut t = 0.3;
        let mut gap = 0.37;
        while t < span {
            times.push(t);
            t += gap;
            // Vary the spacing so the pattern isn't perfectly periodic
            // (which would make the correlation ambiguous).
            gap = 0.25 + (gap * 1.7) % 0.4;
        }
        times
    }

    /// Build a reference that recorded the same performance as `clean`,
    /// but armed at take-time `reference_start`, at its own sample rate,
    /// quieter, and speckled with crackle. `clean` covers take-time
    /// `[clean_start, clean_start + span]`.
    fn scenario(clean_start: f64, reference_start: f64, span: f64) -> (Audio, Audio) {
        let events = beat(span);
        let clean_rate = 48_000.0;
        // Clean sample 0 is at take-time clean_start; events use bounce-relative time.
        let clean = Audio::from_samples(
            clean_rate,
            clicks(
                &events.iter().map(|&e| e - clean_start).collect::<Vec<_>>(),
                span - clean_start + 1.0,
                clean_rate,
            ),
            1,
        );

        let ref_rate = 44_100.0;
        let ref_events: Vec<f64> =
            events.iter().map(|&e| e - reference_start).filter(|&t| t >= 0.0).collect();
        let mut ref_samples = clicks(&ref_events, span - reference_start + 1.0, ref_rate);
        // Quieter, and crackly: occasional sharp spikes, a few per second
        // as a real dropout would be — not a wall of noise.
        let mut lcg = Lcg(0x1234_5678);
        for sample in ref_samples.iter_mut() {
            *sample *= 0.6;
            if lcg.next_unit().abs() > 0.9997 {
                *sample += 0.7 * lcg.next_unit().signum();
            }
        }
        let reference = Audio::from_samples(ref_rate, ref_samples, 1);
        (reference, clean)
    }

    #[test]
    fn recovers_a_clean_file_that_starts_at_take_zero() {
        // Reference armed 2 s in; clean bounce is the whole song from 0.
        let (mut reference, mut clean) = scenario(0.0, 2.0, 24.0);
        let alignment = align(&mut reference, 2.0, &mut clean).unwrap().expect("enough signal");
        assert!(
            alignment.start.abs() < 0.02,
            "clean starts at ~0, got {:.4}s (confidence {:.2})",
            alignment.start,
            alignment.confidence,
        );
        assert!(alignment.confidence > 0.5, "confidence {:.2}", alignment.confidence);
    }

    #[test]
    fn recovers_a_clean_file_with_a_pre_roll() {
        // The bounce has half a second of count-in before song zero, so
        // its sample 0 is at take-time -0.5.
        let (mut reference, mut clean) = scenario(-0.5, 1.5, 22.0);
        let alignment = align(&mut reference, 1.5, &mut clean).unwrap().expect("enough signal");
        assert!(
            (alignment.start - (-0.5)).abs() < 0.02,
            "expected ~-0.5s, got {:.4}s",
            alignment.start,
        );
    }

    #[test]
    fn stereo_polarity_does_not_change_alignment() {
        let (mut reference, mut clean) = scenario(-0.35, 0.5, 4.0);
        let expected =
            align(&mut reference, 0.5, &mut clean).unwrap().expect("mono reference aligns");
        assert!((expected.start + 0.35).abs() < 0.02);
        let onsets: Vec<_> = beat(4.0).into_iter().map(|t| (t, 0.8)).collect();
        let expected_notes =
            align_to_notes(&onsets, 4.0, &mut clean).unwrap().expect("MIDI reference aligns");
        for sign in [1.0, -1.0] {
            let stereo = |audio: &mut Audio| {
                Audio::from_samples(
                    audio.sample_rate,
                    audio.all_samples().iter().flat_map(|&x| [x, sign * x]).collect(),
                    2,
                )
            };
            let (mut reference, mut clean) = (stereo(&mut reference), stereo(&mut clean));
            let actual =
                align(&mut reference, 0.5, &mut clean).unwrap().expect("stereo attacks survive");
            assert_eq!(actual.start, expected.start);
            assert!((actual.confidence - expected.confidence).abs() < 1e-6);
            let notes =
                align_to_notes(&onsets, 4.0, &mut clean).unwrap().expect("stereo aligns to notes");
            assert_eq!(notes.start, expected_notes.start);
            assert!((notes.confidence - expected_notes.confidence).abs() < 1e-6);
        }
    }

    #[test]
    fn recovers_a_positive_offset_too() {
        // The bounce starts 0.75 s into the song.
        let (mut reference, mut clean) = scenario(0.75, 0.75, 22.0);
        let alignment = align(&mut reference, 0.75, &mut clean).unwrap().expect("enough signal");
        assert!(
            (alignment.start - 0.75).abs() < 0.02,
            "expected ~0.75s, got {:.4}s",
            alignment.start,
        );
    }

    #[test]
    fn crackle_does_not_throw_the_alignment_off() {
        // The confidence should stay high despite the reference being
        // the quiet, speckled one — that is the whole premise.
        let (mut reference, mut clean) = scenario(0.0, 0.0, 26.0);
        let alignment = align(&mut reference, 0.0, &mut clean).unwrap().expect("enough signal");
        assert!(alignment.start.abs() < 0.02, "got {:.4}s", alignment.start);
        assert!(alignment.confidence > 0.5, "confidence {:.2}", alignment.confidence);
    }

    #[test]
    fn too_little_audio_declines_rather_than_guessing() {
        let mut tiny = Audio::from_samples(48_000.0, vec![0.0; 32], 1);
        let mut other = Audio::from_samples(48_000.0, vec![0.0; 32], 1);
        assert!(align(&mut tiny, 0.0, &mut other).unwrap().is_none());
    }

    #[test]
    fn aligns_a_bounce_to_midi_onsets_with_no_reference_recording() {
        // A take with no scratch audio: line the bounce up against the
        // note-ons alone, at a few offsets (starts at zero, mid-song, and with
        // a pre-roll).
        let span = 22.0;
        let events = beat(span);
        let onsets: Vec<(f64, f32)> = events.iter().map(|&t| (t, 0.8)).collect();
        for offset in [0.0, 1.3, -0.4] {
            // The bounce's sample 0 sits at take-time `offset`, so an event at
            // take-time e is at bounce-time e - offset; events before the
            // bounce starts aren't captured.
            let bounce_events: Vec<f64> =
                events.iter().map(|&e| e - offset).filter(|&t| t >= 0.0).collect();
            let mut clean = Audio::from_samples(
                48_000.0,
                clicks(&bounce_events, span - offset + 1.0, 48_000.0),
                1,
            );
            let a = align_to_notes(&onsets, span, &mut clean).unwrap().expect("enough signal");
            assert!(
                (a.start - offset).abs() < 0.03,
                "offset {offset}: got {:.4}s (confidence {:.2})",
                a.start,
                a.confidence,
            );
        }
    }
    #[test]
    fn envelope_boundaries_survive_physical_chunks_and_a_partial_final_bin() {
        let channels = 3;
        let samples: Vec<f32> = (0..70_003 * channels).map(|i| (i as f32 * 0.013).sin()).collect();
        // The high rate makes one 5 ms energy bin larger than the decoder's
        // physical buffer, so this reaches segmentation *inside* a measurement.
        for rate in [44_100.0, 48_000.0, 4_000_000.0] {
            let mut audio = Audio::from_samples(rate, samples.clone(), channels);
            let expected: Vec<f32> = (0..(audio.seconds() / HOP).floor() as usize)
                .map(|k| {
                    let start = (k as f64 * HOP * f64::from(rate)) as usize;
                    let end =
                        (((k + 1) as f64 * HOP * f64::from(rate)) as usize).min(audio.frames());
                    let slice = &samples[start * channels..end * channels];
                    let sum: f64 = slice.iter().map(|&x| f64::from(x) * f64::from(x)).sum();
                    (sum / slice.len() as f64).sqrt() as f32
                })
                .collect();
            assert!(expected.len() >= 3);
            assert_eq!(envelope(&mut audio).unwrap(), expected);
        }
    }
    #[test]
    fn failed_recording_alignment_reuses_the_clean_onsets_for_midi() {
        let (_, mut clean) = scenario(-0.35, 0.5, 4.0);
        let notes: Vec<_> = beat(4.0).into_iter().map(|t| (t, 0.8)).collect();
        let clean_onsets = audio_onsets(&mut clean).unwrap();
        let silent_reference = vec![0.0; clean_onsets.len()];
        assert!(super::align(&silent_reference, 0.5, &clean_onsets).is_none());
        // Both correlations take only envelopes: retrying cannot read any WAV.
        let fallback = super::align_to_notes(&notes, 4.0, &clean_onsets).unwrap();
        let direct = align_to_notes(&notes, 4.0, &mut clean).unwrap().unwrap();
        assert!((fallback.start + 0.35).abs() < 0.02);
        assert_eq!(fallback.start, direct.start);
        assert_eq!(fallback.confidence, direct.confidence);
    }
}
