//! The analyzer and its spectrogram history: what is stored, when it is
//! stamped, and how the live path and the offline precompute agree.

use super::probe::fresh;
use crate::*;

#[test]
fn audio_spectrum_shows_while_flowing_and_hides_after() {
    let mut spectrum = AudioSpectrum::default();
    let config = SpectrumConfig::default();
    assert!(spectrum.display(0.0).is_none(), "no audio yet");

    // A 440 Hz sine, long enough to fill the analysis window.
    let sine: Vec<f32> = (0..9_000)
        .map(|i| 0.5 * (std::f32::consts::TAU * 440.0 * i as f32 / 48_000.0).sin())
        .collect();
    spectrum.push_samples(&sine, 1, 48_000.0, 1.0, &config);
    let levels = spectrum.display(1.0).expect("audio is flowing");
    let peak =
        levels.iter().enumerate().max_by(|a, b| a.1.total_cmp(b.1)).map(|(i, _)| i as i32).unwrap();
    // A4 is MIDI 69; its bucket scales with the axis resolution.
    let a4 = ((69.0 - harmonigraph_core::spectrum::SPECTRUM_MIN_MIDI)
        * harmonigraph_core::spectrum::BINS_PER_SEMITONE as f32) as i32;
    assert!((peak - a4).abs() <= 1, "440 Hz should peak at A4 (bucket {a4}), got {peak}");

    // Once samples stop, the curve hides instead of freezing.
    assert!(spectrum.display(1.0 + AudioSpectrum::HOLD_SECONDS + 0.1).is_none());
}

