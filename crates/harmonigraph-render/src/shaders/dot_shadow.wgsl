// The spiral's sounding-note dots as shadow casters. Distance evaluates the
// circle SDF in the surface pass; Gaussian rasterizes coverage from the same
// field into the shared per-caster atlas and uses the common separable blur.

struct DotShadowLocals {
    screen_points: vec2<f32>,
    shadow_atlas_size: vec2<f32>,
    // σ, depth, kernel kind (Distance = 1), and whole reach, in points.
    shadow: vec4<f32>,
};

@group(0) @binding(0) var<uniform> dot_locals: DotShadowLocals;

struct DotShadowOut {
    @builtin(position) position: vec4<f32>,
    @location(0) local: vec2<f32>,
    @location(1) @interpolate(flat) radius: f32,
    @location(2) at: vec2<f32>,
    @location(3) @interpolate(flat) who: u32,
};

fn dot_corner(vertex: u32) -> vec2<f32> {
    return vec2<f32>(
        select(-1.0, 1.0, (vertex & 1u) == 1u),
        select(-1.0, 1.0, (vertex & 2u) == 2u),
    );
}

@vertex
fn vs_dot_shadow(
    @builtin(vertex_index) vertex: u32,
    @builtin(instance_index) who: u32,
    @location(0) center: vec2<f32>,
    @location(1) radius: f32,
    @location(2) color: vec4<f32>,
) -> DotShadowOut {
    let local = dot_corner(vertex) * (radius + dot_locals.shadow.w);
    let point = center + local;
    var out: DotShadowOut;
    out.position = vec4<f32>(
        2.0 * point.x / dot_locals.screen_points.x - 1.0,
        1.0 - 2.0 * point.y / dot_locals.screen_points.y,
        0.0,
        1.0,
    );
    out.local = local;
    out.radius = radius;
    out.at = point;
    out.who = who;
    return out;
}

@vertex
fn vs_dot_shadow_cell(
    @builtin(vertex_index) vertex: u32,
    @location(0) center: vec2<f32>,
    @location(1) radius: f32,
    @location(2) color: vec4<f32>,
    @location(3) box_rect: vec4<f32>,
    @location(4) box_cell: vec4<f32>,
    @location(5) box_meta: vec4<f32>,
    @location(6) box_who: vec4<f32>,
) -> DotShadowOut {
    let corner = 0.5 * (dot_corner(vertex) + vec2<f32>(1.0));
    let point = box_rect.xy + corner * box_rect.zw;
    let texel = cell_texel(point, box_rect, box_cell, box_meta.x);
    var out: DotShadowOut;
    out.position = select(
        no_quad(),
        cell_clip(texel, dot_locals.shadow_atlas_size, 1.0),
        cell_packed(box_cell) && box_who.y < 0.5 * DISTANCE_KIND,
    );
    out.local = point - center;
    out.radius = radius;
    out.at = point;
    out.who = u32(box_who.x + 0.5);
    return out;
}

fn dot_distance(in: DotShadowOut) -> f32 {
    return length(in.local) - in.radius;
}

@fragment
fn fs_dot_shadow_coverage(in: DotShadowOut) -> @location(0) vec4<f32> {
    let d = dot_distance(in);
    let aa = max(fwidth(d), 1.0e-6);
    let coverage = clamp(0.5 - d / aa, 0.0, 1.0);
    return vec4<f32>(coverage, 0.0, 0.0, 1.0);
}

@fragment
fn fs_dot_shadow(in: DotShadowOut) -> @location(0) vec4<f32> {
    var full = standoff_coverage(dot_distance(in), 2.0 * dot_locals.shadow.x);
    if dot_locals.shadow.z < 0.5 * DISTANCE_KIND {
        full = shadow_kernel(in.who, in.at);
    }
    let level = shadow_casters[in.who].shade.x;
    let alpha = 1.0 - shadow_transmittance(full, dot_locals.shadow.y, level);
    return vec4<f32>(0.0, 0.0, 0.0, alpha);
}
