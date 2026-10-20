// The piano roll's notes: one instanced quad per note segment — a solid
// rectangle with a flat color, wrapped on every side by an outline that
// fades out, both falling out of a signed distance field. A segment may also
// end in a fade at its LEADING tip (`lead_coverage`), which takes both layers
// out together, and carry its own outline cap where the note inside it stops
// (`cap_coverage`).
//
// TWO LAYERS, drawn as two passes over the same instances rather than
// composited per note: every note's outline, then every note's body
// (`fs_core_*`), and a note's opacity (its `fade`) takes the two out together.
// The same instances drawn twice, each layer through a quad as big as its own
// ink: the outline's grown by the shadow's reach (`vs_note`), the body's by one
// feather (`vs_note_body`).
//
// The bodies use ordinary over. The outlines do not: each note's glow goes into
// a mask over the roll's rect (`fs_outline_mask`), where a depth test keeps
// whichever note's glow is strongest — its strength AND its color — and the
// mask is laid over the picture once. Where two notes' glows meet, the picture
// there is as covered as the stronger of the two and no more, rather than one
// glow tinting the other.
//
// The order is the whole point. The outline is at its darkest where it meets
// its own note — as strong as the Shadow darkness makes it — so an outline
// composited with its own note lands on the NEIGHBOURING notes it reaches
// into, and along time those neighbours are the next note: repeats of one key
// butt together there, and the later one blanked the tail of the earlier.
// Under every body instead, an outline covers the backdrop before the note
// paints its color.
//
// A body drawn at less than full opacity would let that shadow through, so
// every outline is also HELD OUT wherever any body covers (#1293): `prepare`
// draws the union of the bodies' coverage into `body_holdout` first, and the
// outline layer takes it off ([`body_held`]). One note's glow covers only
// the picture behind the ribbons, never another ribbon, at any opacity. That
// includes a note's own shadow under its own body, so a translucent ribbon
// shows the picture behind it unshadowed. Outside every body the holdout is 0
// and the outline is what it was.
//
// What that costs is the seam between two notes that TOUCH: same key, no gap,
// and the bodies now meet directly in one color where the outline used to
// stand between them. A gap of a point or more still reads as two notes, since
// the outline fills it.
//
// The outline is a glow read straight off the note's distance — solid out to
// the style's spread, then a Gaussian in the distance — so it is as strong
// against a hairline ribbon as against a wide one. Its color and how strongly
// it covers are the pane's, not decisions made here: this shader is told how
// far it reaches, how gradually it goes, how strong it is and what color, and
// invents none of the four.
//
// The glow never covers wholly, so over bright spectrogram energy the backdrop
// shows through it. The BACKDROP CUT (#1525) takes the backdrop away faster
// than the glow lays its color on: read off the same distance and held out by
// the same bodies ([`cut_strength`]), combined across notes into a cut `c`, and
// composited with the mask so the picture keeps `1 - max(s, c)` of what was
// there ([`fs_glow_cut_gamma`]). Notes' cuts combine by a smooth minimum of
// their distances (the halo blend, [`fs_cut_blend`]), since the strongest-wins
// minimum folds into a crease between two notes once the cut is wide and soft.
// That takes two more passes over the notes into two targets over the roll's
// rect, both skipped while the cut is off, when the plain glow composite runs.
//
// Nothing here is tessellated. A note is four vertices whatever its shape,
// and the outline the egui path drew as a separate stroked rounded rect is
// read off the distance instead, which costs a compare rather than a second
// shape.
//
// Coordinates arrive in egui POINTS, exactly as egui's own vertex shader
// takes them, and `vs_note` does the same screen->clip mapping. The pane's
// orientation lives entirely in `pitch_dir` / `depth_dir`, so this shader
// never names a screen side either.

struct Locals {
    /// The glow's σ, strength, solid spread and whole reach, in points except
    /// the strength (0..1). First, so the vec4 is aligned with no padding.
    shadow: vec4<f32>,
    /// The backdrop cut (#1525): how much of the picture it takes at a note's
    /// edge (0 when off), how far past the spread its fade runs in points, the
    /// fade's curve (0 straight, 1 smoothstep), and the halo blend's softness
    /// as a share of that fade (0 for the strongest note's cut alone). See
    /// [`cut_coverage`] and [`fs_glow_cut_gamma`].
    cut: vec4<f32>,
    /// Where the viewport being drawn into starts, in egui points, and how big
    /// it is. The draw into the egui pass takes the whole surface (origin 0);
    /// the bloom's own pass takes the roll's rect alone, so the same
    /// instances — which are in surface points either way — land in a texture
    /// that covers only the roll.
    origin_points: vec2<f32>,
    viewport_points: vec2<f32>,
    /// Width of the antialiasing ramp in points — one pixel of whatever is
    /// being drawn into, which is not the display's pixel in the bloom's pass.
    feather: f32,
    _feather_pad: f32,
    /// Unit screen vectors of the pane's two axes. Pitch runs across the
    /// pane's short side, depth (time) along its long side.
    pitch_dir: vec2<f32>,
    depth_dir: vec2<f32>,
    /// Where the target being drawn into starts, in its own pixels, and how
    /// many points one of them measures, per axis. What takes a fragment's own
    /// pixel back to the surface point it stands on ([`surface_point`]); set
    /// with the viewport above by `RollUniforms::drawn_into`.
    origin_pixels: vec2<f32>,
    pixel_points: vec2<f32>,
    /// How much of what the cut takes away is filled with the glow's own
    /// color, 0..1, rather than left black.
    cut_fill: f32,
    /// Rounds the block up to its vec4's alignment, spelled out to match
    /// the Rust side.
    _tail_pad: f32,
};

@group(0) @binding(0) var<uniform> locals: Locals;

/// The union of every body's coverage, one texel per device pixel of the
/// roll's rect, read only by the outline layer's mask. Binding 2 keeps it clear of
/// the common module's glow slots in the same group.
@group(1) @binding(2) var body_holdout: texture_2d<f32>;

