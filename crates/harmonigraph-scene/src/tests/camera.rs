//! The camera: what it projects, what it refuses to project, and the
//! clamps on the input that moves it.

use crate::*;
use glam::{Vec2, Vec3};
use harmonigraph_core::{NoteEvent, NoteEventKind, NoteTracker, PitchClass, Tuning};
use super::harness::*;

#[test]
fn camera_target_projects_to_viewport_center() {
    let camera = Camera::default();
    let scene = scene_of(
        &NoteTracker::new(),
        &Tuning::default(),
        &ViewConfig::default(),
        &FrameParams::default(),
        0.0,
    );
    let viewport = Vec2::new(800.0, 600.0);
    let p = scene.project(viewport, camera.target).unwrap();
    assert!((p.x - 400.0).abs() < 0.5, "x = {}", p.x);
    assert!((p.y - 300.0).abs() < 0.5, "y = {}", p.y);
}

#[test]
fn points_behind_the_camera_do_not_project() {
    for projection in [
        Projection::Perspective,
        Projection::Orthographic,
        Projection::Cabinet,
    ] {
        let camera = Camera { projection, ..Camera::default() };
        let mut scene = scene_of(
            &NoteTracker::new(),
            &Tuning::default(),
            &ViewConfig::default(),
            &FrameParams::default(),
            0.0,
        );
        scene.camera = camera;
        // Continue from the target through the eye and beyond it.
        let behind = camera.eye() + (camera.eye() - camera.target);
        assert_eq!(
            scene.project(Vec2::new(800.0, 600.0), behind),
            None,
            "{projection:?}"
        );
    }
}

#[test]
fn cabinet_faces_the_sheet_and_shears_sevens_uniformly() {
    let viewport = Vec2::new(800.0, 600.0);
    // Orbit angles are ignored: cabinet always faces the sheet. Pin the
    // shear scale to 0.5 so the "half scale" checks below hold whatever
    // the default is.
    let camera = Camera {
        projection: Projection::Cabinet,
        yaw: 1.0,
        pitch: -0.7,
        cabinet_scale: 0.5,
        ..Camera::default()
    };
    assert_eq!(camera.eye(), Vec3::new(0.0, 0.0, camera.distance));

    let mut s = scene_of(
        &NoteTracker::new(),
        &Tuning::default(),
        &ViewConfig::default(),
        &FrameParams::default(),
        0.0,
    );
    s.camera = camera;
    let px = |w: Vec3| s.project(viewport, w).unwrap();

    // Target centered; front-plane steps map to pure screen axes
    // (the sheet renders undistorted).
    let origin = px(Vec3::ZERO);
    assert!((origin - Vec2::new(400.0, 300.0)).length() < 0.5, "{origin:?}");
    let dx = px(Vec3::X) - origin;
    assert!(dx.x > 1.0 && dx.y.abs() < 1e-3, "{dx:?}");
    let dy = px(Vec3::Y) - origin;
    assert!(dy.y < -1.0 && dy.x.abs() < 1e-3, "{dy:?}"); // screen y is down

    // A +sevens step (toward the viewer) is the same up-right arrow
    // anywhere on the sheet, at half scale split evenly over x/y.
    let dz = px(Vec3::Z) - origin;
    let dz_elsewhere = px(Vec3::new(3.0, -2.0, 1.0)) - px(Vec3::new(3.0, -2.0, 0.0));
    assert!(dz.distance(dz_elsewhere) < 1e-3, "{dz:?} vs {dz_elsewhere:?}");
    let k = 0.5 * std::f32::consts::FRAC_1_SQRT_2;
    assert!((dz.x - dx.x * k).abs() < 0.1, "{dz:?} vs {dx:?}");
    assert!((dz.y - dy.y * k).abs() < 0.1, "{dz:?} vs {dy:?}");

    // The knobs steer the arrow: angle 0 at full (cavalier) scale
    // shears purely horizontally, one front-plane step long.
    s.camera.cabinet_angle = 0.0;
    s.camera.cabinet_scale = 1.0;
    let dz = s.project(viewport, Vec3::Z).unwrap() - s.project(viewport, Vec3::ZERO).unwrap();
    assert!((dz.x - dx.x).abs() < 0.1 && dz.y.abs() < 1e-3, "{dz:?} vs {dx:?}");
}

