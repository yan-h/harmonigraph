// The spiral's sounding-note dots as producers of the notes' backdrop cut
// (surround.wgsl): each dot's cut read off the circle's distance, drawn into
// targets over the spiral's rect and laid over the picture by the surround's
// composite, before the dots themselves are painted over it.

struct DotLocals {
    /// The backdrop cut, the roll's own. First, so its vec4 is aligned with no
    /// padding.
    surround: SurroundCut,
    /// Where the rect drawn into starts, in egui points, and how big it is.
    origin_points: vec2<f32>,
    viewport_points: vec2<f32>,
    /// Width of the antialiasing ramp in points: one device pixel.
    feather: f32,
    _feather_pad: f32,
    /// Rounds the block up to its vec4's alignment, spelled out to match the
    /// Rust side.
    _tail_pad: vec2<f32>,
};

@group(0) @binding(0) var<uniform> dot_locals: DotLocals;

struct DotOut {
    @builtin(position) position: vec4<f32>,
    /// Offset from the dot's centre, in points.
    @location(0) local: vec2<f32>,
    @location(1) @interpolate(flat) radius: f32,
    /// The fill color, straight gamma-space RGB, and in `a` how strongly the
    /// dot cuts: its own opacity, so a fading dot leaves no solid cut behind.
    @location(2) @interpolate(flat) aura: vec4<f32>,
};

/// One corner of a dot's quad: its disc grown by the cut's reach and half a
/// feather past that.
@vertex
fn vs_dot_cut(
    @builtin(vertex_index) vertex: u32,
    @location(0) center: vec2<f32>,
    @location(1) radius: f32,
    @location(2) aura: vec4<f32>,
) -> DotOut {
    let corner = vec2<f32>(
        select(-1.0, 1.0, (vertex & 1u) == 1u),
        select(-1.0, 1.0, (vertex & 2u) == 2u),
    );
    let reach = select(0.0, dot_locals.surround.cut.y, dot_locals.surround.cut.x > 0.0);
    let local = corner * (radius + reach + dot_locals.feather);
    let in_viewport = center + local - dot_locals.origin_points;
    var out: DotOut;
    out.position = vec4<f32>(
        2.0 * in_viewport.x / dot_locals.viewport_points.x - 1.0,
        1.0 - 2.0 * in_viewport.y / dot_locals.viewport_points.y,
        0.0,
        1.0,
    );
    out.local = local;
    out.radius = radius;
    out.aura = aura;
    return out;
}

/// How much of the picture this dot's cut takes here, as a share of the cut's
/// amount: [`surround_ramp`] of the distance past the disc's edge, nothing from
/// the reach on, and held out by the dot's own disc, so a translucent dot shows
/// the picture behind it uncut, as a roll ribbon does.
///
/// By its OWN disc only: unlike the roll, the dots draw no holdout of every
/// body, so a neighbour's cut still darkens the picture behind a translucent
/// dot standing inside its reach — two notes a semitone apart on an inner turn,
/// say. That dot then reads a little darker over the strip than it is. A
/// holdout pass, as the roll's, is what it would take, and nothing has asked
/// for one yet.
fn dot_cut(in: DotOut) -> f32 {
    let reach = dot_locals.surround.cut.y;
    if reach <= 0.0 {
        return 0.0;
    }
    let d = length(in.local) - in.radius;
    let f = max(dot_locals.feather, 1e-6);
    let fill = clamp(0.5 - d / f, 0.0, 1.0);
    let end = clamp((reach - d) / f + 0.5, 0.0, 1.0);
    let fade = surround_ramp(dot_locals.surround.cut.z, max(d, 0.0) / reach);
    return in.aura.a * fade * end * (1.0 - fill);
}

struct SurroundOut {
    @location(0) color: vec4<f32>,
    @builtin(frag_depth) depth: f32,
};

/// This dot's share of the surround: its fill color with its cut in alpha,
/// kept where its cut is the strongest of any dot's.
///
/// The cut is written as DEPTH, under a `Less` test against a target cleared
/// to 1, so the strongest dot's fragment is the one that lands and its color
/// lands with it. A fragment of no cut leaves the target clear.
@fragment
fn fs_dot_surround(in: DotOut) -> SurroundOut {
    let cut = dot_locals.surround.cut.x * dot_cut(in);
    if cut <= 0.0 {
        discard;
    }
    return SurroundOut(vec4<f32>(in.aura.rgb, cut), 1.0 - cut);
}
