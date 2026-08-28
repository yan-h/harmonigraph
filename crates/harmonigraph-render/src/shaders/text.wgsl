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
    /// How the light standing under a name is mixed out of the two blends it
    /// was written under — the lattice's Meld bar, the same number every other
    /// reader of that target takes (`glow_light` in lattice.wgsl). Read by the
    /// lattice's entry points alone; every other surface draws no light and
    /// leaves it at 0.
    meld: f32,
    /// A node's own radius on this pane, in points.
    ///
    /// The unit the four Shadow terms below are dialled in: the lattice sets
    /// them as shares of a node (`ViewConfig::glow_shadow`), and a glyph pass
    /// has no node under it to measure a coordinate against, so the conversion
    /// arrives here instead. ONE number for the pane, as the size the names are
    /// typeset at is — an off-sheet node draws smaller and its name with it,
    /// but its shadow is the home sheet's width.
    node_points: f32,
    /// How far a name's standoff reaches, in node radii — the lattice's Shadow
    /// width, `glow_shadow` there.
    shadow: f32,
    /// How much of that width is the fade back to the light, same units —
    /// `glow_shadow_soft`.
    shadow_soft: f32,
    /// How the fade is skewed across its own width, 0..1 as the bar carries it
    /// — `glow_shadow_shape`.
    shadow_shape: f32,
    /// How much of the light a name holds off where its standoff is solid,
    /// 0..1 — `glow_shadow_depth`.
    shadow_depth: f32,
    /// WGSL aligns a `vec4<f32>` to 16 bytes, so the rings start at 64 and
    /// this is the gap in front of them.
    _pad: f32,
    /// The rim's two rings, as (radius in points, stamp alpha, samples, 0).
    /// Zero samples is a ring that isn't drawn.
    ring0: vec4<f32>,
    ring1: vec4<f32>,
    /// The pane's own ground, which is what a name's KNOCKOUT clears to where
    /// no light stands (`fs_fill_lit`). The lattice's `Scene::background`, and
    /// the same value its nodes clear to (`u.background` there) — a hole
    /// painting a different one is a hole that announces itself over empty
    /// lattice, where it should be invisible.
    background: vec4<f32>,
};

@group(0) @binding(0) var<uniform> locals: Locals;
@group(0) @binding(1) var atlas: texture_2d<f32>;
@group(0) @binding(2) var atlas_sampler: sampler;
@group(0) @binding(3) var mark_atlas: texture_2d<f32>;

// The lattice's light, at group 1: the same target its nodes and markers read
// back, in the same layout (`Resources::glow_layout`), so the group the scene
// pass already has bound serves this pass unchanged. The two textures are the
// field under both of its blends, mixed by `locals.meld`.
//
// Bound by the LATTICE's glyph pipelines alone. The bindings a pipeline must
// carry are the ones its entry point reads, so `fs_rim` and `fs_fill` — the
// two every other surface's text draws through — take a layout with group 0 and
// nothing else, and a pane with no light has no dummy to bind.
@group(1) @binding(0) var glow_tex: texture_2d<f32>;
@group(1) @binding(2) var glow_max_tex: texture_2d<f32>;
// The standoff cut into that light, at the third slot the layout carries it in
// — lattice.wgsl's `glow_shade_tex`, the same layer read from a third side.
// What the hole a name knocks out is cleared TO is the light AFTER this, which
// is the one arrangement that lets `fs_fill_lit` and `node_paint` paint the same
// ground at the same pixel.
@group(1) @binding(3) var glow_shade_tex: texture_2d<f32>;

// How far every pixel of the pane stands from the nearest ink of any name on
// it, at group 2: the jump flood's finished field (field.wgsl), plus the ink
// mask it was seeded from.
//
// The field carries a nearest-seed COORDINATE rather than a distance, which is
// what lets the ink mask be read at that coordinate — the strength a shadow is
// cast at is the one at the ink casting it, and off a shared field that is the
// nearest ink's and not the drawing quad's. `NO_SEED` in field.wgsl is the
// coordinate no pane can address, and means no ink within reach.
//
// Bound by the LATTICE's shadow entry points alone. Every other surface's text
// casts no shadow and names no group 2, so a pane with no field has no dummy to
// bind.
@group(2) @binding(0) var field_tex: texture_2d<u32>;
@group(2) @binding(1) var field_ink: texture_2d<f32>;

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
    return glyph_vertex(vertex, rect, uv, fill, rim, sheet, rim_reach());
}

