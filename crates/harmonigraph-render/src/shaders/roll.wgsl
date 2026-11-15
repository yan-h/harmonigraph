// The piano roll's notes: one instanced quad per note segment — a solid
// rectangle with a flat color, surrounded on every side by a BACKDROP CUT that
// fades out, both falling out of a signed distance field. A box may instead be
// a LEAD, a stretch of ribbon carried past where its note stops, whose
// opacity and tip fade take both out together (`lead_coverage`).
//
// TWO LAYERS, drawn as passes over the same instances rather than composited
// per note: every note's surround, then every note's body (`fs_core_*`), and a
// note's opacity (its `fade`) takes the two out together. The same instances
// drawn more than once, each layer through a quad as big as its own ink: the
// surround's grown by the cut's reach (`vs_note_cut`), the body's by one
// feather (`vs_note_body`).
//
// The bodies use ordinary over. The surrounds do not: each note's cut goes into
// a target over the roll's rect (`fs_note_surround`), where a depth test keeps
// whichever note's cut is strongest — its cut AND its fill color — and the
// target is laid over the picture once (surround.wgsl and its composite).
// Where two notes' cuts meet, the picture there is as cut as the stronger of
// the two and no more, rather than one cut taken twice.
//
// The order is the whole point. The surround is at its strongest where it
// meets its own note, so a surround composited with its own note lands on the
// NEIGHBOURING notes it reaches into, and along time those neighbours are the
// next note: repeats of one key butt together there, and the later one blanked
// the tail of the earlier. Under every body instead, a surround takes the
// backdrop before the note paints its color.
//
// A body drawn at less than full opacity would let that cut through, so every
// surround is also HELD OUT wherever any body covers (#1293): `prepare` draws
// the union of the bodies' coverage into `body_holdout` first, and the surround
// layer takes it off ([`body_held`]). One note's cut takes only the picture
// behind the ribbons, never another ribbon, at any opacity. That includes a
// note's own cut under its own body, so a translucent ribbon shows the picture
// behind it uncut. Outside every body the holdout is 0 and the cut is what it
// was.
//
// What that costs is the seam between two notes that TOUCH: only across
// voices now, since same-voice repeats keep a 2 pt gap or fuse into one dense
// run (`GAP_PT` in the pane's density pass). There the bodies meet directly in
// one color where the surround used to stand between them. A gap of a point or
// more still reads as two notes, since the surround takes it.
//
// The surround is read straight off the note's distance, so it is as strong
// against a hairline ribbon as against a wide one. Its reach, amount, shape and
// fill are the pane's, not decisions made here, and so is each note's fill
// color: this shader invents none of them.

// Nothing here is tessellated. A note is four vertices whatever its shape,
// and the outline the egui path drew as a separate stroked rounded rect is
// now a surround read off the distance, which costs a compare rather than a
// second shape.
//
// Coordinates arrive in egui POINTS, exactly as egui's own vertex shader
// takes them, and `note_vertex` does the same screen->clip mapping. The pane's
// orientation lives entirely in `pitch_dir` / `depth_dir`, so this shader
// never names a screen side either.

struct Locals {
    /// The backdrop cut (surround.wgsl): its amount, reach, shape and tint
    /// fade. First, so its vec4 is aligned with no padding.
    surround: SurroundCut,
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
    /// Rounds the block up to its vec4's alignment, spelled out to match
    /// the Rust side.
    _tail_pad: vec2<f32>,
};

@group(0) @binding(0) var<uniform> locals: Locals;

/// The union of every body's coverage, one texel per device pixel of the
/// roll's rect, read only by the surround's passes. Binding 2 keeps it clear of
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
    /// How far the box's corners are rounded, in points: 0 for a note, the
    /// radius for a dot. See [`box_distance`].
    @location(3) @interpolate(flat) rounding: f32,
    /// Two to a slot:
    /// - `x`: how much of the box's leading tip is spent fading out, in
    ///   points; 0 for a note. See [`lead_coverage`].
    /// - `y`: how much of a lead is still standing, 0..1; 1 for a note.
    @location(4) @interpolate(flat) lead: vec2<f32>,
    /// Four depth offsets, ascending, and the ribbon's width at each as a share
    /// of its full `half_extent.x`. See [`taper_at`].
    @location(5) @interpolate(flat) taper_depth: vec4<f32>,
    @location(6) @interpolate(flat) taper: vec4<f32>,
    /// Dense-repeat phase (composition points) and point scale; zero disables.
    @location(7) @interpolate(flat) tremolo: vec2<f32>,
    /// Premultiplied, gamma-space, exactly as egui carries `Color32`.
    @location(8) @interpolate(flat) core: vec4<f32>,
    /// The surround's fill color, straight gamma-space RGB, and in `a` how
    /// strongly the note cuts at full coverage; the fade takes it from there.
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

