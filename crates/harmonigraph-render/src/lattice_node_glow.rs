//! Lattice node light: projected gather candidates and tile lists.
//! The egui mark-halo callback is the separate `glow` module.

use super::*;

pub(super) mod tiles;

/// One lit node, as the light's own pass reads it: `GlowNode` in lattice.wgsl,
/// which is where each field is argued.
///
/// A read-only storage buffer and not a vertex stream, because the gather has
/// no geometry per node to expand — the pass is one quad over the whole target
/// and this is the list its fragment stage walks (`shadow_casters` in
/// common.wgsl is the same shape for the same reason).
///
/// Ten floats, so the WGSL struct's own alignment of 8 makes the array stride
/// exactly this struct's size; nothing here is padded to a vec4 it does not
/// fill.
#[repr(C)]
#[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
pub(super) struct GpuGlowNode {
    pub(super) inv_x: [f32; 2],
    pub(super) inv_y: [f32; 2],
    pub(super) centre: [f32; 2],
    pub(super) light: [f32; 2],
    /// Mark envelope and conservative halo radius in target pixels (for tiling).
    pub(super) mark: [f32; 2],
}

/// What binds that list to the light's pass: one read-only storage buffer, at
/// group 2.
///
/// FRAGMENT alone, unlike `shadow::caster_layout`'s pair — the gather's vertex
/// stage is four corners and reads nothing.
pub(super) fn glow_node_layout(device: &wgpu::Device) -> wgpu::BindGroupLayout {
    shadow::storage_list_layout(device, "lattice_glow_nodes_layout", wgpu::ShaderStages::FRAGMENT)
}

/// A buffer for `capacity` lit nodes and the bind group naming it
/// (`shadow::storage_list`, which holds why the two come as one).
///
/// Keyed on the CAPACITY and on nothing else — a frame writes its own nodes
/// into the buffer it finds and rebuilds neither object, so lighting one more
/// node than last frame costs an upload and not a bind group.
pub(super) fn glow_node_buffer(
    device: &wgpu::Device,
    layout: &wgpu::BindGroupLayout,
    capacity: usize,
) -> (wgpu::Buffer, wgpu::BindGroup) {
    shadow::storage_list::<GpuGlowNode>(device, layout, capacity, "lattice_glow_nodes")
}

impl LatticeCallback {
    /// Whether this frame's view asks for a node glow at all — a reach to
    /// spread it over and a strength to draw it at, which `from_scene` has
    /// already reduced to one number. False and nothing is allocated, encoded
    /// or composited: no target, no pass, and every wash reading the stand-in
    /// transparent texture rather than a light. The SHADOW is not gated by it
    /// — an item casts with no light in the picture (see
    /// [`ShadowParams`]).
    pub(super) fn glow_draws(&self) -> bool {
        self.uniforms.glow.reach > 0.0
    }