/// One name run's KNOCKOUT quad: the run's own glyph rects as one box, grown
/// by the reach the hole is cut on.
///
/// A box per RUN rather than a quad per glyph, and the whole of the difference
/// is that a fragment is visited ONCE. The hole is painted over what stands
/// behind the name and composited into it, so two quads covering a pixel cut it
/// twice: a coverage of `c` arrives as `1 - (1 - c)^2`, and a name's glyphs all
/// overlap at any Shadow worth the name — seven quads deep, a soft 0.3 of a
/// hole lands as 0.92 of one and the Shadow's whole fade is crushed to a block
/// of ground. One box has no overlap to compound, and off a shared field its
/// answer at a pixel is the same one every glyph of the run would have given.
///
/// `rect` is the run's bounding box in POINTS, tight around the ink; the growth
/// is added here because [`shadow_stop`] is a uniform and the box would
/// otherwise have to be rebuilt on the CPU every time the bar moves.
@vertex
fn vs_glyph_gutter(
    @builtin(vertex_index) vertex: u32,
    // The run's bounding box in points: min, then size.
    @location(0) rect: vec4<f32>,
) -> @builtin(position) vec4<f32> {
    let corner = vec2<f32>(
        select(0.0, 1.0, (vertex & 1u) == 1u),
        select(0.0, 1.0, (vertex & 2u) == 2u),
    );
    let reach = shadow_stop();
    let pos = rect.xy - vec2<f32>(reach) + corner * (rect.zw + 2.0 * reach);
    return vec4<f32>(
        2.0 * pos.x / locals.screen_points.x - 1.0,
        1.0 - 2.0 * pos.y / locals.screen_points.y,
        0.0,
        1.0,
    );
}

/// The whole pane as one quad, for the shade ([`fs_field_shade`]) — the one
/// draw here that is neither a glyph nor a run of them, the field it reads
/// already covering every pixel.
@vertex
fn vs_pane(@builtin(vertex_index) vertex: u32) -> @builtin(position) vec4<f32> {
    let corner = vec2<f32>(
        select(0.0, 1.0, (vertex & 1u) == 1u),
        select(0.0, 1.0, (vertex & 2u) == 2u),
    );
    return vec4<f32>(corner.x * 2.0 - 1.0, 1.0 - corner.y * 2.0, 0.0, 1.0);
}

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

/// The finished light at a pixel of the lattice's glow target, mixed out of the
/// two blends it was written under — `glow_light` in lattice.wgsl, and the same
/// mix for the same reason: ink painting a different one from the picture it
/// stands in is a halo drawn round every name.
///
/// `coord` is clamped into the texture, which is the lattice's own rule
/// (`light_coord`): the glow target is the scene target's size, so a fragment
/// of this pass stands at one of its texels, and the clamp is what keeps a
/// rounding at the last row from reading nothing.
fn glyph_light(coord: vec2<i32>) -> vec4<f32> {
    let edge = vec2<i32>(textureDimensions(glow_tex)) - vec2<i32>(1, 1);
    let at = clamp(coord, vec2<i32>(0, 0), edge);
    let screened = textureLoad(glow_tex, at, 0);
    let brightest = textureLoad(glow_max_tex, at, 0);
    return mix(brightest, screened, clamp(locals.meld, 0.0, 1.0));
}

/// How much of the light at that pixel the lattice's standoffs have taken, off
/// the layer written beside it — `glow_shade_tex`'s x, clamped into the texture
/// the same way [`glyph_light`] clamps into its own.
///
/// Every emitter's, and NOT this name's alone: what a hole is cleared to is the
/// field the picture around it stands in, so the two agree at the hole's edge.
fn load_shade(coord: vec2<i32>) -> f32 {
    let edge = vec2<i32>(textureDimensions(glow_shade_tex)) - vec2<i32>(1, 1);
    let at = clamp(coord, vec2<i32>(0, 0), edge);
    return textureLoad(glow_shade_tex, at, 0).x;
}

