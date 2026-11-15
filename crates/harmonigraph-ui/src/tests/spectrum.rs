//! The analyzer and its spectrogram history: what is stored, when it is
//! stamped, and how stereo partials survive analysis.

use super::probe::fresh;
use crate::*;

#[test]
fn display_recovers_after_nonfinite_audio() {
    let mut spectrum = AudioSpectrum::default();
    let config = SpectrumConfig {
        window: SpectrumWindow::Fast,
        attack: 0.0,
        release: 0.0,
        ..SpectrumConfig::default()
    };
    let n = config.window.frame(48_000.0);
    let mut poisoned = vec![0.0; n + 384];
    poisoned[n / 2] = f32::NAN;
    let mut feed = crate::spectrum::TestFeed::default();
    let poisoned_columns = feed.feed(&mut spectrum, &poisoned, 1, 48_000.0, 1.0, &config);
    assert!(spectrum.display(1.0).unwrap().iter().all(|p| p.is_finite()));

    // Replace the entire FFT window and leave time for a complete clean hop.
    // Zero smoothing rules out a long release being mistaken for failed recovery.
    let clean: Vec<f32> = (0..2 * n)
        .map(|i| 0.5 * (std::f32::consts::TAU * 440.0 * i as f32 / 48_000.0).sin())
        .collect();
    let now = 1.0 + clean.len() as f64 / 48_000.0;
    let columns = feed.feed(&mut spectrum, &clean, 1, 48_000.0, now, &config);
    let last = columns.last().expect("clean input emitted columns");
    assert!(last.power().iter().all(|p| p.is_finite()));
    assert!(last.power().iter().any(|p| *p > 0.01), "latest FFT measures the clean tone");
    let shown = spectrum.display(now).expect("clean input is flowing");
    assert!(shown.iter().all(|p| p.is_finite()));
    assert!(shown.iter().any(|p| *p > 0.01), "the curve recovers, not just the history");

    // And the history: both batches share the coarsest level's open slab,
    // which the poisoned window's columns did not poison.
    assert!(!poisoned_columns.is_empty(), "the poisoned window made no column");
    let coarsest = crate::spectrogram::LEVELS - 1;
    let pyramid = spectrum.pyramid();
    let both = (poisoned_columns.len() + columns.len()) as u64;
    assert_eq!(pyramid.open_count(coarsest), both, "the two batches' columns share the slab");
    let (_, slabs) = pyramid.level(coarsest);
    let a4 = ((69.0 - harmonigraph_core::spectrum::SPECTRUM_MIN_MIDI)
        * harmonigraph_core::spectrum::BINS_PER_SEMITONE as f32) as usize;
    assert!(slabs.last().unwrap()[a4] > 0, "the open slab lost the clean tone to the NaN");
}

#[test]
fn audio_spectrum_shows_while_flowing_and_hides_after() {
    // Each window reads A4 at its own pitch, to within a bucket, the Fast one
    // included: reading the transform's bins drew it about 21¢ (seven buckets)
    // sharp through the old Fast window (#1575).
    for window in [SpectrumWindow::Fast, SpectrumWindow::Balanced] {
        let buckets = 1;
        let mut spectrum = AudioSpectrum::default();
        let config = SpectrumConfig { window, ..SpectrumConfig::default() };
        assert!(spectrum.display(0.0).is_none(), "no audio yet");

        // A 440 Hz sine, long enough to fill the analysis window.
        let sine: Vec<f32> = (0..window.frame(48_000.0) + 1000)
            .map(|i| 0.5 * (std::f32::consts::TAU * 440.0 * i as f32 / 48_000.0).sin())
            .collect();
        crate::spectrum::heard_at(&mut spectrum, &sine, 1, 48_000.0, 1.0, &config);
        let levels = spectrum.display(1.0).expect("audio is flowing");
        let peak = levels
            .iter()
            .enumerate()
            .max_by(|a, b| a.1.total_cmp(b.1))
            .map(|(i, _)| i as i32)
            .unwrap();
        // A4 is MIDI 69; its bucket scales with the axis resolution.
        let a4 = ((69.0 - harmonigraph_core::spectrum::SPECTRUM_MIN_MIDI)
            * harmonigraph_core::spectrum::BINS_PER_SEMITONE as f32) as i32;
        assert!(
            (peak - a4).abs() <= buckets,
            "{window:?}: 440 Hz should peak at A4 (bucket {a4}), got {peak}"
        );

        // Once samples stop, the curve hides instead of freezing.
        assert!(spectrum.display(1.0 + AudioSpectrum::HOLD_SECONDS + 0.1).is_none());
    }
}

