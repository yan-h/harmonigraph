//! Camera, projection, and the cached [`Projector`] used for on-screen
//! label placement and node picking.

use crate::Scene;
use glam::{Mat4, Vec2, Vec3};
use harmonigraph_core::LatticePos;

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
///
/// Container-level `default`, like every other persisted struct: `impl
/// Default` below is the one source of a field's fallback, so a blob
/// missing a key costs that key alone rather than failing the parse and
/// taking the whole persist with it.
#[derive(Clone, Copy, Debug, serde::Serialize, serde::Deserialize)]
#[serde(default)]
pub struct Camera {
    pub target: Vec3,
    pub yaw: f32,
    pub pitch: f32,
    pub distance: f32,
    pub fov_y: f32,
    pub projection: Projection,
    /// Cabinet only: on-screen direction of the sevens axis, radians
    /// counterclockwise from screen-right (drafting convention picks 30°,
    /// 45°, or 60°; default 45°).
    pub cabinet_angle: f32,
    /// Cabinet only: screen length of one seventh-step as a fraction of a
    /// front-plane step (0.5 = classic cabinet, 1.0 = cavalier).
    pub cabinet_scale: f32,
}

/// What one pane shows of the sheets, as a world-space rectangle — and
/// whether that rectangle is the whole of it.
///
/// `bounded` is the part worth reading. It is false where a corner of the
/// viewport looks PAST the sheets: the corner's line meets their plane only
/// behind the eye, so what the pane shows runs to the horizon and has no far
/// edge at all. `min`/`max` are then one finite piece of an unbounded wedge
/// rather than its bounds — the far side is an extrapolation of a line going
/// the other way, and the NEAR side is a corner that happens to cross, which
/// is nowhere near the nearest thing on screen. A caller that takes them for
/// edges draws a window with the foreground cut off; see
/// [`ViewConfig::scrolled`](crate::ViewConfig::scrolled) for what it does
/// instead.
///
/// Bounded is the ordinary case and the one the rectangle is for: cabinet at
/// every setting, and perspective and orthographic out to about 0.8 radians of
/// pitch, which is past the tilt anyone reads a lattice at.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct VisibleSheet {
    pub min: Vec2,
    pub max: Vec2,
    pub bounded: bool,
}