/// The whole of `light`, laid over ink already premultiplied by `alpha` —
/// `wash_over` in lattice.wgsl at a share of 1, which is the share every piece
/// of the lattice's RESTING field takes (see `plus_paint`).
///
/// A SCREEN rather than an over, and that is what a neighbour's light is
/// allowed to do: `w + ink * (1 - w)` per channel can only brighten, where an
/// over lets a saturated halo standing behind a name take the name's other
/// channels down with it.
fn wash_over(ink: vec3<f32>, alpha: f32, light: vec3<f32>) -> vec3<f32> {
    return light * alpha + ink * (1.0 - light);
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
/// The INK alone. What the name hides behind it is the hole
/// [`fs_glyph_gutter`] cuts, drawn just before this over the same run — a name
/// is one of the things the lattice puts in front of another and hides what it
/// stands over exactly as a node and a marker do (`node_paint` in lattice.wgsl),
/// but the hole is a shape of the RUN and the ink is a shape of the glyph, and
/// only the second of those belongs in a per-glyph draw.
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
    return vec4<f32>(wash_over(ink.rgb, ink.a, light.rgb), ink.a);
}

/// Every lattice name's coverage and strength, into the mask the distance field
/// is seeded from (field.wgsl).
///
/// Both channels are written under a MAX blend, so this is the union over every
/// glyph in the pane and the order the quads arrive in decides nothing. That is
/// what makes one field serve every name: the Shadow is cast from the nearest
/// ink anywhere on the pane, and a mask that composited would make it depend on
/// which letters happened to overlap.
///
/// The STRENGTH rides along in `g` because a shadow's level is the one at the
/// ink casting it. Read off a shared field, a fragment has no drawing glyph to
/// take it from — the nearest ink may belong to a name easing out while the
/// quad over it belongs to one at full strength — so the flood carries the
/// coordinate and both readers take the strength back at the seed.
@fragment
fn fs_glyph_ink(in: VertexOut) -> @location(0) vec2<f32> {
    return vec2<f32>(coverage(in, in.texel), clamp(in.rim.a, 0.0, 1.0));
}

/// The hole a run of names knocks out of what stands behind it: the Shadow's
/// own coverage off the shared field, cleared to the ground the picture around
/// it stands on.
///
/// Drawn as its own pass over the run BEFORE any of its ink, which is the order
/// [`fs_rim`] already carries and for the same reason — all of the hole, then
/// all of the text. A hole painted in the ink's own draw is protected only by
/// the alpha of the glyph drawing it, and two letters of one name stand well
/// inside each other's Shadow, so the later one paints ground over the earlier
/// one's letters.
///
/// The `(1 - ink)` is the UNION's and not one glyph's: no name's letters are
/// ever cleared, whether they belong to this run, to a run already drawn behind
/// it, or to one still to come. A Shadow holds LIGHT off; it is not a second
/// occluder, and a name that erases the name behind it is the artefact the
/// per-glyph hole shipped.
///
/// The strength lands on the COVERAGE here where it lands on the shade in the
/// glow pass, which is the difference `ring_shade` sets out and the same split
/// `node_clearing` and `glow_standoff` are two halves of: there is no depth
/// over the hole to put it the other side of.
@fragment
fn fs_glyph_gutter(@builtin(position) pos: vec4<f32>) -> @location(0) vec4<f32> {
    if locals.shadow <= 0.0 {
        discard;
    }
    let coord = vec2<i32>(pos.xy);
    let stand = field_standoff(coord);
    let here = textureLoad(field_ink, field_coord(coord), 0);
    let gutter = stand.x * stand.y * (1.0 - clamp(here.r, 0.0, 1.0));
    if gutter <= 0.0 {
        discard;
    }
    let light = glyph_light(coord);
    // What the hole is cleared TO: the finished light standing at this pixel
    // over the bare ground, which is `node_paint`'s own ground down to the
    // multiply — the same two targets under the same Meld, cut by the same
    // shade, over the same background. The glow is composited at the BOTTOM of
    // this pass, so a hole painting the bare ground would stamp the light out
    // exactly where the picture wants it.
    let shade = clamp(load_shade(coord), 0.0, 1.0);
    let lit = light * (1.0 - shade);
    let ground = lit.rgb + locals.background.rgb * (1.0 - lit.a);
    return vec4<f32>(ground * gutter, gutter);
}

