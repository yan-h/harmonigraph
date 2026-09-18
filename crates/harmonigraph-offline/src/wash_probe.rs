//! SCRATCH — delete before finishing. Renders the wash look to PNGs.

#[cfg(test)]
mod tests {
    use crate::render::{render, Settings};
    use crate::replay::Replay;
    use harmonigraph_render::wgpu::TextureFormat;
    use harmonigraph_scene::{CloudStyle, SpectralAtmosphere};
    use harmonigraph_take::{Header, Take};
    use harmonigraph_ui::{Layout, PictureState, SpectrogramRender};

    const OUT: &str = "/Users/yan/.claude/jobs/57dd5ad2/tmp/port";
    const OLD: &str = "/Users/yan/.claude/jobs/f5be7549/tmp/port";
    const TAKE: &str = "/Users/yan/Music/Harmonigraph Takes/take-2026-09-11_03-16-17.wav";
    const START: f64 = 72.0;
    const SPAN: f32 = 8.0;

    fn shot(size: [u32; 2], ppp: f32, tune: impl Fn(&mut SpectralAtmosphere)) -> (Settings, Take) {
        let mut state = PictureState::new(TextureFormat::Rgba8Unorm);
        let cfg = &mut state.appearance.spectrum;
        cfg.show_roll = false;
        cfg.roll_fraction = 1.0;
        cfg.roll_seconds = SPAN;
        // 70 Hz .. 9 kHz, the prototype's own pane: 84 semitones, so a harmonic
        // line spacing lands where the prototype's did.
        cfg.low_midi = 37.0;
        cfg.high_midi = 121.0;
        tune(&mut cfg.atmosphere);
        state.appearance.render.spectrogram = SpectrogramRender::Scrolling;
        let take = Take {
            header: Header { appearance: Some(state.appearance.serialize()), ..Default::default() },
            ..Default::default()
        };
        let settings = Settings {
            layout: Layout::preset("spectral").unwrap(),
            size,
            pixels_per_point: ppp,
            fps: 8.0,
            start: START,
            end: START + f64::from(SPAN),
            audio_start: 0.0,
            whole_song_spectrogram: false,
        };
        (settings, take)
    }

    /// The same shot with the level window pulled so far under the noise that
    /// EVERY bucket clips to full — a genuinely flat picture, which white noise
    /// is not: its own per-bin variance is structure, and a lens over structure
    /// moves it.
    fn shot_flat(size: [u32; 2], tune: impl Fn(&mut SpectralAtmosphere)) -> (Settings, Take) {
        let mut state = PictureState::new(TextureFormat::Rgba8Unorm);
        let cfg = &mut state.appearance.spectrum;
        cfg.show_roll = false;
        cfg.roll_fraction = 1.0;
        cfg.roll_seconds = SPAN;
        cfg.low_midi = 37.0;
        cfg.high_midi = 121.0;
        cfg.floor_db = -130.0;
        cfg.ceiling_db = -110.0;
        tune(&mut cfg.atmosphere);
        state.appearance.render.spectrogram = SpectrogramRender::Scrolling;
        let take = Take {
            header: Header { appearance: Some(state.appearance.serialize()), ..Default::default() },
            ..Default::default()
        };
        let settings = Settings {
            layout: Layout::preset("spectral").unwrap(),
            size,
            pixels_per_point: 1.0,
            fps: 8.0,
            start: 0.0,
            end: f64::from(SPAN),
            audio_start: 0.0,
            whole_song_spectrogram: false,
        };
        (settings, take)
    }

