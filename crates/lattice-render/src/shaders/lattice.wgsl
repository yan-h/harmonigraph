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
    // x: global time (s, wraps hourly). The gas styles clock on this so
    //    their fields keep flowing across note events (at worst the pattern
    //    jumps once an hour at the wrap); the other styles animate on the
    //    instance age/seed, which stay small and precise however long the
    //    session runs.
    // y: base node radius (world units),
    // z: outer octave layer (0 off, 5 on — one glyph shape is left, and
    //    its original index is kept; see OuterStyle::shader_index),
    // w: node style — the core orb's paint (0 steady, 3 vortex,
    //    11 pinwheel, 12 spiral, 13 checker). Indices are sparse: they
    //    are preserved from the original 15-style set so the kept styles'
    //    branches below stay unchanged (see NodeStyle::shader_index in
    //    lattice-scene).
    misc: vec4<f32>,
    // x: darkest_pitch, y: brightest_pitch (MIDI notes); z: render scale
    // (offscreen pixels per screen pixel — converts the screen-pixel
    // softness knob to render pixels); w unused. An octave glyph maps its
    // pitch through x/y to index pitch_lut.
    misc2: vec4<f32>,
    // x: core radius in quad UV units (0 turns the core off). y/z: the
    // outer layer's inner/outer band radii (same units; the scene
    // guarantees z > y). w: outer backdrop opacity 0..1 (ghost the silent
    // octaves to complete the ring; independent of the core). 0 draws no
    // backdrop; 1 is the full built-in ghost strength.
    misc3: vec4<f32>,
    // Pitch->color lookup for the octave glyphs, matching the node disc
    // gradient (length mirrors lattice_scene::PITCH_LUT_N).
    pitch_lut: array<vec4<f32>, 16>,
    // Idle node color (the view's grid color): the home-sheet placeholder ring is
    // drawn in this constant grey, so a releasing note's ring stays grey
    // (not the note hue) and never snaps color when the voice is pruned.
    node_idle: vec4<f32>,
    // x: core solidity (0 = soft glow, 1 = solid orb) — the single axis the
    // core layer runs on. y: outer solidity (0 = soft glowy glyphs, 1 =
    // crisp octave shapes). z: idle marker radius. w: idle marker style
    // (0 none, 1 dot, 2 circle).
    misc4: vec4<f32>,
    // x: grid line thickness, a multiple of the built-in grid width.
    // y: opacity of the part of a melody/bass ring cut off from the octave
    // that owns it (see mark_ring_alpha).
    // z: padding inside the octave layer, in quad UV units — the gap
    // between neighbouring sectors AND between the band and the mark
    // rings. w: melody/bass ring thickness, same units; 0 = no rings.
    misc5: vec4<f32>,
    // x: trail mark style — how a node the music has already visited is
    //    marked (0 off, 1 lift, 2 ring, 3 tint; see TrailMark). y: trail
    //    strength 0..1. z/w unused.
    // Read ONLY by idle_marker: a memory must never be mistakable for a
    // sounding note, and confining it to the idle layer is what guarantees
    // that rather than merely intending it.
    misc6: vec4<f32>,
    // The ground the lattice is painted onto (the pane fill this pass is
    // composited over). Only the sevens knockout reads it: without it the
    // gutter can knock out only to black, which is darker than the pane and
    // reads as a plate sitting ON the picture rather than a hole THROUGH it.
    background: vec4<f32>,
};

const TAU: f32 = 6.2831853;

// Billboard headroom past the octave band's outer edge (uv 1.0): the quad
// and its uv are both scaled by this, so the uv->world mapping is
// unchanged (disc, band, glyphs, glow all render identically) but there is
// margin out to this radius for things that live OUTSIDE the band -- a
// low-solidity glyph's soft edge, and the melody ring, which at the default
// band (outer 1.0) sits entirely out here. Costs a bit of fill (bigger
// quads, which alpha-blend and discard where empty).
const QUAD_MARGIN: f32 = 1.6;
// Where a soft glyph's overflow finishes easing off. Pinned to what
// QUAD_MARGIN was when this fade was tuned, so widening the billboard for
// the mark rings doesn't quietly restyle every low-solidity glyph.
const GLYPH_FADE_LIMIT: f32 = 1.3;

@group(0) @binding(0) var<uniform> u: Uniforms;

struct Instance {
    @location(0) world_pos: vec3<f32>,
    @location(1) color: vec4<f32>,
    // x: activation 0..1, w: outlined 0/1 (channel-14 voices render as a
    // ring, not a disc). y/z: the melody and bass marks' own fade levels,
    // which follow the marked voice rather than this node's activation.
    @location(2) params: vec4<f32>,
    // Per-octave activation, 8 bits per slot, little-endian packed.
    @location(3) octaves: vec3<u32>,
    // Animation seed: a small constant, NOT a timestamp. A stable per-NODE
    // hash for the field styles (the scene picks — see node_seed in
    // lattice-scene); Steady ignores it.
    @location(4) seed: f32,
    // The node's pitch class in cents (0..1200). The octave indicator
    // The octave glyphs sit at the note's absolute-pitch angle, which needs
    // the pitch class within the octave.
    @location(5) cents: f32,
    // 1 on the home (center sevens) sheet: idle home nodes draw a blank
    // placeholder ring where their disc would be.
    @location(6) home: f32,
    // Melody/bass marks: x = melody slots, y = bass slots, one bit per
    // octave slot. Which SECTOR each mark's ring links back to (see
    // mark_ring); the ring itself is per node, and its fade level rides
    // params.y/params.z.
    @location(7) marks: vec2<u32>,
    // Each mark's own color: the marked note's, lightened a little (see
    // NodeInstance::melody_color), so a ring reads as belonging to the note
    // it marks rather than as a fixed livery.
    @location(8) melody_color: vec4<f32>,
    @location(9) bass_color: vec4<f32>,
    // How strongly the music is remembered at this node, 0..1 (see
    // NodeInstance::trail). Feeds the idle marker and nothing else.
    @location(10) visited: f32,
    // The sevens layer: x = billboard size factor (1 on the home sheet,
    // smaller with every step off it), y = knockout gutter width in uv
    // units (0 on the home sheet). See ViewConfig::sevens_size / _gutter.
    @location(11) sevens: vec2<f32>,
};

struct VsOut {
    @builtin(position) clip_pos: vec4<f32>,
    @location(0) uv: vec2<f32>, // -1..1 across the quad
    @location(1) color: vec4<f32>,
    @location(2) params: vec4<f32>,
    @location(3) @interpolate(flat) octaves: vec3<u32>,
    @location(4) seed: f32,
    @location(5) @interpolate(flat) cents: f32,
    @location(6) @interpolate(flat) home: f32,
    @location(7) @interpolate(flat) marks: vec2<u32>,
    @location(8) @interpolate(flat) melody_color: vec4<f32>,
    @location(9) @interpolate(flat) bass_color: vec4<f32>,
    @location(10) @interpolate(flat) visited: f32,
    @location(11) @interpolate(flat) gutter: f32,
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

