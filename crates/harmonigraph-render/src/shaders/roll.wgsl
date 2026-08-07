// The piano roll's notes: one instanced quad per note segment — a solid
// rectangle in the note's own color, wrapped on every side by an outline that
// fades out, both falling out of a signed distance field.
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
    /// egui's screen size in points (the whole surface, not the pane).
    screen_points: vec2<f32>,
    /// Width of the antialiasing ramp in points — one physical pixel.
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
    /// Premultiplied, gamma-space, exactly as egui carries `Color32`.
    @location(5) @interpolate(flat) core: vec4<f32>,
    /// The outline's color at full coverage; the fade takes it from there.
    @location(6) @interpolate(flat) outline: vec4<f32>,
};

@vertex
fn vs_note(
    @builtin(vertex_index) vertex: u32,
    @location(0) center: vec2<f32>,
    @location(1) half_extent: vec2<f32>,
    @location(2) shear: f32,
    @location(3) outline_reach: f32,
    @location(4) outline_fade: f32,
    @location(5) core: vec4<f32>,
    @location(6) outline: vec4<f32>,
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

    var out: VertexOut;
    out.position = vec4<f32>(
        2.0 * pos.x / locals.screen_points.x - 1.0,
        1.0 - 2.0 * pos.y / locals.screen_points.y,
        0.0,
        1.0,
    );
    out.local = local;
    out.half_extent = half_extent;
    out.shear = shear;
    out.outline_reach = outline_reach;
    out.outline_fade = outline_fade;
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
/// reach, gone by `outline_reach`.
///
/// The same pair of dials the lattice's knockout gutter takes, and for the same
/// reason they are two rather than one: a fade tied to the reach makes a wider
/// outline always a blurrier one, so how far it stands out and how softly it
/// ends are set apart. A fade at or past the reach fades the whole of it, from
/// the note's edge outward; a fade of 0 is a hard edge.
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
fn outline_coverage(in: VertexOut, d: f32) -> f32 {
    let w = max(min(in.outline_fade, in.outline_reach), max(locals.feather, 1e-6));
    return clamp((in.outline_reach - max(d, 0.0)) / w, 0.0, 1.0);
}

/// Premultiplied gamma-space color of one fragment of a note.
fn note_color(in: VertexOut) -> vec4<f32> {
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
    let q = vec2<f32>(abs(across) - half_across, abs(in.local.y) - in.half_extent.y);
    let d = min(max(q.x, q.y), 0.0) + length(max(q, vec2<f32>(0.0)));

    // Reading outward: the note, solid in its own color right to its edge, then
    // the outline standing against that edge on every side and fading out.
    //
    // The outline stands entirely OUTSIDE the note, which is why it is read off
    // the distance rather than stroked along the note's path: a centered stroke
    // grows inward exactly as much as outward, and at the ribbon widths this
    // pane is used at the two long edges would meet in the middle and flood the
    // note with the outline's own color. Coverage taken at a positive distance
    // cannot reach back inside the box however thin the ribbon is.
    //
    // Composited in order rather than summed — the note over the outline —
    // since the two overlap by a ramp at the boundary. The alternative is to
    // butt the outline against the fill's own edge, which leaves the seam
    // between them showing whatever is behind the note.
    let fill = inside(d, 0.0);
    var out = in.core * fill;
    out += in.outline * outline_coverage(in, d) * (1.0 - fill);
    return out;
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
fn fs_note_gamma(in: VertexOut) -> @location(0) vec4<f32> {
    return note_color(in);
}

@fragment
fn fs_note_linear(in: VertexOut) -> @location(0) vec4<f32> {
    let gamma = note_color(in);
    return vec4<f32>(linear_from_gamma_rgb(gamma.rgb), gamma.a);
}
