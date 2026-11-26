//! The pass ahead of the frames that Frequency range: Fit piece needs: the
//! soundtrack analyzed once over the render window, and the take's notes, into
//! a [`PitchFit`].

use crate::render::Settings;
use crate::wav::Audio;
use harmonigraph_core::notes::semitones;
use harmonigraph_take::{CanonicalRecord, NoteKind, Take};
use harmonigraph_ui::{PitchFit, SpectrumAnalysis, SpectrumConfig};

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
        // The stretch the frames analyze, the pre-roll before the first and the
        // read-ahead past the last included, so a band they draw near a
        // `--start` or `--end` is counted as they draw it.
        let (from, to) = settings.soundtrack_window(config, rate);
        let range = audio.range_seconds(from - settings.audio_start, to - settings.audio_start);
        let mut analysis = SpectrumAnalysis::default();
        audio.for_frames(range, |chunk| {
            analysis.feed_source(chunk, channels, rate, 0.0, config, |delivery| {
                if let Some(column) = delivery.column() {
                    fit.column(column);
                }
            })
        })?;
    }
    for record in &take.events {
        match record {
            CanonicalRecord::Note(note) if matches!(note.kind, NoteKind::On { .. }) => {
                fit.note(f32::from(note.note));
            }
            CanonicalRecord::Delta(delta) => {
                if let Some(pitch) = delta.pitch_microcents {
                    fit.note(semitones(pitch));
                }
            }
            CanonicalRecord::Baseline(baseline) => {
                for voice in &baseline.voices {
                    fit.note(semitones(voice.pitch_microcents));
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

    /// The soundtrack the frames analyze and the take's notes set the range:
    /// a note's recorded pitch rather than its key; a 3 kHz tone that ends
    /// 0.15 s into the render, which its first frames draw from the audio
    /// read before them; and not a 7 kHz tone a second before the render, past
    /// that read.
    #[test]
    fn the_fit_reads_what_the_frames_analyze_and_the_notes() {
        let rate = 48_000.0;
        let tone = |hz: f64, from: f64, to: f64| {
            move |t: f64| (from..to).contains(&t).then(|| (std::f64::consts::TAU * hz * t).sin())
        };
        let tones = [tone(7_040.0, 0.0, 1.0), tone(3_000.0, 1.5, 2.15), tone(880.0, 2.2, 4.0)];
        let samples = (0..(4.0 * rate) as usize)
            .flat_map(|frame| {
                let t = frame as f64 / rate;
                let sample = 0.5 * tones.iter().filter_map(|tone| tone(t)).sum::<f64>();
                [sample as f32; 2]
            })
            .collect();
        let mut audio = Audio::from_samples(rate as f32, samples, 2);
        let take = Take {
            events: vec![CanonicalRecord::Delta(DeltaRecord {
                event: NoteRecord {
                    t: 2.5,
                    note: 50,
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
            start: 2.0,
            end: 4.0,
            audio_start: 0.0,
        };
        // Precise, whose half-second analysis frame is the read before the
        // first frame.
        let config = SpectrumConfig {
            window: harmonigraph_ui::SpectrumWindow::Precise,
            volume_floor_db: -68.0,
            volume_ceiling_db: -35.0,
            tilt: -4.5,
            ..Default::default()
        };

        let (low, high) =
            pitch_range(&take, Some(&mut audio), &settings, &config).unwrap().unwrap();
        assert!((low - 29.0).abs() < 1e-3, "the pitch 30 holds the low end under it: {low}");
        // 3 kHz is MIDI 102.4, and 7 kHz 117.3.
        assert!((103.0..110.0).contains(&high), "the 3 kHz tone holds the high end: {high}");
    }
}
