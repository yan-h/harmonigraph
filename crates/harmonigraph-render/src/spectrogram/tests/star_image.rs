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

/// The source this thread's spectrogram pipelines are built from:
/// production's, or a reference written here.
#[derive(Clone, Copy, PartialEq)]
enum Reference {
    Production,
    /// Every drawn depth read over its nine cells by a loop written here,
    /// independently of production's `star_gather3`, so the cheaper reads are
    /// shown to lose nothing and the 3x3 read to index the right cells.
    Wide,
    /// `Wide`, with every star baked as if its depth were read 3x3, which no
    /// star `Star spacing` allows overruns: drawn uncapped, so what the cap
    /// took can be measured against it.
    Uncapped,
}

thread_local! {
    static REFERENCE: std::cell::Cell<Reference> = const { std::cell::Cell::new(Reference::Production) };
}

pub(super) fn reference_source() -> Option<String> {
    let reference = REFERENCE.get();
    if reference == Reference::Production {
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
    let wide = SPECTROGRAM_SRC.replace(
        reads,
        "if s.gather != 0u {
            for (var y = -1; y <= 1; y += 1) {
                for (var x = -1; x <= 1; x += 1) {
                    let cell = index + y * s.grid.x + x;
                    slice += star_texel(s, f - vec2<f32>(f32(x), f32(y)), cell);
                }
            }
        }",
    );
    if reference == Reference::Wide {
        return Some(wide);
    }
    let half_width = "return select(0.5 * f32(gather), 0.5, gather == 0u);";
    assert_eq!(wide.matches(half_width).count(), 1, "the Stars half-width moved");
    Some(wide.replace(half_width, "return 1.5;"))
}

