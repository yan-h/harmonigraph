use super::*;

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

// A reference where every drawn depth is read over its nine cells by a loop
// written here, independently of production's `star_gather3`, so the cheaper
// reads are shown to lose nothing and the 3x3 read to index the right cells.
thread_local! {
    static WIDE_REFERENCE: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
}

pub(super) fn reference_source() -> Option<String> {
    if !WIDE_REFERENCE.get() {
        return None;
    }
    let reads = "if s.gather == 1u {
            slice = star_texel(s, f, index);
        } else if s.gather == 2u {
            slice = star_gather2(s, r);
        } else if s.gather == 3u {
            slice = star_gather3(s, f, index);
        }";
    assert_eq!(SPECTROGRAM_SRC.matches(reads).count(), 1, "the Stars reads moved");
    Some(SPECTROGRAM_SRC.replace(
        reads,
        "if s.gather != 0u {
            for (var y = -1; y <= 1; y += 1) {
                for (var x = -1; x <= 1; x += 1) {
                    let cell = index + y * s.grid.x + x;
                    slice += star_texel(s, f - vec2<f32>(f32(x), f32(y)), cell);
                }
            }
        }",
    ))
}

/// Restore the thread's reference even if an assertion panics.
struct WideReference(bool);
impl WideReference {
    fn set(wide: bool) -> Self {
        Self(WIDE_REFERENCE.replace(wide))
    }
}
impl Drop for WideReference {
    fn drop(&mut self) {
        WIDE_REFERENCE.set(self.0);
    }
}

