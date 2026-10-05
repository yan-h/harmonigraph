// Lattice light is gamma-encoded premultiplied RGBA. No palette or retained color.
struct Settings {
    stars: StarUniforms,
    depth: f32,
    _pad0: f32,
    _pad1: f32,
    _pad2: f32,
};
@group(0) @binding(0) var<uniform> settings: Settings;
@group(0) @binding(1) var source: texture_2d<f32>;
@group(0) @binding(2) var light_sampler: sampler;
@group(1) @binding(0) var star_atlas: texture_2d<u32>;
@group(1) @binding(1) var star_image_texture: texture_2d<f32>;
struct TileVertex {
    @builtin(position) position: vec4<f32>,
};
@vertex
fn vs_stars(@builtin(vertex_index) vertex: u32) -> TileVertex {
    let uv = vec2<f32>(f32((vertex << 1u) & 2u), f32(vertex & 2u));
    return TileVertex(vec4<f32>(uv * vec2<f32>(2.0, -2.0) + vec2<f32>(-1.0, 1.0), 0.0, 1.0));
}
fn star_size() -> vec2<f32> { return settings.stars.size; }
fn star_randomness() -> f32 { return settings.stars.star_randomness; }
fn star_life() -> f32 { return settings.stars.star_life; }
fn star_size_variation() -> f32 { return settings.stars.star_size_variation; }
fn star_image_size() -> vec2<f32> { return settings.stars.star_image_size; }
fn star_slice(k: u32) -> StarSlice { return settings.stars.star_slices[k]; }
fn star_floor() -> vec4<f32> { return vec4<f32>(0.0); }
fn star_image_at(uv: vec2<f32>) -> vec4<f32> {
    return textureSampleLevel(star_image_texture, light_sampler, uv, 0.0);
}
// The spectrogram's star over a linear palette: the light's hue from black at
// level 0 to full brightness at 1, the level being the light's opacity. The
// star is whole and its level is in its colour, as a palette position is.
// Carried as opacity instead, the level was lost wherever dense stars and
// fringes summed past full coverage: faint tails painted at the glow's
// ceiling and Pattern contrast barely darkened them. Pinned at that ceiling
// instead of full brightness, Variation could only dim a glow's centre.
fn star_source(pt: vec2<f32>, rank: f32, index: i32) -> vec4<f32> {
    let light = textureSampleLevel(source, light_sampler, pt / settings.stars.size, 0.0);
    if light.a <= 0.0 { return vec4<f32>(0.0); }
    let randomness = settings.stars.star_randomness;
    let spread = (1.0 - randomness) + randomness * (0.35 + 0.65 * rank);
    let lift = 0.5 * STAR_LIFT * rank * smoothstep(0.0, 0.15, light.a);
    let level = clamp(light.a * spread + lift, 0.0, 1.0);
    return vec4<f32>(light.rgb / light.a * level, 1.0);
}
@fragment
fn fs_lattice_stars(in: TileVertex) -> @location(0) vec4<f32> {
    let raw = textureSampleLevel(source, light_sampler, in.position.xy / settings.stars.size, 0.0);
    var result = star_color(in.position.xy);
    // Star alpha is coverage, not the light's opacity: a dim star is still a
    // whole star. The stars hide what is behind them as the light they sit in
    // does, so dark pickup stays dark pigment rather than turning transparent.
    // Alpha rises past that only to contain a star brighter than its light.
    let brightest = max(max(result.r, result.g), result.b);
    result.a = max(brightest, min(result.a, 1.0) * raw.a);
    return mix(raw, result, settings.depth);
}
