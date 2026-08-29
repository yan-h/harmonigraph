// The piano roll's notes: one instanced quad per note segment — a solid
// rectangle in the note's own color, wrapped on every side by an outline that
// fades out, both falling out of a signed distance field. A segment may also
// end in a fade at its LEADING tip (`lead_coverage`), which takes both layers
// out together, and carry its own outline cap where the note inside it stops
// (`cap_coverage`).
//
// TWO LAYERS, drawn as two passes over the same instances rather than
// composited per note: every note's outline (`fs_outline_*`), then every
// note's body (`fs_core_*`). One quad's worth of geometry drawn twice.
//
// The order is the whole point. The outline is opaque where it meets its own
// note — it has to be, or it takes its color from the spectrogram cell behind
// it and washes out over the bright end of a palette — so an outline
// composited with its own note lands on the NEIGHBOURING notes it reaches
// into, and along time those neighbours are the next note: repeats of one key
// butt together there, and the later one blanked the tail of the earlier.
// Under every body instead, an outline can only ever darken the picture, never
// another note.
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
    _pad: f32,
    /// Unit screen vectors of the pane's two axes. Pitch runs across the
    /// pane's short side, depth (time) along its long side.
    pitch_dir: vec2<f32>,
    depth_dir: vec2<f32>,
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
    /// How much of that reach the outline spends fading out, in points.
    @location(4) @interpolate(flat) outline_fade: f32,
    /// How much of the box's LEADING end is a lead rather than the note itself,
    /// in points — 0 for the ordinary segment that is all note. See
    /// [`lead_coverage`].
    @location(5) @interpolate(flat) lead: f32,
    /// How much of that lead is spent fading out at the tip, in points.
    @location(6) @interpolate(flat) lead_fade: f32,
    /// How much of the lead is still standing, 0..1.
    @location(7) @interpolate(flat) lead_alpha: f32,
    /// How far the outline's cap at the NOTE's own leading end reaches, in
    /// points. See [`cap_coverage`].
    @location(8) @interpolate(flat) cap_reach: f32,
    /// Premultiplied, gamma-space, exactly as egui carries `Color32`.
    @location(9) @interpolate(flat) core: vec4<f32>,
    /// The outline's color at full coverage; the fade takes it from there.
    @location(10) @interpolate(flat) outline: vec4<f32>,
};

@vertex
fn vs_note(
    @builtin(vertex_index) vertex: u32,
    @location(0) center: vec2<f32>,
    @location(1) half_extent: vec2<f32>,
    @location(2) shear: f32,
    @location(3) outline_reach: f32,
    @location(4) outline_fade: f32,
    @location(5) lead: f32,
    @location(6) lead_fade: f32,
    @location(7) lead_alpha: f32,
    @location(8) cap_reach: f32,
    @location(9) core: vec4<f32>,
    @location(10) outline: vec4<f32>,
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
    // Across pitch that reach is measured PERPENDICULAR to the note's long
    // edges — which on a sheared note is not the pitch axis. `note_color`
    // divides by `skew` to get that perpendicular distance, so an outline
    // reaching `w` stands `skew * w` out along pitch, and a steep glide's
    // outline reaches a multiple of its own reach. The `slope` term is the
    // drift of the center line over the quad's own half-length, ends included.
    let skew = sqrt(1.0 + slope * slope);
    let reach = outline_reach + 0.5 * locals.feather;
    let half_depth = half_extent.y + reach + 0.5 * locals.feather;
    let extent = vec2<f32>(
        half_extent.x + abs(slope) * half_depth + skew * reach,
        half_depth,
    );

    let local = corner * extent;
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
    out.outline_reach = outline_reach;
    out.outline_fade = outline_fade;
    out.lead = lead;
    out.lead_fade = lead_fade;
    out.lead_alpha = lead_alpha;
    out.cap_reach = cap_reach;
    out.core = core;
    out.outline = outline;
    return out;
}

/// Coverage of everything on the near side of `edge`: how much of a
/// one-pixel-wide window centered on the signed distance `d` lands inside it.
///
/// A box filter along the distance gradient, which is exact for a straight
/// edge and is what makes a shape thinner than a pixel come out FAINTER rather
/// than snapping to a full pixel — the same bargain epaint's feathering makes,
/// so a hairline ribbon reads the way it does through the tessellator.
fn inside(d: f32, edge: f32) -> f32 {
    let f = max(locals.feather, 1e-6);
    return clamp((edge - d) / f + 0.5, 0.0, 1.0);
}

