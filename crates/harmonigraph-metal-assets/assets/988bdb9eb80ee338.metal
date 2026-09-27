// language: metal3.2
#include <metal_stdlib>
#include <simd/simd.h>

using metal::uint;

struct Settings {
    metal::float2 size;
    float cell;
    float depth;
    metal::float2 drift;
    float refract;
    float layers;
};
struct Vertex {
    metal::float4 position;
    metal::float2 uv;
    char _pad2[8];
};
constant float CLOUD_TILE_ROT_SIN = 0.6;
constant float CLOUD_TILE_ROT_COS = 0.8;
constant int WASH_RING = 2;
constant float WASH_JITTER = 0.4;
constant float WASH_RADIUS_MIN = 1.17;
constant float WASH_RADIUS_MAX = 1.91;
constant float WASH_CELLS = 5.25;
constant float WASH_LACUNARITY = 2.1;
constant float WASH_FINE_OCCUPANCY = 0.2;
constant float WASH_WARP = 0.45;
constant float WASH_WARP_SCALE = 0.9;
constant float WASH_FBM_FINE = 2.07;
constant float WASH_FBM_FINE_TILED = 2.0;
constant float DOME_RADIUS = 1.15;
constant float DOME_JITTER = 0.3;
constant float DOME_RADIUS_MIN = 0.95;
constant float DOME_RADIUS_MAX = 1.32;
constant float DOME_UNION = 9.0;
constant float DOME_VARIETY_GAIN = 5.0;
constant float DOME_FACE = 1.5122874;
constant float DOME_LACUNARITY = 2.1;
constant float DOME_FINE_GAIN = 0.22;
constant float PERIOD = 40.0;

metal::float2 rotate_watercolor_tile_vector_for(
    metal::float2 v,
    uint pitch_vertical
) {
    metal::float2 semantic = (pitch_vertical == 1u) ? v : metal::float2(v.y, v.x);
    metal::float2 turned = metal::float2((CLOUD_TILE_ROT_COS * semantic.x) - (CLOUD_TILE_ROT_SIN * semantic.y), (CLOUD_TILE_ROT_SIN * semantic.x) + (CLOUD_TILE_ROT_COS * semantic.y));
    return (pitch_vertical == 1u) ? turned : metal::float2(turned.y, turned.x);
}

metal::float2 watercolor_tile_uv_for(
    metal::float2 r,
    float period,
    uint pitch_vertical_1
) {
    metal::float2 semantic_1 = (pitch_vertical_1 == 1u) ? r : metal::float2(r.y, r.x);
    return metal::float2((CLOUD_TILE_ROT_COS * semantic_1.x) + (CLOUD_TILE_ROT_SIN * semantic_1.y), (-0.6 * semantic_1.x) + (CLOUD_TILE_ROT_COS * semantic_1.y)) / metal::float2(period);
}

struct fs_materialInput {
    metal::float2 uv [[user(loc0), center_perspective]];
};
struct fs_materialOutput {
    metal::float4 member [[color(0)]];
};
fragment fs_materialOutput fs_material(
  fs_materialInput varyings [[stage_in]]
, metal::float4 position [[position]]
, constant Settings& settings [[buffer(0)]]
, metal::texture2d<float, metal::access::sample> source [[texture(0)]]
, metal::sampler source_sampler [[sampler(0)]]
, metal::texture2d<float, metal::access::sample> tile_a [[texture(1)]]
, metal::texture2d<float, metal::access::sample> tile_b [[texture(2)]]
, metal::sampler tile_sampler [[sampler(1)]]
) {
    const Vertex in = { position, varyings.uv };
    metal::float2 _e4 = settings.size;
    metal::float2 _e8 = settings.size;
    float _e14 = settings.cell;
    metal::float2 p = ((in.uv * _e4) - (_e8 * 0.5)) / metal::float2(_e14);
    metal::float2 _e19 = settings.drift;
    metal::float2 _e23 = watercolor_tile_uv_for(p + _e19, PERIOD, 1u);
    metal::float4 a = tile_a.sample(tile_sampler, _e23, metal::level(0.0));
    metal::float4 b = tile_b.sample(tile_sampler, _e23, metal::level(0.0));
    metal::float2 _e34 = rotate_watercolor_tile_vector_for(a.xy, 1u);
    float _e37 = settings.cell;
    metal::float2 _e41 = settings.size;
    metal::float2 coarse = (_e34 * _e37) / _e41;
    metal::float2 _e45 = rotate_watercolor_tile_vector_for(b.xy, 1u);
    float _e48 = settings.cell;
    metal::float2 _e53 = settings.size;
    metal::float2 fine = (_e45 * _e48) / (WASH_LACUNARITY * _e53);
    metal::float4 raw = source.sample(source_sampler, in.uv, metal::level(0.0));
    float _e66 = settings.refract;
    metal::float4 first = source.sample(source_sampler, in.uv + (coarse * _e66), metal::level(0.0));
    float _e76 = settings.refract;
    metal::float4 second = source.sample(source_sampler, in.uv + (fine * _e76), metal::level(0.0));
    float _e83 = settings.layers;
    float _e89 = settings.depth;
    return fs_materialOutput { metal::mix(raw, metal::mix(first, second, _e83 * b.w), _e89) };
}