impl Default for Camera {
    fn default() -> Self {
        Camera {
            target: Vec3::ZERO,
            yaw: 0.4,
            pitch: 0.3,
            distance: Camera::DEFAULT_DISTANCE,
            fov_y: Camera::DEFAULT_FOV_Y,
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
    /// The far end is what bounds the lattice, the lattice itself being
    /// unbounded: the drawn window is whatever the viewport shows (see
    /// [`ViewConfig::scrolled`](crate::ViewConfig::scrolled)), so pulling back
    /// does not run out of nodes to draw, it asks for more of them. Here the
    /// window is about twenty steps of thirds tall — `distance *
    /// tan(fov/2) * 2` is 19.9 at the default field of view — which is the
    /// point past which a node is too small to read a name on anyway.
    ///
    /// The width that comes with it is the pane's aspect times that, so a wide
    /// pane fully zoomed out is nearer 20x30 than 20x20, and a 16:9 render
    /// 20x35. [`MAX_DRAWN_NODES`](crate::MAX_DRAWN_NODES) is what actually
    /// bounds the work; this is what bounds the PICTURE.
    pub const MAX_DISTANCE: f32 = 24.0;
    /// The framing a fresh view opens at, and the zoom
    /// [`screen_scale`](Self::screen_scale) measures against.
    pub const DEFAULT_DISTANCE: f32 = 12.0;
    /// Fixed: nothing sets the field of view, so the whole of the zoom is the
    /// distance. Named anyway, since [`screen_scale`](Self::screen_scale)'s
    /// arithmetic is only right while the two are read together.
    pub const DEFAULT_FOV_Y: f32 = std::f32::consts::FRAC_PI_4; // 45 degrees

    /// How large a world-space length draws compared to the default framing:
    /// 2 means the lattice is twice the size on screen that it opens at.
    ///
    /// Exact under Orthographic and Cabinet, whose window half-height is
    /// `distance * tan(fov/2)` — the whole of what maps world units to the
    /// viewport. Under Perspective it is true at the focus plane, which is
    /// where the content is: the camera always looks at the lattice it orbits.
    ///
    /// What it is for is the node LABELS. They are typeset in points and the
    /// nodes they name are geometry, so without this the two part company on
    /// every zoom — a name that fits its node at the default distance is a
    /// speck on it zoomed in, and swamps it zoomed out.
    ///
    /// Both terms are read through the range they are navigable in — see
    /// `focus_half_height`, which is where that is done and why it has to be.
    pub fn screen_scale(&self) -> f32 {
        Self::half_height(Self::DEFAULT_DISTANCE, Self::DEFAULT_FOV_Y) / self.focus_half_height()
    }

    /// How many screen POINTS a world-space length draws as, on a viewport
    /// `height` points tall.
    ///
    /// The other half of [`screen_scale`](Self::screen_scale), and the same
    /// arithmetic: that one is a ratio against the default framing, this one is
    /// the framing itself. What it is for is a length the picture holds in
    /// WORLD units — a node's radius, a marker's arm — that something measured
    /// on the pane has to match: the Shadow bar is dialled in node radii, and
    /// the σ every caster is blurred at is in pixels
    /// (`shadow::sigma_px` in harmonigraph-render).
    ///
    /// True at the focus plane under every projection, for the reason
    /// [`screen_scale`](Self::screen_scale) gives: the window's half-height is
    /// `distance * tan(fov/2)` and the viewport's height lands on it.
    pub fn points_per_world(&self, height: f32) -> f32 {
        height.max(0.0) / (2.0 * self.focus_half_height())
    }

    /// The world-space half-height of the window at the focus plane, out of
    /// terms read through the range they are navigable in rather than trusted.
    ///
    /// Nothing the UI does can put either outside it; a hand-edited persisted
    /// blob can, and this scales a FONT SIZE — where a zero distance divides to
    /// infinity, and a field of view anywhere near `PI` sends `tan` through it
    /// and comes back NEGATIVE. What egui does with a size like that is quietly
    /// draw nothing: the glyph is rasterized at a width that saturates to zero
    /// and every label vanishes, which reads as a broken plugin rather than as
    /// a bad number.
    fn focus_half_height(&self) -> f32 {
        let sane = |value: f32, range: std::ops::RangeInclusive<f32>, fallback: f32| {
            if value.is_finite() {
                value.clamp(*range.start(), *range.end())
            } else {
                fallback
            }
        };
        let distance =
            sane(self.distance, Self::MIN_DISTANCE..=Self::MAX_DISTANCE, Self::DEFAULT_DISTANCE);
        // Well short of the half-turn where `tan` changes sign, and off zero,
        // which would divide by it.
        let fov_y = sane(self.fov_y, 0.2..=2.0, Self::DEFAULT_FOV_Y);
        Self::half_height(distance, fov_y)
    }

    fn half_height(distance: f32, fov_y: f32) -> f32 {
        distance * (fov_y * 0.5).tan()
    }

    /// Orbit around the target by a drag delta in pixels.
    pub fn orbit(&mut self, delta: Vec2) {
        self.yaw -= delta.x * Self::ORBIT_SPEED;
        self.pitch =
            (self.pitch + delta.y * Self::ORBIT_SPEED).clamp(-Self::PITCH_LIMIT, Self::PITCH_LIMIT);
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
            self.distance = (self.distance / factor).clamp(Self::MIN_DISTANCE, Self::MAX_DISTANCE);
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

    /// The world-space x/y rectangle this camera shows, across the slab of
    /// depths `z_lo..=z_hi` — `(min, max)`, or `None` where the answer is not
    /// a finite rectangle.
    ///
    /// This is what makes the drawn lattice window derivable rather than a
    /// setting: the lattice's x/y plane is the fifths/thirds sheet, so the
    /// rectangle divided by the spacing IS the range of lattice steps worth
    /// building instances for.
    ///
    /// It reads the answer out of [`view_proj`](Self::view_proj) rather than
    /// re-deriving one per projection, which is the whole of why it is exact
    /// under all three. The set of world points landing on one viewport corner
    /// is a LINE — that is true of any projective map, and it is the only
    /// property used here — so unprojecting the corner at both clip planes
    /// gives two points on it, and where that line crosses each sheet's depth
    /// is a point the corner shows. The bounding box over four corners and the
    /// slab's two ends is the rectangle.
    ///
    /// Only the two ENDS of the slab, not every sheet in it: a corner's line
    /// is straight, so its crossings move monotonically with depth and the
    /// extremes are at the ends. Cabinet's shear and perspective's spread are
    /// both covered by that, differing only in which end is the wide one.
    ///
    /// `None` where a corner's line lies IN the sheets rather than crossing
    /// them, so there is no crossing to take as a bound. That is a narrower
    /// case than it sounds, and it is worth naming precisely rather than as
    /// "a steep camera": the span tested is the line's reach in world z across
    /// the whole clip range, which stays around 13 even at the pitch limit
    /// under perspective. Sweeping 118080 cameras — the full yaw circle, the
    /// whole pitch range, five aspects, three distances, three projections —
    /// it is answered for 1232 of them, every one orthographic with yaw within
    /// a rounding error of a quarter turn.
    ///
    /// So this is not what handles the camera whose honest window is
    /// unbounded — that is [`VisibleSheet::bounded`], and
    /// [`MAX_DRAWN_NODES`](crate::MAX_DRAWN_NODES) is what bounds the work
    /// there. This one case is the degenerate matrix, and the caller wants its
    /// own fallback for it.
    pub fn visible_world_bounds(&self, aspect: f32, z_lo: f32, z_hi: f32) -> Option<VisibleSheet> {
        let inverse = self.view_proj(aspect).inverse();
        // A world point from a clip-space one, perspective divide included —
        // orthographic and cabinet both leave w at 1, so this is a no-op for
        // them and the one path still serves all three.
        let unproject = |ndc: Vec3| {
            let world = inverse * ndc.extend(1.0);
            (world.w.abs() > 1e-6).then(|| world.truncate() / world.w)
        };
        let (mut min, mut max) = (Vec2::splat(f32::INFINITY), Vec2::splat(f32::NEG_INFINITY));
        let mut bounded = true;
        for (nx, ny) in [(-1.0, -1.0), (1.0, -1.0), (-1.0, 1.0), (1.0, 1.0)] {
            // wgpu clip depth runs 0 (near) to 1 (far); both constructors in
            // `view_proj` produce it, which is what lets these be literals.
            let near = unproject(Vec3::new(nx, ny, 0.0))?;
            let far = unproject(Vec3::new(nx, ny, 1.0))?;
            let span = far.z - near.z;
            // The corner's line lying in a sheet rather than crossing one:
            // no crossing to find, and the span it shows is every x and y
            // there is.
            if span.abs() < 1e-4 {
                return None;
            }
            for z in [z_lo, z_hi] {
                let t = (z - near.z) / span;
                // Where along the corner's line the sheet is. Outside 0..=1
                // the line meets the sheet outside the drawn depth — behind
                // the eye, or past the far plane — so the corner is looking
                // PAST the sheets and the point below is an extrapolation
                // rather than anything the pane shows. The extrapolation is
                // still taken, because the three corners that do cross bound
                // nothing on their own, but the caller is told.
                bounded &= (0.0..=1.0).contains(&t);
                let crossing = (near + (far - near) * t).truncate();
                min = min.min(crossing);
                max = max.max(crossing);
            }
        }
        (min.is_finite() && max.is_finite()).then_some(VisibleSheet { min, max, bounded })
    }

    /// Camera-space right/up axes in world space, for billboarding.
    pub fn right_up(&self) -> (Vec3, Vec3) {
        let view = self.view();
        // Rows of the rotation part of the view matrix.
        let right = Vec3::new(view.x_axis.x, view.y_axis.x, view.z_axis.x);
        let up = Vec3::new(view.x_axis.y, view.y_axis.y, view.z_axis.y);
        (right, up)
    }

    /// Fit a deserialized camera to what its own controls can produce.
    ///
    /// A bar cannot produce a nonsense value but a hand-edited blob can, and
    /// this feeds `view_proj` every frame the camera is drawn with —
    /// `screen_scale` already guards `distance` and `fov_y` locally for the
    /// font-size math it does, but that guard is local to it, and nothing
    /// stands between a NaN in the blob and the `tan` in `ortho`. Same
    /// argument as [`ViewConfig::sanitize`], and the same split: finite-only
    /// where a field has no natural bound, clamped into the working range
    /// where it does.
    ///
    /// [`ViewConfig::sanitize`]: crate::ViewConfig::sanitize
    pub fn sanitize(&mut self) {
        let fresh = Camera::default();

        // No bound the geometry cares about — the camera can look anywhere —
        // so only the NaN or infinity a hand-edited blob can carry gets
        // repaired, and as a whole vector: `eye()` reads all three
        // components together, and one bad component NaNs the other two
        // through it.
        self.target = if self.target.is_finite() { self.target } else { fresh.target };

        // Periodic (every use is a `sin`/`cos`), so any finite value draws
        // correctly — only a non-finite one, which `sin`/`cos` turn into
        // NaN, needs a fallback.
        self.yaw = finite_or(self.yaw, fresh.yaw);

        // `orbit` holds this to `PITCH_LIMIT` on every interactive drag; a
        // hand-edited blob is not `orbit`.
        self.pitch =
            finite_or(self.pitch, fresh.pitch).clamp(-Self::PITCH_LIMIT, Self::PITCH_LIMIT);

        // The range `zoom`/`zoom_by` already hold interactive input to; load
        // is the one door those two don't cover.
        self.distance =
            finite_or(self.distance, fresh.distance).clamp(Self::MIN_DISTANCE, Self::MAX_DISTANCE);

        // Nothing in the UI sets this — see the field's own doc, "nothing
        // sets the field of view" — so the only value a session ever chose
        // is the default; a blob can still carry a stale or hand-edited one,
        // and `ortho`'s `tan` is silent about a bad angle rather than
        // panicking on it. Held to the range `screen_scale` already trusts,
        // for the same reason it does.
        self.fov_y = finite_or(self.fov_y, fresh.fov_y).clamp(0.2, 2.0);

        // The two cabinet knobs, ranged to what their own bars offer
        // ("Sevenths angle" 0..=90°, "Sevenths length" 0.1..=1.0).
        self.cabinet_angle = finite_or(self.cabinet_angle, fresh.cabinet_angle)
            .clamp(0.0, std::f32::consts::FRAC_PI_2);
        self.cabinet_scale = finite_or(self.cabinet_scale, fresh.cabinet_scale).clamp(0.1, 1.0);
    }
}

/// `value` if it is a real number, `fallback` if it is a NaN or an infinity
/// — the guard `clamp` cannot be, NaN being its own answer to every
/// comparison a clamp makes.
fn finite_or(value: f32, fallback: f32) -> f32 {
    if value.is_finite() {
        value
    } else {
        fallback
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
    /// Only [visible](crate::NodeInstance::is_visible) nodes are pickable, so the
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

#[cfg(test)]
mod tests {
    use super::*;

    /// The zoom factor labels are sized by is finite and positive for anything
    /// a persisted blob can carry.
    ///
    /// Nothing in the UI writes a bad camera — `zoom`, `zoom_by` and `Default`
    /// all clamp — so the only route in is a hand-edited or corrupted blob,
    /// which is the route this codebase guards every other divisor against
    /// (see `SpectrumConfig::sanitize`). It matters here because the
    /// factor multiplies a font size, and egui answers a nonsense one by
    /// rasterizing nothing: the labels do not draw wrong, they disappear.
    #[test]
    fn a_hand_edited_camera_still_yields_a_usable_label_scale() {
        let at = |distance: f32, fov_y: f32| {
            Camera { distance, fov_y, ..Default::default() }.screen_scale()
        };
        // The framing a fresh view opens at is the identity, by construction.
        assert_eq!(at(Camera::DEFAULT_DISTANCE, Camera::DEFAULT_FOV_Y), 1.0);
        // Twice as close draws twice the size; the working range is bounded at
        // both ends, so the factor is too.
        assert_eq!(at(Camera::DEFAULT_DISTANCE * 0.5, Camera::DEFAULT_FOV_Y), 2.0);
        // Finite and positive is the whole of what is owed here: what a label
        // may finally be SIZED at is bounded downstream, by `text::snap_scale`.
        let inside = |scale: f32| scale.is_finite() && scale > 0.0 && scale < 100.0;
        for distance in [0.0, -1.0, f32::NAN, f32::INFINITY, 1e30, -1e30] {
            let scale = at(distance, Camera::DEFAULT_FOV_Y);
            assert!(inside(scale), "distance {distance} gave a scale of {scale}");
        }
        // A field of view near a half turn sends `tan` through infinity and
        // back negative, which is the one of these that is not obviously bad.
        for fov_y in [0.0, -1.0, f32::NAN, std::f32::consts::PI, 4.0, 1e-30, 1e30] {
            let scale = at(Camera::DEFAULT_DISTANCE, fov_y);
            assert!(inside(scale), "fov {fov_y} gave a scale of {scale}");
        }
    }
}
