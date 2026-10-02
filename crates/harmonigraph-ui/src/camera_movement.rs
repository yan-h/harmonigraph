//! Host-owned camera movement is projected into the live appearance each frame.
//! Absolute pan includes the lattice center that follow_camera rebases.

use crate::params::ParamKey;
use crate::{AppearanceDocument, ParamBackend};

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

    fn set_camera_movement(&mut self, values: [f32; 5]) {
        self.camera.yaw = values[0];
        self.camera.pitch = values[1];
        self.camera.distance = values[2];
        self.view.center_fives = values[3].round() as i32;
        self.view.center_threes = values[4].round() as i32;
        self.camera.target.x = values[3] - self.view.center_fives as f32;
        self.camera.target.y = values[4] - self.view.center_threes as f32;
        self.camera.target.z = 0.0;
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
    let dragging = crate::kept_focus(ctx) && ctx.input(|i| i.pointer.any_down());
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
}
