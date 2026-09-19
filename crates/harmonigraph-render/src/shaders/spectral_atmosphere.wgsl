// Fixed-cost separable filtering of a scalar image sized for the musical blur. The source
// contains only spectral intensity, never note bodies, labels or grid rulings.
struct Cloud {
    origin: vec2<f32>,
    size: vec2<f32>,
    step: vec2<f32>,
    ppp: f32,
    spread: f32,
    contours: f32,
    contour_softness: f32,
    contour_strength: f32,
    // `tone_baked` in the spectrogram shader's copy; the filters never read it.
    // The two declarations are one buffer and have to keep one layout.
    tone_baked: u32,
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
fn filtered(uv: vec2<f32>, step: vec2<f32>, close: bool) -> vec4<f32> {
    // The source resolution follows the close radius (roughly two texels
    // per sigma). Seventeen taps therefore remain dense at every zoom; the
    // wide pass reads an already low-passed field. Clip the integration range
    // to real texels and normalize there, preserving constant fields at edges.
    let dims = vec2<f32>(textureDimensions(source));
    let horizontal = step.x > 0.0;
    let sigma = select(step.y, step.x, horizontal);
    let pixels = select(dims.y, dims.x, horizontal);
    if sigma * pixels < 0.25 { return textureSampleLevel(source, linear_sampler, uv, 0.0); }
    let coordinate = select(uv.y, uv.x, horizontal);
    let low = max(-3.0 * sigma, 0.5 / pixels - coordinate);
    let high = min(3.0 * sigma, 1.0 - 0.5 / pixels - coordinate);
    let direction = select(vec2<f32>(0.0, 1.0), vec2<f32>(1.0, 0.0), horizontal);
    var level = 0.0;
    var total = 0.0;
    for (var i = 0u; i < 17u; i += 1u) {
        var offset = mix(low, high, (f32(i) + 0.5) / 17.0);
        if close {
            // Integer offsets sample the source's texel centers, avoiding
            // quadrature beats against near-Nyquist broadband patterns.
            offset = (f32(i) - 8.0) / pixels;
            if offset < low || offset > high { continue; }
        }
        let x = offset / sigma;
        let weight = exp(-0.5 * x * x);
        level += textureSampleLevel(source, linear_sampler, uv + direction * offset, 0.0).r * weight;
        total += weight;
    }
    return vec4<f32>(level / total, 0.0, 0.0, 1.0);
}
@fragment
fn fs_close_h(in: Vertex) -> @location(0) vec4<f32> {
    return filtered(in.uv, vec2<f32>(cloud.step.x, 0.0), true);
}
@fragment
fn fs_close_v(in: Vertex) -> @location(0) vec4<f32> {
    return filtered(in.uv, vec2<f32>(0.0, cloud.step.y), true);
}
@fragment
fn fs_wide_h(in: Vertex) -> @location(0) vec4<f32> {
    return filtered(in.uv, vec2<f32>(cloud.step.x * 5.0, 0.0), false);
}
@fragment
fn fs_wide_v(in: Vertex) -> @location(0) vec4<f32> {
    return filtered(in.uv, vec2<f32>(0.0, cloud.step.y * 5.0), false);
}
