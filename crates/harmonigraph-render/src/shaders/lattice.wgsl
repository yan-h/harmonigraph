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
    // x: global time (s, wraps hourly). The shimmer clocks on this: its
    //    sheet spans the whole lattice, so every node has to read one clock
    //    (at worst the sheet jumps once an hour at the wrap).
    // y: base node radius (world units),
    // z: unused,
    // w: unused — it carried the node style, the core orb's paint, back
    //    when the core had more than one. A retired slot rather than a
    //    repack, which would renumber the ones around it for nothing.
    misc: vec4<f32>,
    // x: darkest_pitch, y: brightest_pitch (MIDI notes); z: render scale
    // (offscreen pixels per screen pixel — converts the screen-pixel
    // softness knob to render pixels); w: bloom strength, which blit.wgsl
    // reads off this same buffer — NOT a free slot, whatever the fact that
    // nothing in this file touches it suggests. An octave glyph maps its
    // pitch through x/y to index pitch_lut.
    misc2: vec4<f32>,
    // x: core radius in quad UV units (0 turns the core off). y/z: the
    // outer layer's inner/outer band radii (same units; the scene
    // guarantees z > y). w: unused — it carried the backdrop opacity, which
    // is fixed on below.
    misc3: vec4<f32>,
    // Pitch->color lookup for the octave glyphs. The disc is colored through
    // this same table on the CPU, so a glyph and the disc under it match
    // exactly rather than closely (length mirrors
    // harmonigraph_scene::PITCH_LUT_N).
    pitch_lut: array<vec4<f32>, 64>,
    // Idle node color (the view's grid color): the home-sheet placeholder ring is
    // drawn in this constant grey, so a releasing note's ring stays grey
    // (not the note hue) and never snaps color when the voice is pruned.
    node_idle: vec4<f32>,
    // x: core solidity (0 = soft glow, 1 = solid orb) — the single axis the
    // core layer runs on. y: unused — it carried the octave glyphs'
    // solidity, which is fixed crisp below. z: idle marker radius.
    // w: idle marker style (0 none, 1 dot, 2 circle).
    misc4: vec4<f32>,
    // x: grid line thickness, a multiple of the built-in grid width.
    // y: unused.
    // z: padding inside the octave layer, in quad UV units — the gap
    // between neighbouring sectors AND between the band and the mark
    // rings. w: melody/bass ring thickness, same units; 0 = no rings.
    misc5: vec4<f32>,
    // x: trail mark style — how a node the music has already visited is
    //    marked (0 off, 1 lift, 2 ring, 3 tint; see TrailMark). y: trail
    //    strength 0..1. z: the sevens knockout's fade width, read below by
    //    the vertex stage. w: the melody/bass mark rings' shimmer pattern
    //    (0 off, then one index per pattern; see Pulse::shader_index), read
    //    by mark_pulse — NOT a free slot.
    // x and y are read ONLY by idle_marker: a memory must never be mistakable
    // for a sounding note, and confining them to the idle layer is what
    // guarantees that rather than merely intending it.
    misc6: vec4<f32>,
    // The ground the lattice is painted onto (the pane fill this pass is
    // composited over). Only the sevens knockout reads it: without it the
    // gutter can knock out only to black, which is darker than the pane and
    // reads as a plate sitting ON the picture rather than a hole THROUGH it.
    background: vec4<f32>,
    // The wheel. x: octaves one turn is cut into; y: the MIDI pitch at the top
    // of every node; z, w unused.
    // Which SLOTS a node draws, and how far its ring is turned, are derived
    // per node from these — both depend on the node's pitch class, so there is
    // no one answer to send.
    misc7: vec4<f32>,
    // The shimmer, for whichever layers are running it. x: how fast the bands
    // travel, in world units per second; y: how wide they are, in world units
    // (the scene floors it above zero — the band phase divides by it); z: how
    // deep the light they carry is, 0 none and 1 the tuned depth; w unused.
    // One set for both layers: see the Shimmer section below.
    misc8: vec4<f32>,
    // The angle from a ring's own seam to each of its slice boundaries, four
    // to a row and read through oct_bound(): boundary j walking clockwise. One
    // table for every node, since the widths are the node's only in where they
    // are turned to. Computed on the CPU (harmonigraph_scene's
    // `octave_layout`) because it depends on settings alone — the alternative
    // is accumulating the same widths per pixel per sector.
    oct_bounds: array<vec4<f32>, 3>,
};

const TAU: f32 = 6.2831853;

// Billboard headroom past the octave band's outer edge (uv 1.0): the quad
// and its uv are both scaled by this, so the uv->world mapping is
// unchanged (disc, band, glyphs, glow all render identically) but there is
// margin out to this radius for things that live OUTSIDE the band -- the
// mark rings, which at the default band (outer 1.0) sit entirely out here.
// Costs a bit of fill (bigger quads, which alpha-blend and discard where
// empty).
const QUAD_MARGIN: f32 = 1.6;
// Where the octave layer's overflow past uv 1.0 -- the aa fringe of a band
// dialed right out to the edge -- finishes easing off, rather than being
// cut flat by the quad boundary. Pinned to what QUAD_MARGIN was when this
// fade was tuned, so widening the billboard for the mark rings doesn't
// quietly restyle the glyph edges.
const GLYPH_FADE_LIMIT: f32 = 1.3;
// Where the glow's window closes: past this the exponential is multiplied by
// exactly zero, which is what stops the quad boundary reading as a boxy halo.
// Named because `core_reach` is built on it — but it is only ONE of that
// bound's two arms, and the disc's own reach passes it once the core radius
// is dialed much past two thirds. Collapsing `core_reach` to this constant
// clips a big soft core, which is why the bound is a max rather than this.
const GLOW_LIMIT: f32 = 0.95;

@group(0) @binding(0) var<uniform> u: Uniforms;

// The node's own outermost feature, in ITS uv. That is the BASS ring when
// this node is wearing one — the bass ring is the outer of the two, riding
// just past the octave band — and the band's own outer edge when it is not.
//
// Per node, not per view: assuming the ring is always there made every
// clearing as wide as the widest node's, so a node with no ring sat in a
// gap visibly bigger than itself.
//
// Scaled by how far the ring has EASED IN rather than switched on the
// moment it is claimed: the rim sets how wide this node clears the sheets
// behind it, and a rim that jumped to its full reach ahead of the ring left
// the hole in the lattice popping open around a ring still fading up. It
// still steps in the instant the key comes up, because that is when the
// ring itself goes (marks are held-only).
fn node_rim(bass: f32) -> f32 {
    let ring = max(u.misc5.z, 0.0) + u.misc5.w;
    return u.misc3.z + select(0.0, ring * bass, u.misc5.w > 0.0);
}

// How much of the outer (bass) ring this node is wearing, 0..1: it needs
// both a slot to link back to and a level to draw at.
fn bass_ring_level(marks: vec2<u32>, params: vec4<f32>) -> f32 {
    return select(0.0, clamp(params.z, 0.0, 1.0), marks.y != 0u);
}

// How far the billboard has to reach, in uv, for a clearing of reach `g` to
// finish inside it as a circle rather than being clipped square at the
// corners. Never smaller than QUAD_MARGIN, so a node without a gutter — and
// every node on a lattice with no depth — is sized exactly as before.
fn quad_margin(rim: f32, g: f32) -> f32 {
    return max(QUAD_MARGIN, rim + g + 0.05);
}

// Whether the fragment shader may stop early where it can prove it would
// paint nothing (see `paint_reach` and the idle branch in `fs_main`). Only
// ever false in the parity test, which compiles a second pipeline with this
// flipped and requires the two to render the same pixels — the early-outs
// are an optimization, and the test is what keeps them one.
const EARLY_OUT: bool = true;

// How far from the node's center anything can paint, in its own uv.
//
// The billboard is deliberately bigger than the node: QUAD_MARGIN of
// headroom for the mark rings and a soft glyph's overflow, more when a
// gutter has to finish inside it. Between that circle of content and the
// square quad lies a lot of fragment — most of a quad, once the corners are
// counted — where every layer below computes its coverage, arrives at zero,
// and blends nothing. On a zoomed-in lattice, where one node can cover the
// pane, that is the frame's dominant cost.
//
// Every term here is the radius at which the corresponding layer's own
// smoothstep has reached zero, so the bound is exact rather than generous:
//
//   - the glow's `window` closes at 0.95, inside GLYPH_FADE_LIMIT;
//   - the octave glyphs (and their eased-off fringe) end at
//     GLYPH_FADE_LIMIT;
//   - the core disc — and channel 14's ring — end at their radius plus the
//     widest edge softness the solidity axis can ask for;
//   - the idle marker ends at its own radius, or the trail ring's;
//   - the mark rings taper off at QUAD_MARGIN, but only exist while a slot
//     is marked;
//   - the knockout clears out to rim + gutter, and nothing beyond.
fn paint_reach(in: VsOut, aa: f32) -> f32 {
    var reach = max(GLYPH_FADE_LIMIT, u.misc3.x + CORE_EDGE_SOFT + aa);
    reach = max(reach, max(u.misc4.z, TRAIL_RING_R) + aa);
    if in.marks.x != 0u || in.marks.y != 0u {
        reach = max(reach, QUAD_MARGIN);
    }
    if in.gutter > 0.0 {
        reach = max(reach, in.rim + in.gutter);
    }
    return reach;
}

