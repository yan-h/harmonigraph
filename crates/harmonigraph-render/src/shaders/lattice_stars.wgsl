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
fn star_settings() -> StarUniforms { return settings.stars; }
fn star_floor() -> vec4<f32> { return vec4<f32>(0.0); }
fn star_source(pt: vec2<f32>, rank: f32, index: i32) -> vec4<f32> {
    let light = textureSampleLevel(source, cloud_sampler, pt / settings.stars.size, 0.0);
    if light.a <= 0.0 { return vec4<f32>(0.0); }
    let randomness = settings.stars.star_randomness;
    let spread = (1.0 - randomness) + randomness * (0.35 + 0.65 * rank);
    let lift = 0.5 * STAR_LIFT * rank * smoothstep(0.0, 0.15, light.a);
    return vec4<f32>(light.rgb / light.a, clamp(light.a * spread + lift, 0.0, 1.0));
}
@fragment
fn fs_lattice_stars(in: TileVertex) -> @location(0) vec4<f32> {
    let raw = textureSampleLevel(source, cloud_sampler, in.position.xy / settings.stars.size, 0.0);
    var result = star_color(in.position.xy);
    // The five material depths cannot evade the note-glow overlap ceiling.
    // Scale the bounded coverage instead of clipping bright regions flat.
    // Premultiplied channels move together, retaining hue and transparent silence.
    let ceiling = mix(clamp(GLOW_BASE * settings.strength, 0.0, 1.0), 1.0, settings.accumulation);
    result *= ceiling;
    return mix(raw, result, settings.depth);
}