/// The surround's quad ([`fs_note_surround`]): the note's box grown by the
/// cut's reach, past every edge and every corner, and a whole feather past
/// that. The ink stops half a feather out, so the quad's outer half-feather
/// holds none. The roll's culls in the UI ask `Quad` for this same reach.
@vertex
fn vs_note_cut(
    @builtin(vertex_index) vertex: u32,
    @location(0) center: vec2<f32>,
    @location(1) half_extent: vec2<f32>,
    @location(2) shear: f32,
    @location(3) rounding: f32,
    // Lead fade and lead alpha; span then ramp.
    // Packed several to a slot.
    @location(4) lead: vec2<f32>,
    @location(8) core: vec4<f32>,
    @location(9) aura: vec4<f32>,
    @location(14) span_ramp: vec4<f32>,
    @location(15) fade: vec2<f32>,
    @location(5) taper_depth: vec4<f32>,
    @location(6) taper: vec4<f32>,
    @location(7) tremolo: vec2<f32>,
) -> VertexOut {
    return note_vertex(
        vertex, center, half_extent, shear, rounding, lead, core, aura, span_ramp, fade,
        taper_depth, taper, tremolo, cut_reach() + locals.feather,
    );
}

/// The BODY layers' quad — the core layer, the body holdout and the bloom's
/// notes: the note's box grown by one feather. A body's ink is
/// `inside(in, d, 0.0)`, which is 0 from half a feather past its edge, so the
/// cut's reach would only be fragments that compute 0. At long spans that
/// reach is most of the quad: a note a couple of points across drawn through a
/// box some fifteen points wider.
@vertex
fn vs_note_body(
    @builtin(vertex_index) vertex: u32,
    @location(0) center: vec2<f32>,
    @location(1) half_extent: vec2<f32>,
    @location(2) shear: f32,
    @location(3) rounding: f32,
    @location(4) lead: vec2<f32>,
    @location(8) core: vec4<f32>,
    @location(9) aura: vec4<f32>,
    @location(14) span_ramp: vec4<f32>,
    @location(15) fade: vec2<f32>,
    @location(5) taper_depth: vec4<f32>,
    @location(6) taper: vec4<f32>,
    @location(7) tremolo: vec2<f32>,
) -> VertexOut {
    return note_vertex(
        vertex, center, half_extent, shear, rounding, lead, core, aura, span_ramp, fade,
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
    rounding: f32,
    lead: vec2<f32>,
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
    // The distance is Euclidean (see [`box_distance`]), so the grown
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
    out.rounding = rounding;
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
/// than the body and the surround's a cut's reach wider, and interpolated
/// across those two, one pixel read two slightly different coordinates. Every
/// pass onto the surface or a texture over the roll goes through this, so the
/// body, its holdout and its surround agree to the bit on where the note is
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

/// The backdrop cut's reach past a note's edge, in points. 0 while the cut is
/// off.
fn cut_reach() -> f32 {
    return select(0.0, locals.surround.cut.y, locals.surround.cut.x > 0.0);
}

/// How much of the picture the cut takes `d` points off the note's edge, as a
/// share of its amount: [`surround_ramp`] over the cut's reach, and nothing
/// from `reach` on.
///
/// Read off the distance rather than a blur of the note's silhouette, which is
/// the point of it: a blur of a ribbon thinner than its σ never reaches full
/// strength, so the surround all but vanished round exactly the hairline notes
/// that need one, and spread into a wide smear round a fat one. Here a note's
/// edge is equally held at every ribbon width. The amount itself is laid on in
/// [`fs_note_surround`], one amount for every note, so it changes no note's
/// rank against another's.
fn cut_coverage(in: VertexOut, d: f32) -> f32 {
    let reach = cut_reach();
    if reach <= 0.0 {
        return 0.0;
    }
    return surround_fade_at(locals.surround, d) * inside(in, d, reach);
}

/// How much of the ribbon survives at this fragment: 1 across a NOTE, and
/// across a lead whatever the lead's own opacity and tip fade have left there.
///
/// The tip is `-half_extent.y` along the depth axis, always — depth runs away
/// from whatever the roll is drawn beside, so this names an end of the pane's
/// own axis and not a screen side, exactly as everything else here does. `u`
/// counts INWARD from that tip.
///
/// Applied to BOTH layers, which is what makes a fading tip a fading ribbon
/// rather than a ribbon dissolving inside its own surround: the surround wraps
/// the ends as much as the flanks, so a body taken out on its own would leave a
/// hard cap of cut standing where the ink went. Past the tip the ramp is
/// negative and clamps to 0, so that cap is gone rather than merely faint.
///
/// A lead with NO fade keeps its cap, and the branch that gives it one is not a
/// special case dodged. Whether the tip fades is the one real distinction the
/// pane's bar draws with its two handles closed against apart, and the two ends
/// want opposite things: a tip that dissolves has nothing left for a surround to
/// bound, while a tip that ends square is an edge like any other on the note and
/// is owed the same surround the flanks get. So a fade of 0 is uniform right out
/// to the box, and [`inside`] is what ends it, dimmed by the opacity. (The
/// spectral pane clips that cap off, its budget past the now-line being the
/// lead itself; a caller who wants the cap has only to leave it room.)
///
/// The fade is floored at one pixel, [`cut_coverage`]'s bargain for its
/// reason: a sub-pixel ramp is an aliased edge. The caller keeps it within the
/// box, so a fade dialled past the lead's length fades it whole.
///
/// An ordinary note, a fade of 0 and an alpha of 1, leaves at the first line.
fn lead_coverage(in: VertexOut) -> f32 {
    let lead_fade = in.lead.x;
    let lead_alpha = in.lead.y;
    if (lead_fade <= 0.0) {
        return lead_alpha;
    }
    let u = in.local.y + in.half_extent.y;
    return lead_alpha * clamp(u / max(lead_fade, in.feather), 0.0, 1.0);
}

/// Signed distance from this fragment to the note's own box, in points:
/// negative inside it, positive outside, and Euclidean — the true distance to
/// the nearest point of the box, whichever edge or corner that is. On a glide
/// that is perpendicular to the long edges, so the rim bands keep their
/// thickness instead of thinning with its angle.
///
/// A ribbon whose width holds still along this instance is the parallelogram,
/// at that width; one whose width moves is [`tapered_distance`]. Both are
/// exact, so an unmoving note draws exactly as it did before widths moved.
///
/// A box with a `rounding` is the box that much smaller, grown back by it
/// in every direction: rounded, and a disc where it is the half extent,
/// which is how the Spiral's dots come out round.
fn box_distance(in: VertexOut) -> f32 {
    let t = in.taper;
    if (all(t == vec4<f32>(t.x))) {
        let r = in.rounding;
        return parallelogram_distance(in, in.half_extent.x * t.x - r, in.half_extent.y - r) - r;
    }
    return tapered_distance(in);
}

/// [`box_distance`] for a ribbon `half_pitch` either side of its center line
/// all along, `half_along` either side of its center along depth.
fn parallelogram_distance(in: VertexOut, half_pitch: f32, half_along: f32) -> f32 {
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
    // surround's reach along pitch. Per-note tuning routinely makes a segment a
    // hundredth of a point long across part of a semitone — a slope in the
    // thousands — and that segment's surround and antialiasing ramp became a
    // hairline strip through the whole pane.
    //
    // Square corners for a note: a note is a rectangle in the pane's two axes
    // (`RollInstance::rounding`). The surround's own corners are round, being a
    // constant distance from a square one, and that is the shape a note wants
    // wrapped around it.
    // The center line's far end, from the center.
    let end = vec2<f32>(slope * half_along, half_along);
    var p = in.local;
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
/// those: a neighbour shorter than the surround's reach lets the next one's
/// flank be nearer than this piece knows, and the surround steps by that much
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

/// [`box_distance`] for a ribbon whose width moves along the piece:
/// the exact distance to the polygon its two flanks bound, straight between
/// the taper points and square across depth at the box's ends. It is still
/// the TRUE distance (#1118), so a sliver glide's surround stays within its
/// reach along pitch here as well.
fn tapered_distance(in: VertexOut) -> f32 {
    let slope = in.shear;
    let lo = -in.half_extent.y;
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

/// How much of this fragment any body covers: the union `prepare` drew into
/// `body_holdout`, and 0 off the roll's rect, where no body is.
///
/// It is coverage and not opacity — a body's fade and color alpha are not in
/// it — so a translucent body holds a cut out as fully as an opaque one.
/// A lead counts as far as it still stands ([`lead_coverage`]), so a fading
/// lead holds a neighbour's cut out only as far as it is still there.
///
/// The surround takes the larger of this and its own note's fill rather than
/// the product. The union already holds that fill, so the max is exact where
/// the note stands alone, and a product would take it out twice along every
/// antialiased edge.
fn body_held(in: VertexOut) -> f32 {
    // The surround is drawn into targets that cover the roll's rect texel for
    // texel as the holdout does.
    let texel = vec2<i32>(floor(in.position.xy));
    let size = vec2<i32>(textureDimensions(body_holdout));
    if any(texel < vec2<i32>(0)) || any(texel >= size) {
        return 0.0;
    }
    return textureLoad(body_holdout, texel, 0).r;
}

/// How much a note cuts at a fragment, as a share of the amount, and the fade
/// its tint follows ([`surround_tinted`]). A strength above 0 has a fade above
/// 0.
struct NoteCut {
    strength: f32,
    fade: f32,
};

/// How much of the picture this note's backdrop cut takes here, as a share of
/// the cut's amount: the cut standing against every one of the note's edges
/// and fading out.
///
/// The surround stands entirely OUTSIDE the note, which is why it is read off
/// the distance rather than stroked along the note's path: a centered stroke
/// grows inward exactly as much as outward, and at the ribbon widths this pane
/// is used at the two long edges would meet in the middle and cut the note
/// itself.
///
/// Masked by the note's OWN fill. `cut_coverage` clamps the distance at the
/// note's edge, so without the mask the cut runs whole across the interior and
/// a note drawn in anything less than an opaque color has a slab of black
/// under it. Masked too by every OTHER body ([`body_held`]), so over a
/// neighbour the cut is held out rather than left for that neighbour's body to
/// cover only as far as its opacity does.
///
/// Strength here is `1 - fill` where the body is `fill`, so the wrap retreats
/// as the note takes over its antialiased boundary. The body blends its flat
/// color over that backdrop in the next pass.
///
/// The note's opacity reading takes the whole of it out, so a faded note
/// leaves no solid cut behind, and a lead's coverage takes out the cut and its
/// tint's fade alike.
fn cut_strength(in: VertexOut, rim: Rim) -> NoteCut {
    if !rim.live { return NoteCut(0.0, 0.0); }
    let wrap = cut_coverage(in, rim.d) * rim.keep * rim.lead;
    let fade = surround_fade_at(locals.surround, rim.d) * rim.lead;
    return NoteCut(in.aura.a * wrap * along(in, in.fade), fade);
}

/// What the cut reads off a note at this fragment, once: where the ribbon
/// stands, its distance, and how much of a surround there the bodies and the
/// lead leave standing.
struct Rim {
    /// Whether the ribbon has any width at this depth ([`taper_at`]).
    live: bool,
    d: f32,
    /// `1 - max(fill, held)` against the box: the note's own fill and every
    /// other body ([`body_held`]) take the surround out.
    keep: f32,
    /// [`lead_coverage`].
    lead: f32,
};

fn rim_of(in: VertexOut) -> Rim {
    var rim: Rim;
    rim.live = taper_at(in, in.local.y) > 0.0;
    if !rim.live { return rim; }
    rim.d = box_distance(in);
    let held = body_held(in);
    rim.keep = 1.0 - max(inside(in, rim.d, 0.0), held);
    rim.lead = lead_coverage(in);
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
/// Keep an untouched rail beside both pitch edges.
fn tremolo_tone(in: VertexOut) -> f32 {
    let scale = in.tremolo.y;
    if scale <= 0.0 {
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
// its surround together, so a note faded to nothing leaves no cut silhouette
// behind. Held past both ends, which is what carries the newest value into a
// lead.
fn along(in: VertexOut, ends: vec2<f32>) -> f32 {
    let run = in.ramp.y - in.ramp.x;
    let t = select(0.0, clamp((in.local.y - in.ramp.x) / run, 0.0, 1.0), abs(run) > 1e-6);
    return mix(ends.x, ends.y, t);
}

struct SurroundOut {
    @location(0) color: vec4<f32>,
    @builtin(frag_depth) depth: f32,
};

/// This note's share of the surround: its fill color with its cut in alpha,
/// kept where its cut is the strongest of any note's.
///
/// The cut is written as DEPTH, under a `Less` test against a target cleared
/// to 1, so the strongest note's fragment is the one that lands and its color
/// lands with it. A fragment of no cut leaves the target clear.
@fragment
fn fs_note_surround(fragment: VertexOut) -> SurroundOut {
    let in = on_screen(fragment);
    let note = cut_strength(in, rim_of(in));
    let cut = locals.surround.cut.x * note.strength;
    if cut <= 0.0 {
        discard;
    }
    let fill = surround_tinted(in.aura.rgb, note.fade, locals.surround.cut.w);
    return SurroundOut(vec4<f32>(fill, cut), 1.0 - cut);
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
