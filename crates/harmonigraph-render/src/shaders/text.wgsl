// Haloed label text: one instanced quad per GLYPH, with the rim computed
// per pixel instead of drawn as repeated copies of the text.
//
// The rim used to be the label stamped around two rings — 20 more copies of
// every glyph, every frame, which on a lattice full of labels was most of
// the geometry in the frame. It is the same rim here, arrived at from the
// other side: every stamp shares one color, so their composite collapses to
//
//     alpha = 1 - PRODUCT over samples of (1 - ring_alpha * coverage_i)
//
// and a fragment can evaluate that by sampling the glyph's own atlas patch
// at the same offsets. Not an approximation of the stamped rim — the same
// arithmetic, with the loop moved from the CPU's shape list into the
// fragment shader.
//
// Glyphs come from egui: it lays the text out, rasterizes into its font
// atlas, and hands over each glyph's screen rect and atlas rect. This
// shader only decides how they reach the framebuffer.

struct Locals {
    /// egui's screen size in points; positions arrive in points.
    screen_points: vec2<f32>,
    /// Font atlas size in texels, for normalizing the glyph's uv rect.
    atlas_size: vec2<f32>,
    /// Physical pixels per point: the atlas is rasterized at device scale,
    /// so this converts a rim radius in points into a texel offset.
    pixels_per_point: f32,
    _pad: f32,
    /// The rim's two rings, as (radius in points, stamp alpha, samples, 0).
    /// Zero samples is a ring that isn't drawn.
    ring0: vec4<f32>,
    ring1: vec4<f32>,
};

@group(0) @binding(0) var<uniform> locals: Locals;
@group(0) @binding(1) var atlas: texture_2d<f32>;
@group(0) @binding(2) var atlas_sampler: sampler;

struct VertexOut {
    @builtin(position) position: vec4<f32>,
    /// Atlas coordinate of this fragment, in TEXELS — outside the glyph's
    /// own rect over the margin the rim reaches into.
    @location(0) texel: vec2<f32>,
    /// The glyph's own patch of the atlas, in texels. Samples outside it
    /// belong to a different glyph and must read as nothing.
    @location(1) @interpolate(flat) uv_min: vec2<f32>,
    @location(2) @interpolate(flat) uv_max: vec2<f32>,
    /// Premultiplied sRGB, as egui carries `Color32`.
    @location(3) @interpolate(flat) fill: vec4<f32>,
    @location(4) @interpolate(flat) rim: vec4<f32>,
};

/// How far outside its own rect a glyph's rim can reach, in points.
fn rim_reach() -> f32 {
    return max(
        select(0.0, locals.ring0.x, locals.ring0.z > 0.0),
        select(0.0, locals.ring1.x, locals.ring1.z > 0.0),
    );
}

@vertex
fn vs_glyph(
    @builtin(vertex_index) vertex: u32,
    // Screen rect of the glyph's ink in points: min, then size.
    @location(0) rect: vec4<f32>,
    // The same glyph in the atlas, in texels: min, then max.
    @location(1) uv: vec4<f32>,
    @location(2) fill: vec4<f32>,
    @location(3) rim: vec4<f32>,
) -> VertexOut {
    let corner = vec2<f32>(
        select(0.0, 1.0, (vertex & 1u) == 1u),
        select(0.0, 1.0, (vertex & 2u) == 2u),
    );

    // Grown by the rim's reach, since that is painted outside the ink. The
    // grown edge lands on a whole physical pixel (the radius is snapped
    // before it arrives), so the glyph's own texels stay aligned 1:1 with
    // the framebuffer exactly as egui's own quad has them.
    //
    // Never less than the margin `coverage` reads into, whatever the rings
    // say: a quad that stops at the ink cuts off the same edge column the
    // margin exists to keep, and a caller asking for no rim at all is the one
    // way to get one. The two are the same distance in different spaces, so
    // this is the margin's own texel expressed in points. Both rings drawn,
    // it is a quarter point against the rim's two and changes nothing.
    let reach = max(rim_reach(), PATCH_MARGIN / locals.pixels_per_point);
    let pos = rect.xy - vec2<f32>(reach) + corner * (rect.zw + 2.0 * reach);

    var out: VertexOut;
    out.position = vec4<f32>(
        2.0 * pos.x / locals.screen_points.x - 1.0,
        1.0 - 2.0 * pos.y / locals.screen_points.y,
        0.0,
        1.0,
    );
    // Points map to texels at the device scale, and the glyph's atlas patch is
    // exactly its ink: the same corner, in the other space.
    //
    // Exactly so only while `rect` carries the size its atlas patch was
    // rasterized at. A caller may draw a glyph off that size — the UI does, to
    // follow a zoom without asking for an atlas entry per frame — and then the
    // two spans across this quad are in a ratio of `k` rather than 1, while the
    // margin below is still added in the raster's texels. The ink lands about
    // `(k - 1) * reach / (glyph + reach)` narrow of the quad, which is under
    // half a percent at the couple of percent of magnification the UI's size
    // ladder can leave over, and the rim likewise. Sub-pixel, and the reason
    // the caller bounds `k` rather than this shader taking it per instance:
    // one more vertex attribute on every glyph in the frame, to correct
    // something below the grid it is drawn on.
    let texel_reach = reach * locals.pixels_per_point;
    out.texel = uv.xy - vec2<f32>(texel_reach)
        + corner * ((uv.zw - uv.xy) + 2.0 * texel_reach);
    out.uv_min = uv.xy;
    out.uv_max = uv.zw;
    out.fill = fill;
    out.rim = rim;
    return out;
}

