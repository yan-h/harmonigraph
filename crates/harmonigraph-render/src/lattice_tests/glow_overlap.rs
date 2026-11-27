//! Peak-normalized screen is bounded by the fixed full-strength glow, not by
//! either tail at the pixel where notes meet. Read the production glow field
//! with linear reconstruction, excluding rings, shadows, bloom and background.

use super::fixtures::*;
use crate::gpu_harness::{readback, render_to_texture};
use crate::*;

pub(super) const SIZE: [u32; 2] = [256, 256];
const CENTRE: glam::Vec2 = glam::Vec2::new(128.5, 128.5);

pub(super) fn scene(levels: &[f32], gain: f32, separated: bool) -> Scene {
    let mut scene = single_marked_node(0, 0);
    scene.view.glow_reach = 2.88;
    scene.view.glow_strength = gain;
    scene.view.glow_curve = harmonigraph_scene::GlowCurve { shape: 0.0 };
    scene.view.glow_blend = 1.0;
    scene.view.note_bloom = 0.0;
    let original = scene.nodes[0];
    scene.nodes = levels
        .iter()
        .enumerate()
        .map(|(i, level)| {
            let mut node = original;
            node.glow.level = *level;
            node.lattice_pos = harmonigraph_core::LatticePos::new(i as i32, 0, 0);
            if separated {
                node.world_pos.x =
                    (i as f32 * 2.0 - 1.0) * scene.node_radius * harmonigraph_scene::RADII_PER_UV;
            }
            node.cents = if i % 2 == 0 { 0.0 } else { 1100.0 };
            node
        })
        .collect();
    for _ in 0..12 {
        scene.camera.pan(CENTRE - on_screen(&scene, SIZE, glam::Vec3::ZERO));
    }
    assert!(on_screen(&scene, SIZE, glam::Vec3::ZERO).distance(CENTRE) < 0.001);
    scene
}

/// Reconstruct the half-float glow at the requested output size using the
/// production linear blit. No scene ink or dither hides the sampled field.
pub(super) fn glow(shooter: &mut Shooter, scene: &Scene) -> Vec<u8> {
    shooter.shot(scene);
    read_glow(shooter)
}

pub(super) fn read_glow(shooter: &Shooter) -> Vec<u8> {
    let resources = shooter.resources.get::<LatticeResources>().unwrap();
    let offscreen = resources.panes[&shooter.pane].offscreen.as_ref().unwrap();
    let Some(glow) = &offscreen.glow else {
        return vec![0; (shooter.size[0] * shooter.size[1] * 4) as usize];
    };
    read_glow_binding(shooter, glow.binding())
}

pub(super) fn read_glow_binding(shooter: &Shooter, binding: &wgpu::BindGroup) -> Vec<u8> {
    let pipeline = shooter.blit.get_or_init(|| {
        let resources = shooter.resources.get::<LatticeResources>().unwrap();
        let shader = blit_module(&shooter.device);
        let layout = &resources.compiled.filter_layout;
        create_post_pipeline(&shooter.device, &shader, "fs_blit", shooter.format, layout, None)
    });
    let texture = render_to_texture(
        &shooter.device,
        &shooter.queue,
        shooter.size,
        shooter.format,
        wgpu::Color::TRANSPARENT,
        |pass| {
            pass.set_pipeline(pipeline);
            pass.set_bind_group(0, binding, &[]);
            pass.draw(0..4, 0..1);
        },
    );
    readback(&shooter.device, &shooter.queue, &texture, shooter.size)
}

fn linear(gamma: f64) -> f64 {
    if gamma <= 0.04045 {
        gamma / 12.92
    } else {
        ((gamma + 0.055) / 1.055).powf(2.4)
    }
}

pub(super) fn luminance(pixel: &[u8; 4]) -> f64 {
    [0.2126, 0.7152, 0.0722]
        .iter()
        .zip(pixel)
        .map(|(weight, byte)| weight * linear(f64::from(*byte) / 255.0))
        .sum()
}