    // Notes, hover, and distance from the camera all leave a node's size
    // alone, so a note changes only brightness and glow. The ONE thing that
    // sizes it is which sevens sheet it sits on (inst.sevens.x, 1 on the
    // home sheet): the home sheet is the ground the music is read against,
    // so sheets off it draw smaller — in both directions, since that is
    // distance from the ground and not depth toward the eye. The uv is
    // deliberately NOT scaled with it, so every layer inside the node keeps
    // its proportions and only the node's size on screen changes. (The quad
    // is twice the disc radius to leave room for the glow, plus QUAD_MARGIN
    // for the outer glyphs' soft edge — see QUAD_MARGIN.)
    let radius = u.misc.y * 0.90 * 2.0 * QUAD_MARGIN * inst.sevens.x;

    let world = inst.world_pos
        + (u.cam_right.xyz * corner.x + u.cam_up.xyz * corner.y) * radius;

    var out: VsOut;
    out.clip_pos = u.view_proj * vec4<f32>(world, 1.0);
    out.uv = corner * QUAD_MARGIN;
    out.color = inst.color;
    out.params = inst.params;
    out.octaves = inst.octaves;
    out.seed = inst.seed;
    out.cents = inst.cents;
    out.home = inst.home;
    out.marks = inst.marks;
    out.melody_color = inst.melody_color;
    out.bass_color = inst.bass_color;
    out.visited = inst.visited;
    out.gutter = inst.sevens.y;
    return out;
}

// Number of octave slots the indicators display (MIDI octaves 0..9).
const OCTAVE_SLOTS: u32 = 10u;

// Length of the pitch->color LUT (mirrors lattice_scene::PITCH_LUT_N
// and the `pitch_lut` array in Uniforms).
const PITCH_LUT_N: u32 = 16u;

// Dimmest-visible convention, shared with the UI panes (visibility_floor
// in lattice-ui): quiet elements sit at 35% and scale up to full.
fn level_floor(level: f32) -> f32 {
    return 0.35 + 0.65 * level;
}

// Coverage of `x` inside the threshold `edge`, with a screen-constant
// soft band: `w` is ~a pixel expressed in `x`'s units (from fwidth at
// the call site — taken at the top of the fragment fn, outside any
// non-uniform control flow). Fixed-width smoothstep edges blur as a
// quad grows on screen and alias as it shrinks; this keeps every shape
// edge equally soft at all zooms. Glows and gas interiors deliberately
// keep their proportional falloffs.
fn aa_inside(edge: f32, x: f32, w: f32) -> f32 {
    return 1.0 - smoothstep(edge - w, edge + w, x);
}

// Half-width of the soft band, in SCREEN pixels on each side of an edge
// (the one knob for how soft shape edges feel; softness stays
// screen-constant at every zoom). ~4px total on a Retina surface.
// aa_width() converts to render pixels via the render scale, so the
// super/sub-sampling view setting changes resolved detail without
// changing how soft edges look.
const AA_SOFTNESS_PX: f32 = 2.0;