/// How much of the outline survives at distance `d` outside the note: solid
/// against the note's edge, fading over the last `outline_fade` points of its
/// reach, gone by `reach`.
///
/// The reach is a parameter rather than read off the instance because the cap
/// at a led note's own end is given a shorter one ([`cap_coverage`]) — the same
/// band, measured against a different edge and allowed less room.
///
/// TWO numbers rather than one: a fade tied to the reach makes a wider outline
/// always a blurrier one, so how far it stands out and how softly it ends are
/// set apart. A fade at or past the reach fades the whole of it, from the note's
/// edge outward; a fade of 0 is a hard edge.
///
/// `d` is clamped at the note's edge rather than run on inward, which is what
/// makes an outline of no reach paint NOTHING: run inward, its ramp would still
/// be at full coverage across the note's own antialiased boundary, and every
/// note would wear a dark fringe with the outline turned off.
///
/// The ramp is never narrower than one pixel, so a hard-edged outline is still
/// an antialiased one. Under a pixel of reach the whole outline comes out
/// FAINTER instead of snapping to a pixel of full black — [`inside`]'s bargain,
/// taken here as well so the two edges of a hairline agree.
///
/// And never wider than the reach either, so a fade set past it eats outward
/// rather than into the coverage against the note: whatever the two are set to,
/// the outline is solid where it meets the note's edge.
fn outline_coverage(in: VertexOut, d: f32, reach: f32) -> f32 {
    let w = max(min(in.outline_fade, reach), max(locals.feather, 1e-6));
    return clamp((reach - max(d, 0.0)) / w, 0.0, 1.0);
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
    if (in.lead <= 0.0) {
        return 1.0;
    }
    let f = max(locals.feather, 1e-6);
    let u = in.local.y + in.half_extent.y;
    // The lead's own coverage: its opacity, taken out over the fade at the tip.
    var led = in.lead_alpha;
    if (in.lead_fade > 0.0) {
        led = in.lead_alpha * clamp(u / max(min(in.lead_fade, in.lead), f), 0.0, 1.0);
    }
    // ...and the note's, which is solid. 1 inside the note, 0 in the lead, a
    // pixel wide in between.
    let note = clamp((u - in.lead) / f + 0.5, 0.0, 1.0);
    return mix(led, 1.0, note);
}

/// Signed distance from this fragment to the note's own box, in points:
/// negative inside it, positive outside, and measured PERPENDICULAR to the
/// note's long edges rather than along the pitch axis.
fn box_distance(in: VertexOut) -> f32 {
    return box_distance_trimmed(in, 0.0);
}

/// The same distance, to the box with its LEADING end pulled in by `trim`
/// points. `trim` of the lead is the distance to the NOTE inside a box that
/// carries one; 0 is the box itself.
///
/// Pulling one end in shortens the box by `trim` and slides its center half
/// that far along depth. Only the depth term moves: `across` is measured from
/// the note's center LINE rather than its center point, and the slide runs
/// along that line, so a sheared box comes out the same number either way and
/// the shear needs no correction of its own.
fn box_distance_trimmed(in: VertexOut, trim: f32) -> f32 {
    let slope = in.shear;
    // A bent note is a sheared box: its long edges run at `slope`, its ends
    // stay square across the depth axis. Shearing the sample point back
    // makes it a box again, and dividing by the shear's length turns the
    // sheared offset into a true perpendicular distance — so the rim bands
    // keep their thickness on a glide instead of thinning with its angle.
    let skew = sqrt(1.0 + slope * slope);
    let across = (in.local.x - slope * in.local.y) / skew;
    let half_across = in.half_extent.x / skew;

    // Box distance. Square corners, always: a note is a rectangle in the
    // pane's two axes, and rounding one was a setting until it turned out to
    // be doing nothing a piano roll wants — on the notes short enough for it
    // to show (a tapped key), the radius clamps to the note's own half-length
    // and turns it into a bead. The outline's own corners are round, being a
    // constant distance from a square one, and that is the shape a note wants
    // wrapped around it.
    let along = in.local.y - 0.5 * trim;
    let half_along = in.half_extent.y - 0.5 * trim;
    let q = vec2<f32>(abs(across) - half_across, abs(along) - half_along);
    return min(max(q.x, q.y), 0.0) + length(max(q, vec2<f32>(0.0)));
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
    let reach = min(in.cap_reach, in.outline_reach);
    if (in.lead <= 0.0 || reach <= 0.0) {
        return 0.0;
    }
    let d = box_distance_trimmed(in, in.lead);
    return outline_coverage(in, d, reach) * (1.0 - inside(d, 0.0));
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
/// neighbour's body — drawn in the pass after this one — covers it.
///
/// The ramp is what pairs with it. Coverage here is `1 - fill` where the body
/// is `fill`, so the two sum to one across the note's antialiased boundary and
/// the seam between them never shows what is behind the note — the same
/// arithmetic the two had when they were composited in one fragment, split
/// across two passes.
///
/// [`cap_coverage`] is the second shape this layer draws, standing against the
/// end of the note INSIDE a box that carries a lead — a place the mask above
/// keeps the wrap out of, correctly, and where an edge nonetheless is.
fn outline_color(in: VertexOut) -> vec4<f32> {
    let d = box_distance(in);
    let wrap =
        outline_coverage(in, d, in.outline_reach) * (1.0 - inside(d, 0.0)) * lead_coverage(in);
    return in.outline * max(wrap, cap_coverage(in));
}

/// Premultiplied gamma-space color of the BODY layer: the note, solid in its
/// own color right to its edge — except at a leading tip that is set to fade,
/// where [`lead_coverage`] takes it out.
fn core_color(in: VertexOut) -> vec4<f32> {
    return in.core * inside(box_distance(in), 0.0) * lead_coverage(in);
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