/// Quiet music fills most of the analyzer's height, rather than disappearing
/// into its lower half.
///
/// Nothing musical puts its whole level in ONE bucket: a chord splits its
/// power across its partials, and the default tilt takes another cut below the
/// 1 kHz pivot. The captured ceiling spends the pane on that profile rather
/// than reserving most of it for a full-scale test tone.
///
/// So the defaults are held to a quiet chord rather than to a test tone. The
/// lower bound catches the ceiling drifting back up; the upper bound catches
/// the other failure, where a curve clipped flat against the top has lost the
/// shape of its own peaks.
#[test]
fn a_chord_fills_most_of_the_analyzers_height() {
    let sr = 48_000.0;
    let cfg = SpectrumConfig::default();
    // Six partials sharing the headroom, peaking about -20 dBFS — a quiet mix,
    // not a tone. Half a second, so the smoothing has long settled.
    let samples: Vec<f32> = (0..24_000)
        .map(|i| {
            let t = i as f32 / sr;
            let mix: f32 = [220.0, 277.2, 329.6, 440.0, 554.4, 659.3]
                .iter()
                .map(|f| (std::f32::consts::TAU * f * t).sin())
                .sum();
            0.1 * mix / 6.0_f32.sqrt()
        })
        .collect();
    let mut spectrum = AudioSpectrum::default();
    crate::spectrum::heard_at(&mut spectrum, &samples, 1, sr, 1.0, &cfg);
    let levels = spectrum.display(1.0).expect("audio is flowing");

    // The drawn height of the tallest bucket, through the same mapping the
    // curve is painted with — bucket index back to MIDI, since the tilt is a
    // function of pitch.
    let peak = levels
        .iter()
        .enumerate()
        .map(|(i, &power)| {
            let midi = harmonigraph_core::spectrum::SPECTRUM_MIN_MIDI
                + i as f32 / harmonigraph_core::spectrum::BINS_PER_SEMITONE as f32;
            crate::panes::spectral::axes::loudness(&cfg, power, midi)
        })
        .fold(0.0_f32, f32::max);

    assert!(peak > 0.75, "the curve only reaches {peak:.2} of the pane; the top is empty");
    assert!(peak < 0.99, "the curve is clipped flat against the ceiling at {peak:.2}");
}

/// The Attack is the coefficient a RISING bucket takes and the Release the one
/// a falling bucket takes, and swapping the pair swaps the picture.
///
/// One tone and one silence, run twice with the two times exchanged, because
/// each setting on its own is answerable by the other: a curve that climbed
/// fast proves nothing about which bar did it until the same audio through the
/// swapped pair climbs slowly. What it holds is the branch itself — with one
/// coefficient for both directions, or with the comparison inverted, this is
/// the picture that comes out wrong.
///
/// Every other test here leaves the two equal or pushes seconds of steady tone,
/// where both coefficients converge to the same answer and the branch is
/// invisible.
#[test]
fn a_bucket_rises_on_the_attack_and_falls_on_the_release() {
    let sr = 48_000.0;
    // The window fills before a single column comes out, so each phase is that
    // much audio plus ten hops — enough for a 10 ms time to land and for a 1 s
    // one to have moved a few percent.
    let window = SpectrumConfig::default().window.frame(48_000.0);
    let phase = window + 10 * (AudioSpectrum::FFT_INTERVAL * f64::from(sr)) as usize;
    let tone: Vec<f32> =
        (0..phase).map(|i| 0.5 * (std::f32::consts::TAU * 440.0 * i as f32 / sr).sin()).collect();
    let silence = vec![0.0; phase];
    let a4 = ((69.0 - harmonigraph_core::spectrum::SPECTRUM_MIN_MIDI)
        * harmonigraph_core::spectrum::BINS_PER_SEMITONE as f32) as usize;

    // What A4 reads after the tone, and what is left of it after the silence.
    let run = |attack: f32, release: f32| {
        let cfg = SpectrumConfig { attack, release, ..SpectrumConfig::default() };
        let mut spectrum = AudioSpectrum::default();
        let mut feed = crate::spectrum::TestFeed::default();
        feed.feed(&mut spectrum, &tone, 1, sr, 1.0, &cfg);
        let lit = spectrum.display(1.0).expect("audio is flowing")[a4];
        let later = 1.0 + silence.len() as f64 / f64::from(sr);
        feed.feed(&mut spectrum, &silence, 1, sr, later, &cfg);
        let left = spectrum.display(later).expect("audio is still flowing")[a4];
        (lit, left)
    };
    let (quick_lit, slow_left) = run(0.010, 1.0);
    let (slow_lit, quick_left) = run(1.0, 0.010);

    assert!(
        quick_lit > slow_lit * 5.0,
        "a 10 ms Attack reached {quick_lit:e} where a 1 s Attack reached {slow_lit:e}; \
         the rising bucket is not on the Attack",
    );
    assert!(
        slow_left > quick_left * 5.0,
        "a 1 s Release left {slow_left:e} where a 10 ms Release left {quick_left:e}; \
         the falling bucket is not on the Release",
    );
}

