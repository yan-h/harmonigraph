// The spiral's sounding-note dots as producers of the notes' backdrop cut
// (surround.wgsl), drawn straight over the picture before the dots themselves
// are painted over it: one quad per dot, its disc grown by the cut's reach.
//
// Where two dots' cuts meet, only the stronger lands, with its color. A dozen
// dots want no targets of their own for that: every fragment reads the cut of
// each of its dot's CANDIDATES off the pixel's own position, and the quad
// whose dot is the strongest there is the one that writes, ties to the lowest
// index. A dot's candidates are itself and every dot whose quad overlaps its
// own, in index order (`prepare` lists them); any other cuts nothing anywhere
// in this quad, so cannot win in it. Every fragment of a pixel then runs the
// same loop over the same cuts, through one call, so they agree on the winner
// to the bit and exactly one quad lands there. Read off an interpolated offset
// instead, or a dot's own cut computed apart from the loop, two fragments
// could disagree in the last bit, and a pixel would take both or neither.

struct DotLocals {
    /// The backdrop cut, the roll's own. First, so its vec4 is aligned with no
    /// padding.
    surround: SurroundCut,
    /// Where the rect drawn into starts, in egui points, and how big it is.
    origin_points: vec2<f32>,
    viewport_points: vec2<f32>,
    /// Width of the antialiasing ramp in points: one device pixel, so also
    /// what turns a framebuffer position into points.
    feather: f32,
    _feather_pad: f32,
    /// Rounds the block up to its vec4's alignment, spelled out to match the
    /// Rust side.
    _tail_pad: vec2<f32>,
};

/// One sounding note's dot: `GpuDot`, a `SurroundDot` with its candidates.
/// The fill color is straight gamma-space bytes, red lowest, with in the top
/// byte how strongly the dot cuts: its own opacity, so a fading dot leaves no
/// solid cut behind.
struct Dot {
    /// Centre, in screen points.
    center: vec2<f32>,
    radius: f32,
    aura: u32,
    /// This dot's run of `candidates`.
    first: u32,
    candidates: u32,
};

@group(0) @binding(0) var<uniform> dot_locals: DotLocals;
@group(0) @binding(1) var<storage, read> dots: array<Dot>;
@group(0) @binding(2) var<storage, read> candidates: array<u32>;

struct DotOut {
    @builtin(position) position: vec4<f32>,
    @location(0) @interpolate(flat) dot: u32,
};

/// One corner of a dot's quad: its disc grown by the cut's reach and a feather
/// past that, which takes in the reach's antialiasing.
@vertex
fn vs_dot_cut(
    @builtin(vertex_index) vertex: u32,
    @builtin(instance_index) dot: u32,
) -> DotOut {
    let corner = vec2<f32>(
        select(-1.0, 1.0, (vertex & 1u) == 1u),
        select(-1.0, 1.0, (vertex & 2u) == 2u),
    );
    let at = dots[dot];
    let local = corner * (at.radius + surround_reach(dot_locals.surround) + dot_locals.feather);
    let in_viewport = at.center + local - dot_locals.origin_points;
    var out: DotOut;
    out.position = vec4<f32>(
        2.0 * in_viewport.x / dot_locals.viewport_points.x - 1.0,
        1.0 - 2.0 * in_viewport.y / dot_locals.viewport_points.y,
        0.0,
        1.0,
    );
    out.dot = dot;
    return out;
}

/// How much of the picture `at`'s cut takes at `p` (in points), as a share of
/// the cut's amount, before any disc holds it out: [`surround_cut_coverage`] of
/// the distance past the disc's edge, whose reach's antialiasing ends half a
/// feather inside the dot's quad, at the dot's own strength.
fn dot_cut(at: Dot, p: vec2<f32>) -> f32 {
    let d = length(p - at.center) - at.radius;
    let strength = unpack4x8unorm(at.aura).a;
    return surround_cut_coverage(dot_locals.surround, strength, d, dot_locals.feather);
}

/// How much of `p` `at`'s disc covers: its coverage, not its opacity, so a
/// translucent dot holds a cut out as fully as an opaque one.
fn dot_fill(at: Dot, p: vec2<f32>) -> f32 {
    return surround_inside(length(p - at.center) - at.radius, 0.0, dot_locals.feather);
}

/// The cut `in`'s dot lands here, its amount included, when it is the
/// strongest of its candidates', ties to the lowest index; otherwise the
/// fragment is discarded.
///
/// Held out by every dot's disc, its own included, as the roll's ribbons hold
/// out every cut under every body (#1293): a translucent dot shows the picture
/// behind it uncut, a neighbour's cut included. The union of the discs'
/// coverage is the roll's own holdout, each disc leaving `1 - fill` of what the
/// ones before it left uncovered. Every disc that covers any of this pixel
/// stands within its own quad of it, so is among the candidates, whose loop
/// reads it here.
fn landed(in: DotOut) -> f32 {
    let p = in.position.xy * dot_locals.feather;
    let mine = dots[in.dot];
    let f = max(dot_locals.feather, 1e-6);
    // A fragment whose own dot cannot cut here cannot win, so it skips the
    // loop: inside its own disc, which holds its cut out, and past the reach's
    // antialiasing. A hundredth of a feather inside where `dot_cut` reads 0,
    // so only where the loop's own reading is 0 too.
    let own = length(p - mine.center) - mine.radius;
    if own <= -0.51 * f || own >= surround_reach(dot_locals.surround) + 0.51 * f {
        discard;
    }
    let grow = surround_reach(dot_locals.surround) + f;
    var strongest = 0.0;
    var winner = in.dot;
    var uncovered = 1.0;
    for (var n = 0u; n < mine.candidates; n++) {
        let i = candidates[mine.first + n];
        let at = dots[i];
        // Outside a dot's own quad it cuts and covers nothing here.
        if any(abs(p - at.center) > vec2<f32>(at.radius + grow)) {
            continue;
        }
        uncovered *= 1.0 - dot_fill(at, p);
        let cut = dot_cut(at, p);
        if cut > strongest {
            strongest = cut;
            winner = i;
        }
    }
    let landing = strongest * uncovered;
    if landing <= 0.0 || winner != in.dot {
        discard;
    }
    return dot_locals.surround.cut.x * landing;
}

/// `in`'s fill where it landed: only a dot that cuts here lands, so the fade
/// is above 0.
fn dot_tint(in: DotOut) -> vec3<f32> {
    let at = dots[in.dot];
    let d = length(in.position.xy * dot_locals.feather - at.center) - at.radius;
    let fade = surround_fade_at(dot_locals.surround, d);
    return surround_tinted(unpack4x8unorm(at.aura).rgb, fade, dot_locals.surround.cut.w);
}

@fragment
fn fs_dot_surround_gamma(in: DotOut) -> @location(0) vec4<f32> {
    let cut = landed(in);
    return surround_lands_gamma(dot_tint(in), cut);
}

/// The same onto a target that encodes sRGB for itself.
@fragment
fn fs_dot_surround_linear(in: DotOut) -> @location(0) vec4<f32> {
    let cut = landed(in);
    return surround_lands_linear(dot_tint(in), cut);
}
