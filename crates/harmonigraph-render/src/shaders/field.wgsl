// The distance a name's Shadow is cast from: how far every pixel of a lattice
// pane stands from the nearest ink of any note name on it.
//
// A node's rings and a marker's arms are shapes with a distance FUNCTION, so
// their standoff is one evaluation of the Shadow's curve. A glyph is a bitmap
// and has none, and the only answers available to a fragment that must have one
// are to go and look, or to be told. Looking is what this replaces: a dilation
// sampled at N offsets is a binary dilation by a disc wherever the Shadow's
// coverage is flat, and N discrete taps cannot fill a disc unless they sit
// closer together than a stroke is wide — under which the count goes as the
// square of the width and the shortfall reads as shifted copies of the letters.
//
// Being TOLD is a jump flood: seed every inked pixel with its own coordinate,
// then pass the coordinates outward in halving steps until every pixel holds
// the nearest seed it can reach. The reach costs passes logarithmically rather
// than taps quadratically, and the answer is a distance, so the Shadow becomes
// the same one-line expression at a name that it already is at a ring.
//
// SCREEN SPACE, per frame, rather than a field baked into the glyph sheets.
// A baked field reaches only as far as the padding rasterized around each
// glyph, and the Shadow scales with a node's radius up to `GLOW_SHADOW_MAX`, so
// every glyph in every sheet would carry the widest setting's padding whether
// or not the view asks for it — and epaint's atlas is dynamic and repacked, so
// the field would have to be mirrored and re-derived on every repack. Here the
// cost follows the pane, and nothing is mirrored.

/// A coordinate that no seed occupies, in both channels — what a pixel out of
/// reach of every letter carries.
///
/// The field's texels are `Rg16Uint`, so this is the format's own top value and
/// no pane can address it: a lattice pane 65535 pixels across is past every
/// device limit wgpu reports. That is what lets one sentinel stand for "no ink
/// within reach" without a third channel to say so.
const NO_SEED: u32 = 65535u;

/// How much of a texel must be inked for it to seed the field.
///
/// The half-coverage contour, which is where a rasterizer puts a shape's edge:
/// a stroke's own texels read near 1 and the texels its antialiased edge falls
/// across read between, so this reconstructs the letter the eye sees rather
/// than the letter plus its fringe. Lower and every glyph grows a texel of
/// bogus ink all round, which the Shadow would then stand off from.
const INK_FLOOR: f32 = 0.5;

/// The jump this pass takes, in pixels, in `x`. Halves from a power of two at
/// or above the Shadow's own reach down to 1 (`steps` in field.rs, which is
/// where the sequence is decided).
///
/// A whole `vec4<i32>` for one number, which is what the uniform address space
/// costs: a struct there is laid out on 16-byte alignment, so three explicit
/// pad words and a `vec3` beside the `i32` come to the same 16 bytes with more
/// ways to get the Rust side's size wrong.
struct Jump {
    step: vec4<i32>,
};

// One group for both passes, though each reads about half of it: a bind group
// layout is per pipeline and these two differ only in which texture they take,
// so one layout is one bind group per ping-pong direction instead of two of
// each. What an entry point does not read is pruned before the pipeline is
// built, so the seed pass carries `src` at no cost.
@group(0) @binding(0) var src: texture_2d<u32>;
@group(0) @binding(1) var<uniform> jump: Jump;
/// The ink the chain is seeded from: coverage in `r`, the name's own strength
/// in `g` (`fs_glyph_ink` in text.wgsl writes both).
@group(0) @binding(2) var ink: texture_2d<f32>;

/// The whole pane as one quad, in the four-vertex strip every full-screen pass
/// in this tree draws through.
@vertex
fn vs_field(@builtin(vertex_index) vertex: u32) -> @builtin(position) vec4<f32> {
    let corner = vec2<f32>(
        select(0.0, 1.0, (vertex & 1u) == 1u),
        select(0.0, 1.0, (vertex & 2u) == 2u),
    );
    return vec4<f32>(corner.x * 2.0 - 1.0, 1.0 - corner.y * 2.0, 0.0, 1.0);
}

/// The chain's first pass: every inked pixel becomes its own seed and every
/// other pixel becomes [`NO_SEED`].
///
/// Its own entry point rather than a step over an initialised target, because
/// what it reads is the ink (one float channel) and what every pass after it
/// reads is a field of coordinates (two uint channels) — different formats, so
/// different bindings, so different pipelines whatever the arithmetic.
@fragment
fn fs_field_seed(@builtin(position) pos: vec4<f32>) -> @location(0) vec2<u32> {
    let coord = vec2<i32>(pos.xy);
    if textureLoad(ink, coord, 0).r < INK_FLOOR {
        return vec2<u32>(NO_SEED, NO_SEED);
    }
    return vec2<u32>(u32(coord.x), u32(coord.y));
}

/// One flood step: keep the nearest of this pixel's own seed and the nine
/// candidates a jump of [`Jump::step`] reaches.
///
/// Nine and not eight, the centre being one of them: a pixel that already holds
/// a seed has to defend it against the ring, or a step that lands on a nearer
/// letter's territory would overwrite a closer answer with a further one.
///
/// Squared distances throughout — the comparison is all this makes, and a
/// square root is monotone, so the ordering is the same and one per candidate
/// is saved. The `f32` they are computed in holds a pane's squared diagonal
/// exactly: at 8192 across it is under 2^27, well inside the 24 bits an f32
/// mantissa carries whole, so two candidates never tie by rounding.
@fragment
fn fs_field_step(@builtin(position) pos: vec4<f32>) -> @location(0) vec2<u32> {
    let coord = vec2<i32>(pos.xy);
    let size = vec2<i32>(textureDimensions(src));
    var best = vec2<u32>(NO_SEED, NO_SEED);
    var best_d = 3.4e38;
    for (var dy = -1; dy <= 1; dy = dy + 1) {
        for (var dx = -1; dx <= 1; dx = dx + 1) {
            let at = coord + vec2<i32>(dx, dy) * jump.step.x;
            // A jump off the pane reads nothing rather than the edge pixel's
            // seed: `textureLoad` out of bounds is undefined, and clamping
            // would smear the border column's answer across the margin.
            if at.x < 0 || at.y < 0 || at.x >= size.x || at.y >= size.y {
                continue;
            }
            let cand = textureLoad(src, at, 0).xy;
            if cand.x == NO_SEED {
                continue;
            }
            let d = vec2<f32>(coord - vec2<i32>(cand));
            let dist = dot(d, d);
            if dist < best_d {
                best_d = dist;
                best = cand;
            }
        }
    }
    return best;
}
