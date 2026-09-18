//! Render the selectable gestures, including an audio ring surviving MIDI release.
use super::fixtures::*;
use harmonigraph_scene::{AnimationOrder, NoteAnimation, NoteAnimationConfig};

#[test]
fn distance_shadows_fade_continuously_across_layers_and_settling() {
    let Some(mut shooter) = Shooter::new([256, 256]) else { return };
    shooter.clear = crate::wgpu::Color { r: 0.8, g: 0.8, b: 0.8, a: 1.0 };
    let root = std::env::var_os("HARMONIGRAPH_SHADOW_FRAMES").map(std::path::PathBuf::from);
    if let Some(root) = &root {
        std::fs::create_dir_all(root).unwrap();
    }
    for kernel in
        [harmonigraph_scene::ShadowKernel::Gaussian, harmonigraph_scene::ShadowKernel::Distance]
    {
        for (name, animation, grow, order, spread) in [
            ("plain", NoteAnimation::Fade, false, AnimationOrder::Simultaneous, 0.28),
            ("pop", NoteAnimation::Pop, false, AnimationOrder::Simultaneous, 0.28),
            ("grow", NoteAnimation::Fade, true, AnimationOrder::Simultaneous, 0.28),
            ("ordered", NoteAnimation::Pop, true, AnimationOrder::Circular, 0.28),
            ("wide", NoteAnimation::Pop, true, AnimationOrder::Circular, 0.9),
            ("marks", NoteAnimation::Fade, false, AnimationOrder::Simultaneous, 0.28),
            ("audio", NoteAnimation::Fade, false, AnimationOrder::Simultaneous, 0.28),
        ] {
            let mut scene = single_marked_node(MIDDLE_C, 0);
            scene.pluses.clear();
            scene.glow_strength = 0.0;
            scene.bloom_strength = 0.0;
            scene.note_animation = NoteAnimationConfig {
                animation,
                order,
                stagger_spread: spread,
                radial_start: if grow { -1.0 } else { 0.0 },
                start_size: if grow { 0.0 } else { 1.0 },
            };
            scene.shadow = one_shadow(0.6, 0.8, kernel);
            if name == "marks" {
                scene.shadow.lattice_geometry.falloff = 1.8;
            }
            if name == "audio" {
                scene.shadow.lattice_geometry.falloff = 0.6;
                scene.outer_outer = 0.0;
                scene.mark_thickness = 0.0;
                scene.spectral.inner = 0.2;
                scene.spectral.outer = 0.45;
                *scene.spectral.levels = [200; harmonigraph_scene::SPECTRAL_BUCKETS];
                *scene.spectral.color_levels = [200; harmonigraph_scene::SPECTRAL_BUCKETS];
                scene.spectral.lut = [glam::Vec4::ONE; harmonigraph_scene::PITCH_LUT_N];
            }
            let delays =
                scene.note_animation.delays(&scene.octave_layout, scene.nodes[0].cents, 42, 1.0);
            // `p` sweeps the WHOLE arrival, which a stagger widens past one
            // sector's own duration: every sector animates for 1.0 and the
            // last one only starts at the widest delay.
            let span = 1.0 + delays.iter().copied().fold(0.0, f32::max);
            let mut previous = vec![0i16; 256 * 256];
            let mut csv = String::from("time,mass,max_delta,mean_delta\n");
            let mut peak_mass = 0i64;
            let mut initial_mass = 0i64;
            // The final substeps isolate the opaque endpoint from a full
            // frame of geometry movement; the cell representation stays fixed.
            for step in 0..=102 {
                let p = match step {
                    100 => 0.999,
                    101 => 0.99999,
                    102 => 1.0,
                    _ => step as f32 / 100.0,
                };
                scene.nodes[0].slice_progress =
                    std::array::from_fn(|i| (p * span - delays[i]).clamp(0.0, 1.0));
                scene.nodes[0].activation = p;
                scene.nodes[0].octaves = [p; 11];
                scene.nodes[0].melody_level = p;
                scene.nodes[0].bass_level = p;
                if name == "marks" {
                    scene.nodes[0].slice_progress = [1.0; 11];
                    scene.nodes[0].activation = 1.0;
                    scene.nodes[0].octaves = [1.0; 11];
                    scene.nodes[0].melody_slots = if p > 0.0 { MIDDLE_C } else { 0 };
                }
                if name == "audio" {
                    scene.nodes[0].slice_progress = [1.0; 11];
                    scene.nodes[0].activation = 0.0;
                    scene.nodes[0].octaves = [0.0; 11];
                    scene.nodes[0].audio_ring = p;
                }
                let frame = shooter.shot_again(&scene);
                let bare = shooter.draw_modified(&scene, crate::LatticeLabels::default(), |cb| {
                    cb.uniforms.geometry_shadow.depth = 0.0
                });
                let shadow: Vec<i16> = bare
                    .chunks_exact(4)
                    .zip(frame.chunks_exact(4))
                    .map(|(a, b)| i16::from(a[0]) - i16::from(b[0]))
                    .collect();
                let delta: Vec<i16> =
                    shadow.iter().zip(&previous).map(|(a, b)| (a - b).abs()).collect();
                let max = delta.iter().max().unwrap();
                let mean = delta.iter().map(|&v| f64::from(v)).sum::<f64>() / 65536.0;
                let mass = shadow.iter().map(|&v| i64::from(v)).sum::<i64>();
                peak_mass = peak_mass.max(mass);
                if step == 0 {
                    initial_mass = mass;
                }
                if step > 0 && kernel == harmonigraph_scene::ShadowKernel::Distance {
                    assert!(
                        *max < 80 && mean < 0.8,
                        "{name} at {p}: shadow jumped {max} levels, mean {mean}"
                    );
                    if step == 102 {
                        assert!(
                            *max <= 1 && mean < 0.001,
                            "{name}: opaque endpoint jumped {max}, mean {mean}"
                        );
                    } else if matches!(name, "plain" | "marks" | "audio") {
                        assert!(*max <= 8, "{name} at {p}: fixed geometry snapped by {max}");
                    }
                }
                csv.push_str(&format!(
                    "{p},{},{max},{mean}\n",
                    shadow.iter().map(|&v| i64::from(v)).sum::<i64>()
                ));
                if let Some(root) = &root {
                    let mut ppm = b"P6\n256 256\n255\n".to_vec();
                    for pixel in frame.chunks_exact(4) {
                        ppm.extend_from_slice(&pixel[..3]);
                    }
                    std::fs::write(root.join(format!("{kernel:?}-{name}-{step}.ppm")), ppm)
                        .unwrap();
                }
                previous = shadow;
            }
            assert!(peak_mass > 10_000, "{kernel:?}/{name}: fixture cast no measurable shadow");
            if name == "marks" {
                assert!(
                    peak_mass - initial_mass > 10_000,
                    "mark shadow was hidden by the held body"
                );
            }
            if let Some(root) = &root {
                std::fs::write(root.join(format!("{kernel:?}-{name}.csv")), csv).unwrap();
            }
        }
    }
}