/// The live path stamps its columns the same way the offline one does, and
/// the near-edge grace knows about the lag that creates. Two halves of one
/// thing: a column a lag old is not a stale column, it is the newest there
/// can be, and the heatmap must still reach the now-line.
#[test]
fn a_live_column_is_stamped_at_the_centre_of_its_window() {
    let mut spectrum = AudioSpectrum::default();
    let config = SpectrumConfig::default();
    // A WHOLE number of hops, so the last column's window ends exactly at `now`
    // and the stamp can be checked exactly. A spectrum is taken every
    // FFT_INTERVAL of audio (see `feed_source`), so a batch ending mid-hop
    // leaves its newest column up to one hop further back than this — correctly,
    // since that is where the window it measured ends.
    let hop = (AudioSpectrum::FFT_INTERVAL * 48_000.0).round() as usize;
    let samples = hop * (config.window.frame(48_000.0) / hop + 1); // enough to fill the frame
    let sine: Vec<f32> = (0..samples)
        .map(|i| 0.5 * (std::f32::consts::TAU * 440.0 * i as f32 / 48_000.0).sin())
        .collect();
    let now = 5.0;
    crate::spectrum::heard_at(&mut spectrum, &sine, 1, 48_000.0, now, &config);
    spectrum.display(now).expect("audio is flowing");

    // Where the window's weight is centred: about a third of the Fast frame
    // back, since its listen sits toward the newest end.
    let lag = config.column_lag(48_000.0);
    let frame = f64::from(config.window.frame(48_000.0) as u32) / 48_000.0;
    assert!((0.3..0.4).contains(&(lag / frame)), "a {lag} s lag on a {frame} s frame");
    let stamped = spectrum.newest_column().expect("a column was kept");
    assert!(
        (stamped - (now - lag)).abs() < 1e-9,
        "a column measured over [{:.3}, {now}] was stamped {stamped}, not {lag} s back",
        now - frame,
    );
    // The pane allows for this lag when extending fresh data to the now-line.
    assert!((spectrum.column_lag() - lag).abs() < 1e-9);
}

#[test]
fn changing_the_estimator_preserves_history_and_the_source_hop_grid() {
    let mut spectrum = AudioSpectrum::default();
    let mut analysis = SpectrumAnalysis::default();
    let sample_rate = 48_000.0;
    let origin = 5.0;
    let feed =
        |spectrum: &mut AudioSpectrum, analysis: &mut SpectrumAnalysis, samples: &[f32], config| {
            analysis.feed_source(samples, 1, sample_rate, origin, config, |delivery| {
                spectrum.apply(delivery, config)
            });
        };
    let config = SpectrumConfig::default();
    let initial = 9007; // Filled, and deliberately between hop boundaries.
    feed(&mut spectrum, &mut analysis, &vec![0.2; initial], &config);
    let columns = spectrum.columns_taken();
    assert!(columns > 0);
    let last = spectrum.newest_column().unwrap();

    // A longer frame carries as much of the shorter one's window as it holds
    // and waits only for the rest of itself (#1584).
    let carried = initial.min(config.window.frame(48_000.0));
    let config = SpectrumConfig { window: SpectrumWindow::Balanced, ..config };
    let hop = (AudioSpectrum::FFT_INTERVAL * f64::from(sample_rate)).round() as usize;
    let first_ready = (initial + config.window.frame(48_000.0) - carried).div_ceil(hop) * hop;
    let rest = vec![0.2; first_ready - initial - 1];
    feed(&mut spectrum, &mut analysis, &rest, &config);
    assert_eq!(spectrum.columns_taken(), columns);
    assert_eq!(spectrum.newest_column(), Some(last));
    feed(&mut spectrum, &mut analysis, &[0.2], &config);
    assert_eq!(spectrum.columns_taken(), columns + 1);
    let expected =
        origin + (first_ready - 1) as f64 / f64::from(sample_rate) - config.column_lag(sample_rate);
    assert!((spectrum.newest_column().unwrap() - expected).abs() < 1e-9);
}

/// The live pane and the offline render must analyze stereo IDENTICALLY, or a
/// video would differ from the look it was dialed in against — and only for
/// stereo-wide material, which is the hardest kind of difference to attribute to
/// its cause. They share `ChannelBank` so that this holds by construction; this
/// is what says the sharing actually reaches both paths.
///
/// The signal is deliberately one a mono mixdown would mangle: an anti-phase A4
/// (erased entirely by a sum) under an in-phase E5. If either path mixed down,
/// its columns would be missing a partial the other one has.
#[test]
fn stereo_analysis_preserves_an_antiphase_partial() {
    use harmonigraph_core::spectrum::midi_to_hz;
    let sr = 48_000.0f32;
    let frames = 48_000usize; // one second
    let (a4, e5) = (midi_to_hz(69.0), midi_to_hz(76.0));
    let samples: Vec<f32> = (0..frames)
        .flat_map(|i| {
            let t = i as f32 / sr;
            let anti = 0.6 * (std::f32::consts::TAU * a4 * t).sin();
            let both = 0.3 * (std::f32::consts::TAU * e5 * t).sin();
            [both + anti, both - anti]
        })
        .collect();
    let cfg = SpectrumConfig::default();
    let span = f64::from(frames as u32) / f64::from(sr);

    // Live: one batch, dated so the newest frame sits at the end of the second.
    let mut spectrum = AudioSpectrum::default();
    let live = crate::spectrum::heard_at(&mut spectrum, &samples, 2, sr, span, &cfg);

    assert!(live.len() > 50, "only {} live columns for a second of audio", live.len());
    let bucket_of = |hz: f32| {
        ((harmonigraph_core::spectrum::hz_to_midi(hz)
            - harmonigraph_core::spectrum::SPECTRUM_MIN_MIDI)
            * harmonigraph_core::spectrum::BINS_PER_SEMITONE as f32)
            .round() as usize
    };
    let loudest = |bucket: usize| {
        live.iter()
            .flat_map(|c| c.power()[bucket.saturating_sub(1)..=bucket + 1].iter().copied())
            .map(harmonigraph_core::spectrogram::quantize)
            .max()
            .unwrap_or(0)
    };
    assert!(
        loudest(bucket_of(a4)) > loudest(bucket_of(e5)) / 2,
        "the anti-phase A4 is missing: {} against E5's {}",
        loudest(bucket_of(a4)),
        loudest(bucket_of(e5)),
    );
}

