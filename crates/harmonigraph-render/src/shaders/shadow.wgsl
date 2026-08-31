// What turns every caster's ink, drawn into a cell of its own (`vs_glyph_cell`
// in text.wgsl), into what that cell HOLDS — which is one of two things.
//
// A BLUR cell holds the ink convolved with a Gaussian of that cell's σ, once
// along x into the atlas's second texture and once along y back into the first.
// A DISTANCE cell holds how far each of its texels stands from the caster's
// nearest ink, in the pane's points, answered by a jump flood: seed every inked
// texel with its own coordinate, pass those coordinates outward in halving
// steps, then resolve the field into a distance. Either way the scene pass
// samples one texel of one cell and spends it (`fs_shadow_box`).
//
// A pass here is a draw over the CELLS rather than one quad over the atlas, and
// that is what lets every cell carry its own σ, its own resolution and its own
// kind: a cell's quad reads them off its instance, a tap that falls outside the
// cell's own rect reads nothing, and a cell of the wrong kind collapses its
// quad off the viewport. So one chain serves every distance cell in the atlas
// at once, no seed crosses into the neighbour packed beside it, and a frame on
// a blur row draws none of the flood at all. The padding a cell is packed with
// (`pack` in shadow.rs) is what puts the whole of its blur — or the whole of
// the reach its curve is windowed to — inside the rect the taps are clamped to.
//
// Why a flood rather than sampling a dilation at N offsets: a dilation sampled
// at N offsets is a binary dilation by a disc wherever the coverage is flat, and
// N discrete taps cannot fill a disc unless they sit closer together than a
// stroke is wide — under which N goes as the square of the reach and the
// shortfall reads as shifted copies of the letters. The flood costs `log2` of
// the reach in passes and answers with a distance, so the shadow at a name is
// the same one-line expression it is at a ring.

@group(0) @binding(0) var src: texture_2d<f32>;
// The layout's sampler slot, which the scene pass's one tap takes and nothing
// here does: every tap in this module is a texel by construction
// (`textureLoad`).
@group(0) @binding(1) var src_sampler: sampler;

// The flood's own group. The three passes read different halves of it — the
// seed takes the atlas alone, the step the field alone, the resolve both — and
// what an entry point does not read is pruned before its pipeline is built.
@group(1) @binding(0) var seeds: texture_2d<u32>;
@group(1) @binding(1) var<uniform> jump: Jump;

/// The jump this step takes, in texels, in `x`. Halves from a power of two at
/// or above the widest distance cell's own reach down to 1 (`steps` in
/// shadow.rs, which is where the sequence is decided).
///
/// A whole `vec4<i32>` for one number, which is what the uniform address space
/// costs: a struct there is laid out on 16-byte alignment, so three explicit pad
/// words and a `vec3` beside the `i32` come to the same 16 bytes with more ways
/// to get the Rust side's size wrong.
struct Jump {
    step: vec4<i32>,
};

/// What `ShadowBox::who.y` holds for a cell that is a DISTANCE rather than
/// blurred ink — `shadow::DISTANCE_KIND`, and spelled again in common.wgsl
/// because there is no linkage between shader modules here.
const DISTANCE_KIND: f32 = 1.0;

struct CellOut {
    @builtin(position) position: vec4<f32>,
    /// The cell's own rect in atlas texels — min, then max, the max exclusive.
    @location(0) @interpolate(flat) bounds: vec4<f32>,
    /// σ in atlas texels. Zero on a distance cell, which is what makes the blur
    /// chain a pass-through over one.
    @location(1) @interpolate(flat) sigma: f32,
    /// How many of this cell's texels one point of the pane spans
    /// (`ShadowBox::terms.x`) — what turns the flood's answer, which is in
    /// texels, into the points the cell is read back in.
    @location(2) @interpolate(flat) k: f32,
    /// How far past the caster's ink this cell reaches, in points
    /// (`ShadowBox::who.z`). The distance resolved where no seed is within
    /// reach: the standoff's curve is windowed to exactly zero there, so a
    /// texel the flood says nothing about and one at the very edge of the reach
    /// carry the same coverage and the bilinear tap between them has no step to
    /// cross.
    @location(3) @interpolate(flat) pad: f32,
};