struct VertexOut {
    @builtin(position) position: vec4<f32>,
    /// Offset from the note's center in points, along (pitch, depth).
    ///
    /// Every pass REPLACES it with the offset of the fragment's own pixel
    /// center ([`on_screen`]) before reading it: interpolated,
    /// it moved with where the rasterizer's sub-pixel grid snapped the quad's
    /// corners, so a layer's edges shifted by a fraction of a level whenever its
    /// quad changed size, and a tremolo note's hard step at its leading end
    /// flipped a whole tone.
    @location(0) local: vec2<f32>,
    /// Half extents of the note's solid body, same two axes.
    @location(1) @interpolate(flat) half_extent: vec2<f32>,
    /// The center line's pitch drift per point of depth: 0 for a held note,
    /// non-zero for a glide, which shears the box into a parallelogram.
    @location(2) @interpolate(flat) shear: f32,
    /// How far the outline reaches past the note's edge, in points, and 0 when
    /// the outline is off: the style's whole reach (`locals.shadow.w`). It
    /// wraps every side — see [`outline_coverage`].
    @location(3) @interpolate(flat) outline_reach: f32,
    /// Four to a slot, since a stage passes sixteen at most:
    /// - `x`: how much of the box's LEADING end is a lead rather than the note
    ///   itself, in points — 0 for the ordinary segment that is all note. See
    ///   [`lead_coverage`].
    /// - `y`: how much of that lead is spent fading out at the tip, in points.
    /// - `z`: how much of the lead is still standing, 0..1.
    /// - `w`: how far the outline's cap at the NOTE's own leading end reaches,
    ///   in points. See [`cap_coverage`].
    @location(4) @interpolate(flat) lead: vec4<f32>,
    /// Four depth offsets, ascending, and the ribbon's width at each as a share
    /// of its full `half_extent.x`. See [`taper_at`].
    @location(5) @interpolate(flat) taper_depth: vec4<f32>,
    @location(6) @interpolate(flat) taper: vec4<f32>,
    /// Dense-repeat phase (composition points) and point scale; zero disables.
    @location(7) @interpolate(flat) tremolo: vec2<f32>,
    /// Premultiplied, gamma-space, exactly as egui carries `Color32`.
    @location(8) @interpolate(flat) core: vec4<f32>,
    /// The outline's glow: its color, straight gamma-space RGB, and in `a` how
    /// strongly it covers at full coverage; the fade takes it from there.
    @location(9) @interpolate(flat) aura: vec4<f32>,
    /// The note's center in surface points, which [`on_screen`] measures
    /// `local` from.
    @location(10) @interpolate(flat) center: vec2<f32>,
    /// Width of one coverage sample in points: one pixel of the target being
    /// drawn into.
    @location(12) @interpolate(flat) feather: f32,
    /// The two depth offsets the readings below are given at. See [`along`].
    @location(13) @interpolate(flat) ramp: vec2<f32>,
    /// Opacity at those two depths.
    @location(14) @interpolate(flat) fade: vec2<f32>,
};

/// The outline layer's quad: the note's box grown by everything
/// `outline_strength` can reach to.
///
/// The outline wraps the note, so it is owed room on BOTH axes: ink runs out
/// wherever the box distance passes `outline_reach`, which is the shadow's
/// reach past every edge and every corner, and half a feather past that. While
/// the cut fills with the glow's color the mask names a color as far as the cut
/// reaches too ([`fs_outline_mask`]), so the quad is grown by the larger.
@vertex
fn vs_note(
    @builtin(vertex_index) vertex: u32,
    @location(0) center: vec2<f32>,
    @location(1) half_extent: vec2<f32>,
    @location(2) shear: f32,
    // Lead, lead fade, lead alpha and cap reach; span then ramp.
    // Packed, since a vertex takes sixteen at most.
    @location(4) lead: vec4<f32>,
    @location(8) core: vec4<f32>,
    @location(9) aura: vec4<f32>,
    @location(14) span_ramp: vec4<f32>,
    @location(15) fade: vec2<f32>,
    @location(5) taper_depth: vec4<f32>,
    @location(6) taper: vec4<f32>,
    @location(7) tremolo: vec2<f32>,
) -> VertexOut {
    let reach = select(locals.shadow.w, max(locals.shadow.w, cut_reach()), cut_fills());
    return note_vertex(
        vertex, center, half_extent, shear, lead, core, aura, span_ramp, fade,
        taper_depth, taper, tremolo, reach + locals.feather,
    );
}

/// The cut's quad, for the two passes that read the cut alone
/// ([`fs_cut_reference`], [`fs_cut_blend`]): the note's box grown by the cut's
/// own reach, which may be longer or shorter than the glow's.
@vertex
fn vs_note_cut(
    @builtin(vertex_index) vertex: u32,
    @location(0) center: vec2<f32>,
    @location(1) half_extent: vec2<f32>,
    @location(2) shear: f32,
    @location(4) lead: vec4<f32>,
    @location(8) core: vec4<f32>,
    @location(9) aura: vec4<f32>,
    @location(14) span_ramp: vec4<f32>,
    @location(15) fade: vec2<f32>,
    @location(5) taper_depth: vec4<f32>,
    @location(6) taper: vec4<f32>,
    @location(7) tremolo: vec2<f32>,
) -> VertexOut {
    return note_vertex(
        vertex, center, half_extent, shear, lead, core, aura, span_ramp, fade,
        taper_depth, taper, tremolo, cut_reach() + locals.feather,
    );
}

/// The BODY layers' quad — the core layer, the body holdout and the bloom's
/// notes: the note's box grown by one feather. A body's ink is
/// `inside(in, d, 0.0)`, which is 0 from half a feather past its edge, so the
/// shadow's reach would only be fragments that compute 0. At long spans that
/// reach is most of the quad: a note a couple of points across drawn through a
/// box some fifteen points wider.
@vertex
fn vs_note_body(
    @builtin(vertex_index) vertex: u32,
    @location(0) center: vec2<f32>,
    @location(1) half_extent: vec2<f32>,
    @location(2) shear: f32,
    @location(4) lead: vec4<f32>,
    @location(8) core: vec4<f32>,
    @location(9) aura: vec4<f32>,
    @location(14) span_ramp: vec4<f32>,
    @location(15) fade: vec2<f32>,
    @location(5) taper_depth: vec4<f32>,
    @location(6) taper: vec4<f32>,
    @location(7) tremolo: vec2<f32>,
) -> VertexOut {
    return note_vertex(
        vertex, center, half_extent, shear, lead, core, aura, span_ramp, fade,
        taper_depth, taper, tremolo, locals.feather,
    );
}

