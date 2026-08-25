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
//
// A drawn MARK — the `+`, the chevron, the accidentals — is a glyph here too,
// cut from a sheet of its own that the UI rasterizes and packs. It reaches the
// framebuffer through everything below unchanged, which is the point of it
// being here at all: it takes its rim from the same arithmetic instead of a
// second bitmap, and its place in the lattice's draw order from the run of
// letters it was collected with, so a node in front covers a name and its
// marks together.

struct Locals {
    /// egui's screen size in points; positions arrive in points.
    screen_points: vec2<f32>,
    /// Font atlas size in texels, for normalizing the glyph's uv rect.
    atlas_size: vec2<f32>,
    /// And the drawn marks' own sheet, the same way.
    mark_atlas_size: vec2<f32>,
    /// The screen axis this pane's labels TRAVEL along, as a unit vector —
    /// where `coverage` puts its two taps. See `FILTER_TAP`, which is how far
    /// along it they sit and why one axis rather than both.
    filter_axis: vec2<f32>,
    /// Physical pixels per point: both sheets are rasterized at device scale,
    /// so this converts a rim radius in points into a texel offset.
    pixels_per_point: f32,
    /// WGSL aligns a `vec4<f32>` to 16 bytes, so the rings start at 48 and
    /// these three are the gap in front of them.
    _pad0: f32,
    _pad1: f32,
    _pad2: f32,
    /// The rim's two rings, as (radius in points, stamp alpha, samples, 0).
    /// Zero samples is a ring that isn't drawn.
    ring0: vec4<f32>,
    ring1: vec4<f32>,
};

@group(0) @binding(0) var<uniform> locals: Locals;
@group(0) @binding(1) var atlas: texture_2d<f32>;
@group(0) @binding(2) var atlas_sampler: sampler;
@group(0) @binding(3) var mark_atlas: texture_2d<f32>;

/// The `atlas` an instance carries when it is a drawn mark rather than a
/// letter — `GlyphInstance::MARK`, against `::TYPE`'s zero.
const SHEET_MARK: u32 = 1u;

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
    /// Which sheet this glyph is cut from, and that sheet's size in texels —
    /// carried rather than looked up per sample, so only the texture read
    /// itself has to choose.
    @location(5) @interpolate(flat) sheet: u32,
    @location(6) @interpolate(flat) sheet_size: vec2<f32>,
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
    // Which sheet `uv` addresses.
    @location(4) sheet: u32,
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
    // Never less than the distance `coverage` reads into, whatever the rings
    // say: a quad that stops at the ink cuts off the same edge column the
    // margin exists to keep, and a caller asking for no rim at all is the one
    // way to get one. That distance is the patch margin plus the offset its
    // taps sit at, since a fragment a quarter texel inside the bound still
    // reaches the bound with its outer tap. The two are the same distance in
    // different spaces, so this is those texels expressed in points. Both
    // rings drawn, it is three eighths of a point against the rim's two and
    // changes nothing.
    let reach = max(rim_reach(), (PATCH_MARGIN + FILTER_TAP) / locals.pixels_per_point);
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
    out.sheet = sheet;
    out.sheet_size = select(locals.atlas_size, locals.mark_atlas_size, sheet == SHEET_MARK);
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
/// bound exists to prevent, arriving through the margin meant to fix it. The
/// mark sheet buys the margin the same way and owes the same bound: every mark
/// bitmap carries exactly one clear texel of its own on each side
/// (`MARK_BITMAP_PAD`), and they are packed touching.
///
/// At MOST one, and the difference is the whole of [`outside_atlas`]: epaint
/// pads after a glyph and between bands, never before the first of either, so
/// against the atlas's own walls there is no texel out there at all. What this
/// margin reads there is decided by the sampler rather than by the atlas, and
/// has to be corrected for.
const PATCH_MARGIN: f32 = 0.5;

