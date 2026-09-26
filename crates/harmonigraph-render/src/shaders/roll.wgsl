// The piano roll's notes: one instanced quad per note segment — a solid
// rectangle with a flat color, wrapped on every side by an outline that
// fades out, both falling out of a signed distance field. A segment may also
// end in a fade at its LEADING tip (`lead_coverage`), which takes both layers
// out together, and carry its own outline cap where the note inside it stops
// (`cap_coverage`).
//
// TWO LAYERS, drawn as two passes over the same instances rather than
// composited per note: every note's outline (`fs_outline_*`), then every
// note's body (`fs_core_*`). Both use ordinary over; body opacity is independent
// of the dark outline. One quad's worth of geometry drawn twice.
//
// The order is the whole point. The outline is opaque where it meets its own
// note — it has to be, or it takes its color from the spectrogram cell behind
// it and washes out over the bright end of a palette — so an outline
// composited with its own note lands on the NEIGHBOURING notes it reaches
// into, and along time those neighbours are the next note: repeats of one key
// butt together there, and the later one blanked the tail of the earlier.
// Under every body instead, an outline darkens the backdrop before the note
// paints its color. Only the body's own transparency can let that shadow through.
//
// What that costs is the seam between two notes that TOUCH: same key, no gap,
// and the bodies now meet directly in one color where the outline used to
// stand between them. A gap of a point or more still reads as two notes, since
// the outline fills it.
//
// The outline is a color the pane hands over, not a decision made here: this
// shader is told how far it reaches, how gradually it goes, and what color it
// is, and invents none of the three.
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
    /// 1 in the bloom's pass, which draws each body at its glow rather than its
    /// fade (see [`core_color`]); 0 on screen.
    light: f32,
    /// Unit screen vectors of the pane's two axes. Pitch runs across the
    /// pane's short side, depth (time) along its long side.
    pitch_dir: vec2<f32>,
    depth_dir: vec2<f32>,
    _axis_pad: vec2<f32>,
    /// σ, depth, kernel kind (Distance = 1), and whole kernel reach, in points.
    shadow: vec4<f32>,
    shadow_atlas_size: vec2<f32>,
    // The group's Shadow falloff (`ShadowStyle::falloff`), read only on the
    // distance path, in what was the block's own tail padding.
    shadow_falloff: f32,
    _shadow_pad: f32,
};

@group(0) @binding(0) var<uniform> locals: Locals;

struct VertexOut {
    @builtin(position) position: vec4<f32>,
    /// Offset from the note's center in points, along (pitch, depth).
    @location(0) local: vec2<f32>,
    /// Half extents of the note's solid body, same two axes.
    @location(1) @interpolate(flat) half_extent: vec2<f32>,
    /// The center line's pitch drift per point of depth: 0 for a held note,
    /// non-zero for a glide, which shears the box into a parallelogram.
    @location(2) @interpolate(flat) shear: f32,
    /// How far the outline reaches past the note's edge, in points, and 0 when
    /// the outline is off. It wraps every side — see [`outline_coverage`].
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
    /// Premultiplied, gamma-space, exactly as egui carries `Color32`.
    @location(8) @interpolate(flat) core: vec4<f32>,
    /// The outline's color at full coverage; the fade takes it from there.
    @location(9) @interpolate(flat) outline: vec4<f32>,
    /// Surface point and caster index for the Gaussian atlas read.
    @location(10) at: vec2<f32>,
    @location(11) @interpolate(flat) who: u32,
    /// Width of one coverage sample in points. A visible note uses one display
    /// pixel; a Gaussian cell uses one of its own deliberately coarser texels.
    @location(12) @interpolate(flat) feather: f32,
    /// The two depth offsets the readings below are given at. See [`along`].
    @location(13) @interpolate(flat) ramp: vec2<f32>,
    /// Opacity at those two depths (`xy`), and the body's light for the bloom
    /// (`zw`).
    @location(14) @interpolate(flat) reads: vec4<f32>,
};