/// One corner of a note's quad: its bounding box grown by `margin` points on
/// both axes, which is what the layer drawing it can reach to.
fn note_vertex(
    vertex: u32,
    center: vec2<f32>,
    half_extent: vec2<f32>,
    shear: f32,
    lead: vec4<f32>,
    core: vec4<f32>,
    aura: vec4<f32>,
    span_ramp: vec4<f32>,
    fade: vec2<f32>,
    taper_depth: vec4<f32>,
    taper: vec4<f32>,
    tremolo: vec2<f32>,
    margin: f32,
) -> VertexOut {
    // Triangle-strip corners: (-1,-1) (1,-1) (-1,1) (1,1).
    let corner = vec2<f32>(
        select(-1.0, 1.0, (vertex & 1u) == 1u),
        select(-1.0, 1.0, (vertex & 2u) == 2u),
    );

    let slope = shear;
    // A shortfall in the margin CLIPS ink rather than costing a little fill
    // rate, so each caller passes the exact reach of its own layer.
    //
    // The distance is Euclidean (see [`box_distance_trimmed`]), so the grown
    // box is the note's own bounding box — its ribbon's half width plus the
    // center line's drift over the note's half-length — with the same margin
    // on both axes, however steep the glide.
    let extent = vec2<f32>(
        half_extent.x + abs(slope) * half_extent.y + margin,
        half_extent.y + margin,
    );

    // Cut along depth to this instance's own span of the box: a piece of a
    // segment draws its stretch and no more, and neighbouring pieces share the
    // cut, so each pixel is drawn once. A whole box's span reaches past both
    // ends and cuts nothing.
    var local = corner * extent;
    let span = span_ramp.xy;
    local.y = select(max(-extent.y, span.x), min(extent.y, span.y), corner.y > 0.0);
    let pos = center + locals.pitch_dir * local.x + locals.depth_dir * local.y;

    let in_viewport = pos - locals.origin_points;
    var out: VertexOut;
    out.position = vec4<f32>(
        2.0 * in_viewport.x / locals.viewport_points.x - 1.0,
        1.0 - 2.0 * in_viewport.y / locals.viewport_points.y,
        0.0,
        1.0,
    );
    out.local = local;
    out.half_extent = half_extent;
    out.shear = shear;
    out.outline_reach = locals.shadow.w;
    out.lead = lead;
    out.taper_depth = taper_depth;
    out.taper = taper;
    out.tremolo = tremolo;
    out.core = core;
    out.aura = aura;
    out.center = center;
    out.feather = locals.feather;
    out.ramp = span_ramp.zw;
    out.fade = fade;
    return out;
}

/// The surface point, in points, at this fragment's own pixel center: the
/// inverse of `note_vertex`'s mapping, read off the rasterizer's exact pixel
/// position rather than interpolated across the quad.
///
/// Summed in pixels and scaled once. A pixel center and a whole-pixel origin
/// add exactly in f32, so two targets over the same surface pixel — the
/// surface and the body holdout, each scaled by exactly `1 / ppp` — reach the
/// same bits, at fractional pixels-per-point too.
fn surface_point(in: VertexOut) -> vec2<f32> {
    return (locals.origin_pixels + in.position.xy) * locals.pixel_points;
}

/// `in`, with `local` measured from the note's center to this fragment's own
/// pixel center rather than interpolated from the quad's corners.
///
/// So what a pixel draws depends on the note and the pixel alone, and not on
/// how big a quad the layer drew it through: the body's quad is a feather wider
/// than the body and the outline's a shadow's reach wider, and interpolated
/// across those two, one pixel read two slightly different coordinates. Every
/// pass onto the surface or a texture over the roll goes through this, so the
/// body, its holdout and its outline agree to the bit on where the note is
/// ([`surface_point`]); the bloom's half-size notes read their own coarser
/// pixels the same way. The axes are unit and perpendicular, so projecting onto
/// them inverts `center + pitch_dir * local.x + depth_dir * local.y` exactly.
fn on_screen(in: VertexOut) -> VertexOut {
    var out = in;
    let delta = surface_point(in) - in.center;
    out.local = vec2<f32>(dot(delta, locals.pitch_dir), dot(delta, locals.depth_dir));
    return out;
}

/// Coverage of everything on the near side of `edge`: how much of a
/// one-pixel-wide window centered on the signed distance `d` lands inside it.
///
/// A box filter along the distance gradient, which is exact for a straight
/// edge and is what makes a shape thinner than a pixel come out FAINTER rather
/// than snapping to a full pixel — the same bargain epaint's feathering makes,
/// so a hairline ribbon reads the way it does through the tessellator.
fn inside(in: VertexOut, d: f32, edge: f32) -> f32 {
    let f = max(in.feather, 1e-6);
    return clamp((edge - d) / f + 0.5, 0.0, 1.0);
}

/// How strongly the glow covers `d` points off the note's edge: the style's
/// strength out to its spread, then a Gaussian of σ in the distance past it,
/// lowered by its own value at the style's reach so it lands on exactly 0
/// there rather than stepping.
///
/// Read off the distance rather than a blur of the note's silhouette, which is
/// the point of it: a blur of a ribbon thinner than σ never reaches full
/// coverage, so the surround all but vanished round exactly the hairline notes
/// that need one, and spread into a wide smear round a fat one. Here a note's
/// edge is equally held at every ribbon width.
///
/// `reach` may still shorten an interior cap when the pane has less room
/// before its now-line. That bound stays hard — shortening a cap moves its end
/// instead of dimming the whole profile — while the profile's own shape is the
/// style's whatever bound it is cut at.
fn outline_coverage(in: VertexOut, d: f32, reach: f32) -> f32 {
    let sigma = locals.shadow.x;
    if reach <= 0.0 || locals.shadow.y <= 0.0 || sigma <= 0.0 {
        return 0.0;
    }
    let spread = locals.shadow.z;
    let x = max(d - spread, 0.0) / sigma;
    let end = (locals.shadow.w - spread) / sigma;
    let tail = exp(-0.5 * end * end);
    let profile = max(exp(-0.5 * x * x) - tail, 0.0) / max(1.0 - tail, 1e-6);
    return locals.shadow.y * profile * inside(in, d, reach);
}

/// The backdrop cut's whole reach past a note's edge, in points: the style's
/// spread, then the cut's own fade. 0 while the cut is off.
fn cut_reach() -> f32 {
    return select(0.0, locals.shadow.z + locals.cut.y, locals.cut.x > 0.0);
}

/// Whether what the cut takes away is filled with the glow's color, which is
/// when the glow mask has to name a color as far as the cut reaches.
fn cut_fills() -> bool {
    return locals.cut.x > 0.0 && locals.cut_fill > 0.0;
}

