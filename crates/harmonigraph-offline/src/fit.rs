//! The pass ahead of the frames that Frequency range: Fit piece needs: the
//! soundtrack analyzed once over the render window, and the take's notes, into
//! a [`PitchFit`].

use crate::render::Settings;
use crate::wav::Audio;
use harmonigraph_take::{CanonicalRecord, NoteKind, Take};
use harmonigraph_ui::{PitchFit, SpectrumAnalysis, SpectrumConfig};

/// MIDI per microcent, the unit a take records pitches in.
const MIDI_PER_MICROCENT: f64 = 1e-8;

/// The range Fit piece draws `take` at over `settings`' window, or `None` to
/// keep the dialled one (see [`PitchFit::range`]).
///
/// About 0.9 s at Fast and 2.9 s at Precise for a 98 s take on an M5 Pro
/// (`take-2026-11-19_06-40-06`), against the 30-40 s its 1440p60 export takes.
pub fn pitch_range(
    take: &Take,
    audio: Option<&mut Audio>,
    settings: &Settings,
    config: &SpectrumConfig,
) -> Result<Option<(f32, f32)>, String> {
    let mut fit = PitchFit::new(config);
    if let Some(audio) = audio {
        let (channels, rate) = (audio.channels, audio.sample_rate);
        let range = audio.range_seconds(
            settings.start - settings.audio_start,
            settings.end - settings.audio_start,
        );
        let mut analysis = SpectrumAnalysis::default();
        audio.for_frames(range, |chunk| {
            analysis.feed_source(chunk, channels, rate, 0.0, config, |delivery| {
                if let Some(column) = delivery.column() {
                    fit.column(column);
                }
            })
        })?;
    }
    let microcents = |pitch: i64| (pitch as f64 * MIDI_PER_MICROCENT) as f32;
    for record in &take.events {
        match record {
            CanonicalRecord::Note(note) if matches!(note.kind, NoteKind::On { .. }) => {
                fit.note(f32::from(note.note));
            }
            CanonicalRecord::Delta(delta) => {
                if let Some(pitch) = delta.pitch_microcents {
                    fit.note(microcents(pitch));
                }
            }
            CanonicalRecord::Baseline(baseline) => {
                for voice in &baseline.voices {
                    fit.note(microcents(voice.pitch_microcents));
                }
            }
            CanonicalRecord::Note(_) | CanonicalRecord::Gap(_) => {}
        }
    }
    Ok(fit.range())
}

#[cfg(test)]
mod tests {
    use super::*;
    use harmonigraph_take::canonical::DeltaRecord;
    use harmonigraph_take::NoteRecord;

    /// The window's soundtrack and the take's notes set the range, and sound
    /// outside the window does not: an 880 Hz tone in the window and a note at
    /// MIDI 30 fit it, and a 7 kHz tone in the second before it starts is left
    /// out.
    #[test]
    fn the_fit_reads_the_render_window_and_the_notes() {
        let rate = 48_000.0;
        let tone = |hz: f64, seconds: f64| {
            (0..(seconds * rate) as usize).flat_map(move |frame| {
                let sample =
                    (0.5 * (std::f64::consts::TAU * hz * frame as f64 / rate).sin()) as f32;
                [sample, sample]
            })
        };
        let samples = tone(7_040.0, 1.0).chain(tone(880.0, 2.0)).collect();
        let mut audio = Audio::from_samples(rate as f32, samples, 2);
        let take = Take {
            events: vec![CanonicalRecord::Delta(DeltaRecord {
                event: NoteRecord {
                    t: 1.5,
                    note: 30,
                    kind: NoteKind::On { velocity: 0.8 },
                    ..Default::default()
                },
                pitch_microcents: Some(3_000_000_000),
                ..Default::default()
            })],
            ..Default::default()
        };
        let settings = Settings {
            layout: harmonigraph_ui::Layout::split(harmonigraph_ui::LatticeSide::Left, 0.5),
            size: [64, 36],
            pixels_per_point: 1.0,
            fps: 10.0,
            start: 1.0,
            end: 3.0,
            audio_start: 0.0,
        };
        use harmonigraph_core::spectrum::{SPECTRUM_MAX_MIDI, SPECTRUM_MIN_MIDI};
        let config = SpectrumConfig {
            low_midi: SPECTRUM_MIN_MIDI,
            high_midi: SPECTRUM_MAX_MIDI,
            ..Default::default()
        };

        let (low, high) =
            pitch_range(&take, Some(&mut audio), &settings, &config).unwrap().unwrap();
        assert!(
            (low - 29.0).abs() < 1e-3,
            "the note at 30 holds the low end a semitone under it: {low}"
        );
        // 880 Hz is MIDI 81; its lobe lights a few semitones past it at Fast.
        assert!((82.0..88.0).contains(&high), "the tone holds the high end: {high}");
    }
}
