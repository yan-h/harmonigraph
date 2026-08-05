// The piano roll's notes: one instanced quad per note segment — a solid
// rectangle in the note's own color, with a white keyline along its long
// edges and a black shade standing outside that, all falling out of a signed
// distance field.
//
// Nothing here is tessellated. A note is four vertices whatever its shape,
// and the bands the egui path drew as separate stroked rounded rects are read
// off the distance instead, which costs a compare rather than a second and
// third shape.
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
    /// Width of EACH rim band in points — the white keyline and the black
    /// shade beyond it are the same thickness — and zero when Edge turns the
    /// rim off. Both ride the note's long edges only — see [`rail_mask`].
    @location(3) @interpolate(flat) keyline: f32,
    /// Premultiplied, gamma-space, exactly as egui carries `Color32`.
    @location(4) @interpolate(flat) core: vec4<f32>,
    @location(5) @interpolate(flat) glow: vec4<f32>,
    @location(6) @interpolate(flat) shade: vec4<f32>,
};

@vertex
fn vs_note(
    @builtin(vertex_index) vertex: u32,
    @location(0) center: vec2<f32>,
    @location(1) half_extent: vec2<f32>,
    @location(2) shear: f32,
    @location(3) keyline: f32,
    @location(4) core: vec4<f32>,
    @location(5) glow: vec4<f32>,
    @location(6) shade: vec4<f32>,
) -> VertexOut {
    // Triangle-strip corners: (-1,-1) (1,-1) (-1,1) (1,1).
    let corner = vec2<f32>(
        select(-1.0, 1.0, (vertex & 1u) == 1u),
        select(-1.0, 1.0, (vertex & 2u) == 2u),
    );

    let slope = shear;
    // How far outside its own box a note can paint: the two rim bands standing
    // against it, one keyline wide each, and the antialiasing ramp.
    let reach = 2.0 * keyline + locals.feather;
    // The quad is the note's bounding box grown by `reach` on every side.
    // Growing a box by a disc of radius r grows its bounds by exactly r,
    // so this covers the rim however the note is slanted; the shear term
    // is the pitch drift of a bent note's center line between its ends.
    let extent = vec2<f32>(
        half_extent.x + abs(slope) * half_extent.y + reach,
        half_extent.y + reach,
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
    out.keyline = keyline;
    out.core = core;
    out.glow = glow;
    out.shade = shade;
    return out;
}

/// Coverage of the band `inner..outer` (distances from the note's outline)
/// at signed distance `d`: how much of a one-pixel-wide window centered on
/// `d` lands inside the band.
///
/// A box filter along the distance gradient, which is exact for a straight
/// edge and is what makes a band thinner than a pixel come out FAINTER
/// rather than snapping to a full pixel — the same bargain epaint's
/// feathering makes, so a hairline keyline reads the way it did when the
/// roll went through the tessellator.
fn band(d: f32, inner: f32, outer: f32) -> f32 {
    let f = max(locals.feather, 1e-6);
    let lo = max(inner, d - 0.5 * f);
    let hi = min(outer, d + 0.5 * f);
    return clamp((hi - lo) / f, 0.0, 1.0);
}

/// [`band`] with no inner bound: coverage of everything on the near side of
/// `edge`. The note's interior fill, and the cut the rail mask makes.
fn inside(d: f32, edge: f32) -> f32 {
    let f = max(locals.feather, 1e-6);
    return clamp((edge - d) / f + 0.5, 0.0, 1.0);
}

/// How much of the rim survives at this fragment: both bands ride the note's
/// two LONG edges and are cut at its ends, leaving rails that stop where the
/// note does.
///
/// Not a choice. The rim stands OUTSIDE the note, so along whichever axis it
/// is allowed to grow it reaches into the note's surroundings — and along
/// time those surroundings are the next note. Repeats of one key butt together
/// there, so a rim that wrapped the ends paints each note's halo over its
/// neighbour, and the ends are also where a note is shortest, so a cap there
/// is an edge the length of the note itself. That holds for the black shade as
/// much as the keyline: capped, it would draw a dark seam between two repeats
/// of a key that were played as one run.
///
/// The cut is at the note's box, which is exactly where its ink ends now that
/// a note is a plain solid rectangle — so a rail runs the full length of the
/// note it edges, corner to corner, rather than tapering off short of it. (It
/// used to be cut at the box plus the half outline, and the band's own corner
/// rounding then took its outer edge off half an outline early.)
fn rail_mask(in: VertexOut) -> f32 {
    return inside(abs(in.local.y), in.half_extent.y);
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
    // and turns it into a bead.
    let q = vec2<f32>(abs(across) - half_across, abs(in.local.y) - in.half_extent.y);
    let d = min(max(q.x, q.y), 0.0) + length(max(q, vec2<f32>(0.0)));

    // Reading outward: the note, solid in its own color right to its edge, the
    // white keyline standing against that edge, then the black shade standing
    // against the keyline — one keyline width each.
    //
    // Every band is a window on the same box filter as the fill, so they tile
    // the distance without ever overlapping — which is what keeps them OUTSIDE
    // the note however thin the ribbon is. A centered stroke would grow inward
    // exactly as much as outward, and at the ribbon widths this pane is used at
    // the two long edges would meet in the middle and paint the note white.
    //
    // Summed rather than composited in order, which is exact here and only
    // here: the bands are disjoint windows on one distance, so each fragment's
    // coverages add to at most one and premultiplied colors add with them.
    let rail = rail_mask(in);
    var out = in.core * inside(d, 0.0);
    out += in.glow * band(d, 0.0, in.keyline) * rail;
    out += in.shade * band(d, in.keyline, 2.0 * in.keyline) * rail;
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