/// A quad with no area, off the viewport: what a pass emits for a cell of the
/// other kind. `no_quad` in common.wgsl is the same value, spelled there for
/// the draws that FILL a cell.
fn no_quad() -> vec4<f32> {
    return vec4<f32>(2.0, 2.0, 0.0, 1.0);
}

/// One cell's quad, over exactly the texels its packer gave it — or nothing at
/// all where `draws` is false. The attributes are `ShadowBox`'s four rows
/// (shadow.rs).
fn cell_quad(
    vertex: u32,
    cell: vec4<f32>,
    terms: vec4<f32>,
    who: vec4<f32>,
    draws: bool,
) -> CellOut {
    let corner = vec2<f32>(
        select(0.0, 1.0, (vertex & 1u) == 1u),
        select(0.0, 1.0, (vertex & 2u) == 2u),
    );
    // The atlas's size, off the texture at group 0 — which every pass here
    // binds, and which is the same size as the seed pair the flood writes.
    let atlas = vec2<f32>(textureDimensions(src));
    let texel = cell.xy + corner * cell.zw;
    var out: CellOut;
    out.position = vec4<f32>(
        texel.x / atlas.x * 2.0 - 1.0,
        1.0 - texel.y / atlas.y * 2.0,
        0.0,
        1.0,
    );
    if !draws {
        out.position = no_quad();
    }
    out.bounds = vec4<f32>(cell.xy, cell.xy + cell.zw);
    out.sigma = terms.y;
    out.k = terms.x;
    out.pad = who.z;
    return out;
}

/// EVERY cell, whatever it holds — the x blur, whose pass-through over a
/// distance cell is what carries that cell's coverage to the resolve.
@vertex
fn vs_cell(
    @builtin(vertex_index) vertex: u32,
    @location(0) rect: vec4<f32>,
    @location(1) cell: vec4<f32>,
    @location(2) terms: vec4<f32>,
    @location(3) who: vec4<f32>,
) -> CellOut {
    return cell_quad(vertex, cell, terms, who, true);
}

/// The BLUR cells alone: the y pass, which shares its target with the resolve
/// and must not write a texel the resolve is about to.
@vertex
fn vs_cell_blur(
    @builtin(vertex_index) vertex: u32,
    @location(0) rect: vec4<f32>,
    @location(1) cell: vec4<f32>,
    @location(2) terms: vec4<f32>,
    @location(3) who: vec4<f32>,
) -> CellOut {
    return cell_quad(vertex, cell, terms, who, who.y < 0.5);
}

/// The DISTANCE cells alone: the flood's three passes, so a frame whose row is
/// a mixture of kinds floods only the cells that hold one.
@vertex
fn vs_cell_distance(
    @builtin(vertex_index) vertex: u32,
    @location(0) rect: vec4<f32>,
    @location(1) cell: vec4<f32>,
    @location(2) terms: vec4<f32>,
    @location(3) who: vec4<f32>,
) -> CellOut {
    return cell_quad(vertex, cell, terms, who, who.y >= 0.5);
}

/// How many σ out the kernel reaches. Three, where 0.3% of a Gaussian's mass is
/// left: the padding a cell is packed with reaches the same distance
/// (`REACH_SIGMAS` in shadow.rs), so the kernel and the cell agree on where a
/// shadow stops.
const REACH: f32 = 3.0;

/// The widest kernel a cell can ask for, in taps either side of the centre —
/// `ceil(REACH * SIGMA_CELL_MAX)` for shadow.rs's `SIGMA_CELL_MAX`, which the
/// packer holds every cell's σ under. A bound on the LOOP rather than a second
/// copy of the packer's rule: a cell arriving over it draws wrong, not forever.
const MAX_RADIUS: i32 = 9;

/// What every weight is dropped by, so the kernel arrives at [`REACH`] σ on
/// zero: the Gaussian's own value there. Past that distance the drop takes a
/// weight below zero and the clamp holds it, so the kernel's support is exactly
/// ±REACH σ however far `ceil` put the loop's last tap.
///
/// Below σ 0.35 texels every tap but the centre clamps away and the blur is a
/// pass-through. That is a blur a third of a texel wide — narrower than the
/// thing it would be blurring — and it is reachable only at a Shadow the eye
/// cannot see at all, the packer holding σ at the cap for every width past
/// about 0.14 of the bar.
const PEDESTAL: f32 = 0.011109;

