//! Select geometry for off-pane light without creating another animation clock.

use glam::{Vec2, Vec3};
use harmonigraph_core::LatticePos;
use harmonigraph_scene::{DrawnWindow, NodeMotion, MAX_DRAWN_NODES};

pub(super) fn owners(
    state: &crate::PictureState,
    window: &DrawnWindow,
    aspect: f32,
    surface: usize,
    now: f64,
) -> Vec<LatticePos> {
    let spare = MAX_DRAWN_NODES.saturating_sub(window.count());
    let view = &state.appearance.view;
    if spare == 0 || !now.is_finite() || view.glow_reach <= 0.0 || view.glow_strength <= 0.0 {
        return Vec::new();
    }
    let camera = &state.appearance.camera;
    let Some(bounds) = view.halo_window(camera, aspect) else { return Vec::new() };
    if window.contains(bounds.min) && window.contains(bounds.max) {
        return Vec::new();
    }
    let classes = NodeMotion::light_classes(
        &state.runtime.tracker,
        view,
        &view.envelope(&state.runtime.frame_params),
        now,
    );
    let motion = state.surfaces.node_motion.get(&surface);
    let glow = state.surfaces.glow_fade.get(&surface);
    if classes.is_empty() && motion.is_none() && glow.is_none() {
        return Vec::new();
    }
    let center = view.center();
    let transform = camera.view_proj(aspect);
    let (right, up) = camera.right_up();
    let radius = view.halo_radius();
    let tuning = &state.runtime.tuning;
    let mut extra: Vec<_> = bounds
        .positions()
        .filter(|&pos| {
            if window.contains(pos) {
                return false;
            }
            let class =
                harmonigraph_core::PitchClass::from_cents(tuning.pitch_class(pos).to_cents());
            // A nearest class is on either side in sorted octave order, wrapping
            // at the ends, just as the trail's remembered-class lookup does.
            let matches = if classes.is_empty() {
                false
            } else {
                let above = classes.partition_point(|&pitch| pitch < class);
                tuning.matches(classes[above % classes.len()], class)
                    || tuning.matches(classes[(above + classes.len() - 1) % classes.len()], class)
            };
            if !matches
                && !motion.is_some_and(|m| m.owns_light(pos))
                && !glow.is_some_and(|g| g.owns(pos))
            {
                return false;
            }
            let centered = pos - center;
            let world =
                Vec3::new(centered.fives as f32, centered.threes as f32, centered.sevens as f32);
            let scale = (if view.sevens_size.is_finite() { view.sevens_size } else { 0.15 })
                .clamp(0.15, 1.0)
                .powi(centered.sevens.abs())
                .max(0.05);
            let r = radius * scale;
            let mut lo = Vec2::splat(f32::INFINITY);
            let mut hi = Vec2::splat(f32::NEG_INFINITY);
            for x in [-1.0, 1.0] {
                for y in [-1.0, 1.0] {
                    let p = transform * (world + (right * x + up * y) * r).extend(1.0);
                    if p.w > 0.0 && p.z >= 0.0 {
                        let projected = p.truncate().truncate() / p.w;
                        lo = lo.min(projected);
                        hi = hi.max(projected);
                    }
                }
            }
            hi.x >= -1.0 && hi.y >= -1.0 && lo.x <= 1.0 && lo.y <= 1.0
        })
        .collect();
    if extra.len() > spare {
        let eye = camera.eye();
        let distance = |p: &LatticePos| {
            let p = *p - center;
            Vec3::new(p.fives as f32, p.threes as f32, p.sevens as f32).distance_squared(eye)
        };
        extra.sort_unstable_by(|a, b| distance(a).total_cmp(&distance(b)));
        extra.truncate(spare);
    }
    extra
}

#[cfg(test)]
mod tests {
    use super::*;
    use harmonigraph_core::{NoteEvent, SourceId};
    use harmonigraph_scene::{Projection, Scene};

    fn state() -> crate::PictureState {
        let mut state = crate::tests::probe::fresh().picture;
        state.appearance.view.min_sevens = 0;
        state.appearance.view.max_sevens = 0;
        state.appearance.view.glow_attack = 0.0;
        state.appearance.view.glow_release = 1.0;
        state.runtime.frame_params.fade_time = 0.1;
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
        let env = state.appearance.view.envelope(&state.runtime.frame_params);
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
                        assert_eq!(node.activation, 0.0, "the fixture must outlive MIDI ink");
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
        state.runtime.spectrum.push_samples(&samples, 1, 48000.0, 1.0, &state.appearance.spectrum);
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
            assert_eq!(
                node.glow.incarnation, old.glow.incarnation,
                "crossing the window restarted the light"
            );
            assert!(node.glow.level >= old.glow.level, "a held light must continue its attack");
        }
    }
}
