use super::*;

/// Restore the thread's override even if an assertion panics.
struct SplitOverride(Option<bool>);

impl SplitOverride {
    fn set(split: Option<bool>) -> Self {
        let old = STAR_SPLIT_OVERRIDE.replace(split);
        Self(old)
    }
}

impl Drop for SplitOverride {
    fn drop(&mut self) {
        STAR_SPLIT_OVERRIDE.set(self.0);
    }
}

fn star_fixture(size: [u32; 2], origin: egui::Pos2) -> SpectrogramCallback {
    let mut cb = refracted_fixture();
    cb.rect = egui::Rect::from_min_size(origin, egui::vec2(size[0] as f32, size[1] as f32));
    relay_quad(&mut cb, 12);
    for vertex in &mut cb.vertices {
        vertex.pos[0] += origin.x;
        vertex.pos[1] += origin.y;
    }
    cb.read.rows = size[1];
    let atmosphere = cb.atmosphere.as_mut().unwrap();
    atmosphere.region = cb.rect;
    atmosphere.now = 3.25;
    atmosphere.settings.cloud_style = harmonigraph_scene::CloudStyle::Stars;
    atmosphere.settings.cloud_depth = 1.0;
    cb
}

fn frame_at_ppp(
    device: &wgpu::Device,
    queue: &wgpu::Queue,
    resources: &mut CallbackResources,
    cb: &SpectrogramCallback,
    ppp: f32,
) -> Vec<u8> {
    let size = [(cb.rect.max.x * ppp).ceil() as u32 + 1, (cb.rect.max.y * ppp).ceil() as u32 + 1];
    let screen = ScreenDescriptor { size_in_pixels: size, pixels_per_point: ppp };
    let mut encoder = device.create_command_encoder(&Default::default());
    let bufs = cb.prepare(device, queue, &screen, &mut encoder, resources);
    queue.submit(bufs.into_iter().chain([encoder.finish()]));
    let texture =
        render_to_texture(device, queue, size, cb.target_format, wgpu::Color::BLACK, |pass| {
            let info = egui::PaintCallbackInfo {
                viewport: cb.rect,
                clip_rect: cb.rect,
                pixels_per_point: ppp,
                screen_size_px: size,
            };
            // Match egui's scissor: paint() keeps it while restoring the
            // whole-surface viewport for screen-space vertices.
            let clip = info.clip_rect_in_pixels();
            pass.set_scissor_rect(
                clip.left_px as u32,
                clip.top_px as u32,
                clip.width_px as u32,
                clip.height_px as u32,
            );
            cb.paint(info, pass, resources);
        });
    readback(device, queue, &texture, size)
}

fn target(resources: &CallbackResources) -> &atmosphere::Targets {
    resources.get::<SpectrogramResources>().unwrap().panes.get(0).unwrap().cloud.as_ref().unwrap()
}