struct Instance {
    @location(0) world_pos: vec3<f32>,
    @location(1) color: vec4<f32>,
    // x: activation 0..1, w: outlined 0/1 (channel-14 voices render as a
    // ring, not a disc). y/z: the melody and bass marks' own levels, which
    // follow the marked voice rather than this node's activation — each
    // ring eases in over the scene layer's attack when its note takes that
    // end, and drops to 0 the frame the key comes up.
    @location(2) params: vec4<f32>,
    // Per-octave activation, 8 bits per slot, little-endian packed.
    @location(3) octaves: vec3<u32>,
    // The node's pitch class in cents (0..1200). It both PLACES the octave
    // indicators and COLORS them, off the one quantity: each indicator's
    // octave has a pitch, that octave's C plus this, and the indicator sits
    // at that pitch's angle on the shared axis (see oct_sector) in that
    // pitch's color.
    @location(4) cents: f32,
    // 1 on the home (center sevens) sheet: idle home nodes draw a blank
    // placeholder ring where their disc would be.
    @location(5) home: f32,
    // Melody/bass marks: x = melody slots, y = bass slots, one bit per
    // octave slot. Which SECTOR each mark's ring links back to (see
    // mark_ring); the ring itself is per node, and its fade level rides
    // params.y/params.z.
    @location(6) marks: vec2<u32>,
    // Each mark's own color: its own sector's pitch off the ramp, with no lift
    // on top of it (see NodeInstance::melody_color), so a ring reads as
    // belonging to the indicator it points at rather than as a fixed livery.
    @location(7) melody_color: vec4<f32>,
    @location(8) bass_color: vec4<f32>,
    // How strongly the music is remembered at this node, 0..1 (see
    // NodeInstance::trail). Feeds the idle marker and nothing else.
    @location(9) visited: f32,
    // The sevens layer: x = billboard size factor (1 on the home sheet,
    // smaller with every step off it), y = knockout gutter width, in the uv
    // units of a FULL-SIZE node — the vertex shader divides by the size
    // factor so the gap comes out the same width whatever the node's size.
    // 0 means no gutter. See ViewConfig::sevens_size / _gutter.
    @location(10) sevens: vec2<f32>,
};

struct VsOut {
    @builtin(position) clip_pos: vec4<f32>,
    @location(0) uv: vec2<f32>, // -1..1 across the quad
    @location(1) color: vec4<f32>,
    @location(2) params: vec4<f32>,
    @location(3) @interpolate(flat) octaves: vec3<u32>,
    @location(4) @interpolate(flat) cents: f32,
    @location(5) @interpolate(flat) home: f32,
    @location(6) @interpolate(flat) marks: vec2<u32>,
    @location(7) @interpolate(flat) melody_color: vec4<f32>,
    @location(8) @interpolate(flat) bass_color: vec4<f32>,
    @location(9) @interpolate(flat) visited: f32,
    // Already converted to THIS node's uv (see vs_main).
    @location(10) @interpolate(flat) gutter: f32,
    // The node's own outermost feature and the clearing's fade width, both
    // in this node's uv — computed once in the vertex shader because both
    // depend on the instance's size and its mark state.
    @location(11) @interpolate(flat) rim: f32,
    @location(12) @interpolate(flat) soft: f32,
    // Where this fragment sits on the plane the billboards face, in world
    // units: its world position resolved onto the camera's own right/up
    // axes. Every billboard faces that same plane, so this is ONE coordinate
    // system spanning the whole lattice — which is what lets the shimmer be
    // a single sheet of bands crossing node after node instead of a copy per
    // node. Interpolated rather than flat for the same reason: a band has to
    // cross a node, not step from one to the next.
    @location(13) field: vec2<f32>,
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
    let scale = max(inst.sevens.x, 0.05);
    // The gutter is a constant width ON SCREEN, not a share of the node.
    // uv is the node's own coordinate system, so it shrinks with the node —
    // which meant a half-size node cleared a half-size gap, and the gap read
    // as a property of the note rather than of the layer it sits on. Dividing
    // by the scale converts the setting from "of this node" back to "of a
    // full-size node", i.e. one fixed distance everywhere.
    let gutter_uv = max(inst.sevens.y, 0.0) / scale;
    let rim = node_rim(bass_ring_level(inst.marks, inst.params));
    // ...which can want more room than the standard billboard has, on the
    // smallest sheets. Only then does the quad grow: uv 1.0 still maps to
    // the same world distance either way, so nothing about the node's own
    // content moves.
    let margin = quad_margin(rim, gutter_uv);
    let radius = u.misc.y * 0.90 * 2.0 * margin * scale;

    let world = inst.world_pos
        + (u.cam_right.xyz * corner.x + u.cam_up.xyz * corner.y) * radius;

    var out: VsOut;
    out.clip_pos = u.view_proj * vec4<f32>(world, 1.0);
    out.uv = corner * margin;
    out.color = inst.color;
    out.params = inst.params;
    out.octaves = inst.octaves;
    out.cents = inst.cents;
    out.home = inst.home;
    out.marks = inst.marks;
    out.melody_color = inst.melody_color;
    out.bass_color = inst.bass_color;
    out.visited = inst.visited;
    out.gutter = gutter_uv;
    out.rim = rim;
    // The shimmer's shared coordinate — see VsOut::field. Taken off the
    // CORNER's world position rather than the node's center, so the field
    // varies across the quad and the interpolator hands the fragment shader
    // the real plane position of every pixel.
    out.field = vec2<f32>(dot(world, u.cam_right.xyz), dot(world, u.cam_up.xyz));
    // The fade is a constant width on screen too, so it converts the
    // same way the reach does.
    out.soft = u.misc6.z / scale;
    return out;
}

// Octave indicator slots the packing carries: MIDI octaves -1..=9, so slot =
// octave + 1 and slot s is the octave whose C is MIDI 12*s. Which of them a
// node DRAWS is the span setting and its own pitch class — see oct_ring. A
// ring at the pitch limits reaches for slots outside this, which draw as
// backdrop and never light; oct_slot_level is what holds the lookup inside.
// (octave_level unpacks without that guard, and is not the one to reach for.)
const OCTAVE_SLOTS: u32 = 11u;

// Octaves one turn can be cut into (harmonigraph_scene::MAX_SPAN). One fewer
// than the boundary table's entries, since a slice needs a boundary each end.
const MAX_SPAN: u32 = 11u;

// Length of the pitch->color LUT (mirrors harmonigraph_scene::PITCH_LUT_N
// and the `pitch_lut` array in Uniforms).
const PITCH_LUT_N: u32 = 64u;

// Coverage of `x` inside the threshold `edge`, with a screen-constant
// soft band: `w` is ~a pixel expressed in `x`'s units (from fwidth at
// the call site — taken at the top of the fragment fn, outside any
// non-uniform control flow). Fixed-width smoothstep edges blur as a
// quad grows on screen and alias as it shrinks; this keeps every shape
// edge equally soft at all zooms. The glow deliberately keeps its
// proportional falloff.
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

// ---- Where the octave indicators sit ---------------------------------------
// The wheel is SPAN octaves to the turn with the CENTER pitch straight up, and
// every node draws the span octaves of ITSELF nearest that center. Three
// things follow, and they are the whole layout:
//
//   - a slice is exactly one octave, so with no extras it is exactly a turn
//     over the span — on every node, whatever its pitch class;
//   - the center pitch is straight up on every node too, which is what makes
//     the top of the picture mean one pitch across the whole lattice;
//   - a node whose class sits d semitones from the center's has its whole ring
//     TURNED by d — left below, right above, never more than half a slice —
//     which is what puts each of its slices at the angle of the pitch it
//     stands for.
//
// What moves with the turn is the seam, the point where the ring's lowest
// slice meets its highest and the pitch wraps a whole span of octaves. It is
// at the bottom for the center's own pitch class and turns away from it with
// everything else.
//
// The widths are computed on the CPU (harmonigraph_scene's `octave_layout`)
// and read out of `oct_bounds` here — one table, shared by every node, giving
// the angle from a ring's own seam to each of its slice boundaries.