    /// This frame's lit nodes, each carrying the map from a pixel of a `size`
    /// target back into that node's own uv — the list `fs_glow_gather` walks.
    ///
    /// The set the billboard pass used to light, less the nodes whose halo
    /// cannot reach the target at all. The pass drew every shipped instance
    /// whose carried level is above zero and discarded the rest a fragment at a
    /// time; a gather pays instead for a guard on every candidate it
    /// carries, so a node the guard can only ever answer "no" for is dropped
    /// here. Whether a node's CENTRE is on screen still decides nothing — a
    /// halo reaches well past its node, and one whose middle sits off the pane
    /// lights the pixels it reaches ([`halo_pixels`] is what says how far).
    /// Sheets are not distinguished either; the fold is commutative, so this is
    /// one list in instance order.
    ///
    /// The frame is inverted HERE, once per node, because the alternative is
    /// three matrix multiplies per node per pixel inside the loop. It is exact:
    /// a billboard lies in the camera's own right/up plane, so one projection
    /// depth covers the whole of it and the projection restricted to that plane
    /// is a scale and an offset — a 2x2 basis to invert, under perspective as
    /// under an orthographic camera.
    ///
    /// Three kinds of node are dropped rather than mapped, and each is one the
    /// rasterizer already dropped — a pass with no quads in it has to drop
    /// them somewhere.
    ///
    /// A node the projection cannot place — at or behind the eye — had every
    /// corner clipped away. So did a node the frustum excludes in DEPTH, which
    /// is the one that is not obvious and the one that bit: the billboard lies
    /// in the camera's right/up plane, so all four of its corners share one
    /// depth and the primitive is clipped whole rather than trimmed. A node
    /// nearer than the near plane therefore lit NOTHING, however much of the
    /// pane its halo reached — and #680's own fixture holds one, a lattice
    /// corner that the steeply pitched perspective camera puts 0.08 in front of
    /// an eye whose near plane is at 0.1. Gathering it lights the whole frame
    /// with a node the billboard pass never drew.
    ///
    /// And a node whose basis is degenerate had a quad of no area. Neither lit
    /// anything, and a singular matrix has no inverse to write down.
    ///
    /// The fourth drop is the gather's own and is picture-identical rather than
    /// inherited: a node whose halo disc misses the target rectangle. What the
    /// shader would compute for it is exactly zero everywhere. The retained
    /// discs also feed [`tiles::pack`], so zooming out to thousands of lit
    /// nodes does not make every pixel check the whole on-screen list either.
    pub(super) fn glow_nodes(&self, size: [u32; 2]) -> Vec<GpuGlowNode> {
        let view_proj =
            glam::Mat4::from_cols_array_2d(&self.uniforms.camera.view_proj.0.map(|c| c.0));
        let axis = |v: Float4| glam::Vec3::new(v.0[0], v.0[1], v.0[2]);
        let (right, up) = (axis(self.uniforms.camera.right), axis(self.uniforms.camera.up));
        let pixels = glam::vec2(size[0] as f32, size[1] as f32);
        // The viewport transform the fragment stage's `@builtin(position)` is
        // on the far side of: the glow pass covers its whole attachment, so
        // this is the target's own pixels with no offset in it.
        let to_pixels = |p: glam::Vec3| project_onto(&view_proj, pixels, p);
        self.instances
            .iter()
            .filter(|inst| inst.glow[0] > 0.0)
            .filter_map(|inst| {
                // One node uv in world units, as `node_vertex` spends it: the
                // quad's own margin cancels against the uv it hands out, so the
                // map is the same whatever margin sized the billboard.
                //
                // Off `u.node.radius`, which is what `node_vertex` reads, and
                // not `u.marker.world_unit`: the two are one number out of
                // `derive_scene` but a fixture that sets `Scene::node_radius`
                // by hand moves only the first.
                let uv_world = self.uniforms.node.radius * 1.8 * inst.scale.max(0.05);
                let at = glam::Vec3::from(inst.world_pos);
                let (centre, depth) = to_pixels(at)?;
                // The frustum's depth range, asked once for the whole quad: its
                // four corners share this node's depth, so the rasterizer either
                // kept all of them or none.
                if !(0.0..=1.0).contains(&depth) {
                    return None;
                }
                let (r, u) = (
                    to_pixels(at + right * uv_world)?.0 - centre,
                    to_pixels(at + up * uv_world)?.0 - centre,
                );
                // Off the pane entirely: the halo's disc, at the largest radius
                // this frame's bars can give it, does not touch the target.
                // Measured against the rectangle rather than its corners so a
                // node sitting off one EDGE with its light across the pane is
                // kept — the nearest point of the target to the centre is the
                // one the disc reaches first.
                let closest = centre.clamp(glam::Vec2::ZERO, pixels);
                let radius = halo_pixels(&self.uniforms, r, u);
                if closest.distance_squared(centre) > radius.powi(2) {
                    return None;
                }
                // `d = r * uv.x + u * uv.y` inverted: the columns are r and u,
                // so this is the adjugate over the determinant.
                let det = r.x * u.y - u.x * r.y;
                if det.abs() < 1e-9 {
                    return None;
                }
                Some(GpuGlowNode {
                    inv_x: [u.y / det, -u.x / det],
                    inv_y: [-r.y / det, r.x / det],
                    centre: centre.to_array(),
                    // Modulate only the displayed light. Feeding this back
                    // into InkHistory would change its attack/release decision
                    // and colour history on every breath.
                    light: [
                        inst.glow[0]
                            * self
                                .glow_timing
                                .map_or(1.0, |clock| breath(inst.world_pos, clock.now)),
                        inst.glow[1],
                    ],
                    mark: [inst.glow[3], radius],
                })
            })
            .collect()
    }
}