/// The heatmap's color level is independent of the curve's height level. A
/// bucket can therefore sit at a different fraction of the color ramp while
/// the analyzer's geometry stays fixed, and moving the volume-color window
/// changes only that color fraction.
///
/// The tolerance is the store's, not the mapping's: `bin_level` reads a bucket
/// quantized to a byte of dB, so the two agree to within half a step of that
/// grid. `quantizing_a_bucket_does_not_move_its_colour` is where the step
/// itself is held.
#[test]
fn the_heatmap_reads_its_own_color_level_scale() {
    use crate::panes::spectral::axes::spectrogram_level_db;
    let mut cfg = SpectrumConfig::default();
    let midi = 60.0;
    let check = |cfg: &SpectrumConfig, power: f32| {
        let tolerance = 0.5 * harmonigraph_core::spectrogram::DB_STEP
            / (cfg.volume_ceiling_db - cfg.volume_floor_db)
            + 1e-6;
        let curve = spectrogram_level_db(cfg, 10.0 * power.max(1e-12).log10(), midi);
        let heatmap = crate::spectrogram::bin_level_for_test(
            cfg,
            harmonigraph_core::spectrogram::quantize(power),
            midi,
        );
        assert!(
            (heatmap - curve).abs() <= tolerance,
            "power {power}: the curve reads {curve}, the heatmap {heatmap}",
        );
    };

    for power in [0.0, 1e-8, 1e-4, 1e-2, 1.0, 1e9] {
        check(&cfg, power);
    }
    // The color window moves independently of the analyzer's Level window.
    cfg.volume_floor_db = -20.0;
    cfg.volume_ceiling_db = 0.0;
    check(&cfg, 1e-4);
    cfg.volume_floor_db = -90.0;
    cfg.volume_ceiling_db = -30.0;
    check(&cfg, 1e-6);
    // The tilt is the one input that makes the mapping pitch-dependent, so the
    // two have to track each other across pitch as well as across level.
    cfg.tilt = -6.0;
    for midi in [30.0f32, 60.0, 120.0] {
        let tolerance = 0.5 * harmonigraph_core::spectrogram::DB_STEP
            / (cfg.volume_ceiling_db - cfg.volume_floor_db)
            + 1e-6;
        let curve = spectrogram_level_db(&cfg, 10.0 * (1e-5f32).log10(), midi);
        let heatmap = crate::spectrogram::bin_level_for_test(
            &cfg,
            harmonigraph_core::spectrogram::quantize(1e-5),
            midi,
        );
        assert!((heatmap - curve).abs() <= tolerance, "MIDI {midi}: {curve} vs {heatmap}");
    }
}

/// "Clear everything" empties all four accumulations, not three of them.
///
/// Each is filled by a different path — the trail only once a released voice
/// has faded past `prune`, the roll on the note-off, the spectrogram from
/// analyzed audio, the glow from a lit surface's own step — so the
/// interesting failure is one of the four quietly not being wired up.
/// Asserting they are non-empty FIRST is what makes this a test of the clear
/// rather than of four things that were already empty.
#[test]
fn clearing_everything_empties_all_four_accumulations() {
    let mut state = fresh();

    // A note played and released; `prune` past its fade turns the released
    // voice into trail history and leaves the roll's record of it.
    state.picture.runtime.tracker.handle_event(harmonigraph_core::NoteEvent::on(
        0.0,
        harmonigraph_core::SourceId::DIRECT,
        0,
        60,
        1.0,
    ));
    state.picture.runtime.tracker.handle_event(harmonigraph_core::NoteEvent::off(
        0.5,
        harmonigraph_core::SourceId::DIRECT,
        0,
        60,
    ));
    let env = harmonigraph_core::Envelope { fade_time: 0.1, ..Default::default() };
    state.picture.runtime.tracker.prune(600.0, &env);
    // One analyzed column is enough; how audio becomes columns is the
    // spectrogram's own business and tested there.
    state
        .picture
        .runtime
        .spectrum
        .push_history(0.0, &[1.0; harmonigraph_core::spectrum::SPECTRUM_BINS]);
    // A surface's glow state, standing in for whatever a lit lattice pane
    // be carrying by now — how it gets there is `glow_fade`'s
    // own business and tested there.
    state.picture.surfaces.glow_fade.insert(0, Default::default());

    assert!(!state.picture.runtime.tracker.history().is_empty(), "no trail to clear");
    assert!(!state.picture.runtime.tracker.roll().is_empty(), "no roll to clear");
    assert!(state.picture.runtime.spectrum.newest_column().is_some(), "no spectrogram to clear");
    assert!(!state.picture.surfaces.glow_fade.is_empty(), "no glow to clear");

    state.picture.clear_accumulated();

    assert!(state.picture.runtime.tracker.history().is_empty(), "the lattice trail survived");
    assert!(state.picture.runtime.tracker.roll().is_empty(), "the piano roll survived");
    assert!(state.picture.runtime.spectrum.newest_column().is_none(), "the spectrogram survived");
    assert!(state.picture.surfaces.glow_fade.is_empty(), "the lattice glow survived");
}