/// The three attachments the lattice's glow pass carries: the light under its
/// two blends, and the standoff every emitter cuts into it. Named here as it is
/// in lattice.wgsl, because it is the same target.
struct GlowOut {
    @location(0) screened: vec4<f32>,
    @location(1) brightest: vec4<f32>,
    @location(2) shade: vec4<f32>,
}

// The standoff's own curve, written a second time.
//
// lattice.wgsl holds the first copy and the whole of the reasoning —
// `standoff_coverage`, `gap_shade` and the five constants under them. Neither
// module can call into the other (WGSL has no linkage across shader modules and
// there is no source composition in this tree), and the shape has to be
// IDENTICAL rather than merely similar: the Shadow is one bar across a node's
// rings, a marker's cross and a name, so two curves under one bar is the bar
// meaning two things. `the_names_shadow_is_the_rings_own_curve` reads both
// files and pins every constant here against the one it was copied from.
//
// What is NOT copied is where the numbers come from. There they are read off a
// node's own uv; here they arrive in `Locals` as shares of a node radius plus
// the points that radius draws as, because a glyph has no node under it.
const SHADOW_TAIL: f32 = 4.0;
const SHADOW_STOP: f32 = 2.0;
const SHADOW_SHAPE_RIND: f32 = 0.25;
const SHADOW_SHAPE_PLAIN: f32 = 1.0;
const SHADOW_SOFT_FLOOR: f32 = 0.02;
const SHADOW_KEEP_FLOOR: f32 = 0.0009765625;

/// The Shadow's outer handle in points, floored off zero as `clearing_edge`
/// floors it.
fn shadow_edge() -> f32 {
    return max(locals.shadow * locals.node_points, 0.001);
}

/// The width of its fade in points, under `standoff_soft`'s own floor.
fn shadow_soft() -> f32 {
    return max(locals.shadow_soft, SHADOW_SOFT_FLOOR) * locals.node_points;
}

/// How far out the standoff still has anything to say, in points: where
/// [`standoff_coverage`]'s own window has shut it.
///
/// What the knockout's box is grown by and how far the flood is run
/// (`FieldChain` in lib.rs), so the coverage is exactly zero where either stops
/// and the tail is never cut off in a screen-aligned line.
///
/// Free of the Shadow DEPTH, which is a factor on the LIGHT alone —
/// `gap_reach`'s rule in lattice.wgsl: the hole a name cuts is cut at every
/// depth, so what it is cut on has to reach at every depth. One chain serves
/// the shade beside it because the depth only ever SHUTS that, and a field run
/// too far costs passes rather than correctness.
fn shadow_stop() -> f32 {
    if locals.shadow <= 0.0 {
        return 0.0;
    }
    let edge = shadow_edge();
    let inner = clamp(edge - shadow_soft(), 0.0, edge - 0.001);
    return inner + SHADOW_STOP * (edge - inner);
}

/// How much of the light standing `sd` points out from a name's ink that name
/// holds off — `standoff_coverage` in lattice.wgsl, in points rather than in a
/// node's uv.
fn standoff_coverage(sd: f32) -> f32 {
    let edge = shadow_edge();
    let inner = clamp(edge - shadow_soft(), 0.0, edge - 0.001);
    let u = max(sd - inner, 0.0) / (edge - inner);
    let shape = SHADOW_SHAPE_RIND * pow(SHADOW_SHAPE_PLAIN / SHADOW_SHAPE_RIND,
        clamp(locals.shadow_shape, 0.0, 1.0));
    return exp(-SHADOW_TAIL * pow(u, shape)) * (1.0 - smoothstep(1.0, SHADOW_STOP, u));
}