/// Stable world position gives each node a phase independent of draw order,
/// culling, and recycled ink rows. Only carried live/offline light breathes;
/// clockless renderer callers continue to supply their exact light level.
fn breath(position: [f32; 3], now: f64) -> f32 {
    let phase = f64::from(position[0]) * 2.173
        + f64::from(position[1]) * 3.719
        + f64::from(position[2]) * 5.137;
    (0.91 + 0.06 * (now * 0.73 + phase).sin() + 0.03 * (now * 1.13 + phase * 1.7).sin()) as f32
}

/// An upper bound on how far one lit node's halo reaches from its centre, in
/// the pixels of the target it is gathered into. `r` and `u` are the node's own
/// uv axes as pixel vectors, which is the frame
/// [`LatticeCallback::glow_nodes`] inverts.
///
/// A BOUND and not the exact extent, because the one thing it is read for is a
/// cull: too large keeps a node that lights nothing, which costs a loop
/// iteration, and too small drops a node that lights something, which is a hole
/// in the picture. It is loose in the RIM, and loose toward keeping.
///
/// In uv the halo stops at `glow_layer`'s `span` — the rim the LIGHT is measured
/// against plus the Reach, floored where the shader floors it. `glow_rim` eases
/// that rim between `node_rim`'s two answers on the mark this node carries, and
/// the marked answer is the larger of the two, so taking it once for the frame
/// bounds every node whatever any of them carries. That is the whole of the
/// slack: a frame with no marks in it is bounded by the mark's rim anyway.
///
/// From uv to pixels the halo's disc maps to an ELLIPSE, whose semi-major axis
/// is the largest singular value of the 2x2 frame `[r u]`. Written out rather
/// than bounded by `|r| + |u|` or the Frobenius norm, both of which are up to
/// √2 too wide on the square frame an orthographic camera hands every node —
/// and a bound √2 too wide in RADIUS keeps twice the area's worth of nodes off
/// the pane, which is the cost this is here to remove.
pub(super) fn halo_pixels(uniforms: &Uniforms, r: glam::Vec2, u: glam::Vec2) -> f32 {
    let node = &uniforms.node;
    let bare = node.rings_outer.max(0.0);
    let rim = if node.mark_thickness > 0.0 {
        bare.max(node.mark_inner + node.mark_thickness)
    } else {
        bare
    };
    let span = (rim + uniforms.glow.reach.max(0.0)).max(0.1);
    // The larger eigenvalue of `[r u]^T [r u]`, whose root is that singular
    // value: half the trace plus the root of the discriminant. Both halves are
    // non-negative, so no floor is wanted under the root — one at zero would
    // fire on nothing but a NaN, and would turn it into a bound of zero, which
    // is the DROP side. A NaN left alone fails the comparison at the call site
    // instead and keeps the node, which is the side a bound is loose toward.
    let (a, b, c) = (r.length_squared(), u.length_squared(), r.dot(u));
    let half = (a + b) * 0.5;
    let off = (a - b) * 0.5;
    span * (half + (off * off + c * c).sqrt()).sqrt()
}