@vertex
fn vs_note(
    @builtin(vertex_index) vertex: u32,
    @builtin(instance_index) who: u32,
    @location(0) center: vec2<f32>,
    @location(1) half_extent: vec2<f32>,
    @location(2) shear: f32,
    @location(3) outline_reach: f32,
    // Lead, lead fade, lead alpha and cap reach; span then ramp; fade then
    // glow: packed, since a vertex takes sixteen at most.
    @location(4) lead: vec4<f32>,
    @location(8) core: vec4<f32>,
    @location(9) outline: vec4<f32>,
    @location(14) span_ramp: vec4<f32>,
    @location(15) reads: vec4<f32>,
    @location(5) taper_depth: vec4<f32>,
    @location(6) taper: vec4<f32>,
) -> VertexOut {
    // Triangle-strip corners: (-1,-1) (1,-1) (-1,1) (1,1).
    let corner = vec2<f32>(
        select(-1.0, 1.0, (vertex & 1u) == 1u),
        select(-1.0, 1.0, (vertex & 2u) == 2u),
    );

    let slope = shear;
    // How far outside its own box a note can paint, per axis. The quad is its
    // bounding box grown by that, and a shortfall here CLIPS ink rather than
    // costing a little fill rate, so each term is the exact one `note_color`
    // can reach to.
    //
    // The outline wraps the note, so it is owed room on BOTH axes: ink runs out
    // wherever the box distance passes `outline_reach`, which is `reach` past
    // every edge and every corner.
    //
    // The distance is Euclidean (see [`box_distance_trimmed`]), so the grown
    // box is the note's own bounding box — its ribbon's half width plus the
    // center line's drift over the note's half-length — with the same margin
    // on both axes, however steep the glide.
    let reach = locals.shadow.w + 0.5 * locals.feather;
    let margin = reach + 0.5 * locals.feather;
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
    out.core = core;
    out.outline = outline;
    out.at = pos;
    out.who = who;
    out.feather = locals.feather;
    out.ramp = span_ramp.zw;
    out.reads = reads;
    return out;
}

/// Rasterize the roll box field's coverage into a Gaussian cell. The quad is
/// the packer's padded box; each point is projected back onto the roll's own
/// pitch/depth axes before the same `box_distance` as the scene draw reads it.
@vertex
fn vs_shadow_cell(
    @builtin(vertex_index) vertex: u32,
    @location(0) center: vec2<f32>,
    @location(1) half_extent: vec2<f32>,
    @location(2) shear: f32,
    @location(3) outline_reach: f32,
    @location(4) lead: vec4<f32>,
    @location(8) core: vec4<f32>,
    @location(9) outline: vec4<f32>,
    @location(5) taper_depth: vec4<f32>,
    @location(6) taper: vec4<f32>,
    @location(10) box_rect: vec4<f32>,
    @location(11) box_cell: vec4<f32>,
    @location(12) box_meta: vec4<f32>,
    @location(13) box_who: vec4<f32>,
) -> VertexOut {
    let corner = vec2<f32>(
        select(0.0, 1.0, (vertex & 1u) == 1u),
        select(0.0, 1.0, (vertex & 2u) == 2u),
    );
    let point = box_rect.xy + corner * box_rect.zw;
    let delta = point - center;
    var out: VertexOut;
    let texel = cell_texel(point, box_rect, box_cell, box_meta.x);
    out.position = select(
        no_quad(),
        cell_clip(texel, locals.shadow_atlas_size, 1.0),
        cell_packed(box_cell) && box_who.y < 0.5 * DISTANCE_KIND,
    );
    out.local = vec2<f32>(dot(delta, locals.pitch_dir), dot(delta, locals.depth_dir));
    out.half_extent = half_extent;
    out.shear = shear;
    out.outline_reach = box_who.w;
    out.lead = lead;
    // The cell holds this piece's tapered shape, so its shadow narrows with
    // the ribbon.
    out.taper_depth = taper_depth;
    out.taper = taper;
    out.core = core;
    out.outline = outline;
    out.at = point;
    out.who = u32(box_who.x + 0.5);
    out.feather = 1.0 / max(box_meta.x, 1e-6);
    // The cell holds the segment's whole coverage whatever piece it is for,
    // so a piece's shadow runs on across the cut; fading is the outline's.
    out.ramp = vec2<f32>(0.0);
    out.reads = vec4<f32>(1.0);
    return out;
}