#[test]
fn a_lone_glow_keeps_its_colour_profile_and_fade() {
    let Some(mut shooter) = Shooter::new(SIZE) else { return };
    // Read a 256-square glow directly: a pane the glow target's divisor times
    // that size renders one, read back at SIZE. This checks the analytic
    // profile before reconstruction.
    let pane = SIZE.map(|n| n * crate::lattice_targets::GLOW_DIVISOR);
    let mut glow = |scene: &Scene| {
        shooter.size = pane;
        shooter.shot(scene);
        shooter.size = SIZE;
        read_glow(&shooter)
    };
    for (gain, level) in [(0.15, 1.0), (1.0, 0.25), (2.0, 1.0)] {
        let mut scene = scene(&[level], gain, false);
        let colour = [0.8, 0.4, 0.1];
        scene.pitch_lut = [glam::Vec4::new(colour[0], colour[1], colour[2], 1.0);
            harmonigraph_scene::PITCH_LUT_N];
        let pixels = glow(&scene);
        for accumulation in [0.5, 1.0] {
            scene.view.glow_accumulation = accumulation;
            assert_eq!(glow(&scene), pixels, "a lone glow cannot change");
        }
        let per_uv = on_screen(
            &scene,
            SIZE,
            glam::Vec3::X * scene.node_radius * harmonigraph_scene::RADII_PER_UV,
        )
        .x - CENTRE.x;
        let mut visible = 0;
        for (index, pixel) in pixels.as_chunks::<4>().0.iter().enumerate() {
            let at = glam::vec2((index % 256) as f32 + 0.5, (index / 256) as f32 + 0.5);
            let d = at.distance(CENTRE) / per_uv;
            // Even unmarked nodes use the maximum configured mark rim.
            let span = harmonigraph_scene::glow_span(
                scene.rings_outer,
                scene.mark_inner,
                scene.view.mark_thickness,
                scene.view.glow_reach,
            );
            let a = (0.8 * gain * level * (1.0 - d / span).max(0.0)).min(1.0);
            for (got, channel) in pixel.iter().zip([colour[0], colour[1], colour[2], 1.0]) {
                assert!(
                    (f32::from(*got) - a * channel * 255.0).abs() < 1.1,
                    "gain {gain}, level {level}, pixel {index}: {pixel:?}, expected coverage {a}"
                );
            }
            visible += usize::from(a > 0.02);
        }
        assert!(visible > 1000, "the fixture must read a visible halo");
    }
}

#[test]
fn different_hues_meet_at_screened_luminance_in_either_order() {
    let Some(mut shooter) = Shooter::new(SIZE) else { return };
    let at = |levels: &[f32]| {
        let mut scene = scene(levels, 0.75, true);
        scene.pitch_lut = std::array::from_fn(|i| {
            if i * 2 < harmonigraph_scene::PITCH_LUT_N {
                glam::Vec4::new(1.0, 0.15, 0.1, 1.0)
            } else {
                glam::Vec4::new(0.1, 1.0, 0.25, 1.0)
            }
        });
        scene
    };
    let left = glow(&mut shooter, &at(&[1.0, 0.0]));
    let right = glow(&mut shooter, &at(&[0.0, 1.0]));
    let mut pair = at(&[1.0, 1.0]);
    let both = glow(&mut shooter, &pair);
    let peak = linear(0.8 * 0.75);
    let mut overlap = 0;
    let mut lift = 0.0f64;
    for ((l, r), b) in
        left.as_chunks::<4>().0.iter().zip(right.as_chunks::<4>().0).zip(both.as_chunks::<4>().0)
    {
        let (a, c, got) = (luminance(l), luminance(r), luminance(b));
        if a > 0.01 && c > 0.01 {
            let expected = a + c - a * c / peak;
            // Three byte-quantized readings plus the intermediate half-float.
            assert!((got - expected).abs() < 0.006, "{got} != {expected}: {l:?}, {r:?}, {b:?}");
            overlap += 1;
            lift = lift.max(got - a.max(c));
        }
    }
    assert!(overlap > 500, "both coloured halos must reach the measured pixels");
    assert!(lift > 0.02, "the join must rise above the tails, not reproduce a near-max");
    pair.nodes.reverse();
    let reversed = glow(&mut shooter, &pair);
    assert!(both.iter().zip(reversed).all(|(a, b)| a.abs_diff(b) <= 1));
}