/// The cut's fade at `u`, the distance past the spread as a share of the fade:
/// whole at 0 and nothing at 1, along a straight line at a curve of 0 and a
/// smoothstep at 1, and a mix of the two between.
fn cut_ramp(u: f32) -> f32 {
    let x = clamp(u, 0.0, 1.0);
    return 1.0 - mix(x, x * x * (3.0 - 2.0 * x), locals.cut.z);
}

/// The `u` at which [`cut_ramp`] reads `y`: how far out a bare note's cut is as
/// strong as `y` is. What lets a cut taken down by its note's fade, a body or
/// a lead join the halo blend as a distance ([`fs_cut_reference`]), so the
/// blend of one note alone is that note's cut exactly.
///
/// Exact at the two ends of the curve — the identity, and smoothstep's own
/// inverse — and Newton from the mix of the two between, where the fade's
/// slope is at least `1 - curve` and the steps settle in a few.
fn cut_unramp(y: f32) -> f32 {
    let g = clamp(1.0 - y, 0.0, 1.0);
    let c = locals.cut.z;
    let eased = 0.5 - sin(asin(1.0 - 2.0 * g) / 3.0);
    var u = mix(g, eased, c);
    if c > 0.0 && c < 1.0 {
        for (var i = 0; i < 4; i++) {
            let miss = mix(u, u * u * (3.0 - 2.0 * u), c) - g;
            let slope = mix(1.0, 6.0 * u * (1.0 - u), c);
            u = clamp(u - miss / max(slope, 1e-3), 0.0, 1.0);
        }
    }
    return u;
}

/// How much of the picture the cut takes `d` points off the note's edge, as a
/// share of its amount: whole out to the style's spread, as the glow is, then
/// [`cut_ramp`] over the cut's own fade, and nothing from `reach` on.
///
/// A distance profile like [`outline_coverage`] and for its reason, so a
/// hairline ribbon is cut round as firmly as a wide one. The amount itself is
/// laid on in the composite ([`fs_glow_cut_gamma`]), after the notes' cuts have
/// been combined.
fn cut_coverage(in: VertexOut, d: f32, reach: f32) -> f32 {
    let fade = locals.cut.y;
    if reach <= 0.0 || fade <= 0.0 {
        return 0.0;
    }
    return cut_ramp(max(d - locals.shadow.z, 0.0) / fade) * inside(in, d, reach);
}

/// How much of the ribbon survives at this fragment: 1 across the NOTE, and
/// inside the lead at its leading end, whatever the lead's own fade and opacity
/// have left there.
///
/// The leading end is `-half_extent.y` along the depth axis, always — depth
/// runs away from whatever the roll is drawn beside, so this names an end of
/// the pane's own axis and not a screen side, exactly as everything else here
/// does. `u` counts INWARD from that tip, so `u >= lead` is the note itself.
///
/// The note is untouched, and that division is the whole reason `lead` is
/// carried per instance. A lead on its way out is a translucent extension of a
/// solid ribbon; taking the opacity across the whole box instead would fade the
/// note along with the thing hanging off it, which is a note the picture claims
/// is quieter than it is.
///
/// Applied to BOTH layers, which is what makes a fading tip a fading ribbon
/// rather than a ribbon dissolving inside its own outline: the surround wraps
/// the ends as much as the flanks, so a body taken out on its own would leave a
/// hard cap of glow standing where the ink went. Past the tip the ramp is
/// negative and clamps to 0, so that cap is gone rather than merely faint.
///
/// A lead with NO fade keeps its cap, and the branch that gives it one is not a
/// special case dodged. Whether the tip fades is the one real distinction the
/// pane's bar draws with its two handles closed against apart, and the two ends
/// want opposite things: a tip that dissolves has nothing left for an outline to
/// bound, while a tip that ends square is an edge like any other on the note and
/// is owed the same surround the flanks get. So a fade of 0 is uniform right out
/// to the box, and [`inside`] is what ends it — the same square end the ribbon
/// would have without a lead at all, dimmed by the opacity. (The spectral pane
/// clips that cap off, its budget past the now-line being the lead itself; a
/// caller who wants the cap has only to leave it room.)
///
/// The fade is floored at one pixel and CAPPED at the lead, [`outline_coverage`]'s
/// bargain in both directions and for its reasons: a sub-pixel ramp is an
/// aliased edge, and a fade wider than the thing it is taking out has nothing
/// past that to take. Capped, a fade dialled past its reach fades the whole
/// lead from the note's own end outward rather than stepping at it — which is
/// what this crate's `lead_fade` promises, and what lets the pane's bar clamp
/// the pair for the bar's sake alone.
///
/// The junction at the note's own end is CROSSED over a pixel rather than
/// stepped, and that is the one place the two coverages meet. They differ by
/// however much opacity the lead has lost, so at full opacity there is nothing
/// to cross and at none of it the step would be the ribbon's whole edge — drawn
/// hard, with no antialiasing of its own, since the box's own edge is a lead
/// away and [`inside`] is not looking here.
///
/// A segment with no lead leaves at the first line, so the ordinary ribbon pays
/// nothing at all for a lead it does not have.
fn lead_coverage(in: VertexOut) -> f32 {
    let lead = in.lead.x;
    let lead_fade = in.lead.y;
    let lead_alpha = in.lead.z;
    if (lead <= 0.0) {
        return 1.0;
    }
    let f = max(in.feather, 1e-6);
    let u = in.local.y + in.half_extent.y;
    // The lead's own coverage: its opacity, taken out over the fade at the tip.
    var led = lead_alpha;
    if (lead_fade > 0.0) {
        led = lead_alpha * clamp(u / max(min(lead_fade, lead), f), 0.0, 1.0);
    }
    // ...and the note's, which is solid. 1 inside the note, 0 in the lead, a
    // pixel wide in between.
    let note = clamp((u - lead) / f + 0.5, 0.0, 1.0);
    return mix(led, 1.0, note);
}

/// Signed distance from this fragment to the note's own box, in points:
/// negative inside it, positive outside, and Euclidean — the true distance to
/// the nearest point of the box, whichever edge or corner that is. On a glide
/// that is perpendicular to the long edges, so the rim bands keep their
/// thickness instead of thinning with its angle.
fn box_distance(in: VertexOut) -> f32 {
    return box_distance_trimmed(in, 0.0);
}

