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
/// One tap's weighted level and its weight, so the two quadratures below
/// accumulate the same way and differ only in where they put their taps.
fn gaussian_tap(uv: vec2<f32>, direction: vec2<f32>, offset: f32, sigma: f32) -> vec2<f32> {
    let x = offset / sigma;
    let weight = exp(-0.5 * x * x);
    let level = textureSampleLevel(source, linear_sampler, uv + direction * offset, 0.0).r;
    return vec2<f32>(level * weight, weight);
}
fn filtered(uv: vec2<f32>, step: vec2<f32>, close: bool) -> vec4<f32> {
    // The source resolution follows the close radius (roughly two texels
    // per sigma). Seventeen taps therefore remain dense at every zoom; the
    // wide pass reads an already low-passed field. Clip the integration range
    // to real texels and normalize there, preserving constant fields at edges.
    //
    // Which quadrature to use is decided by the kernel's width in TEXELS
    // rather than by which pass asks. Where the whole +-3 sigma support is
    // narrower than seventeen texels, seventeen sub-texel-spaced taps ask the
    // grid for more than it holds, and integer offsets on the texel centers
    // are both the better rule and the cheaper one: they beat nothing against
    // near-Nyquist broadband patterns, and the loop stops where the Gaussian
    // does instead of always stepping seventeen times. The close pass is
    // always in that regime, since its source follows its own radius. The
    // wide pass joins it whenever its axis was left at full resolution
    // because the softness on it is well under a pixel — which is the whole
    // time axis of a zoomed-out pane, where a 600 s span puts a wide sigma of
    // two thirds of a device pixel under seventeen taps.
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
    if close || 6.0 * sigma * pixels < 17.0 {
        // The taps within +-3 sigma, and never more than the seventeen the
        // sparse arm spends — which is what keeps this bit-identical to the
        // close pass's own loop, where the eight either side were stepped
        // through and the ones past the Gaussian skipped.
        let reach = i32(min(8.0, ceil(3.0 * sigma * pixels)));
        for (var i = -reach; i <= reach; i += 1) {
            let offset = f32(i) / pixels;
            if offset < low || offset > high { continue; }
            sum += gaussian_tap(uv, direction, offset, sigma);
        }
    } else {
        for (var i = 0u; i < 17u; i += 1u) {
            sum += gaussian_tap(uv, direction, mix(low, high, (f32(i) + 0.5) / 17.0), sigma);
        }
    }
    return vec4<f32>(sum.x / sum.y, 0.0, 0.0, 1.0);
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