#[test]
fn a_dense_saturated_chord_stays_under_the_fixed_peak() {
    let Some(mut shooter) = Shooter::new(SIZE) else { return };
    for gain in [0.25, 1.0, 2.0] {
        let mut scene = scene(&[1.0; 32], gain, false);
        // All red forces the gamut repair: keeping its hue at the screened
        // luminance would push red past the ceiling. A grey fixture misses it.
        scene.pitch_lut = [glam::Vec4::new(1.0, 0.0, 0.0, 1.0); harmonigraph_scene::PITCH_LUT_N];
        let peak = (0.8 * f64::from(gain)).min(1.0);
        let ceiling = linear(peak);
        let pixels = glow(&mut shooter, &scene);
        let mut brightest = 0.0f64;
        for pixel in pixels.as_chunks::<4>().0 {
            let light = luminance(pixel);
            assert!(light <= ceiling + 0.005, "gain {gain}: {pixel:?} exceeds peak {peak}");
            assert!(pixel[..3].iter().all(|channel| *channel <= pixel[3]));
            assert!(f64::from(pixel[3]) <= peak * 255.0 + 1.0);
            brightest = brightest.max(light);
        }
        assert!(brightest > ceiling * 0.95, "the chord must actually approach the ceiling");
        let centre = &pixels[(128 * 256 + 128) * 4..][..4];
        assert!(centre[1] > 20 && centre[2] > 20, "gamut repair must be exercised: {centre:?}");
    }
}

#[test]
fn a_fading_neighbour_returns_to_the_lone_glow_without_moving_the_ceiling() {
    let Some(mut shooter) = Shooter::new(SIZE) else { return };
    let at = |level| {
        let mut scene = scene(&[1.0, level], 0.6, true);
        scene.pitch_lut = [glam::Vec4::ONE; harmonigraph_scene::PITCH_LUT_N];
        scene
    };
    let lone = glow(&mut shooter, &at(0.0));
    let mut previous = glow(&mut shooter, &at(1.0));
    for level in [0.5, 0.1, 0.01, 0.0001, 0.0] {
        let current = glow(&mut shooter, &at(level));
        for ((before, after), single) in previous
            .as_chunks::<4>()
            .0
            .iter()
            .zip(current.as_chunks::<4>().0)
            .zip(lone.as_chunks::<4>().0)
        {
            assert!(after[0] <= before[0] + 1);
            assert!(after[0] + 1 >= single[0]);
        }
        if level == 0.0 {
            assert_eq!(current, lone);
        }
        previous = current;
    }
    let mut distant = at(0.0);
    distant.nodes[1].glow.level = 1.0;
    distant.nodes[1].world_pos.x = 10000.0;
    assert_eq!(glow(&mut shooter, &distant), lone, "an unrelated note cannot reset the ceiling");
    distant.view.glow_strength = 0.0;
    assert!(glow(&mut shooter, &distant).iter().all(|byte| *byte == 0));
}

#[test]
fn accumulation_sweeps_to_the_original_screen_of_each_colour_channel() {
    let Some(mut shooter) = Shooter::new(SIZE) else { return };
    let at = |levels: &[f32], accumulation| {
        let mut scene = scene(levels, 0.35, false);
        scene.view.glow_accumulation = accumulation;
        scene.pitch_lut = std::array::from_fn(|i| {
            if i * 2 < harmonigraph_scene::PITCH_LUT_N {
                glam::Vec4::new(0.8, 0.5, 0.2, 1.0)
            } else {
                glam::Vec4::new(0.2, 1.0, 0.4, 1.0)
            }
        });
        scene
    };
    let levels = [1.0, 0.65, 0.85];
    let singles: Vec<_> = (0..3)
        .map(|i| {
            let mut alone = [0.0; 3];
            alone[i] = levels[i];
            glow(&mut shooter, &at(&alone, 0.0))
        })
        .collect();
    let bounded = glow(&mut shooter, &at(&levels, 0.0));
    let original = glow(&mut shooter, &at(&levels, 1.0));
    for (i, actual) in original.iter().enumerate() {
        // The old gather screened gamma-encoded premultiplied RGBA, including
        // alpha. Three quantized singles incur at most 1.5 bytes of input error.
        let expected =
            255.0 * (1.0 - singles.iter().map(|s| 1.0 - f64::from(s[i]) / 255.0).product::<f64>());
        assert!((f64::from(*actual) - expected).abs() < 2.1, "byte {i}: {actual} != {expected}");
    }
    let peak = linear(0.8 * 0.35);
    let brightest = original.as_chunks::<4>().0.iter().map(luminance).fold(0.0f64, f64::max);
    assert!(brightest > peak + 0.02, "the old endpoint must actually exceed the fixed ceiling");
    for accumulation in [0.25, 0.5, 0.75] {
        let mut scene = at(&levels, accumulation);
        let mixed = glow(&mut shooter, &scene);
        for ((a, b), actual) in bounded.iter().zip(&original).zip(&mixed) {
            let expected = f32::from(*a) * (1.0 - accumulation) + f32::from(*b) * accumulation;
            assert!(
                (f32::from(*actual) - expected).abs() < 1.5,
                "the sweep must blend the endpoints"
            );
        }
        assert!(mixed.as_chunks::<4>().0.iter().all(|p| p[..3].iter().all(|c| *c <= p[3])));
        scene.nodes.reverse();
        assert!(mixed.iter().zip(glow(&mut shooter, &scene)).all(|(a, b)| a.abs_diff(b) <= 1));
    }
}

