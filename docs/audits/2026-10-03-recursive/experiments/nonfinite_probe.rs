#[test]
fn audit_display_recovers_after_one_nonfinite_audio_sample() {
    let mut spectrum = AudioSpectrum::default();
    let config = SpectrumConfig {
        window: SpectrumWindow::Fast,
        attack: 0.0,
        release: 0.0,
        ..SpectrumConfig::default()
    };
    let n = config.window.samples();
    let mut poisoned = vec![0.0; n + 384];
    poisoned[n / 2] = f32::NAN;
    spectrum.push_samples(&poisoned, 1, 48_000.0, 1.0, &config);
    let bad_before = spectrum.display(1.0).unwrap().iter().filter(|p| !p.is_finite()).count();
    let clean: Vec<f32> = (0..2 * n).map(|i| 0.5 * (std::f32::consts::TAU * 440.0 * i as f32 / 48_000.0).sin()).collect();
    let clean_end = 1.0 + clean.len() as f64 / 48_000.0;
    spectrum.push_samples(&clean, 1, 48_000.0, clean_end, &config);
    let last = spectrum.history().back().expect("clean input emitted columns");
    assert!(last.power_sum.iter().all(|p| p.is_finite()));
    assert!(last.power_sum.iter().any(|p| *p > 0.01), "last FFT measures clean tone");
    let shown = spectrum.display(clean_end).expect("clean input is flowing");
    let bad_after = shown.iter().filter(|p| !p.is_finite()).count();
    eprintln!("window={n}; nonfinite display buckets before={bad_before}, after two clean windows={bad_after}; latest history is finite and lit");
    assert_eq!(bad_after, 0, "past nonfinite input must not permanently poison the display");
}
