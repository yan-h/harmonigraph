//! The pass ahead of the frames that Frequency range: Fit piece needs: the
//! soundtrack the frames analyze, analyzed once, and the notes they draw, into
//! a [`PitchFit`].

use crate::render::Settings;
use crate::wav::Audio;
use harmonigraph_core::NoteTracker;
use harmonigraph_take::Take;
use harmonigraph_ui::{PitchFit, PitchRangeRender, SpectrumAnalysis, SpectrumConfig};

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
    // The notes the frames draw: each shown voice's pitch, bends included,
    // through the tracker the replay draws with, sampled at every record up to
    // the render's end. From the take's start rather than the render's, since
    // the first frame's roll draws what was released before it (Yan, #1976);
    // a released voice was counted while it was held.
    let mut tracker = NoteTracker::new();
    for record in take.events.iter().take_while(|record| record.time() <= settings.end) {
        record.apply(&mut tracker).expect("validated canonical take");
        for voice in tracker.held_shown() {
            fit.note(voice.pitch);
        }
    }
    Ok(fit.range())
}

/// Frequency range: Fit piece, set on `config` once before the first frame as
/// Video history's Fit video is, and named, since the preview showed the range
/// as dialled. Dialled leaves `config` as it is.
pub fn apply(
    mode: PitchRangeRender,
    take: &Take,
    audio: Option<&mut Audio>,
    settings: &Settings,
    config: &mut SpectrumConfig,
) -> Result<(), String> {
    if mode != PitchRangeRender::Fit {
        return Ok(());
    }
    let readout = harmonigraph_ui::hz_readout;
    match pitch_range(take, audio, settings, config)? {
        Some((low, high)) => {
            eprintln!(
                "frequency range fitted to the piece: {}-{} (dialled {}-{})",
                readout(low),
                readout(high),
                readout(config.low_midi),
                readout(config.high_midi),
            );
            (config.low_midi, config.high_midi) = (low, high);
        }
        None => eprintln!("frequency range: nothing lit or played to fit, so drawn as dialled"),
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use harmonigraph_take::canonical::{BaselineRecord, DeltaRecord};
    use harmonigraph_take::{CanonicalRecord, NoteKind, NoteRecord};

    /// The soundtrack the frames analyze and the notes they draw set the
    /// range: a 3 kHz tone that ends 0.15 s into the render, which its first
    /// frames draw from the audio read before them, but not a 7 kHz tone a
    /// second before the render, past that read; and a note's recorded pitch
    /// rather than its key and one before the render, but not one after it or
    /// one while its source is hidden.
    #[test]
    fn the_fit_reads_what_the_frames_analyze_and_draw() {
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
        let note = |t: f64, note: u8, on: bool| {
            let kind = if on { NoteKind::On { velocity: 0.8 } } else { NoteKind::Off };
            NoteRecord { t, note, kind, ..Default::default() }
        };
        let shown = |t: f64, shown: bool| {
            CanonicalRecord::Baseline(Box::new(BaselineRecord { t, shown, ..Default::default() }))
        };
        let take = Take {
            events: vec![
                shown(2.1, false),
                CanonicalRecord::Note(note(2.2, 27, true)),
                CanonicalRecord::Note(note(2.3, 27, false)),
                shown(2.4, true),
                CanonicalRecord::Delta(DeltaRecord {
                    event: note(2.5, 50, true),
                    pitch_microcents: Some(3_000_000_000),
                    ..Default::default()
                }),
                CanonicalRecord::Note(note(4.5, 22, true)),
            ],
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

        // A note released before the render, which its first frames draw in
        // their history.
        let mut earlier = take.clone();
        earlier.events.splice(
            0..0,
            [
                CanonicalRecord::Note(note(0.2, 25, true)),
                CanonicalRecord::Note(note(0.3, 25, false)),
            ],
        );
        let (low, _) =
            pitch_range(&earlier, Some(&mut audio), &settings, &config).unwrap().unwrap();
        assert!((low - 24.0).abs() < 1e-3, "the note at 25 holds the low end under it: {low}");
    }

    /// Fit piece sets the range it fits; Dialled draws the range as dialled.
    #[test]
    fn only_fit_piece_replaces_the_dialled_range() {
        let take = Take {
            events: vec![CanonicalRecord::Note(NoteRecord {
                t: 2.5,
                note: 60,
                kind: NoteKind::On { velocity: 0.8 },
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
        let dialled = SpectrumConfig { low_midi: 12.0, high_midi: 120.0, ..Default::default() };
        let fitted = pitch_range(&take, None, &settings, &dialled).unwrap().unwrap();
        assert_ne!(fitted, (dialled.low_midi, dialled.high_midi), "a fit that moves the range");
        for (mode, range) in [
            (PitchRangeRender::Dialled, (dialled.low_midi, dialled.high_midi)),
            (PitchRangeRender::Fit, fitted),
        ] {
            let mut config = dialled.clone();
            apply(mode, &take, None, &settings, &mut config).unwrap();
            assert_eq!((config.low_midi, config.high_midi), range, "{mode:?}");
        }
    }
}
