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
    // x: global time (s, wraps hourly; unused by the built-in styles —
    //    per-note animation uses the instance age/seed, which stay small
    //    and precise however long the session runs),
    // y: base node radius (world units),
    // z: octave display mode (0 off, 1 dots),
    // w: node style (0 steady, 1 breathe, 2 corona, 3 sparks, 4 wire)
    misc: vec4<f32>,
    // x: darkest_pitch, y: brightest_pitch (MIDI notes); z, w unused. The
    // dots style maps a dot's pitch through these to index dot_ramp.
    misc2: vec4<f32>,
    // Pitch->color lookup for the dots octave style, matching the node disc
    // gradient (length mirrors lattice_scene::DOT_RAMP_N).
    dot_ramp: array<vec4<f32>, 16>,
};

const TAU: f32 = 6.2831853;

@group(0) @binding(0) var<uniform> u: Uniforms;

struct Instance {
    @location(0) world_pos: vec3<f32>,
    @location(1) color: vec4<f32>,
    // x: activation 0..1, y: hovered 0/1, z: age (s since note-on),
    // w: outlined 0/1 (channel-14 voices render as a ring, not a disc)
    @location(2) params: vec4<f32>,
    // Per-octave activation, 8 bits per slot, little-endian packed.
    @location(3) octaves: vec3<u32>,
    // Per-note animation seed: a small constant, NOT a timestamp.
    @location(4) seed: f32,
    // The node's pitch class in cents (0..1200). Dots mode places each
    // octave dot at the note's absolute-pitch angle, which needs the pitch
    // class within the octave; unused by the other octave styles.
    @location(5) cents: f32,
};

struct VsOut {
    @builtin(position) clip_pos: vec4<f32>,
    @location(0) uv: vec2<f32>, // -1..1 across the quad
    @location(1) color: vec4<f32>,
    @location(2) params: vec4<f32>,
    @location(3) @interpolate(flat) octaves: vec3<u32>,
    @location(4) seed: f32,
    @location(5) @interpolate(flat) cents: f32,
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

    // Node size is constant: idle nodes are the same size as active ones, so a
    // note never grows or shrinks — it changes only in brightness and glow.
    // (The quad is twice the disc radius to leave room for the glow; hover
    // still nudges the size up a touch.)
    var radius = u.misc.y * (0.90 + 0.15 * hovered) * 2.0;

