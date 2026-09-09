// Evaluation-only private observation, never called from the timed public path.
// Independent direct f64 DFT of selected bins, using the SAME f32-windowed
// samples. This checks transform, bin ordering and summed taper power, without
// treating agreement between two FFTs as an independent reference.
pub fn reference_error(bank: &ChannelBank) -> (f64, f64) {
    let mut max_scaled = 0.0f64;
    let mut max_abs = 0.0f64;
    for a in &bank.per_channel {
        let n = a.fft_size;
        let peak = a.bin_power.iter().copied().fold(0.0f32, f32::max) as f64;
        for bin in [2, 3, 7, 37, 75, 113, 301, 554, 555, n / 4, n / 2 - 2] {
            let mut power = 0.0;
            for k in 0..a.taper_count {
                let (mut re, mut im) = (0.0, 0.0);
                for i in 0..n {
                    let x = (a.ring[(a.write + i) % n] * a.tapers[k * n + i]) as f64;
                    let phase = std::f64::consts::TAU * bin as f64 * i as f64 / n as f64;
                    re += x * phase.cos();
                    im -= x * phase.sin();
                }
                power += re * re + im * im;
            }
            let error = (power - a.bin_power[bin] as f64).abs();
            max_abs = max_abs.max(error * a.norm_power as f64);
            max_scaled = max_scaled.max(error / peak.max(1e-30));
        }
    }
    (max_scaled, max_abs)
}