/// How far either side of a fragment [`coverage`]'s reconstruction reaches,
/// in texels, along the axis a label travels.
///
/// One bilinear tap is a tent one texel wide, and a stroke about a pixel
/// across read through it is a different picture at every sub-pixel offset:
/// one dark column at one phase, two half-lit ones a half pixel later. The
/// ink is the same either way — a resample conserves it — so what changes is
/// how much of the mark is partial coverage, and that is a symbol visibly
/// tightening and loosening as it slides. Averaging two taps a quarter texel
/// apart puts a zero in the filter's response at exactly the frequency that
/// swing lives at, `cos(PI * f / 2)` at f = 1, and most of it goes.
///
/// Measured on the composite at the size the spectral roll sets its names,
/// as a share of each symbol's own ink: the flat falls from 12.6% to 4.2%
/// and the sharp from 6.6% to 1.5%. They are the marks it is for — ink that
/// is mostly VERTICAL strokes, against a roll that scrolls sideways — and the
/// bill is a twentieth of their contrast, the flat's darkest pixel going from
/// 0.87 to 0.86 and the sharp's from 1.00 to 0.95. Type pays nothing
/// measurable: a letter's strokes are over a pixel and its halo saturates, so
/// every letter, digit and lattice name keeps a peak of 1.00 and improves
/// besides.
///
/// ONE axis, and the caller's rather than x, because which way a label
/// travels is a property of the pane and not of the filter. The roll's names
/// ride the time axis: with the now-line at the left or the right, time runs
/// ACROSS the pane and their whole motion is sideways; turn it to the top or
/// the bottom and time runs DOWN it instead. Bilinear is separable —
/// `B(x,y) = Bx(x)·By(y)` — so offsetting in x changes the x factor alone and
/// leaves the y response its full swing, which is a filter that helps in two
/// of the four orientations and does nothing in the other two. `filter_axis`
/// is that choice, made once per pane in `Locals`.
///
/// Not answered by taking a second pair of taps up and down instead. Isotropic
/// is worth 0.2 of a point on the reading above and costs the flat 15% of its
/// contrast and the count digits 14%, so it would buy the second axis by
/// taxing the first — everywhere, including a 30pt lattice name that swings
/// 0.6% and has nothing to fix.
///
/// The lattice takes x for want of an answer rather than as one: an orbiting
/// camera moves a node name both ways at once, so it has no single travel
/// axis, and at 30pt there is no measurable swing to spend a choice on. What
/// that leaves open is the small end of its zoom, where the names are the roll
/// mark's size and the motion is still diagonal — issue #311 holds it.
///
/// A quarter texel, and no more, because [`PATCH_MARGIN`]'s bound is the
/// wall — a filter reaching a whole texel out reads the glyph packed next
/// door. It also has to be HERE rather than baked into the sheets. A kernel
/// applied on a bitmap's own grid is periodic at the texel rate, so whatever
/// it does at f = 0 it does again at f = 1, and f = 1 is where all of the
/// swing is; only a sub-texel offset reaches it, which is a thing the sampler
/// can do and a rasterizer cannot.
const FILTER_TAP: f32 = 0.25;

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
/// half a texel of the sheet's own size; everywhere between, this is 1.
///
/// The mark sheet's walls are the same case for the same reason: a shelf
/// packer starts its first shelf at row 0 and every shelf at column 0.
fn outside_atlas(in: VertexOut, texel: vec2<f32>) -> f32 {
    let low = clamp(texel + vec2<f32>(0.5), vec2<f32>(0.0), vec2<f32>(1.0));
    let high = clamp(in.sheet_size + vec2<f32>(0.5) - texel, vec2<f32>(0.0), vec2<f32>(1.0));
    let weight = low * high;
    return weight.x * weight.y;
}

/// The alpha of `in`'s own sheet at `texel`.
///
/// `textureSampleLevel` rather than `textureSample`: which sheet is read is a
/// per-instance branch, so the read sits in non-uniform control flow and an
/// implicit-derivative sample is not allowed there. Nothing is lost — both
/// sheets are single-level, so level 0 is what the implicit form resolved to.
fn sheet_alpha(in: VertexOut, texel: vec2<f32>) -> f32 {
    let uv = texel / in.sheet_size;
    if in.sheet == SHEET_MARK {
        return textureSampleLevel(mark_atlas, atlas_sampler, uv, 0.0).a;
    }
    return textureSampleLevel(atlas, atlas_sampler, uv, 0.0).a;
}