/// What a standoff coverage of `cov` leaves of the light — `gap_shade` in
/// lattice.wgsl: the Shadow depth, spent geometrically over that coverage.
fn glyph_shade(cov: f32) -> f32 {
    let keep = max(1.0 - clamp(locals.shadow_depth, 0.0, 1.0), SHADOW_KEEP_FLOOR);
    return 1.0 - pow(keep, clamp(cov, 0.0, 1.0));
}

/// `coord` clamped into the field, which is the lattice's own rule for reading
/// a pane-sized target back (`light_coord` there, and [`glyph_light`] here):
/// the field is the scene target's size, so a fragment stands at one of its
/// texels and the clamp is what keeps a rounding at the last row from reading
/// outside it.
fn field_coord(coord: vec2<i32>) -> vec2<i32> {
    let edge = vec2<i32>(textureDimensions(field_tex)) - vec2<i32>(1, 1);
    return clamp(coord, vec2<i32>(0, 0), edge);
}

/// How much of the Shadow's own coverage stands at this pixel, and at what
/// strength: `(coverage, strength)`, off the jump flood's field.
///
/// The distance is to the nearest ink of ANY name on the pane, which for the
/// SHADE is exactly the answer a per-name dilation gives and not an
/// approximation of it: the shade is written under a `max` blend and
/// [`standoff_coverage`] falls monotonically, so the brightest of the profiles
/// is already the profile of the smallest distance. For the HOLE the union is
/// the stronger rule rather than an equal one — see [`fs_glyph_gutter`], where
/// cutting once off the union is what stops two holes compounding into a pit.
///
/// The INK, not a rim: the letters are what the eye reads a name by, exactly as
/// the arms are what it reads a cross by, and `plus_standoff` casts a marker's
/// field from the arms themselves.
///
/// Read TWICE and one function for the two, which is `standoff_coverage`'s own
/// arrangement in lattice.wgsl: [`fs_field_shade`] takes it as the shade laid
/// over the light, [`fs_glyph_gutter`] as the coverage of the hole a name
/// knocks out of what stands behind it. One shape for the two is the whole of
/// what makes the Shadow a single bar.
///
/// Flat in the Shadow's width and in the number of names, where a dilation
/// sampled at the fragment is quadratic in the first and linear in the second.
fn field_standoff(coord: vec2<i32>) -> vec2<f32> {
    let here = field_coord(coord);
    let own = textureLoad(field_ink, here, 0);
    // The fragment's OWN ink is the floor, at the coverage the sheet actually
    // holds there. Under [`INK_FLOOR`] a texel is no seed, so the flood says
    // nothing about it at all — and at a Shadow narrower than a texel the flood
    // reaches nowhere, which makes that edge the entire picture. A floor rather
    // than a term of its own: `standoff_coverage(0)` is 1, so wherever the
    // Shadow does reach it is the profile at a distance of nothing and the
    // comparison below carries it.
    var cov = clamp(own.r, 0.0, 1.0);
    var strength = own.g;
    let seed = textureLoad(field_tex, here, 0).xy;
    // No ink within the flood's reach. The chain runs only as far as
    // `shadow_stop` (see `steps` in field.rs), so this is also every pixel the
    // Shadow has nothing to say about.
    if seed.x != NO_SEED {
        let at = vec2<i32>(seed);
        let ink = textureLoad(field_ink, at, 0);
        // The seed's own coverage is spent on the DISTANCE, never on the
        // profile's height. A seed is a whole texel, and the contour the eye
        // reads the letter by crosses it: a texel covered `c` has its centre
        // `c - INK_FLOOR` INSIDE that contour, to within the straight-edge
        // approximation a rasterizer's own coverage already is. Subtracting it
        // puts the profile's origin on the letter's edge rather than on the
        // grid, which is a correction of at most half a texel.
        //
        // A HEIGHT scaled by that coverage is the one thing it must not be.
        // Coverage runs the whole of `[INK_FLOOR, 1]` along any contour not
        // parallel to the grid, so neighbouring seeds differ by up to a factor
        // of two — and each seed owns a wedge of the plane that widens with
        // distance, so the pair reads as bright and dark rays fanning out of
        // every curve, and as a hard seam wherever two strokes' wedges meet.
        // The correction here is bounded by half a texel of DISTANCE instead,
        // which is a fraction of a percent of the same ramp.
        let contour = clamp(ink.r, 0.0, 1.0) - INK_FLOOR;
        let sd = max(length(vec2<f32>(here - at)) - contour, 0.0);
        let dilated = standoff_coverage(sd / locals.pixels_per_point);
        // The deeper of the two wins, and takes its own strength with it: a
        // level belongs to the ink casting the shadow, and which ink that is is
        // exactly what this comparison decides.
        if dilated > cov {
            cov = dilated;
            strength = ink.g;
        }
    }
    return vec2<f32>(cov, strength);
}