/// Music fills most of the analyzer's height, rather than half of it.
///
/// The ceiling used to be full scale, and nothing musical puts full scale in
/// ONE bucket: a chord splits its power across its partials, and the default
/// tilt takes another 10 dB off anything well under the 1 kHz pivot. The curve
/// topped out halfway up and the top half of the pane was empty in normal use.
///
/// So the defaults are held to a chord rather than to a test tone. This one
/// reads 0.90 of the pane as they stand and 0.60 against a full-scale ceiling,
/// so 0.75 is the line between the two — what it catches is the ceiling
/// drifting back up, not a shift of a few dB either way. The upper bound is
/// the other failure: a curve clipped flat against the top has lost the shape
/// of its own peaks, which is worse than empty space above it.
#[test]
fn a_chord_fills_most_of_the_analyzers_height() {
    let sr = 48_000.0;
    let cfg = SpectrumConfig::default();
    // Six partials sharing the headroom, peaking about -12 dBFS — a mix, not a
    // tone. Two seconds, so the smoothing has long settled.
    let samples: Vec<f32> = (0..24_000)
        .map(|i| {
            let t = i as f32 / sr;
            let mix: f32 = [220.0, 277.2, 329.6, 440.0, 554.4, 659.3]
                .iter()
                .map(|f| (std::f32::consts::TAU * f * t).sin())
                .sum();
            0.25 * mix / 6.0_f32.sqrt()
        })
        .collect();
    let mut spectrum = AudioSpectrum::default();
    spectrum.push_samples(&samples, 1, sr, 1.0, &cfg);
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
    let window = SpectrumConfig::default().window.samples();
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
        spectrum.push_samples(&tone, 1, sr, 1.0, &cfg);
        let lit = spectrum.display(1.0).expect("audio is flowing")[a4];
        spectrum.push_samples(&silence, 1, sr, 2.0, &cfg);
        let left = spectrum.display(2.0).expect("audio is still flowing")[a4];
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

/// The Tapers setting reaches the analyzer: the same audio measured at one
/// taper and at three does not come out as the same curve.
///
/// Nothing else in the workspace constructs `Three` or `Five` — the button row
/// names them and `count()` maps them, and every taper claim in
/// `harmonigraph-core` is made against a bare `SpectrumAnalyzer`. So without
/// this, deleting the `set_tapers` line in `push_samples` leaves the control
/// inert with the whole suite green.
///
/// WHAT the difference is stays in core, where it is measured
/// (`more_tapers_steady_a_bucket_against_noise` and
/// `the_noise_floor_reads_higher_as_tapers_are_added`). This holds only that
/// the setting is connected to something.
#[test]
fn the_tapers_setting_reaches_the_analyzer() {
    let sr = 48_000.0;
    // Deterministic white noise: something in every bucket, which is what makes
    // a change of estimator show up across the axis rather than in the skirts
    // of one partial.
    let mut seed = 0x1234_5678u32;
    let noise: Vec<f32> = (0..24_000)
        .map(|_| {
            seed ^= seed << 13;
            seed ^= seed >> 17;
            seed ^= seed << 5;
            (seed as f32 / u32::MAX as f32) * 2.0 - 1.0
        })
        .collect();

    let curve = |tapers| {
        let cfg = SpectrumConfig { tapers, ..SpectrumConfig::default() };
        let mut spectrum = AudioSpectrum::default();
        spectrum.push_samples(&noise, 1, sr, 1.0, &cfg);
        spectrum.display(1.0).expect("audio is flowing").to_vec()
    };
    let one = curve(SpectrumTapers::One);
    for more in [SpectrumTapers::Three, SpectrumTapers::Five] {
        // Counted rather than compared whole: the grid is 3828 buckets, and an
        // `assert_ne!` on the pair prints both of them.
        let moved = one.iter().zip(curve(more)).filter(|(a, b)| **a != *b).count();
        assert!(moved > 0, "{more:?} tapers measured what one taper did, in every bucket");
    }
}

#[test]
fn spectrogram_history_stays_bounded() {
    let bins = [0.0f32; harmonigraph_core::spectrum::SPECTRUM_BINS];

    // Retention is span-INDEPENDENT: it must NOT track the current window, or
    // shrinking the span pops history that widening it again can never recover
    // (the "reducing then increasing span erases history" bug). Everything
    // within HISTORY_MAX_SECONDS is kept no matter what the span is doing.
    let mut spec = AudioSpectrum::default();
    for i in 0..300 {
        spec.push_history(i as f64, &bins);
    }
    assert_eq!(spec.history().front().unwrap().time, 0.0, "no column within the cap is dropped");
    assert_eq!(spec.history().back().unwrap().time, 299.0, "newest kept");

    // The retention never exceeds the hard age cap — the ceiling on how far
    // back the heatmap can read.
    let mut spec = AudioSpectrum::default();
    for i in 0..800 {
        spec.push_history(i as f64, &bins);
    }
    let cutoff = 799.0 - AudioSpectrum::HISTORY_MAX_SECONDS;
    assert!(spec.history().front().unwrap().time >= cutoff, "capped at HISTORY_MAX_SECONDS");

    // Memory holds even when every column shares one timestamp, so the age trim
    // never fires — the store's own tier caps are the backstop.
    let mut spec = AudioSpectrum::default();
    for _ in 0..(SpectrumHistory::MAX_COLUMNS + 50) {
        spec.push_history(0.0, &bins);
    }
    assert!(spec.history().len() <= SpectrumHistory::MAX_COLUMNS, "column count capped");

    spec.clear_history();
    assert!(spec.history().is_empty());
}

/// The live path stamps its columns the same way the offline one does, and
/// the near-edge grace knows about the lag that creates. Two halves of one
/// thing: a column half a window old is not a stale column, it is the newest
/// there can be, and the heatmap must still reach the now-line.
#[test]
fn a_live_column_is_stamped_at_the_middle_of_its_window() {
    let mut spectrum = AudioSpectrum::default();
    let config = SpectrumConfig::default();
    // A WHOLE number of hops, so the last column's window ends exactly at `now`
    // and the stamp can be checked exactly. A spectrum is taken every
    // FFT_INTERVAL of audio (see `push_samples`), so a batch ending mid-hop
    // leaves its newest column up to one hop further back than this — correctly,
    // since that is where the window it measured ends.
    let hop = (AudioSpectrum::FFT_INTERVAL * 48_000.0).round() as usize;
    let samples = hop * (9_000 / hop + 1); // enough to fill the 8192 window
    let sine: Vec<f32> = (0..samples)
        .map(|i| 0.5 * (std::f32::consts::TAU * 440.0 * i as f32 / 48_000.0).sin())
        .collect();
    let now = 5.0;
    spectrum.push_samples(&sine, 1, 48_000.0, now, &config);
    spectrum.display(now).expect("audio is flowing");

    let window = f64::from(config.window.samples() as u32) / 48_000.0;
    let stamped = spectrum.history().back().expect("a column was kept").time;
    assert!(
        (stamped - (now - window * 0.5)).abs() < 1e-9,
        "a column measured over [{:.3}, {now}] was stamped {stamped}, not its middle",
        now - window,
    );
    // And the pane's own idea of that lag agrees, which is what keeps the
    // strip's near edge on the now-line rather than half a window short.
    assert!((spectrum.column_lag() - window * 0.5).abs() < 1e-9);
}

/// The column grid is a function of the SAMPLES, not of when the shell happened
/// to hand them over — which is the whole reason the FFT moved into
/// `push_samples`. A shell drains its audio ring on frame boundaries while the
/// ring fills in audio blocks, so batch sizes swing by a block and the frame
/// clock wobbles against the audio clock by several ms; the old frame-gated FFT
/// passed all of that into the picture. It could only fire ON a frame, so a
/// 20 ms interval on a 60 Hz display fired every 33.3 ms — wider than the slabs
/// the heatmap cuts the window into, which then went empty and were painted by
/// duplicating a neighbour.
///
/// Hence the second assertion, which is the one the eye sees: no gap wider than
/// `MIN_BUCKET` means no slab is ever empty, at any frame rate or cap.
#[test]
fn columns_are_evenly_spaced_however_the_shell_batches_them() {
    use crate::spectrogram::MIN_BUCKET;
    let sr = 48_000.0f32;
    let config = SpectrumConfig::default();
    let mut spectrum = AudioSpectrum::default();
    let mut written = 0usize;
    for batch in 0..48 {
        // Sizes a block apart, and a frame clock that leads and lags the audio
        // it is dating by 4 ms — twice what it takes to lose a column.
        let n = [512usize, 256, 1024, 128, 768][batch % 5];
        let chunk: Vec<f32> = (0..n)
            .map(|k| {
                let t = (written + k) as f32 / sr;
                0.5 * (std::f32::consts::TAU * 440.0 * t).sin()
            })
            .collect();
        written += n;
        let now =
            f64::from(written as u32) / f64::from(sr) + if batch % 2 == 0 { 0.004 } else { -0.004 };
        spectrum.push_samples(&chunk, 1, sr, now, &config);
    }

    let times: Vec<f64> = spectrum.history().iter().map(|c| c.time).collect();
    assert!(times.len() > 20, "only {} columns for 48 batches", times.len());
    let hop = AudioSpectrum::FFT_INTERVAL;
    for pair in times.windows(2) {
        let gap = pair[1] - pair[0];
        assert!(
            (gap - hop).abs() < hop * 0.25,
            "columns {:.4} s apart, not {hop} — the batching is reaching the grid",
            gap,
        );
        assert!(gap < MIN_BUCKET, "a {gap:.4} s gap can leave a {MIN_BUCKET} s slab empty");
    }
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
fn the_live_path_and_the_offline_precompute_agree_on_stereo() {
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
    spectrum.push_samples(&samples, 2, sr, span, &cfg);
    let live: Vec<_> = spectrum.history().iter().map(|c| (c.time, c.db.clone())).collect();

    // Offline: the same buffer, the whole-song build.
    let ws = WholeSong::precompute(&samples, 2, sr, 0.0, 0.0, span, &cfg);
    let offline: Vec<_> = ws.columns.iter().map(|c| (c.time, c.db.clone())).collect();

    assert!(live.len() > 50, "only {} live columns for a second of audio", live.len());
    assert_eq!(live.len(), offline.len(), "different column counts");
    for (i, ((lt, ldb), (ot, odb))) in live.iter().zip(&offline).enumerate() {
        assert!((lt - ot).abs() < 1e-6, "column {i} stamped {lt} live, {ot} offline");
        assert!(ldb == odb, "column {i} holds different buckets live and offline");
    }

    // And both really did keep the anti-phase partial — otherwise the two could
    // agree by both being wrong in the same way.
    let bucket_of = |hz: f32| {
        ((harmonigraph_core::spectrum::hz_to_midi(hz)
            - harmonigraph_core::spectrum::SPECTRUM_MIN_MIDI)
            * harmonigraph_core::spectrum::BINS_PER_SEMITONE as f32)
            .round() as usize
    };
    let loudest = |bucket: usize| {
        live.iter()
            .flat_map(|(_, db)| db[bucket.saturating_sub(1)..=bucket + 1].iter().copied())
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

/// A spectrum is measured over a WINDOW, not at an instant, so where it lands
/// on the time axis is a choice — and the only defensible one is the middle of
/// what it measured. Stamping it when the FFT ran (the end of that window) drew
/// every sound half a window late: at the default 8192 that is 85 ms, so a note
/// ribbon sat 85 ms further from the now-line than the energy it made, and
/// reached the far edge — and vanished — that much before its own audio did.
///
/// Checked where it is measurable rather than argued: a tone that starts at a
/// known moment must light the heatmap at that moment.
#[test]
fn a_tones_energy_lands_at_the_time_the_tone_started() {
    use harmonigraph_core::spectrum::midi_to_hz;
    let sr = 48_000.0f32;
    let onset = 1.0f64; // silence before this, a steady A4 after
    let seconds = 3.0;
    let freq = midi_to_hz(69.0);
    let samples: Vec<f32> = (0..(sr as f64 * seconds) as usize)
        .map(|i| {
            let t = f64::from(i as u32) / f64::from(sr);
            if t < onset {
                0.0
            } else {
                0.8 * (std::f32::consts::TAU * freq * i as f32 / sr).sin()
            }
        })
        .collect();
    let cfg = SpectrumConfig::default();
    let ws = WholeSong::precompute(&samples, 1, sr, 0.0, 0.0, seconds, &cfg);

    // The bin the tone sits in, and how loud it reads once fully sounding.
    // Columns are stored as bytes of dB, so "half power" is 3 dB down from the
    // peak rather than half its stored value.
    use harmonigraph_core::spectrogram::{db_of, DB_STEP};
    let a4 = ((69.0 - harmonigraph_core::spectrum::SPECTRUM_MIN_MIDI)
        * harmonigraph_core::spectrum::BINS_PER_SEMITONE as f32)
        .round() as usize;
    let loudest = ws.columns.iter().map(|c| c.db[a4]).max().expect("columns");
    assert!(db_of(loudest) > -10.0, "the tone should read loudly at its own bin");

    // Where the ridge reaches half power is what the eye reads as the onset:
    // the window is Hann-weighted, so it crosses half when it is half over the
    // start of the tone. That must be the moment the tone started.
    let half_power = loudest.saturating_sub((3.01 / DB_STEP).round() as u8);
    let half = ws
        .columns
        .iter()
        .find(|c| c.db[a4] >= half_power)
        .expect("the tone must reach half power somewhere");
    let window = f64::from(cfg.window.samples() as u32) / f64::from(sr);
    assert!(
        (half.time - onset).abs() < window * 0.25,
        "the tone starts at {onset} s but its energy reads as starting at {} s \
         (a {window:.3} s window; half of one late would be the old end-stamping)",
        half.time,
    );
}

/// The store has to be sized for the retention policy above it: every second
/// inside `HISTORY_MAX_SECONDS` must have columns to draw, or a long span shows
/// a heatmap that stops partway and bare roll beyond it (which is exactly what
/// a fixed-rate ring does — 160 MB buys 3.5 minutes of a 10 minute span).
/// Raising the cap means adding a tier; this is what says so.
#[test]
fn spectrum_history_reaches_the_retention_cap() {
    let reach = SpectrumHistory::reach(AudioSpectrum::FFT_INTERVAL);
    assert!(
        reach >= AudioSpectrum::HISTORY_MAX_SECONDS,
        "history reaches {reach:.0} s, retention asks for {:.0} s — add a tier",
        AudioSpectrum::HISTORY_MAX_SECONDS,
    );
    // And it fits in a budget worth calling an optimization: the fixed-rate
    // f32 ring needed 160 MB to reach a third as far.
    //
    // The 30 MB is bought by the DISPLAY rather than by reach: `LIVE_SLAB_CAP`
    // is 1024 so that a close-up span is cut into slabs as fine as the data,
    // and the tiers have to keep up with the cap (see COARSE_COLUMNS) — so the
    // cap, the tier size, and this number are one decision. Reach comes along
    // for free.
    let megabytes = SpectrumHistory::max_bytes() as f64 / (1024.0 * 1024.0);
    assert!(megabytes < 32.0, "the full store is {megabytes:.1} MB");
}

/// The bargain the tiers are struck on: a column of age `a` is only ever drawn
/// when the window is at least `a` long, and a window is cut into at most
/// `LIVE_SLAB_CAP` slabs — so nothing needs storing finer than `a / cap`. Every
/// tier must stay on the right side of that, or its columns land more than a
/// slab apart and the heatmap grows stripes of false silence between them.
///
/// This is the test to look at if the tier sizes, the FFT rate, or the slab cap
/// ever move: they are three legs of one stool.
///
/// The display now picks its slab off the same power-of-two ladder the tiers
/// merge on (`live_slab`), so the two round to the same rung rather than merely
/// bounding each other — but the bargain is what makes that ladder the right
/// one, so it is still worth stating against the real function.
#[test]
fn stored_columns_stay_finer_than_the_slabs_they_are_drawn_into() {
    use crate::spectrogram::{live_slab, LIVE_SLAB_CAP};
    let mut age = 0.0f64; // youngest age the tier holds
    let mut spacing = AudioSpectrum::FFT_INTERVAL;
    for tier in 0..SpectrumHistory::TIERS {
        // The finest slab any window that reaches this tier's youngest columns
        // can use — the tightest the tier is ever asked to be.
        let finest = live_slab(age, LIVE_SLAB_CAP as usize);
        assert!(
            spacing <= finest,
            "tier {tier} stores {spacing:.3} s apart but can be drawn into \
             {finest:.3} s slabs (from age {age:.1} s)",
        );
        let columns =
            if tier == 0 { SpectrumHistory::FINE_COLUMNS } else { SpectrumHistory::COARSE_COLUMNS };
        age += columns as f64 * spacing;
        spacing *= 2.0;
    }
}

#[test]
fn whole_song_precompute_lays_the_take_out_deterministically() {
    use harmonigraph_core::spectrum::{
        midi_to_hz, BINS_PER_SEMITONE, SPECTRUM_BINS, SPECTRUM_MIN_MIDI,
    };
    let sr = 48_000.0f32;
    let seconds = 2.0;
    let n = (sr as f64 * seconds) as usize;
    // A steady A4 (MIDI 69) across the whole buffer.
    let freq = midi_to_hz(69.0);
    let samples: Vec<f32> =
        (0..n).map(|i| 0.8 * (std::f32::consts::TAU * freq * i as f32 / sr).sin()).collect();
    let cfg = SpectrumConfig::default();

    let ws = WholeSong::precompute(&samples, 1, sr, 0.0, 0.0, seconds, &cfg);
    assert_eq!(ws.span, seconds);
    assert_eq!(ws.start, 0.0);
    assert!(ws.columns.len() > 10, "a 2 s take yields many columns, got {}", ws.columns.len());

    // A full-take request starts at sample 0 and ends at the buffer's last
    // frame, so bounding the feed leaves its established hop grid untouched.
    let window_frames = cfg.window.samples();
    let hop_frames = (AudioSpectrum::FFT_INTERVAL * f64::from(sr)).round() as usize;
    let first_end = window_frames.div_ceil(hop_frames) * hop_frames;
    let expected_columns = (n - first_end) / hop_frames + 1;
    assert_eq!(ws.columns.len(), expected_columns, "the full-take column grid changed");
    let window_center = window_frames as f64 / f64::from(sr) * 0.5;
    assert!((ws.columns[0].time - (first_end as f64 / f64::from(sr) - window_center)).abs() < 1e-9);
    assert!((ws.columns.last().unwrap().time - (seconds - window_center)).abs() < 1e-9);

    // Columns are in take time, strictly increasing, inside the take.
    let mut prev = -1.0;
    for c in &ws.columns {
        assert!(c.time > prev, "columns are time-ordered");
        assert!(c.time > 0.0 && c.time <= seconds + 0.1, "column time {} in range", c.time);
        prev = c.time;
    }

    // A steady tone lands its energy at A4's bin.
    let a4 = ((69.0 - SPECTRUM_MIN_MIDI) * BINS_PER_SEMITONE as f32).round() as usize;
    let mid = &ws.columns[ws.columns.len() / 2];
    let peak = (0..SPECTRUM_BINS).max_by_key(|&b| mid.db[b]).unwrap();
    assert!(peak.abs_diff(a4) <= 1, "peak bin {peak} should be A4 (bin {a4})");

    // `time_origin` shifts every column onto the take's timeline.
    let shifted = WholeSong::precompute(&samples, 1, sr, 5.0, 5.0, seconds, &cfg);
    assert!(
        (shifted.columns[0].time - ws.columns[0].time - 5.0).abs() < 1e-6,
        "time_origin offsets the columns"
    );

    // Pure: same inputs in, byte-identical columns out (the render leans on
    // this for reproducibility).
    let again = WholeSong::precompute(&samples, 1, sr, 0.0, 0.0, seconds, &cfg);
    assert_eq!(ws.columns.len(), again.columns.len());
    for (a, b) in ws.columns.iter().zip(&again.columns) {
        assert_eq!(a.time, b.time);
        assert_eq!(a.db, b.db, "precompute is deterministic");
    }
}

/// A late render window pays for that window and its analyzer input margins,
/// while keeping the same absolute sample grid as a full-take analysis.
///
/// The nonzero audio origin and a start four seconds into a six-second take are
/// both load-bearing: a fixture starting at zero would never enter the trimmed
/// path, and relative slice timestamps would happen to look like take time.
/// The signal changes pitch at the render start so the first drawable column
/// also depends on samples from both sides of that boundary; matching the full
/// analysis proves the pre-roll reaches the FFT rather than merely moving a
/// timestamp.
#[test]
fn a_late_window_precomputes_only_its_audio_on_the_take_grid() {
    use harmonigraph_core::spectrum::midi_to_hz;
    let sr = 4_000.0f32;
    let take_seconds = 6.0;
    let time_origin = 10.0;
    let start = 14.0;
    let span = 1.0;
    let frames = (f64::from(sr) * take_seconds) as usize;
    let samples: Vec<f32> = (0..frames)
        .map(|i| {
            let t = i as f32 / sr;
            let hz = if f64::from(t) < start - time_origin {
                midi_to_hz(57.0)
            } else {
                midi_to_hz(69.0)
            };
            0.7 * (std::f32::consts::TAU * hz * t).sin()
        })
        .collect();
    let cfg = SpectrumConfig { window: SpectrumWindow::Fast, ..SpectrumConfig::default() };
    let full = WholeSong::precompute(&samples, 1, sr, time_origin, time_origin, take_seconds, &cfg);
    let late = WholeSong::precompute(&samples, 1, sr, time_origin, start, span, &cfg);
    let past_audio = WholeSong::precompute(
        &samples,
        1,
        sr,
        time_origin,
        time_origin + take_seconds + 1.0,
        span,
        &cfg,
    );
    assert!(past_audio.columns.is_empty(), "a disjoint window still analyzed the take");

    let window = cfg.window.samples() as f64 / f64::from(sr);
    let hop = AudioSpectrum::FFT_INTERVAL;
    let stored_ceiling = ((span + window) / hop).ceil() as usize + 1;
    assert!(
        late.columns.len() <= stored_ceiling,
        "{} columns exceed a {span} s window plus {window:.3} s of history",
        late.columns.len(),
    );
    assert!(
        full.columns.len() > late.columns.len() * 3,
        "the fixture did not distinguish a late slice ({} columns) from the full take ({})",
        late.columns.len(),
        full.columns.len(),
    );
    assert!(
        late.columns.first().is_some_and(|c| c.time >= start - window && c.time < start),
        "the stored set does not begin in the analyzer history before {start}",
    );
    assert!(
        late.columns.last().is_some_and(|c| c.time <= start + span),
        "the stored set reaches beyond the requested end",
    );

    let drawn: Vec<_> = late.drawn_columns(span).collect();
    assert!(drawn.len() > 10, "the late fixture never reached drawable columns");
    assert!(
        (drawn[0].time - start).abs() < hop * 0.25,
        "the first drawable column is stamped {} instead of absolute take time {start}",
        drawn[0].time,
    );
    assert!(
        (drawn.last().unwrap().time - (start + span)).abs() < hop * 0.25,
        "the last drawable column is stamped {} instead of reaching {}",
        drawn.last().unwrap().time,
        start + span,
    );
    for column in &late.columns {
        let reference = full
            .columns
            .iter()
            .find(|candidate| candidate.time == column.time)
            .unwrap_or_else(|| panic!("{} is not on the full take's hop grid", column.time));
        assert_eq!(
            column.db, reference.db,
            "the sliced analyzer lost the history behind column {}",
            column.time,
        );
    }
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
    state.tracker.handle_event(harmonigraph_core::NoteEvent::on(
        0.0,
        harmonigraph_core::SourceId::DIRECT,
        0,
        60,
        1.0,
    ));
    state.tracker.handle_event(harmonigraph_core::NoteEvent::off(
        0.5,
        harmonigraph_core::SourceId::DIRECT,
        0,
        60,
    ));
    let env = harmonigraph_core::Envelope { fade_time: 0.1, ..Default::default() };
    state.tracker.prune(600.0, &env);
    // One analyzed column is enough; how audio becomes columns is the
    // spectrogram's own business and tested there.
    state.spectrum.history.push(harmonigraph_core::SpectrogramColumn::from_power(
        0.0,
        &[1.0; harmonigraph_core::spectrum::SPECTRUM_BINS],
    ));
    // A surface's glow state, standing in for whatever a lit lattice pane
    // would have handed a row by now — how it gets there is `glow_fade`'s
    // own business and tested there.
    state.glow_fade.insert(0, Default::default());

    assert!(!state.tracker.history().is_empty(), "no trail to clear");
    assert!(!state.tracker.roll().is_empty(), "no roll to clear");
    assert!(!state.spectrum.history().is_empty(), "no spectrogram to clear");
    assert!(!state.glow_fade.is_empty(), "no glow to clear");

    state.clear_accumulated();

    assert!(state.tracker.history().is_empty(), "the lattice trail survived");
    assert!(state.tracker.roll().is_empty(), "the piano roll survived");
    assert!(state.spectrum.history().is_empty(), "the spectrogram survived");
    assert!(state.glow_fade.is_empty(), "the lattice glow survived");
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
        state.spectrum_config.show_spectrogram = true;
        state.spectrum_config.roll_seconds = 10.0;
        let mut bins = [0.0f32; harmonigraph_core::spectrum::SPECTRUM_BINS];
        bins[harmonigraph_core::spectrum::SPECTRUM_BINS / 2] = 1.0;
        for i in 0..80 {
            state.spectrum.push_history(90.0 + f64::from(i) * 0.125, &bins);
        }
        state
    };
    let draw = |state: &mut SharedState, placements: &[(usize, egui::Rect)]| {
        let placements = placements.to_vec();
        super::probe::painted_full(screen, |ui| {
            for (surface, rect) in &placements {
                let mut child = ui.new_child(egui::UiBuilder::new().max_rect(*rect));
                crate::draw_pane(&mut child, crate::Pane::Spectral, state, 100.0, *surface);
            }
        });
    };
    let slabs =
        |state: &mut SharedState, surface| state.spectrum.spectrogram.at(surface).gpu.run_slabs();

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
