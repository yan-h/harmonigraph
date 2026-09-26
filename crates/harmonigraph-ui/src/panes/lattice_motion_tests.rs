//! Final visibility is read through the same composition used by live and offline panes.
use super::*;
use harmonigraph_core::{LatticePos, NoteEvent, SourceId};
use harmonigraph_scene::{AnimationOrder, NodeInstance, Scene, SpectralReading};

fn state(order: AnimationOrder) -> PictureState {
    let mut state = crate::tests::probe::fresh().picture;
    let view = &mut state.appearance.view;
    view.fade_shape = 0.0;
    view.note_animation.order = order;
    view.note_animation.stagger_spread = 0.9;
    view.spectral_reading = SpectralReading::Spectrum;
    view.spectral_ring_width = 0.1;
    view.spectral_ring_gate = 0.15;
    view.spectral_ring_attack = 0.0;
    view.spectral_ring_release = 0.0;
    view.min_sevens = -1;
    view.max_sevens = 1;
    state.runtime.frame_params.fade_time = 1.0;
    state
}

fn event(state: &mut PictureState, now: f64, held: bool) {
    let event = if held {
        NoteEvent::on(now, SourceId::DIRECT, 0, 72, 1.0)
    } else {
        NoteEvent::off(now, SourceId::DIRECT, 0, 72)
    };
    state.runtime.tracker.handle_event(event);
}

fn frame(state: &mut PictureState, now: f64, surface: usize, audio: bool) -> Scene {
    let env = state.appearance.view.envelope(&state.runtime.frame_params);
    state.runtime.tracker.prune(now, &env);
    if audio {
        // A real C5 reaches a late slice under every order; a fabricated ring
        // or C4 alone does not exercise that starting-pose handoff.
        let samples: Vec<_> = (0..48000)
            .map(|i| (std::f32::consts::TAU * 523.2511 * i as f32 / 48000.0).sin())
            .collect();
        state.runtime.spectrum.push_samples(&samples, 1, 48000.0, now, &state.appearance.spectrum);
    }
    let window = state.appearance.view.reach();
    compose_scene(state, &window, None, surface, now)
}

fn origin(scene: &Scene) -> &NodeInstance {
    scene.nodes.iter().find(|n| n.lattice_pos == LatticePos::ORIGIN).unwrap()
}

fn pose(scene: &Scene) -> ([f32; 11], [f32; 11]) {
    let node = origin(scene);
    (node.octaves, node.slice_progress)
}

#[test]
fn markers_and_midi_ring_floor_follow_the_reversed_note_and_existing_history() {
    for names in [NoteNames::Played, NoteNames::Past, NoteNames::All] {
        let mut state = state(AnimationOrder::ALL[0]);
        state.appearance.view.note_names = names;
        state.appearance.view.note_animation.stagger_spread = 0.0;
        // No measured audio: every nonzero ring below must be the MIDI floor.
        event(&mut state, 0.15, true);
        frame(&mut state, 0.18, 0, false);
        event(&mut state, 0.20, false);
        for (now, activation) in [(0.22, 0.03), (0.30, 0.0), (2.20, 0.0)] {
            let scene = frame(&mut state, now, 0, false);
            let node = origin(&scene);
            assert!((node.activation - activation).abs() < 1e-5);
            assert_eq!(node.audio_ring, node.activation);
            assert_eq!(state.runtime.tracker.voices().count() > 0, now < 2.0);
            let expected_name = match names {
                NoteNames::All => 1.0,
                NoteNames::Past if !(0.25..=2.0).contains(&now) => 1.0,
                _ => activation,
            };
            assert!((node.name_level(&state.appearance.view) - expected_name).abs() < 1e-5);
            let index =
                scene.nodes.iter().position(|n| n.lattice_pos == LatticePos::ORIGIN).unwrap();
            let marker = scene.pluses.iter().find(|p| p.node == index).map_or(0.0, |p| p.strength);
            assert!((marker - (1.0 - expected_name)).abs() < 1e-5);
            assert!(scene.nodes.iter().any(|n| !n.on_home));
            assert!(scene.pluses.iter().all(|p| scene.nodes[p.node].on_home));
            // Hover wins even in the Past gap between carried zero and history.
            let window = state.appearance.view.reach();
            let hovered = compose_scene(&mut state, &window, Some(LatticePos::ORIGIN), 0, now);
            assert_eq!(origin(&hovered).name_level(&state.appearance.view), 1.0);
            assert!(!hovered.pluses.iter().any(|p| p.node == index));
        }
    }
}