#[test]
fn star_split_matches_native_at_fractional_scale_with_and_without_memory() {
    let Some((device, queue)) = headless_device() else { return };
    const PPP: f32 = 1.25;
    let origin = egui::pos2(7.2, 11.6);
    let mut last_frames = Vec::new();
    for (memory, jitter) in [0.0, 0.5, 1.0]
        .into_iter()
        .flat_map(|jitter| [false, true].into_iter().map(move |memory| (memory, jitter)))
    {
        let mut cb = star_fixture([128, 96], origin);
        cb.atmosphere.as_mut().unwrap().region = egui::Rect::from_min_max(
            egui::pos2(origin.x + 13.0, origin.y + 9.0),
            egui::pos2(origin.x + 117.0, origin.y + 87.0),
        );
        let settings = &mut cb.atmosphere.as_mut().unwrap().settings;
        settings.cloud_depth = 0.65;
        settings.star_jitter = jitter;
        if memory {
            settings.color_pickup = 0.6;
            settings.color_release = 0.6;
        }
        let mut native = CallbackResources::default();
        let mut changing = CallbackResources::default();
        let mut first: Option<Vec<u8>> = None;
        for step in 0u64..4 {
            cb.pass_nr = step;
            cb.atmosphere.as_mut().unwrap().now = 3.25 + step as f64 * 0.25;
            if step == 1 {
                cb.grid.fill(80);
            } else if step == 2 {
                cb.grid.fill(220);
            }
            if step == 3 {
                // Exercise an interior far-pass scissor, including its
                // fractional right/bottom edges, rather than only its gate.
                let region = cb.atmosphere.as_ref().unwrap().region;
                for vertex in &mut cb.vertices {
                    vertex.pos[0] = vertex.pos[0].clamp(region.left(), region.right());
                    vertex.pos[1] = vertex.pos[1].clamp(region.top(), region.bottom());
                }
                let settings = &mut cb.atmosphere.as_mut().unwrap().settings;
                settings.star_fringe = harmonigraph_scene::STAR_FRINGE_MAX;
                settings.star_defocus = harmonigraph_scene::STAR_DEFOCUS_MAX;
                settings.star_size_min = harmonigraph_scene::STAR_SIZE_MIN;
                settings.star_size_max = harmonigraph_scene::STAR_SIZE_MAX;
                settings.cloud_depth = 1.0;
            }
            let a = {
                let _override = SplitOverride::set(Some(false));
                frame_at_ppp(&device, &queue, &mut native, &cb, PPP)
            };
            let split = step != 1;
            let b = {
                let _override = SplitOverride::set(Some(split));
                frame_at_ppp(&device, &queue, &mut changing, &cb, PPP)
            };
            assert!(target(&native).tone_size().is_none());
            assert_eq!(target(&changing).tone_size().is_some(), split);
            assert_eq!(target(&changing).memory_size().is_some(), memory);
            assert_eq!(target(&changing).halo_size(), Some([80, 60]));
            let worst = a.iter().zip(&b).map(|(x, y)| x.abs_diff(*y)).max().unwrap();
            // Bound both half-float rounding and aggregate error so a
            // wider spatial mismatch still fails.
            let error: usize = a.iter().zip(&b).map(|(x, y)| x.abs_diff(*y) as usize).sum();
            let mean = error as f64 / a.len() as f64;
            assert!(
                worst <= 2 && mean <= 0.01,
                "memory={memory}, jitter={jitter}, step={step}: split differs by max {worst}, mean {mean}/255"
            );
            if let Some(previous) = first.as_ref() {
                let changed = a.iter().zip(previous).filter(|(x, y)| x.abs_diff(**y) > 4).count();
                assert!(changed > a.len() / 100, "memory={memory}: fixture did not evolve");
            } else {
                let floor = cb.shades.lut[0];
                let side = (cb.rect.max.x * PPP).ceil() as usize + 1;
                let lit = (28..100)
                    .flat_map(|y| (30..140).map(move |x| (x, y)))
                    .filter(|&(x, y)| {
                        let i = (y * side + x) * 4;
                        a[i..i + 3] != floor[..3]
                    })
                    .count();
                assert!(lit > 1000, "memory={memory}: clipped pane has no visible light");
                first = Some(a);
            }
            if step == 2 {
                last_frames.push(b);
            }
        }
    }
    for pair in last_frames.chunks_exact(2) {
        let memory_effect =
            pair[0].iter().zip(&pair[1]).filter(|(a, b)| a.abs_diff(**b) > 4).count();
        assert!(memory_effect > pair[0].len() / 100, "memory fixture did not retain color");
    }
}

#[test]
fn star_split_activates_at_the_real_size_threshold() {
    let Some((device, queue)) = headless_device() else { return };
    let _override = SplitOverride::set(None);
    let below = star_fixture([2560, 1439], egui::Pos2::ZERO);
    let atmosphere = below.atmosphere.unwrap();
    assert_eq!(atmosphere::tone_size([2560, 1439], 1.0, atmosphere, 1.0), None);
    assert_eq!(atmosphere::tone_size([2560, 1440], 1.0, atmosphere, 1.0), Some([2560, 1440]));
    let mut cb = star_fixture([2560, 1441], egui::Pos2::ZERO);
    cb.target_format = wgpu::TextureFormat::Rgba8UnormSrgb;
    let mut native = CallbackResources::default();
    let mut automatic = CallbackResources::default();
    let reference = {
        let _override = SplitOverride::set(Some(false));
        frame_at_ppp(&device, &queue, &mut native, &cb, 1.0)
    };
    let actual = frame_at_ppp(&device, &queue, &mut automatic, &cb, 1.0);
    assert_eq!(target(&automatic).tone_size(), Some([2560, 1441]));
    assert_eq!(
        target(&automatic).encoded_passes.load(Ordering::Relaxed),
        target(&native).encoded_passes.load(Ordering::Relaxed) + 1,
        "the threshold allocated a far target but did not draw the far pass"
    );
    let worst = actual.iter().zip(&reference).map(|(x, y)| x.abs_diff(*y)).max().unwrap();
    assert!(worst <= 1, "automatic split differs from the native walk by {worst}/255");

    // A large combined pane with a small Stars region must stay unsplit.
    // Production's measured mesh occupies the same region as its backdrop.
    cb.pass_nr += 1;
    for vertex in &mut cb.vertices {
        vertex.pos[1] = vertex.pos[1] * 0.5 + 720.5;
    }
    cb.atmosphere.as_mut().unwrap().region.min.y = 720.5;
    let partial = frame_at_ppp(&device, &queue, &mut automatic, &cb, 1.0);
    assert!(target(&automatic).tone_size().is_none());
    let reference = {
        let _override = SplitOverride::set(Some(false));
        frame_at_ppp(&device, &queue, &mut native, &cb, 1.0)
    };
    assert_eq!(partial, reference);
}