/// Two placements of one pane in a layout fold a grid each, keyed on the
/// placement's index.
///
/// The slab width a fold settles on comes off the pane's own depth in points,
/// so at UNEQUAL rects the two placements ask for runs of different lengths —
/// which is what makes one surface between them destructive rather than merely
/// shared: the second placement's fold replaces the first's every frame, and
/// the first then draws a run cut for a rect it does not cover. At equal rects
/// both folds agree and the picture comes out right by accident.
///
/// Each reading is calibrated against the same pane drawn alone, so what it
/// pins is that a placement's grid is the one its own rect asks for rather than
/// some number this test would have to restate.
#[test]
fn two_placements_of_one_pane_fold_a_grid_each() {
    // The issue's own split of the frame, 0.0-0.3 and 0.3-1.0.
    let screen = egui::vec2(500.0, 500.0);
    let narrow = egui::Rect::from_min_max(egui::pos2(0.0, 0.0), egui::pos2(150.0, 500.0));
    let wide = egui::Rect::from_min_max(egui::pos2(150.0, 0.0), egui::pos2(500.0, 500.0));

    let seeded = || {
        let mut state = fresh();
        // Time across the screen, which is the axis the two rects differ on:
        // turned upright, both would be the frame's full height deep and fold
        // one run between them.
        state.picture.appearance.spectrum.orientation = crate::SpectralOrientation::Left;
        state.picture.appearance.spectrum.show_spectrogram = true;
        state.picture.appearance.spectrum.roll_seconds = 10.0;
        let mut bins = [0.0f32; harmonigraph_core::spectrum::SPECTRUM_BINS];
        bins[harmonigraph_core::spectrum::SPECTRUM_BINS / 2] = 1.0;
        for i in 0..80 {
            state.picture.runtime.spectrum.push_history(90.0 + f64::from(i) * 0.125, &bins);
        }
        state
    };
    let draw = |state: &mut SharedState, placements: &[(usize, egui::Rect)]| {
        let placements = placements.to_vec();
        super::probe::painted_full(screen, |ui| {
            for (surface, rect) in &placements {
                let mut child = ui.new_child(egui::UiBuilder::new().max_rect(*rect));
                crate::draw_pane(
                    &mut child,
                    crate::Pane::Spectral,
                    &mut state.picture,
                    100.0,
                    *surface,
                );
            }
        });
    };
    let slabs = |state: &mut SharedState, surface| {
        state.picture.surfaces.spectrogram.at(surface).folded.run_slabs()
    };

    let mut alone = seeded();
    draw(&mut alone, &[(0, narrow)]);
    let want_narrow = slabs(&mut alone, 0);
    let mut alone = seeded();
    draw(&mut alone, &[(0, wide)]);
    let want_wide = slabs(&mut alone, 0);
    assert!(want_narrow > 0, "the fixture drew no heatmap, so it never reached a fold at all");
    assert_ne!(
        want_narrow, want_wide,
        "the two rects fold the same run, so neither can tell them apart"
    );

    let mut both = seeded();
    draw(&mut both, &[(0, narrow), (1, wide)]);
    assert_eq!(slabs(&mut both, 0), want_narrow, "the first placement is drawing the second's run");
    assert_eq!(slabs(&mut both, 1), want_wide, "the second placement has no grid of its own");
}

#[test]
fn runtime_advances_histories_without_a_surface() {
    use harmonigraph_core::{NoteEvent, SourceId};
    let mut runtime = VisualRuntime::default();
    let appearance = AppearanceDocument::default();
    let samples: Vec<_> = (0..appearance.spectrum.window.frame(48_000.0) * 2)
        .map(|i| (std::f32::consts::TAU * 440.0 * i as f32 / 48_000.0).sin() * 0.5)
        .collect();
    crate::spectrum::heard_at(
        &mut runtime.spectrum,
        &samples,
        1,
        48_000.0,
        1.0,
        &appearance.spectrum,
    );
    runtime.tracker.handle_event(NoteEvent::on(0.0, SourceId::DIRECT, 0, 60, 0.8));
    runtime.tracker.handle_event(NoteEvent::off(0.1, SourceId::DIRECT, 0, 60));
    runtime.tracker.handle_event(NoteEvent::on(0.5, SourceId::DIRECT, 0, 64, 0.8));
    assert!(runtime.spectrum.columns_taken() > 2, "fixture must produce real FFT columns");
    assert_eq!(runtime.tracker.voices().count(), 2);
    runtime.advance_time(10.0, &appearance);
    assert_eq!(runtime.tracker.voices().count(), 1, "old released tail must be pruned");
    let columns = runtime.spectrum.columns_taken();
    runtime.advance_time(10.0, &appearance);
    assert_eq!(runtime.spectrum.columns_taken(), columns, "advancement must not repeat analysis");
    runtime.advance_time(1.0 + AudioSpectrum::HISTORY_MAX_SECONDS + 1.0, &appearance);
    assert!(
        runtime.spectrum.newest_column().is_some(),
        "the history is bounded by its rings, not trimmed by the clock",
    );
    assert_eq!(runtime.tracker.held_count(), 1, "a held note survives age pruning");
}

