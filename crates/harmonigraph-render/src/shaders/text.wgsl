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
    /// How much of what stands under a name its shadow takes where that shadow
    /// is whole, 0..1 — the lattice's `glow_shadow_depth`, the same number a
    /// ring's own shadow spends. Read by [`fs_shadow_box`] alone; every other
    /// surface casts no shadow and leaves it at 0.
    shadow_depth: f32,
    /// How many terms the lattice's kernel has, and so how many cells a caster
    /// carries and how many taps its box takes (`ShadowKernel::terms`). Read by
    /// [`fs_shadow_box`] alone, and 0 everywhere else — which is a surface that
    /// samples the atlas not at all.
    shadow_terms: f32,
    /// The exponent a DISTANCE row's decay is taken over (the lattice's
    /// `u.misc11.y`), in the slot that was the gap before the atlas size. Read
    /// by [`fs_shadow_box`] alone, and only where a term of the row holds a
    /// distance; 0 everywhere else.
    shadow_shape: f32,
    /// The shadow atlas's size in texels — what [`vs_glyph_cell`] maps a cell
    /// into. The scene pass reads the same size off the texture itself.
    shadow_atlas_size: vec2<f32>,
    /// The lattice's shadow CURVE, in the two slots that were the gap before
    /// the rings: how much a caster thin against σ is worth against a solid
    /// one, and the exponent that bends where along the shadow's width the
    /// depth sits. The pair `shadow_transmittance` takes beside the depth, so a
    /// name's shadow and a ring's are shaped by one number each. Read by
    /// [`fs_shadow_box`] alone; every other surface casts no shadow and leaves
    /// both at 0.
    shadow_gain: f32,
    shadow_curve: f32,
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

