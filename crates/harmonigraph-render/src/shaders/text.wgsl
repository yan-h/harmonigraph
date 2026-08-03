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
    /// The pane the `occluder` depth buffer covers, in points: min, then
    /// size. A zero size is a batch with nothing to be hidden behind, and
    /// the only thing that stops `occluder` from being read.
    pane: vec4<f32>,
};

@group(0) @binding(0) var<uniform> locals: Locals;
@group(0) @binding(1) var atlas: texture_2d<f32>;
@group(0) @binding(2) var atlas_sampler: sampler;
/// Depth of whatever the lattice drew last at each pixel of `locals.pane`,
/// which under that pass's back-to-front order is what covers there, and how
/// much of the pixel that fragment covered. 1x1 stand-ins — the far plane and
/// no coverage — stand in for batches with no lattice under them, so both are
/// always bound and never conditional.
@group(0) @binding(3) var occluder: texture_depth_2d;
@group(0) @binding(4) var occluder_cover: texture_2d<f32>;

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
    /// Where this glyph sits among the lattice's nodes, in the clip depth
    /// `occluder` holds. Flat: a label is a flat thing standing at one
    /// depth, not a surface leaning through the picture.
    @location(5) @interpolate(flat) depth: f32,
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
    @location(4) depth: f32,
) -> VertexOut {
    let corner = vec2<f32>(
        select(0.0, 1.0, (vertex & 1u) == 1u),
        select(0.0, 1.0, (vertex & 2u) == 2u),
    );

    // Grown by the rim's reach, since that is painted outside the ink. The
    // grown edge lands on a whole physical pixel (the radius is snapped
    // before it arrives), so the glyph's own texels stay aligned 1:1 with
    // the framebuffer exactly as egui's own quad has them.
    let reach = rim_reach();
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
    out.depth = depth;
    return out;
}

/// How much of this fragment survives what the lattice drew in FRONT of it:
/// 1 where nothing does, 0 under something opaque, and the fraction the
/// picture left of the background in between.
///
/// This is what makes a label part of the picture rather than a layer over
/// it: a node nearer the camera hides the name of a node behind it, the same
/// way it hides the node itself. Per pixel, so a name goes under the shape
/// that covers it rather than being dropped whole.
///
/// It is a FRACTION rather than a yes or no because depth carries no alpha,
/// and a node paints a long way past what a reader can see — the glow, and
/// the sevens knockout's fade, both run out to a percent of opacity. Cut on
/// depth alone, a name loses whole letters to a halo that is not visibly
/// there, along an edge that is nowhere in the picture. The coverage the
/// lattice pass records beside the depth (see `Covered` in lattice.wgsl) is
/// exactly the fraction to take.
///
/// The depth test is strict, and a label is handed a depth slightly in FRONT
/// of its own node (see `GlyphInstance::depth`) — the two together are what
/// keeps a name off the disc it is written on, where the depths are equal to
/// within the difference between a matrix multiplied on the CPU and the same
/// one multiplied on the GPU.
///
/// Anything the buffers do not cover survives whole: a batch with no lattice
/// under it, and a glyph that has wandered outside the pane they describe,
/// which happens where a pane's own clip has already decided the pixel is
/// not drawn.
fn visible(in: VertexOut) -> f32 {
    if locals.pane.z <= 0.0 || locals.pane.w <= 0.0 {
        return 1.0;
    }
    // `position` is in physical pixels of the whole surface; the pane is in
    // points, as the glyph rects are.
    let point = in.position.xy / max(locals.pixels_per_point, 1e-6);
    let uv = (point - locals.pane.xy) / locals.pane.zw;
    if uv.x < 0.0 || uv.y < 0.0 || uv.x >= 1.0 || uv.y >= 1.0 {
        return 1.0;
    }
    // The buffers are the pane at the render scale, which is neither the
    // pane's points nor its pixels — so they are asked for their own size
    // rather than told one. Both are that size, being one pass's
    // attachments.
    let size = vec2<f32>(textureDimensions(occluder));
    let texel = vec2<i32>(clamp(uv * size, vec2<f32>(0.0), size - 1.0));
    if textureLoad(occluder, texel, 0) >= in.depth {
        return 1.0;
    }
    return 1.0 - clamp(textureLoad(occluder_cover, texel, 0).r, 0.0, 1.0);
}

/// The glyph's coverage at `texel`, and zero outside its own patch of the
/// atlas — a neighbouring glyph sits immediately next to it there, and
/// reading that would smear pieces of unrelated letters into the rim.
fn coverage(in: VertexOut, texel: vec2<f32>) -> f32 {
    if texel.x < in.uv_min.x || texel.y < in.uv_min.y
        || texel.x > in.uv_max.x || texel.y > in.uv_max.y {
        return 0.0;
    }
    return textureSample(atlas, atlas_sampler, texel / locals.atlas_size).a;
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
    let survives = visible(in);
    if in.rim.a <= 0.0 || survives <= 0.0 {
        discard;
    }
    var alpha = ring(in, locals.ring0, 0.0);
    alpha = ring(in, locals.ring1, alpha);
    alpha = alpha * survives;
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
    let cov = coverage(in, in.texel) * visible(in);
    if cov <= 0.0 {
        discard;
    }
    // Premultiplied throughout, so one factor dims the ink and thins it by
    // the same amount — which is what going under something translucent is.
    return in.fill * cov;
}