#[test]
fn fold_is_shared_by_real_dock_preview_and_discarded_passes() {
    use super::harness::RecordingBackend;
    let mut state = fresh();
    state.picture.appearance.view.spectral_ring_width = 0.15;
    state.picture.appearance.view.spectral_reading = harmonigraph_scene::SpectralReading::Fold;
    state.workspace.layout.select(panes::Tab::Video);
    let cfg = state.picture.appearance.spectrum;
    let samples: Vec<_> = (0..cfg.window.frame(48_000.0) * 2)
        .map(|i| (std::f32::consts::TAU * 440.0 * i as f32 / 48_000.0).sin() * 0.5)
        .collect();
    crate::spectrum::heard_at(
        &mut state.picture.runtime.spectrum,
        &samples,
        1,
        48_000.0,
        1.0,
        &cfg,
    );
    assert!(state.picture.runtime.spectrum.columns_taken() > 2);
    let ctx = super::probe::themed();
    let mut passes = 0;
    let _ = ctx.run_ui(
        egui::RawInput {
            screen_rect: Some(egui::Rect::from_min_size(
                egui::Pos2::ZERO,
                egui::vec2(1600.0, 1000.0),
            )),
            time: Some(1.0),
            ..Default::default()
        },
        |ui| {
            egui::Grid::new("runtime-discarded-pass").show(ui, |ui| {
                ui.label("Force a real sizing pass");
            });
            root_ui(ui, &mut state, &RecordingBackend::default(), 1.0, "test @0123456");
            if passes == 0 {
                state.picture.appearance.camera.yaw += 0.2;
                state.picture.appearance.view.pitch_gradient =
                    state.picture.appearance.spectrum.spectrogram_gradient;
            }
            passes += 1;
        },
    );
    assert!(passes >= 2, "the fixture must actually discard a pass");
    assert!(state.picture.surfaces.glow_fade.len() >= 2, "both lattice surfaces must draw");
    assert_eq!(state.picture.runtime.spectrum.store.fold_measurements, 1);
}

#[test]
fn fold_measurement_observes_same_time_audio_and_width_edits() {
    let cfg = SpectrumConfig::default();
    let mut spectrum = AudioSpectrum::default();
    let tone = |frequency: f32| {
        (0..cfg.window.frame(48_000.0) * 2)
            .map(|i| (std::f32::consts::TAU * frequency * i as f32 / 48_000.0).sin() * 0.5)
            .collect::<Vec<_>>()
    };
    let mut feed = crate::spectrum::TestFeed::default();
    feed.feed(&mut spectrum, &tone(440.0), 1, 48_000.0, 1.0, &cfg);
    let first = spectrum.folded(1.0, 2.0).unwrap().to_vec();
    assert!(first.iter().any(|v| *v > 0.0), "the fixture must contain measured energy");
    assert_eq!(spectrum.folded(1.0, 2.0).unwrap().as_slice(), first);
    assert_eq!(spectrum.store.fold_measurements, 1);
    let wide = spectrum.folded(1.0, 50.0).unwrap().to_vec();
    assert_ne!(wide, first, "a changed width must change this fixture's measurement");
    // More audio while the frame's time stands still, so only the display
    // revision can tell the memo it is stale.
    let batch = AudioSpectrum::FFT_INTERVAL * 48_000.0 * 3.0;
    let short: Vec<f32> = tone(660.0)[..batch as usize].to_vec();
    feed.feed(&mut spectrum, &short, 1, 48_000.0, 1.0 + batch / 48_000.0, &cfg);
    let next = spectrum.folded(1.0, 50.0).unwrap().to_vec();
    assert_ne!(next, wide, "same-time audio must invalidate the measured grid");
    assert_eq!(spectrum.store.fold_measurements, 3);
    assert!(spectrum.folded(2.0, 50.0).is_none(), "expired audio must not reuse a Fold");
    assert_eq!(spectrum.store.fold_measurements, 3);
}

/// A run's columns are a function of its samples and the frame zero its
/// feeder states, not of how the feeder cuts the run into calls: the offline
/// renderer hands a frame's audio over in bounded chunks, and the plugin a
/// callback's block at a time.
#[test]
fn a_run_cut_into_any_chunks_makes_the_same_columns() {
    let samples: Vec<f32> = (0..62_271)
        .flat_map(|i| {
            let x = (i as f32 * 0.047).sin();
            [x, -x]
        })
        .collect();
    let cfg = SpectrumConfig::default();
    // Feed one call's worth, and say what it made, as `(time, power)`.
    let feed =
        |spectrum: &mut AudioSpectrum, analysis: &mut SpectrumAnalysis, input, sr, origin| {
            let mut columns = Vec::new();
            analysis.feed_source(input, 2, sr, origin, &cfg, |delivery| {
                columns.extend(delivery.column().map(|c| (c.time(), c.power().to_vec())));
                spectrum.apply(delivery, &cfg)
            });
            columns
        };
    for sr in [48_000.0, 192_000.0] {
        for origin in [0.0, 7.125, 1_000_000_000.417_31] {
            for chunk_frames in [1, 7, 4093, 16_384] {
                let (mut whole, mut chunked) = (AudioSpectrum::default(), AudioSpectrum::default());
                let mut analyses = (SpectrumAnalysis::default(), SpectrumAnalysis::default());
                let (mut expected, mut actual) = (Vec::new(), Vec::new());
                for range in [0..20_031, 20_031..62_271] {
                    let input = &samples[range.start * 2..range.end * 2];
                    expected.extend(feed(&mut whole, &mut analyses.0, input, sr, origin));
                    for chunk in input.chunks(chunk_frames * 2) {
                        actual.extend(feed(&mut chunked, &mut analyses.1, chunk, sr, origin));
                    }
                    assert_eq!(analyses.0.frames_seen, analyses.1.frames_seen);
                    assert_eq!(whole.store.display, chunked.store.display);
                    assert_eq!(whole.store.last_samples, chunked.store.last_samples);
                }
                assert!(expected.len() > 10, "fixture must reach measurements");
                assert!(
                    expected == actual,
                    "{sr} Hz, origin {origin}, {chunk_frames}-frame chunks"
                );
            }
        }
    }
}