#[test]
fn orthographic_matches_perspective_at_the_focus_plane_and_is_uniform() {
    let viewport = Vec2::new(800.0, 600.0);
    let perspective = Camera { projection: Projection::Perspective, ..Camera::default() };
    let ortho = Camera { projection: Projection::Orthographic, ..perspective };
    let mut s = scene_of(
        &NoteTracker::new(),
        &Tuning::default(),
        &ViewConfig::default(),
        &FrameParams::default(),
        0.0,
    );

    // The target projects to the viewport center in both projections.
    s.camera = ortho;
    let p = s.project(viewport, ortho.target).unwrap();
    assert!((p.x - 400.0).abs() < 0.5 && (p.y - 300.0).abs() < 0.5, "{p:?}");

    // Framing matches at the focus plane: a point one unit up (in view
    // space) from the target lands on the same pixel either way.
    let (_, up) = perspective.right_up();
    let in_plane = perspective.target + up;
    let ortho_px = s.project(viewport, in_plane).unwrap();
    s.camera = perspective;
    let persp_px = s.project(viewport, in_plane).unwrap();
    assert!(ortho_px.distance(persp_px) < 0.5, "{ortho_px:?} vs {persp_px:?}");

    // The property the projection exists for: equal world offsets give
    // equal pixel offsets at ANY depth. Step one unit right at the
    // focus plane and again two units toward the eye; perspective
    // renders the nearer step longer, orthographic identically.
    s.camera = ortho;
    let (right, _) = ortho.right_up();
    let toward_eye = (ortho.eye() - ortho.target).normalize() * 2.0;
    let d_focus = s.project(viewport, ortho.target + right).unwrap()
        - s.project(viewport, ortho.target).unwrap();
    let d_near = s.project(viewport, ortho.target + toward_eye + right).unwrap()
        - s.project(viewport, ortho.target + toward_eye).unwrap();
    assert!(d_focus.distance(d_near) < 1e-3, "{d_focus:?} vs {d_near:?}");
}

#[test]
fn pick_selects_the_node_nearest_the_pointer() {
    let scene = scene_of(
        &NoteTracker::new(),
        &Tuning::default(),
        &ViewConfig::default(),
        &FrameParams::default(),
        0.0,
    );
    let viewport = Vec2::new(800.0, 600.0);
    // Pointer exactly on the projected origin node must pick it, not a
    // neighbor; a pointer far outside every node picks nothing.
    let origin_px = scene.project(viewport, Vec3::ZERO).unwrap();
    assert_eq!(scene.pick(viewport, origin_px, 24.0), Some(LatticePos::ORIGIN));
    assert_eq!(scene.pick(viewport, Vec2::new(-500.0, -500.0), 24.0), None);
}

#[test]
fn idle_off_sheet_nodes_are_not_pickable() {
    // An idle node off the home sheet draws nothing, so hovering where
    // it would be must not hand back its pitch. Sounding makes it
    // visible, and pickable again. Needs a sevens extent: the default view
    // is the home sheet alone, which has no off-sheet node to hover.
    let view = ViewConfig { extent_sevens: 1, ..plain_view() };
    let tuning = Tuning::default();
    let viewport = Vec2::new(800.0, 600.0);

    let idle = scene_of(
        &NoteTracker::new(),
        &tuning,
        &view,
        &FrameParams::default(),
        0.0,
    );
    let off = *idle
        .nodes
        .iter()
        .find(|n| !n.on_home)
        .expect("default view spans more than the home sheet");
    assert_eq!(off.activation, 0.0);
    assert!(!off.is_visible());
    let off_px = idle.project(viewport, off.world_pos).unwrap();
    assert_ne!(
        idle.pick(viewport, off_px, 24.0),
        Some(off.lattice_pos),
        "idle off-sheet node should not be pickable"
    );

    // Same position, now sounding: play a note carrying its pitch class.
    let pc = tuning.pitch_class(off.lattice_pos);
    let note = (60u8..72)
        .find(|&n| tuning.matches(pc, PitchClass::from_midi_note(n)))
        .expect("some MIDI note lands on this node under 12-TET");
    let mut tracker = NoteTracker::new();
    tracker.handle_event(NoteEvent {
        time: 0.0,
        channel: 0,
        note,
        kind: NoteEventKind::On { velocity: 1.0 },
    });
    let lit = scene_of(&tracker, &tuning, &view, &FrameParams::default(), 0.0);
    let lit_off = lit
        .nodes
        .iter()
        .find(|n| n.lattice_pos == off.lattice_pos)
        .unwrap();
    assert!(lit_off.activation > 0.0, "the note should light this node");
    assert!(lit_off.is_visible());
    assert_eq!(
        lit.pick(viewport, off_px, 24.0),
        Some(off.lattice_pos),
        "sounding off-sheet node should be pickable again"
    );
}

