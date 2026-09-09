// Included in spectral_fold::tests by setup.py; no production hooks.
#[test]
#[ignore]
fn adoption_gate_histories() {
    use harmonigraph_core::Envelope;
    use harmonigraph_scene::{RingFade, RingGate};
    use std::io::Write;
    let dir = std::env::var("ADOPTION_OUTPUT").unwrap();
    std::fs::create_dir_all(&dir).unwrap();
    let mut out = std::fs::File::create(format!("{dir}/gates.csv")).unwrap();
    writeln!(out, "n,tapers,reading,phase,gain_bits,peak_bucket,byte,node_peak,fade").unwrap();
    let calibrate = std::env::var_os("ADOPTION_CALIBRATE").is_some();
    let path = "/tmp/realfft-adoption/gains.txt";
    let mut gains: Vec<u32> = if calibrate {
        Vec::new()
    } else {
        std::fs::read_to_string(path).unwrap().lines().map(|x| x.parse().unwrap()).collect()
    };
    let mut nodes = std::fs::File::create(format!("{dir}/nodes.csv")).unwrap();
    writeln!(nodes, "n,tapers,reading,phase,node,cents,level,peak").unwrap();
    let mut index = 0;
    for (window, n) in [
        (crate::SpectrumWindow::Fast, 4096usize),
        (crate::SpectrumWindow::Balanced, 8192),
        (crate::SpectrumWindow::Precise, 16384),
    ] {
        for tapers in
            [crate::SpectrumTapers::One, crate::SpectrumTapers::Three, crate::SpectrumTapers::Five]
        {
            for reading in [SpectralReading::Fold, SpectralReading::Spectrum] {
                let mut state = ringing();
                state.appearance.spectrum.window = window;
                state.appearance.spectrum.tapers = tapers;
                state.appearance.spectrum.attack = 0.0;
                state.appearance.spectrum.release = 0.0;
                state.appearance.spectrum.tilt = 0.0;
                state.appearance.view.spectral_reading = reading;
                state.appearance.view.spectral_ring_attack = 0.0;
                state.appearance.view.spectral_ring_release = 0.0;
                state.appearance.view.spectral_ring_gate = 128.0 / 255.0;
                state.appearance.view.spectral_ring_hysteresis = 26.0 / 255.0;
                let cfg = state.appearance.spectrum;
                let view = state.appearance.view.clone();
                let b = ((81.0 - SPECTRUM_MIN_MIDI) * BINS_PER_SEMITONE as f32) as usize;
                let pitch = bucket_pitch(b);
                let layout = octave_layout(1, pitch, 0, 1.0, 0.0);
                let cents = (pitch * 100.0).rem_euclid(1200.0);
                let frames = n.div_ceil(384) * 384; // first due hop at or beyond a full FFT window
                let unit: Vec<f32> = (0..frames)
                    .map(|i| (std::f64::consts::TAU * 880.0 * i as f64 / 48000.0).sin() as f32)
                    .collect();
                let measure = |gain: f32| {
                    let mut spectrum = crate::AudioSpectrum::default();
                    let samples: Vec<f32> =
                        unit.iter().flat_map(|&x| [x * gain, -x * gain]).collect();
                    spectrum.push_samples(&samples, 2, SR, 1.0, &cfg);
                    let raw = *spectrum.display(1.0).unwrap();
                    let winner =
                        raw.iter().enumerate().max_by(|a, b| a.1.total_cmp(b.1)).unwrap().0;
                    let grid = if reading == SpectralReading::Fold {
                        *spectrum.folded(1.0, view.spectral_width).unwrap()
                    } else {
                        raw
                    };
                    (grid, winner)
                };
                let initial_scene = scene_of_at(&mut state, 0.0);
                let node = initial_scene
                    .nodes
                    .iter()
                    .min_by(|a, b| (a.cents - 900.0).abs().total_cmp(&(b.cents - 900.0).abs()))
                    .unwrap();
                assert_eq!(node.activation, 0.0);
                let node_cents = node.cents;
                let node_layout = initial_scene.octave_layout;
                if calibrate {
                    for held in [false, true] {
                        let admits = |bits| {
                            let (grid, _) = measure(f32::from_bits(bits));
                            let mut paint = SpectralPaint::new(&view, cfg.spectrogram_gradient);
                            RingLevels::default().fill(&mut paint, &cfg, Some(&grid), &view, 1.0);
                            if held {
                                paint.gate = (paint.gate - paint.hysteresis).max(1.0 / 255.0);
                            }
                            RingGate::new(&paint).draws(&node_layout, node_cents)
                        };
                        let (mut low, mut high) = (0.000001f32.to_bits(), 1.0f32.to_bits());
                        while high - low > 1 {
                            let mid = low + (high - low) / 2;
                            if admits(mid) {
                                high = mid;
                            } else {
                                low = mid;
                            }
                        }
                        assert!(
                            !admits(low) && admits(high),
                            "actual idle node threshold not reached"
                        );
                        gains.extend([low, high]);
                    }
                }
                let [strict_lo, strict_hi, held_lo, held_hi] =
                    gains[index..index + 4].try_into().unwrap();
                index += 4;
                let middle =
                    ((f32::from_bits(strict_hi) + f32::from_bits(held_hi)) * 0.5).to_bits();
                let phases = [0, strict_lo, strict_hi, middle, held_hi, held_lo, 0, 0, 0, 0];
                state.runtime.frame_params.fade_time = 1.0;
                state.appearance.view.fade_shape = 0.0;
                let mut levels = RingLevels::default();
                let mut fade = RingFade::default();
                let env = Envelope { attack_time: 0.1, fade_time: 0.1, shape: 0.0 };
                for (phase, gain) in phases.into_iter().enumerate() {
                    let (grid, winner) = measure(f32::from_bits(gain));
                    // One carried PictureState, full configured window at every stage,
                    // strictly increasing sample and frame clocks; no reset between stages.
                    let scene_now = (phase + 1) as f64 * frames as f64 / 48000.0;
                    let samples: Vec<f32> = unit
                        .iter()
                        .flat_map(|&x| [x * f32::from_bits(gain), -x * f32::from_bits(gain)])
                        .collect();
                    state.runtime.spectrum.push_samples(&samples, 2, SR, scene_now, &cfg);
                    let scene = scene_of_at(&mut state, scene_now);
                    let scene_gate = RingGate::new(&scene.spectral);
                    for (j, node) in scene.nodes.iter().enumerate() {
                        writeln!(
                            nodes,
                            "{n},{},{reading:?},{phase},{j},{:.9},{:.9},{:.9}",
                            tapers.count(),
                            node.cents,
                            node.audio_ring,
                            scene_gate.peak(&scene.octave_layout, node.cents)
                        )
                        .unwrap();
                    }
                    let now = phase as f64 * 0.02;
                    let mut paint = SpectralPaint::new(&view, cfg.spectrogram_gradient);
                    levels.fill(&mut paint, &cfg, Some(&grid), &view, now);
                    let gate = RingGate::new(&paint);
                    fade.advance(&gate, &env, now);
                    let level = fade.level(&layout, cents);
                    writeln!(
                        out,
                        "{n},{},{reading:?},{phase},{gain},{winner},{},{:.9},{level:.9}",
                        tapers.count(),
                        paint.levels[b],
                        gate.peak(&layout, cents)
                    )
                    .unwrap();
                }
            }
        }
    }
    if calibrate {
        std::fs::write(path, gains.iter().map(|x| format!("{x}\n")).collect::<String>()).unwrap();
    }
}