// The span, held inside the boundary table it indexes (oct_walk reads
// bound(span)), so a stale or oversized uniform draws a wrong sector rather
// than reading past the last row.
fn oct_span() -> u32 {
    return clamp(u32(u.misc7.x), 1u, MAX_SPAN);
}
// The MIDI pitch at the top of every node's wheel.
fn oct_center() -> f32 {
    return u.misc7.y;
}
// Straight up, in these angles: the bottom of a node is a quarter turn back
// from zero and clockwise — the direction pitch rises — subtracts.
const OCT_UP: f32 = -0.75 * TAU;
// Boundary `j` of a ring, four to a uniform row. j runs 0..span: 0 is the seam
// and span is the same seam a full turn on.
fn oct_bound(j: u32) -> f32 {
    return u.oct_bounds[j / 4u][j % 4u];
}
// Angle from a ring's seam to `x` slices along it, walking clockwise. Linear
// inside a slice, so a pitch stands at the same fraction of its own octave's
// wedge as it does of the octave.
fn oct_walk(x: f32) -> f32 {
    let c = clamp(x, 0.0, f32(oct_span()));
    let j = min(u32(max(floor(c), 0.0)), oct_span() - 1u);
    return mix(oct_bound(j), oct_bound(j + 1u), c - f32(j));
}
// MIDI pitch of octave slot `s` on a node whose pitch class is `cents`: slot
// s is the octave whose C is MIDI 12*s. Signed, since a ring at the pitch
// limits names slots the packing has no room for.
fn oct_slot_pitch(s: i32, cents: f32) -> f32 {
    return f32(s) * 12.0 + cents / 100.0;
}
// Where one node's ring sits: the slot of its LOWEST slice, and the angle of
// the seam the walk starts from.
struct OctRing {
    base: i32,
    seam: f32,
}
// Derived per node (harmonigraph_scene's `ring`) — where its class falls
// against the center is what decides both which octaves it draws and how far
// it is turned. Per node is also as often as it is worth deriving, so callers
// take it ONCE per fragment and hand it down; nothing below recomputes it per
// slot, per sector or per edge, which is what it would come to inside the
// loops.
fn oct_ring(cents: f32) -> OctRing {
    let off = cents / 100.0;
    let span = i32(oct_span());
    // The node's octave nearest the center, and how far above the center it
    // sits. Halves round up, so a node exactly a tritone away counts as the
    // half octave ABOVE.
    let nearest = floor((oct_center() - off) / 12.0 + 0.5);
    let d = nearest * 12.0 + off - oct_center();
    // The span octaves nearest the center: symmetric when the span is odd, and
    // when it is even one octave deeper on the side of the node's nearest
    // octave the center itself sits, with a tie going down.
    var low = -(span - 1) / 2;
    if (span % 2) == 0 {
        low = select(-span / 2, 1 - span / 2, d < 0.0);
    }
    var ring: OctRing;
    ring.base = i32(nearest) + low;
    // Turned so the CENTER pitch lands straight up. Solved for rather than
    // derived from the ring's middle: with extras the slice the center falls
    // in is not one span-th of the turn, and the pitch sits at its own
    // fraction of whatever width that slice has.
    let along = (oct_center() - oct_slot_pitch(ring.base, cents)) / 12.0 + 0.5;
    ring.seam = OCT_UP + oct_walk(along);
    return ring;
}
// The two angular edges of slot `s`'s indicator, in the order the wedge tests
// below want them: x the counter-clockwise edge, y the clockwise one. Exactly
// its own octave's ends, at every slot — nothing is cut to fit, which is what
// keeps the indicators meeting edge to edge and closing the ring.
fn oct_sector(s: i32, ring: OctRing) -> vec2<f32> {
    let i = u32(clamp(s - ring.base, 0, i32(oct_span()) - 1));
    return vec2<f32>(ring.seam - oct_bound(i), ring.seam - oct_bound(i + 1u));
}
// Where an indicator "points": the angle of its own pitch, which is the middle
// of its wedge — for anything that needs one angle for the whole of it rather
// than its two edges.
fn oct_mid(s: i32, ring: OctRing) -> f32 {
    let e = oct_sector(s, ring);
    return 0.5 * (e.x + e.y);
}
// The level of slot `s`, or nothing when the ring names an octave the packing
// has no room for: a ring near the pitch limits draws octaves no note can
// reach, and those are backdrop that never lights.
fn oct_slot_level(octaves: vec3<u32>, s: i32) -> f32 {
    if s < 0 || s >= i32(OCTAVE_SLOTS) {
        return 0.0;
    }
    return octave_level(octaves, u32(s));
}
// Whether a pixel's angle falls inside sector `edges`'s own arc (between its
// two boundaries), 0..1 with a soft edge over `aa`. Shared by `outer_glyph`
// (where it decides which wedge owns the pixel) and `mark_ring_alpha` (where
// it decides how much of the pulse's "near the marked slice" phase a pixel
// gets), so the two agree on exactly the same wedge rather than each running
// its own copy that could drift.
//
// A wedge under a half turn is the INTERSECTION of its two half-planes; one
// PAST a half turn is their union, and reading it as an intersection would
// empty the sector instead of filling it — see `outer_glyph`'s own note on
// `an_indicator_can_pass_a_half_turn_but_never_a_whole_one`.
fn oct_arc_coverage(edges: vec2<f32>, uv: vec2<f32>, aa: f32) -> f32 {
    let b1 = vec2<f32>(cos(edges.x), sin(edges.x));
    let b2 = vec2<f32>(cos(edges.y), sin(edges.y));
    let c1 = uv.x * b1.y - uv.y * b1.x;
    let c2 = uv.x * b2.y - uv.y * b2.x;
    let s1 = smoothstep(-aa, aa, c1);
    let s2 = smoothstep(-aa, aa, -c2);
    return select(s1 * s2, 1.0 - (1.0 - s1) * (1.0 - s2), edges.x - edges.y > TAU * 0.5);
}

// The octave glyphs' shimmer pattern (u.misc7.z — see `Pulse`).
fn pulse_octaves_mode() -> u32 {
    return u32(u.misc7.z + 0.5);
}
// The melody/bass mark rings' shimmer pattern (u.misc6.w — see `Pulse`).
fn pulse_marks_mode() -> u32 {
    return u32(u.misc6.w + 0.5);
}