#[test]
fn transition_prototypes_draw_distinct_arrivals_and_settle() {
    let Some(mut shooter) = Shooter::new([256, 256]) else { return };
    let mut scene = single_marked_node(MIDDLE_C, 0);
    scene.glow_reach = 1.5;
    scene.glow_strength = 0.8;
    scene.shadow = one_shadow(0.0, 0.0, harmonigraph_scene::ShadowKernel::Gaussian);
    let phases = [0.15f32, 0.4, 0.7, 1.0, -0.7, -0.4, -0.15];
    let mut reference = Vec::new();
    for mode in NoteAnimation::ALL {
        scene.note_animation.animation = mode;
        let mut arrival_diff = 0;
        for (step, phase) in phases.into_iter().enumerate() {
            let node = &mut scene.nodes[0];
            node.slice_progress = [phase.abs(); 11];
            node.activation = phase.abs();
            node.octaves[harmonigraph_scene::MIDDLE_C_SLOT] = phase.abs();
            node.melody_level = phase.abs();
            node.glow.level = phase.abs();
            let frame = shooter.shot(&scene);
            if mode == NoteAnimation::Fade {
                reference.push(frame.clone());
                for order in AnimationOrder::ALL {
                    scene.note_animation.order = order;
                    scene.note_animation.stagger_spread = 0.0;
                    assert_eq!(
                        shooter.shot(&scene),
                        frame,
                        "zero spread changed Fade for {order:?}"
                    );
                }
                scene.note_animation = NoteAnimationConfig::default();
            } else if step < 3 {
                arrival_diff += differing_pixels(&frame, &reference[step]);
            } else if step == 3 {
                assert!(frame == reference[step], "{mode:?} did not settle onto Fade");
            }
            // Optional scratch frames for visual inspection; no golden baseline changes.
            if let Some(root) = std::env::var_os("HARMONIGRAPH_TRANSITION_FRAMES") {
                let root = std::path::PathBuf::from(root);
                std::fs::create_dir_all(&root).unwrap();
                let mut ppm = b"P6\n256 256\n255\n".to_vec();
                for pixel in frame.chunks_exact(4) {
                    ppm.extend_from_slice(&pixel[..3]);
                }
                std::fs::write(root.join(format!("{mode:?}-{step}.ppm")), ppm).unwrap();
            }
        }
        if mode != NoteAnimation::Fade {
            assert!(arrival_diff > 100, "{mode:?} did not visibly animate: {arrival_diff}");
        }
    }
}

