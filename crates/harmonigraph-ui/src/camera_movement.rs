//! Host-owned camera movement is projected into the live appearance each frame.
//! Absolute pan includes the lattice center that follow_camera rebases.

use crate::params::ParamKey;
use crate::{AppearanceDocument, ParamBackend};
use harmonigraph_scene::{Camera, ViewConfig};

impl AppearanceDocument {
    pub fn camera_movement(&self) -> [f32; 5] {
        [
            self.camera.yaw,
            self.camera.pitch,
            self.camera.distance,
            self.view.center_fives as f32 + self.camera.target.x,
            self.view.center_threes as f32 + self.camera.target.y,
        ]
    }

    pub(crate) fn set_camera_movement(&mut self, values: [f32; 5]) {
        self.camera.yaw = values[0];
        self.camera.pitch = values[1];
        self.camera.distance = values[2];
        self.view.center_fives = values[3].round() as i32;
        self.view.center_threes = values[4].round() as i32;
        self.camera.target.x = values[3] - self.view.center_fives as f32;
        self.camera.target.y = values[4] - self.view.center_threes as f32;
        self.camera.target.z = 0.0;
    }

    /// Copy every appearance field another owner writes from `from` into `to`:
    /// the host's camera movement, which [`set_camera_movement`] lays out as
    /// yaw, pitch, distance and the pan split between the camera's target and
    /// the view's lattice center.
    ///
    /// The one list of them. A look (`appearance_edit::Look`) resets these on
    /// capture and keeps the live document's on apply, so undo and the slots
    /// never carry the host's camera; `read-plugin-state.py` keeps a copy,
    /// joined to this body by its test.
    ///
    /// [`set_camera_movement`]: Self::set_camera_movement
    pub(crate) fn copy_host_owned(
        (camera, view): (&mut Camera, &mut ViewConfig),
        (from_camera, from_view): (&Camera, &ViewConfig),
    ) {
        camera.yaw = from_camera.yaw;
        camera.pitch = from_camera.pitch;
        camera.distance = from_camera.distance;
        camera.target = from_camera.target;
        view.center_fives = from_view.center_fives;
        view.center_threes = from_view.center_threes;
    }

    /// Called before drawing and recording snapshots. Missing channels retain
    /// the chosen appearance, so old takes can still choose a camera.
    pub fn sync_camera(&mut self, params: &dyn ParamBackend) {
        let mut movement = self.camera_movement();
        let mut supplied = false;
        for (index, key) in ParamKey::CAMERA.into_iter().enumerate() {
            if let Some(value) = params.camera_value(key) {
                movement[index] = bounded(key, value);
                supplied = true;
            }
        }
        if supplied {
            self.set_camera_movement(movement);
        }
    }
}

fn bounded(key: ParamKey, value: f32) -> f32 {
    let value = if value.is_finite() { value } else { key.default_value() };
    let value = if key == ParamKey::CameraYaw && !key.range().contains(&value) {
        (value + std::f32::consts::PI).rem_euclid(std::f32::consts::TAU) - std::f32::consts::PI
    } else {
        value
    };
    value.clamp(*key.range().start(), *key.range().end())
}