// An independent native nine-neighbor response checks the decomposition; the
// split/unsplit test alone would compare two consumers of the same halo bug.
thread_local! {
    static STAR_REFERENCE: std::cell::Cell<u8> = const { std::cell::Cell::new(0) };
}

pub(super) fn reference_source() -> Option<String> {
    let mode = STAR_REFERENCE.get();
    let mut source = SPECTROGRAM_SRC.to_owned();
    if STAR_HALO_DIVISOR.get() == 1 {
        source = source
            .replace("const STAR_HALO_SCALE: f32 = 0.5;", "const STAR_HALO_SCALE: f32 = 1.0;");
    }
    if mode == 0 {
        return (STAR_HALO_DIVISOR.get() != 2).then_some(source);
    }
    let read = "var slice = star_texel(s, f, index, false);\n        slice += textureSampleLevel(star_halos, cloud_sampler, pt / cloud.size, i32(k), 0.0);";
    assert_eq!(SPECTROGRAM_SRC.matches(read).count(), 1, "the Stars read moved");
    let core = "let slice = star_texel(s, f, index, false);";
    let full = r#"
        var slice = vec4<f32>(0.0);
        for (var y = -1; y <= 1; y += 1) {
            for (var x = -1; x <= 1; x += 1) {
                slice += reference_star(s, f - vec2<f32>(f32(x), f32(y)), index + y * s.grid.x + x);
            }
        }
    "#;
    Some(
        source.replace(read, if mode == 1 { full } else { core })
            + r#"
// The original full response, without any compact-core subtraction.
fn reference_star(s: StarSlice, f: vec2<f32>, index: i32) -> vec4<f32> {
    let t = textureLoad(star_atlas, atlas_texel(index), 0);
    if t.w == 0u { return vec4<f32>(0.0); }
    let dist = length(f - vec2<f32>(bitcast<f32>(t.x), bitcast<f32>(t.y))) * s.cell;
    let reach = 1.2 * s.cell;
    if dist >= reach { return vec4<f32>(0.0); }
    let rgb = vec3<f32>(vec3<u32>(t.z >> 20u, t.z >> 10u, t.z) & vec3<u32>(1023u)) / 1023.0;
    let shape = unpack2x16float(t.w);
    let d = dist * shape.x;
    var cover = exp(-0.5 * d * d);
    if s.fringe > 0.0 { cover += s.fringe * exp(-0.4 * d); }
    cover = min(cover, 1.0) * shape.y * (1.0 - smoothstep(0.7 * reach, reach, dist));
    return vec4<f32>(rgb * cover, cover);
}
"#,
    )
}

struct HaloOverride {
    divisor: u32,
    reference: u8,
}
impl HaloOverride {
    fn set(divisor: u32, reference: u8) -> Self {
        Self {
            divisor: STAR_HALO_DIVISOR.replace(divisor),
            reference: STAR_REFERENCE.replace(reference),
        }
    }
}
impl Drop for HaloOverride {
    fn drop(&mut self) {
        STAR_HALO_DIVISOR.set(self.divisor);
        STAR_REFERENCE.set(self.reference);
    }
}

#[test]
fn separate_halos_reconstruct_the_wide_response_including_gaussian_tails() {
    let Some((device, queue)) = headless_device() else { return };
    let _split = SplitOverride::set(Some(false));
    let mut cb = star_fixture([385, 217], egui::Pos2::ZERO);
    for jitter in [0.0, 0.5, 1.0] {
        for fringe in [0.0, harmonigraph_scene::STAR_FRINGE_MAX] {
            let settings = &mut cb.atmosphere.as_mut().unwrap().settings;
            settings.star_jitter = jitter;
            settings.star_fringe = fringe;
            let mut frames = Vec::new();
            for reference in [0, 1, 2] {
                // Native halo sampling isolates the split's algebra from the
                // intentional interpolation of the production half-size image.
                let _halo = HaloOverride::set(1, reference);
                let mut resources = CallbackResources::default();
                frames.push(frame_at_ppp(&device, &queue, &mut resources, &cb, 1.0));
                assert_eq!(target(&resources).halo_size(), Some([385, 217]));
            }
            let differences: Vec<_> =
                frames[0].iter().zip(&frames[1]).map(|(a, b)| a.abs_diff(*b)).collect();
            let max = *differences.iter().max().unwrap();
            let mean =
                differences.iter().map(|&d| f64::from(d)).sum::<f64>() / differences.len() as f64;
            assert!(
                max <= 1 && mean <= 0.03,
                "jitter={jitter}, fringe={fringe}: reconstruction max={max}, mean={mean}"
            );
            let halo_pixels = frames[0]
                .chunks_exact(4)
                .zip(frames[2].chunks_exact(4))
                .filter(|(a, b)| a[..3].iter().zip(&b[..3]).any(|(x, y)| x.abs_diff(*y) > 4))
                .count();
            assert!(
                halo_pixels > 500,
                "jitter={jitter}, fringe={fringe}: halo witness only {halo_pixels} pixels"
            );
        }
    }
}