/// A point of the pane, in points, as clip space.
fn on_screen(pos: vec2<f32>) -> vec4<f32> {
    return vec4<f32>(
        2.0 * pos.x / locals.screen_points.x - 1.0,
        1.0 - 2.0 * pos.y / locals.screen_points.y,
        0.0,
        1.0,
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
    var out = glyph_vertex(vertex, rect, uv, fill, rim, sheet, rim_reach());
    out.position = on_screen(out.position.xy);
    return out;
}

/// The same glyph, drawn into its name's cell of the shadow atlas rather than
/// onto the pane — [`fs_glyph_ink`]'s quad.
///
/// The three attributes after the glyph's own are the name's `ShadowBox`
/// (shadow.rs), one copy per glyph: the box's corner in points and the cell's
/// in texels are the same point, and `terms.x` is the scale between the two, so
/// the quad lands in the cell exactly where the pane's quad lands on the pane.
/// Everything a fragment reads — `texel`, the sheet, the patch bounds — is
/// interpolated across the quad and does not know which of the two it is on.
@vertex
fn vs_glyph_cell(
    @builtin(vertex_index) vertex: u32,
    @location(0) rect: vec4<f32>,
    @location(1) uv: vec4<f32>,
    @location(2) fill: vec4<f32>,
    @location(3) rim: vec4<f32>,
    @location(4) sheet: u32,
    @location(5) box_rect: vec4<f32>,
    @location(6) box_cell: vec4<f32>,
    @location(7) box_meta: vec4<f32>,
) -> VertexOut {
    var out = glyph_vertex(vertex, rect, uv, fill, rim, sheet, rim_reach());
    let texel = cell_texel(out.position.xy, box_rect, box_cell, box_meta.x);
    // Drawn in plain screen space, so the `w` a cell's clip position is built
    // with is the 1 this quad already carries.
    out.position = select(
        no_quad(),
        cell_clip(texel, locals.shadow_atlas_size, 1.0),
        cell_packed(box_cell),
    );
    return out;
}

/// Everything a glyph's quad carries, with `position` holding the corner in
/// the PANE's points rather than in clip space — the caller maps it, because
/// which surface the quad lands on is the caller's ([`vs_glyph`],
/// [`vs_glyph_cell`]).
fn glyph_vertex(
    vertex: u32,
    rect: vec4<f32>,
    uv: vec4<f32>,
    fill: vec4<f32>,
    rim: vec4<f32>,
    sheet: u32,
    grown_by: f32,
) -> VertexOut {
    let corner = vec2<f32>(
        select(0.0, 1.0, (vertex & 1u) == 1u),
        select(0.0, 1.0, (vertex & 2u) == 2u),
    );

    // Grown by whatever this draw paints outside the ink — the rim's reach, or
    // the shadow's. The grown edge lands on a whole physical pixel (the rim's
    // radius is snapped before it arrives), so the glyph's own texels stay
    // aligned 1:1 with the framebuffer exactly as egui's own quad has them.
    //
    // Never less than the distance `coverage` reads into, whatever the caller
    // asks for: a quad that stops at the ink cuts off the same edge column the
    // margin exists to keep, and a caller asking for no rim at all is the one
    // way to get one. That distance is the patch margin plus the offset its
    // taps sit at, since a fragment a quarter texel inside the bound still
    // reaches the bound with its outer tap. The two are the same distance in
    // different spaces, so this is those texels expressed in points. Both
    // rings drawn, it is three eighths of a point against the rim's two and
    // changes nothing.
    let reach = max(grown_by, (PATCH_MARGIN + FILTER_TAP) / locals.pixels_per_point);
    let pos = rect.xy - vec2<f32>(reach) + corner * (rect.zw + 2.0 * reach);

    var out: VertexOut;
    out.position = vec4<f32>(pos, 0.0, 1.0);
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

/// The light at a glyph's own pixel: [`glow_light`] with the coordinate held
/// inside the texture.
///
/// The glow target is the scene target's size, so a fragment of this pass
/// stands at one of its texels, and the clamp is what keeps a rounding at the
/// last row from reading nothing.
fn glyph_light(coord: vec2<i32>) -> vec4<f32> {
    let edge = vec2<i32>(textureDimensions(glow_tex)) - vec2<i32>(1, 1);
    return glow_light(clamp(coord, vec2<i32>(0, 0), edge));
}

/// The lattice's own glyphs: [`fs_fill`]'s ink, washed by the light it stands
/// in.
///
/// A name is ink laid over ground the light is already under, exactly as a
/// resting marker's cross is (`plus_paint` in lattice.wgsl), so unwashed it
/// comes out DARKER inside a halo than the ground to either side of it — a
/// name reading as a hole punched in the light precisely where the light is
/// brightest. The wash is what makes a lit node's name stand IN its halo.
///
/// The WHOLE field, at any setting of the lattice's Wash bar, and the RAW
/// light rather than the shaded one: both are the cross's terms (see
/// `plus_paint`, which sets out why), and the resting field is one field
/// whether a position is showing its name or its marker.
///
/// The INK alone. The shadow the name casts on what stands behind it is
/// [`fs_shadow_box`], drawn just before this over the name's own box: the
/// shadow is a shape of the NAME and the ink a shape of the glyph, and only
/// the second belongs in a per-glyph draw.
///
/// Its own entry point rather than a branch in [`fs_fill`], because what parts
/// them is the BINDING: only the lattice has a light to hand a glyph pipeline,
/// and a pipeline declares the groups its entry point reads.
@fragment
fn fs_fill_lit(in: VertexOut) -> @location(0) vec4<f32> {
    let cov = coverage(in, in.texel);
    if cov <= 0.0 {
        discard;
    }
    // Premultiplied, as everything this pass draws is: the ink is the colour
    // the Marker ink bar names, and only its coverage varies across a glyph.
    let ink = in.fill * cov;
    let coord = vec2<i32>(in.position.xy);
    let light = glyph_light(coord);
    return vec4<f32>(wash_over(ink.rgb, ink.a, light.rgb, 1.0), ink.a);
}

/// A lattice name's coverage, into its own cell of the shadow atlas
/// (`shadow.rs`) — what its shadow is a blur of.
///
/// Written under a MAX blend, so where two glyphs of one name overlap in their
/// margins the cell holds their union and the order they arrive in decides
/// nothing. The coverage ALONE: the name's strength is applied where the cell
/// is read ([`fs_shadow_box`]), as a share of the shadow rather than a scale on
/// the ink it is blurred from, and the two are not the same thing — see there.
///
/// Drawn through [`vs_glyph_cell`], at the cell's own transform rather than the
/// pane's; nothing here knows or cares which, since `texel` is interpolated
/// across the quad whatever the quad is mapped to.
@fragment
fn fs_glyph_ink(in: VertexOut) -> @location(0) vec4<f32> {
    return vec4<f32>(coverage(in, in.texel), 0.0, 0.0, 0.0);
}

struct BoxOut {
    @builtin(position) position: vec4<f32>,
    /// Where this fragment stands on the pane, in points — the space every
    /// term's cell is mapped from (`shadow_kernel`).
    @location(0) at: vec2<f32>,
    /// The caster's level, 0..1.
    @location(1) @interpolate(flat) level: f32,
    /// Which caster this is, in `shadow_casters`.
    @location(2) @interpolate(flat) who: u32,
};

/// A caster's box: the quad its shadow is laid over, which is its ink's own box
/// grown by the WIDEST of its kernel's terms (`pack` in shadow.rs).
///
/// No vertex buffer at all. The draw is one instance at the caster's own index
/// (`Draw::Label` in lib.rs), so the index IS the instance index, and every
/// number the quad needs is in the shared caster array. The same representation
/// serves nodes and labels without making this draw carry a second copy in an
/// instance stream.
@vertex
fn vs_shadow_box(
    @builtin(vertex_index) vertex: u32,
    @builtin(instance_index) who: u32,
) -> BoxOut {
    let corner = vec2<f32>(
        select(0.0, 1.0, (vertex & 1u) == 1u),
        select(0.0, 1.0, (vertex & 2u) == 2u),
    );
    let caster = shadow_casters[min(who, arrayLength(&shadow_casters) - 1u)];
    let at = caster.rect.xy + corner * caster.rect.zw;
    var out: BoxOut;
    out.position = on_screen(at);
    out.at = at;
    out.level = caster.level.x;
    out.who = who;
    return out;
}

/// A name's shadow: everything already in the frame under the box, multiplied
/// by the transmittance of the name's blurred ink.
///
/// The multiply rides on the blend the scene pass already composites under.
/// `PREMULTIPLIED_ALPHA_BLENDING` is `out = src + dst * (1 - src.a)`, so a
/// fragment of `rgb = 0, a = 1 - T` leaves `dst * T`: the frame under the name
/// darkened by `T`, whatever the frame holds there — ground, a ring's ink, a
/// marker, another name, or nothing at all (a transparent texel becomes
/// `(0, 1 - T)`, which composites the pane's own ground to `ground * T`). No
/// receiver carries any shadow code; the light is under everything and takes
/// the shadow by being there first. Drawn before the name's own glyphs, so the
/// name's ink is the one thing its shadow never touches.
///
/// `T` is `shadow_transmittance` over the blur standing at this fragment, and
/// it is the same function a ring and a cross spend over their own cells
/// (`shadow_through` in lattice.wgsl) — one Shadow bar, one darkness, whatever
/// the caster. The name's LEVEL is spent there as a share, which is what a name
/// easing in as its marker eases out casts.
///
/// One bilinear tap PER TERM of the kernel, each in its own cell at its own
/// resolution (`shadow_kernel` in common.wgsl), summed by weight before the
/// transmittance is taken: a sum of transmittances is a different picture from
/// the transmittance of a sum, and the second is the one a kernel means.
///
/// The LATTICE's box draw alone binds group 2. The bindings a pipeline must
/// carry are the ones its entry point reads, so every other surface's text —
/// which casts no shadow and draws through [`fs_rim`] and [`fs_fill`] — takes a
/// layout with group 0 and nothing else, and a pane with no atlas has no dummy
/// to bind.
///
/// The two attachments part in the SHADOW here as they do at every other
/// caster's draw (`Painted` in lattice.wgsl): one ink into both, and the alpha
/// — which is what the fragment takes off the frame under it — deeper in the
/// copy the bright pass reads. What is a NAME's alone is the ink, kept out of
/// `nodes` so it neither glows nor bites the halo of the node it covers
/// (`SceneOut`, common.wgsl).
///
/// What it buys: the composite is `scene + bloom * strength` into an 8-bit
/// target, so over a bright halo the unshadowed pixel is already past 1 and
/// pins to white, and a shadow that does not carry the sum back under 1 lands
/// as nothing at all. Modelled at a halo of 0.9 and a name's `T` of 0.62, the
/// darkening that reaches the screen is 38% once the bloom's own copy is taken
/// to a whole shadow — with the visible shadow left exactly as light as
/// `shadow_depth` says. Over an unlit node there is no bloom to take away and
/// this does nothing, which is what makes it an answer to the bright case
/// alone.
fn name_shadow_through(in: BoxOut, full: f32) -> vec2<f32> {
    return vec2<f32>(
        shadow_transmittance(full, locals.shadow_depth, in.level, locals.shadow_curve),
        shadow_transmittance(full, 1.0, in.level, locals.shadow_curve),
    );
}

fn name_shadow_at(in: BoxOut, at: vec2<f32>) -> vec2<f32> {
    return name_shadow_through(in, shadow_kernel(
        in.who,
        at,
        u32(max(locals.shadow_terms, 0.0)),
        locals.shadow_gain,
        locals.shadow_shape,
    ));
}

/// Whether this name carries a distance term whose reconstructed output needs
/// the pixel-footprint filter below. A blur cell is already band-limited by its
/// kernel; sampling it again changes that profile and makes every other row pay
/// for a Distance representation constraint.
fn name_shadow_has_distance(who: u32) -> bool {
    if who >= arrayLength(&shadow_casters) {
        return false;
    }
    for (var t = 0u; t < min(u32(max(locals.shadow_terms, 0.0)), SHADOW_TERMS); t = t + 1u) {
        if shadow_casters[who].kind[t] >= 0.5 * DISTANCE_KIND {
            return true;
        }
    }
    return false;
}

@fragment
fn fs_shadow_box(in: BoxOut) -> SceneOut {
    // Centres of the output pixel's four quadrants, expressed in pane points.
    // The derivatives stay outside the branch because WGSL requires them in
    // uniform control flow even though `who` is flat across this whole quad.
    let dx = 0.25 * dpdx(in.at);
    let dy = 0.25 * dpdy(in.at);
    var through: vec2<f32>;
    if name_shadow_has_distance(in.who) {
        // Each sample spends the COMPLETE profile and transmittance before the
        // average. Averaging distances or kernel coverage first would move the
        // nonlinear powers in `standoff_coverage` and `shadow_transmittance`
        // outside the filter and draw a different shadow.
        through = 0.25 * (
            name_shadow_at(in, in.at - dx - dy)
            + name_shadow_at(in, in.at + dx - dy)
            + name_shadow_at(in, in.at - dx + dy)
            + name_shadow_at(in, in.at + dx + dy)
        );
    } else {
        through = name_shadow_at(in, in.at);
    }
    let t = through.x;
    // The bright pass's copy, always at a WHOLE shadow (1) rather than at
    // `shadow_depth`: the copy the bright pass reads takes every caster's
    // shadow to the shader's own floor, whatever the visible one is left at.
    let lit = through.y;
    return SceneOut(
        vec4<f32>(0.0, 0.0, 0.0, 1.0 - t),
        vec4<f32>(0.0, 0.0, 0.0, 1.0 - lit),
    );
}