/// The Gaussian along `axis` at this fragment, over this cell's texels and no
/// others.
///
/// Normalised by the whole kernel and not by the taps that landed inside the
/// cell: outside the rect the ink IS zero, so a tap there contributes nothing
/// and still counts, and the blur of a half-plane comes out at exactly half at
/// its edge. Normalising over the taps that landed would lift every cell's
/// edge back toward its interior.
///
/// The kernel is LOWERED onto zero at [`REACH`] rather than cut there. A
/// Gaussian truncated at three σ still carries `½erfc(3/√2)` ≈ 1.3e-3 of the
/// light at the cut, and a caster's quad ends on that same distance — so the
/// shadow stopped at a step, which `shadow_transmittance` spends as 5.9/255 at
/// the top of the depth bar, in a straight line along the caster's BOX. Every
/// weight is dropped by [`PEDESTAL`] and clamped at zero, which is the standard
/// treatment for a truncated kernel: same taps, same padding, and the blur of a
/// half-plane reaches exactly zero where the quad ends.
///
/// It costs 4.3% of the effective σ — the pedestal comes out of the kernel's
/// own width — so the Shadow bar reads a hair narrower than it did, and that is
/// the whole of what the goldens' re-baseline pays for. Taken against the
/// constant and not against the loop's own last tap, which is where it wants to
/// be written: `radius` is a CEILING, so a pedestal read off it would step
/// every time `ceil` did and swing the effective σ by ±3% as the bar moves,
/// where this holds it at 0.955..0.963 of σ across the whole range.
fn blur(in: CellOut, axis: vec2<i32>) -> f32 {
    let sigma = max(in.sigma, 1.0e-3);
    let radius = min(i32(ceil(REACH * sigma)), MAX_RADIUS);
    let at = vec2<i32>(in.position.xy);
    var sum = 0.0;
    var weight = 0.0;
    for (var i = -radius; i <= radius; i = i + 1) {
        let w = max(exp(-0.5 * f32(i * i) / (sigma * sigma)) - PEDESTAL, 0.0);
        weight = weight + w;
        let tap = at + axis * i;
        let centre = vec2<f32>(tap) + vec2<f32>(0.5);
        if centre.x < in.bounds.x || centre.y < in.bounds.y
            || centre.x >= in.bounds.z || centre.y >= in.bounds.w {
            continue;
        }
        sum = sum + w * textureLoad(src, tap, 0).r;
    }
    return sum / weight;
}

@fragment
fn fs_blur_x(in: CellOut) -> @location(0) vec4<f32> {
    return vec4<f32>(blur(in, vec2<i32>(1, 0)), 0.0, 0.0, 1.0);
}

@fragment
fn fs_blur_y(in: CellOut) -> @location(0) vec4<f32> {
    return vec4<f32>(blur(in, vec2<i32>(0, 1)), 0.0, 0.0, 1.0);
}

/// A coordinate no seed occupies — what a texel out of reach of every stroke
/// carries in both channels.
///
/// Atlas coordinates take the low fourteen bits, enough for every 2D texture
/// wgpu exposes here. One spare bit in x and two in y carry the contour's
/// undirected normal; x's top bit remains clear on every real seed, so setting
/// it alone is an unambiguous sentinel with no third channel.
const NO_SEED: u32 = 32768u;
const SEED_COORD_MASK: u32 = 16383u;

/// How much of a texel must be inked for it to seed the field.
///
/// The half-coverage contour, which is where a rasterizer puts a shape's edge:
/// a stroke's own texels read near 1 and the texels its antialiased edge falls
/// across read between, so this reconstructs the shape the eye sees rather than
/// the shape plus its fringe. Lower and every glyph and every ring grows a texel
/// of bogus ink all round, which the shadow would then stand off from.
const INK_FLOOR: f32 = 0.5;

/// How far one seed's local contour may reach along its tangent, in texels.
///
/// Two spans one missing seed between covered texels. Keeping it at that
/// immediate neighbourhood stops a tangent at a corner standing in for a
/// distant edge.
const CONTOUR_TANGENT_REACH: f32 = 2.0;