/// Build this thread's pipelines from `reference` while it lives, and
/// restore the one before even if an assertion panics.
struct Using(Reference);
impl Using {
    fn set(reference: Reference) -> Self {
        Self(REFERENCE.replace(reference))
    }
}
impl Drop for Using {
    fn drop(&mut self) {
        REFERENCE.set(self.0);
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
            settings.stars.star_solid = [harmonigraph_scene::STAR_SOLID_MAX; 5];
            // Far stars spaced so 2x2 holds them uncapped at every jitter, at
            // 0.63 of a cell under its half-width of one less the 0.3 a
            // centre strays at 1; the cap is
            // `a_capped_depth_draws_what_a_wide_read_draws`'s. Every
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
                    let _wide = Using::set(Reference::Wide);
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
        stars.star_solid = [solid; 5];
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
/// nothing a 3x3 read would see, at stars a hair inside the largest the 1x1
/// read takes, so that any `Position variation` caps a strayed star.
#[test]
fn a_core_depth_draws_its_stars_whole() {
    let Some((device, queue)) = headless_device() else { return };
    let mut cb = star_fixture([129, 97], egui::pos2(7.2, 11.6));
    cb.grid.fill(200);
    for jitter in [0.0, 0.5, 1.0] {
        let stars = &mut cb.atmosphere.as_mut().unwrap().settings.stars;
        stars.star_jitter = jitter;
        // The widest star over its cell 1x1 takes: one it holds at its
        // cell's centre, and three quarters of at the edge of the band.
        use harmonigraph_scene::star_plan::{star_jitter_width, StarGather, STAR_CAP_SHARE};
        let edge = StarGather::Core.half_width() - star_jitter_width(jitter) / 2.0;
        let core = StarGather::Core.half_width().min(edge / STAR_CAP_SHARE);
        let spacing = 1.001 * 0.5 / core;
        stars.star_spacing_ratio = [spacing; 5];
        // Wide enough that 1x1 holds a texel of the 75% image (6 star
        // pixels) at the edge of the band at full variation, or the texel
        // floor would read them 2x2.
        stars.star_size = [32.0; 5];
        let layout = atmosphere::star_layout(*stars, cb.rect.aspect_ratio());
        let image = atmosphere::star_image_size([161, 121], *stars);
        let slices = atmosphere::star_slices(*stars, 0.0, 0.0, &layout, image);
        assert_eq!(slices.map(|s| s.gather), [1; 5], "jitter={jitter}");
        let frames: Vec<_> = [Reference::Production, Reference::Wide]
            .into_iter()
            .map(|reference| {
                let _reference = Using::set(reference);
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

/// A depth whose largest star 2x2 holds only at its cell's centre is read
/// 2x2, where the worst-case reach it replaced read it 3x3, and draws what a
/// 3x3 read draws: every star strayed past what 2x2 holds is drawn smaller
/// to stay inside it, not cut, and no brighter. Every star is 0.9 of a cell
/// at full `Position variation`, so most are capped.
#[test]
fn a_capped_depth_draws_what_a_wide_read_draws() {
    use harmonigraph_scene::star_plan::{star_jitter_width, StarGather};
    let Some((device, queue)) = headless_device() else { return };
    let mut cb = star_fixture([129, 97], egui::pos2(7.2, 11.6));
    cb.grid.fill(200);
    let stars = &mut cb.atmosphere.as_mut().unwrap().settings.stars;
    (stars.star_jitter, stars.star_size_variation) = (1.0, 0.0);
    stars.star_solo = [false, false, false, false, true];
    stars.star_spacing_ratio = [0.5 / 0.9; 5];
    // Wider than a texel of the 75% image (6 star pixels), or the texel
    // floor would decide the read.
    stars.star_size = [16.0; 5];
    let layout = atmosphere::star_layout(*stars, cb.rect.aspect_ratio());
    let image = atmosphere::star_image_size([161, 121], *stars);
    let nearest = atmosphere::star_slices(*stars, 0.0, 0.0, &layout, image)[4];
    assert_eq!(nearest.gather, 2);
    let worst = StarGather::Two.half_width() - 0.5 * star_jitter_width(1.0);
    assert!(nearest.radius > worst * nearest.cell, "the old rule read 2x2 too");
    let frames: Vec<_> = [Reference::Production, Reference::Wide, Reference::Uncapped]
        .into_iter()
        .map(|reference| {
            let _reference = Using::set(reference);
            frame_at_ppp(&device, &queue, &mut CallbackResources::default(), &cb, 1.25)
        })
        .collect();
    // The reference's own summation order moves a channel by one at most; a
    // cut star loses whole levels.
    let errors: Vec<_> = frames[0].iter().zip(&frames[1]).map(|(a, b)| a.abs_diff(*b)).collect();
    let mean = errors.iter().map(|e| f64::from(*e)).sum::<f64>() / errors.len() as f64;
    let worst = errors.iter().copied().max().unwrap();
    assert!(worst <= 1 && mean < 0.01, "the 2x2 read cut a star: worst {worst}/255, mean {mean}");
    // A capped star is smaller and no brighter, so the field holds less light
    // over the floor than uncapped: measured 0.85 of it. Brightened to keep
    // its light, a capped star kept 0.96.
    let floor = cb.shades.lut[0];
    let light = |frame: &[u8]| -> f64 {
        let over = |px: &[u8]| -> f64 {
            px[..3].iter().zip(&floor).map(|(&c, &f)| f64::from(c.abs_diff(f))).sum()
        };
        frame.chunks_exact(4).map(over).sum()
    };
    let kept = light(&frames[0]) / light(&frames[2]);
    assert!(kept < 0.92, "capping kept {kept} of the uncapped field's light");
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
    (stars.star_solid, stars.star_glow_falloff) = ([0.9; 5], 0.0);
    let mut darkest = Vec::new();
    for twinkle in [0.0, 1.0] {
        let stars = &mut cb.atmosphere.as_mut().unwrap().settings.stars;
        stars.star_twinkle = [twinkle; 5];
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
/// The far stars sit one to a cell, alike and steady, 2 star pixels in
/// radius under a 25% texel of 7.2, so a texel centre lands inside only some
/// of them. Each cell's light over the floor is mostly its star's; drawn at
/// its centres alone, the dimmest of them measured 0 of the mean.
///
/// Once on a grid of cells four texels wide, where each cell's light is its
/// star's alone. Then in cells hardly wider than a texel, at the fresh and
/// at full `Position variation`, where its neighbours' light spills into each
/// cell: the floor widens every star to a texel, and the read must hold that
/// wherever a star strays, so the cap never draws one narrower. Read 2x2
/// instead, which holds the floor only at a cell's centre, the cap drew them
/// down to 0.85 and 0.7 of a cell, and the dimmest cell measured 0.46 and
/// 0.24 of the mean.
#[test]
fn stars_narrower_than_a_texel_all_show() {
    use harmonigraph_scene::star_plan::star_jitter_width;
    let Some((device, queue)) = headless_device() else { return };
    let (width, height) = (400, 300);
    // Measured 0.64, 0.50 and 0.44: the last two as the same stars drawn
    // whole by 3x3 with no cap at all.
    for (spacing, jitter, gather, bound) in
        [(8.0, 0.0, 1, 0.5), (1.85, 0.5, 3, 0.45), (1.85, 1.0, 3, 0.38)]
    {
        let mut cb = star_fixture([width, height], egui::Pos2::ZERO);
        cb.grid.fill(220);
        let stars = &mut cb.atmosphere.as_mut().unwrap().settings.stars;
        stars.star_solo = [true, false, false, false, false];
        stars.star_resolution = 0.25;
        (stars.star_size[0], stars.star_spacing_ratio[0], stars.star_jitter) =
            (4.0, spacing, jitter);
        (stars.star_size_variation, stars.star_randomness) = (0.0, 0.0);
        stars.star_twinkle = [0.0; 5];
        let layout = atmosphere::star_layout(*stars, cb.rect.aspect_ratio());
        let image = atmosphere::star_image_size([width, height], *stars);
        let slice = atmosphere::star_slices(*stars, 0.0, 0.0, &layout, image)[0];
        let texel = crate::stars::STAR_PANE / image[1] as f32;
        assert!(slice.radius < 0.5 * texel && slice.inverse_floor == 1.0 / texel);
        let mut plan = *stars;
        plan.star_solo = [false; 5];
        let read = atmosphere::star_slices(plan, 0.0, 0.0, &layout, image)[0].gather;
        assert_eq!(read, gather, "spacing {spacing}");
        // The read holds the floor even for the worst-placed star, though
        // 2x2 would hold it at a cell's centre.
        let half_width = 0.5 * gather as f32;
        let held = (half_width - 0.5 * star_jitter_width(jitter)) * slice.cell;
        assert!(held >= texel && (jitter == 0.0 || texel <= slice.cell), "holds {held}");
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
                *cells.entry(key).or_default() += pixel
                    .iter()
                    .zip(&floor)
                    .map(|(&c, &f)| f64::from(c) - f64::from(f))
                    .sum::<f64>();
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
        let dimmest = light.iter().copied().fold(f64::INFINITY, f64::min) / mean;
        assert!(
            dimmest > bound,
            "jitter {jitter}: a star went missing: the dimmest held {dimmest} of the mean"
        );
    }
}