/// The same distance, to the box with its LEADING end pulled in by `trim`
/// points. `trim` of the lead is the distance to the NOTE inside a box that
/// carries one; 0 is the box itself.
///
/// Pulling one end in shortens the box by `trim` and slides its center half
/// that far along the note's center LINE — along depth, and `slope` times that
/// along pitch — so a sheared box keeps its long edges where they were.
///
/// A ribbon whose width holds still along this instance is the parallelogram,
/// at that width; one whose width moves is [`tapered_distance`]. Both are
/// exact, so an unmoving note draws exactly as it did before widths moved.
fn box_distance_trimmed(in: VertexOut, trim: f32) -> f32 {
    let t = in.taper;
    if (all(t == vec4<f32>(t.x))) {
        return parallelogram_distance(in, trim, in.half_extent.x * t.x);
    }
    return tapered_distance(in, trim);
}

/// [`box_distance_trimmed`] for a ribbon `half_pitch` either side of its
/// center line all along.
fn parallelogram_distance(in: VertexOut, trim: f32, half_pitch: f32) -> f32 {
    let slope = in.shear;
    // A bent note is a sheared box: its long edges run at `slope`, its ends
    // stay square across the depth axis, `half_extent.x` either side of the
    // center line along pitch. That is a parallelogram, and this is its exact
    // distance (Inigo Quilez's `sdParallelogram`), in (pitch, depth).
    //
    // Exact rather than the box distance in sheared coordinates with the
    // across term divided by the shear's length. That shortcut is right beside
    // the long edges and wrong past the ends, where it reads the depth offset
    // alone: on a steep glide the region it calls near runs `slope` times the
    // outline's reach along pitch. Per-note tuning routinely makes a segment a
    // hundredth of a point long across part of a semitone — a slope in the
    // thousands — and that segment's outline and antialiasing ramp became a
    // hairline strip through the whole pane.
    //
    // Square corners, always: a note is a rectangle in the pane's two axes,
    // and rounding one was a setting until it turned out to be doing nothing a
    // piano roll wants — on the notes short enough for it to show (a tapped
    // key), the radius clamps to the note's own half-length and turns it into
    // a bead. The outline's own corners are round, being a constant distance
    // from a square one, and that is the shape a note wants wrapped around it.
    let half_along = in.half_extent.y - 0.5 * trim;
    // The center line's far end, from the center.
    let end = vec2<f32>(slope * half_along, half_along);
    var p = in.local - 0.5 * trim * vec2<f32>(slope, 1.0);
    // The box is point-symmetric: fold onto the far end's half.
    p = select(p, -p, p.y < 0.0);
    // Nearest point on that end, and whether `p` is short of it.
    var w = p - end;
    w.x -= clamp(w.x, -half_pitch, half_pitch);
    var near = dot(w, w);
    var within = -w.y;
    // Nearest point on the long edge on `p`'s side, and whether `p` is inside
    // it — the sign alone is read, the magnitude being scaled by `half_along`.
    let side = p.x * end.y - p.y * end.x;
    p = select(p, -p, side < 0.0);
    var v = p - vec2<f32>(half_pitch, 0.0);
    v -= end * clamp(dot(v, end) / max(dot(end, end), 1e-12), -1.0, 1.0);
    near = min(near, dot(v, v));
    within = min(within, half_pitch * half_along - abs(side));
    return select(sqrt(near), -sqrt(near), within > 0.0);
}

/// The ribbon's half width at depth `y`, as a share of the full
/// `half_extent.x`: straight lines through the four taper points, held past
/// both ends. Two points at one depth are a step, read at its newer value on
/// the far side.
///
/// The middle two points are this piece's own ends, and the outer two are
/// its NEIGHBOURS' far ends (a step's other side, where one stands between),
/// so near a cut this piece measures the shape the piece beside it draws
/// rather than its own flank run on. What it cannot see is a piece beyond
/// those: a neighbour shorter than the outline's reach lets the next one's
/// flank be nearer than this piece knows, and the outline steps by that much
/// at the cut. Douglas–Peucker keeps pieces short only where the width bends
/// hard, which is where that difference is smallest.
fn taper_at(in: VertexOut, y: f32) -> f32 {
    let d = in.taper_depth;
    let w = in.taper;
    if (y < d.y) {
        return mix(w.x, w.y, clamp((y - d.x) / max(d.y - d.x, 1e-6), 0.0, 1.0));
    }
    if (y < d.z) {
        return mix(w.y, w.z, clamp((y - d.y) / max(d.z - d.y, 1e-6), 0.0, 1.0));
    }
    return mix(w.z, w.w, clamp((y - d.z) / max(d.w - d.z, 1e-6), 0.0, 1.0));
}

/// Squared distance from `p` to the segment `a`..`b`.
fn segment_distance2(p: vec2<f32>, a: vec2<f32>, b: vec2<f32>) -> f32 {
    let pa = p - a;
    let ba = b - a;
    let h = clamp(dot(pa, ba) / max(dot(ba, ba), 1e-12), 0.0, 1.0);
    let q = pa - ba * h;
    return dot(q, q);
}

/// Squared distance from `p` to both flanks between two depths: `a` and `b`
/// are each a depth and the half width there, in points, either side of a
/// center line drifting `slope` along pitch per point of depth.
fn flanks_distance2(p: vec2<f32>, slope: f32, a: vec2<f32>, b: vec2<f32>) -> f32 {
    let near = vec2<f32>(slope * a.x, a.x);
    let far = vec2<f32>(slope * b.x, b.x);
    let across_a = vec2<f32>(a.y, 0.0);
    let across_b = vec2<f32>(b.y, 0.0);
    return min(
        segment_distance2(p, near + across_a, far + across_b),
        segment_distance2(p, near - across_a, far - across_b),
    );
}

/// One taper point, as a depth and a half width in points, pulled onto the
/// box between `lo` and `hi`. A point past either end stands at that end at
/// the width the ribbon has there; a point on the box keeps its own width, so
/// a step keeps both its sides.
fn taper_vertex(in: VertexOut, depth: f32, width: f32, lo: f32, hi: f32) -> vec2<f32> {
    let y = clamp(depth, lo, hi);
    let share = select(taper_at(in, y), width, depth >= lo && depth <= hi);
    return vec2<f32>(y, in.half_extent.x * share);
}