// Soft-band width in the units of a coordinate whose per-render-pixel
// derivative is `coord_fwidth`: the softness knob, converted from screen
// pixels to render pixels.
fn aa_width(coord_fwidth: f32) -> f32 {
    return max(coord_fwidth, 1e-4) * AA_SOFTNESS_PX * max(u.misc2.z, 0.01);
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
const RAD_PER_OCTAVE: f32 = 0.7853982;
// ---- Outer octave layer ----------------------------------------------------
// Every outer style draws its glyphs inside the radial band
// [u.misc3.y, u.misc3.z] (quad UV units): the band IS the glyph set's
// radial footprint, so switching styles keeps the octave display the same
// size. The glyphs are drawn identically whatever the core does — the
// layers are independent.
//
// The backdrop flag (u.misc3.w, its own outer-layer setting) adds a
// cohesion device so a note reads as ONE whole shape even when a single
// octave sounds: the SILENT slots draw as faint ghosts in the note's own
// color, completing the circle silhouette around the bright sector.

// Neighboring sectors (slots are 0.785 rad apart) are separated by a
// CONSTANT-thickness gap: the slice edges are radial lines offset half the
// gap from the sector bisectors, not constant-angle edges (those read as a V
// that widens outward). At band inner 0 the sectors become full pie wedges;
// near the center every wedge falls inside the gap band, leaving a small
// clear hub instead of a ten-way mush point.
//
// The gap's full width is u.misc5.z (the view's Gap bar), in quad UV units.
// The SAME value separates the mark rings from the band, so one number is
// the padding everywhere in the octave layer.
fn slice_gap_half() -> f32 {
    return max(u.misc5.z, 0.0) * 0.5;
}
// Ghost coverage of a silent slot when the backdrop is on, scaled
// by the note's activation so ghosts fade out with the pitch class.
const GHOST_LEVEL: f32 = 0.16;
// How far the outer glyphs' soft edge spreads at outer solidity 0, as a
// fraction of the band width (added to the screen-constant aa). ~1 band
// width makes the sectors melt into diffuse glows; smaller keeps them
// tighter. Tunes the soft end of the octave solidity slider.
const OUTER_GLOW_SOFT: f32 = 1.0;
// The classic disc-edge radius: normalizes the field paint to the sized
// orb, and stands in for the core radius where a coreless node still needs
// one (channel 14's outline ring).
const CORE_R_CLASSIC: f32 = 0.46;
// Extra half-width added to the core disc's edge as solidity drops from 1
// to 0: at solidity 1 the edge is a crisp screen-constant band (the
// classic orb), at 0 it has spread this far and the disc has faded out
// into the glow skirt. Tunes how "soft" a mid-solidity core reads.
const CORE_EDGE_SOFT: f32 = 0.30;
// Radius below which the whole core fades to nothing, so a radius of 0 is
// the off state and the core grows in smoothly (no pop) as the bar leaves
// the bottom.
const CORE_FADE_IN: f32 = 0.06;
// Thickness of the idle position ring (see idle_marker), matched to the old
// home placeholder ring (which spanned 0.37..0.46 at the classic radius).
const IDLE_RING_THICK: f32 = 0.09;

// Coverage (0..1) of the outer glyph for octave slot `i` on a node whose
// pitch class is `cents`, drawn in the uniform band. Reads nothing from the
// core layer — the outer glyphs are independent of it. The glyph's angle is
// its absolute pitch: middle C straight up, 45deg clockwise per octave,
// pitch class within the octave included. `aa` is the caller's per-pixel
// soft-band width, giving the shape screen-constant edges.
fn outer_glyph(i: u32, cents: f32, uv: vec2<f32>, inner: f32, outer: f32, aa: f32) -> f32 {
    // (uv.y is up, so clockwise = subtracting from the angle.)
    let octaves_from_mid_c = (f32(i) - MIDDLE_C_SLOT) + cents / 1200.0;
    let ang = 1.5707963 - RAD_PER_OCTAVE * octaves_from_mid_c;
    let d = length(uv);

    // Annular sectors, screen-constant edges. The sector bisector
    // directions b1/b2 bound this slot's wedge; the cross products against
    // them give BOTH the side-of-line tests (which wedge owns the pixel)
    // and the Euclidean distance to each edge line, thresholded at
    // half the gap width, for a gap of constant thickness at every radius.
    let band = aa_inside(outer, d, aa) * (1.0 - aa_inside(inner, d, aa));
    let hb = RAD_PER_OCTAVE * 0.5;
    let b1 = vec2<f32>(cos(ang + hb), sin(ang + hb));
    let b2 = vec2<f32>(cos(ang - hb), sin(ang - hb));
    let c1 = uv.x * b1.y - uv.y * b1.x;
    let c2 = uv.x * b2.y - uv.y * b2.x;
    // Ownership softened over `aa`: at crisp aa this is a ~1px step buried
    // in the gap below (invisible, as before), but when the glyph is
    // softened (low outer solidity) the gap no longer reaches zero at the
    // wedge boundary, so a hard step here would show as a straight cut on
    // the slice's sides. Soft ownership lets adjacent slices cross-fade
    // (the loop keeps the max), so the sector edges stay soft.
    let own = smoothstep(-aa, aa, c1) * smoothstep(-aa, aa, -c2);
    let gap_half = slice_gap_half();
    let g = (1.0 - aa_inside(gap_half, abs(c1), aa))
        * (1.0 - aa_inside(gap_half, abs(c2), aa));
    return band * own * g;
}

// Color at absolute MIDI `pitch`, read from the pitch gradient LUT so an
// octave glyph is the same hue as the disc that pitch would light.
fn pitch_lut_color(pitch: f32) -> vec3<f32> {
    let t = clamp((pitch - u.misc2.x) / max(u.misc2.y - u.misc2.x, 0.01), 0.0, 1.0);
    let f = t * f32(PITCH_LUT_N - 1u);
    let i0 = u32(floor(f));
    let i1 = min(i0 + 1u, PITCH_LUT_N - 1u);
    return mix(u.pitch_lut[i0].rgb, u.pitch_lut[i1].rgb, f - floor(f));
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

// Three octaves of value noise: enough turbulence to read as gas without
// eating fill-rate. Normalized back to ~0..1.
fn fbm(p: vec2<f32>) -> f32 {
    var v = 0.0;
    var amp = 0.5;
    var q = p;
    for (var i = 0u; i < 3u; i = i + 1u) {
        v = v + amp * vnoise(q);
        q = q * 2.03 + vec2<f32>(1.7, -4.3);
        amp = amp * 0.5;
    }
    return v * (1.0 / 0.875);
}

fn rot2(a: f32) -> mat2x2<f32> {
    let c = cos(a);
    let s = sin(a);
    return mat2x2<f32>(vec2<f32>(c, s), vec2<f32>(-s, c));
}

// ---- Field styles ----------------------------------------------------------
// Everything except Steady: an active disc is painted by a FIELD — a
// function of position that drives BOTH a brightness profile and a position
// along the octave color gradient below, so every sounding octave's color is
// present somewhere in the disc — in its own patches, bands, or cells,
// interpolated where they meet, never averaged into a single hue. Vortex
// drives the field with noise (the gas look); the pattern styles
// (checker/spiral/pinwheel) with deterministic geometry mapped onto a
// sphere and softened by the same noise wobble, so they share the gassy
// look while staying recognizably structured.
//
// All fields are functions of GLOBAL time and a stable per-node seed (never
// the per-note age/seed): the field at a node is one continuous flow that a
// note lights up, so pressing, retriggering, or stacking octaves never
// restarts or reshuffles the pattern.

// Mirrors NodeStyle::is_field_style in lattice-scene; keep the two in sync.
// Steady is index 0; every kept field style has a nonzero index.
fn is_field_style(style: u32) -> bool {
    return style != 0u;
}

// Color at position `t` (0..1) along the gradient of SOUNDING octaves: the
// sounding slots' pitch colors in pitch order, each owning a span of t
// proportional to its level, blending linearly between span centers. A gas
// pixel's noise value picks its t, so patch area tracks octave loudness.
// Falls back to the node color when every octave envelope has faded (the
// disc can outlive them when the note fade is longer).
fn octave_swirl_color(octaves: vec3<u32>, cents: f32, t: f32, fallback: vec3<f32>) -> vec3<f32> {
    var total = 0.0;
    for (var i = 0u; i < OCTAVE_SLOTS; i = i + 1u) {
        total = total + octave_level(octaves, i);
    }
    if total < 1e-3 {
        return fallback;
    }
    let pick = clamp(t, 0.0, 1.0) * total;
    // Kernel-weighted blend: each octave contributes its color weighted by
    // its level times a bump centered on its span, so pixels inside a span
    // show ~pure color and seams crossfade narrowly. The level factor is
    // the point: a releasing octave's color influence fades out WITH its
    // envelope. (The previous span-center interpolation handed the
    // outermost octaves full-color ownership of the gradient ends no
    // matter how faded they were, so their color vanished with a snap the
    // instant the level reached zero.)
    var acc = 0.0;
    var wsum = 0.0;
    var csum = vec3<f32>(0.0);
    for (var i = 0u; i < OCTAVE_SLOTS; i = i + 1u) {
        let level = octave_level(octaves, i);
        if level <= 0.0 {
            continue;
        }
        let center = acc + level * 0.5;
        acc = acc + level;
        // Bump width tracks the span (+ a small floor so it never hits
        // zero); at full level a neighbor's kernel is ~exp(-4) inside this
        // span, keeping patches pure rather than averaged.
        let x = (pick - center) / (level * 0.5 + 0.02);
        let w = level * exp(-x * x);
        // Slot i is MIDI octave i (its C is MIDI (i+1)*12), same mapping as
        // the dots style.
        wsum = wsum + w;
        csum = csum + pitch_lut_color((f32(i) + 1.0) * 12.0 + cents / 100.0) * w;
    }
    if wsum < 1e-5 {
        return fallback;
    }
    return csum / wsum;
}

// Concentration of each octave's angular color lobe in the glow (a von
// Mises-like falloff): higher is tighter, more separated arcs. Tuned so
// octaves a dot-step (45deg) apart blend softly rather than banding.
const GLOW_LOBE_KAPPA: f32 = 4.0;

// The glow's color when a chord sounds: every sounding octave's hue laid
// around the halo in the direction of its OWN dot (the shared dots angle
// convention), so the glow shows ALL the playing notes at once instead of
// just the loudest voice's single color. Seam-free — each octave's weight
// is a periodic bump in cos(angle - dot_angle), never an atan2 wrap. A
// lone sounding octave yields its color uniformly (the single term cancels
// the angle dependence); with a solo note or nothing sounding it falls
// back to `fallback`, so a single voice keeps its exact color (fixed
// channel hues included, which the pitch ramp would not reproduce).
fn octave_glow_color(octaves: vec3<u32>, cents: f32, angle: f32, fallback: vec3<f32>) -> vec3<f32> {
    var count = 0u;
    var wsum = 0.0;
    var csum = vec3<f32>(0.0);
    for (var i = 0u; i < OCTAVE_SLOTS; i = i + 1u) {
        let level = octave_level(octaves, i);
        if level <= 0.0 {
            continue;
        }
        count = count + 1u;
        // Octave i's dot angle (matches outer_glyph): middle C straight
        // up, 45deg clockwise per octave, this node's pitch class folded in.
        let octaves_from_mid_c = (f32(i) - MIDDLE_C_SLOT) + cents / 1200.0;
        let theta = 1.5707963 - RAD_PER_OCTAVE * octaves_from_mid_c;
        let w = level * exp(GLOW_LOBE_KAPPA * (cos(angle - theta) - 1.0));
        // Slot i is MIDI octave i, whose C is MIDI (i+1)*12; fold in this
        // node's pitch class for the octave's true pitch (as the dots do).
        csum = csum + pitch_lut_color((f32(i) + 1.0) * 12.0 + cents / 100.0) * w;
        wsum = wsum + w;
    }
    // A solo note (or none) keeps its exact node color; two or more
    // sounding octaves spread their hues around the glow.
    if count < 2u || wsum < 1e-5 {
        return fallback;
    }
    return csum / wsum;
}

// Orthographic sphere mapping for the pattern styles: the unit-sphere
// point under this pixel. uv is clamped just inside the disc rim, so the
// glow region beyond the disc keeps sampling the limb color instead of
// hitting the degenerate z=0 edge. Feeding patterns angles from here
// (longitude/latitude/polar) instead of raw uv is what makes them read
// as painted ON a ball: equal angular steps compress toward the limb.
fn sphere_point(uv: vec2<f32>) -> vec3<f32> {
    let r = length(uv);
    let p = uv * (min(r, 0.495) / max(r, 1e-5));
    return vec3<f32>(p * 2.0, 2.0 * sqrt(max(0.25 - dot(p, p), 0.0)));
}

// A field style's scalar fields at one pixel: x picks the color along the
// octave swirl gradient, y modulates brightness (billows / streaks / cells).
// `time` is global (u.misc.x), `seed` the stable per-node hash.
fn field_pattern(style: u32, uv: vec2<f32>, d: f32, time: f32, seed: f32) -> vec2<f32> {
    if style == 3u {
        // Vortex: differential rotation shears the colors into spiral
        // streaks, like paint stirred with a spoon. Only integer multiples
        // of the angle go through cos(), so there is no seam at +-pi.
        let ang = atan2(uv.y, uv.x);
        let spin = 2.0 * ang + 4.0 * d - time * 1.1 + seed;
        let wob = (fbm(uv * 3.1 + vec2<f32>(seed * 5.1, time * 0.16)) - 0.5) * 2.6;
        let t = 0.5 + 0.5 * cos(spin + wob);
        // Brightness streaks rotate rigidly with the swirl (rotated-domain
        // noise stays seam-free where angle-domain noise would not).
        let q = rot2(time * 0.5) * uv;
        let lum = 0.74 + 0.5 * fbm(q * 3.0 + vec2<f32>(seed * 5.0, 0.0));
        return vec2<f32>(t, lum);
    } else if style == 11u {
        // Pinwheel: beach-ball sectors — azimuthal color waves around a
        // pole tilted toward the viewer, so the wedges curve over the
        // sphere like a globe seen at an angle. The integer harmonic keeps
        // the atan2 wrap invisible.
        let q0 = sphere_point(rot2(seed) * uv);
        let q = vec3<f32>(q0.x, 0.62 * q0.y - 0.78 * q0.z, 0.78 * q0.y + 0.62 * q0.z);
        let phi = atan2(q.x, q.z);
        let wob = (fbm(uv * 2.4 + vec2<f32>(seed * 6.3, time * 0.12)) - 0.5) * 2.6;
        let lum = 0.78 + 0.4 * fbm(uv * 3.1 + vec2<f32>(seed * 2.7, time * 0.09));
        return vec2<f32>(0.5 + 0.5 * cos(3.0 * phi + time * 0.25 + wob), lum);
    } else if style == 12u {
        // Spiral: two-armed spiral of color waves winding out from the
        // face center; the polar-angle radial term keeps the arms hugging
        // the sphere. Integer angular harmonic, so no atan2 seam.
        let q = sphere_point(uv);
        let rho = acos(clamp(q.z, -1.0, 1.0));
        let ang = atan2(uv.y, uv.x);
        let wob = (fbm(uv * 2.5 + vec2<f32>(seed * 5.9, time * 0.12)) - 0.5) * 2.4;
        let lum = 0.78 + 0.4 * fbm(uv * 3.0 + vec2<f32>(seed * 3.3, -time * 0.10));
        return vec2<f32>(0.5 + 0.5 * cos(rho * 7.0 - 2.0 * ang - time * 0.5 + seed + wob), lum);
    }
    // Checker (the only remaining field style): soft cells on the globe
    // graticule (longitude x latitude), tilted per node and slowly
    // revolving, so the cells foreshorten toward the limb like a painted
    // ball. cos*cos keeps the borders soft; middle octave colors seep
    // through them.
    let q = sphere_point(rot2(seed * 0.7) * uv);
    let lon = atan2(q.x, q.z);
    let lat = asin(clamp(q.y, -1.0, 1.0));
    let wob = (fbm(uv * 2.7 + vec2<f32>(seed * 4.9, time * 0.12)) - 0.5) * 1.2;
    let v = cos(lon * 4.0 + time * 0.22 + seed + wob) * cos(lat * 4.0 - wob);
    let lum = 0.78 + 0.35 * fbm(uv * 3.1 + vec2<f32>(seed * 2.7, time * 0.10));
    return vec2<f32>(0.5 + 0.5 * v, lum);
}

// Halo density (0..~1.5) at this pixel: turbulent gas wafting off the
// star, replacing the old smooth exponential fringe (which read as a
// halo pasted behind the disc). Three ingredients sell the motion:
// - plume: a slowly evolving per-direction strength choosing WHERE the
//   surface vents, so the fringe is plumes and gaps, not an even rim;
// - waft: turbulence whose noise domain slides along the radial axis
//   over time, so the detail visibly streams OUTWARD, and whose sample
//   direction rotates with height so wisps curl sideways as they rise
//   (curl direction/strength is per-node);
// - reach: the decay length itself follows plume and waft, giving an
//   irregular, filamentous outer edge instead of a level exp() falloff.
// Everything is sampled on the direction circle (never atan2), so there
// is no angular seam. Same clock and seed rules as field_pattern.
fn field_halo(style: u32, uv: vec2<f32>, d: f32, time: f32, seed: f32) -> f32 {
    // The direction clamp keeps samples smooth near the disc center,
    // where the halo shows through outlined (ring) nodes.
    let dir = uv / max(d, 0.30);
    let h = max(d - 0.42, 0.0);

    // Style character: where and how strongly the surface vents.
    var plume_dir = dir;
    if style == 3u {
        // Vortex: plumes trail around with the rotation.
        plume_dir = rot2(time * 0.55) * dir;
    }
    let plume = fbm(plume_dir * 2.3 + vec2<f32>(seed * 3.1, time * 0.16));

    // Outward-streaming turbulence, curling as it rises.
    let curl = (fract(seed * 0.618034) - 0.5) * 3.0;
    let waft = fbm(rot2(h * curl) * dir * 1.6 + vec2<f32>(seed * 7.7, d * 3.5 - time * 0.35));

    let reach = 0.5 + 1.4 * plume * plume + 0.6 * (waft - 0.5);
    return exp(-h * 9.0 / max(reach, 0.15)) * (0.30 + 0.70 * waft) * (0.35 + 0.65 * plume);
}

// ---- Trail marks -----------------------------------------------------------
// How a node the music has ALREADY been to differs from one it hasn't. The
// whole feature lives inside idle_marker below, which is the point: it can
// only ever change the small grey mark on a resting node, so no setting and
// no future edit can let a memory read as a sounding note.
//
// Every constant here is a ceiling on how loud a mark can get at strength 1.
// They are deliberately low. A trail is meant to be noticed on the second
// look, not the first.

// Lift: how far toward white the idle grey goes.
const TRAIL_LIFT_MAX: f32 = 0.55;
// Ring: radius of the pale circle, its thickness, its opacity, and how
// pale it is. The radius is the classic disc edge -- the circle sits where
// the note's own core would light, reading as a ghost of it -- and is
// deliberately independent of the idle marker's radius, so the ring keeps
// its size (and stays inside the default octave band) whatever that is set
// to.
const TRAIL_RING_R: f32 = 0.40;
const TRAIL_RING_THICK: f32 = 0.045;
const TRAIL_RING_ALPHA: f32 = 0.55;
const TRAIL_RING_PALE: f32 = 0.45;
// Tint: how much of the remembered note's color the grey takes.
const TRAIL_TINT_MAX: f32 = 0.75;

// The idle (unlit) node marker: a minimal grey mark at a home-sheet
// position, drawn from its OWN uniforms (misc4.z/.w) so it is independent
// of the active appearance. A filled dot or an outline circle at the idle
// radius (style in misc4.w: 0 none, 1 dot, 2 circle), in the idle grey.
// Returns the color premultiplied in .xyz with coverage in .w; style None
// draws nothing. The caller keeps it showing regardless of the note state.
//
// A node the music has been to (visited > 0) wears a quietly different
// version of that same mark -- see the trail constants above. It also gets
// a marker OFF the home sheet, where an unvisited node draws nothing: a
// blank off-sheet node is blank because its pitch would be information from
// nowhere, and having been played there is exactly what answers that.
//
// `tint` is the node's own color, which for a silent node the scene sets to
// the remembered note's (see TrailField::apply).
fn idle_marker(d: f32, home: f32, visited: f32, tint: vec3<f32>, aa: f32) -> vec4<f32> {
    let style = u32(u.misc4.w + 0.5);
    let trail_style = u32(u.misc6.x + 0.5);
    var trail = 0.0;
    if trail_style != 0u {
        trail = clamp(visited, 0.0, 1.0) * clamp(u.misc6.y, 0.0, 1.0);
    }

    // Straight (non-premultiplied) color and coverage; premultiplied on the
    // way out so the two marks below can composite in the obvious order.
    var rgb = u.node_idle.rgb;
    var cov = 0.0;
    if style != 0u && (home >= 0.5 || trail > 0.0) {
        let r = u.misc4.z;
        cov = aa_inside(r, d, aa);                // dot: filled disc
        if style == 2u {                          // circle: hollow it out
            cov = cov * (1.0 - aa_inside(r - IDLE_RING_THICK, d, aa));
        }
        if trail_style == 1u {                    // lift: a lighter grey
            rgb = mix(rgb, vec3<f32>(1.0), trail * TRAIL_LIFT_MAX);
        } else if trail_style == 3u {             // tint: a hint of the note
            rgb = mix(rgb, tint, trail * TRAIL_TINT_MAX);
        }
    }

    // The pale circle is its own mark rather than a change to the marker,
    // so it still reads with the idle marker turned off.
    if trail_style == 2u && trail > 0.0 {
        let ring = aa_inside(TRAIL_RING_R, d, aa)
            * (1.0 - aa_inside(TRAIL_RING_R - TRAIL_RING_THICK, d, aa));
        let a = ring * trail * TRAIL_RING_ALPHA;
        let pale = mix(u.node_idle.rgb, vec3<f32>(1.0), TRAIL_RING_PALE);
        // Circle over the marker, premultiplied.
        return vec4<f32>(pale * a + rgb * cov * (1.0 - a), a + cov * (1.0 - a));
    }

    return vec4<f32>(rgb * cov, cov);
}

// ---- Melody / bass marks ---------------------------------------------------
// Two full rings concentric with the octave band: the melody just INSIDE
// it, the bass just OUTSIDE. Radius is what tells them apart -- each ring is
// drawn in its own note's color, not a fixed livery -- and because they
// never share one, both draw at full weight even when ONE note is both ends
// of the chord (a lone held note, or a chord whose top and bottom share a
// pitch class). Earlier passes had to split a rim between two colors to say
// that, and the pass before them dropped the mark on the floor entirely.
//
// What a full ring cannot say is WHICH of the node's octaves is the melody,
// and on a chord voiced inside one pitch class that is the whole question --
// so a link device ties the ring back to the sector responsible. See
// MarkLink for the candidates; the modes here mirror its shader_index.

// Floor on the ring's thickness (the view sets the rest, u.misc5.w), in
// soft-band widths — about a couple of render pixels, so a thin setting
// can't go sub-pixel on a densely packed lattice and read as nothing. A
// thickness of exactly 0 is the off state and skips the floor.
const MARK_RING_MIN_AA: f32 = 1.5;

// The ring's opacity at this pixel, given the slot(s) the mark came from.
//
// Each ring is slit at the responsible sector's two angular boundaries. A
// slit IS the gap between two octaves continued outward: the same
// perpendicular-distance test against the same boundary line, with the same
// soft band, measured on the pixel ITSELF -- projecting to the band first
// would scale both the width and the blur by the ring's radius over the
// band's, which reads as a wider, softer cut.
//
// Those slits leave the stretch of ring beside the marked sector separated
// from the remainder of the circle. That stretch always draws full; `rest`
// fades everything else, from a whole ring (1) down to just that arc (0).
//
// The dot gate throws away the antipode: a boundary line runs through the
// origin, so it passes just as close on the far side of the node and would
// otherwise cut the ring twice. The ownership test needs no such gate --
// its two smoothsteps disagree in sign there and it falls to zero.
//
// A slot mask can name more than one sector: releasing the top of a chord
// leaves the old melody fading on its slot while the new one takes another,
// and both are the melody for as long as that lasts.
fn mark_ring_alpha(slots: u32, cents: f32, uv: vec2<f32>, rest: f32, aa: f32) -> f32 {
    let half = slice_gap_half();
    let hb = RAD_PER_OCTAVE * 0.5;
    var own = 0.0;
    var slit = 0.0;
    for (var i = 0u; i < OCTAVE_SLOTS; i = i + 1u) {
        if (slots & (1u << i)) != 0u {
            let octaves_from_mid_c = (f32(i) - MIDDLE_C_SLOT) + cents / 1200.0;
            let ang = 1.5707963 - RAD_PER_OCTAVE * octaves_from_mid_c;
            let b1 = vec2<f32>(cos(ang + hb), sin(ang + hb));
            let b2 = vec2<f32>(cos(ang - hb), sin(ang - hb));
            let c1 = uv.x * b1.y - uv.y * b1.x;
            let c2 = uv.x * b2.y - uv.y * b2.x;
            own = max(own, smoothstep(-aa, aa, c1) * smoothstep(-aa, aa, -c2));
            let facing = step(0.0, dot(uv, vec2<f32>(cos(ang), sin(ang))));
            let cut = max(aa_inside(half, abs(c1), aa), aa_inside(half, abs(c2), aa));
            slit = max(slit, cut * facing);
        }
    }
    return mix(rest, 1.0, own) * (1.0 - slit);
}

// Coverage of one mark ring, `r_in..r_out`. Radii are passed rather than
// derived so the bass ring (outside the band) and the melody ring (inside)
// can share this one body.
fn mark_ring(
    slots: u32,
    cents: f32,
    uv: vec2<f32>,
    r_in: f32,
    r_out: f32,
    rest: f32,
    aa: f32,
) -> f32 {
    // No room for this ring: the band's inner radius can be dialed to 0
    // (pie wedges), which leaves the inner ring nothing to sit in.
    if r_out <= 0.0 || r_out <= r_in {
        return 0.0;
    }
    let d = length(uv);
    let ring = aa_inside(r_out, d, aa) * (1.0 - aa_inside(max(r_in, 0.0), d, aa));
    return ring * mark_ring_alpha(slots, cents, uv, rest, aa);
}

@fragment
fn fs_main(in: VsOut) -> @location(0) vec4<f32> {
    let d = length(in.uv); // 0 at center, 1 at quad edge (2x disc radius)
    let activation = in.params.x;

    let style = u32(u.misc.w + 0.5);
    let seed = in.seed;

    // Screen-constant soft-band width: uv units per pixel (uv.x is linear
    // across the billboard, so fwidth is uniform over the quad and safe to
    // take before any branching), scaled to the softness knob. Shape edges
    // below use this instead of fixed-uv smoothsteps.
    let aa = aa_width(fwidth(in.uv.x));

    // Core layer, unified onto ONE solidity axis. The radius (u.misc3.x,
    // quad UV units) sizes it, and a radius of 0 turns it off entirely — no
    // enum, no separate flag. Solidity (u.misc4.x, 0..1) morphs the core
    // between the two ends of a single shape: 0 is a soft under-glow, 1 is
    // the classic solid orb, and in between the opaque disc fades in over
    // its glow skirt while its edge crisps. Channel-14 voices render as an
    // outline ring instead of a filled disc (v1 semantics) so the channel
    // stays recognizable; unplayed nodes draw no disc (the grid gap marks
    // the position). Activation fades the disc in and back out on release.
    let outlined = in.params.w;
    let presence = activation;
    let radius = max(u.misc3.x, 0.0);  // core radius; 0 = off
    let solidity = u.misc4.x;          // 0 glow .. 1 orb
    // Radius 0 is off; fade the whole core in over the first sliver of the
    // bar so it grows from nothing with no pop. `on` gates the (skippable)
    // field work; core_on scales everything the core draws.
    let on = radius > 0.0;
    let core_on = smoothstep(0.0, CORE_FADE_IN, radius);

    // Channel-14 outline ring, at the core radius.
    let ring = aa_inside(radius, d, aa) * (1.0 - aa_inside(radius - 0.12, d, aa));
    // The opaque disc: a core of radius R whose edge softens (CORE_EDGE_SOFT)
    // and whose opacity fades as solidity drops, so a full orb (solidity 1:
    // crisp screen-constant edge, fully opaque) dissolves smoothly into
    // nothing (solidity 0: the glow skirt below then carries the note
    // alone). At solidity 1 this is exactly the classic aa_inside(R) disc.
    let edge = aa + (1.0 - solidity) * CORE_EDGE_SOFT;
    let core_cov = 1.0 - smoothstep(radius - edge, radius + edge, d);
    let filled = core_cov * solidity;
    let disc = mix(filled, ring, outlined) * presence * core_on;

    // Soft additive-looking glow for active nodes. The exponential alone
    // never reaches zero, so the quad boundary showed as a boxy halo;
    // window it so it fades to exactly zero (with zero slope) inside the
    // quad edge.
    let window = 1.0 - smoothstep(0.5, 0.95, d);
    // Field/glow domain scale: the paint and falloff were authored for the
    // classic 0.46 orb, so scale their coordinates to the sized core (a
    // bigger radius spreads the glow with it; at the default radius this is
    // 1). The glow dims toward the soft end of the solidity axis (an
    // under-glow) and brightens toward the orb; it drops to zero when None.
    let fs = CORE_R_CLASSIC / max(radius, 0.1);
    let glow_base = mix(0.35, 0.6, solidity);
    var glow = glow_base * activation * exp(-3.0 * d * fs) * window;

    // Field styles waft turbulent gas off the surface instead of a smooth
    // halo (see field_halo — venting plumes, outward-streaming curl, ragged
    // reach). Crossfade it in with solidity, so a solid gas orb gets its
    // wisps while the glow end stays a clean under-glow; clocked on global
    // time so a retrigger never restarts the motion.
    if on && is_field_style(style) && activation > 0.0 {
        let gassy = activation * field_halo(style, in.uv * fs, d * fs, u.misc.x, seed) * window;
        glow = mix(glow, gassy, solidity);
    }
    glow = glow * core_on;

    let brightness = level_floor(activation);
    // Every sounding octave's color, blended by angle — each hue laid in
    // its dot's direction (see octave_glow_color). This is the node's
    // multi-color fill: Steady shows it directly on the disc (so a chord's
    // disc mixes ALL its notes, not just the loudest), and the glow skirt
    // carries the same blend, so disc and halo read as one colored field. A
    // solo note falls back to its single color.
    let octave_mix =
        octave_glow_color(in.octaves, in.cents, atan2(in.uv.y, in.uv.x), in.color.rgb) * brightness;
    var rgb = octave_mix;

    // Field styles instead paint the disc as a ball of gas or patterned
    // light. The field sweeps each pixel through the sounding octaves'
    // colors (patches, bands, or cells — never averaged), modulates
    // brightness, and a limb-darkened profile keeps it reading as a sphere.
    // Only the solid part shows it (the disc coverage weights it into the
    // composite below), so it dissolves with the disc toward the glow end.
    if on && is_field_style(style) && activation > 0.0 {
        let field = field_pattern(style, in.uv * fs, d * fs, u.misc.x, seed);
        let gas = octave_swirl_color(in.octaves, in.cents, field.x, in.color.rgb);
        let limb = 1.12 - 0.35 * smoothstep(0.0, 0.5, d * fs);
        rgb = mix(in.color.rgb * brightness, gas * brightness * field.y * limb, activation);
    }
    let disc_rgb = rgb;

    // Composite disc OVER glow: the disc keeps its own color, the multi-
    // color glow reads only where the disc doesn't reach (the halo). `f` is
    // the share of this pixel's coverage that is glow-beyond-disc; where
    // disc and glow are the same color it collapses to a no-op, so combined
    // alpha and brightness are exactly the old additive glow.
    let glow_rgb = octave_mix;
    let f = glow * (1.0 - disc) / max(disc + glow, 1e-4);
    let rgb_core = mix(disc_rgb, glow_rgb, f);

    // The active note's core (disc + glow), premultiplied. The idle marker
    // (below) is composited UNDER this, so a sounding note draws over its
    // own idle marker and reveals it again as it fades.
    let core_alpha = clamp(disc + glow, 0.0, 1.0);
    var base_alpha = core_alpha;
    var base_rgb = rgb_core * core_alpha;

    // Octave indicators, composited over the disc/glow. Each slot fades on
    // its own envelope. Whichever element covers a pixel most strongly owns
    // its color there: sounding glyphs are tinted by their own pitch;
    // ghosts and the rest use the whitened node color.
    // The outer layer is on or off; there is one glyph shape (see
    // OuterStyle), whose index the scene still passes through misc.z.
    let outer_on = u.misc.z > 0.5;
    let node_glyph_rgb = mix(in.color.rgb, vec3<f32>(1.0, 1.0, 1.0), 0.55);
    var glyph = 0.0;
    var glyph_rgb = node_glyph_rgb;

    // Backdrop opacity 0..1, scaling the built-in ghost level: 0
    // draws no backdrop at all, 1 is the full strength this always had.
    let backdrop = u.misc3.w;
    let has_backdrop = backdrop > 0.0;
    // Outer solidity (u.misc4.y, 0..1) is the octave layer's own
    // crisp/soft knob: it widens every glyph's soft edge (proportional to
    // the band width so narrow bands soften proportionally), so at 1 the
    // shapes are crisp (the classic look) and toward 0 they melt into soft
    // glowy marks. It only feeds the edge width, so shapes and angles stay
    // put. Mirrors the core's solidity.
    let outer_aa = aa + (1.0 - u.misc4.y) * OUTER_GLOW_SOFT * (u.misc3.z - u.misc3.y);
    // Melody/bass ring geometry.
    let band_in = u.misc3.y;
    let band_out = u.misc3.z;
    let ring_thick = u.misc5.w;
    let ring_w = select(max(ring_thick, outer_aa * MARK_RING_MIN_AA), 0.0, ring_thick <= 0.0);
    let ring_gap = slice_gap_half() * 2.0;
    let mark_rest = clamp(u.misc5.y, 0.0, 1.0);
    // Headroom: the band's outer radius can be dialed to 1.0, so the outer
    // ring lives in the QUAD_MARGIN margin. Cap it inside the billboard (a
    // circle of radius QUAD_MARGIN fits the square quad) and ease it off
    // there, rather than letting the corner clip it flat.
    let lim = QUAD_MARGIN - 0.02;
    let outer_in = min(band_out + ring_gap, lim);
    let inner_out = band_in - ring_gap;
    if outer_on {
        // Sounding slots draw bright, tinted by their own pitch, each
        // fading on its own envelope. The backdrop opacity (its own
        // outer-layer setting, independent of the core) fades in the
        // layer's cohesion device: the silent slots drawn as ghosts in the
        // loop below.
        let ghosted = has_backdrop;
        for (var i = 0u; i < OCTAVE_SLOTS; i = i + 1u) {
            let level = octave_level(in.octaves, i);
            if level <= 0.0 && !(ghosted && presence > 0.0) {
                continue;
            }
            let shape = outer_glyph(i, in.cents, in.uv, band_in, band_out, outer_aa);
            // Ghosts complete the circle silhouette in the note's own
            // color; a sounding slot never dips below its ghost, so a
            // fading octave hands off to it instead of leaving a hole.
            var cov = shape * GHOST_LEVEL * backdrop * presence * f32(ghosted);
            var slot_rgb = node_glyph_rgb;
            if level > 0.0 {
                // The 35% dimmest-visible floor is right while the octave
                // sounds, but a released glyph must END at nothing: without
                // this taper it holds 35% brightness all the way down the
                // fade and then pops off. Ease the last 15% of the envelope
                // to zero instead (the max() hands a backdrop slot off to
                // its ghost as the lit coverage sinks through it).
                let tail = smoothstep(0.0, 0.15, level);
                cov = max(cov, shape * level_floor(level) * tail);
                // Slot i is MIDI octave i, whose C is MIDI (i+1)*12; add
                // this node's pitch class for the glyph's true pitch.
                let pitch = (f32(i) + 1.0) * 12.0 + in.cents / 100.0;
                // Exactly the color that pitch lights everywhere else. The LUT
                // is the pitch ramp, which is defined already lightened
                // (pitch_ramp_lch / NOTE_LIGHTEN in lattice-scene), and the
                // core disc and the piano roll sample that same ramp — so all
                // three read as one color. No separate white mix here anymore.
                slot_rgb = pitch_lut_color(pitch);
            }
            if cov > glyph {
                glyph = cov;
                glyph_rgb = slot_rgb;
            }
        }
    }
    // Fade a soft (low-solidity) glyph out across the billboard's margin
    // instead of letting the quad boundary clip it flat. The fade starts at
    // uv 1.0 — the outer band's own limit — so a crisp glyph (which never
    // reaches past its band, all within uv 1.0) is untouched; only the soft
    // overflow into the headroom is eased to zero by GLYPH_FADE_LIMIT.
    glyph = glyph * (1.0 - smoothstep(1.0, GLYPH_FADE_LIMIT, d));

    // Melody/bass rings, bracketing the octave band: melody inside, bass
    // outside — the ring's radius echoes where its note sits in the chord.
    // Their own layer, composited over the glyphs — a sector's color is its
    // pitch, which is what the octave layer is FOR, so nothing here
    // repaints one.
    let melody_cov = mark_ring(
        in.marks.x, in.cents, in.uv,
        inner_out - ring_w, inner_out,
        mark_rest, outer_aa,
    ) * in.params.y;
    let bass_cov = mark_ring(
        in.marks.y, in.cents, in.uv,
        outer_in, min(outer_in + ring_w, lim),
        mark_rest, outer_aa,
    ) * in.params.z;
    // Disjoint radii, so at most one of the two covers any given pixel.
    var mark = max(melody_cov, bass_cov);
    let mark_rgb = select(in.bass_color.rgb, in.melody_color.rgb, melody_cov > bass_cov);
    // Safety taper only. The radii above are already capped inside the
    // billboard (a circle of radius QUAD_MARGIN fits the square quad), so
    // this just keeps a soft edge from ending on the boundary; starting it
    // any earlier eats the ring, which at the default band (outer 1.0)
    // lives entirely in this margin.
    mark = mark * (1.0 - smoothstep(QUAD_MARGIN - 0.04, QUAD_MARGIN, d));
    glyph_rgb = (mark_rgb * mark + glyph_rgb * glyph * (1.0 - mark))
        / max(mark + glyph * (1.0 - mark), 1e-4);
    glyph = mark + glyph * (1.0 - mark);

    // The active note: glyph over (disc + glow), premultiplied.
    let active_alpha = glyph + base_alpha * (1.0 - glyph);
    let active_rgb = glyph_rgb * glyph + base_rgb * (1.0 - glyph);

    // The idle marker (how an unlit home-sheet node reads), computed from
    // its OWN uniforms — independent of the active appearance AND of the
    // note state: it is drawn at full strength always. A sounding note
    // simply composites over it below (occluding it where the note is
    // opaque, showing it around/through the note otherwise).
    let idle = idle_marker(d, in.home, in.visited, in.color.rgb, aa);

    // Active over idle, premultiplied: a sounding note draws over its own
    // marker; the marker is unchanged whether or not a note plays.
    let over_idle = active_alpha + idle.a * (1.0 - active_alpha);
    let final_rgb = active_rgb + idle.rgb * (1.0 - active_alpha);

    // The knockout gutter (off-sheet nodes only; in.gutter is 0 on the home
    // sheet). This is what lets the sevens layer overlap the home sheet
    // instead of needing clearance of its own: the node clears its own
    // footprint out of whatever was drawn before it and sits in the hole.
    //
    // TWO things make it read as a hole rather than a dark blob stuck on the
    // picture, and it needs both:
    //
    //  - It clears to the GROUND (u.background), not to black. With no color
    //    of its own a premultiplied layer knocks out to black, and black is
    //    several shades darker than this skin's panel, so the cleared disc
    //    announced itself everywhere — including over empty lattice, where
    //    it should be invisible. Against the real ground it disappears
    //    wherever it crosses nothing.
    //  - It FADES rather than ending at a rim. A hard circle cutting across
    //    a lit ring reads as a bite taken out of it; a gradient reads as the
    //    small node sitting in front. The clearing is solid across the
    //    node's own footprint and eases off over a band twice the gutter
    //    width, which is what the setting really buys.
    //
    // Compositing it UNDER the node's own paint is what the `(1 - over_idle)`
    // terms say: the node keeps its color exactly, and the ground only fills
    // the part of its quad the node itself leaves empty.
    var gutter_cov = 0.0;
    if in.gutter > 0.0 {
        // Solid out to the node's own outermost feature — the melody ring
        // when the rings are on, the band's outer edge otherwise — then the
        // falloff. Capped inside the quad so the tail finishes as a circle
        // instead of being clipped square at the billboard's corners.
        let rings = select(0.0, u.misc5.z + u.misc5.w, u.misc5.w > 0.0);
        let solid = u.misc3.z + rings;
        let fade = min(solid + in.gutter * 2.0, QUAD_MARGIN - 0.05);
        // Scaled by the note's OWN envelope, the same one `presence` paints
        // the node with, so the clearing fades out with the note instead of
        // outliving it. The width above stays put while it does: a hole
        // that shrinks as it fades reads as the node retreating, and a hole
        // that holds full strength to the last frame (which is what scaling
        // the width alone did) vanishes with an audible pop.
        gutter_cov = (1.0 - smoothstep(min(solid, fade - 0.001), fade, d)) * activation;
    }
    let final_alpha = over_idle + gutter_cov * (1.0 - over_idle);
    if final_alpha < 0.01 {
        discard;
    }
    let with_ground = final_rgb + u.background.rgb * gutter_cov * (1.0 - over_idle);
    return vec4<f32>(with_ground, final_alpha);
}

// ---- Chord edges & grid lines ----------------------------------------------
// One pipeline, two kinds of instance: beams between simultaneously
// sounding, lattice-adjacent nodes (a held chord's interval structure
// rendered as geometry), and the faint background grid between node
// positions (segments arrive pre-inset from the scene, leaving a gap at
// every node position). Drawn under the nodes, grid first.

struct EdgeInstance {
    // xyz: endpoint A, w: strength (chord: min of the two node
    // activations; grid: line opacity)
    @location(0) a_strength: vec4<f32>,
    // xyz: endpoint B, w: kind (0 chord beam, 1 grid line, 2 dashed grid)
    @location(1) b_kind: vec4<f32>,
    @location(2) color: vec4<f32>,
};

struct EdgeVsOut {
    @builtin(position) clip_pos: vec4<f32>,
    // x: 0..1 along the edge, y: -1..1 across it
    @location(0) uv: vec2<f32>,
    @location(1) color: vec4<f32>,
    @location(2) strength: f32,
    @location(3) @interpolate(flat) kind: f32,
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
    let b = inst.b_kind.xyz;
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
    // Grid lines (kind >= 1) are much thinner than chord beams, and carry
    // the user's thickness multiplier; chord beams keep their fixed width.
    let is_grid = min(inst.b_kind.w, 1.0);
    let grid_width = 0.09 * u.misc5.x;
    let half_width = u.misc.y * mix(0.35, grid_width, is_grid);
    let world = a + axis * corner.x + perp * corner.y * half_width;

    var out: EdgeVsOut;
    out.clip_pos = u.view_proj * vec4<f32>(world, 1.0);
    out.uv = corner;
    out.color = inst.color;
    out.strength = inst.a_strength.w;
    out.kind = inst.b_kind.w;
    return out;
}

@fragment
fn fs_edge(in: EdgeVsOut) -> @location(0) vec4<f32> {
    // Screen-constant soft band across the beam (see aa_inside; computed
    // before the branch so the derivative stays in uniform control flow).
    let aa_y = aa_width(fwidth(in.uv.y));

    // Grid line: uniformly faint with a screen-constant soft edge (line
    // edge at the old 0.35..1.0 band's midpoint), the ends easing off
    // toward the node gaps.
    if in.kind > 0.5 {
        let across = aa_inside(0.675, abs(in.uv.y), aa_y);
        var along = smoothstep(0.0, 0.12, in.uv.x) * (1.0 - smoothstep(0.88, 1.0, in.uv.x));
        // Dashed grid lines (kind 2): the sevens links, plus every in-plane
        // line when the grid's dashed style is on. Chop the length into
        // short dashes, leaving a faint floor in the gaps so the line still
        // reads as continuous structure.
        if in.kind > 1.5 {
            let tri = abs(fract(in.uv.x * 5.0) - 0.5) * 2.0;
            along = along * (0.15 + 0.85 * smoothstep(0.35, 0.65, tri));
        }
        let alpha = in.strength * across * along;
        if alpha < 0.01 {
            discard;
        }
        return vec4<f32>(in.color.rgb * alpha, alpha);
    }

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
