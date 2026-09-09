// One-off fixture generation, compiled as an offline test module by setup.py.
#[path = "/tmp/realfft-adoption/evaluation/baseline/src/golden_audio.rs"]
mod golden_audio;

#[test]
#[ignore]
fn adoption_fixtures() {
    use harmonigraph_scene::SpectralReading;
    use harmonigraph_take::{Header, WavWriter, Writer};
    use harmonigraph_ui::{PictureState, SpectrumTapers, SpectrumWindow};
    let dir = std::path::Path::new("/tmp/realfft-adoption/fixtures");
    std::fs::create_dir_all(dir).unwrap();
    let mono = golden_audio::probe_audio();
    let mut stereo = Vec::new();
    for (i, &x) in mono.iter().enumerate() {
        stereo.extend([x, mono[(i + 97) % mono.len()] * 0.8]);
    }
    stereo.resize(stereo.len() + 2 * 57600, 0.0); // 1.2 s silence reaches release and fade.
    let mut wav = WavWriter::create(dir.join("audio.wav"), 48000.0, 2).unwrap();
    wav.write(&stereo).unwrap();
    wav.finish().unwrap();
    for (name, window, tapers) in [
        ("default", SpectrumWindow::Balanced, SpectrumTapers::One),
        ("largest", SpectrumWindow::Precise, SpectrumTapers::Five),
    ] {
        for (reading_name, reading) in
            [("fold", SpectralReading::Fold), ("spectrum", SpectralReading::Spectrum)]
        {
            let mut state = PictureState::new(egui_wgpu::wgpu::TextureFormat::Rgba8Unorm);
            state.appearance.spectrum.window = window;
            state.appearance.spectrum.tapers = tapers;
            state.appearance.spectrum.roll_seconds = 1.5;
            state.appearance.spectrum.show_roll = false;
            state.appearance.view.spectral_ring_width = 0.1;
            state.appearance.view.spectral_reading = reading;
            state.appearance.view.spectral_ring_gate = 0.5;
            state.appearance.view.spectral_ring_hysteresis = 0.1;
            let header =
                Header { appearance: Some(state.appearance.serialize()), ..Default::default() };
            Writer::create(dir.join(format!("{name}-{reading_name}.take")), &header)
                .unwrap()
                .flush()
                .unwrap();
        }
    }
}

#[test]
#[ignore]
fn adoption_boundary_frames() {
    use harmonigraph_scene::SpectralReading;
    use harmonigraph_ui::{draw_pane, Layout, PictureState, SpectrumTapers, SpectrumWindow};
    use std::io::Write;
    let dir = std::env::var("ADOPTION_OUTPUT").unwrap();
    let gains: Vec<u32> = std::fs::read_to_string("/tmp/realfft-adoption/gains.txt")
        .unwrap()
        .lines()
        .map(|x| x.parse().unwrap())
        .collect();
    for (name, n, window, tapers, index, reading) in [
        (
            "boundary-8192-3-fold",
            8192usize,
            SpectrumWindow::Balanced,
            SpectrumTapers::Three,
            8 * 4,
            SpectralReading::Fold,
        ),
        (
            "boundary-16384-5-fold",
            16384usize,
            SpectrumWindow::Precise,
            SpectrumTapers::Five,
            16 * 4,
            SpectralReading::Fold,
        ),
        (
            "boundary-16384-1-spectrum",
            16384usize,
            SpectrumWindow::Precise,
            SpectrumTapers::One,
            13 * 4,
            SpectralReading::Spectrum,
        ),
    ] {
        let [strict_lo, strict_hi, held_lo, held_hi] = gains[index..index + 4].try_into().unwrap();
        let middle = ((f32::from_bits(strict_hi) + f32::from_bits(held_hi)) * 0.5).to_bits();
        let phases = [0, strict_lo, strict_hi, middle, held_hi, held_lo, 0, 0, 0, 0];
        let frames = n.div_ceil(384) * 384;
        let unit: Vec<f32> = (0..frames)
            .map(|i| (std::f64::consts::TAU * 880.0 * i as f64 / 48000.0).sin() as f32)
            .collect();
        let mut renderer =
            crate::frames::Renderer::new([640, 400]).expect("GPU required for comparison");
        let ctx = egui::Context::default();
        harmonigraph_ui::theme::apply_theme(&ctx);
        let mut state = PictureState::new(crate::frames::FORMAT);
        state.appearance.spectrum.window = window;
        state.appearance.spectrum.tapers = tapers;
        state.appearance.spectrum.attack = 0.0;
        state.appearance.spectrum.release = 0.0;
        state.appearance.spectrum.tilt = 0.0;
        state.appearance.view.spectral_ring_width = 0.1;
        state.appearance.view.spectral_reading = reading;
        state.appearance.view.spectral_ring_attack = 0.0;
        state.appearance.view.spectral_ring_release = 0.0;
        state.appearance.view.spectral_ring_gate = 128.0 / 255.0;
        state.appearance.view.spectral_ring_hysteresis = 26.0 / 255.0;
        state.appearance.view.fade_shape = 0.0;
        state.runtime.frame_params.fade_time = 1.0;
        let layout = Layout::preset("lattice").unwrap();
        let screen = egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(640.0, 400.0));
        let placements = layout.resolve(screen.size());
        let bg =
            egui::Color32::from_rgb(layout.background.0, layout.background.1, layout.background.2);
        let mut out = std::fs::File::create(format!("{dir}/{name}.rgba")).unwrap();
        for (phase, bits) in phases.into_iter().enumerate() {
            let now = (phase + 1) as f64 * frames as f64 / 48000.0;
            let samples: Vec<f32> = unit
                .iter()
                .flat_map(|x| [*x * f32::from_bits(bits), -*x * f32::from_bits(bits)])
                .collect();
            state.runtime.spectrum.push_samples(
                &samples,
                2,
                48000.0,
                now,
                &state.appearance.spectrum,
            );
            let output = ctx.run_ui(
                egui::RawInput {
                    screen_rect: Some(screen),
                    time: Some(now),
                    max_texture_side: Some(renderer.max_texture_side()),
                    ..Default::default()
                },
                |ui| {
                    for (surface, (pane, rect)) in placements.iter().enumerate() {
                        let mut child = ui.new_child(egui::UiBuilder::new().max_rect(*rect));
                        draw_pane(&mut child, *pane, &mut state, now, surface);
                    }
                },
            );
            let primitives = ctx.tessellate(output.shapes, 1.0);
            out.write_all(&renderer.render(&primitives, &output.textures_delta, 1.0, bg)).unwrap();
        }
    }
}