// ---- Shimmer: one sheet of soft white light over the whole lattice --------
// Every pulse mode but 0, and the same animation in each: a pattern of light
// laid over the layer, travelling. What the mode picks is its SHAPE.
//
// WHICH layer it covers is not quite the same question. The octave layer's
// sheet stays the whole octave layer; the mark rings' also takes the octave
// slices those rings point at (`mark_slice` in `fs_main`), because a mark is
// the ring together with the octave it names rather than the annulus alone.
//
// The field is SHARED. Every node samples the same sheet at its own place on
// `in.field` — the plane the billboards face, in world units — so the light
// crosses the lattice as one thing rather than each node running a copy of it
// inside its own uv, which would read as many small identical animations
// rather than one big one.
//
// There is no texture, for the same reason there is no per-node phase: every
// pattern here is a handful of dot products, sines and one power, so the
// shared sheet is a few lines of arithmetic instead of an upload, a sampler
// and a bind group. It is also seamless and resolution-free, which a scrolled
// image is not — and being built from sines rather than sampled is what lets
// it be band-limited honestly below.
//
// World units rather than screen pixels: the DAW window and a 1080p offline
// render then draw the same pattern at the same size ON THE LATTICE, where a
// period in pixels would lay a different number of cells across the picture
// in each — the same look in the plugin and in the exported video is worth
// more here than a pattern that holds still while the camera moves. Both the
// settings below are in those units for that reason.
//
// Distance from one bright peak to the next (u.misc8.y, the view's Width
// bar), and how fast the sheet travels (u.misc8.x, the Speed bar). The pair
// sizes and moves ONE shape: the softness below is what shares the period out
// between the lit part and the dark, so a wider setting widens both together
// rather than spacing out peaks of a fixed size. See `ViewConfig::shimmer_width`
// for what a setting under the node spacing costs.
fn shimmer_period() -> f32 {
    // The scene clamps this well clear of zero; the floor is here so a hand-
    // built Scene in a test cannot divide by it either.
    return max(u.misc8.y, 0.01);
}
// The exponent the raised cosine is taken to, from the Softness bar
// (u.misc8.w): 8 at 0, 1 at 1, log-spaced so equal drags are equal RATIOS of
// sharpness rather than equal steps of an exponent, which is not a scale
// anyone reads by eye.
//
// It is the whole of what the bar means, and the two ends are different
// pictures. High up, the pattern IS the cosine: every point of the period is
// on its way somewhere and the brightest part fades into the clearest across
// the whole gap, which is the gradual sweep. Low down the peak narrows to a
// crest on a layer otherwise at rest — a hard band with edges, and at a tight
// width a stripe laid on the layer rather than light crossing it.
//
// Clamped rather than trusted to the scene's clamp, because the range is what
// keeps the shape a shape: below 1 the exponent would widen the lit part past
// the dark and invert the pattern into holes.
const SHIMMER_SHARP_MAX: f32 = 3.0;
fn shimmer_sharpness() -> f32 {
    return exp2(SHIMMER_SHARP_MAX * (1.0 - clamp(u.misc8.w, 0.0, 1.0)));
}
// How far a peak pulls the layer toward white, at intensity 1.
//
// This is the term that costs something real, and it is the ask: light that
// only went half way to white would not read as WHITE light. Where it crosses
// a sounding glyph it leaves an eighth of that octave's pitch color, so under
// a peak an indicator says "an octave sounds here" without saying which — the
// layer's whole message, spent for the sweep. What keeps it payable is that
// the peak is a small part of the period and moving, so any given indicator
// is legible again a second later. The Intensity bar is where that trade is
// made rather than here: this is where the full-strength end of it sits.
//
// Note it is NOT the bound SHIMMER_TROUGH is held to below. That one keeps
// every indicator VISIBLE at every instant; this one gives up their colors
// under a passing peak and nothing else.
const SHIMMER_WHITE: f32 = 0.85;
// What the layer's coverage sits at between peaks at intensity 1, against 1
// under one. A shallow dip on purpose: it is the trough that gives the sweep
// a body to travel through, but the indicators still have to be readable at
// every moment of the cycle. Intensity past 1 deepens it, which is that
// promise being spent — deliberately, by the bar.
const SHIMMER_TROUGH: f32 = 0.82;
// How much of that tuned depth this view asks for (u.misc8.z, the Intensity
// bar). It scales the pair above TOGETHER, so the shimmer is one shape at
// every setting rather than a brightness and a fade to be dialed against each
// other, and 0 leaves `shimmer_terms` returning its exact identity — the
// layer as it draws unshimmered, from the bar rather than from the mode.
fn shimmer_depth() -> f32 {
    return max(u.misc8.z, 0.0);
}
// Which way the octave layer's sheet is laid and travels. A diagonal because
// the lattice's own structure is upright — its rows of fifths and thirds — so
// a pattern along either axis would run parallel to something already in the
// picture and read as part of it.
const SHIMMER_ANGLE: f32 = 0.125 * TAU;
// The mark rings' sheet is laid a QUARTER TURN from the octaves', which is the
// 90 degrees between the two textures. Written as an offset from the one
// angle rather than as a second literal, so the two cannot drift out of
// square when the diagonal is retuned.
const SHIMMER_QUARTER: f32 = 0.25 * TAU;
// Where one fragment's shimmer stops being resolvable: how much of a period
// a single pixel spans. Below the first the sheet is drawn at full strength;
// past the second it is gone, and between them it fades.
//
// The upper bound is Nyquist — half a period to a pixel is the tightest a
// sampled sine can mean anything at all, and past it the pattern does not get
// finer, it gets WRONG: a moire of the sampling grid, which crawls as the
// camera moves and lands differently in the DAW window than in a render of
// another size. So the sheet is faded to nothing rather than aliased, and a
// width dialed past what the zoom can carry settles to the layer's own steady
// look. The lower bound sits well short of Nyquist so the fade is finishing
// where the moire would be starting, rather than racing it.
//
// Both are scaled by a ROOT TWO the Width bar never sees, because the period
// the bar sets is not every pattern's finest feature. Crossing two gratings
// multiplies into their sum and difference frequencies (Checker literally so,
// Weave with a crease along the same diagonals), and those run at k*sqrt(2) —
// so a Checker at the bar's Nyquist is already half a period past its own.
// The row is faded on its tightest member rather than per pattern: one fade
// for one sheet, and what Bands gives up for it is a slightly earlier finish
// at a width no shot is framed for anyway. Weave's crease has no band limit
// at all, being a corner rather than a sine; the scaling is what keeps its
// gratings honest, and the crease is one line rather than a field.
const SHIMMER_RESOLVE_FULL: f32 = 0.14;
const SHIMMER_RESOLVE_GONE: f32 = 0.35;
// Turn a unit vector by `angle`, for the gratings below.
fn rotated(v: vec2<f32>, angle: f32) -> vec2<f32> {
    let c = cos(angle);
    let s = sin(angle);
    return vec2<f32>(v.x * c - v.y * s, v.x * s + v.y * c);
}
// The signed pattern at `p`, in -1..1 — mode by mode, and every one of them
// built out of gratings of the SAME period, so the Width bar means one thing
// across the row. `d` is the direction the sheet is laid along and `n` its
// perpendicular; `field` and `slide` are the unslid position and how far the
// sheet has travelled, which only the radial pattern needs apart.
//
// The three that cross gratings are the tessellating family the checkerboard
// belongs to: multiply two and you get its cells, take the brighter of two
// and you get the lines between them instead, sum three at sixty degrees and
// the cells come out hexagonal — which lands on the lattice better than
// squares do, its rows running three ways rather than two.
fn shimmer_pattern(
    mode: u32, p: vec2<f32>, d: vec2<f32>, n: vec2<f32>,
    field: vec2<f32>, slide: f32,
) -> f32 {
    let k = TAU / shimmer_period();
    if mode == 2u {
        // Checker: two gratings at right angles, multiplied. The product is
        // +1 where both crests meet AND where both troughs do, which is the
        // checkerboard's two colors of cell; it crosses zero along the lines
        // between them.
        return sin(k * dot(p, d)) * sin(k * dot(p, n));
    }
    if mode == 3u {
        // Hex: three gratings sixty degrees apart, summed. Their wavevectors
        // close a triangle (the middle one is the sum of the outer two), so
        // the three crests can meet, and where they do the sum reaches 3
        // against a floor of -1.5 — bright cells rarer and sharper than the
        // dark between them, which IS the honeycomb. Mapped through that
        // range rather than divided by the count, which would leave the
        // pattern unable to reach either end.
        //
        // COSINES, where every other pattern here takes sines, and the
        // asymmetry is the whole point: sin(A) + sin(C) + sin(A+C) is an odd
        // function, so it runs a symmetric ±3sqrt(3)/2 with as many dark
        // blobs as bright and no crest that ever reaches full. Only the
        // cosine form has the crest the honeycomb is made of, and it is the
        // one the -1.5..3 above is the range of.
        let a = cos(k * dot(p, d));
        let b = cos(k * dot(p, rotated(d, TAU / 6.0)));
        let c = cos(k * dot(p, rotated(d, TAU / 3.0)));
        return (a + b + c - 0.75) / 2.25;
    }
    if mode == 4u {
        // Weave: the same two crossed gratings as Checker with the BRIGHTER
        // taken instead of the product, which lights the lines where Checker
        // lights the cells, and brightest where two lines cross.
        return max(sin(k * dot(p, d)), sin(k * dot(p, n)));
    }
    if mode == 5u {
        // Rings: one grating of the distance from the origin, which is why
        // this is the arm that takes `field` and `slide` apart. A translated
        // field would slide the center off the lattice, where having one is
        // what this pattern is FOR, so the travel goes into the radius. The
        // `length` sits inside this arm rather than at the call site so the
        // four patterns that never look at it do not pay for a square root.
        return sin(k * (length(field) - slide));
    }
    // Bands (mode 1): one grating along the sheet's own direction.
    return sin(k * dot(p, d));
}
// What the shimmer does to a layer here, as (white mix, coverage scale):
// how far to pull its color toward white at this fragment, and what to scale
// its coverage by. Both terms and not just one — an octave ghost is already
// most of the way to white and would barely move on color alone, and
// coverage is an opacity that cannot go past 1, so the brightening has to be
// the color's job and the dip has to be the coverage's.
//
// The identity (0, 1) in mode 0, so a caller applies it unconditionally and
// Off stays byte-for-byte the look it was. The coverage term is never ABOVE 1
// either, which is what keeps `paint_reach` exact: a shimmering layer can only
// cover less than it did, so no bound out there moves.
//
// `footprint` is how much of the field one pixel spans, in the field's own
// world units, taken with the other derivatives at the top of the fragment
// body — derivatives have to be in uniform control flow, and by here the
// shader has already been free to discard.
//
// The layer's direction arrives as `quarter_turns` off the base diagonal
// rather than as the vector itself, so the cos/sin that build it sit AFTER
// the early return. Passed as a vector they would be evaluated at the call
// site in every mode, and Off would be free only if the backend inlined this
// and folded the constant — which is the sort of thing that holds on one
// driver and not the next.
fn shimmer_terms(mode: u32, field: vec2<f32>, footprint: f32, quarter_turns: f32) -> vec2<f32> {
    if mode == 0u {
        return vec2<f32>(0.0, 1.0);
    }
    let period = shimmer_period();
    let a = SHIMMER_ANGLE + SHIMMER_QUARTER * quarter_turns;
    let dir = vec2<f32>(cos(a), sin(a));
    let norm = vec2<f32>(-dir.y, dir.x);
    // How far the sheet has slid, plus the one layer separation the quarter
    // turn cannot supply.
    //
    // The turn is what keeps the two layers' sheets two, and for four of the
    // five patterns it is enough on its own: it lands Bands square, and it is
    // not a symmetry of Checker, Weave or Hex, so each comes out somewhere
    // its other copy is not. RINGS has no orientation for a turn to act on —
    // a circle turned a quarter is the same circle — so without something
    // else the mark layer would sit exactly under the octave layer, and a
    // marked slice under two sheets would be under one, with nothing for the
    // crossing below to take the brighter of. Half a period is that
    // something, and it goes ONLY here: added to the square patterns it
    // cancels the turn's own inversion rather than adding to it, and puts
    // their two sheets back into lockstep twice a cycle.
    let slide = u.misc.x * u.misc8.x
        + select(0.0, 0.5 * period * quarter_turns, mode == 5u);
    // The field slid along the sheet's own direction. The radial pattern
    // takes `field` and `slide` apart for itself, inside `shimmer_pattern`.
    let p = field - dir * slide;
    let pattern = shimmer_pattern(mode, p, dir, norm, field, slide);
    // Clamped because the power below is `pow`, which is undefined for a
    // negative base — and sin is only promised to land NEAR its range, so a
    // wave of -1e-8 at a trough would put a NaN into the node's color.
    let wave = clamp(0.5 + 0.5 * pattern, 0.0, 1.0);
    let band = pow(wave, shimmer_sharpness());
    // Fade the sheet out as its period closes on the pixel — see
    // SHIMMER_RESOLVE_*. It rides on the DEPTH rather than on the pattern's
    // amplitude, so what a sheet running out of resolution settles onto is
    // the identity below — the layer's own steady look. Damping the amplitude
    // instead would leave `wave` at a half and the layer under a flat white
    // haze at a flat coverage dip: the average of a sheet nobody can see, and
    // a picture that never returns to the one Off draws.
    let resolve = 1.0 - smoothstep(
        SHIMMER_RESOLVE_FULL,
        SHIMMER_RESOLVE_GONE,
        footprint / period,
    );
    let depth = shimmer_depth() * resolve;
    // Both clamped into what each term can mean rather than trusted to the
    // bar's range: a mix past 1 would overshoot white into whatever the
    // blend does with it, and a coverage scale below 0 would take the layer
    // negative. The peak is clamped BEFORE the band shapes it, so a clamped
    // intensity is still a band and not a flat lid over one.
    let white = min(SHIMMER_WHITE * depth, 1.0);
    let trough = clamp(1.0 - (1.0 - SHIMMER_TROUGH) * depth, 0.0, 1.0);
    return vec2<f32>(white * band, mix(trough, 1.0, band));
}

