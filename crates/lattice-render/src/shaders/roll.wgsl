// The piano roll's notes: one instanced quad per note segment, with the
// ribbon, its fill, its black outline and its white keyline all falling out
// of a signed distance field.
//
// Nothing here is tessellated. A note is four vertices whatever its shape,
// and the concentric bands the egui path drew as separate stroked rounded
// rects are read off the distance instead — bands cost nothing but a
// compare, which is also why every one of them is a setting: a width, a
// fill, and which edges the rim rides all cost the same nothing.
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
    /// Half extents of the note's own outline, same two axes.
    @location(1) @interpolate(flat) half_extent: vec2<f32>,
    /// (shear, corner radius, outline width, interior fill).
    @location(2) @interpolate(flat) shape: vec4<f32>,
    /// (black outline width, white keyline width, which edges the rim rides,
    /// spare). Widths are in points, and either is zero when that band is
    /// turned off. See [`rim_mask`] for the edge modes.
    @location(3) @interpolate(flat) rim: vec4<f32>,
    /// Premultiplied, gamma-space, exactly as egui carries `Color32`.
    @location(4) @interpolate(flat) core: vec4<f32>,
    @location(5) @interpolate(flat) border: vec4<f32>,
    @location(6) @interpolate(flat) glow: vec4<f32>,
};

@vertex
fn vs_note(
    @builtin(vertex_index) vertex: u32,
    @location(0) center: vec2<f32>,
    @location(1) half_extent: vec2<f32>,
    @location(2) shape: vec4<f32>,
    @location(3) rim: vec4<f32>,
    @location(4) core: vec4<f32>,
    @location(5) border: vec4<f32>,
    @location(6) glow: vec4<f32>,
) -> VertexOut {
    // Triangle-strip corners: (-1,-1) (1,-1) (-1,1) (1,1).
    let corner = vec2<f32>(
        select(-1.0, 1.0, (vertex & 1u) == 1u),
        select(-1.0, 1.0, (vertex & 2u) == 2u),
    );

    let slope = shape.x;
    // How far outside its own outline a note can paint: half the outline
    // (the stroke is centered), the two rim bands standing outside that,
    // and the antialiasing ramp.
    let reach = shape.z * 0.5 + rim.x + rim.y + locals.feather;
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
    out.shape = shape;
    out.rim = rim;
    out.core = core;
    out.border = border;
    out.glow = glow;
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
/// `edge`. The note's interior fill, and the cut the rim mask makes.
fn inside(d: f32, edge: f32) -> f32 {
    let f = max(locals.feather, 1e-6);
    return clamp((edge - d) / f + 0.5, 0.0, 1.0);
}

/// The corner radius a note can actually hold.
///
/// Clamped to half the ribbon's thickness, or a short or thin note rounds
/// itself inside out — and then TAPERED away on a note too short to carry
/// it. Clamping alone (against the note's own length) rounds a tapped key
/// into a stadium: a few points long, a note becomes a bead, and a run of
/// taps a string of beads whose rims curl around neighbours they are nearly
/// touching. The taper is smooth in the length, so a held note growing out of
/// a tap rounds up gradually instead of popping the moment it clears its own
/// radius.
fn corner_radius(want: f32, half_across: f32, half_depth: f32) -> f32 {
    let r = min(want, half_across);
    let taper = smoothstep(0.0, 2.0 * max(r, 1e-6), half_depth);
    return min(r * taper, half_depth);
}

/// How much of the rim survives at this fragment, per the Edge shape setting:
/// 0 all the way around, 1 the long edges only, 2 the ends only.
///
/// The rim stands OUTSIDE the note, so along whichever axis it is allowed to
/// grow it reaches into the note's surroundings — and along time those
/// surroundings are the next note. Repeats of one key butt together there, so
/// a rim that wraps the ends paints each note's halo over its neighbour.
/// Mode 1 cuts the rim at the note's ends, leaving rails that stop where the
/// note does; mode 2 is the mirror image, a marker at the onset and release
/// and nothing along the length.
///
/// The cut is at the note's PAINTED extent — its box plus the half outline
/// that straddles the boundary — so a rail runs the full length of the ink it
/// edges rather than stopping short of it.
fn rim_mask(in: VertexOut, across: f32, half_across: f32, inner: f32) -> f32 {
    let mode = in.rim.z;
    if mode < 0.5 {
        return 1.0;
    }
    if mode < 1.5 {
        return inside(abs(in.local.y), in.half_extent.y + inner);
    }
    return inside(abs(across), half_across + inner);
}

/// Premultiplied gamma-space color of one fragment of a note.
fn note_color(in: VertexOut) -> vec4<f32> {
    let slope = in.shape.x;
    // A bent note is a sheared box: its long edges run at `slope`, its ends
    // stay square across the depth axis. Shearing the sample point back
    // makes it a box again, and dividing by the shear's length turns the
    // sheared offset into a true perpendicular distance — so the rim bands
    // keep their thickness on a glide instead of thinning with its angle.
    let skew = sqrt(1.0 + slope * slope);
    let across = (in.local.x - slope * in.local.y) / skew;
    let half_across = in.half_extent.x / skew;

    // Rounded-box distance, at whatever radius this note can hold.
    let radius = corner_radius(in.shape.y, half_across, in.half_extent.y);
    let q = vec2<f32>(
        abs(across) - (half_across - radius),
        abs(in.local.y) - (in.half_extent.y - radius),
    );
    let d = min(max(q.x, q.y), 0.0) + length(max(q, vec2<f32>(0.0))) - radius;

    // Reading outward from the note's own outline: its interior at whatever
    // Fill asks for (0 leaves it hollow and the spectrogram shows straight
    // through), its color on the outline itself, then the solid black outline
    // hugging that, then the white keyline riding the black's outer edge.
    //
    // The two rim bands are windows on the same box filter as the outline, so
    // they tile the distance without ever overlapping it — which is what keeps
    // the rim OUTSIDE the note however thin the ribbon is.
    let inner = in.shape.z * 0.5;
    let dark = inner + in.rim.x;
    let light = dark + in.rim.y;
    var out = in.core * in.shape.w * inside(d, -inner);
    out += in.core * band(d, -inner, inner);
    let mask = rim_mask(in, across, half_across, inner);
    out += in.border * band(d, inner, dark) * mask;
    out += in.glow * band(d, dark, light) * mask;
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
