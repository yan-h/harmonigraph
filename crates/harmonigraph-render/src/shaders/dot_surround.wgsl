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

/// The cut's reach in points, 0 while it is off.
fn cut_reach() -> f32 {
    return select(0.0, dot_locals.surround.cut.y, dot_locals.surround.cut.x > 0.0);
}

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
    let local = corner * (at.radius + cut_reach() + dot_locals.feather);
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
/// the cut's amount: [`surround_ramp`] of the distance past the disc's edge,
/// nothing from the reach's antialiasing on, which ends half a feather inside
/// the dot's quad, and held out by the dot's own disc, so a
/// translucent dot shows the picture behind it uncut, as a roll ribbon does.
///
/// By its OWN disc only: unlike the roll, the dots draw no holdout of every
/// body, so a neighbour's cut still darkens the picture behind a translucent
/// dot standing inside its reach — two notes a semitone apart on an inner turn,
/// say. That dot then reads a little darker over the strip than it is.
fn dot_cut(at: Dot, p: vec2<f32>) -> f32 {
    let reach = cut_reach();
    if reach <= 0.0 {
        return 0.0;
    }
    let f = max(dot_locals.feather, 1e-6);
    let d = length(p - at.center) - at.radius;
    let fill = clamp(0.5 - d / f, 0.0, 1.0);
    let end = clamp((reach - d) / f + 0.5, 0.0, 1.0);
    let fade = surround_ramp(dot_locals.surround.cut.z, max(d, 0.0) / reach);
    return unpack4x8unorm(at.aura).a * fade * end * (1.0 - fill);
}

/// The cut `in`'s dot lands here, its amount included, when it is the
/// strongest of its candidates', ties to the lowest index; otherwise the
/// fragment is discarded.
fn landed(in: DotOut) -> f32 {
    let p = in.position.xy * dot_locals.feather;
    let mine = dots[in.dot];
    let grow = cut_reach() + max(dot_locals.feather, 1e-6);
    var strongest = 0.0;
    var winner = in.dot;
    for (var n = 0u; n < mine.candidates; n++) {
        let i = candidates[mine.first + n];
        let at = dots[i];
        // Outside a dot's own quad it cuts nothing here.
        if any(abs(p - at.center) > vec2<f32>(at.radius + grow)) {
            continue;
        }
        let cut = dot_cut(at, p);
        if cut > strongest {
            strongest = cut;
            winner = i;
        }
    }
    if strongest <= 0.0 || winner != in.dot {
        discard;
    }
    return dot_locals.surround.cut.x * strongest;
}

@fragment
fn fs_dot_surround_gamma(in: DotOut) -> @location(0) vec4<f32> {
    let cut = landed(in);
    return surround_lands_gamma(unpack4x8unorm(dots[in.dot].aura).rgb, cut);
}

/// The same onto a target that encodes sRGB for itself.
@fragment
fn fs_dot_surround_linear(in: DotOut) -> @location(0) vec4<f32> {
    let cut = landed(in);
    return surround_lands_linear(unpack4x8unorm(dots[in.dot].aura).rgb, cut);
}