#[test]
fn transition_keeps_gated_audio_fixed_through_midi_release_and_prune() {
    use harmonigraph_core::{NoteEvent, NoteTracker, SourceId, Tuning};
    use harmonigraph_scene::{derive_scene, Camera, FrameParams, ViewConfig};
    let Some(mut shooter) = Shooter::new([256, 256]) else { return };
    for mode in NoteAnimation::ALL {
        let mut tracker = NoteTracker::new();
        tracker.handle_event(NoteEvent::on(0.0, SourceId::DIRECT, 0, 60, 1.0));
        tracker.handle_event(NoteEvent::off(1.0, SourceId::DIRECT, 0, 60));
        let view = ViewConfig {
            note_animation: NoteAnimationConfig {
                animation: mode,
                radial_start: 1.0,
                start_size: 2.0,
                ..Default::default()
            },
            ..Default::default()
        };
        let frame = FrameParams { fade_time: 1.0, ..Default::default() };
        for now in [1.1, 1.5, 1.9, 2.1] {
            tracker.prune(now, &view.envelope(&frame));
            let mut scene = derive_scene(
                &tracker,
                &Tuning::default(),
                &view,
                &view.reach(),
                &frame,
                Camera::default(),
                None,
                now,
            );
            scene.nodes.retain(|n| n.lattice_pos == harmonigraph_core::LatticePos::ORIGIN);
            scene.pluses.clear();
            scene.node_radius = 1.1;
            scene.glow_strength = 0.0;
            scene.bloom_strength = 0.0;
            scene.outer_inner = 0.0;
            scene.outer_outer = 0.0;
            scene.mark_thickness = 0.0;
            scene.shadow = one_shadow(0.0, 0.0, harmonigraph_scene::ShadowKernel::Gaussian);
            // An actually visible independently gated audio annulus; silence at
            // the gate is deliberately not the fixture this test measures.
            scene.spectral.inner = 0.2;
            scene.spectral.outer = 0.45;
            *scene.spectral.levels = [200; harmonigraph_scene::SPECTRAL_BUCKETS];
            *scene.spectral.color_levels = [200; harmonigraph_scene::SPECTRAL_BUCKETS];
            scene.spectral.lut =
                [glam::Vec4::new(0.9, 0.4, 0.2, 1.0); harmonigraph_scene::PITCH_LUT_N];
            scene.nodes[0].audio_ring = 1.0;
            scene.nodes[0].slice_progress = [((2.0 - now) as f32).clamp(0.0, 1.0); 11];
            let shot = shooter.shot(&scene);
            scene.note_animation = NoteAnimationConfig::default();
            let reference = shooter.shot(&scene);
            // The spark can move outside this independently opaque annulus;
            // measure the audio pixels, not the empty space around the ring.
            let audio_pixels: Vec<_> = reference
                .chunks_exact(4)
                .enumerate()
                .filter(|(_, p)| p[0] > 10 || p[1] > 10 || p[2] > 10)
                .map(|(i, _)| i)
                .collect();
            assert!(audio_pixels.len() > 100, "the audio fixture drew no ring");
            for i in audio_pixels {
                assert_eq!(
                    &shot[i * 4..i * 4 + 4],
                    &reference[i * 4..i * 4 + 4],
                    "{mode:?}: audio moved at {now}, pixel {i}"
                );
            }
        }
    }
}

