//! The shared log-frequency pitch axis and Hz/MIDI conversions.
//! Analysis lives in `harmonigraph-analysis`; history and drawing need only this axis.

/// The spectrum's pitch axis: MIDI notes [MIN, MAX), which is 20 Hz to
/// 20 kHz — the audible band, as every analyzer states it. The axis is linear
/// in MIDI pitch, i.e. logarithmic in frequency, so every octave gets equal
/// width.
///
/// Deliberately NOT whole octaves from a C. MIDI 12..132 — ten octaves C to
/// C — would land the C labels on the axis ends, which is tidy, but it
/// stops at 16.7 kHz and leaves the top third of an octave of the audible
/// band unanalyzed. There is no C anywhere near 20 kHz (the next one is
/// 44 kHz), so covering the band means giving that tidiness up.
pub const SPECTRUM_MIN_MIDI: f32 = 15.486_82; // 20 Hz
pub const SPECTRUM_MAX_MIDI: f32 = 135.076_23; // 20 kHz
/// Axis resolution: 32 buckets per semitone (3.125 cents).
///
/// The grid the magnitude spectrum is resampled onto. It is finer than the
/// FFT resolves anywhere below ~3.2 kHz, which is deliberate: the extra rows
/// cost almost nothing and they let the axis be zoomed right in without the
/// grid itself becoming the thing you see. What the FFT can actually
/// distinguish is set by the window length, not by this.
pub const BINS_PER_SEMITONE: usize = 32;
/// Enough buckets to cover the axis, plus slack: the span is not a whole
/// number of semitones, and `pitch_spectrum` writes to `b0 + 1`, so the top
/// partial needs a bucket above the one it lands in or it would be dropped.
pub const SPECTRUM_BINS: usize =
    ((SPECTRUM_MAX_MIDI - SPECTRUM_MIN_MIDI) * BINS_PER_SEMITONE as f32) as usize + 2;

/// Frequency of a (fractional) MIDI pitch at A440.
pub fn midi_to_hz(midi: f32) -> f32 {
    440.0 * ((midi - 69.0) / 12.0).exp2()
}

/// The inverse of [`midi_to_hz`]: the (fractional) MIDI pitch of `hz`.
pub fn hz_to_midi(hz: f32) -> f32 {
    69.0 + 12.0 * (hz / 440.0).log2()
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn midi_to_hz_anchors_a440_and_doubles_each_octave() {
        assert!((midi_to_hz(69.0) - 440.0).abs() < 1e-2, "A4 = 440 Hz");
        assert!((midi_to_hz(57.0) - 220.0).abs() < 1e-2, "A3 = 220 Hz");
        assert!((midi_to_hz(81.0) - 880.0).abs() < 1e-2, "A5 = 880 Hz");
        assert!((midi_to_hz(60.0) - 261.6256).abs() < 0.1, "middle C ≈ 261.63 Hz");
    }
}
