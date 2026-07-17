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
    // z: octave display mode (0 off, 1 dots, 2 rings, 3..6 ticks),
    // w: node style (0 steady, 1 breathe, 2 corona, 3 sparks, 4 wire)
    misc: vec4<f32>,
};

const TAU: f32 = 6.2831853;

@group(0) @binding(0) var<uniform> u: Uniforms;

struct Instance {
    @location(0) world_pos: vec3<f32>,
    @location(1) color: vec4<f32>,
    // x: activation 0..1, y: hovered 0/1, z: phase (note start time, s),
    // w: outlined 0/1 (channel-14 voices render as a ring, not a disc)
    @location(2) params: vec4<f32>,
    // Per-octave activation, 8 bits per slot, little-endian packed.
    @location(3) octaves: vec3<u32>,
};

struct VsOut {
    @builtin(position) clip_pos: vec4<f32>,
    @location(0) uv: vec2<f32>, // -1..1 across the quad
    @location(1) color: vec4<f32>,
    @location(2) params: vec4<f32>,
    @location(3) @interpolate(flat) octaves: vec3<u32>,
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
    var radius = u.misc.y * (0.55 + 0.35 * activation + 0.15 * hovered) * 2.0;

    // Breathe style: held nodes pulse in size, each on its own phase
    // (oscillation starts neutral at note-on).
    if u32(u.misc.w + 0.5) == 1u {
        let beat = (u.misc.x - inst.params.z) * TAU * 0.8;
        radius = radius * (1.0 + 0.10 * activation * sin(beat));
    }

    let world = inst.world_pos
        + (u.cam_right.xyz * corner.x + u.cam_up.xyz * corner.y) * radius;

    var out: VsOut;
    out.clip_pos = u.view_proj * vec4<f32>(world, 1.0);
    out.uv = corner;
    out.color = inst.color;
    out.params = inst.params;
    out.octaves = inst.octaves;
    return out;
}

// Number of octave slots the indicators display (MIDI octaves 0..9).
const OCTAVE_SLOTS: u32 = 10u;

// Activation level (0..1) of octave slot `i`, unpacked from 8-bit fields.
// Each octave carries its OWN envelope so indicators fade independently
// (a released C5 decays even while C4 holds the node fully lit).
fn octave_level(octaves: vec3<u32>, i: u32) -> f32 {
    let word = octaves[i / 4u];
    return f32((word >> ((i % 4u) * 8u)) & 0xFFu) / 255.0;
}

// Tick column geometry (modes 3..6), all in quad UV units.
const TICK_X: f32 = 0.76; // column center
const TICK_HALF_W: f32 = 0.10;
const TICK_HALF_H: f32 = 0.030;
const TICK_Y_MIN: f32 = -0.62;
const TICK_Y_MAX: f32 = 0.62;

fn tick_slot_y(i: u32) -> f32 {
    return TICK_Y_MIN + (TICK_Y_MAX - TICK_Y_MIN) * f32(i) / f32(OCTAVE_SLOTS - 1u);
}

// Coverage of the tick pip for slot `i`.
fn tick_pip(i: u32, uv: vec2<f32>) -> f32 {
    let dx = max(abs(uv.x - TICK_X) - TICK_HALF_W, 0.0);
    let dy = max(abs(uv.y - tick_slot_y(i)) - TICK_HALF_H, 0.0);
    return 1.0 - smoothstep(0.0, 0.035, dx + dy);
}

// Coverage (0..1) of the octave glyph for a SOUNDING slot `i`:
//   1 = dots:     satellites orbiting the disc, clock position = octave
//   2 = rings:    concentric rings, inner ring = lowest octave
//   3..6 = ticks: column right of the disc, bottom = lowest octave
//                 (variants differ only in reference furniture, below)
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
        return tick_pip(i, uv);
    }
}