/// A setting is a length of time, so it lags and resolves the same at 96 kHz
/// as at 48. Counted in samples, every setting read twice as blurry there at
/// half the lag, and Fast could not place 20 to 44 Hz at all: a 25 Hz tone
/// read as silence.
#[test]
fn a_setting_listens_as_long_at_every_sample_rate() {
    for window in [SpectrumWindow::Fast, SpectrumWindow::Balanced, SpectrumWindow::Precise] {
        assert_eq!(window.frame(44_100.0), window.frame(48_000.0));
        assert_eq!(
            window.frame(32_000.0),
            window.frame(48_000.0),
            "32 kHz must not listen shorter"
        );
        assert_eq!(window.frame(96_000.0), 2 * window.frame(48_000.0));
        let cfg = SpectrumConfig { window, ..SpectrumConfig::default() };
        let ratio = cfg.column_lag(96_000.0) / cfg.column_lag(48_000.0);
        assert!((ratio - 1.0).abs() < 0.01, "{window:?} lags {ratio:.3}x as long at 96 kHz");
    }
    let sr = 96_000.0;
    let config =
        SpectrumConfig { window: SpectrumWindow::Fast, attack: 0.0, ..SpectrumConfig::default() };
    let sine: Vec<f32> = (0..2 * config.window.frame(sr))
        .map(|i| 0.5 * (std::f64::consts::TAU * 25.0 * i as f64 / f64::from(sr)).sin() as f32)
        .collect();
    let mut spectrum = AudioSpectrum::default();
    let columns = crate::spectrum::heard_at(&mut spectrum, &sine, 1, sr, 1.0, &config);
    let power = columns.last().expect("the frame filled").power();
    let peak = (0..power.len()).max_by(|&a, &b| power[a].total_cmp(&power[b])).unwrap();
    let midi = harmonigraph_core::spectrum::SPECTRUM_MIN_MIDI
        + (peak as f32 + 0.5) / harmonigraph_core::spectrum::BINS_PER_SEMITONE as f32;
    let off = midi - harmonigraph_core::spectrum::hz_to_midi(25.0);
    assert!(
        power[peak] > 0.0 && off.abs() < 0.5,
        "a 25 Hz tone at 96 kHz peaks {off:+.2} semitones off"
    );
}

/// A column of `power` in every bucket at `time`, as the analysis hands one
/// over.
fn delivered(time: f64, power: f32) -> Delivery {
    Delivery::Column(Box::new(crate::spectrum::AnalyzedColumn {
        time,
        power: [power; harmonigraph_core::spectrum::SPECTRUM_BINS],
        step: AudioSpectrum::FFT_INTERVAL,
    }))
}

/// A column at or up to a second before the newest is dropped and counted:
/// it folds into no slab, moves the newest date nowhere, and stamps no
/// heard column — which is what tells a drop from a fold here, since the
/// column before it was silent.
#[test]
fn the_guard_drops_a_column_at_or_just_before_the_newest() {
    let cfg = SpectrumConfig::default();
    let mut spectrum = AudioSpectrum::default();
    let clears = spectrum.clears();
    spectrum.give(clears, [delivered(100.0, 0.0)], &cfg);
    assert_eq!(spectrum.last_audible().column, None, "the fixture's first column is silent");
    crate::spectrum::expecting_backward_dates(|| {
        let backward = [100.0, 100.0 - 0.13, 100.0 - 0.99, f64::NAN];
        spectrum.give(clears, backward.map(|time| delivered(time, 1.0)), &cfg);
    });
    assert_eq!(spectrum.guard_counts(), (4, 0));
    assert_eq!(spectrum.newest_column(), Some(100.0));
    assert_eq!(spectrum.last_audible().column, None, "a dropped column was heard");
    assert_eq!(spectrum.pyramid().open_count(0), 1, "a dropped column was folded");
}