/// [`box_distance_trimmed`] for a ribbon whose width moves along the piece:
/// the exact distance to the polygon its two flanks bound, straight between
/// the taper points and square across depth at the box's ends. It is still
/// the TRUE distance (#1118), so a sliver glide's outline stays within its
/// reach along pitch here as well.
fn tapered_distance(in: VertexOut, trim: f32) -> f32 {
    let slope = in.shear;
    let lo = -in.half_extent.y + trim;
    let hi = in.half_extent.y;
    let p = in.local;
    let d = in.taper_depth;
    let w = in.taper;
    let start = vec2<f32>(lo, in.half_extent.x * taper_at(in, lo));
    let v0 = taper_vertex(in, d.x, w.x, lo, hi);
    let v1 = taper_vertex(in, d.y, w.y, lo, hi);
    let v2 = taper_vertex(in, d.z, w.z, lo, hi);
    let v3 = taper_vertex(in, d.w, w.w, lo, hi);
    let end = vec2<f32>(hi, in.half_extent.x * taper_at(in, hi));
    var near = min(
        segment_distance2(p, vec2<f32>(slope * lo - start.y, lo), vec2<f32>(slope * lo + start.y, lo)),
        segment_distance2(p, vec2<f32>(slope * hi - end.y, hi), vec2<f32>(slope * hi + end.y, hi)),
    );
    near = min(near, flanks_distance2(p, slope, start, v0));
    near = min(near, flanks_distance2(p, slope, v0, v1));
    near = min(near, flanks_distance2(p, slope, v1, v2));
    near = min(near, flanks_distance2(p, slope, v2, v3));
    near = min(near, flanks_distance2(p, slope, v3, end));
    let inside = p.y > lo && p.y < hi
        && abs(p.x - slope * p.y) < in.half_extent.x * taper_at(in, p.y);
    return select(sqrt(near), -sqrt(near), inside);
}

/// How much of the outline's cap at the NOTE's own leading end is painted
/// here: the same surround every other edge gets, standing against the end of
/// the note inside a box that carries a lead.
///
/// Without it that end wears no cap at all. It is INTERIOR to the box — the
/// lead was added to the box's length, not drawn beside it — and
/// [`outline_strength`]'s mask keeps the outline out of a box's middle, correctly,
/// since a box has no edge there. So the cap would arrive only when the caller
/// drops the lead and the box shrinks back to the note.
///
/// Drawn in the outline layer, under every body (see the head of this file),
/// so a lead standing over the cap covers it and a spent one leaves it bare.
/// The spectral pane gives a cap room only behind a spent lead (`lead.z` of
/// 0): the box it keeps on a released note while that note's end scrolls
/// clear of the now-line.
///
/// `cap_reach` is the one thing the caller decides, and it is about ROOM rather
/// than about time — the cap stands in the stretch the lead was drawn over, and
/// only the caller knows whether its ink is welcome there. Shortened, the cap
/// grows out of the note's end rather than fading in over it, so it is wholly
/// on the near side of whatever the caller is protecting at every reach it is
/// given, and reaching its full `outline_reach` is the same picture the box's
/// own outline draws once the lead is dropped.
///
/// Unioned with the wrap rather than added to it: the two are the same color
/// off two shapes that share three of their sides, and along the note's flanks
/// both are looking at the same ink. Added, that overlap comes out darker than
/// the outline's own color is.
fn cap_coverage(in: VertexOut, rim: Rim, cut: bool) -> f32 {
    // Bounded by the outline the cap is part of, in both directions at once.
    // The profile itself runs out at the outline's reach, so what a wider cap
    // would add is the antialiasing ramp's tail past it: a softer end than
    // every other edge of the note wears. With no outline at all there is no
    // band for the cap to be part of.
    let reach = min(in.lead.w, select(in.outline_reach, cut_reach(), cut));
    if (in.lead.x <= 0.0 || reach <= 0.0) {
        return 0.0;
    }
    if cut {
        return cut_coverage(in, rim.cap_d, reach) * rim.cap_keep;
    }
    return outline_coverage(in, rim.cap_d, reach) * rim.cap_keep;
}

/// How much of this fragment any body covers: the union `prepare` drew into
/// `body_holdout`, and 0 off the roll's rect, where no body is.
///
/// It is coverage and not opacity — a body's fade and color alpha are not in
/// it — so a translucent body holds a shadow out as fully as an opaque one.
/// A lead counts as far as it still stands ([`lead_coverage`]), so a spent
/// lead leaves the cap behind it bare, as it always has.
///
/// The outline's masks take the larger of this and their own shape's fill
/// rather than the product. The union already holds that fill, so the max is
/// exact where the note stands alone, and a product would take it out twice
/// along every antialiased edge.
fn body_held(in: VertexOut) -> f32 {
    // The outline is drawn into the mask, which covers the roll's rect texel
    // for texel as the holdout does.
    let texel = vec2<i32>(floor(in.position.xy));
    let size = vec2<i32>(textureDimensions(body_holdout));
    if any(texel < vec2<i32>(0)) || any(texel >= size) {
        return 0.0;
    }
    return textureLoad(body_holdout, texel, 0).r;
}

/// How strongly the OUTLINE layer covers here for this note: the glow
/// standing against every one of the note's edges and fading out.
///
/// The outline stands entirely OUTSIDE the note, which is why it is read off
/// the distance rather than stroked along the note's path: a centered stroke
/// grows inward exactly as much as outward, and at the ribbon widths this pane
/// is used at the two long edges would meet in the middle and flood the note
/// with the outline's own color.
///
/// Masked by the note's OWN fill. `outline_coverage` clamps the distance at
/// the note's edge, so without the mask the outline runs solid across the
/// interior and a note drawn in anything less than an opaque color has a slab
/// of glow under it. Masked too by every OTHER body ([`body_held`]), so over a
/// neighbour the outline is held out rather than left for that neighbour's
/// body to cover only as far as its opacity does.
///
/// Coverage here is `1 - fill` where the body is `fill`, so the wrap
/// retreats as the note takes over its antialiased boundary. The body blends
/// its flat color over that backdrop in the next pass.
///
/// [`cap_coverage`] is the second shape this layer draws, standing against the
/// end of the note INSIDE a box that carries a lead — a place the mask above
/// keeps the wrap out of, correctly, and where an edge nonetheless is.
///
/// The cut's share ([`cut_strength`]) is held out exactly as this is, off the
/// same [`Rim`], so a faded, held-out or translucent-lead note leaves no solid
/// cut where it leaves no solid glow.
fn outline_strength(in: VertexOut, rim: Rim) -> f32 {
    if !rim.live { return 0.0; }
    let wrap = outline_coverage(in, rim.d, in.outline_reach) * rim.keep * rim.lead;
    return in.aura.a * max(wrap, cap_coverage(in, rim, false)) * along(in, in.fade);
}

