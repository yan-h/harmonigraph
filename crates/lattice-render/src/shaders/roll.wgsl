// The piano roll's notes: one instanced quad per note segment, with the
// ribbon, its black outline and its white keyline all falling out of a
// signed distance field.
//
// Nothing here is tessellated. A note is four vertices whatever its shape,
// and the three concentric bands the egui path drew as three separate
// stroked rounded rects are read off the distance instead — bands cost
// nothing but a compare.
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
    /// (shear, corner radius, outline width, spare).
    @location(2) @interpolate(flat) shape: vec4<f32>,
    /// Thickness of the black outline and the white keyline, in points.
    /// Both zero when the Edge setting is off.
    @location(3) @interpolate(flat) rim: vec2<f32>,
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
    @location(3) rim: vec2<f32>,
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

    // Rounded-box distance. The radius is clamped to what the note can
    // actually hold, so a short or thin ribbon rounds to a stadium rather
    // than turning itself inside out.
    let radius = min(in.shape.y, min(half_across, in.half_extent.y));
    let q = vec2<f32>(
        abs(across) - (half_across - radius),
        abs(in.local.y) - (in.half_extent.y - radius),
    );
    let d = min(max(q.x, q.y), 0.0) + length(max(q, vec2<f32>(0.0))) - radius;

    // Reading outward from the note's own outline: its color, then the
    // solid black outline hugging it, then the white glow riding the
    // black's outer edge. Inside the outline nothing is painted — a note
    // is a hollow ribbon, and the spectrogram shows through it.
    let inner = in.shape.z * 0.5;
    let dark = inner + in.rim.x;
    let light = dark + in.rim.y;
    var out = in.core * band(d, -inner, inner);
    out += in.border * band(d, inner, dark);
    out += in.glow * band(d, dark, light);
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