    fn draw(name: &str, size: [u32; 2], ppp: f32, tune: impl Fn(&mut SpectralAtmosphere)) {
        let (settings, take) = shot(size, ppp, tune);
        let mut audio = crate::wav::read(TAKE).expect("the take");
        let mut replay = Replay::new(take.clone());
        let appearance = crate::render::appearance_for(&take, None);
        let mut last = Vec::new();
        let t = std::time::Instant::now();
        render(&mut replay, Some(&mut audio), &settings, appearance, |bytes| {
            last.clear();
            last.extend_from_slice(bytes);
            Ok(true)
        })
        .expect("a GPU");
        let path = format!("{OUT}/{name}.png");
        image::save_buffer(&path, &last, size[0], size[1], image::ColorType::Rgba8).unwrap();
        eprintln!("{name}: {:.1}s -> {path}", t.elapsed().as_secs_f32());
        // A 4x nearest-neighbour zoom on a band edge, to look for tearing.
        let (cw, ch) = (160u32, 90u32);
        let (x0, y0) = (size[0] / 3, size[1] / 3);
        let mut zoom = vec![0u8; (cw * 4 * ch * 4 * 4) as usize];
        for y in 0..ch * 4 {
            for x in 0..cw * 4 {
                let s = (((y / 4 + y0) * size[0] + x / 4 + x0) * 4) as usize;
                let d = ((y * cw * 4 + x) * 4) as usize;
                zoom[d..d + 4].copy_from_slice(&last[s..s + 4]);
            }
        }
        image::save_buffer(
            format!("{OUT}/{name}-zoom4x.png"),
            &zoom,
            cw * 4,
            ch * 4,
            image::ColorType::Rgba8,
        )
        .unwrap();
    }

    fn wash(a: &mut SpectralAtmosphere) {
        a.cloud_style = CloudStyle::Wash;
    }

    fn j1(a: &mut SpectralAtmosphere) {
        wash(a);
        a.wash_size = 0.5;
        a.wash_fuzz = 0.35;
        a.wash_ragged = 0.29;
        a.wash_pool = 0.36;
        a.wash_refract = 0.85;
        a.wash_layers = 0.5;
        a.wash_grain = 0.0;
    }

    fn j5(a: &mut SpectralAtmosphere) {
        wash(a);
        a.wash_size = 0.59;
        a.wash_fuzz = 0.45;
        a.wash_ragged = 0.29;
        a.wash_pool = 0.36;
        a.wash_refract = 0.7;
        a.wash_layers = 0.42;
        a.wash_grain = 0.5;
    }

    /// The prototype's globs are `rad 0.90` cells wide and the shader's are
    /// `WASH_RADIUS 1.18`, so matching the CELL count leaves the plugin's globs
    /// 31% too big. These three ask which number reproduces J2's picture.
    #[test]
    #[ignore = "scratch"]
    fn wash_size_match() {
        for (tag, size) in [("cells", 1.0f32), ("diameter", 0.763), ("between", 0.87)] {
            draw(&format!("size-match-{tag}"), [960, 540], 1.0, |a| {
                wash(a);
                a.wash_size = size;
            });
        }
    }

    #[test]
    #[ignore = "scratch"]
    fn wash_contact_sheet() {
        draw("plug-water", [960, 540], 1.0, |_| {});
        draw("plug-j2-default", [960, 540], 1.0, wash);
        draw("plug-j1", [960, 540], 1.0, j1);
        draw("plug-j5", [960, 540], 1.0, j5);
        draw("plug-j2-retina", [1920, 1080], 2.0, wash);
    }

    #[test]
    #[ignore = "scratch"]
    fn wash_sweeps() {
        for (i, f) in [0.0f32, 0.25, 0.5, 0.75, 1.0].iter().enumerate() {
            draw(&format!("plug-fuzz-{i}"), [960, 540], 1.0, |a| {
                wash(a);
                a.wash_fuzz = *f;
            });
        }
        for (i, s) in [0.3f32, 0.5, 1.0, 2.0].iter().enumerate() {
            draw(&format!("plug-size-{i}"), [960, 540], 1.0, |a| {
                wash(a);
                a.wash_size = *s;
            });
        }
        for (i, v) in [0.0f32, 1.0].iter().enumerate() {
            draw(&format!("plug-variety-{i}"), [960, 540], 1.0, |a| {
                wash(a);
                a.wash_variety = *v;
            });
        }
    }

    /// White noise: energy everywhere and a concentration nowhere, which is the
    /// featureless field the refraction must invent nothing over.
    fn flat_audio() -> crate::wav::Audio {
        let mut seed = 0x5eed_1234u32;
        let samples = (0..(48_000.0 * (f64::from(SPAN) + 2.0)) as usize)
            .map(|_| {
                seed = seed.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
                ((seed >> 8) as f32 / (1 << 23) as f32 - 1.0) * 0.2
            })
            .collect();
        crate::wav::Audio::from_samples(48_000.0, samples, 1)
    }