#[test]
fn measured_audio_keeps_repress_poses_until_the_tracked_lifetime_expires() {
    for order in AnimationOrder::ALL {
        for off in [0.1, 5.0] {
            let mut audio = state(order);
            let mut silent = state(order);
            for state in [&mut audio, &mut silent] {
                event(state, 0.0, true);
            }
            let times: &[f64] = if off < 1.0 {
                &[0.05, 0.1, 0.2, 0.3, 0.35, 1.2, 2.2]
            } else {
                &[0.05, 1.0, 2.0, 4.9, 5.0, 5.9, 6.0, 6.8, 7.0, 7.1, 7.2, 7.3, 9.2]
            };
            let again = if off < 1.0 { 0.3 } else { 7.2 };
            // One fade after the off, plus the 0.9 spread an ordered release
            // holds its presence for.
            let released = if order == AnimationOrder::Simultaneous { 6.0 } else { 7.0 };
            for &now in times {
                if now == off || now == again {
                    event(&mut audio, now, now == again);
                    event(&mut silent, now, now == again);
                }
                let measured = frame(&mut audio, now, 0, true);
                let control = frame(&mut silent, now, 0, false);
                assert!(origin(&measured).audio_ring > 0.0);
                if off < 1.0 || now < released {
                    assert_eq!(pose(&measured), pose(&control), "{order:?}, off={off}, now={now}");
                } else {
                    // The long note's release has ended; its former audio ring
                    // has become an audio-only settled pose by re-press.
                    assert_eq!(origin(&measured).slice_progress, [1.0; 11]);
                }
            }
        }
        // After the old lifetime is pruned, audio alone establishes a settled
        // pose. A subsequent MIDI entrance correctly inherits that pose.
        let mut audio = state(order);
        event(&mut audio, 0.0, true);
        frame(&mut audio, 0.05, 0, true);
        event(&mut audio, 0.1, false);
        frame(&mut audio, 0.2, 0, true);
        let settled = frame(&mut audio, 2.0, 0, true);
        assert_eq!(audio.runtime.tracker.voices().count(), 0);
        assert_eq!(origin(&settled).activation, 0.0);
        assert!(origin(&settled).audio_ring > 0.0);
        assert_eq!(origin(&settled).slice_progress, [1.0; 11]);
        event(&mut audio, 2.1, true);
        assert_eq!(origin(&frame(&mut audio, 2.1, 0, true)).slice_progress, [1.0; 11]);
    }
}

#[test]
fn exact_onset_and_audio_first_keep_surface_local_stable_poses() {
    for order in AnimationOrder::ALL {
        for audio_first in [false, true] {
            let mut state = state(order);
            if audio_first {
                for surface in [0, 1] {
                    let scene = frame(&mut state, 0.0, surface, true);
                    assert_eq!(origin(&scene).activation, 0.0);
                    assert!(origin(&scene).audio_ring > 0.0);
                }
            }
            let on = if audio_first { 0.3 } else { 0.0 };
            event(&mut state, on, true);
            for now in [on, on + 0.2, on + 0.9, on + 2.0] {
                let first = frame(&mut state, now, 0, true);
                let repeated = frame(&mut state, now, 0, false);
                let other = frame(&mut state, now, 1, false);
                assert_eq!(pose(&first), pose(&repeated));
                assert_eq!(pose(&first), pose(&other));
                assert_eq!(origin(&first).slice_progress, [1.0; 11]);
            }
        }
        let mut hidden = state(order);
        hidden.appearance.view.spectral_ring_width = 0.0;
        event(&mut hidden, 0.0, true);
        let scene = frame(&mut hidden, 0.0, 0, true);
        assert_eq!(origin(&scene).slice_progress, [0.0; 11]);
    }
}
