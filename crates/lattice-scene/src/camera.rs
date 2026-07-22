//! Camera, projection, and the cached [`Projector`] used for on-screen
//! label placement and node picking.

use crate::Scene;
use lattice_core::LatticePos;
use glam::{Mat4, Vec2, Vec3};

/// How the camera maps depth to the screen.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default, serde::Serialize, serde::Deserialize)]
pub enum Projection {
    /// Classic perspective: farther content converges and shrinks.
    #[default]
    Perspective,
    /// Orthographic ("isometric-style"): uniform scale at every depth, so
    /// equal intervals render at equal screen offsets everywhere and
    /// parallel lattice lines stay parallel. Depth then reads only
    /// through the deliberate cues (node depth-scale, occlusion).
    Orthographic,
    /// Cabinet (oblique): the camera faces the fifths/thirds sheet
    /// straight on (orbit is ignored; the UI turns plain drags into
    /// pans), which renders that primary plane with zero distortion, and
    /// the sevens axis shears to a uniform screen offset — every
    /// seventh-step is the same arrow anywhere on screen. Direction and
    /// length of that arrow are [`Camera::cabinet_angle`] and
    /// [`Camera::cabinet_scale`].
    Cabinet,
}

fn default_cabinet_angle() -> f32 {
    45f32.to_radians()
}

fn default_cabinet_scale() -> f32 {
    0.6
}

/// Simple orbit camera. Angles in radians.
#[derive(Clone, Copy, Debug, serde::Serialize, serde::Deserialize)]
pub struct Camera {
    pub target: Vec3,
    pub yaw: f32,
    pub pitch: f32,
    pub distance: f32,
    pub fov_y: f32,
    /// serde(default) keeps pre-projection persisted blobs loadable.
    #[serde(default)]
    pub projection: Projection,
    /// Cabinet only: on-screen direction of the sevens axis, radians
    /// counterclockwise from screen-right (drafting convention picks 30°,
    /// 45°, or 60°; default 45°).
    #[serde(default = "default_cabinet_angle")]
    pub cabinet_angle: f32,
    /// Cabinet only: screen length of one seventh-step as a fraction of a
    /// front-plane step (0.5 = classic cabinet, 1.0 = cavalier).
    #[serde(default = "default_cabinet_scale")]
    pub cabinet_scale: f32,
}

impl Default for Camera {
    fn default() -> Self {
        Camera {
            target: Vec3::ZERO,
            yaw: 0.4,
            pitch: 0.3,
            distance: 12.0,
            fov_y: 45f32.to_radians(),
            projection: Projection::Cabinet,
            cabinet_angle: default_cabinet_angle(),
            cabinet_scale: default_cabinet_scale(),
        }
    }
}

impl Camera {
    /// Yaw/pitch radians per dragged pixel.
    const ORBIT_SPEED: f32 = 0.01;
    /// Pan speed per pixel, scaled by distance so a pan feels the same at
    /// any zoom.
    const PAN_SPEED: f32 = 0.0016;
    /// Multiplicative zoom rate per scroll unit.
    const ZOOM_RATE: f32 = 0.002;
    /// Keep pitch shy of the poles so look_at's up vector stays valid.
    pub const PITCH_LIMIT: f32 = 1.5;
    /// Zoom clamp; CLIP_NEAR/CLIP_FAR must bracket it with margin.
    pub const MIN_DISTANCE: f32 = 2.0;
    pub const MAX_DISTANCE: f32 = 80.0;

    /// Orbit around the target by a drag delta in pixels.
    pub fn orbit(&mut self, delta: Vec2) {
        self.yaw -= delta.x * Self::ORBIT_SPEED;
        self.pitch = (self.pitch + delta.y * Self::ORBIT_SPEED)
            .clamp(-Self::PITCH_LIMIT, Self::PITCH_LIMIT);
    }

    /// Pan the target by a drag delta in pixels, grab-style: the content
    /// follows the pointer.
    pub fn pan(&mut self, delta: Vec2) {
        let (right, up) = self.right_up();
        let k = self.distance * Self::PAN_SPEED;
        self.target += up * (delta.y * k) - right * (delta.x * k);
    }

    /// Zoom by a scroll delta (positive = closer), clamped to the working
    /// range.
    pub fn zoom(&mut self, scroll: f32) {
        self.distance = (self.distance * (1.0 - scroll * Self::ZOOM_RATE))
            .clamp(Self::MIN_DISTANCE, Self::MAX_DISTANCE);
    }

    /// Zoom by a multiplicative factor (>1 = closer), clamped to the working
    /// range. This is the pinch/zoom-gesture counterpart to [`Self::zoom`]:
    /// egui reports trackpad pinches and modifier+scroll as a `zoom_delta`
    /// factor rather than a scroll delta, and the lattice honors both.
    pub fn zoom_by(&mut self, factor: f32) {
        if factor > 0.0 {
            self.distance =
                (self.distance / factor).clamp(Self::MIN_DISTANCE, Self::MAX_DISTANCE);
        }
    }

    pub fn eye(&self) -> Vec3 {
        // Cabinet is a fixed-viewpoint projection: the eye always faces
        // the fifths/thirds sheet straight on, whatever yaw/pitch say (they
        // keep their values for when the user switches back).
        let dir = if self.projection == Projection::Cabinet {
            Vec3::Z
        } else {
            Vec3::new(
                self.pitch.cos() * self.yaw.sin(),
                self.pitch.sin(),
                self.pitch.cos() * self.yaw.cos(),
            )
        };
        self.target + dir * self.distance
    }