    fn render_flat(name: &str, size: [u32; 2], tune: impl Fn(&mut SpectralAtmosphere)) -> Vec<u8> {
        let (mut settings, take) = shot_flat(size, tune);
        settings.start = 0.0;
        settings.end = f64::from(SPAN);
        let mut audio = flat_audio();
        let mut replay = Replay::new(take.clone());
        let appearance = crate::render::appearance_for(&take, None);
        let mut last = Vec::new();
        render(&mut replay, Some(&mut audio), &settings, appearance, |bytes| {
            last.clear();
            last.extend_from_slice(bytes);
            Ok(true)
        })
        .expect("a GPU");
        image::save_buffer(
            format!("{OUT}/{name}.png"),
            &last,
            size[0],
            size[1],
            image::ColorType::Rgba8,
        )
        .unwrap();
        last
    }

    /// The layer over a featureless field, and the refraction dial over it.
    #[test]
    #[ignore = "scratch"]
    fn wash_over_a_flat_field() {
        let a = render_flat("plug-flat-r0", [640, 360], |a| {
            wash(a);
            a.wash_refract = 0.0;
        });
        let b = render_flat("plug-flat-r1", [640, 360], |a| {
            wash(a);
            a.wash_refract = 1.0;
        });
        let moved = a
            .chunks_exact(4)
            .zip(b.chunks_exact(4))
            .filter(|(p, q)| (0..3).any(|c| p[c] != q[c]))
            .count();
        eprintln!("flat field, refraction 0 -> 1: {moved} of {} pixels differ", a.len() / 4);
        render_flat("plug-flat-depth-half", [640, 360], |a| {
            wash(a);
            a.cloud_depth = 0.5;
        });
        render_flat("plug-depth-half", [640, 360], wash);
    }

    /// Consecutive frames with the field stirring, at low Fuzz where a hard
    /// winner-take-all edge would shimmer worst.
    /// Milliseconds a frame takes end to end with one texture selected, timed
    /// the way `what_the_heatmap_costs_a_frame` times its own: past a warm-up,
    /// inside a real render, and read as a DIFFERENCE against the same frame
    /// with the layer off.
    fn layer_ms(size: [u32; 2], ppp: f32, tune: impl Fn(&mut SpectralAtmosphere)) -> f64 {
        let (mut settings, take) = shot(size, ppp, tune);
        settings.fps = 60.0;
        settings.end = START + 4.0;
        let mut audio = crate::wav::read(TAKE).expect("the take");
        let mut replay = Replay::new(take.clone());
        let appearance = crate::render::appearance_for(&take, None);
        let (mut seen, mut frames) = (0u64, 0u64);
        let (mut first, mut last) = (None, None);
        render(&mut replay, Some(&mut audio), &settings, appearance, |_| {
            seen += 1;
            if seen <= 16 {
                return Ok(true);
            }
            let now = std::time::Instant::now();
            if first.is_none() {
                first = Some(now);
            } else {
                frames += 1;
            }
            last = Some(now);
            Ok(true)
        })
        .expect("a GPU");
        let span: std::time::Duration = last.unwrap().duration_since(first.unwrap());
        span.as_secs_f64() * 1000.0 / frames as f64
    }

    /// How far the one-walk front differs from the exhaustive two-walk one,
    /// against renders of the same shots saved before the change.
    #[test]
    #[ignore = "scratch"]
    fn wash_against_two_pass() {
        for (a, b) in [("twopass-j2", "plug-j2-default"), ("twopass-fuzz0", "plug-fuzz-0")] {
            let old = image::open(format!("{OLD}/{a}.png")).unwrap().to_rgba8();
            let new = image::open(format!("{OLD}/{b}.png")).unwrap().to_rgba8();
            let n = old.len() / 4;
            let count = |t: u8| {
                old.chunks_exact(4)
                    .zip(new.chunks_exact(4))
                    .filter(|(p, q)| (0..3).any(|c| p[c].abs_diff(q[c]) > t))
                    .count() as f32
                    * 100.0
                    / n as f32
            };
            let worst = old
                .chunks_exact(4)
                .zip(new.chunks_exact(4))
                .map(|(p, q)| (0..3).map(|c| p[c].abs_diff(q[c])).max().unwrap())
                .max()
                .unwrap();
            eprintln!("{a} vs {b}: >4 {:.2}%  >16 {:.3}%  worst {worst}", count(4), count(16));
        }
    }