/// How much of the picture this note's backdrop cut takes here, as a share of
/// the cut's amount: [`outline_strength`] with [`cut_coverage`] for its
/// profile, over the cut's reach.
fn cut_strength(in: VertexOut, rim: Rim) -> f32 {
    if !rim.live { return 0.0; }
    let wrap = cut_coverage(in, rim.d, cut_reach()) * rim.keep * rim.lead;
    return in.aura.a * max(wrap, cap_coverage(in, rim, true)) * along(in, in.fade);
}

/// What the glow and the cut both read off a note at this fragment, once:
/// where the ribbon stands, its distance, and how much of a surround there the
/// bodies and the lead leave standing — around the box, and around the note's
/// own end inside a box that carries a lead ([`cap_coverage`]).
struct Rim {
    /// Whether the ribbon has any width at this depth ([`taper_at`]).
    live: bool,
    d: f32,
    /// `1 - max(fill, held)` against the box: the note's own fill and every
    /// other body ([`body_held`]) take the surround out.
    keep: f32,
    /// [`lead_coverage`].
    lead: f32,
    /// The distance to the note's own end inside a lead box, and `keep`
    /// against it; read only where there is a lead.
    cap_d: f32,
    cap_keep: f32,
};

fn rim_of(in: VertexOut) -> Rim {
    var rim: Rim;
    rim.live = taper_at(in, in.local.y) > 0.0;
    if !rim.live { return rim; }
    rim.d = box_distance(in);
    let held = body_held(in);
    rim.keep = 1.0 - max(inside(in, rim.d, 0.0), held);
    rim.lead = lead_coverage(in);
    if in.lead.x > 0.0 {
        rim.cap_d = box_distance_trimmed(in, in.lead.x);
        rim.cap_keep = 1.0 - max(inside(in, rim.cap_d, 0.0), held);
    }
    return rim;
}

/// This note's share of `body_holdout`: its geometric coverage, as far as a
/// lead still stands, without its fade. Blended into the union in `prepare`.
@fragment
fn fs_body_holdout(fragment: VertexOut) -> @location(0) vec4<f32> {
    let in = on_screen(fragment);
    if taper_at(in, in.local.y) <= 0.0 { return vec4<f32>(0.0); }
    return vec4<f32>(inside(in, box_distance(in), 0.0) * lead_coverage(in), 0.0, 0.0, 1.0);
}

/// Evenly spaced diagonal strokes every six composition points. The marks only
/// dim RGB: neither the continuous silhouette nor its opacity is patterned.
/// Keep an untouched rail beside both pitch edges, and leave the lead plain.
fn tremolo_tone(in: VertexOut) -> f32 {
    let scale = in.tremolo.y;
    if scale <= 0.0 || in.local.y < -in.half_extent.y + in.lead.x {
        return 1.0;
    }
    let x = (in.local.x - in.shear * in.local.y) / scale;
    let half_width = in.half_extent.x * taper_at(in, in.local.y) / scale;
    let reach = min(4.0, half_width * (2.0 / 3.0));
    let diagonal = in.tremolo.x - in.local.y / scale - 0.7 * x;
    let stroke_distance = (fract(diagonal / 6.0 + 0.5) - 0.5) * 6.0;
    let distance = max(abs(x) - reach, abs(stroke_distance) / 1.220656 - 0.75);
    let ink = inside(in, distance * scale, 0.0);
    // The AA ramp must also stay inside a thin ribbon's edges.
    let inset = clamp((half_width - abs(x)) / max(half_width * 0.5, 1e-6), 0.0, 1.0);
    return 1.0 - 0.75 * ink * inset;
}

/// Flat premultiplied gamma-space body color, at the note's own opacity
/// reading. A leading tip set to fade loses its contribution through
/// [`lead_coverage`].
///
/// The bloom pass draws the bodies through this too, so a note's halo is lit
/// at the opacity the note is drawn at (#1289).
fn core_color(in: VertexOut) -> vec4<f32> {
    if taper_at(in, in.local.y) <= 0.0 { return vec4<f32>(0.0); }
    let color = vec4<f32>(in.core.rgb * tremolo_tone(in), in.core.a);
    return color * inside(in, box_distance(in), 0.0) * lead_coverage(in) * along(in, in.fade);
}

