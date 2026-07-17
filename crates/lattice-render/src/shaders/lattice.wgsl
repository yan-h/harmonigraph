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
    // x: time (s), y: base node radius (world units),
    // z: octave display mode (0 off, 1 dots, 2 rings, 3 ticks), w: unused
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

// Number of octave slots the indicators display (MIDI octaves 0..9).
const OCTAVE_SLOTS: u32 = 10u;

// Coverage (0..1) of the octave glyph for slot `i` at quad position `uv`.
// Each mode maps the octave index to a different geometry:
//   1 = dots:  satellites orbiting the disc, clock position = octave
//   2 = rings: concentric rings, inner ring = lowest octave
//   3 = ticks: a column right of the disc, bottom tick = lowest octave
fn octave_glyph(mode: u32, i: u32, uv: vec2<f32>, d: f32) -> f32 {
    if mode == 1u {
        // Start at 12 o'clock, go clockwise. (uv.y is up.)
        let ang = 1.5707963 - 6.2831853 * f32(i) / f32(OCTAVE_SLOTS);
        let center = vec2<f32>(cos(ang), sin(ang)) * 0.74;
        return 1.0 - smoothstep(0.055, 0.095, distance(uv, center));
    } else if mode == 2u {
        let r = 0.56 + 0.042 * f32(i);
        return 1.0 - smoothstep(0.008, 0.024, abs(d - r));
    } else {
        let y = -0.62 + 1.24 * f32(i) / f32(OCTAVE_SLOTS - 1u);
        let dx = max(abs(uv.x - 0.76) - 0.10, 0.0);
        let dy = max(abs(uv.y - y) - 0.030, 0.0);
        return 1.0 - smoothstep(0.0, 0.035, dx + dy);
    }
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
    let base_alpha = clamp(disc + glow, 0.0, 1.0);

    // Octave indicators, composited over the disc/glow. Glyphs fade with
    // the same activation envelope as the node so releases decay together.
    let mode = u32(u.misc.z + 0.5);
    let mask = u32(in.params.z + 0.5);
    var glyph = 0.0;
    if mode != 0u && mask != 0u {
        for (var i = 0u; i < OCTAVE_SLOTS; i = i + 1u) {
            if ((mask >> i) & 1u) != 0u {
                glyph = max(glyph, octave_glyph(mode, i, in.uv, d));
            }
        }
        glyph = glyph * (0.35 + 0.65 * activation);
    }
    // Glyphs render in a lifted, whitened version of the node color so
    // they read against both the disc and the background.
    let glyph_rgb = mix(in.color.rgb, vec3<f32>(1.0, 1.0, 1.0), 0.55);

    // "Over" composite: glyph over (disc + glow), premultiplied.
    let alpha = glyph + base_alpha * (1.0 - glyph);
    if alpha < 0.01 {
        discard;
    }
    let out_rgb = glyph_rgb * glyph + rgb * base_alpha * (1.0 - glyph);
    return vec4<f32>(out_rgb, alpha);
}