// Reference furniture for the tick variants: static geometry showing where
// the octave range begins and ends, so a lit tick has an absolute position.
// Returns coverage ALREADY scaled to its intended dimness relative to a lit
// tick (1.0).
fn tick_reference(mode: u32, uv: vec2<f32>) -> f32 {
    var ref_cov = 0.0;

    // Rail: a faint spine the full height of the column (modes 3 and 5).
    if mode == 3u || mode == 5u {
        let dx = max(abs(uv.x - TICK_X) - 0.016, 0.0);
        let dy = max(abs(uv.y) - (TICK_Y_MAX + 0.02), 0.0);
        ref_cov = max(ref_cov, 0.22 * (1.0 - smoothstep(0.0, 0.03, dx + dy)));
    }

    // Ladder: every slot as a dim pip (modes 4 and 6).
    if mode == 4u || mode == 6u {
        for (var i = 0u; i < OCTAVE_SLOTS; i = i + 1u) {
            ref_cov = max(ref_cov, 0.20 * tick_pip(i, uv));
        }
    }

    // End caps: emphasized bars just past the bottom and top slots (mode 5).
    if mode == 5u {
        let dx = max(abs(uv.x - TICK_X) - 0.13, 0.0);
        let dy_bot = max(abs(uv.y - (TICK_Y_MIN - 0.075)) - 0.014, 0.0);
        let dy_top = max(abs(uv.y - (TICK_Y_MAX + 0.075)) - 0.014, 0.0);
        let dy = min(dy_bot, dy_top);
        ref_cov = max(ref_cov, 0.45 * (1.0 - smoothstep(0.0, 0.03, dx + dy)));
    }

    // Middle-C octave marker: a brighter line through slot 4 (mode 6).
    if mode == 6u {
        let dx = max(abs(uv.x - TICK_X) - 0.14, 0.0);
        let dy = max(abs(uv.y - tick_slot_y(4u)) - 0.010, 0.0);
        ref_cov = max(ref_cov, 0.45 * (1.0 - smoothstep(0.0, 0.025, dx + dy)));
    }

    return ref_cov;
}

// ---- Style helpers ---------------------------------------------------------

fn hash21(p: vec2<f32>) -> f32 {
    return fract(sin(dot(p, vec2<f32>(127.1, 311.7))) * 43758.5453);
}

// Smooth value noise; cheap and good enough for flame flicker.
fn vnoise(p: vec2<f32>) -> f32 {
    let i = floor(p);
    let f = fract(p);
    let w = f * f * (3.0 - 2.0 * f);
    return mix(
        mix(hash21(i), hash21(i + vec2<f32>(1.0, 0.0)), w.x),
        mix(hash21(i + vec2<f32>(0.0, 1.0)), hash21(i + vec2<f32>(1.0, 1.0)), w.x),
        w.y,
    );
}

fn seg_dist(p: vec2<f32>, a: vec2<f32>, b: vec2<f32>) -> f32 {
    let pa = p - a;
    let ba = b - a;
    let h = clamp(dot(pa, ba) / dot(ba, ba), 0.0, 1.0);
    return length(pa - ba * h);
}

// Wire style: distance-field wireframe of a tumbling octahedron, projected
// orthographically into the billboard plane. Returns line coverage.
fn wire_octahedron(uv: vec2<f32>, t: f32, phase: f32) -> f32 {
    let yaw = (t - phase) * 0.6 + phase * 2.3;
    let pitch = (t - phase) * 0.37 + phase * 1.1;
    let cy = cos(yaw);
    let sy = sin(yaw);
    let cp = cos(pitch);
    let sp = sin(pitch);

    // Octahedron vertices, rotated (yaw about Y then pitch about X) and
    // projected by dropping z.
    var v3 = array<vec3<f32>, 6>(
        vec3<f32>(0.46, 0.0, 0.0),
        vec3<f32>(-0.46, 0.0, 0.0),
        vec3<f32>(0.0, 0.46, 0.0),
        vec3<f32>(0.0, -0.46, 0.0),
        vec3<f32>(0.0, 0.0, 0.46),
        vec3<f32>(0.0, 0.0, -0.46),
    );
    var v: array<vec2<f32>, 6>;
    for (var i = 0u; i < 6u; i = i + 1u) {
        let p0 = v3[i];
        let p1 = vec3<f32>(cy * p0.x + sy * p0.z, p0.y, -sy * p0.x + cy * p0.z);
        let p2 = vec3<f32>(p1.x, cp * p1.y - sp * p1.z, sp * p1.y + cp * p1.z);
        v[i] = p2.xy;
    }

    // 12 edges: each of +-x,+-y connects to +-z and to each other's axis
    // neighbors (every vertex pair except the three opposite pairs).
    var ea = array<u32, 12>(0u, 0u, 0u, 0u, 1u, 1u, 1u, 1u, 2u, 2u, 3u, 3u);
    var eb = array<u32, 12>(2u, 3u, 4u, 5u, 2u, 3u, 4u, 5u, 4u, 5u, 4u, 5u);
    var wire = 0.0;
    for (var e = 0u; e < 12u; e = e + 1u) {
        let dist = seg_dist(uv, v[ea[e]], v[eb[e]]);
        wire = max(wire, 1.0 - smoothstep(0.015, 0.05, dist));
    }
    return wire;
}

