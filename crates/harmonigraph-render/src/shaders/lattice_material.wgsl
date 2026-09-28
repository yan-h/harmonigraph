// The light stays premultiplied throughout: every lookup and crossfade acts
// on all four channels, preserving silence, colors and the overlap ceiling.
struct Settings {
    size: vec2<f32>,
    cell: f32,
    depth: f32,
    drift: vec2<f32>,
    refract: f32,
    layers: f32,
    randomness: f32,
    velvet: vec4<f32>,
};
struct Geometry { fuzz: f32, lobe: f32, variety: f32, padding: f32 };
@group(0) @binding(3) var<uniform> geometry: Geometry;
@group(0) @binding(0) var<uniform> settings: Settings;
@group(0) @binding(1) var source: texture_2d<f32>;
@group(0) @binding(2) var source_sampler: sampler;
@group(1) @binding(0) var tile_a: texture_2d<f32>;
@group(1) @binding(1) var tile_b: texture_2d<f32>;
@group(1) @binding(2) var tile_sampler: sampler;
const PERIOD: f32 = 40.0;
struct Vertex {
    @builtin(position) position: vec4<f32>,
    @location(0) uv: vec2<f32>,
};
@vertex
fn vs_fullscreen(@builtin(vertex_index) i: u32) -> Vertex {
    let uv = vec2<f32>(f32(i & 1u), f32(i >> 1u));
    return Vertex(vec4<f32>(uv.x * 2.0 - 1.0, 1.0 - uv.y * 2.0, 0.0, 1.0), uv);
}
struct TileOut {
    @location(0) a: vec4<f32>,
    @location(1) b: vec4<f32>,
};
@fragment
fn fs_tile(in: Vertex) -> TileOut {
    let field = wash_field(in.uv * PERIOD, i32(PERIOD), geometry.fuzz, geometry.lobe);
    return TileOut(vec4<f32>(field.coarse.offset, field.coarse.brightness, 0.0),
        vec4<f32>(field.fine.offset, field.fine.brightness, field.cover));
}
@fragment
fn fs_material(in: Vertex) -> @location(0) vec4<f32> {
    let p = (in.uv * settings.size - settings.size * 0.5) / settings.cell;
    let uv = watercolor_tile_uv_for(p + settings.drift, PERIOD, 1u);
    let a = textureSampleLevel(tile_a, tile_sampler, uv, 0.0);
    let b = textureSampleLevel(tile_b, tile_sampler, uv, 0.0);
    let coarse = rotate_watercolor_tile_vector_for(a.xy, 1u) * settings.cell / settings.size;
    let fine = rotate_watercolor_tile_vector_for(b.xy, 1u) * settings.cell / (WASH_LACUNARITY * settings.size);
    let raw = textureSampleLevel(source, source_sampler, in.uv, 0.0);
    let first = textureSampleLevel(source, source_sampler, in.uv + coarse * settings.refract, 0.0);
    let second = textureSampleLevel(source, source_sampler, in.uv + fine * settings.refract, 0.0);
    let over = settings.layers * b.w;
    let painted = mix(first, second, over);
    let varied = vec4<f32>(wash_vary_brightness(painted.rgb, painted.a,
        mix(a.z, b.z, over), settings.randomness), painted.a);
    return mix(raw, varied, settings.depth);
}

// Same soft-union geometry and flat-centre reading as spectrogram Mosaic.
// Its one RGBA tile carries both face and centre offsets; no second tile is needed.
@fragment
fn fs_mosaic_tile(in: Vertex) -> @location(0) vec4<f32> {
    let pile = mosaic_field(in.uv * PERIOD, i32(PERIOD), geometry.variety);
    return vec4<f32>(pile.face, pile.to_centre);
}
@fragment
fn fs_mosaic(in: Vertex) -> @location(0) vec4<f32> {
    let p = (in.uv * settings.size - settings.size * 0.5) / settings.cell;
    let uv = (p + settings.drift) / PERIOD;
    let tile = textureSampleLevel(tile_a, tile_sampler, uv, 0.0);
    let offset = (-tile.xy * max(settings.refract, 0.0) + tile.zw * max(-settings.refract, 0.0)) * settings.cell / settings.size;
    let raw = textureSampleLevel(source, source_sampler, in.uv, 0.0);
    let facet = textureSampleLevel(source, source_sampler, in.uv + offset, 0.0);
    return mix(raw, facet, settings.depth);
}

@fragment
fn fs_velvet(in: Vertex) -> @location(0) vec4<f32> {
    let body = velvet_material(source, source_sampler, in.uv * settings.size,
        settings.size, settings.cell, settings.drift, settings.velvet);
    let raw = textureSampleLevel(source, source_sampler, in.uv, 0.0);
    return mix(raw, body, settings.depth);
}