/// Marks can change direction and hue, but no longer resize a held halo.
#[test]
fn marks_do_not_change_the_halo_footprint() {
    let Some(mut shooter) = Shooter::new(SIZE) else { return };
    let mut scene = scene(&[1.0], 0.75, false);
    assert!(
        scene.mark_inner + scene.view.mark_thickness > scene.rings_outer + 0.1,
        "the mark must extend beyond the rings to exercise the old size change"
    );
    scene.nodes[0].octaves.fill(1.0);
    read_light(&mut scene.nodes[0]);
    let alpha =
        |pixels: Vec<u8>| pixels.as_chunks::<4>().0.iter().map(|p| p[3]).collect::<Vec<_>>();
    let bare = alpha(glow(&mut shooter, &scene));
    assert!(bare.iter().filter(|&&v| v > 20).count() > 1000);
    for level in [0.25, 1.0, 0.0] {
        scene.nodes[0].melody_slots = 1 << harmonigraph_scene::MIDDLE_C_SLOT;
        scene.nodes[0].melody_level = level;
        read_light(&mut scene.nodes[0]);
        assert_eq!(bare, alpha(glow(&mut shooter, &scene)), "mark level {level}");
    }
}

/// Inactive mark geometry may widen the footprint, but must not wash the
/// directional colors toward their mean outside the ordinary ring edge.
#[test]
fn the_color_transition_ends_at_the_ring_even_with_a_wider_halo() {
    let Some(mut shooter) = Shooter::new(SIZE) else { return };
    let mut scene = scene(&[1.0], 0.75, false);
    scene.view.glow_blend = 0.0;
    scene.nodes[0].octaves.fill(1.0);
    read_light(&mut scene.nodes[0]);
    scene.pitch_lut = std::array::from_fn(|i| {
        let t = i as f32 / (harmonigraph_scene::PITCH_LUT_N - 1) as f32;
        glam::vec4(1.0 - t, 0.05, t, 1.0)
    });
    scene.view.mark_thickness = 0.0;
    let ordinary = glow(&mut shooter, &scene);
    scene.mark_inner = scene.rings_outer;
    scene.view.mark_thickness = 0.6;
    let wide = glow(&mut shooter, &scene);
    let per_uv = on_screen(
        &scene,
        SIZE,
        glam::Vec3::X * scene.node_radius * harmonigraph_scene::RADII_PER_UV,
    )
    .x - CENTRE.x;
    let hue = |p: &[u8]| {
        let sum = p[..3].iter().map(|&c| f32::from(c)).sum::<f32>();
        glam::vec3(f32::from(p[0]), f32::from(p[1]), f32::from(p[2])) / sum
    };
    let mut count = 0;
    let mut low = glam::Vec3::ONE;
    let mut high = glam::Vec3::ZERO;
    let mut worst = 0.0f32;
    for (i, (a, b)) in ordinary.as_chunks::<4>().0.iter().zip(wide.as_chunks::<4>().0).enumerate() {
        let at = glam::vec2((i % 256) as f32 + 0.5, (i / 256) as f32 + 0.5);
        let d = at.distance(CENTRE) / per_uv;
        if d <= scene.rings_outer + 0.02 || d >= scene.rings_outer + 0.3 {
            continue;
        }
        assert!(a[3] > 40 && b[3] > 40, "sample a visible halo, not quantization noise");
        let (a, b) = (hue(a), hue(b));
        low = low.min(a);
        high = high.max(a);
        worst = worst.max((a - b).abs().max_element());
        count += 1;
    }
    assert!(
        count > 100 && (high - low).max_element() > 0.2,
        "probe multiple directional colors in the affected annulus: count={count}, spread={:?}",
        high - low
    );
    assert!(worst < 0.025, "a wider footprint averaged directional colors: hue error {worst}");
}