/// A column further back than a second resets the history and starts it
/// again, counted, and so does a correct column after one dated an hour
/// ahead, which would otherwise have every correct column dropped for an
/// hour. A reset is not a Clear: the rest of the hand-over it happened in
/// still joins the history it started, where a Clear between two
/// hand-overs keeps the second out.
#[test]
fn the_guard_resets_on_a_long_step_back_and_keeps_the_rest_of_the_batch() {
    let cfg = SpectrumConfig::default();
    let mut spectrum = AudioSpectrum::default();
    let clears = spectrum.clears();
    spectrum.give(clears, [delivered(100.0, 1.0)], &cfg);
    let epoch = spectrum.history_epoch();
    crate::spectrum::expecting_backward_dates(|| {
        spectrum.give(clears, [delivered(100.0 - 1.01, 1.0)], &cfg);
    });
    assert_eq!(spectrum.guard_counts(), (0, 1));
    assert_eq!(spectrum.newest_column(), Some(100.0 - 1.01), "the reset dropped its column");
    assert_ne!(spectrum.history_epoch(), epoch, "the colour memory missed the reset");

    let ahead = 100.0 + 3600.0;
    let batch = [ahead, 101.0, 101.008, 101.016].map(|time| delivered(time, 1.0));
    crate::spectrum::expecting_backward_dates(|| spectrum.give(clears, batch, &cfg));
    assert_eq!(spectrum.guard_counts(), (0, 2));
    assert_eq!(spectrum.newest_column(), Some(101.016), "the batch after the reset was lost");
    assert_eq!(spectrum.pyramid().open_count(crate::spectrogram::LEVELS - 1), 3);

    // A Clear between two hand-overs of audio taken before it.
    spectrum.give(clears, [delivered(101.024, 1.0)], &cfg);
    spectrum.clear_history();
    spectrum.give(clears, [delivered(101.032, 1.0)], &cfg);
    assert_eq!(spectrum.newest_column(), None, "a hand-over taken before the Clear joined it");
}

/// A crash inside the fold leaves the levels part-folded, which a history
/// written once would draw for as long as a level keeps a slab, so it resets
/// the history; a crash anywhere else costs it nothing. Either way the next
/// column folds.
#[test]
fn a_crash_in_the_fold_resets_the_history_and_one_outside_it_does_not() {
    let bins = [0.5; harmonigraph_core::spectrum::SPECTRUM_BINS];
    let mut spectrum = AudioSpectrum::default();
    spectrum.push_history(1.0, &bins);
    let epoch = spectrum.history_epoch();

    let outside = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        spectrum.push_history(1.008, &bins);
        panic!("a fault after the fold");
    }));
    assert!(outside.is_err());
    assert_eq!(spectrum.newest_column(), Some(1.008), "a crash outside the fold cost history");
    assert_eq!((spectrum.history_epoch(), spectrum.guard_counts()), (epoch, (0, 0)));

    crate::spectrogram::FAIL_AT_LEVEL.set(Some(7));
    let inside = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        spectrum.push_history(1.016, &bins);
    }));
    crate::spectrogram::FAIL_AT_LEVEL.set(None);
    assert!(inside.is_err(), "the fault was not injected");
    assert_eq!(spectrum.newest_column(), None, "a part-folded history survived the crash");
    assert_ne!(spectrum.history_epoch(), epoch, "the colour memory missed the reset");
    assert_eq!(spectrum.guard_counts(), (0, 1));

    spectrum.push_history(1.024, &bins);
    assert_eq!(spectrum.newest_column(), Some(1.024));
}

/// The costs the slab pyramid adds, for the numbers #1758 quotes: a column
/// folded into all sixteen levels, a silent one on the fast path, and the
/// hold of the editor lock a mono catch-up takes, 371 columns given at
/// once. Printed, not asserted; run in release with `--ignored
/// --nocapture`.
#[test]
#[ignore]
fn the_folds_timings() {
    use std::time::Instant;
    const BINS: usize = harmonigraph_core::spectrum::SPECTRUM_BINS;
    let cfg = SpectrumConfig::default();
    let mut seed = 0x9E37_79B9_7F4A_7C15u64;
    let mut power = || {
        let mut power = [0.0f32; BINS];
        for p in power.iter_mut() {
            seed ^= seed << 13;
            seed ^= seed >> 7;
            seed ^= seed << 17;
            *p = 10f32.powf(-11.0 + 10.0 * (seed >> 40) as f32 / (1u64 << 24) as f32);
        }
        power
    };
    let per_column = |columns: &[[f32; BINS]]| {
        let mut pyramid = crate::spectrogram::SlabPyramid::default();
        let start = Instant::now();
        for (k, power) in columns.iter().enumerate() {
            pyramid.push(90.0 + k as f64 * AudioSpectrum::FFT_INTERVAL, power);
        }
        start.elapsed().as_secs_f64() / columns.len() as f64 * 1e6
    };
    let loud: Vec<_> = (0..4000).map(|_| power()).collect();
    let silent = vec![[0.0f32; BINS]; 4000];
    println!("a column through all sixteen levels: {:.2} us", per_column(&loud));
    println!("a silent column, fast path: {:.3} us", per_column(&silent));

    let mut spectrum = AudioSpectrum::default();
    let clears = spectrum.clears();
    let deliveries: Vec<_> = (0..371)
        .map(|k| {
            Delivery::Column(Box::new(crate::spectrum::AnalyzedColumn {
                time: 90.0 + k as f64 * AudioSpectrum::FFT_INTERVAL,
                power: power(),
                step: AudioSpectrum::FFT_INTERVAL,
            }))
        })
        .collect();
    let start = Instant::now();
    spectrum.give(clears, deliveries, &cfg);
    let hold = start.elapsed().as_secs_f64() * 1e3;
    println!("the hold of a 371-column catch-up given at once: {hold:.3} ms");
}