/// field.wgsl's own sentinel, which has to be spelled twice for the same reason
/// the standoff's curve is: there is no linkage between shader modules here.
/// `a_names_field_and_its_flood_agree_on_no_seed` pins the pair.
const NO_SEED: u32 = 65535u;

/// The coverage field.wgsl seeds the flood at, spelled twice for [`NO_SEED`]'s
/// reason and pinned by the same test.
///
/// Here it is the contour a seed's coverage is measured FROM: the flood picks
/// the texels at or above it, so this is the coverage whose texel centre stands
/// on the letter's own edge, and the two numbers have to be the one number or
/// the correction in [`field_standoff`] is taken about a contour the flood does
/// not seed at.
const INK_FLOOR: f32 = 0.5;

/// The shadow every lattice name holds the light off by, laid into the glow
/// pass across the whole pane at once.
///
/// A name paints no rim. What a cross keeps a halo off itself with is a shape
/// in the LIGHT rather than one painted on the ink (`plus_standoff`), and a
/// lattice where the crosses stand in the light and the names carry a black
/// halo of their own reads as two pictures laid over each other. The halo is
/// cast here instead, on the Shadow bars every other emitter is held off on:
/// dark where there is light to hold off, and nothing at all where there is
/// none — which a painted halo cannot be, standing at full strength over a
/// ground no light ever reached.
///
/// Closed by the name's own STRENGTH, the one number the rim colour carries
/// here (`LABEL_SHADOW` in `harmonigraph_ui`): a name easing in as the marker
/// under it eases out grows its shadow on the same clock its ink arrives on, so
/// a position handing itself between the two is never twice shadowed nor
/// briefly unshadowed.
///
/// The strength lands on the SHADE and not on the coverage, which is
/// `ring_shade`'s reading and for its reason: the coverage is an exponent, so a
/// level spent there is a number of stops rather than a share of the light, and
/// a name a tenth of the way out would still hold off half of what it held off
/// whole.
///
/// ONE full-screen quad for every name on the pane, where a node's rings and a
/// marker's cross each write their own billboard. The field already holds the
/// nearest ink's distance and strength at every pixel, so there is nothing per
/// name left to draw — and the `max` blend this writes under makes the two
/// identical rather than merely close, a max of profiles over one shared
/// minimum distance being the profile of that distance.
@fragment
fn fs_field_shade(@builtin(position) pos: vec4<f32>) -> GlowOut {
    if locals.shadow_depth <= 0.0 {
        discard;
    }
    let stand = field_standoff(vec2<i32>(pos.xy));
    let shade = glyph_shade(stand.x) * stand.y;
    if shade <= 0.0 {
        discard;
    }
    // Nothing into the two light attachments: a name is ink standing in the
    // light and never a source of any, which is also what keeps it out of the
    // bloom (`a_label_adds_no_light_through_the_bloom`).
    let dark = vec4<f32>(0.0);
    return GlowOut(dark, dark, vec4<f32>(shade, 0.0, 0.0, 0.0));
}