/// All interactive camera paths (pane navigation, preview and presets) edit
/// the frame's snapshot. Submit only their changed channels to the host.
pub(crate) fn finish_edits(
    appearance: &mut AppearanceDocument,
    before: [f32; 5],
    params: &dyn ParamBackend,
    ctx: &egui::Context,
) {
    // Without a gesture held open, the host gets a whole begin/set/end for
    // every frame the camera moves.
    let dragging = crate::gesture_open(ctx);
    let mut after = appearance.camera_movement();
    let mut changed = false;
    for (index, key) in ParamKey::CAMERA.into_iter().enumerate() {
        if params.camera_value(key).is_some() && after[index] != before[index] {
            after[index] = bounded(key, after[index]);
            if dragging {
                params.begin_set(key);
            }
            params.set(key, after[index]);
            changed = true;
        }
    }
    if changed {
        appearance.set_camera_movement(after);
    }
    if !dragging {
        for key in ParamKey::ALL {
            params.end_set(key);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::RefCell;

    struct Backend(RefCell<[f32; 5]>);
    impl ParamBackend for Backend {
        fn get(&self, key: ParamKey) -> f32 {
            ParamKey::CAMERA
                .iter()
                .position(|k| *k == key)
                .map_or(key.default_value(), |i| self.0.borrow()[i])
        }
        fn set(&self, key: ParamKey, value: f32) {
            let i = ParamKey::CAMERA.iter().position(|k| *k == key).unwrap();
            self.0.borrow_mut()[i] = value;
        }
        fn camera_value(&self, key: ParamKey) -> Option<f32> {
            Some(self.get(key))
        }
    }

    #[test]
    fn absolute_pan_sync_is_idempotent_for_every_projection_and_restore() {
        let backend = Backend(RefCell::new([0.8, -0.3, 9.0, 12.25, -7.375]));
        let mut appearance = AppearanceDocument::default();
        for projection in [
            harmonigraph_scene::Projection::Perspective,
            harmonigraph_scene::Projection::Orthographic,
            harmonigraph_scene::Projection::Cabinet,
        ] {
            appearance.camera.projection = projection;
            for _ in 0..20 {
                appearance.sync_camera(&backend);
                appearance.view.follow_camera(&mut appearance.camera);
                assert_eq!(appearance.camera_movement(), *backend.0.borrow());
            }
            assert_eq!(appearance.camera.projection, projection);
        }
        let restored = AppearanceDocument::parse(&appearance.serialize()).unwrap();
        appearance = restored;
        backend.0.borrow_mut()[3] = -30.75;
        appearance.sync_camera(&backend);
        assert_eq!(appearance.camera_movement(), *backend.0.borrow());
        assert_eq!(
            ParamKey::CameraDistance.range(),
            harmonigraph_scene::Camera::MIN_DISTANCE..=harmonigraph_scene::Camera::MAX_DISTANCE
        );
        assert_eq!(*ParamKey::CameraPitch.range().end(), harmonigraph_scene::Camera::PITCH_LIMIT);
        assert_eq!(bounded(ParamKey::CameraYaw, std::f32::consts::PI), std::f32::consts::PI);
    }

    /// Counts gestures as the plugin's backend makes them: a `set` outside a
    /// `begin_set` is a whole gesture to the host.
    #[derive(Default)]
    struct Gestures {
        value: std::cell::Cell<f32>,
        open: std::cell::Cell<bool>,
        count: std::cell::Cell<usize>,
        /// Gestures closed by an `end_set`, as against a bare `set`.
        closed: std::cell::Cell<usize>,
    }
    impl ParamBackend for Gestures {
        fn get(&self, _: ParamKey) -> f32 {
            self.value.get()
        }
        fn set(&self, key: ParamKey, value: f32) {
            assert_eq!(key, ParamKey::CameraDistance);
            self.value.set(value);
            if !self.open.get() {
                self.count.set(self.count.get() + 1);
            }
        }
        fn camera_value(&self, key: ParamKey) -> Option<f32> {
            (key == ParamKey::CameraDistance).then(|| self.value.get())
        }
        fn begin_set(&self, key: ParamKey) {
            if key == ParamKey::CameraDistance && !self.open.replace(true) {
                self.count.set(self.count.get() + 1);
            }
        }
        fn end_set(&self, key: ParamKey) {
            if key == ParamKey::CameraDistance && self.open.replace(false) {
                self.closed.set(self.closed.get() + 1);
            }
        }
    }

    #[test]
    fn a_wheel_zoom_of_the_camera_is_one_host_gesture() {
        let ctx = egui::Context::default();
        let mut appearance = AppearanceDocument::default();
        let backend = Gestures::default();
        backend.value.set(appearance.camera.distance);
        let mut zooming_frames = 0;
        // Three wheel notches, then the smoothed scroll running out.
        for frame in 0..40 {
            let mut events = Vec::new();
            if (2..5).contains(&frame) {
                events.push(egui::Event::MouseWheel {
                    unit: egui::MouseWheelUnit::Point,
                    delta: egui::vec2(0.0, 20.0),
                    phase: egui::TouchPhase::Move,
                    modifiers: egui::Modifiers::NONE,
                });
            }
            let _ = ctx.run_ui(egui::RawInput { events, ..Default::default() }, |ui| {
                let before = appearance.camera_movement();
                let scroll = ui.input(|i| i.smooth_scroll_delta.y);
                if scroll != 0.0 {
                    appearance.camera.zoom(scroll);
                    zooming_frames += 1;
                }
                finish_edits(&mut appearance, before, &backend, ui.ctx());
            });
        }
        assert!(zooming_frames >= 3, "the wheel must zoom over several frames: {zooming_frames}");
        assert_eq!(backend.count.get(), 1, "one zoom, one gesture");
        assert!(!backend.open.get(), "the gesture closes once the wheel settles");
    }

    /// A frame of a picture that zooms the camera while a drag holds it, the
    /// way the lattice pane navigates on `dragged()`.
    fn drag_frame(
        ctx: &egui::Context,
        appearance: &mut AppearanceDocument,
        backend: &Gestures,
        events: Vec<egui::Event>,
        strand: bool,
    ) -> bool {
        let mut moved = false;
        let _ = ctx.run_ui(egui::RawInput { events, ..Default::default() }, |ui| {
            if strand {
                // What `end_stranded_drag` does to a drag whose release went
                // missing: egui still believes the button held.
                ui.ctx().stop_dragging();
            }
            let before = appearance.camera_movement();
            let rect = egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(400.0, 400.0));
            let response = ui.interact(rect, egui::Id::new("camera"), egui::Sense::drag());
            if response.dragged() && response.drag_delta().y != 0.0 {
                appearance.camera.zoom(response.drag_delta().y);
                moved = true;
            }
            finish_edits(appearance, before, backend, ui.ctx());
        });
        moved
    }

    fn primary(pos: egui::Pos2, pressed: bool) -> egui::Event {
        egui::Event::PointerButton {
            pos,
            button: egui::PointerButton::Primary,
            pressed,
            modifiers: egui::Modifiers::NONE,
        }
    }

    #[test]
    fn a_drag_of_the_camera_is_one_host_gesture() {
        let ctx = egui::Context::default();
        let mut appearance = AppearanceDocument::default();
        let backend = Gestures::default();
        backend.value.set(appearance.camera.distance);
        let mut at = egui::pos2(200.0, 200.0);
        drag_frame(&ctx, &mut appearance, &backend, vec![egui::Event::PointerMoved(at)], false);
        drag_frame(&ctx, &mut appearance, &backend, vec![primary(at, true)], false);
        let mut moving_frames = 0;
        for _ in 0..10 {
            at.y += 10.0;
            let moved = vec![egui::Event::PointerMoved(at)];
            moving_frames += usize::from(drag_frame(&ctx, &mut appearance, &backend, moved, false));
            assert!(backend.open.get(), "the gesture closed while the drag was held");
        }
        drag_frame(&ctx, &mut appearance, &backend, vec![primary(at, false)], false);
        assert!(moving_frames >= 3, "the drag must move the camera: {moving_frames}");
        assert_eq!(backend.count.get(), 1, "one drag, one gesture");
        assert_eq!(backend.closed.get(), 1, "the release closes it once");
        assert!(!backend.open.get());
    }

    #[test]
    fn a_stranded_drag_closes_its_host_gesture_with_the_button_still_down() {
        let ctx = egui::Context::default();
        let mut appearance = AppearanceDocument::default();
        let backend = Gestures::default();
        backend.value.set(appearance.camera.distance);
        let mut at = egui::pos2(200.0, 200.0);
        drag_frame(&ctx, &mut appearance, &backend, vec![egui::Event::PointerMoved(at)], false);
        drag_frame(&ctx, &mut appearance, &backend, vec![primary(at, true)], false);
        for _ in 0..3 {
            at.y += 10.0;
            drag_frame(&ctx, &mut appearance, &backend, vec![egui::Event::PointerMoved(at)], false);
        }
        assert!(backend.open.get(), "the fixture must open a gesture");
        drag_frame(&ctx, &mut appearance, &backend, vec![], true);
        assert!(ctx.input(|i| i.pointer.any_down()), "the button must still read as held");
        assert!(!backend.open.get(), "a stranded drag held the host gesture open");
        assert_eq!(backend.closed.get(), 1);
    }
}