    #[test]
    #[ignore = "scratch"]
    fn wash_cost() {
        const REPS: usize = 5;
        for (size, ppp) in [([960u32, 540], 1.0f32), ([1280, 720], 2.0)] {
            let mut runs = [const { Vec::new() }; 4];
            for rep in 0..REPS {
                for step in 0..4 {
                    let slot = (rep + step) % 4;
                    runs[slot].push(match slot {
                        0 => layer_ms(size, ppp, |a| a.cloud_depth = 0.0),
                        1 => layer_ms(size, ppp, |_| {}),
                        2 => layer_ms(size, ppp, wash),
                        _ => layer_ms(size, ppp, |a| {
                            wash(a);
                            a.wash_size = 0.25;
                            a.wash_variety = 1.0;
                            a.wash_layers = 1.0;
                            a.wash_wander = 1.0;
                        }),
                    });
                }
            }
            let mid = |v: &mut Vec<f64>| {
                v.sort_by(f64::total_cmp);
                v[v.len() / 2]
            };
            let off = mid(&mut runs[0]);
            eprintln!(
                "{}x{} @{ppp}x   off {off:.2}   water {:.2} (+{:.2})   wash {:.2} (+{:.2})   \
                 wash worst {:.2} (+{:.2})",
                size[0],
                size[1],
                mid(&mut runs[1]),
                mid(&mut runs[1]) - off,
                mid(&mut runs[2]),
                mid(&mut runs[2]) - off,
                mid(&mut runs[3]),
                mid(&mut runs[3]) - off,
            );
        }
    }

    #[test]
    #[ignore = "scratch"]
    fn wash_motion() {
        let mut last: Vec<Vec<u8>> = Vec::new();
        for (tag, wander, fuzz) in
            [("still", 0.0f32, 0.1f32), ("wander", 1.0, 0.1), ("wander-fuzzy", 1.0, 1.0)]
        {
            let (mut settings, take) = shot([640, 360], 1.0, |a| {
                wash(a);
                a.wash_fuzz = fuzz;
                a.wash_wander = wander;
                a.cloud_speed = 6.0;
            });
            settings.fps = 24.0;
            let mut audio = flat_audio();
            settings.start = 0.0;
            settings.end = f64::from(SPAN);
            let mut replay = Replay::new(take.clone());
            let appearance = crate::render::appearance_for(&take, None);
            let mut frames = Vec::new();
            render(&mut replay, Some(&mut audio), &settings, appearance, |bytes| {
                frames.push(bytes.to_vec());
                Ok(true)
            })
            .expect("a GPU");
            let n = frames.len();
            // How much of the pane changes between neighbouring frames, and how
            // much of that is a step bigger than a feathered edge could make.
            for k in [n - 4, n - 3, n - 2] {
                let (a, b) = (&frames[k], &frames[k + 1]);
                let count = |t: u8| {
                    a.chunks_exact(4)
                        .zip(b.chunks_exact(4))
                        .filter(|(p, q)| (0..3).any(|c| p[c].abs_diff(q[c]) > t))
                        .count() as f32
                        * 100.0
                        / (640.0 * 360.0)
                };
                eprintln!(
                    "{tag} {k}->{}: moved {:.2}%  stepped>16 {:.3}%  stepped>48 {:.4}%",
                    k + 1,
                    count(4),
                    count(16),
                    count(48),
                );
            }
            image::save_buffer(
                format!("{OUT}/plug-motion-{tag}.png"),
                &frames[n - 1],
                640,
                360,
                image::ColorType::Rgba8,
            )
            .unwrap();
            last.push(frames.pop().unwrap());
        }
        // How far the wander has carried the field by the end of the run,
        // against the same run with it held — the frame-to-frame figures above
        // cannot see a movement this slow.
        let moved = last[0]
            .chunks_exact(4)
            .zip(last[1].chunks_exact(4))
            .filter(|(p, q)| (0..3).any(|c| p[c].abs_diff(q[c]) > 4))
            .count() as f32
            * 100.0
            / (640.0 * 360.0);
        eprintln!("wander against still after 8 s: {moved:.1}% of the pane");
    }
}