@fragment
fn fs_main(in: VsOut) -> @location(0) vec4<f32> {
    let d = length(in.uv); // 0 at center, 1 at quad edge (2x disc radius)
    let activation = in.params.x;
    let hovered = in.params.y;

    let style = u32(u.misc.w + 0.5);
    let t = u.misc.x;
    let phase = in.params.z;

    // Solid disc occupies the inner half of the quad; channel-14 voices
    // render as an outline ring instead (v1 semantics).
    let outlined = in.params.w;
    let filled = 1.0 - smoothstep(0.42, 0.5, d);
    let ring = (1.0 - smoothstep(0.42, 0.5, d)) * smoothstep(0.30, 0.38, d);
    var disc = mix(filled, ring, outlined);

    // Wire style: active nodes morph from disc into a tumbling wireframe
    // octahedron (idle nodes stay discs in every style).
    if style == 4u && activation > 0.0 {
        disc = mix(disc, wire_octahedron(in.uv, t, phase), activation);
    }

    // Soft additive-looking glow for active nodes. The exponential alone
    // never reaches zero, so the quad boundary showed as a boxy halo;
    // window it so it fades to exactly zero (with zero slope) inside the
    // quad edge.
    let window = 1.0 - smoothstep(0.5, 0.95, d);
    var glow = (0.6 * activation + 0.25 * hovered) * exp(-3.0 * d) * window;

    // Breathe: glow strength pulses in sync with the vertex-side size pulse.
    if style == 1u {
        glow = glow * (1.0 + 0.35 * activation * sin((t - phase) * TAU * 0.8));
    }
    // Corona: replace the smooth glow with a noise-flickered flame edge
    // whose reach flutters over time, seeded per note.
    if style == 2u && activation > 0.0 {
        let flame = pow(vnoise(in.uv * 4.0 + vec2<f32>(t * 1.4 + phase * 9.0, t * 1.1)), 2.0);
        glow = activation * exp(-4.5 * max(d - 0.44, 0.0) * (2.4 - 1.8 * flame)) * window
            * (0.55 + 0.45 * flame)
            + 0.25 * hovered * exp(-3.0 * d) * window;
    }

    let brightness = 0.35 + 0.65 * activation + 0.2 * hovered;
    let rgb = in.color.rgb * brightness;
    let base_alpha = clamp(disc + glow, 0.0, 1.0);

    // Octave indicators, composited over the disc/glow. Each slot fades on
    // its own octave's envelope; the reference furniture follows the
    // brightest slot so it disappears with the last sounding octave.
    let mode = u32(u.misc.z + 0.5);
    var glyph = 0.0;
    var max_level = 0.0;

    // Sparks: bright particles orbiting held nodes, drawn in the same
    // whitened layer as the octave glyphs.
    if style == 3u && activation > 0.0 {
        for (var k = 0u; k < 3u; k = k + 1u) {
            let fk = f32(k);
            let dir = select(1.0, -1.0, k == 1u);
            let ang = dir * (t - phase) * (1.6 + 0.5 * fk) + fk * 2.094 + phase * 5.0;
            let orbit = 0.60 + 0.10 * sin((t - phase) * (0.7 + 0.3 * fk) + fk * 1.7);
            let pos = vec2<f32>(cos(ang), sin(ang)) * orbit;
            let spark = 1.0 - smoothstep(0.02, 0.075, distance(in.uv, pos));
            glyph = max(glyph, spark * activation);
        }
    }

    if mode != 0u {
        for (var i = 0u; i < OCTAVE_SLOTS; i = i + 1u) {
            let level = octave_level(in.octaves, i);
            if level > 0.0 {
                max_level = max(max_level, level);
                glyph = max(
                    glyph,
                    octave_glyph(mode, i, in.uv, d) * (0.35 + 0.65 * level),
                );
            }
        }
        if mode >= 3u && max_level > 0.0 {
            glyph = max(
                glyph,
                tick_reference(mode, in.uv) * (0.35 + 0.65 * max_level),
            );
        }
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