/// Coverage at `at`, or no ink for a tap outside this cell.
///
/// The atlas packs unrelated casters beside each other, so the gradient used
/// to place a contour seed must not read the neighbour as part of this one.
fn cell_coverage(in: CellOut, at: vec2<i32>) -> f32 {
    let centre = vec2<f32>(at) + vec2<f32>(0.5);
    if centre.x < in.bounds.x || centre.y < in.bounds.y
        || centre.x >= in.bounds.z || centre.y >= in.bounds.w {
        return 0.0;
    }
    return clamp(textureLoad(src, at, 0).r, 0.0, 1.0);
}

/// A seed coordinate with the coverage gradient's line direction in its spare
/// high bits. Eight directions over half a turn keep the normal within 11.25°
/// of the rasterizer's while leaving x's sentinel bit untouched.
fn pack_seed(at: vec2<i32>, gradient: vec2<f32>) -> vec2<u32> {
    let ax = abs(gradient.x);
    let ay = abs(gradient.y);
    // Tangents of 11.25°, 33.75°, 56.25° and 78.75° split a quadrant around
    // its five candidate directions. Comparisons keep this pass off `atan2`,
    // whose cost would be paid at every texel of the distance atlas.
    var octant = 0u;
    if ay >= 0.19891237 * ax {
        octant = 1u;
    }
    if ay >= 0.66817864 * ax {
        octant = 2u;
    }
    if ay >= 1.49660576 * ax {
        octant = 3u;
    }
    if ay >= 5.02733949 * ax {
        octant = 4u;
    }
    let direction = select(octant & 7u, (8u - octant) & 7u, gradient.x * gradient.y < 0.0);
    let coord = vec2<u32>(at);
    return vec2<u32>(
        coord.x | ((direction & 1u) << 14u),
        coord.y | (((direction >> 1u) & 3u) << 14u),
    );
}

/// The absolute atlas texel a packed seed names.
fn seed_texel(seed: vec2<u32>) -> vec2<i32> {
    return vec2<i32>(seed & vec2<u32>(SEED_COORD_MASK));
}

/// The undirected normal a packed seed carries.
fn seed_normal(seed: vec2<u32>) -> vec2<f32> {
    let direction = ((seed.x >> 14u) & 1u) | (((seed.y >> 14u) & 3u) << 1u);
    switch direction {
        case 0u: { return vec2<f32>(1.0, 0.0); }
        case 1u: { return vec2<f32>(0.92387953, 0.38268343); }
        case 2u: { return vec2<f32>(0.70710678, 0.70710678); }
        case 3u: { return vec2<f32>(0.38268343, 0.92387953); }
        case 4u: { return vec2<f32>(0.0, 1.0); }
        case 5u: { return vec2<f32>(-0.38268343, 0.92387953); }
        case 6u: { return vec2<f32>(-0.70710678, 0.70710678); }
        default: { return vec2<f32>(-0.92387953, 0.38268343); }
    }
}

/// The chain's first pass: every inked texel of a distance cell becomes its own
/// seed, and every other texel becomes [`NO_SEED`].
///
/// Its own entry point rather than a step over an initialised target, because
/// what it reads is the coverage (one float channel) and what every pass after
/// it reads is a field of coordinates (two uint channels).
@fragment
fn fs_flood_seed(in: CellOut) -> @location(0) vec2<u32> {
    let at = vec2<i32>(in.position.xy);
    let coverage = cell_coverage(in, at);
    if coverage < INK_FLOOR {
        return vec2<u32>(NO_SEED, NO_SEED);
    }
    // Only inked texels need a normal. The padding is most of a wide distance
    // cell, so four local reads here cost less than evaluating the direction
    // over every empty texel in the pass.
    let gradient = 0.5 * vec2<f32>(
        cell_coverage(in, at + vec2<i32>(1, 0))
            - cell_coverage(in, at - vec2<i32>(1, 0)),
        cell_coverage(in, at + vec2<i32>(0, 1))
            - cell_coverage(in, at - vec2<i32>(0, 1)),
    );
    return pack_seed(at, gradient);
}