@fragment
fn fs_shadow_coverage(in: VertexOut) -> @location(0) vec4<f32> {
    let d = box_distance(in);
    var source = in;
    // Only the producer repurposes outline_reach as the expansion radius.
    // Move the fading lead outward with its source, leaving the visible note.
    source.local.y += in.outline_reach;
    let coverage = inside(in, d, in.outline_reach) * lead_coverage(source);
    return vec4<f32>(coverage, 0.0, 0.0, 1.0);
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

/// The selected common shadow renderer at this fragment. Distance consumes the
/// box SDF directly; Gaussian reads the one blurred coverage cell made from
/// that same SDF. The pane keeps only the outside half as its dark surround.
fn outline_coverage(in: VertexOut, d: f32, reach: f32) -> f32 {
    if reach <= 0.0 || locals.shadow.y <= 0.0 {
        return 0.0;
    }
    var full = standoff_coverage(d, 2.0 * locals.shadow.x, locals.shadow_falloff);
    if locals.shadow.z < 0.5 * DISTANCE_KIND {
        full = shadow_kernel(in.who, in.at);
    }
    // The common style owns the profile, while `reach` may still shorten an
    // interior cap when the pane has less room before its now-line. Keep that
    // geometric bound hard: shortening a cap moves its end instead of dimming
    // the whole profile.
    return (1.0 - shadow_transmittance(full, locals.shadow.y, 1.0)) * inside(in, d, reach);
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
/// hard black cap standing where the ink went. Past the tip the ramp is
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
/// [`outline_color`]'s mask keeps the outline out of a box's middle, correctly,
/// since a box has no edge there. So the cap arrives only when the pane drops
/// the spent lead and the box shrinks back to the note, and it arrives whole,
/// in one frame, on a ribbon that has been dissolving for a quarter second.
///
/// Drawn under the lead it needs no ramp of its own. The outline layer goes
/// down before ANY body (see the head of this file), so the lead's own ink
/// covers this while the lead is opaque and uncovers it at exactly the rate the
/// lead goes: `lead_coverage` and this are the two halves of one boundary and
/// sum to 1 across it. The crossfade is the compositing.
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
/// off two shapes that share three of their sides, and beside the note's
/// leading corners both are looking at the same ink. Added, that overlap comes
/// out darker than black is.
fn cap_coverage(in: VertexOut) -> f32 {
    // Bounded by the outline the cap is part of, in both directions at once.
    // Wider, the cap would band the note further than every other edge of it
    // — and `vs_note` sizes the quad from `outline_reach` alone, so the
    // surplus is CLIPPED across pitch rather than merely drawn, which is a
    // hard vertical edge standing where a rounded corner belongs. With no
    // outline at all there is no band for the cap to be part of.
    let reach = min(in.lead.w, in.outline_reach);
    if (in.lead.x <= 0.0 || reach <= 0.0) {
        return 0.0;
    }
    let d = box_distance_trimmed(in, in.lead.x);
    return outline_coverage(in, d, reach) * (1.0 - inside(in, d, 0.0));
}

/// Premultiplied gamma-space color of the OUTLINE layer: the dark surround
/// standing against every one of the note's edges and fading out.
///
/// The outline stands entirely OUTSIDE the note, which is why it is read off
/// the distance rather than stroked along the note's path: a centered stroke
/// grows inward exactly as much as outward, and at the ribbon widths this pane
/// is used at the two long edges would meet in the middle and flood the note
/// with the outline's own color.
///
/// Masked by the note's OWN fill, which is the one thing this layer still
/// knows about its body. `outline_coverage` clamps the distance at the note's
/// edge, so without the mask the outline runs solid across the interior and a
/// note drawn in anything less than an opaque color has a black slab under it.
/// The mask is that note's fill and no other's, so it takes nothing back off
/// the fix: over a NEIGHBOUR the outline still paints in full, and the
/// neighbour's body — drawn in the pass after this one — covers it to its own opacity.
///
/// Coverage here is `1 - fill` where the body is `fill`, so the dark wrap
/// retreats as the note takes over its antialiased boundary. The body blends
/// its flat color over that backdrop in the next pass.
///
/// [`cap_coverage`] is the second shape this layer draws, standing against the
/// end of the note INSIDE a box that carries a lead — a place the mask above
/// keeps the wrap out of, correctly, and where an edge nonetheless is.
fn outline_color(in: VertexOut) -> vec4<f32> {
    let d = box_distance(in);
    let wrap =
        outline_coverage(in, d, in.outline_reach) * (1.0 - inside(in, d, 0.0)) * lead_coverage(in);
    return in.outline * max(wrap, cap_coverage(in)) * along(in, in.reads.xy);
}

/// Flat premultiplied gamma-space body color. A leading tip set to fade loses
/// its contribution through
/// [`lead_coverage`].
fn core_color(in: VertexOut) -> vec4<f32> {
    let body = in.core * inside(in, box_distance(in), 0.0) * lead_coverage(in);
    if (locals.light < 0.5) {
        return body * along(in, in.reads.xy);
    }
    // In the bloom's pass the body wears its glow instead, so the light a note
    // gives off follows its own display. A glow past 1 is a note blooming over
    // the pass's strength: its colour takes the whole of it, into a float
    // target, and its alpha stops at 1. Past 1 an alpha would take more than
    // everything under it away in the blend, and the chain's threshold reads
    // colour over alpha, so the note's colour reads that much brighter there
    // too: past the knee a share of 2 is twice the light, and a dim note is
    // lifted through it.
    let glow = along(in, in.reads.zw);
    return vec4<f32>(body.rgb * glow, body.a * min(glow, 1.0));
}

// One of the note's intensity readings at this depth: `ends` is its value at
// the two depths `ramp` names, which the caller sampled at the ends of this
// piece, and between them a straight line. The fade multiplies the body and
// its outline together, so a note faded to nothing leaves no dark silhouette
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
// plain Unorm surface and take `fs_note_gamma`.
fn linear_from_gamma_rgb(srgb: vec3<f32>) -> vec3<f32> {
    let cutoff = srgb < vec3<f32>(0.04045);
    let lower = srgb / vec3<f32>(12.92);
    let higher = pow((srgb + vec3<f32>(0.055)) / vec3<f32>(1.055), vec3<f32>(2.4));
    return select(higher, lower, cutoff);
}

@fragment
fn fs_outline_gamma(in: VertexOut) -> @location(0) vec4<f32> {
    return outline_color(in);
}

@fragment
fn fs_outline_linear(in: VertexOut) -> @location(0) vec4<f32> {
    let gamma = outline_color(in);
    return vec4<f32>(linear_from_gamma_rgb(gamma.rgb), gamma.a);
}

@fragment
fn fs_core_gamma(in: VertexOut) -> @location(0) vec4<f32> {
    return core_color(in);
}

@fragment
fn fs_core_linear(in: VertexOut) -> @location(0) vec4<f32> {
    let gamma = core_color(in);
    return vec4<f32>(linear_from_gamma_rgb(gamma.rgb), gamma.a);
}
