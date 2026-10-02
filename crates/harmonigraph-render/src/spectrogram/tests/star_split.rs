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
        let mut cb = star_fixture([129, 97], origin);
        cb.atmosphere.as_mut().unwrap().region = egui::Rect::from_min_max(
            egui::pos2(origin.x + 13.0, origin.y + 9.0),
            egui::pos2(origin.x + 117.0, origin.y + 87.0),
        );
        let settings = &mut cb.atmosphere.as_mut().unwrap().settings;
        settings.cloud_depth = 0.65;
        settings.stars.star_jitter = jitter;
        if memory {
            settings.color_pickup = 0.6;
            settings.color_release = 0.6;
        }
        let mut native = CallbackResources::default();
        let mut changing = CallbackResources::default();
        let mut first: Option<Vec<u8>> = None;
        for step in 0u64..5 {
            cb.pass_nr = step;
            use harmonigraph_scene::StarHaloProfile::Uniform;
            let (profile, resolution) = [
                (Uniform, 0.5),
                (Uniform, 0.25),
                (Uniform, 1.0 / 3.0),
                (Uniform, 0.6),
                (Uniform, 1.0),
            ][step as usize];
            cb.atmosphere.as_mut().unwrap().settings.stars.star_halo_profile = profile;
            cb.atmosphere.as_mut().unwrap().settings.stars.star_halo_resolution = resolution;
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
                // Half solid in a broad glow, short of a flat wash.
                settings.stars.star_glow_falloff = 0.0;
                (settings.stars.star_solid_far, settings.stars.star_solid_near) = (0.5, 0.5);
                settings.stars.star_spacing_ratio_far = harmonigraph_scene::STAR_SPACING_MIN;
                settings.stars.star_spacing_ratio_near = harmonigraph_scene::STAR_SPACING_MAX;
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
            assert_eq!(
                target(&changing).halo_layout(),
                Some(atmosphere::star_halo_layout(
                    [161, 121],
                    cb.atmosphere.unwrap().settings.stars
                )),
            );
            let worst = a.iter().zip(&b).map(|(x, y)| x.abs_diff(*y)).max().unwrap();
            // Bound both half-float rounding and aggregate error so a
            // wider spatial mismatch still fails. The mean is one level of
            // half-float rounding on about 1% of channels (0.0098..0.0111):
            // a star's glow eases to nothing at its radius, so more pixels
            // carry a sliver of one than under the old cut-off fringe.
            let error: usize = a.iter().zip(&b).map(|(x, y)| x.abs_diff(*y) as usize).sum();
            let mean = error as f64 / a.len() as f64;
            assert!(
                worst <= 2 && mean <= 0.015,
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
    let mut below = star_fixture([2560, 1439], egui::Pos2::ZERO);
    below.atmosphere.as_mut().unwrap().settings.stars.star_halo_profile =
        harmonigraph_scene::StarHaloProfile::Uniform;
    let atmosphere = below.atmosphere.unwrap();
    assert_eq!(atmosphere::tone_size([2560, 1439], 1.0, atmosphere, 1.0), None);
    assert_eq!(atmosphere::tone_size([2560, 1440], 1.0, atmosphere, 1.0), Some([2560, 1440]));
    let mut cb = star_fixture([2560, 1441], egui::Pos2::ZERO);
    cb.atmosphere.as_mut().unwrap().settings.stars.star_halo_profile =
        harmonigraph_scene::StarHaloProfile::Uniform;
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
    if mode == 0 {
        return None;
    }
    let source = SPECTROGRAM_SRC.to_owned();
    if mode == 4 {
        // Every whole-star read, 2x2 and 1x1 alike, becomes a 3x3 one.
        let core = "slice = star_far_texel(s, f, index);";
        assert_eq!(source.matches(core).count(), 1, "the 1x1 read moved");
        let source = source.replace(core, "slice = star_far_gather(s, r);");
        let start = source.find("fn star_far_gather(").unwrap();
        let end = source[start..].find("\nfn star_layers(").unwrap() + start;
        return Some(format!(
            "{}{}{}",
            &source[..start],
            r#"
fn star_far_gather(s: StarSlice, r: vec2<f32>) -> vec4<f32> {
    let o = floor(r);
    let f = r - o;
    let local = vec2<i32>(o) - vec2<i32>(floor(s.offset)) - s.origin;
    let index = s.base + local.y * s.grid.x + local.x;
    var result = vec4<f32>(0.0);
    for (var y = -1; y <= 1; y += 1) {
        for (var x = -1; x <= 1; x += 1) {
            result += star_far_texel(s, f - vec2<f32>(f32(x), f32(y)), index + y * s.grid.x + x);
        }
    }
    return result;
}
"#,
            &source[end..]
        ));
    }
    if mode == 3 {
        let read = "slice += star_halo_at(pt, k);";
        assert_eq!(source.matches(read).count(), 1, "the Stars read moved");
        return Some(source.replace(
            read,
            "slice += textureSampleLevel(star_halos, cloud_sampler, pt / cloud.size, i32(k), 0.0);",
        ));
    }
    let read = "slice = star_texel(s, f, index, false);\n            slice += star_halo_at(pt, k);";
    assert_eq!(SPECTROGRAM_SRC.matches(read).count(), 1, "the Stars read moved");
    let core = "slice = star_texel(s, f, index, false);";
    // The whole star, read from all nine cells without the inner/halo split.
    let full = r#"
        slice = vec4<f32>(0.0);
        for (var y = -1; y <= 1; y += 1) {
            for (var x = -1; x <= 1; x += 1) {
                slice += star_far_texel(s, f - vec2<f32>(f32(x), f32(y)), index + y * s.grid.x + x);
            }
        }
    "#;
    Some(source.replace(read, if mode == 1 { full } else { core }))
}

struct HaloOverride(u8);
impl HaloOverride {
    fn set(reference: u8) -> Self {
        Self(STAR_REFERENCE.replace(reference))
    }
}
impl Drop for HaloOverride {
    fn drop(&mut self) {
        STAR_REFERENCE.set(self.0);
    }
}

#[test]
fn separate_halos_reconstruct_the_wide_response_including_faint_tails() {
    let Some((device, queue)) = headless_device() else { return };
    let _split = SplitOverride::set(Some(false));
    let mut cb = star_fixture([385, 217], egui::Pos2::ZERO);
    for jitter in [0.0, 0.5, 1.0] {
        for falloff in [0.0, 1.0] {
            let settings = &mut cb.atmosphere.as_mut().unwrap().settings;
            settings.stars.star_jitter = jitter;
            // A broad glow and a long faint tail, each round a solid half so
            // every star leaves its halo image light to witness.
            settings.stars.star_glow_falloff = falloff;
            (settings.stars.star_solid_far, settings.stars.star_solid_near) = (0.5, 0.5);
            settings.stars.star_halo_profile = harmonigraph_scene::StarHaloProfile::Uniform;
            settings.stars.star_halo_resolution = 1.0;
            let mut frames = Vec::new();
            for reference in [0, 1, 2] {
                // Native halo sampling isolates the split's algebra from the
                // intentional interpolation of reduced halo images.
                let _halo = HaloOverride::set(reference);
                let mut resources = CallbackResources::default();
                frames.push(frame_at_ppp(&device, &queue, &mut resources, &cb, 1.0));
                assert_eq!(
                    target(&resources).halo_layout(),
                    Some(atmosphere::star_halo_layout(
                        [385, 217],
                        cb.atmosphere.unwrap().settings.stars
                    ))
                );
            }
            let differences: Vec<_> =
                frames[0].iter().zip(&frames[1]).map(|(a, b)| a.abs_diff(*b)).collect();
            let max = *differences.iter().max().unwrap();
            let mean =
                differences.iter().map(|&d| f64::from(d)).sum::<f64>() / differences.len() as f64;
            assert!(
                max <= 1 && mean <= 0.03,
                "jitter={jitter}, falloff={falloff}: reconstruction max={max}, mean={mean}"
            );
            let halo_pixels = frames[0]
                .chunks_exact(4)
                .zip(frames[2].chunks_exact(4))
                .filter(|(a, b)| a[..3].iter().zip(&b[..3]).any(|(x, y)| x.abs_diff(*y) > 4))
                .count();
            assert!(
                halo_pixels > 500,
                "jitter={jitter}, falloff={falloff}: halo witness only {halo_pixels} pixels"
            );
        }
    }
}

#[test]
fn uniform_halos_preserve_the_original_array_lookup() {
    let Some((device, queue)) = headless_device() else { return };
    let _split = SplitOverride::set(Some(false));
    let mut cb = star_fixture([129, 97], egui::pos2(7.2, 11.6));
    let stars = &mut cb.atmosphere.as_mut().unwrap().settings.stars;
    stars.star_halo_profile = harmonigraph_scene::StarHaloProfile::Uniform;
    // Stars too close for 2x2 at every depth, so every depth owns a layer of
    // the one Uniform array.
    let closest = harmonigraph_scene::STAR_SPACING_MIN;
    (stars.star_spacing_ratio_far, stars.star_spacing_ratio_near) = (closest, closest);
    let three = harmonigraph_scene::star_plan::StarGather::Three;
    assert!(stars.plan().depths.iter().all(|depth| depth.gather == three));
    for resolution in [0.25, 0.5, 1.0] {
        cb.atmosphere.as_mut().unwrap().settings.stars.star_halo_resolution = resolution;
        let mut frames = Vec::new();
        for mode in [0, 3] {
            let _halo = HaloOverride::set(mode);
            let mut resources = CallbackResources::default();
            frames.push(frame_at_ppp(&device, &queue, &mut resources, &cb, 1.25));
        }
        assert_eq!(frames[0], frames[1], "uniform resolution {resolution}");
    }
}

#[test]
fn halo_profile_transitions_preserve_color_history() {
    use harmonigraph_scene::StarHaloProfile::{Medium, Uniform, P3};
    let Some((device, queue)) = headless_device() else { return };
    let _split = SplitOverride::set(Some(false));
    let mut cb = star_fixture([129, 97], egui::pos2(7.2, 11.6));
    let settings = &mut cb.atmosphere.as_mut().unwrap().settings;
    settings.color_pickup = 0.6;
    settings.color_release = 0.6;
    let mut fixed = CallbackResources::default();
    let mut changing = CallbackResources::default();
    let mut final_frame = Vec::new();
    for (step, (profile, resolution, level)) in
        [(P3, 0.5, 220), (Medium, 0.5, 0), (Uniform, 0.25, 80), (P3, 1.0, 0), (Uniform, 1.0, 0)]
            .into_iter()
            .enumerate()
    {
        cb.pass_nr = step as u64;
        cb.grid.fill(level);
        let atmosphere = cb.atmosphere.as_mut().unwrap();
        atmosphere.now = 3.25 + step as f64 * 0.04;
        atmosphere.settings.stars.star_halo_profile = Uniform;
        atmosphere.settings.stars.star_halo_resolution = 1.0;
        let reference = frame_at_ppp(&device, &queue, &mut fixed, &cb, 1.25);
        let settings = &mut cb.atmosphere.as_mut().unwrap().settings;
        settings.stars.star_halo_profile = profile;
        settings.stars.star_halo_resolution = resolution;
        final_frame = frame_at_ppp(&device, &queue, &mut changing, &cb, 1.25);
        assert_eq!(
            target(&changing).halo_layout(),
            Some(atmosphere::star_halo_layout([161, 121], cb.atmosphere.unwrap().settings.stars))
        );
        if step == 4 {
            assert_eq!(final_frame, reference, "halo reallocations changed retained color");
        }
    }
    let mut cold = CallbackResources::default();
    let cold_frame = frame_at_ppp(&device, &queue, &mut cold, &cb, 1.25);
    let retained = final_frame.iter().zip(&cold_frame).filter(|(a, b)| a.abs_diff(**b) > 4).count();
    assert!(retained > final_frame.len() / 100, "fixture did not retain color across transitions");
}

#[test]
fn quality_profiles_cover_partial_panes_at_fractional_scale() {
    let Some((device, queue)) = headless_device() else { return };
    const PPP: f32 = 1.25;
    let mut high_passes = [0; 3];
    for (profile, format) in [
        (harmonigraph_scene::StarHaloProfile::P3, wgpu::TextureFormat::Rgba8Unorm),
        (harmonigraph_scene::StarHaloProfile::Medium, wgpu::TextureFormat::Rgba8Unorm),
        (harmonigraph_scene::StarHaloProfile::Medium, wgpu::TextureFormat::Rgba8UnormSrgb),
        (harmonigraph_scene::StarHaloProfile::Low, wgpu::TextureFormat::Rgba8Unorm),
        (harmonigraph_scene::StarHaloProfile::Low, wgpu::TextureFormat::Rgba8UnormSrgb),
    ] {
        for (case, (jitter, memory)) in
            [(0.0, false), (0.5, true), (1.0, true)].into_iter().enumerate()
        {
            let mut cb = star_fixture([129, 97], egui::pos2(7.3, 11.7));
            cb.target_format = format;
            let region = egui::Rect::from_min_max(
                cb.rect.min + egui::vec2(13.3, 9.3),
                cb.rect.min + egui::vec2(117.07, 87.07),
            );
            // BOTH the source mesh and final region must be interior, otherwise
            // their union silently forces a full-pane far pass.
            for vertex in &mut cb.vertices {
                vertex.pos[0] = vertex.pos[0].clamp(region.left(), region.right());
                vertex.pos[1] = vertex.pos[1].clamp(region.top(), region.bottom());
            }
            let settings = &mut cb.atmosphere.as_mut().unwrap().settings;
            settings.stars.star_jitter = jitter;
            settings.stars.star_halo_profile = profile;
            settings.cloud_depth = 0.65;
            (settings.stars.star_solid_far, settings.stars.star_solid_near) =
                (harmonigraph_scene::STAR_SOLID_MAX, harmonigraph_scene::STAR_SOLID_MAX);
            // Far stars spaced so 2x2 holds them at every jitter: its bound
            // shrinks to 0.7 cells at 1, where the fresh spacing needs 3x3 and
            // the wide reference would compare the frame with itself.
            settings.stars.star_spacing_ratio_far = 0.8;
            let layout = atmosphere::star_layout(settings.stars, cb.rect.aspect_ratio());
            let slices = atmosphere::star_slices(settings.stars, 0.0, 0.0, &layout);
            assert_eq!(slices.map(|s| s.gather)[..3], [2; 3], "jitter={jitter}: far not 2x2");
            if memory {
                settings.color_pickup = 0.6;
                settings.color_release = 0.6;
            }
            let mut full = CallbackResources::default();
            let mut partial = CallbackResources::default();
            let mut wide_reference = CallbackResources::default();
            for step in 0..3 {
                cb.pass_nr = step;
                cb.atmosphere.as_mut().unwrap().now = 3.25 + step as f64 * 0.25;
                cb.grid.fill(80 + step as u8 * 60);
                cb.atmosphere.as_mut().unwrap().region = cb.rect;
                let a = frame_at_ppp(&device, &queue, &mut full, &cb, PPP);
                // The wider gather changes subtraction order at cell boundaries,
                // so permit one quantization level, separately from exact clipping.
                STAR_REFERENCE.set(4);
                let wide = frame_at_ppp(&device, &queue, &mut wide_reference, &cb, PPP);
                STAR_REFERENCE.set(0);
                let errors: Vec<_> = a.iter().zip(&wide).map(|(a, b)| a.abs_diff(*b)).collect();
                let mean = errors.iter().map(|e| f64::from(*e)).sum::<f64>() / errors.len() as f64;
                assert!(
                errors.iter().copied().max().unwrap() <= 1 && mean < 0.01,
                "four-cell gather omitted coverage: jitter={jitter}, memory={memory}, mean={mean}"
            );
                cb.atmosphere.as_mut().unwrap().region = region;
                let b = frame_at_ppp(&device, &queue, &mut partial, &cb, PPP);
                let settings = cb.atmosphere.unwrap().settings;
                assert_eq!(
                    target(&partial).tone_size(),
                    Some(atmosphere::star_far_size([161, 121], settings.stars))
                );
                assert_eq!(
                    target(&partial).near_size(),
                    atmosphere::star_near_size([161, 121], settings.stars)
                );
                if step == 0 {
                    let passes = target(&partial).encoded_passes.load(Ordering::Relaxed);
                    if profile == harmonigraph_scene::StarHaloProfile::P3 {
                        high_passes[case] = passes;
                    } else {
                        assert_eq!(
                            passes,
                            high_passes[case] + 1,
                            "{profile:?} did not encode its foreground pass"
                        );
                    }
                }
                assert_eq!(target(&partial).memory_size().is_some(), memory);
                let width = (cb.rect.max.x * PPP).ceil() as usize + 1;
                let mut checked = 0;
                let mut lit = 0;
                // Include the first and last covered pixel centers on every edge.
                for y in 0..((cb.rect.max.y * PPP).ceil() as usize + 1) {
                    for x in 0..width {
                        let pt = egui::pos2((x as f32 + 0.5) / PPP, (y as f32 + 0.5) / PPP);
                        if !region.contains(pt) {
                            continue;
                        }
                        let i = (y * width + x) * 4;
                        assert_eq!(
                            &a[i..i + 4],
                            &b[i..i + 4],
                            "jitter={jitter}, memory={memory}, step={step}, pixel={x},{y}"
                        );
                        lit += usize::from(b[i..i + 3] != cb.shades.lut[0][..3]);
                        checked += 1;
                    }
                }
                assert!(checked > 10000 && lit > 1000, "fixture did not exercise a lit interior");
            }
        }
    }
}

/// The star the shader draws is the CPU's `star_plan::star_profile`, which
/// the Stars panel previews: one star alone in its cell, 64 pixels in radius,
/// read at every pixel it covers at Solid and Glow falloff's ends and middles.
#[test]
fn a_drawn_star_follows_the_cpu_profile() {
    use harmonigraph_scene::star_plan::{star_falloff_bend, star_profile, StarGather};
    let Some((device, queue)) = headless_device() else { return };
    let _split = SplitOverride::set(Some(false));
    // 540 points tall, so a star pixel is a point and two device pixels.
    const PPP: f32 = 2.0;
    let mut cb = star_fixture([540, 540], egui::Pos2::ZERO);
    // Bright light under every star, so a star's colour sits far from the floor.
    cb.grid.fill(230);
    let width = (540.0 * PPP).ceil() as usize + 1;
    let mut resources = CallbackResources::default();
    for (solid, falloff) in
        [0.0, 0.5, 0.9].into_iter().flat_map(|solid| [0.0, 0.5, 1.0].map(|f| (solid, f)))
    {
        let stars = &mut cb.atmosphere.as_mut().unwrap().settings.stars;
        // The nearest depth alone, still, at the pane's own resolution (the
        // presets draw it into a smaller image and filter it up), each star
        // its full size at its cell's centre in a cell twice its diameter: a
        // 1x1 read, so no neighbour's light adds in.
        stars.star_solo = [false, false, false, false, true];
        stars.star_halo_profile = harmonigraph_scene::StarHaloProfile::Uniform;
        (stars.star_size_far, stars.star_size_near) = (64.0, 64.0);
        (stars.star_spacing_ratio_far, stars.star_spacing_ratio_near) = (2.0, 2.0);
        (stars.star_speed_far, stars.star_speed_near) = (0.0, 0.0);
        (stars.star_jitter, stars.star_size_variation) = (0.0, 0.0);
        (stars.star_solid_far, stars.star_solid_near) = (solid, solid);
        stars.star_glow_falloff = falloff;
        let depth = stars.plan().depths[4];
        assert_eq!(depth.gather, StarGather::Core);
        cb.pass_nr += 1;
        let frame = frame_at_ppp(&device, &queue, &mut resources, &cb, PPP);
        let pixel = |x: usize, y: usize| &frame[(y * width + x) * 4..][..3];
        // Cell (0, 0)'s star, whose corner is the pane's centre: every pixel
        // of the square round it, a pixel past its edge on each side.
        let (centre, radius) = (270.0 + 0.5 * depth.cell, depth.radius);
        let span = ((centre - radius) * PPP) as usize - 2..((centre + radius) * PPP) as usize + 3;
        let at = |p: usize| (p as f32 + 0.5) / PPP - centre;
        let samples: Vec<_> = span
            .clone()
            .flat_map(|y| span.clone().map(move |x| (x, y)))
            .map(|(x, y)| (at(x).hypot(at(y)) / radius, pixel(x, y)))
            .collect();
        let floor = pixel((270.0 * PPP) as usize, (270.0 * PPP) as usize);
        let profile = |t| star_profile(t, solid, star_falloff_bend(falloff));
        // The star's colour and life fade scale the whole curve, so it is
        // read relative to the pixel nearest the centre, in the channel that
        // moves furthest from the floor.
        let (t0, peak) = samples.iter().min_by(|a, b| a.0.total_cmp(&b.0)).unwrap();
        let channel = (0..3).max_by_key(|&c| peak[c].abs_diff(floor[c])).unwrap();
        let contrast = f32::from(peak[channel]) - f32::from(floor[channel]);
        let glow = samples.iter().filter(|(t, _)| (0.05..0.95).contains(&profile(*t))).count();
        assert!(
            contrast > 128.0 && glow > 1000,
            "solid={solid}, falloff={falloff}: the star spans {contrast} levels, {glow} glow pixels"
        );
        // Half a level of 8-bit rounding in the pixel and half in the peak
        // that scales it is one; the quarter over that is the target
        // conversion's slack. Measured worst 0.88 over 17,689 pixels a case.
        let worst = samples
            .iter()
            .map(|(t, value)| {
                let want = f32::from(floor[channel]) + contrast * profile(*t) / profile(*t0);
                (f32::from(value[channel]) - want).abs()
            })
            .fold(0.0, f32::max);
        assert!(worst <= 1.25, "solid={solid}, falloff={falloff}: off the CPU curve by {worst}");
    }
}

/// A depth whose stars fit their own cell reads that cell alone and loses
/// nothing a 3x3 read would see, at stars a hair inside the 1x1 bound.
#[test]
fn a_core_depth_draws_its_stars_whole() {
    let Some((device, queue)) = headless_device() else { return };
    let _split = SplitOverride::set(Some(false));
    let mut cb = star_fixture([129, 97], egui::pos2(7.2, 11.6));
    cb.grid.fill(200);
    for jitter in [0.0, 0.5, 1.0] {
        let stars = &mut cb.atmosphere.as_mut().unwrap().settings.stars;
        stars.star_jitter = jitter;
        let core = harmonigraph_scene::star_plan::StarGather::Core.bound(jitter);
        let spacing = 1.001 * 0.5 / core;
        (stars.star_spacing_ratio_far, stars.star_spacing_ratio_near) = (spacing, spacing);
        let layout = atmosphere::star_layout(*stars, cb.rect.aspect_ratio());
        let slices = atmosphere::star_slices(*stars, 0.0, 0.0, &layout);
        assert_eq!(slices.map(|s| s.gather), [1; 5], "jitter={jitter}");
        let frames: Vec<_> = [0, 4]
            .into_iter()
            .map(|mode| {
                let _halo = HaloOverride::set(mode);
                frame_at_ppp(&device, &queue, &mut CallbackResources::default(), &cb, 1.25)
            })
            .collect();
        let floor = cb.shades.lut[0];
        let lit = frames[0].chunks_exact(4).filter(|px| px[..3] != floor[..3]).count();
        assert!(lit > 500, "jitter={jitter}: only {lit} lit pixels");
        assert_eq!(frames[0], frames[1], "jitter={jitter}: the 1x1 read cut a star");
    }
}

/// At `Twinkle` 0 a layer at the closest `Star spacing` covers the sky at
/// every moment, at full `Position variation`: no star dips as its life turns
/// over. At 100% the same layer shows the floor through cracks wherever the
/// one star that reached a pixel is fading, which is what this fixture
/// measured before the dial.
#[test]
fn a_layer_that_does_not_twinkle_never_shows_a_gap() {
    let Some((device, queue)) = headless_device() else { return };
    let _split = SplitOverride::set(Some(false));
    let (width, height) = (480, 270);
    let mut cb = star_fixture([width, height], egui::Pos2::ZERO);
    // Even light under every star, and every star the same brightness and
    // size, so a pixel below the stars' level is the floor showing through.
    cb.grid.fill(220);
    let stars = &mut cb.atmosphere.as_mut().unwrap().settings.stars;
    stars.star_solo = [false, false, false, false, true];
    stars.star_halo_profile = harmonigraph_scene::StarHaloProfile::Uniform;
    stars.star_halo_resolution = 1.0;
    (stars.star_size_far, stars.star_size_near) = (30.0, 30.0);
    let closest = harmonigraph_scene::STAR_SPACING_MIN;
    (stars.star_spacing_ratio_far, stars.star_spacing_ratio_near) = (closest, closest);
    (stars.star_jitter, stars.star_size_variation, stars.star_randomness) = (1.0, 0.0, 0.0);
    (stars.star_solid_far, stars.star_solid_near, stars.star_glow_falloff) = (0.9, 0.9, 0.0);
    let mut darkest = Vec::new();
    for twinkle in [0.0, 1.0] {
        let stars = &mut cb.atmosphere.as_mut().unwrap().settings.stars;
        (stars.star_twinkle_far, stars.star_twinkle_near) = (twinkle, twinkle);
        let mut resources = CallbackResources::default();
        let mut sums = Vec::new();
        // A dozen moments over four lives, so every cell turns over.
        for step in 0..12u64 {
            cb.pass_nr = step;
            cb.atmosphere.as_mut().unwrap().now = 3.25 + step as f64;
            let frame = frame_at_ppp(&device, &queue, &mut resources, &cb, 1.0);
            let side = width as usize + 1;
            for y in 10..height as usize - 10 {
                for x in 10..width as usize - 10 {
                    let pixel = &frame[(y * side + x) * 4..][..3];
                    sums.push(pixel.iter().map(|&c| u32::from(c)).sum::<u32>());
                }
            }
        }
        let mut sorted = sums.clone();
        sorted.sort_unstable();
        let level = sorted[sorted.len() / 2] as f32;
        darkest.push((
            sorted[0] as f32 / level,
            sums.iter().filter(|&&s| (s as f32) < 0.8 * level).count(),
        ));
    }
    let [(held, _), (twinkling, cracks)] = darkest[..] else { unreachable!() };
    // Measured 0.96 held; 0.75 is the floor showing through.
    assert!(held > 0.9, "a held layer dipped to {held} of its level");
    assert!(
        twinkling < 0.8 && cracks > 50,
        "the twinkling layer showed {cracks} cracks, darkest {twinkling}"
    );
}
