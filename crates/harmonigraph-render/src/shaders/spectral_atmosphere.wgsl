// Fixed-cost separable filtering of a quarter-resolution image. The source
// contains only spectral light, never note bodies, labels or grid rulings.
struct Cloud {
    origin: vec2<f32>,
    size: vec2<f32>,
    step: vec2<f32>,
    glow: f32,
    texture: f32,
    time: vec2<f32>,
    ppp: f32,
    _pad: f32,
};
@group(0) @binding(0) var source: texture_2d<f32>;
@group(0) @binding(1) var linear_sampler: sampler;
@group(0) @binding(2) var<uniform> cloud: Cloud;

struct Vertex {
    @builtin(position) position: vec4<f32>,
    @location(0) uv: vec2<f32>,
};
@vertex
fn vs_fullscreen(@builtin(vertex_index) vertex: u32) -> Vertex {
    let uv = vec2<f32>(f32((vertex << 1u) & 2u), f32(vertex & 2u));
    var out: Vertex;
    out.position = vec4<f32>(uv * vec2<f32>(2.0, -2.0) + vec2<f32>(-1.0, 1.0), 0.0, 1.0);
    out.uv = uv;
    return out;
}
fn filtered(uv: vec2<f32>, step: vec2<f32>) -> vec4<f32> {
    let weights = array<f32, 5>(0.227027, 0.1945946, 0.1216216, 0.054054, 0.016216);
    var color = vec3<f32>(0.0);
    var total = 0.0;
    for (var i = -4; i <= 4; i = i + 1) {
        let x = f32(i);
        let weight = weights[u32(abs(i))];
        let tap = uv + step * x;
        let inside = all(tap >= vec2<f32>(0.0)) && all(tap <= vec2<f32>(1.0));
        color += textureSampleLevel(source, linear_sampler, tap, 0.0).rgb * weight * select(0.0, 1.0, inside);
        total += weight;
    }
    return vec4<f32>(color / total, 1.0);
}
@fragment
fn fs_close_h(in: Vertex) -> @location(0) vec4<f32> {
    return filtered(in.uv, vec2<f32>(cloud.step.x, 0.0));
}
@fragment
fn fs_close_v(in: Vertex) -> @location(0) vec4<f32> {
    return filtered(in.uv, vec2<f32>(0.0, cloud.step.y));
}
@fragment
fn fs_wide_h(in: Vertex) -> @location(0) vec4<f32> {
    return filtered(in.uv, vec2<f32>(cloud.step.x * 3.0, 0.0));
}
@fragment
fn fs_wide_v(in: Vertex) -> @location(0) vec4<f32> {
    return filtered(in.uv, vec2<f32>(0.0, cloud.step.y * 3.0));
}
