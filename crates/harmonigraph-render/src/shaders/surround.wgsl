// The BACKDROP CUT round a note (#1525, #1528): the arithmetic every producer
// of one shares — the roll's ribbons and the Spiral's dots — and the composite
// that lays the notes' combined cut over the picture (surround_composite.wgsl).
//
// A note's surround is the picture taken away next to it: whole at its edge
// (at the cut's full amount), fading out over the cut's reach, and, as far as
// the fill says, replaced with the note's own hue rather than left black. Over
// black it draws nothing but that fill, since it takes away only light that was
// there; over bright energy it is what keeps the note legible.
//
// Functions only, with no bindings, so it can be joined to any module: a
// producer's own module puts it after common.wgsl and before its own source,
// and the composite's after nothing.

/// The cut as every module that draws or lays it reads it. Two vec4s, so it
/// sits at the head of a uniform block with no padding of its own.
struct SurroundCut {
    /// The amount the cut takes at a note's edge (0 when off), its reach past
    /// that edge in points, the fade's bias `k` ([`surround_ramp`]), and the
    /// halo blend's softness κ as a share of the reach, 0 for the strongest
    /// note's cut alone ([`surround_share`]).
    cut: vec4<f32>,
    /// How much of what the cut takes is filled with the note's own hue, 0..1,
    /// then padding.
    fill: vec4<f32>,
};

/// The cut's fade at `x`, the distance past a note's edge as a share of the
/// reach: whole at 0 and nothing from 1 on.
///
/// One minus Schlick's bias, `x / (k (1 - x) + 1)`. A `k` of 0 is a straight
/// line; a positive one holds the cut near the note and drops it steeply at
/// the far end; one between -1 and 0 drops it steeply at the note and leaves a
/// long soft tail. The family is closed under inversion ([`surround_unramp`]),
/// which is what lets a note's cut join the halo blend as a distance.
fn surround_ramp(k: f32, x: f32) -> f32 {
    let t = clamp(x, 0.0, 1.0);
    return 1.0 - t / (k * (1.0 - t) + 1.0);
}

/// The `x` at which [`surround_ramp`] reads `y`: how far out a bare note's
/// cut is as strong as `y`. Exact, in closed form, at every bias.
fn surround_unramp(k: f32, y: f32) -> f32 {
    let g = clamp(1.0 - y, 0.0, 1.0);
    return g * (k + 1.0) / (1.0 + g * k);
}

/// What a note whose cut here is `c` (a share of the amount, 0..1) adds to the
/// cut target, which is summed under an additive blend or kept under a max.
///
/// With no halo blend, `c` itself, and the target keeps the strongest note's.
/// Where two notes' cuts meet that minimum folds along the line between them, a
/// crease a wide, soft cut makes plain; the halo blend is a smooth minimum of
/// the notes' distances, `-κ ln Σ exp(-x / κ)`, which has no fold and darkens a
/// narrow gap a little more. Each share is lowered by its own value at the
/// reach, so a note's share reaches 0 exactly where its cut does: the blend of
/// one note is that note's cut, and the blend never reaches past the notes'
/// own reach. [`surround_strength`] puts that value back.
///
/// Summed in f32, which holds `exp(-1 / κ)` in normal range at every κ the
/// renderer hands over; below that it hands over 0, the strongest-wins cut,
/// which differs from the smooth minimum there by at most κ ln 2 of the reach.
fn surround_share(s: SurroundCut, c: f32) -> f32 {
    let kappa = s.cut.w;
    if kappa <= 0.0 {
        return c;
    }
    let x = surround_unramp(s.cut.z, c);
    return max(exp(-x / kappa) - exp(-1.0 / kappa), 0.0);
}

/// How much of the picture the notes' cuts take together, from what
/// [`surround_share`] left in the cut target: the amount, at the combined
/// distance's fade.
fn surround_strength(s: SurroundCut, stored: f32) -> f32 {
    let kappa = s.cut.w;
    if kappa <= 0.0 {
        return s.cut.x * clamp(stored, 0.0, 1.0);
    }
    let x = -kappa * log(max(stored, 0.0) + exp(-1.0 / kappa));
    return s.cut.x * surround_ramp(s.cut.z, x);
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
