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

/// RMS energy per [`HOP`]-length frame.
///
/// Frame boundaries are placed by *time*, not by a fixed sample count, so
/// frame `k` covers exactly `[k*HOP, (k+1)*HOP)` seconds whatever the rate
/// — 44.1 kHz rounds to 220 samples a frame, 48 kHz to 240, and a fixed
/// count would let their time grids drift apart over a few seconds and
/// smear the correlation. Placing boundaries by time keeps both files on
/// one grid, which is what lets a reference and a bounce at different
/// rates line up at all.
fn envelope(samples: &[f32], sample_rate: f32) -> Vec<f32> {
    let sr = f64::from(sample_rate);
    let frames = ((samples.len() as f64 / sr) / HOP).floor() as usize;
    (0..frames)
        .map(|k| {
            let start = (k as f64 * HOP * sr) as usize;
            let end = (((k + 1) as f64 * HOP * sr) as usize).min(samples.len());
            let frame = &samples[start..end.max(start)];
            if frame.is_empty() {
                return 0.0;
            }
            let sum_sq: f64 = frame.iter().map(|&x| f64::from(x) * f64::from(x)).sum();
            (sum_sq / frame.len() as f64).sqrt() as f32
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

/// Find where `clean` sits on the take's timeline by matching it against
/// `reference`, the take's own recording (which starts at take-time
/// `reference_start`). Returns `None` when either file is too short to
/// correlate.
pub fn align(reference: &Audio, reference_start: f64, clean: &Audio) -> Option<Alignment> {
    let reference_onsets = onset_strength(&envelope(&reference.mono(), reference.sample_rate));
    let clean_onsets = onset_strength(&envelope(&clean.mono(), clean.sample_rate));
    align_onsets(&reference_onsets, reference_start, &clean_onsets)
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
pub fn align_to_notes(onsets: &[(f64, f32)], span: f64, clean: &Audio) -> Option<Alignment> {
    let reference_onsets = note_onset_envelope(onsets, span);
    let clean_onsets = onset_strength(&envelope(&clean.mono(), clean.sample_rate));
    // The note train is on the take clock, so its frame 0 is take-time 0.
    align_onsets(&reference_onsets, 0.0, &clean_onsets)
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

/// The correlation core shared by [`align`] and [`align_to_notes`]: slide the
/// reference's strongest window over the clean file's onsets and read off the
/// best lag. `reference_start` is the take-time of the reference's frame 0.
fn align_onsets(
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
        let clean = Audio {
            sample_rate: clean_rate,
            // clean sample 0 is at take-time clean_start, so an event at
            // take-time e sits at clean-relative time e - clean_start.
            samples: clicks(
                &events.iter().map(|&e| e - clean_start).collect::<Vec<_>>(),
                span - clean_start + 1.0,
                clean_rate,
            ),
            channels: 1,
        };

        let ref_rate = 44_100.0;
        let ref_events: Vec<f64> = events
            .iter()
            .map(|&e| e - reference_start)
            .filter(|&t| t >= 0.0)
            .collect();
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
        let reference = Audio { sample_rate: ref_rate, samples: ref_samples, channels: 1 };
        (reference, clean)
    }

    #[test]
    fn recovers_a_clean_file_that_starts_at_take_zero() {
        // Reference armed 2 s in; clean bounce is the whole song from 0.
        let (reference, clean) = scenario(0.0, 2.0, 24.0);
        let alignment = align(&reference, 2.0, &clean).expect("enough signal");
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
        let (reference, clean) = scenario(-0.5, 1.5, 22.0);
        let alignment = align(&reference, 1.5, &clean).expect("enough signal");
        assert!(
            (alignment.start - (-0.5)).abs() < 0.02,
            "expected ~-0.5s, got {:.4}s",
            alignment.start,
        );
    }

    #[test]
    fn recovers_a_positive_offset_too() {
        // The bounce starts 0.75 s into the song.
        let (reference, clean) = scenario(0.75, 0.75, 22.0);
        let alignment = align(&reference, 0.75, &clean).expect("enough signal");
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
        let (reference, clean) = scenario(0.0, 0.0, 26.0);
        let alignment = align(&reference, 0.0, &clean).expect("enough signal");
        assert!(alignment.start.abs() < 0.02, "got {:.4}s", alignment.start);
        assert!(alignment.confidence > 0.5, "confidence {:.2}", alignment.confidence);
    }

    #[test]
    fn too_little_audio_declines_rather_than_guessing() {
        let tiny = Audio { sample_rate: 48_000.0, samples: vec![0.0; 32], channels: 1 };
        assert!(align(&tiny, 0.0, &tiny).is_none());
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
            let clean = Audio {
                sample_rate: 48_000.0,
                samples: clicks(&bounce_events, span - offset + 1.0, 48_000.0),
                channels: 1,
            };
            let a = align_to_notes(&onsets, span, &clean).expect("enough signal");
            assert!(
                (a.start - offset).abs() < 0.03,
                "offset {offset}: got {:.4}s (confidence {:.2})",
                a.start,
                a.confidence,
            );
        }
    }
}