#[test]
fn camera_right_up_is_orthonormal_to_the_view() {
    let camera = Camera::default();
    let (right, up) = camera.right_up();
    assert!((right.length() - 1.0).abs() < 1e-5);
    assert!((up.length() - 1.0).abs() < 1e-5);
    assert!(right.dot(up).abs() < 1e-5);
    let view_dir = (camera.target - camera.eye()).normalize();
    assert!(right.dot(view_dir).abs() < 1e-5);
    assert!(up.dot(view_dir).abs() < 1e-5);
}

#[test]
fn camera_input_respects_clamps() {
    let mut camera = Camera::default();
    camera.orbit(Vec2::new(0.0, 10_000.0));
    assert_eq!(camera.pitch, Camera::PITCH_LIMIT);
    camera.orbit(Vec2::new(0.0, -100_000.0));
    assert_eq!(camera.pitch, -Camera::PITCH_LIMIT);
    camera.zoom(1e6);
    assert_eq!(camera.distance, Camera::MIN_DISTANCE);
    camera.zoom(-1e9);
    assert_eq!(camera.distance, Camera::MAX_DISTANCE);
    // Panning moves the target in the view plane, never toward the eye.
    let before = camera.eye() - camera.target;
    camera.pan(Vec2::new(40.0, -25.0));
    let after = camera.eye() - camera.target;
    assert!((before - after).length() < 1e-4);
}

#[test]
fn zoom_by_scales_distance_and_clamps() {
    let mut camera = Camera::default();
    let start = camera.distance;
    // factor > 1 pulls the eye in (distance divides down)...
    camera.zoom_by(2.0);
    assert!((camera.distance - start / 2.0).abs() < 1e-4);
    // ...and factor < 1 pushes it back out.
    camera.zoom_by(0.5);
    assert!((camera.distance - start).abs() < 1e-4);
    // A huge factor clamps at the near limit; a tiny one at the far limit.
    camera.zoom_by(1e6);
    assert_eq!(camera.distance, Camera::MIN_DISTANCE);
    camera.zoom_by(1e-6);
    assert_eq!(camera.distance, Camera::MAX_DISTANCE);
    // Non-positive factors are ignored (no divide-by-zero or sign flip).
    let held = camera.distance;
    camera.zoom_by(0.0);
    camera.zoom_by(-3.0);
    assert_eq!(camera.distance, held);
}

#[test]
fn visible_count_matches_visible_positions() {
    // `visible_count` is a `Vec::with_capacity` hint; it must equal the
    // number `visible_positions` actually enumerates, including the
    // degenerate cases where a non-positive extent collapses an axis to
    // empty.
    for &(t, f, s) in &[(0, 0, 0), (2, 1, 0), (3, 3, 3), (1, 0, 4), (-1, 2, 0)] {
        let view = ViewConfig {
            extent_threes: t,
            extent_fives: f,
            extent_sevens: s,
            ..ViewConfig::default()
        };
        assert_eq!(
            view.visible_count(),
            view.visible_positions().count(),
            "extents ({t}, {f}, {s})"
        );
    }
}
