// Instanced billboard rendering of lattice nodes.
//
// Each instance is one node, expanded into a camera-facing quad in the
// vertex shader; the fragment shader draws a soft disc with a glow whose
// strength follows the node's activation. Skins/effects iterate here:
// this file is the main thing to edit when trying a new look.

struct Uniforms {
    view_proj: mat4x4<f32>,
    cam_right: vec4<f32>,
    cam_up: vec4<f32>,
    // x: time (s), y: base node radius (world units), z/w: unused
    misc: vec4<f32>,
};

@group(0) @binding(0) var<uniform> u: Uniforms;

struct Instance {
    @location(0) world_pos: vec3<f32>,
    @location(1) color: vec4<f32>,
    // x: activation 0..1, y: hovered 0/1, z: octave_mask (bits as f32), w: unused
    @location(2) params: vec4<f32>,
};

struct VsOut {
    @builtin(position) clip_pos: vec4<f32>,
    @location(0) uv: vec2<f32>, // -1..1 across the quad
    @location(1) color: vec4<f32>,
    @location(2) params: vec4<f32>,
};

@vertex
fn vs_main(@builtin(vertex_index) vertex_index: u32, inst: Instance) -> VsOut {
    var corners = array<vec2<f32>, 4>(
        vec2<f32>(-1.0, -1.0),
        vec2<f32>(1.0, -1.0),
        vec2<f32>(-1.0, 1.0),
        vec2<f32>(1.0, 1.0),
    );
    let corner = corners[vertex_index];

    let activation = inst.params.x;
    let hovered = inst.params.y;

    // Lit nodes grow a little; hover grows a little more. The quad is twice
    // the disc radius to leave room for the glow.
    let radius = u.misc.y * (0.55 + 0.35 * activation + 0.15 * hovered) * 2.0;

    let world = inst.world_pos
        + (u.cam_right.xyz * corner.x + u.cam_up.xyz * corner.y) * radius;

    var out: VsOut;
    out.clip_pos = u.view_proj * vec4<f32>(world, 1.0);
    out.uv = corner;
    out.color = inst.color;
    out.params = inst.params;
    return out;
}

@fragment
fn fs_main(in: VsOut) -> @location(0) vec4<f32> {
    let d = length(in.uv); // 0 at center, 1 at quad edge (2x disc radius)
    let activation = in.params.x;
    let hovered = in.params.y;

    // Solid disc occupies the inner half of the quad.
    let disc = 1.0 - smoothstep(0.42, 0.5, d);
    // Soft additive-looking glow for active nodes. The exponential alone
    // never reaches zero, so the quad boundary showed as a boxy halo;
    // window it so it fades to exactly zero (with zero slope) inside the
    // quad edge.
    let window = 1.0 - smoothstep(0.5, 0.95, d);
    let glow = (0.6 * activation + 0.25 * hovered) * exp(-3.0 * d) * window;

    let brightness = 0.35 + 0.65 * activation + 0.2 * hovered;
    let rgb = in.color.rgb * brightness;

    let alpha = clamp(disc + glow, 0.0, 1.0);
    if alpha < 0.01 {
        discard;
    }
    // Premultiplied alpha output (pipeline blend state expects it).
    return vec4<f32>(rgb * alpha, alpha);
}