// ---- Outer octave layer ----------------------------------------------------
// Every outer style draws its glyphs inside the radial band
// [u.misc3.y, u.misc3.z] (quad UV units): the band IS the glyph set's
// radial footprint, so switching styles keeps the octave display the same
// size. The glyphs are drawn identically whatever the core does — the
// layers are independent.
//
// The backdrop is always on: it is the cohesion device that makes a note
// read as ONE whole shape even when a single octave sounds, so the SILENT
// octaves draw as faint ghosts in the note's own color, carrying the ring's
// shape around the bright one. It completes the circle, since the indicators
// tile the whole turn: the only breaks in it are the gaps between them.
//
// The glyphs are always crisp, too: `aa` alone is their edge width, which
// is what makes it screen-constant at every zoom.

// Neighboring sectors are separated by a CONSTANT-thickness gap: the slice
// edges are radial lines offset half the gap from the boundary the two
// sectors share, not constant-angle edges (those read as a V that widens
// outward). That matters more the more indicators there are, and their
// number is a setting. At band inner 0 the sectors become full pie wedges;
// near the center every wedge falls inside the gap band, leaving a small
// clear hub instead of an N-way mush point.
//
// The gap's full width is u.misc5.z (the view's Gap bar), in quad UV units.
// The SAME value separates the mark rings from the band, so one number is
// the padding everywhere in the octave layer.
fn slice_gap_half() -> f32 {
    return max(u.misc5.z, 0.0) * 0.5;
}
// Ghost coverage of a silent slot, scaled by the note's activation so the
// backdrop fades out with the pitch class.
const GHOST_LEVEL: f32 = 0.16;
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

// How much of the glyph BAND this pixel is inside, which every slot's glyph
// is scaled by. It asks only about the radius, so it is the same answer for
// every slot on the node — hoisted out of [`outer_glyph`] so the caller can
// take it once and, where it is zero, skip the whole per-slot loop. The band
// is a narrow annulus (0.64..0.85 by default) and the glyph layer is done
// with by GLYPH_FADE_LIMIT, inside a billboard reaching QUAD_MARGIN or more,
// so most of a lit node's fragments are outside the band, and running the
// loop there is `span` sectors of work for an answer of zero.
fn glyph_band(d: f32, inner: f32, outer: f32, aa: f32) -> f32 {
    return aa_inside(outer, d, aa) * (1.0 - aa_inside(inner, d, aa));
}