// One of the note's intensity readings at this depth: `ends` is its value at
// the two depths `ramp` names, which the caller sampled at the ends of this
// piece, and between them a straight line. The fade multiplies the body and
// its outline together, so a note faded to nothing leaves no glowing silhouette
// behind. Held past both ends, which is what carries the newest value into a
// lead.
fn along(in: VertexOut, ends: vec2<f32>) -> f32 {
    let run = in.ramp.y - in.ramp.x;
    let t = select(0.0, clamp((in.local.y - in.ramp.x) / run, 0.0, 1.0), abs(run) > 1e-6);
    return mix(ends.x, ends.y, t);
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

struct MaskOut {
    @location(0) glow: vec4<f32>,
    @builtin(frag_depth) depth: f32,
};

/// This note's share of the glow mask: its color and how strongly it covers,
/// kept where it is the strongest of any note's.
///
/// The strength is written as DEPTH, under a `Less` test against a mask
/// cleared to 1, so the strongest note's fragment is the one that lands and
/// its color lands with it. A `max` blend would keep the strongest coverage
/// too, but take each color channel's max separately — a hue neither note has,
/// wherever two glows meet. A fragment of no strength writes depth 1 and so
/// never lands, leaving the mask clear there.
///
/// While the cut fills with the glow's color, the mask also has to name a color
/// past the glow's own reach, as far as the cut goes. The strength then maps
/// into the nearer half of the depth range and the cut into the farther, so
/// any glow still beats every bare cut and among bare cuts the strongest names
/// the color; the strengths, and so the picture where the glow stands, are the
/// same either way.
@fragment
fn fs_outline_mask(fragment: VertexOut) -> MaskOut {
    let in = on_screen(fragment);
    let rim = rim_of(in);
    let strength = outline_strength(in, rim);
    if !cut_fills() {
        return MaskOut(vec4<f32>(in.aura.rgb, strength), 1.0 - strength);
    }
    let cut = cut_strength(in, rim);
    let bare = select(1.0, 1.0 - 0.5 * cut, cut > 0.0);
    let depth = select(bare, 0.5 * (1.0 - strength), strength > 0.0);
    return MaskOut(vec4<f32>(in.aura.rgb, strength), depth);
}

/// This note's cut as a DISTANCE ([`cut_unramp`]), into a depth target under a
/// `Less` test: what lands is the nearest, the strongest note's cut, which is
/// the cut itself at a halo blend of 0 and the reference the blend below sums
/// against. A fragment of no cut leaves the target at its clear of 1.
struct CutReferenceOut {
    @builtin(frag_depth) depth: f32,
};

@fragment
fn fs_cut_reference(fragment: VertexOut) -> CutReferenceOut {
    let in = on_screen(fragment);
    let cut = cut_strength(in, rim_of(in));
    if cut <= 0.0 {
        discard;
    }
    return CutReferenceOut(cut_unramp(cut));
}

/// The nearest note's cut distance under this fragment, as [`fs_cut_reference`]
/// left it, and 1 (no cut) off the roll's rect.
@group(1) @binding(5) var cut_reference: texture_depth_2d;

fn cut_reference_at(texel: vec2<i32>) -> f32 {
    let size = vec2<i32>(textureDimensions(cut_reference));
    if any(texel < vec2<i32>(0)) || any(texel >= size) {
        return 1.0;
    }
    return textureLoad(cut_reference, texel, 0);
}

/// This note's share of the HALO BLEND: a smooth minimum of every note's cut
/// distance, `-κ ln Σ exp(-u / κ)`, in place of the plain minimum the
/// reference keeps. Where two notes' cuts meet, the minimum folds along the
/// line between them, a crease that a wide, soft cut makes plain (#1525);
/// the smooth minimum has no fold, and darkens a narrow gap a little more.
///
/// Summed in half floats under an additive blend, which is why each share is
/// taken RELATIVE to the nearest note's: `exp(-(u - nearest) / κ)` is 1 for that
/// note and less for every other, so the sum stays between 1 and the count
/// of notes reaching here and keeps its precision at any κ. Taken straight,
/// `exp(-u / κ)` falls under a half float's smallest value within the cut's
/// reach once κ is under about a sixteenth of it, and the fade's tail would
/// end in a step.
///
/// Each share is lowered by its own value at the cut's reach, as the glow's
/// profile is, so a note's share reaches 0 exactly where its cut does and a
/// note with no cut adds nothing: the blend of one note is that note's cut,
/// and the blend never reaches past the notes' own reach. The composite
/// ([`fs_glow_cut_gamma`]) puts that value back.
@fragment
fn fs_cut_blend(fragment: VertexOut) -> @location(0) vec4<f32> {
    let in = on_screen(fragment);
    let cut = cut_strength(in, rim_of(in));
    if cut <= 0.0 {
        discard;
    }
    let nearest = cut_reference_at(vec2<i32>(floor(in.position.xy)));
    let kappa = locals.cut.w;
    let share = exp(-max(cut_unramp(cut) - nearest, 0.0) / kappa)
        - exp(-max(1.0 - nearest, 0.0) / kappa);
    return vec4<f32>(max(share, 0.0), 0.0, 0.0, 0.0);
}

/// The roll's surround laid over the picture with the backdrop cut, in place
/// of `fs_shadow_mask` in blit.wgsl while the cut is on.
///
/// The glow mask (its strongest note's color and strength `s`), the nearest
/// cut distance, and the halo blend's sum, each the size of the viewport it is
/// laid over, so a pixel reads its own texel.
@group(1) @binding(4) var glow_mask: texture_2d<f32>;
@group(1) @binding(6) var cut_blend: texture_2d<f32>;

struct CoverOut {
    @builtin(position) position: vec4<f32>,
    @location(0) uv: vec2<f32>,
};

/// The whole viewport, as `vs_blit` covers it.
@vertex
fn vs_cover(@builtin(vertex_index) vi: u32) -> CoverOut {
    let corner = vec2<f32>(f32(vi & 1u), f32(vi >> 1u));
    var out: CoverOut;
    out.position = vec4<f32>(corner * 2.0 - 1.0, 0.0, 1.0);
    out.uv = vec2<f32>(corner.x, 1.0 - corner.y);
    return out;
}

/// The surround over this pixel: the glow's straight gamma-space color in
/// `rgb`, and in `a` how much of the picture it COVERS — the larger of the
/// glow's strength and the cut — with the color laid on at
/// `max(s, fill * covered)` of that, which [`glow_cut_weight`] returns.
///
/// The cut takes the backdrop away faster than the glow lays its color on:
/// what is under the surround keeps `1 - max(s, cut)`, and only `s` of the
/// glow's color (or, filling, as much as the cut took) goes on in its place.
/// So at a cut of 0 this is exactly the glow, and over black it is the glow
/// whatever the cut — the cut only takes away light that was there.
struct GlowCut {
    color: vec3<f32>,
    weight: f32,
    covered: f32,
};

fn glow_cut(in: CoverOut) -> GlowCut {
    let size = vec2<i32>(textureDimensions(glow_mask));
    let texel = min(vec2<i32>(in.uv * vec2<f32>(size)), size - vec2<i32>(1));
    let glow = textureLoad(glow_mask, texel, 0);
    let nearest = cut_reference_at(texel);
    var cut = 0.0;
    if nearest < 1.0 {
        var u = nearest;
        let kappa = locals.cut.w;
        if kappa > 0.0 {
            // Undo `fs_cut_blend`: its shares were relative to the nearest
            // note's and lowered by their value at the reach.
            let sum = textureLoad(cut_blend, texel, 0).r;
            let tail = exp(-(1.0 - nearest) / kappa);
            u = nearest - kappa * log(max(sum + tail, 1e-30));
        }
        cut = locals.cut.x * cut_ramp(u);
    }
    let covered = max(glow.a, cut);
    return GlowCut(glow.rgb, max(glow.a, locals.cut_fill * covered), covered);
}

@fragment
fn fs_glow_cut_gamma(in: CoverOut) -> @location(0) vec4<f32> {
    let surround = glow_cut(in);
    return vec4<f32>(surround.color * surround.weight, surround.covered);
}

/// The same onto a target that encodes sRGB for itself, as `fs_core_linear`.
@fragment
fn fs_glow_cut_linear(in: CoverOut) -> @location(0) vec4<f32> {
    let surround = glow_cut(in);
    return vec4<f32>(linear_from_gamma_rgb(surround.color) * surround.weight, surround.covered);
}

@fragment
fn fs_core_gamma(in: VertexOut) -> @location(0) vec4<f32> {
    return core_color(on_screen(in));
}

@fragment
fn fs_core_linear(in: VertexOut) -> @location(0) vec4<f32> {
    let gamma = core_color(on_screen(in));
    return vec4<f32>(linear_from_gamma_rgb(gamma.rgb), gamma.a);
}
