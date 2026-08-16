// The marks a pane wants bloomed, redrawn as instanced quads filled from a
// circle's signed distance — one quad per disc, four vertices, no tessellation.
//
// Nothing here is a picture a reader ever looks at. The pane draws its own
// marks through egui's tessellator and keeps them; this is the copy the bloom
// chain reads (`crate::glow`), so what it owes that picture is the same ink in
// the same places, and NOT the same crispness — the first thing the chain does
// with it is threshold it at half size and blur it at a quarter.
//
// Coordinates arrive in egui POINTS, exactly as egui's own vertex shader takes
// them, and `vs_disc` does the same screen->clip mapping.

struct Locals {
    /// Where the viewport being drawn into starts, in egui points (xy), and how
    /// big it is (zw). The instances are in SURFACE points and the texture
    /// covers the pane's rect alone, so this is what puts a mark in the texture
    /// where it stands on screen.
    viewport: vec4<f32>,
    /// x: width of the antialiasing ramp in points — one pixel of the texture
    /// being drawn into, which is not the display's pixel. The rest is the 16
    /// bytes a uniform block takes.
    feather: vec4<f32>,
};

@group(0) @binding(0) var<uniform> locals: Locals;

struct VertexOut {
    @builtin(position) position: vec4<f32>,
    /// Offset from the disc's centre, in points.
    @location(0) local: vec2<f32>,
    /// The disc's radius, in points.
    @location(1) @interpolate(flat) radius: f32,
    /// Premultiplied, gamma-space, exactly as egui carries `Color32`.
    @location(2) @interpolate(flat) color: vec4<f32>,
};

@vertex
fn vs_disc(
    @builtin(vertex_index) vertex: u32,
    @location(0) center: vec2<f32>,
    @location(1) radius: f32,
    @location(2) color: vec4<f32>,
) -> VertexOut {
    // Triangle-strip corners: (-1,-1) (1,-1) (-1,1) (1,1).
    let corner = vec2<f32>(
        select(-1.0, 1.0, (vertex & 1u) == 1u),
        select(-1.0, 1.0, (vertex & 2u) == 2u),
    );
    // The disc's own bounding box, grown by half the ramp so the outermost
    // fragment the fill can reach is inside the quad. A shortfall here CLIPS
    // ink rather than costing a little fill rate, and a clipped disc is a
    // square-shouldered halo.
    let local = corner * (radius + 0.5 * locals.feather.x);
    let in_viewport = center + local - locals.viewport.xy;

    var out: VertexOut;
    out.position = vec4<f32>(
        2.0 * in_viewport.x / locals.viewport.z - 1.0,
        1.0 - 2.0 * in_viewport.y / locals.viewport.w,
        0.0,
        1.0,
    );
    out.local = local;
    out.radius = radius;
    out.color = color;
    return out;
}

/// Premultiplied gamma-space color of the disc: its own color, solid to its
/// edge and taken out over one pixel of the target.
///
/// A box filter along the distance gradient, which is roll.wgsl's `inside`
/// bargain taken here for its reason: a disc smaller than a pixel comes out
/// FAINTER rather than snapping to a whole one, so a mark fading out of the
/// picture takes its halo with it smoothly instead of dropping it.
fn disc_color(in: VertexOut) -> vec4<f32> {
    let f = max(locals.feather.x, 1e-6);
    return in.color * clamp((in.radius - length(in.local)) / f + 0.5, 0.0, 1.0);
}

// 0-1 linear from 0-1 sRGB gamma. Lifted from egui's own shader, and used for
// the same reason: on an sRGB-aware target egui hands the hardware linear
// values and lets it encode.
fn linear_from_gamma_rgb(srgb: vec3<f32>) -> vec3<f32> {
    let cutoff = srgb < vec3<f32>(0.04045);
    let lower = srgb / vec3<f32>(12.92);
    let higher = pow((srgb + vec3<f32>(0.055)) / vec3<f32>(1.055), vec3<f32>(2.4));
    return select(higher, lower, cutoff);
}

@fragment
fn fs_disc_gamma(in: VertexOut) -> @location(0) vec4<f32> {
    return disc_color(in);
}

@fragment
fn fs_disc_linear(in: VertexOut) -> @location(0) vec4<f32> {
    let gamma = disc_color(in);
    return vec4<f32>(linear_from_gamma_rgb(gamma.rgb), gamma.a);
}