/// One flood step: keep the nearest of this texel's own seed and the nine
/// candidates a jump of [`Jump::step`] reaches.
///
/// Nine and not eight, the centre being one of them: a texel that already holds
/// a seed has to defend it against the ring, or a step landing on a nearer
/// stroke's territory would overwrite a closer answer with a further one.
///
/// The taps are clamped to this CELL's own rect exactly as [`blur`]'s are. A
/// candidate outside it belongs to whichever caster was packed beside this one,
/// and a seed that crossed would draw the neighbour's letters inside this
/// caster's shadow.
///
/// Squared distances throughout — the comparison is all this makes, and a square
/// root is monotone, so the ordering is the same and one per candidate is saved.
/// The `f32` they are computed in holds an atlas's squared diagonal exactly: at
/// 8192 across it is under 2^27, well inside the 24 bits an f32 mantissa carries
/// whole, so two candidates never tie by rounding.
@fragment
fn fs_flood_step(in: CellOut) -> @location(0) vec2<u32> {
    let at = vec2<i32>(in.position.xy);
    var best = vec2<u32>(NO_SEED, NO_SEED);
    var best_d = 3.4e38;
    for (var dy = -1; dy <= 1; dy = dy + 1) {
        for (var dx = -1; dx <= 1; dx = dx + 1) {
            let tap = at + vec2<i32>(dx, dy) * jump.step.x;
            let centre = vec2<f32>(tap) + vec2<f32>(0.5);
            if centre.x < in.bounds.x || centre.y < in.bounds.y
                || centre.x >= in.bounds.z || centre.y >= in.bounds.w {
                continue;
            }
            let cand = textureLoad(seeds, tap, 0).xy;
            if cand.x == NO_SEED {
                continue;
            }
            let d = vec2<f32>(at - seed_texel(cand));
            let dist = dot(d, d);
            if dist < best_d {
                best_d = dist;
                best = cand;
            }
        }
    }
    return best;
}

/// The field turned into what the cell holds: the distance from this texel to
/// the caster's nearest ink, in the pane's POINTS.
///
/// Points and not texels because a cell's resolution is its own (`pack`) and
/// what reads it back is a fragment of the pane: one division here, at the one
/// site that knows both, against a scale the sampler would otherwise have to
/// carry per term.
///
/// One seed represents the short, straight contour segment crossing its texel.
/// The coverage gradient supplies the segment's normal, and the covered share
/// past [`INK_FLOOR`] moves it from the texel centre onto the rasterizer's
/// contour. The bounded tangent bridges a one-texel diagonal gap between seed
/// texels and stays inside the seed's immediate neighbourhood, so a tangent at
/// a corner does not claim a distant edge.
///
/// A single contour POINT is not enough. Its position remains quantized along
/// the tangent, so a diagonal edge is still up to half a texel away from the
/// closest point it actually contains. The segment supplies that missing
/// degree of freedom without making the seed texture or any flood pass larger.
///
/// Clamped to the cell's own pad, which is where the curve is windowed to zero:
/// past it the coverage is 0 whatever the number, so this is the value that
/// makes "no seed within reach" and "further than the shadow goes" one answer.
@fragment
fn fs_flood_resolve(in: CellOut) -> @location(0) vec4<f32> {
    let at = vec2<i32>(in.position.xy);
    let seed = textureLoad(seeds, at, 0).xy;
    if seed.x == NO_SEED {
        return vec4<f32>(in.pad, 0.0, 0.0, 1.0);
    }
    let coverage = cell_coverage(in, at);
    if coverage >= INK_FLOOR {
        return vec4<f32>(0.0, 0.0, 0.0, 1.0);
    }
    let ink = seed_texel(seed);
    let seed_coverage = cell_coverage(in, ink);
    var normal = seed_normal(seed);
    let to_sample = vec2<f32>(at - ink);
    // The packed normal is a LINE direction. The sample is outside the ink,
    // so whichever sign points toward it is the contour's outward one.
    if dot(to_sample, normal) < 0.0 {
        normal = -normal;
    }
    let offset = (seed_coverage - INK_FLOOR) * normal;
    let contour = vec2<f32>(ink) + offset;
    let tangent = vec2<f32>(-normal.y, normal.x);
    let delta = vec2<f32>(at) - contour;
    let along = clamp(dot(delta, tangent), -CONTOUR_TANGENT_REACH, CONTOUR_TANGENT_REACH);
    let texels = length(delta - along * tangent);
    return vec4<f32>(min(texels / max(in.k, 1.0e-6), in.pad), 0.0, 0.0, 1.0);
}
