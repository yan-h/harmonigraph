// Lattice light is gamma-encoded premultiplied RGBA. No palette or retained color.
struct Settings {
    stars: StarUniforms,
    depth: f32,
    strength: f32,
    accumulation: f32,
    padding: f32,
};
@group(0) @binding(0) var<uniform> settings: Settings;
@group(0) @binding(1) var source: texture_2d<f32>;
@group(0) @binding(2) var cloud_sampler: sampler;
@group(1) @binding(0) var star_atlas: texture_2d<u32>;
@group(1) @binding(1) var star_halos: texture_2d_array<f32>;
@group(1) @binding(2) var star_halos_b: texture_2d_array<f32>;
@group(1) @binding(3) var star_halos_c: texture_2d_array<f32>;
@group(1) @binding(4) var cloud_tone: texture_2d<f32>;
struct TileVertex {
    @builtin(position) position: vec4<f32>,
    @location(0) @interpolate(flat) layer: u32,
};
@vertex
fn vs_stars(@builtin(vertex_index) vertex: u32, @builtin(instance_index) layer: u32) -> TileVertex {
    let uv = vec2<f32>(f32((vertex << 1u) & 2u), f32(vertex & 2u));
    return TileVertex(vec4<f32>(uv * vec2<f32>(2.0, -2.0) + vec2<f32>(-1.0, 1.0), 0.0, 1.0), layer);
}
fn star_origin() -> vec2<f32> { return settings.stars.origin; }
fn star_size() -> vec2<f32> { return settings.stars.size; }
fn star_ppp() -> f32 { return settings.stars.ppp; }
fn star_randomness() -> f32 { return settings.stars.star_randomness; }
fn star_life() -> f32 { return settings.stars.star_life; }
fn star_far() -> vec4<f32> { return settings.stars.star_far; }
fn star_near() -> vec4<f32> { return settings.stars.star_near; }
fn star_geometry() -> vec4<f32> { return settings.stars.star_geometry; }
fn star_slice(k: u32) -> StarSlice { return settings.stars.star_slices[k]; }
fn star_halo_sample(k: u32) -> StarHaloSample { return settings.stars.star_halo_samples[k]; }
fn star_floor() -> vec4<f32> { return vec4<f32>(0.0); }
// The note-glow overlap ceiling: the top of the lattice's "palette".
fn star_ceiling() -> f32 {
    return mix(clamp(GLOW_BASE * settings.strength, 0.0, 1.0), 1.0, settings.accumulation);
}
// A star is fully present and carries the light's LEVEL in its colour, as the
// spectrogram's palette position does. Carried as opacity instead, the level
// was lost wherever dense stars and fringes summed past full coverage: faint
// tails painted at the ceiling and Pattern contrast barely darkened them.
// The ceiling stands for the palette's top, so the lift is measured in it.
fn star_source(pt: vec2<f32>, rank: f32, index: i32) -> vec4<f32> {
    let light = textureSampleLevel(source, cloud_sampler, pt / settings.stars.size, 0.0);
    if light.a <= 0.0 { return vec4<f32>(0.0); }
    let ceiling = star_ceiling();
    let randomness = settings.stars.star_randomness;
    let spread = (1.0 - randomness) + randomness * (0.35 + 0.65 * rank);
    let lift = 0.5 * STAR_LIFT * rank * smoothstep(0.0, 0.15, light.a / ceiling);
    let level = clamp(light.a * spread + lift * ceiling, 0.0, ceiling);
    return vec4<f32>(light.rgb / light.a * level, 1.0);
}
@fragment
fn fs_lattice_stars(in: TileVertex) -> @location(0) vec4<f32> {
    let raw = textureSampleLevel(source, cloud_sampler, in.position.xy / settings.stars.size, 0.0);
    var result = star_color(in.position.xy);
    // Star alpha is coverage, not the light's opacity: a dim star is still a
    // whole star. The stars hide what is behind them as the light they sit in
    // does, so dark pickup stays dark pigment rather than turning transparent.
    // Alpha rises past that only to contain a star brighter than its light.
    let brightest = max(max(result.r, result.g), result.b);
    result.a = clamp(max(brightest, min(result.a, 1.0) * raw.a), 0.0, 1.0);
    // Distant gap fill can push past the ceiling; scale, retaining hue.
    result *= min(1.0, star_ceiling() / max(result.a, 1e-6));
    return mix(raw, result, settings.depth);
}
