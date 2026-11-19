//! The lattice pane's off-pane light owners, chosen by
//! [`harmonigraph_scene::light_owners`] beside the rules that size and place a
//! node; what this surface already carries light at is the one thing only the
//! UI knows, since the glow fade lives here.

use harmonigraph_core::LatticePos;
use harmonigraph_scene::DrawnWindow;

pub(super) fn owners(
    state: &crate::PictureState,
    window: &DrawnWindow,
    aspect: f32,
    surface: usize,
    now: f64,
) -> Vec<LatticePos> {
    let motion = state.surfaces.node_motion.get(&surface);
    let glow = state.surfaces.glow_fade.get(&surface);
    harmonigraph_scene::light_owners(
        &state.runtime.tracker,
        &state.runtime.tuning,
        &state.appearance.view,
        &state.appearance.camera,
        window,
        aspect,
        |pos| motion.is_some_and(|m| m.owns_light(pos)) || glow.is_some_and(|g| g.owns(pos)),
        now,
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use harmonigraph_core::{NoteEvent, SourceId};
    use harmonigraph_scene::{Projection, Scene, MAX_DRAWN_NODES};

    fn state() -> crate::PictureState {
        let mut state = crate::tests::probe::fresh().picture;
        state.appearance.view.min_sevens = 0;
        state.appearance.view.max_sevens = 0;
        state.appearance.view.glow_attack = 0.0;
        state.appearance.view.glow_release = 1.0;
        state.appearance.view.fade_time = 0.1;
        state
    }
    fn notes(state: &mut crate::PictureState, now: f64, on: bool) {
        for note in [55, 60, 64, 67, 71] {
            state.runtime.tracker.handle_event(if on {
                NoteEvent::on(now, SourceId::DIRECT, 0, note, 1.0)
            } else {
                NoteEvent::off(now, SourceId::DIRECT, 0, note)
            });
        }
    }
    fn compose(state: &mut crate::PictureState, window: &DrawnWindow, now: f64) -> Scene {
        let env = state.appearance.view.envelope();
        state.runtime.tracker.prune(now, &env);
        super::super::lattice::compose_scene(state, window, 1.0, None, 0, now)
    }
    #[test]
    fn selective_owners_keep_regular_nodes_and_carried_release() {
        for projection in [Projection::Cabinet, Projection::Orthographic, Projection::Perspective] {
            let mut sparse = state();
            let mut reference = state();
            for s in [&mut sparse, &mut reference] {
                s.appearance.camera.projection = projection;
                s.appearance.camera.target.x = 0.25;
                notes(s, 0.0, true);
            }
            let window = sparse.appearance.view.scrolled(&sparse.appearance.camera, 1.0);
            let mut padded = window;
            padded.min.threes -= 10;
            padded.min.fives -= 10;
            padded.max.threes += 10;
            padded.max.fives += 10;
            for now in [1.0, 1.1, 1.3, 2.0] {
                if now == 1.1 {
                    notes(&mut sparse, now, false);
                    notes(&mut reference, now, false);
                }
                let actual = compose(&mut sparse, &window, now);
                let expected = compose(&mut reference, &padded, now);
                assert_eq!(
                    actual
                        .nodes
                        .iter()
                        .filter(|n| window.contains(n.lattice_pos))
                        .map(|n| n.lattice_pos)
                        .collect::<Vec<_>>(),
                    window.positions().collect::<Vec<_>>()
                );
                let extra: Vec<_> =
                    actual.nodes.iter().filter(|n| !window.contains(n.lattice_pos)).collect();
                assert!(
                    !extra.is_empty(),
                    "{projection:?} at {now}: fixture must retain off-pane light"
                );
                for node in extra {
                    let other =
                        expected.nodes.iter().find(|n| n.lattice_pos == node.lattice_pos).unwrap();
                    assert_eq!(node.octaves, other.octaves);
                    assert_eq!(node.glow.level, other.glow.level);
                    if now == 2.0 {
                        assert_eq!(
                            node.presence_with_opacity, 0.0,
                            "the fixture must outlive MIDI ink"
                        );
                        assert!(node.glow.level > 0.0);
                    }
                }
            }
        }
    }
    #[test]
    fn audio_only_adds_no_owners_and_a_full_window_has_no_spare_capacity() {
        let mut state = state();
        state.appearance.view.spectral_ring_gate = 0.0;
        state.appearance.view.spectral_ring_width = 0.1;
        state.appearance.view.spectral_reading = harmonigraph_scene::SpectralReading::Spectrum;
        let samples: Vec<_> = (0..48000)
            .map(|i| (std::f32::consts::TAU * 523.2511 * i as f32 / 48000.0).sin())
            .collect();
        crate::spectrum::heard_at(
            &mut state.runtime.spectrum,
            &samples,
            1,
            48000.0,
            1.0,
            &state.appearance.spectrum,
        );
        let window = state.appearance.view.scrolled(&state.appearance.camera, 1.0);
        let audio = compose(&mut state, &window, 1.0);
        assert!(audio.nodes.iter().any(|n| n.audio_ring > 0.0), "fixture must carry audio ink");

        state.appearance.camera.target.x += 2.0;
        let window = state.appearance.view.scrolled(&state.appearance.camera, 1.0);
        assert!(audio
            .nodes
            .iter()
            .all(|n| !state.surfaces.node_motion[&0].owns_light(n.lattice_pos)));
        assert!(owners(&state, &window, 1.0, 0, 1.0).is_empty());
        notes(&mut state, 0.0, true);
        let full = DrawnWindow { min: LatticePos::new(0, 0, 0), max: LatticePos::new(159, 127, 0) };
        assert_eq!(full.count(), MAX_DRAWN_NODES);
        assert!(owners(&state, &full, 1.0, 0, 1.0).is_empty());
    }

    #[test]
    fn a_midi_pan_keeps_outgoing_halo_history_across_the_ordinary_window_edge() {
        let mut state = state();
        state.appearance.view.glow_attack = 0.4;
        notes(&mut state, 0.0, true);
        let before = state.appearance.view.scrolled(&state.appearance.camera, 1.0);
        compose(&mut state, &before, 0.0);
        let previous = compose(&mut state, &before, 1.0);
        state.appearance.camera.target.x += 2.0;
        let after = state.appearance.view.scrolled(&state.appearance.camera, 1.0);
        let current = compose(&mut state, &after, 1.05);
        let outgoing: Vec<_> = current
            .nodes
            .iter()
            .filter(|n| !after.contains(n.lattice_pos))
            .filter(|n| before.contains(n.lattice_pos) && n.glow.level > 0.0)
            .collect();
        assert!(!outgoing.is_empty(), "pan must carry a lit node beyond the ordinary window");
        for node in outgoing {
            let old = previous.nodes.iter().find(|n| n.lattice_pos == node.lattice_pos).unwrap();
            assert!(old.glow.level > 0.0);
            // A restarted light would attack from darkness again, far under
            // where this one had got to.
            assert!(node.glow.level >= old.glow.level, "a held light must continue its attack");
        }
    }
}