    // Breathe style: held nodes pulse in size on their own age
    // (oscillation starts neutral at note-on).
    if u32(u.misc.w + 0.5) == 1u {
        let beat = inst.params.z * TAU * 0.8;
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
    out.seed = inst.seed;
    out.cents = inst.cents;
    return out;
}

// Number of octave slots the indicators display (MIDI octaves 0..9).
const OCTAVE_SLOTS: u32 = 10u;

// Length of the dots-mode pitch->color LUT (mirrors lattice_scene::DOT_RAMP_N
// and the `dot_ramp` array in Uniforms).
const DOT_RAMP_N: u32 = 16u;

// Dimmest-visible convention, shared with the UI panes (visibility_floor
// in lattice-ui): quiet elements sit at 35% and scale up to full.
fn level_floor(level: f32) -> f32 {
    return 0.35 + 0.65 * level;
}

// Activation level (0..1) of octave slot `i`, unpacked from 8-bit fields.
// Each octave carries its OWN envelope so indicators fade independently
// (a released C5 decays even while C4 holds the node fully lit).
fn octave_level(octaves: vec3<u32>, i: u32) -> f32 {
    let word = octaves[i / 4u];
    return f32((word >> ((i % 4u) * 8u)) & 0xFFu) / 255.0;
}

// Octave slot of middle C (C4): MIDI 60 -> octave 4. The dots anchor this
// slot straight up and measure every other dot's angle relative to it.
const MIDDLE_C_SLOT: f32 = 4.0;
// 45deg (pi/4) of rotation per octave, so two octaves reach the horizontal.
// Clockwise as pitch rises.
const DOTS_RAD_PER_OCTAVE: f32 = 0.7853982;
// Dots geometry, in quad UV units: the ring the dots orbit (kept close to the
// disc edge ~0.5), and the dot's solid-core / fade-edge radii.
const DOT_ORBIT: f32 = 0.67;
const DOT_CORE: f32 = 0.10;
const DOT_EDGE: f32 = 0.15;

// Coverage (0..1) of the dot for a SOUNDING octave slot `i` on a node whose
// pitch class is `cents`: a satellite around the disc whose angle is the
// note's absolute pitch (middle C straight up, 45deg clockwise per octave,
// pitch class within the octave included).
fn octave_dot(i: u32, cents: f32, uv: vec2<f32>) -> f32 {
    // (uv.y is up, so clockwise = subtracting from the angle.)
    let octaves_from_mid_c = (f32(i) - MIDDLE_C_SLOT) + cents / 1200.0;
    let ang = 1.5707963 - DOTS_RAD_PER_OCTAVE * octaves_from_mid_c;
    let center = vec2<f32>(cos(ang), sin(ang)) * DOT_ORBIT;
    return 1.0 - smoothstep(DOT_CORE, DOT_EDGE, distance(uv, center));
}

// Color of a dots-mode dot at absolute MIDI `pitch`, read from the pitch
// gradient LUT so a dot is the same hue as the disc that pitch would light.
fn dot_pitch_color(pitch: f32) -> vec3<f32> {
    let t = clamp((pitch - u.misc2.x) / max(u.misc2.y - u.misc2.x, 0.01), 0.0, 1.0);
    let f = t * f32(DOT_RAMP_N - 1u);
    let i0 = u32(floor(f));
    let i1 = min(i0 + 1u, DOT_RAMP_N - 1u);
    return mix(u.dot_ramp[i0].rgb, u.dot_ramp[i1].rgb, f - floor(f));
}

// Faint C-octave reference frame for the dots style: short radial ticks
// hugging the disc rim (just INSIDE the dot orbit) at the four cardinals,
// pointing at where a C would land — up = middle C (C3 in the C3=middle-C
// naming), left/right = -/+2 octaves (C1 / C5), down = +/-4 octaves. They sit
// in the gap between the disc edge (~0.5) and the dots' inner reach
// (~DOT_ORBIT - DOT_EDGE), so a lit dot never overlaps a tick; the caller
// tints them with the node color so they read as part of the disc.
const DOTS_REF_R: f32 = 0.51;         // tick mid-radius (disc edge ~0.5, dots at DOT_ORBIT)
const DOTS_REF_HALF_LEN: f32 = 0.035; // radial half-length
const DOTS_REF_HALF_W: f32 = 0.018;   // tangential half-width
fn dots_reference(uv: vec2<f32>) -> f32 {
    var dirs = array<vec2<f32>, 4>(
        vec2<f32>(0.0, 1.0),  // up:    middle C
        vec2<f32>(1.0, 0.0),  // right: +2 octaves
        vec2<f32>(0.0, -1.0), // down:  +/-4 octaves
        vec2<f32>(-1.0, 0.0), // left:  -2 octaves
    );
    var cov = 0.0;
    for (var k = 0u; k < 4u; k = k + 1u) {
        let n = dirs[k];
        let radial = dot(uv, n);                        // outward along the cardinal
        let tang = abs(dot(uv, vec2<f32>(-n.y, n.x)));  // sideways offset
        let dr = max(abs(radial - DOTS_REF_R) - DOTS_REF_HALF_LEN, 0.0);
        let ds = max(tang - DOTS_REF_HALF_W, 0.0);
        cov = max(cov, 1.0 - smoothstep(0.0, 0.02, dr + ds));
    }
    // Dimmed relative to a lit dot; the caller further scales by the note's
    // fade so the frame appears only while the node sounds.
    return cov * 0.4;
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
fn wire_octahedron(uv: vec2<f32>, age: f32, seed: f32) -> f32 {
    let yaw = age * 0.6 + seed * 2.3;
    let pitch = age * 0.37 + seed * 1.1;
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
    let age = in.params.z;
    let seed = in.seed;

    // Solid disc occupies the inner half of the quad; channel-14 voices
    // render as an outline ring instead (v1 semantics).
    let outlined = in.params.w;
    let filled = 1.0 - smoothstep(0.42, 0.5, d);
    let ring = (1.0 - smoothstep(0.42, 0.5, d)) * smoothstep(0.30, 0.38, d);
    var disc = mix(filled, ring, outlined);

    // Wire style: active nodes morph from disc into a tumbling wireframe
    // octahedron (idle nodes stay discs in every style).
    if style == 4u && activation > 0.0 {
        disc = mix(disc, wire_octahedron(in.uv, age, seed), activation);
    }

    // Soft additive-looking glow for active nodes. The exponential alone
    // never reaches zero, so the quad boundary showed as a boxy halo;
    // window it so it fades to exactly zero (with zero slope) inside the
    // quad edge.
    let window = 1.0 - smoothstep(0.5, 0.95, d);
    var glow = (0.6 * activation + 0.25 * hovered) * exp(-3.0 * d) * window;

    // Breathe: glow strength pulses in sync with the vertex-side size pulse.
    if style == 1u {
        glow = glow * (1.0 + 0.35 * activation * sin(age * TAU * 0.8));
    }
    // Corona: replace the smooth glow with a noise-flickered flame edge
    // whose reach flutters over time, seeded per note.
    if style == 2u && activation > 0.0 {
        let flame = pow(vnoise(in.uv * 4.0 + vec2<f32>(age * 1.4 + seed * 9.0, age * 1.1)), 2.0);
        glow = activation * exp(-4.5 * max(d - 0.44, 0.0) * (2.4 - 1.8 * flame)) * window
            * (0.55 + 0.45 * flame)
            + 0.25 * hovered * exp(-3.0 * d) * window;
    }

    let brightness = level_floor(activation) + 0.2 * hovered;
    let rgb = in.color.rgb * brightness;
    let base_alpha = clamp(disc + glow, 0.0, 1.0);

    // Octave indicators, composited over the disc/glow. Each slot fades on
    // its own envelope; the reference frame follows the brightest slot so it
    // disappears with the last sounding octave. Whichever element covers a
    // pixel most strongly owns its color there: dots are tinted by their own
    // pitch, everything else uses the whitened node color.
    let mode = u32(u.misc.z + 0.5);
    let node_glyph_rgb = mix(in.color.rgb, vec3<f32>(1.0, 1.0, 1.0), 0.55);
    var glyph = 0.0;
    var glyph_rgb = node_glyph_rgb;
    var max_level = 0.0;

    // Sparks: bright particles orbiting held nodes, drawn in the same
    // whitened layer as the octave glyphs.
    if style == 3u && activation > 0.0 {
        for (var k = 0u; k < 3u; k = k + 1u) {
            let fk = f32(k);
            let dir = select(1.0, -1.0, k == 1u);
            let ang = dir * age * (1.6 + 0.5 * fk) + fk * 2.094 + seed * 5.0;
            let orbit = 0.60 + 0.10 * sin(age * (0.7 + 0.3 * fk) + fk * 1.7);
            let pos = vec2<f32>(cos(ang), sin(ang)) * orbit;
            let spark = (1.0 - smoothstep(0.02, 0.075, distance(in.uv, pos))) * activation;
            if spark > glyph {
                glyph = spark;
                glyph_rgb = node_glyph_rgb;
            }
        }
    }

    // Dots octave indicator (mode 1; 0 is off). Each slot fades on its own
    // envelope. Whichever element covers a pixel most strongly owns its color
    // there: dots are tinted by their own pitch, the reference by node color.
    if mode != 0u {
        for (var i = 0u; i < OCTAVE_SLOTS; i = i + 1u) {
            let level = octave_level(in.octaves, i);
            if level > 0.0 {
                max_level = max(max_level, level);
                let cov = octave_dot(i, in.cents, in.uv) * level_floor(level);
                if cov > glyph {
                    glyph = cov;
                    // Slot i is MIDI octave i, whose C is MIDI (i+1)*12; add
                    // this node's pitch class for the dot's true pitch.
                    let pitch = (f32(i) + 1.0) * 12.0 + in.cents / 100.0;
                    glyph_rgb = mix(dot_pitch_color(pitch), vec3<f32>(1.0, 1.0, 1.0), 0.30);
                }
            }
        }
        // Faint cardinal C-octave tick frame, faded with the brightest slot so
        // it appears only while the node sounds.
        if max_level > 0.0 {
            let rc = dots_reference(in.uv) * level_floor(max_level);
            if rc > glyph {
                glyph = rc;
                // Node-colored, only lightly lifted, so the ticks blend into
                // the disc rather than reading as separate marks.
                glyph_rgb = mix(in.color.rgb, vec3<f32>(1.0, 1.0, 1.0), 0.4);
            }
        }
    }

    // "Over" composite: glyph over (disc + glow), premultiplied.
    let alpha = glyph + base_alpha * (1.0 - glyph);
    if alpha < 0.01 {
        discard;
    }
    let out_rgb = glyph_rgb * glyph + rgb * base_alpha * (1.0 - glyph);
    return vec4<f32>(out_rgb, alpha);
}

// ---- Chord edges -----------------------------------------------------------
// Beams between simultaneously sounding, lattice-adjacent nodes: a held
// chord's interval structure rendered as geometry. Drawn under the nodes.

struct EdgeInstance {
    // xyz: endpoint A, w: strength (min of the two node activations)
    @location(0) a_strength: vec4<f32>,
    // xyz: endpoint B, w: unused
    @location(1) b_pad: vec4<f32>,
    @location(2) color: vec4<f32>,
};

struct EdgeVsOut {
    @builtin(position) clip_pos: vec4<f32>,
    // x: 0..1 along the edge, y: -1..1 across it
    @location(0) uv: vec2<f32>,
    @location(1) color: vec4<f32>,
    @location(2) strength: f32,
};

@vertex
fn vs_edge(@builtin(vertex_index) vertex_index: u32, inst: EdgeInstance) -> EdgeVsOut {
    var corners = array<vec2<f32>, 4>(
        vec2<f32>(0.0, -1.0),
        vec2<f32>(1.0, -1.0),
        vec2<f32>(0.0, 1.0),
        vec2<f32>(1.0, 1.0),
    );
    let corner = corners[vertex_index];

    let a = inst.a_strength.xyz;
    let b = inst.b_pad.xyz;
    let axis = b - a;
    // Billboard the beam's width: perpendicular to both the edge and the
    // view direction, falling back to camera-up for edge-on views.
    let view_dir = cross(u.cam_right.xyz, u.cam_up.xyz);
    var perp = cross(normalize(axis), view_dir);
    let plen = length(perp);
    if plen < 1e-4 {
        perp = u.cam_up.xyz;
    } else {
        perp = perp / plen;
    }
    let half_width = u.misc.y * 0.35;
    let world = a + axis * corner.x + perp * corner.y * half_width;

    var out: EdgeVsOut;
    out.clip_pos = u.view_proj * vec4<f32>(world, 1.0);
    out.uv = corner;
    out.color = inst.color;
    out.strength = inst.a_strength.w;
    return out;
}

@fragment
fn fs_edge(in: EdgeVsOut) -> @location(0) vec4<f32> {
    // Soft-edged beam with a brighter core; the ends taper so the node
    // discs own the joints.
    let across = 1.0 - smoothstep(0.15, 1.0, abs(in.uv.y));
    let along = smoothstep(0.0, 0.10, in.uv.x) * (1.0 - smoothstep(0.90, 1.0, in.uv.x));
    let alpha = in.strength * across * along * 0.85;
    if alpha < 0.01 {
        discard;
    }
    let rgb = in.color.rgb * (0.55 + 0.45 * across);
    return vec4<f32>(rgb * alpha, alpha);
}