/// One tap of [`coverage`]: the sheet's own alpha at `texel`, and zero
/// outside the glyph's own patch of it (plus [`PATCH_MARGIN`]) — past the
/// transparent texel epaint leaves around every glyph a neighbouring letter
/// begins, and reading that would smear pieces of unrelated letters into the
/// rim.
///
/// Inside the margin the answer is the atlas's own alpha only while the tap
/// stays inside the atlas; against a wall it is that alpha scaled by
/// [`outside_atlas`]. So this is not the two-valued thing it reads as, and a
/// caller cannot take a return between zero and the alpha as impossible.
///
/// The bound is applied per TAP rather than once for the pair, and that is
/// what lets the pair exist at all: a tap clipped here is one that has walked
/// past the padding into a neighbour, and it reads zero where the padding it
/// stopped short of also reads zero, so the clip lands between two equal
/// values and puts no step in the picture.
fn tap(in: VertexOut, texel: vec2<f32>) -> f32 {
    if texel.x < in.uv_min.x - PATCH_MARGIN || texel.y < in.uv_min.y - PATCH_MARGIN
        || texel.x > in.uv_max.x + PATCH_MARGIN || texel.y > in.uv_max.y + PATCH_MARGIN {
        return 0.0;
    }
    return sheet_alpha(in, texel) * outside_atlas(in, texel);
}

/// The glyph's coverage at `texel`, reconstructed.
///
/// Two taps [`FILTER_TAP`] either side along the axis a label travels, rather
/// than the one the sampler gives. Both passes read through here — the fill
/// once per fragment, the rim once per stamp — because the shimmer this
/// answers is in the PICTURE and not in either half of it. Measured on the
/// accidentals at the size the spectral roll sets its names, widening the
/// fill alone takes the ink's own swing from 15.9% of its weight to 4.0% and
/// leaves the composite at 12.5%, all but unmoved: the halo is a dilation of
/// the same sub-pixel stroke through `1 - PRODUCT(1 - a)`, it covers several
/// times the area the ink does, and it is where nearly all of what the eye
/// catches lives. Through both, the composite falls to 4.2%.
///
/// The bill is the rim's, and it is the honest cost of this: twenty stamps a
/// fragment become forty taps. The fill's own second tap is free beside it.
fn coverage(in: VertexOut, texel: vec2<f32>) -> f32 {
    // Along the pane's own travel axis, which the sheets' texels are square
    // with: a glyph's quad is axis-aligned and its atlas patch is upright, so
    // a screen direction IS a texel direction and needs no rotating.
    let off = FILTER_TAP * locals.filter_axis;
    return 0.5 * (tap(in, texel - off) + tap(in, texel + off));
}

/// Accumulate one ring of the rim: `1 - PRODUCT(1 - alpha * coverage)`,
/// which is the shape stamping the text around that ring composites to.
///
/// `alpha` is the ring's own opacity ALONE. The rim color's own alpha is the
/// label's strength, and it lands on what this returns instead ([`fs_rim`]) —
/// the one place the rim parts company with what stamping did, and
/// deliberately, because stamping is wrong here. A strength inside the
/// product is spent once per STAMP, and there are twenty of them: the label's
/// level reaches the pixel as `1 - PRODUCT(1 - alpha * s)` where its fill
/// reaches it as `s`, so a name a tenth of the way through its release still
/// draws three quarters of its halo while its ink draws a tenth of itself.
/// The halo IS the letter's shape dilated, so what that paints is the name as
/// a near-black letter with a ghost of white inside it, holding until the
/// last frames and then letting go at once.
///
/// On the accumulated opacity it is what the fill's own alpha already is: a
/// rim at half strength covers half. The RING's opacity stays here, that
/// being the halo's construction rather than the label's level — the tuned
/// pair of sample counts (`RINGS` in `harmonigraph_ui::text`) mean what they
/// meant, and a label at full strength composites to the pixel it did.
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
        open = open * (1.0 - spec.y * coverage(in, in.texel - off));
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
    var cov = ring(in, locals.ring0, 0.0);
    cov = ring(in, locals.ring1, cov);
    if cov <= 0.0 {
        discard;
    }
    // Where the rim color's alpha is spent (see [`ring`]), and a scale of
    // BOTH halves because `rim` arrives premultiplied: one number over the
    // pair is the operation that leaves it so.
    return in.rim * cov;
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
