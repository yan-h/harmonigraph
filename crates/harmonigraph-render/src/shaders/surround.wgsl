// The BACKDROP CUT round a note (#1525, #1528): the arithmetic every producer
// of one shares — the roll's ribbons, laid over the picture by their composite
// (surround_composite.wgsl), and the Spiral's dots, which land straight on it
// (dot_surround.wgsl).
//
// A note's surround is the picture taken away next to it and replaced with the
// note's own hue: whole at its edge (at the cut's full amount), fading out over
// the cut's reach, with the hue fading sooner than the cut does. Over black it
// is that hue laid on; over bright energy it is what keeps the note legible.
// Where two notes' cuts meet, the stronger lands, with its color.
//
// Functions only, with no bindings, so it can be joined to any module: each
// module that uses it puts it before its own source, and no other half.

/// The cut as every producer that draws it reads it. One vec4, so it
/// sits at the head of a uniform block with no padding of its own.
struct SurroundCut {
    /// The amount the cut takes at a note's edge (0 when off), its reach past
    /// that edge in points, the fade's bias `k` ([`surround_ramp`]), and how
    /// much sooner the fill's tint fades ([`surround_tinted`]).
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

/// [`surround_ramp`] of `surround`'s own bias, `d` points past a note's edge. A
/// note's cut and its tint both read this, so they cannot part ways.
fn surround_fade_at(surround: SurroundCut, d: f32) -> f32 {
    return surround_ramp(surround.cut.z, max(d, 0.0) / surround.cut.y);
}

/// The cut's reach past a note's edge, in points. 0 while the cut is off.
fn surround_reach(surround: SurroundCut) -> f32 {
    return select(0.0, surround.cut.y, surround.cut.x > 0.0);
}

/// Coverage of everything on the near side of `edge`: how much of a window
/// `feather` points wide (one pixel of whatever is drawn into), centered on
/// the signed distance `d`, lands inside it.
///
/// A box filter along the distance gradient, which is exact for a straight
/// edge and is what makes a shape thinner than a pixel come out FAINTER rather
/// than snapping to a full pixel — the same bargain epaint's feathering makes,
/// so a hairline ribbon reads the way it does through the tessellator. The
/// note's own edge reads it at an `edge` of 0, and the cut's far end at its
/// reach.
fn surround_inside(d: f32, edge: f32, feather: f32) -> f32 {
    let f = max(feather, 1e-6);
    return clamp((edge - d) / f + 0.5, 0.0, 1.0);
}

/// How much of the picture the cut takes `d` points off a note's edge, as a
/// share of its amount: [`surround_ramp`] over the cut's reach, and nothing
/// from the reach's antialiasing on, which ends half a feather past it.
///
/// Read off the distance rather than a blur of the note's silhouette, which is
/// the point of it: a blur of a ribbon thinner than its σ never reaches full
/// strength, so the surround all but vanished round exactly the hairline notes
/// that need one, and spread into a wide smear round a fat one. Here a note's
/// edge is equally held at every width. The amount itself is laid on by each
/// producer, one amount for every note, so it changes no note's rank against
/// another's.
fn surround_cut_coverage(surround: SurroundCut, d: f32, feather: f32) -> f32 {
    let reach = surround_reach(surround);
    if reach <= 0.0 {
        return 0.0;
    }
    return surround_fade_at(surround, d) * surround_inside(d, reach, feather);
}

/// A note's fill where [`surround_fade_at`] reads `fade`, above 0: `sooner` more
/// powers of it off `color`, so the tint draws in toward the note while the cut
/// keeps its reach. Neighbouring fills then stop merging into a sheet of their
/// own, and what is left of the cut further out is plain dark.
///
/// Distance alone, and never the cut's whole strength: a note's opacity and the
/// antialiasing round its body scale the cut once already, and a power of them
/// would leave a faint note a dark bite with no hue and a rim darker than
/// either side of it.
fn surround_tinted(color: vec3<f32>, fade: f32, sooner: f32) -> vec3<f32> {
    return color * pow(fade, sooner);
}

/// How a note's cut lands on the picture: premultiplied over, so what is under
/// keeps `1 - cut` of itself and gains `color`, the note's straight gamma-space
/// fill, as much as the cut took. `cut` carries the amount.
fn surround_lands_gamma(color: vec3<f32>, cut: f32) -> vec4<f32> {
    return vec4<f32>(color * cut, cut);
}

/// The same onto a target that encodes sRGB for itself, which wants linear
/// color, as the roll's `fs_core_linear` does for the bodies.
fn surround_lands_linear(color: vec3<f32>, cut: f32) -> vec4<f32> {
    return vec4<f32>(linear_from_gamma_rgb(color) * cut, cut);
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
