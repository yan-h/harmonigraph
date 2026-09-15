//! Render the selectable gestures, including an audio ring surviving MIDI release.
use super::fixtures::*;
use harmonigraph_scene::NoteTransition;

#[test]
fn transition_prototypes_draw_distinct_arrivals_and_settle() {
    let Some(mut shooter) = Shooter::new([256, 256]) else { return };
    let mut scene = single_marked_node(MIDDLE_C, 0);
    scene.glow_reach = 1.5;
    scene.glow_strength = 0.8;
    scene.shadow = one_shadow(0.0, 0.0, harmonigraph_scene::ShadowKernel::Gaussian);
    let phases = [0.15, 0.4, 0.7, 1.0, -0.7, -0.4, -0.15];
    let mut reference = Vec::new();
    for mode in NoteTransition::ALL {
        scene.note_transition = mode;
        let mut arrival_diff = 0;
        for (step, phase) in phases.into_iter().enumerate() {
            let node = &mut scene.nodes[0];
            node.transition_phase = phase;
            node.activation = phase.abs();
            node.octaves[harmonigraph_scene::MIDDLE_C_SLOT] = phase.abs();
            node.melody_level = phase.abs();
            node.glow.level = phase.abs();
            let frame = shooter.shot(&scene);
            if mode == NoteTransition::Fade {
                reference.push(frame.clone());
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
        if mode != NoteTransition::Fade {
            assert!(arrival_diff > 100, "{mode:?} did not visibly animate: {arrival_diff}");
        }
    }
}

#[test]
fn transition_keeps_gated_audio_fixed_through_midi_release_and_prune() {
    use harmonigraph_core::{NoteEvent, NoteTracker, SourceId, Tuning};
    use harmonigraph_scene::{derive_scene, Camera, FrameParams, ViewConfig};
    let Some(mut shooter) = Shooter::new([256, 256]) else { return };
    for mode in NoteTransition::ALL {
        let mut tracker = NoteTracker::new();
        tracker.handle_event(NoteEvent::on(0.0, SourceId::DIRECT, 0, 60, 1.0));
        tracker.handle_event(NoteEvent::off(1.0, SourceId::DIRECT, 0, 60));
        let view = ViewConfig { note_transition: mode, ..Default::default() };
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
            let shot = shooter.shot(&scene);
            scene.note_transition = NoteTransition::Fade;
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
fn draw_transition_distance_shadow_softens_across_its_start_seam() {
    let Some(mut shooter) = Shooter::new([256, 256]) else { return };
    let mut scene = single_marked_node(MIDDLE_C, 0);
    scene.note_transition = NoteTransition::DrawAndRetract;
    scene.nodes[0].transition_phase = 0.65;
    scene.mark_inner = 1.1;
    scene.mark_thickness = 0.4;
    scene.glow_strength = 0.0;
    scene.shadow = one_shadow(0.85, 0.8, harmonigraph_scene::ShadowKernel::Distance);
    scene.background = glam::Vec4::new(0.24, 0.24, 0.24, 1.0);
    shooter.clear = crate::wgpu::Color { r: 0.24, g: 0.24, b: 0.24, a: 1.0 };
    let (_, up) = scene.camera.right_up();
    let top = up * (scene.node_radius * 1.8 * (scene.mark_inner + scene.mark_thickness));
    let (at, _) =
        crate::project_onto(&scene.camera.view_proj(1.0), glam::Vec2::splat(256.0), top).unwrap();
    let frame = shooter.shot(&scene);
    let x = at.x.round() as usize;
    let y = at.y.round() as usize;
    let mut largest_jump = 0;
    let mut darkest = 255;
    for above in 3..=12 {
        let left = frame[((y - above) * 256 + x - 1) * 4];
        let right = frame[((y - above) * 256 + x) * 4];
        largest_jump = largest_jump.max(left.abs_diff(right));
        darkest = darkest.min(left.min(right));
    }
    assert!(darkest < 55, "fixture never reached the shadow above the mark: {darkest}");
    assert!(
        largest_jump < 8,
        "hard radial shadow edge: {largest_jump}/255 between adjacent pixels"
    );
}

#[test]
fn slice_pops_move_complete_pieces_in_distinct_orders_and_settle() {
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
        scene.note_transition = NoteTransition::PopAndSettle;
        scene.nodes[0].transition_phase = 1.0;
        let reference = shooter.shot(&scene);
        for mode in [
            NoteTransition::PopAndSettle,
            NoteTransition::StaggeredPop,
            NoteTransition::ClockwisePop,
        ] {
            scene.note_transition = mode;
            for (step, phase) in
                [0.08, 0.2, 0.3, 0.5, 0.72, 0.99, 1.0, -0.7, -0.3].into_iter().enumerate()
            {
                scene.nodes[0].transition_phase = phase;
                let shot = shooter.shot(&scene);
                if phase == 1.0 {
                    assert!(shot == reference, "{mode:?} did not settle under {kernel:?}");
                }
                if phase == 0.99 {
                    let (mean, _) = harmonigraph_golden::drift(&reference, &shot);
                    assert!(mean < 0.5, "{mode:?} stepped at the end under {kernel:?}: {mean}/255");
                }
                if phase == 0.2 && mode != NoteTransition::PopAndSettle {
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