// Coverage (0..1) of the outer glyph for octave slot `s` on the node whose
// ring is `ring`, drawn in the uniform band. Reads nothing from the core
// layer — the outer glyphs are independent of it. `aa` is the caller's
// per-pixel soft-band width, giving the shape screen-constant edges.
//
// `band` is this pixel's radial coverage, from [`glyph_band`]: the caller
// holds it because it is the one term here that does not depend on `s`.
fn outer_glyph(
    s: i32, ring: OctRing,
    uv: vec2<f32>, band: f32, aa: f32,
) -> f32 {
    // Annular sectors, screen-constant edges. The directions b1/b2 are this
    // sector's two edges; the cross products against them give BOTH the
    // side-of-line tests (which wedge owns the pixel) and the Euclidean
    // distance to each edge line, thresholded at half the gap width, for a
    // gap of constant thickness at every radius.
    let edges = oct_sector(s, ring);
    let b1 = vec2<f32>(cos(edges.x), sin(edges.x));
    let b2 = vec2<f32>(cos(edges.y), sin(edges.y));
    let c1 = uv.x * b1.y - uv.y * b1.x;
    let c2 = uv.x * b2.y - uv.y * b2.x;
    // Ownership softened over `aa`: a hard step would show as a straight
    // cut down the slice's sides wherever the gap doesn't reach zero at the
    // wedge boundary — a Gap of 0, which closes the sectors into a solid
    // annulus, is exactly that case. Soft ownership lets adjacent slices
    // cross-fade (the loop keeps the max), so the sector edges stay clean.
    //
    // A wedge under a half turn is the INTERSECTION of its two half-planes;
    // one PAST a half turn is their union, and reading it as an intersection
    // would empty the sector instead of filling it. Both cases are real here:
    // one full-size octave flanked by an extra either side, at the thinnest
    // size, pins those two at a tenth of an even slice each and leaves the one
    // between them 336 degrees.
    // That extreme, and the floor that keeps it under a whole turn, are
    // `an_indicator_can_pass_a_half_turn_but_never_a_whole_one` and `MIN_SPAN`
    // in harmonigraph-scene. (The test itself is `oct_arc_coverage`, shared
    // with `mark_ring_alpha`'s pulse split so the two agree on one wedge.)
    let own = oct_arc_coverage(edges, uv, aa);
    // Each edge's gap is cut only on the side the edge actually runs to. The
    // boundary LINE passes just as close on the far side of the node, which
    // falls outside a narrow wedge and so does not matter there, but lands
    // inside a wide one — where it would read as a slit across an indicator
    // that has no boundary at all in that direction.
    let gap_half = slice_gap_half();
    let g = (1.0 - aa_inside(gap_half, abs(c1), aa) * smoothstep(-aa, aa, dot(uv, b1)))
        * (1.0 - aa_inside(gap_half, abs(c2), aa) * smoothstep(-aa, aa, dot(uv, b2)));
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

// ---- The core's color ------------------------------------------------------
// An active disc is one calm shape carrying every sounding octave's color at
// once, laid around it by angle. There is no second paint to switch to: the
// core is this blend, at whatever solidity the disc is set to.

// The TIGHTEST each octave's angular color lobe is drawn at (a von Mises-like
// falloff): higher is tighter, more separated arcs. Tuned so neighbouring
// octaves blend softly rather than banding at the widest span, where they sit
// closest together. A ceiling rather than the concentration itself — a pixel
// near the node's centre is blended at less, so that the seams, which are
// fixed in ANGLE and would otherwise converge to a cusp there, keep the arc
// width they have at the rim (`core_layer`, and note that the rim width is
// 1/sqrt of this, so the two move together).
const GLOW_LOBE_KAPPA: f32 = 4.0;

// The glow's color when a chord sounds: every sounding octave's hue laid
// around the halo in the direction of its OWN indicator, so the glow shows
// ALL the playing notes at once instead of just the loudest voice's single
// color. Seam-free — each octave's weight is a periodic bump in
// cos(angle - indicator angle), never an atan2 wrap. A lone sounding octave
// yields its color uniformly (the single term cancels the angle dependence);
// with a solo note or nothing sounding it falls back to `fallback`, so a
// single voice keeps its exact color (fixed channel hues included, which the
// pitch ramp would not reproduce).
//
// `kappa` is how tightly to pack those hues — at most GLOW_LOBE_KAPPA, and
// less where the caller wants their seams held open (see `core_layer`). At 0
// every weight collapses to the octave's own level and the hues average into
// one color.
fn octave_glow_color(
    octaves: vec3<u32>, cents: f32, ring: OctRing, angle: f32, kappa: f32,
    fallback: vec3<f32>,
) -> vec3<f32> {
    var count = 0u;
    var wsum = 0.0;
    var csum = vec3<f32>(0.0);
    for (var i = 0u; i < OCTAVE_SLOTS; i = i + 1u) {
        let level = octave_level(octaves, i);
        if level <= 0.0 {
            continue;
        }
        count = count + 1u;
        let theta = oct_mid(i32(i), ring);
        let w = level * exp(kappa * (cos(angle - theta) - 1.0));
        // Slot i is MIDI octave i - 1, whose C is MIDI i*12; fold in this
        // node's pitch class for the octave's true pitch.
        csum = csum + pitch_lut_color(f32(i) * 12.0 + cents / 100.0) * w;
        wsum = wsum + w;
    }
    // A solo note (or none) keeps its exact node color; two or more
    // sounding octaves spread their hues around the glow.
    if count < 2u || wsum < 1e-5 {
        return fallback;
    }
    return csum / wsum;
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
// drawn in its own sector's color, not a fixed livery -- and because they
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
// The slits are the whole of the link. An `Unlinked` opacity would fade the
// stretch of ring on the far side of them, down to just the arc over the
// marked sector; the ring being a whole circle that the slits merely break is
// what says which octave WITHOUT spending the shape to say it.
//
// The facing gate throws away the antipode: a boundary line runs through the
// origin, so it passes just as close on the far side of the node and would
// otherwise cut the ring twice. It is taken per EDGE rather than against the
// sector's bisector, which is the same cut while a sector is narrow and the
// right one once a fringe has made it wide (past a half turn, a point by one
// edge faces away from the bisector entirely).
//
// A slot mask can name more than one sector: releasing the top of a chord
// leaves the old melody fading on its slot while the new one takes another,
// and both are the melody for as long as that lasts.
fn mark_ring_alpha(slots: u32, ring: OctRing, uv: vec2<f32>, aa: f32) -> f32 {
    let half = slice_gap_half();
    let top = ring.base + i32(oct_span()) - 1;
    var slit = 0.0;
    for (var i = 0u; i < OCTAVE_SLOTS; i = i + 1u) {
        let s = i32(i);
        if (slots & (1u << i)) != 0u && s >= ring.base && s <= top {
            let edges = oct_sector(s, ring);
            let b1 = vec2<f32>(cos(edges.x), sin(edges.x));
            let b2 = vec2<f32>(cos(edges.y), sin(edges.y));
            let c1 = uv.x * b1.y - uv.y * b1.x;
            let c2 = uv.x * b2.y - uv.y * b2.x;
            let cut = max(
                aa_inside(half, abs(c1), aa) * smoothstep(-aa, aa, dot(uv, b1)),
                aa_inside(half, abs(c2), aa) * smoothstep(-aa, aa, dot(uv, b2)),
            );
            slit = max(slit, cut);
        }
    }
    return 1.0 - slit;
}

// Coverage of one mark ring, `r_in..r_out`. Radii are passed rather than
// derived so the bass ring (outside the band) and the melody ring (inside)
// can share this one body.
fn mark_ring(
    slots: u32, oct: OctRing,
    uv: vec2<f32>, r_in: f32, r_out: f32, aa: f32,
) -> f32 {
    // No room for this ring: the band's inner radius can be dialed to 0
    // (pie wedges), which leaves the inner ring nothing to sit in.
    if r_out <= 0.0 || r_out <= r_in {
        return 0.0;
    }
    let d = length(uv);
    let ring = aa_inside(r_out, d, aa) * (1.0 - aa_inside(max(r_in, 0.0), d, aa));
    // Off the ring entirely: the slits below only ever scale this coverage, so
    // walking the slots for them would be an 11-iteration answer to a pixel
    // that is already zero. A ring is a thin annulus in a billboard reaching
    // QUAD_MARGIN — the margin it lives in is the one the rings are the
    // reason for — so that is nearly all of them.
    if EARLY_OUT && ring <= 0.0 {
        return 0.0;
    }
    return ring * mark_ring_alpha(slots, oct, uv, aa);
}

// How much of the destination a node's knockout clears at radius `d`.
//
// `reach` is where the clearing ENDS, measured past the node's own rim, and
// `soft` is how gradual that ending is — two settings rather than one,
// because tying the fade to the reach meant a wider gap was always a
// blurrier one. Solid from the rim out to `reach - soft`, gone by `reach`.
// The inner bound is floored at the rim so a fade wider than the reach eats
// outward instead of into the node's own footprint, which is the one part
// that always has to be cleared.
fn gutter_coverage(d: f32, rim: f32, reach: f32, soft: f32) -> f32 {
    let edge = rim + reach;
    let inner = max(edge - soft, rim);
    return 1.0 - smoothstep(min(inner, edge - 0.001), edge, d);
}

// The core layer -- the note's disc and the glow skirt around it --
// premultiplied, as `rgb * alpha` in `xyz` and the coverage in `w`.
//
// A layer of its own so that the one bound governing all of it can be stated
// and taken once. Past `core_reach` every term below is exactly zero: the
// glow's `window` closes at GLOW_LIMIT, and the disc's coverage runs out at
// its radius plus the widest edge the solidity axis can ask for (the
// channel-14 ring ends inside that) -- the same bound `paint_reach` takes for
// this layer. It is worth taking because the layer is the expensive one and
// its reach is SHORT: the mark rings live out at QUAD_MARGIN and the glyph
// fade runs to GLYPH_FADE_LIMIT, so the outer half of a lit node's billboard
// is past it -- an atan2 and an 11-slot color blend for a coverage of zero.
fn core_layer(in: VsOut, d: f32, aa: f32, oct: OctRing) -> vec4<f32> {
    let activation = in.params.x;
    let outlined = in.params.w;
    let radius = max(u.misc3.x, 0.0);  // core radius; 0 = off
    let solidity = u.misc4.x;          // 0 glow .. 1 orb
    // Radius 0 is off; fade the whole core in over the first sliver of the
    // bar so it grows from nothing with no pop.
    let core_on = smoothstep(0.0, CORE_FADE_IN, radius);

    let core_reach = max(GLOW_LIMIT, radius + CORE_EDGE_SOFT + aa);
    if EARLY_OUT && d >= core_reach {
        return vec4<f32>(0.0);
    }

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
    let disc = mix(filled, ring, outlined) * activation * core_on;

    // Soft additive-looking glow for active nodes. The exponential alone
    // never reaches zero, so the quad boundary showed as a boxy halo;
    // window it so it fades to exactly zero (with zero slope) inside the
    // quad edge.
    let window = 1.0 - smoothstep(0.5, GLOW_LIMIT, d);
    // Glow domain scale: the falloff is authored for the classic 0.46 orb,
    // so scale its coordinates to the sized core (a bigger radius spreads
    // the glow with it; at the default radius this is 1). The glow dims
    // toward the soft end of the solidity axis (an under-glow) and brightens
    // toward the orb.
    let fs = CORE_R_CLASSIC / max(radius, 0.1);
    let glow_base = mix(0.35, 0.6, solidity);
    let glow = glow_base * activation * exp(-3.0 * d * fs) * window * core_on;

    // Every sounding octave's color, blended by angle — each hue laid in
    // its dot's direction (see octave_glow_color). This is the node's
    // multi-color fill: the disc shows it directly (so a chord's disc mixes
    // ALL its notes, not just the loudest), and the glow skirt carries the
    // same blend, so disc and halo read as one colored field. A solo note
    // falls back to its single color.
    //
    // Those hues are laid in lobes fixed in ANGLE, so the arc a seam spans is
    // d/sqrt(kappa) — it shrinks with the radius, and every seam converges to a
    // cusp at the node's centre. That is a property of the KERNEL, not of any
    // setting: the cusp is there at every solidity, and the glow skirt, which
    // has no solidity of its own, carries the same blend. So the cure is not
    // hung off the solidity axis (whose business is the disc's opacity and its
    // rim, and which is at its widest exactly where there is no disc left to
    // soften) — one width, everywhere.
    //
    // Hold the seams to that width and pick the concentration that gives it:
    // k = (d/seam)^2, capped at GLOW_LOBE_KAPPA so this only ever loosens the
    // blend, never tightens it. The width is the lobe's own arc where the disc
    // ends, radius/sqrt(GLOW_LOBE_KAPPA), carried inward unchanged: the seams
    // then run at one screen width from rim to centre instead of tapering to a
    // point, and no pixel is ever blurrier than the rim already was.
    let seam = radius * inverseSqrt(GLOW_LOBE_KAPPA);
    let kappa = min(GLOW_LOBE_KAPPA, (d * d) / max(seam * seam, 1e-8));
    let octave_mix = octave_glow_color(
        in.octaves, in.cents, oct, atan2(in.uv.y, in.uv.x), kappa, in.color.rgb,
    ) * activation;

    // Disc and glow carry the SAME blend, so there is nothing to composite
    // between them: the color of the pixel is that blend, and its coverage is
    // the two summed — exactly the additive glow. (A disc painted some other
    // way would need the halo held out of it, which is a mix weighted by the
    // share of the coverage that is glow-beyond-disc.)
    //
    // Premultiplied: the idle marker is composited UNDER this, so a sounding
    // note draws over its own marker and reveals it again as it fades.
    let core_alpha = clamp(disc + glow, 0.0, 1.0);
    return vec4<f32>(octave_mix * core_alpha, core_alpha);
}

/// What a node paints at this fragment. Both node entry points below return
/// exactly this; they differ only in how many attachments they write it to.
fn node_paint(in: VsOut) -> vec4<f32> {
    let d = length(in.uv); // 0 at center, 1 at quad edge (2x disc radius)
    let activation = in.params.x;

    // Screen-constant soft-band width: uv units per pixel (uv.x is linear
    // across the billboard, so fwidth is uniform over the quad and safe to
    // take before any branching), scaled to the softness knob. Shape edges
    // below use this instead of fixed-uv smoothsteps.
    let aa = aa_width(fwidth(in.uv.x));
    // How much of the shimmer's shared field one pixel spans, in that field's
    // own world units — what tells `shimmer_terms` when a pattern has run out
    // of pixels to be drawn in. Taken here, beside `aa` and for the same
    // reason: it is a derivative, and by the time the sheet is wanted the
    // shader has already been free to discard. The field's axes are the
    // camera's own right and up, which the screen's x and y run along, so the
    // larger of the two steps IS the world size of a pixel.
    let field_fw = fwidth(in.field);
    let field_step = max(field_fw.x, field_fw.y);

    // Outside everything this node can paint. `fwidth` above is taken first
    // and in uniform control flow, as its comment requires; from here on the
    // shader is free to leave.
    if EARLY_OUT && d > paint_reach(in, aa) {
        discard;
    }

    // An idle node paints its marker and nothing else — no disc (presence
    // gates it), no glow, no glyphs (a ghost needs presence too),
    // no mark rings (their own levels gate them), no knockout (it fades with
    // the note). Everything below still computes all of it and multiplies it
    // away, which on a lattice where most nodes are idle most of the time is
    // most of the fragment work in the frame. The three levels and the
    // octave word are exactly the terms those gates read, so this branch
    // returns what the full path would, not an approximation of it.
    if EARLY_OUT
        && in.params.x <= 0.0
        && in.params.y <= 0.0
        && in.params.z <= 0.0
        && (in.octaves.x | in.octaves.y | in.octaves.z) == 0u
    {
        let marker = idle_marker(d, in.home, in.visited, in.color.rgb, aa);
        if marker.a < 0.01 {
            discard;
        }
        return marker;
    }

    // Where THIS node's ring sits — which octaves it draws and how far it is
    // turned — derived once for the whole fragment and handed to everything
    // below that draws a sector or points at one. It depends on the wheel and
    // the node's pitch class and on nothing per-pixel, so deriving it inside a
    // loop (or inside oct_sector, or per edge) would be the same answer
    // computed dozens of times over. After the idle branch above, which paints
    // no sector at all.
    let oct = oct_ring(in.cents);

    // Core layer, unified onto ONE solidity axis. The radius (u.misc3.x,
    // quad UV units) sizes it, and a radius of 0 turns it off entirely — no
    // enum, no separate flag. Solidity (u.misc4.x, 0..1) morphs the core
    // between the two ends of a single shape: 0 is a soft under-glow, 1 is
    // the classic solid orb, and in between the opaque disc fades in over
    // its glow skirt while its edge crisps. Channel-14 voices render as an
    // outline ring instead of a filled disc (v1 semantics) so the channel
    // stays recognizable; unplayed nodes draw no disc (the grid gap marks
    // the position). Activation fades the disc in and back out on release.
    let presence = activation;
    // The disc and its glow, premultiplied. Everything the layer needs is on
    // the instance or the uniforms, and nothing below reads its internals --
    // see `core_layer`, which is also where it stops being computed at all.
    let core = core_layer(in, d, aa, oct);
    var base_alpha = core.w;
    var base_rgb = core.xyz;

    // Octave indicators, composited over the disc/glow. Each slot fades on
    // its own envelope. Whichever element covers a pixel most strongly owns
    // its color there: sounding glyphs are tinted by their own pitch;
    // ghosts and the rest use the whitened node color.
    // The octave layer always draws — one glyph shape, no on/off. Which
    // octaves it shows is the per-node bitmask, and how much of the band it
    // covers is the band radii; there is nothing left for a switch to say.
    let node_glyph_rgb = mix(in.color.rgb, vec3<f32>(1.0, 1.0, 1.0), 0.55);
    var glyph = 0.0;
    var glyph_rgb = node_glyph_rgb;

    // Melody/bass ring geometry.
    let band_in = u.misc3.y;
    let band_out = u.misc3.z;
    let ring_thick = u.misc5.w;
    let ring_w = select(max(ring_thick, aa * MARK_RING_MIN_AA), 0.0, ring_thick <= 0.0);
    let ring_gap = slice_gap_half() * 2.0;
    // Headroom: the band's outer radius can be dialed to 1.0, so the outer
    // ring lives in the QUAD_MARGIN margin. Cap it inside the billboard (a
    // circle of radius QUAD_MARGIN fits the square quad) and ease it off
    // there, rather than letting the corner clip it flat.
    let lim = QUAD_MARGIN - 0.02;
    let outer_in = min(band_out + ring_gap, lim);
    let inner_out = band_in - ring_gap;
    // Sounding slots draw bright, tinted by their own pitch, each fading on
    // its own envelope; the silent ones draw as the backdrop's ghosts in the
    // loop below, riding the note's own presence so the whole ring fades
    // with the pitch class rather than outliving it.
    //
    // Over the ring's own SLICES rather than over the packing's slots: a ring
    // near the pitch limits reaches for octaves the packing has no room for,
    // and walking the slots instead would drop those and leave a wedge of the
    // backdrop missing. They never light (oct_slot_level), which is all they
    // should never do.
    // The band coverage every slot's glyph is scaled by — the same answer for
    // all of them, so it is taken once here. Zero is most of the node: the
    // band is a narrow annulus in a billboard that reaches QUAD_MARGIN, so
    // outside it the loop below can be skipped entirely rather than run to
    // reach zero `span` times over.
    let band = glyph_band(d, band_in, band_out, aa);
    // The slots a melody or bass ring is pointing at (in.marks, the same
    // bitmasks `mark_ring` reads below), which is where the MARK layer's
    // sheet reaches into this one.
    let extreme_slots = in.marks.x | in.marks.y;
    // How much of this pixel is a slice a melody or bass ring points at, and
    // how strongly that ring is drawing there: the weight the MARK layer's
    // shimmer reaches the octave glyphs with, below. The slice's own shape,
    // so the sweep fades in exactly with the wedge's edges instead of at a
    // boundary of its own, times the same mark level the ring itself is
    // scaled by -- a released melody's slice stops shimmering as its ring
    // goes, rather than outliving it.
    var mark_slice = 0.0;
    for (var i = 0u; i < oct_span() && (!EARLY_OUT || band > 0.0); i = i + 1u) {
        let slot = oct.base + i32(i);
        let level = oct_slot_level(in.octaves, slot);
        if level <= 0.0 && presence <= 0.0 {
            continue;
        }
        let shape = outer_glyph(slot, oct, in.uv, band, aa);
        // This slot's bit in the mark masks, or none at all where the ring
        // names an octave the packing has no room for. The shift is CLAMPED
        // into the word rather than guarded by the range test alone: `select`
        // evaluates both arms, and a shift past the width is undefined.
        let in_range = slot >= 0 && slot < i32(OCTAVE_SLOTS);
        let bit = select(0u, 1u << u32(clamp(slot, 0, i32(OCTAVE_SLOTS) - 1)), in_range);
        let is_extreme = (extreme_slots & bit) != 0u;
        if is_extreme {
            mark_slice = max(mark_slice, shape * max(
                select(0.0, in.params.y, (in.marks.x & bit) != 0u),
                select(0.0, in.params.z, (in.marks.y & bit) != 0u),
            ));
        }
        // Ghosts carry the ring's shape in the note's own color; a sounding
        // slot never dips below its ghost, so a fading octave hands off to it
        // instead of leaving a hole.
        var cov = shape * GHOST_LEVEL * presence;
        var slot_rgb = node_glyph_rgb;
        if level > 0.0 {
            // Straight off the octave's own envelope, so the glyph eases in
            // over the attack and ends at nothing on release. The max() hands
            // a backdrop slot off to its ghost as the lit coverage sinks
            // through it.
            cov = max(cov, shape * level);
            // Slot s is MIDI octave s - 1, whose C is MIDI 12*s; add this
            // node's pitch class for the glyph's true pitch.
            let pitch = oct_slot_pitch(slot, in.cents);
            // Exactly the color that pitch lights everywhere else. The LUT is
            // the pitch ramp (pitch_ramp_lch in harmonigraph-scene), and the
            // core disc and the piano roll sample that same table — so all
            // three read as one color. A white mix here would be a second
            // definition of what a lit pitch looks like, and it would drift
            // off the disc the moment the gradient's brightness moved.
            slot_rgb = pitch_lut_color(pitch);
        }
        if cov > glyph {
            glyph = cov;
            glyph_rgb = slot_rgb;
        }
    }
    // Ease the glyph layer off across the billboard's margin instead of
    // letting the quad boundary clip it flat. The fade starts at uv 1.0 —
    // the outer band's own limit — so it touches nothing but what reaches
    // past the band: the aa fringe of a band dialed right out to the edge,
    // eased to zero by GLYPH_FADE_LIMIT.
    glyph = glyph * (1.0 - smoothstep(1.0, GLYPH_FADE_LIMIT, d));
    // The octave layer's shimmer, on the whole layer at once rather than per
    // slot: the pattern is a sheet crossing the lattice, so which octave a
    // pixel belongs to has nothing to say about it. After the loop for that
    // reason, and after the margin taper so a peak cannot push the layer
    // back out past the fade the taper just closed.
    let oct_shimmer = shimmer_terms(pulse_octaves_mode(), in.field, field_step, 0.0);
    // The MARK layer's sheet, which the glyphs take too -- over the slices a
    // melody or bass ring points at, and nowhere else. A mark is the ring
    // TOGETHER with the octave it names (the ring is slit at that slice's own
    // boundaries to say so), so light crossing the one crosses the other; a
    // sweep that stopped at the ring's edge would cut the mark in half at the
    // gap. It is taken here rather than with the rings below because this is
    // where the glyph layer is finished, and it is the same terms the rings
    // themselves use -- one sheet, read twice, not two sweeps that could
    // disagree.
    let mark_shimmer = shimmer_terms(pulse_marks_mode(), in.field, field_step, 1.0);
    // Where BOTH layers shimmer, a marked slice is under two sheets running
    // square to each other, and it takes the BRIGHTER of the two: max on the
    // white mix, and max on the coverage as well, that term being 1 under a
    // peak and dipping to the trough between bands. Taking the smaller
    // coverage would let the mark sheet's trough darken a slice under the
    // octave sheet's peak -- the wedge the marks exist to pick out, singled
    // out DIMMER than the plain slices beside it for half of every cycle.
    //
    // Adding them instead would push a crossing past what either sheet does
    // alone -- a bright knot travelling the lattice's diagonals -- and would
    // break the "never above 1" the coverage term owes `paint_reach`. Max
    // keeps that bound for free: neither input is ever above 1, so neither is
    // the larger of them.
    //
    // What the max is taken over is the sheets PRESENT at this fragment, and
    // BOTH absences have to be kept exact rather than left to the max.
    // `shimmer_terms` returns the IDENTITY (0, 1) for a layer that is not
    // shimmering, and the two terms disagree about what identity means: 0 is
    // the smallest white mix, but 1 is the largest coverage. A steady sheet
    // is therefore neutral in one term and DOMINANT in the other, so handing
    // one straight to the max loses whichever sheet is really running:
    //
    //  - an absent MARK sheet wins the coverage term outright, flattening the
    //    octave sweep's own trough over every marked slice with `pulse_marks`
    //    merely Off. `mark_sheet` is the guard -- a weight carrying the mode
    //    as well as the slice, so off the slice, or with the mark layer
    //    steady, this is the octave sweep untouched.
    //  - an absent OCTAVE sheet wins it the same way, reading a steady layer
    //    as a sheet permanently at its peak and pinning the slice's coverage
    //    at 1 for the whole cycle. The slice would then only ever brighten,
    //    while the ring it names -- which takes `mark_shimmer` whole below --
    //    goes on dipping between bands, lighting one mark by two different
    //    lights. `crossed` is the guard: two sheets cross only where there
    //    are two, and where there is one the slice takes it whole.
    let mark_sheet = select(0.0, mark_slice, pulse_marks_mode() != 0u);
    let crossed = select(
        mark_shimmer,
        max(oct_shimmer, mark_shimmer),
        pulse_octaves_mode() != 0u,
    );
    let glyph_shimmer = mix(oct_shimmer, crossed, mark_sheet);
    glyph_rgb = mix(glyph_rgb, vec3<f32>(1.0), glyph_shimmer.x);
    glyph = glyph * glyph_shimmer.y;

    // Melody/bass rings, bracketing the octave band: melody inside, bass
    // outside — the ring's radius echoes where its note sits in the chord.
    // Their own layer, composited over the glyphs — a sector's color is its
    // pitch, which is what the octave layer is FOR, so nothing here
    // repaints one.
    let melody_cov = mark_ring(
        in.marks.x, oct, in.uv,
        inner_out - ring_w, inner_out,
        aa,
    ) * in.params.y;
    let bass_cov = mark_ring(
        in.marks.y, oct, in.uv,
        outer_in, min(outer_in + ring_w, lim),
        aa,
    ) * in.params.z;
    // Disjoint radii, so at most one of the two covers any given pixel.
    var mark = max(melody_cov, bass_cov);
    // The rings' own shimmer (`mark_shimmer`, taken with the glyph layer's
    // above), a quarter turn from the octave layer's — the 90 degrees between
    // the two textures. ONE direction for both rings, not one each: they are
    // concentric and never overlap, so a single sweep crossing both reads as
    // light passing over the node, where two would read as two unrelated
    // animations stacked at different radii.
    let mark_rgb = mix(
        select(in.bass_color.rgb, in.melody_color.rgb, melody_cov > bass_cov),
        vec3<f32>(1.0),
        mark_shimmer.x,
    );
    mark = mark * mark_shimmer.y;
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
    // Scaled by the note's OWN envelope, the same one `presence` paints the
    // node with, so the clearing fades out with the note instead of
    // outliving it. The width stays put while it does: a hole that shrinks
    // as it fades reads as the node retreating, and a hole that holds full
    // strength to the last frame (which is what scaling the width alone
    // did) vanishes with an audible pop.
    var gutter_cov = 0.0;
    if in.gutter > 0.0 {
        gutter_cov = gutter_coverage(d, in.rim, in.gutter, in.soft) * activation;
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

/// The two attachments the offscreen scene pass carries.
///
/// `picture` is everything, and is what the composite puts on screen.
/// `nodes` is the same picture with the node LABELS left out — the bright
/// pass reads it, so a name neither glows nor takes a bite out of the halo
/// of the node it covers. Every draw that is not a label writes the same
/// fragment to both, so the two differ in exactly one thing.
struct SceneOut {
    @location(0) picture: vec4<f32>,
    @location(1) nodes: vec4<f32>,
};

/// The node pipelines. `fs_main` is the single-attachment form, which the
/// parity test's direct-to-egui-pass reference draws through; `fs_main_scene`
/// is the one the offscreen pass uses.
@fragment
fn fs_main(in: VsOut) -> @location(0) vec4<f32> {
    return node_paint(in);
}

@fragment
fn fs_main_scene(in: VsOut) -> SceneOut {
    let paint = node_paint(in);
    return SceneOut(paint, paint);
}

/// What a grid line or chord beam paints; see [`node_paint`] for why the
/// entry points are two.
fn edge_paint(in: EdgeVsOut) -> vec4<f32> {
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

@fragment
fn fs_edge(in: EdgeVsOut) -> @location(0) vec4<f32> {
    return edge_paint(in);
}

@fragment
fn fs_edge_scene(in: EdgeVsOut) -> SceneOut {
    let paint = edge_paint(in);
    return SceneOut(paint, paint);
}