    pub fn view(&self) -> Mat4 {
        Mat4::look_at_rh(self.eye(), self.target, Vec3::Y)
    }

    /// The orthographic projection whose window is the perspective frustum's
    /// cross-section at the target. Shared by the `Orthographic` and
    /// `Cabinet` projections (Cabinet post-multiplies a shear onto it).
    fn ortho(&self, aspect: f32) -> Mat4 {
        let half_h = self.distance * (self.fov_y * 0.5).tan();
        let half_w = half_h * aspect;
        Mat4::orthographic_rh(-half_w, half_w, -half_h, half_h, CLIP_NEAR, CLIP_FAR)
    }

    pub fn view_proj(&self, aspect: f32) -> Mat4 {
        // Both glam _rh constructors produce 0..1 clip-space depth, which
        // is what wgpu expects.
        let aspect = aspect.max(0.01);
        let proj = match self.projection {
            Projection::Perspective => {
                Mat4::perspective_rh(self.fov_y, aspect, CLIP_NEAR, CLIP_FAR)
            }
            // The ortho window is the perspective frustum's cross-section
            // at the target, so toggling projections keeps the framing at
            // the focus plane, and zoom (distance) keeps scaling the view.
            Projection::Orthographic => self.ortho(aspect),
            // Orthographic window plus a shear: view-space depth relative
            // to the focus plane becomes a screen offset along
            // `cabinet_angle` scaled by `cabinet_scale`. The focus plane
            // itself is unmoved (the shear's translation term cancels
            // there), so framing still matches the other projections.
            Projection::Cabinet => {
                let ortho = self.ortho(aspect);
                let (sin, cos) = self.cabinet_angle.sin_cos();
                let (kx, ky) = (self.cabinet_scale * cos, self.cabinet_scale * sin);
                let mut shear = Mat4::IDENTITY;
                shear.z_axis.x = kx;
                shear.z_axis.y = ky;
                shear.w_axis.x = kx * self.distance;
                shear.w_axis.y = ky * self.distance;
                ortho * shear
            }
        };
        proj * self.view()
    }

    /// Camera-space right/up axes in world space, for billboarding.
    pub fn right_up(&self) -> (Vec3, Vec3) {
        let view = self.view();
        // Rows of the rotation part of the view matrix.
        let right = Vec3::new(view.x_axis.x, view.y_axis.x, view.z_axis.x);
        let up = Vec3::new(view.x_axis.y, view.y_axis.y, view.z_axis.y);
        (right, up)
    }
}

/// Camera clip planes; must bracket Camera::MIN/MAX_DISTANCE with margin.
const CLIP_NEAR: f32 = 0.1;
const CLIP_FAR: f32 = 200.0;

/// Cached projection for one (camera, viewport) pair: build once, then
/// project many points (labels, picking) without rebuilding the
/// view-projection matrix per node.
pub struct Projector {
    view_proj: Mat4,
    viewport_px: Vec2,
}

impl Projector {
    /// Project a world position into viewport pixels (origin top-left).
    /// `None` when behind the camera.
    pub fn project(&self, world: Vec3) -> Option<Vec2> {
        let clip = self.view_proj * world.extend(1.0);
        // Behind the camera (or nearer than the near plane): perspective
        // flips w negative there; orthographic keeps w at 1 and instead
        // sends clip z below the near plane's 0. Test both so the check
        // holds under either projection.
        if clip.w <= 0.0 || clip.z < 0.0 {
            return None;
        }
        let ndc = clip.truncate() / clip.w;
        Some(Vec2::new(
            (ndc.x * 0.5 + 0.5) * self.viewport_px.x,
            (1.0 - (ndc.y * 0.5 + 0.5)) * self.viewport_px.y,
        ))
    }
}

impl Scene {
    pub fn projector(&self, viewport_px: Vec2) -> Projector {
        Projector {
            view_proj: self.camera.view_proj(viewport_px.x / viewport_px.y.max(1.0)),
            viewport_px,
        }
    }

    /// Convenience for one-off projections; per-node loops should reuse a
    /// [`Scene::projector`].
    pub fn project(&self, viewport_px: Vec2, world: Vec3) -> Option<Vec2> {
        self.projector(viewport_px).project(world)
    }

    /// CPU picking: the node whose screen projection is nearest the pointer,
    /// within `max_px`. Every pane that wants "hover a pitch class" uses
    /// this and writes the result to the shared UI state.
    ///
    /// Only [visible](NodeInstance::is_visible) nodes are pickable, so the
    /// pointer can't pull a pitch readout out of an off-sheet node that
    /// draws nothing.
    pub fn pick(&self, viewport_px: Vec2, pointer_px: Vec2, max_px: f32) -> Option<LatticePos> {
        let projector = self.projector(viewport_px);
        let mut best: Option<(f32, LatticePos)> = None;
        for node in &self.nodes {
            if !node.is_visible() {
                continue;
            }
            let Some(px) = projector.project(node.world_pos) else {
                continue;
            };
            let d = px.distance(pointer_px);
            if d <= max_px && best.is_none_or(|(bd, _)| d < bd) {
                best = Some((d, node.lattice_pos));
            }
        }
        best.map(|(_, pos)| pos)
    }
}