/// How far past its own patch a glyph may still be read, in texels.
///
/// This is what makes a sampled glyph a picture of a letter rather than of a
/// letter with its last column sheared off. A tap at the patch's own edge
/// already reads half of its neighbouring texel — that is what a bilinear tap
/// on a texel boundary IS — so cutting at the edge exactly puts a step of half
/// the edge texel's coverage into a function of position, and a label at a
/// fractional offset walks through it: sliding one physical pixel snaps that
/// column on and off once, at every glyph, for as long as the picture moves.
/// Half a texel further out the tap reads epaint's padding whole, so the
/// coverage reaches zero on its own and the step is gone.
///
/// Half a texel is also as far as it can go. epaint leaves at most one
/// transparent texel around a glyph, so a tap a whole texel out would start
/// blending the letter packed next door back in — which is the smear the patch
/// bound exists to prevent, arriving through the margin meant to fix it.
///
/// At MOST one, and the difference is the whole of [`outside_atlas`]: epaint
/// pads after a glyph and between bands, never before the first of either, so
/// against the atlas's own walls there is no texel out there at all. What this
/// margin reads there is decided by the sampler rather than by the atlas, and
/// has to be corrected for.
const PATCH_MARGIN: f32 = 0.5;

/// What a tap reaching past the ATLAS's own edge must be scaled by: the weight
/// the texel out there would have carried, had there been one.
///
/// [`PATCH_MARGIN`] reads half a texel outside a glyph's patch in order to find
/// the transparent texel epaint leaves around it. Against the atlas's own
/// boundary there is no such texel — epaint's first band of glyphs starts at
/// row 0, and the first glyph of any band at column 0 — and the sampler's
/// `ClampToEdge` answers a tap past the boundary with the glyph's own edge
/// texel instead. That is the letter's top row read a SECOND time in place of
/// the transparency the margin is reading FOR: a doubled row of ink with a seam
/// under it, which is what a cap detached from the rest of the letter is.
///
/// Not a rare edge, either. The first band holds every glyph until the atlas
/// needs a second one, so at ordinary label sizes this is every glyph in the
/// frame rather than the few unlucky enough to be packed against a wall.
///
/// Exact rather than a softening, because a bilinear tap's four weights are one
/// weight per axis multiplied together: dropping the pair that lies outside is
/// dropping a whole row or column of the 2x2, and that is this factor. `texel`
/// addresses texel CENTRES at `texel - 0.5`, so a tap reaches outside on the
/// low side while `texel < 0.5` and on the high side while `texel` is within
/// half a texel of `atlas_size`; everywhere between, this is 1.
fn outside_atlas(texel: vec2<f32>) -> f32 {
    let low = clamp(texel + vec2<f32>(0.5), vec2<f32>(0.0), vec2<f32>(1.0));
    let high = clamp(locals.atlas_size + vec2<f32>(0.5) - texel, vec2<f32>(0.0), vec2<f32>(1.0));
    let weight = low * high;
    return weight.x * weight.y;
}

/// The glyph's coverage at `texel`, and zero outside its own patch of the
/// atlas (plus [`PATCH_MARGIN`]) — past the transparent texel epaint leaves
/// around every glyph a neighbouring letter begins, and reading that would
/// smear pieces of unrelated letters into the rim.
fn coverage(in: VertexOut, texel: vec2<f32>) -> f32 {
    if texel.x < in.uv_min.x - PATCH_MARGIN || texel.y < in.uv_min.y - PATCH_MARGIN
        || texel.x > in.uv_max.x + PATCH_MARGIN || texel.y > in.uv_max.y + PATCH_MARGIN {
        return 0.0;
    }
    return textureSample(atlas, atlas_sampler, texel / locals.atlas_size).a
        * outside_atlas(texel);
}

/// Accumulate one ring of the rim: `1 - PRODUCT(1 - alpha * coverage)`,
/// which is what stamping the text around that ring composites to.
///
/// `alpha` is the ring's own opacity TIMES the rim color's, because that is
/// what a stamp carried: the color handed in is already faded by the label's
/// strength, and each stamp was drawn in it.
fn ring(in: VertexOut, spec: vec4<f32>, acc: f32) -> f32 {
    let samples = i32(spec.z + 0.5);
    if samples <= 0 {
        return acc;
    }
    let radius = spec.x * locals.pixels_per_point;
    var open = 1.0 - acc;
    for (var i = 0; i < samples; i = i + 1) {
        let angle = 6.2831853 * f32(i) / f32(samples);
        // The stamp is the glyph drawn at +offset, so the fragment reads the
        // glyph at -offset to ask whether that stamp covers it.
        let off = vec2<f32>(cos(angle), sin(angle)) * radius;
        open = open * (1.0 - spec.y * in.rim.a * coverage(in, in.texel - off));
    }
    return 1.0 - open;
}

/// The rim alone, premultiplied. Drawn as its own pass over every glyph
/// before any fill, because that is the order stamping had: all of the rim,
/// then all of the text. Two neighbouring letters otherwise darken each
/// other's ink where their rims overlap.
@fragment
fn fs_rim(in: VertexOut) -> @location(0) vec4<f32> {
    if in.rim.a <= 0.0 {
        discard;
    }
    var alpha = ring(in, locals.ring0, 0.0);
    alpha = ring(in, locals.ring1, alpha);
    if alpha <= 0.0 {
        discard;
    }
    // `rim` arrives premultiplied and its alpha is already inside the
    // accumulation, so the hue is what is left to scale: un-premultiply,
    // take the accumulated opacity, premultiply back.
    return vec4<f32>(in.rim.rgb / in.rim.a * alpha, alpha);
}

/// The glyphs themselves, over the rim.
@fragment
fn fs_fill(in: VertexOut) -> @location(0) vec4<f32> {
    let cov = coverage(in, in.texel);
    if cov <= 0.0 {
        discard;
    }
    return in.fill * cov;
}