#[test]
fn fade_and_pop_grow_complete_pieces_and_settle() {
    let Some(mut shooter) = Shooter::new([384, 384]) else { return };
    let mut scene = single_marked_node(0, 0);
    scene.node_radius = 1.6;
    scene.glow_strength = 0.0;
    scene.octave_layout = harmonigraph_scene::octave_layout(4, 60.0, 1, 0.3, 0.7);
    scene.nodes[0].cents = 350.0;
    scene.nodes[0].octaves = [1.0; harmonigraph_scene::OCTAVE_SLOTS];
    let (low, high) = scene.octave_layout.slots(350.0);
    scene.nodes[0].melody_slots = 1 << high;
    scene.nodes[0].bass_slots = 1 << low;
    scene.nodes[0].melody_level = 1.0;
    scene.nodes[0].bass_level = 1.0;
    let mut previews = Vec::new();
    for kernel in
        [harmonigraph_scene::ShadowKernel::Gaussian, harmonigraph_scene::ShadowKernel::Distance]
    {
        scene.shadow = one_shadow(0.6, 0.8, kernel);
        scene.background = glam::Vec4::splat(0.2);
        scene.background.w = 1.0;
        shooter.clear = crate::wgpu::Color { r: 0.2, g: 0.2, b: 0.2, a: 1.0 };
        scene.note_animation = NoteAnimationConfig::default();
        scene.nodes[0].slice_progress = [1.0; 11];
        let reference = shooter.shot(&scene);
        for mode in NoteAnimation::ALL {
            scene.note_animation = NoteAnimationConfig {
                animation: mode,
                radial_start: -1.0,
                start_size: 0.0,
                ..Default::default()
            };
            for (step, phase) in
                [0.08f32, 0.2, 0.3, 0.5, 0.72, 0.99, 1.0, -0.7, -0.3].into_iter().enumerate()
            {
                scene.nodes[0].slice_progress = [phase.abs(); 11];
                let shot = shooter.shot(&scene);
                if phase == 1.0 {
                    let (mean, worst) = harmonigraph_golden::drift(&reference, &shot);
                    // Fixed config bounds may round raster/atlas sampling by
                    // one level; resizing them at settlement would churn cells.
                    assert!(
                        mean < 0.05 && worst <= 1,
                        "{mode:?}/{kernel:?}: settled mean {mean}, worst {worst}"
                    );
                }
                if phase == 0.99 {
                    let (mean, _) = harmonigraph_golden::drift(&reference, &shot);
                    assert!(mean < 0.5, "{mode:?} stepped at the end under {kernel:?}: {mean}/255");
                }
                if phase == 0.2 {
                    previews.push(shot.clone());
                }
                if let Some(root) = std::env::var_os("HARMONIGRAPH_TRANSITION_FRAMES") {
                    let root = std::path::PathBuf::from(root);
                    std::fs::create_dir_all(&root).unwrap();
                    let mut ppm = b"P6\n384 384\n255\n".to_vec();
                    for pixel in shot.chunks_exact(4) {
                        ppm.extend_from_slice(&pixel[..3]);
                    }
                    std::fs::write(
                        root.join(format!("slices-{kernel:?}-{mode:?}-{step}.ppm")),
                        ppm,
                    )
                    .unwrap();
                }
            }
        }
    }
    for pair in previews.chunks_exact(2) {
        assert!(
            differing_pixels(&pair[0], &pair[1]) > 100,
            "the two slice orders drew one picture"
        );
    }
}

