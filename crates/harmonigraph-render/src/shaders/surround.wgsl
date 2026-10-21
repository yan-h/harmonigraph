// The BACKDROP CUT round a note (#1525, #1528): the arithmetic every producer
// of one shares — the roll's ribbons and the Spiral's dots — and the composite
// that lays the notes' combined cut over the picture (surround_composite.wgsl).
//
// A note's surround is the picture taken away next to it and replaced with the
// note's own hue: whole at its edge (at the cut's full amount), fading out over
// the cut's reach. Over black it is that hue laid on; over bright energy it is
// what keeps the note legible. Where two notes' cuts meet, the stronger lands,
// with its color.
//
// Functions only, with no bindings, so it can be joined to any module: a
// producer's own module puts it after common.wgsl and before its own source,
// and the composite's after nothing.

/// The cut as every module that draws or lays it reads it. One vec4, so it
/// sits at the head of a uniform block with no padding of its own.
struct SurroundCut {
    /// The amount the cut takes at a note's edge (0 when off), its reach past
    /// that edge in points, the fade's bias `k` ([`surround_ramp`]), and
    /// padding.
    cut: vec4<f32>,
};

/// The cut's fade at `x`, the distance past a note's edge as a share of the
/// reach: whole at 0 and nothing from 1 on.
///
/// One minus Schlick's bias, `x / (k (1 - x) + 1)`. A `k` of 0 is a straight
/// line; a positive one holds the cut near the note and drops it steeply at
/// the far end; one between -1 and 0 drops it steeply at the note and leaves a
/// long soft tail.
fn surround_ramp(k: f32, x: f32) -> f32 {
    let t = clamp(x, 0.0, 1.0);
    return 1.0 - t / (k * (1.0 - t) + 1.0);
}

// 0-1 linear from 0-1 sRGB gamma. Lifted from egui's own shader, and used
// for the same reason: on an sRGB-aware target egui hands the hardware
// linear values and lets it encode. Both of this project's shells use a
// plain Unorm surface and take the `_gamma` entry points.
fn linear_from_gamma_rgb(srgb: vec3<f32>) -> vec3<f32> {
    let cutoff = srgb < vec3<f32>(0.04045);
    let lower = srgb / vec3<f32>(12.92);
    let higher = pow((srgb + vec3<f32>(0.055)) / vec3<f32>(1.055), vec3<f32>(2.4));
    return select(higher, lower, cutoff);
}
