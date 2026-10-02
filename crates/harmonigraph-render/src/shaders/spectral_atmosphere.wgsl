// Fixed-cost separable filtering of a scalar image sized for the musical blur. The source
// contains only spectral intensity, never note bodies, labels or grid rulings.
struct Cloud {
    origin: vec2<f32>,
    size: vec2<f32>,
    step: vec2<f32>,
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
/// One Gaussian tap's weighted level and weight.
fn gaussian_tap(uv: vec2<f32>, direction: vec2<f32>, offset: f32, sigma: f32) -> vec2<f32> {
    let x = offset / sigma;
    let weight = exp(-0.5 * x * x);
    let level = textureSampleLevel(source, linear_sampler, uv + direction * offset, 0.0).r;
    return vec2<f32>(level * weight, weight);
}
fn filtered(uv: vec2<f32>, step: vec2<f32>) -> vec4<f32> {
    // The source follows the blur radius at roughly two texels per sigma.
    // Integer texel offsets stay dense at every zoom, with at most seventeen
    // taps. Clip to real texels and normalize to preserve constant edge fields.
    let dims = vec2<f32>(textureDimensions(source));
    let horizontal = step.x > 0.0;
    let sigma = select(step.y, step.x, horizontal);
    let pixels = select(dims.y, dims.x, horizontal);
    if sigma * pixels < 0.25 { return textureSampleLevel(source, linear_sampler, uv, 0.0); }
    let coordinate = select(uv.y, uv.x, horizontal);
    let low = max(-3.0 * sigma, 0.5 / pixels - coordinate);
    let high = min(3.0 * sigma, 1.0 - 0.5 / pixels - coordinate);
    let direction = select(vec2<f32>(0.0, 1.0), vec2<f32>(1.0, 0.0), horizontal);
    var sum = vec2<f32>(0.0);
    let reach = i32(min(8.0, ceil(3.0 * sigma * pixels)));
    for (var i = -reach; i <= reach; i += 1) {
        let offset = f32(i) / pixels;
        if offset < low || offset > high { continue; }
        sum += gaussian_tap(uv, direction, offset, sigma);
    }
    return vec4<f32>(sum.x / sum.y, 0.0, 0.0, 1.0);
}
@fragment
fn fs_close_h(in: Vertex) -> @location(0) vec4<f32> {
    return filtered(in.uv, vec2<f32>(cloud.step.x, 0.0));
}
@fragment
fn fs_close_v(in: Vertex) -> @location(0) vec4<f32> {
    return filtered(in.uv, vec2<f32>(0.0, cloud.step.y));
}