#[test]
fn all_orders_draw_complete_rotated_pieces_at_their_shared_delays() {
    let Some(mut shooter) = Shooter::new([384, 384]) else { return };
    let mut scene = single_marked_node(0, 0);
    scene.node_radius = 1.6;
    scene.glow_strength = 0.0;
    scene.octave_layout = harmonigraph_scene::octave_layout(4, 60.0, 1, 0.3, 0.7);
    scene.nodes[0].cents = 350.0;
    scene.nodes[0].octaves = [1.0; 11];
    let (low, high) = scene.octave_layout.slots(350.0);
    scene.nodes[0].melody_slots = 1 << high;
    scene.nodes[0].bass_slots = 1 << low;
    scene.nodes[0].melody_level = 1.0;
    scene.nodes[0].bass_level = 1.0;
    for kernel in
        [harmonigraph_scene::ShadowKernel::Gaussian, harmonigraph_scene::ShadowKernel::Distance]
    {
        scene.shadow = one_shadow(0.6, 0.8, kernel);
        scene.background = glam::Vec4::new(0.2, 0.2, 0.2, 1.0);
        shooter.clear = crate::wgpu::Color { r: 0.2, g: 0.2, b: 0.2, a: 1.0 };
        for animation in NoteAnimation::ALL {
            let mut arrivals = Vec::new();
            for order in AnimationOrder::ALL {
                scene.note_animation = NoteAnimationConfig {
                    animation,
                    order,
                    radial_start: -1.0,
                    start_size: 0.0,
                    ..Default::default()
                };
                let delays = scene.note_animation.delays(&scene.octave_layout, 350.0, 42, 1.0);
                let span = 1.0 + delays.iter().copied().fold(0.0, f32::max);
                for (step, time) in
                    [0.1f32, 0.25, 0.5, 0.8, 1.0, -0.1, -0.25, -0.5, -1.0].into_iter().enumerate()
                {
                    scene.nodes[0].slice_progress = std::array::from_fn(|i| {
                        let p = (time.abs() * span - delays[i]).clamp(0.0, 1.0);
                        if time < 0.0 {
                            1.0 - p
                        } else {
                            p
                        }
                    });
                    let shot = shooter.shot(&scene);
                    if time == 0.25 {
                        arrivals.push(shot.clone());
                    }
                    if let Some(root) = std::env::var_os("HARMONIGRAPH_TRANSITION_FRAMES") {
                        let root = std::path::PathBuf::from(root);
                        std::fs::create_dir_all(&root).unwrap();
                        let mut ppm = b"P6\n384 384\n255\n".to_vec();
                        for pixel in shot.chunks_exact(4) {
                            ppm.extend_from_slice(&pixel[..3]);
                        }
                        std::fs::write(
                            root.join(format!(
                                "orders-{kernel:?}-{animation:?}-{order:?}-{step}.ppm"
                            )),
                            ppm,
                        )
                        .unwrap();
                    }
                }
            }
            for pair in arrivals.windows(2) {
                assert!(differing_pixels(&pair[0], &pair[1]) > 40, "orders drew the same picture");
            }
        }
    }
}