#[test]
fn star_images_cover_partial_panes_at_fractional_scale() {
    let Some((device, queue)) = headless_device() else { return };
    const PPP: f32 = 1.25;
    for (resolution, format) in [
        (1.0, wgpu::TextureFormat::Rgba8Unorm),
        (0.75, wgpu::TextureFormat::Rgba8Unorm),
        (0.75, wgpu::TextureFormat::Rgba8UnormSrgb),
        (0.25, wgpu::TextureFormat::Rgba8Unorm),
        (0.25, wgpu::TextureFormat::Rgba8UnormSrgb),
    ] {
        for (jitter, memory) in [(0.0, false), (0.5, true), (1.0, true)] {
            let mut cb = star_fixture([129, 97], egui::pos2(7.3, 11.7));
            cb.target_format = format;
            let region = egui::Rect::from_min_max(
                cb.rect.min + egui::vec2(13.3, 9.3),
                cb.rect.min + egui::vec2(117.07, 87.07),
            );
            // BOTH the source mesh and final region must be interior, otherwise
            // their union silently forces a full-pane star pass.
            for vertex in &mut cb.vertices {
                vertex.pos[0] = vertex.pos[0].clamp(region.left(), region.right());
                vertex.pos[1] = vertex.pos[1].clamp(region.top(), region.bottom());
            }
            let settings = &mut cb.atmosphere.as_mut().unwrap().settings;
            settings.stars.star_jitter = jitter;
            settings.stars.star_resolution = resolution;
            settings.cloud_depth = 0.65;
            (settings.stars.star_solid_far, settings.stars.star_solid_near) =
                (harmonigraph_scene::STAR_SOLID_MAX, harmonigraph_scene::STAR_SOLID_MAX);
            // Far stars spaced so 2x2 holds them at every jitter: its bound
            // shrinks to 0.7 cells at 1, where the fresh spacing needs 3x3 and
            // the wide reference would compare the frame with itself. Every
            // depth's stars are wider than a texel of the 25% image (17 star
            // pixels), or the texel floor would read them all 3x3; size
            // variation still floors the smallest.
            settings.stars.star_spacing_ratio[..3].fill(0.8);
            settings.stars.star_size = [40.0; 5];
            let layout = atmosphere::star_layout(settings.stars, cb.rect.aspect_ratio());
            let image = atmosphere::star_image_size([161, 121], settings.stars);
            let slices = atmosphere::star_slices(settings.stars, 0.0, 0.0, &layout, image);
            assert_eq!(slices.map(|s| s.gather)[..3], [2; 3], "jitter={jitter}: far not 2x2");
            assert_eq!(slices[4].gather, 3, "jitter={jitter}: nearest not 3x3");
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
                let wide = {
                    let _wide = WideReference::set(true);
                    frame_at_ppp(&device, &queue, &mut wide_reference, &cb, PPP)
                };
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
                    target(&partial).shape().stars.map(|stars| stars.image),
                    Some(atmosphere::star_image_size([161, 121], settings.stars))
                );
                assert_eq!(target(&partial).shape().memory.is_some(), memory);
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
                            "resolution={resolution}, jitter={jitter}, memory={memory}, step={step}, pixel={x},{y}"
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
        // The nearest depth alone, still, at the pane's own resolution (a
        // smaller image is filtered up), each star its full size at its
        // cell's centre in a cell twice its diameter: a 1x1 read, so no
        // neighbour's light adds in.
        stars.star_solo = [false, false, false, false, true];
        stars.star_resolution = 1.0;
        (stars.star_size, stars.star_spacing_ratio) = ([64.0; 5], [2.0; 5]);
        stars.star_speed = [0.0; 5];
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
    let mut cb = star_fixture([129, 97], egui::pos2(7.2, 11.6));
    cb.grid.fill(200);
    for jitter in [0.0, 0.5, 1.0] {
        let stars = &mut cb.atmosphere.as_mut().unwrap().settings.stars;
        stars.star_jitter = jitter;
        let core = harmonigraph_scene::star_plan::StarGather::Core.bound(jitter);
        let spacing = 1.001 * 0.5 / core;
        stars.star_spacing_ratio = [spacing; 5];
        // Wider than a texel of the 75% image (6 star pixels), or the texel
        // floor would read them 3x3.
        stars.star_size = [16.0; 5];
        let layout = atmosphere::star_layout(*stars, cb.rect.aspect_ratio());
        let image = atmosphere::star_image_size([161, 121], *stars);
        let slices = atmosphere::star_slices(*stars, 0.0, 0.0, &layout, image);
        assert_eq!(slices.map(|s| s.gather), [1; 5], "jitter={jitter}");
        let frames: Vec<_> = [false, true]
            .into_iter()
            .map(|wide| {
                let _wide = WideReference::set(wide);
                frame_at_ppp(&device, &queue, &mut CallbackResources::default(), &cb, 1.25)
            })
            .collect();
        let floor = cb.shades.lut[0];
        let lit = frames[0].chunks_exact(4).filter(|px| px[..3] != floor[..3]).count();
        assert!(lit > 500, "jitter={jitter}: only {lit} lit pixels");
        // A cut star loses whole levels; the reference's own summation order
        // moves a channel by one at most.
        let worst = frames[0].iter().zip(&frames[1]).map(|(a, b)| a.abs_diff(*b)).max().unwrap();
        assert!(worst <= 1, "jitter={jitter}: the 1x1 read cut a star ({worst}/255)");
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
    let (width, height) = (480, 270);
    let mut cb = star_fixture([width, height], egui::Pos2::ZERO);
    // Even light under every star, and every star the same brightness and
    // size, so a pixel below the stars' level is the floor showing through.
    cb.grid.fill(220);
    let stars = &mut cb.atmosphere.as_mut().unwrap().settings.stars;
    stars.star_solo = [false, false, false, false, true];
    stars.star_resolution = 1.0;
    stars.star_size = [30.0; 5];
    stars.star_spacing_ratio = [harmonigraph_scene::STAR_SPACING_MIN; 5];
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

/// Every star narrower than a texel of the star image still shows (#1446).
/// The far stars sit one to a cell on a grid, alike and steady, 2 star
/// pixels in radius under a 25% texel of 7.2, so a texel centre lands inside
/// only some of them. Each cell's light over the floor is its star's; drawn
/// at its centres alone, the dimmest of them measured 0 of the mean.
#[test]
fn stars_narrower_than_a_texel_all_show() {
    let Some((device, queue)) = headless_device() else { return };
    let (width, height) = (400, 300);
    let mut cb = star_fixture([width, height], egui::Pos2::ZERO);
    cb.grid.fill(220);
    let stars = &mut cb.atmosphere.as_mut().unwrap().settings.stars;
    stars.star_solo = [true, false, false, false, false];
    stars.star_resolution = 0.25;
    (stars.star_size[0], stars.star_spacing_ratio[0], stars.star_jitter) = (4.0, 8.0, 0.0);
    (stars.star_size_variation, stars.star_randomness) = (0.0, 0.0);
    (stars.star_twinkle_far, stars.star_twinkle_near) = (0.0, 0.0);
    let layout = atmosphere::star_layout(*stars, cb.rect.aspect_ratio());
    let image = atmosphere::star_image_size([width, height], *stars);
    let slice = atmosphere::star_slices(*stars, 0.0, 0.0, &layout, image)[0];
    assert!(slice.radius < 0.5 * crate::stars::STAR_PANE / image[1] as f32);
    let frame = frame_at_ppp(&device, &queue, &mut CallbackResources::default(), &cb, 1.0);
    // Each pixel's cell, as `star_layers` finds it, and its light.
    let floor = cb.shades.lut[0];
    let size = [width as f32, height as f32];
    let star_px = crate::stars::STAR_PANE / size[1];
    let mut cells = std::collections::HashMap::<[i32; 2], f64>::new();
    for y in 0..height as usize {
        for x in 0..width as usize {
            let at = [x as f32 + 0.5, y as f32 + 0.5];
            let key: [i32; 2] = std::array::from_fn(|axis| {
                let sp = (at[axis] - size[axis] * 0.5) * star_px;
                (sp / slice.cell - slice.offset.0[axis].fract()).floor() as i32
            });
            let pixel = &frame[(y * (width as usize + 1) + x) * 4..][..3];
            *cells.entry(key).or_default() +=
                pixel.iter().zip(&floor).map(|(&c, &f)| f64::from(c) - f64::from(f)).sum::<f64>();
        }
    }
    // Only cells whole inside the pane.
    let span = size.map(|side| side * star_px / slice.cell);
    let whole = |key: [i32; 2]| {
        (0..2).all(|axis| {
            let edge = (span[axis] / 2.0) as i32;
            (-edge + 1..edge - 1).contains(&key[axis])
        })
    };
    let light: Vec<f64> =
        cells.into_iter().filter(|&(key, _)| whole(key)).map(|(_, l)| l).collect();
    let mean = light.iter().sum::<f64>() / light.len() as f64;
    assert!(light.len() > 100 && mean > 50.0, "fixture: {} stars, {mean}", light.len());
    // Measured 0.64.
    let dimmest = light.iter().copied().fold(f64::INFINITY, f64::min) / mean;
    assert!(dimmest > 0.5, "a star went missing: the dimmest held {dimmest} of the mean");
}
