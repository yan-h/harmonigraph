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
    float randomness;
    char _pad7[4];
    metal::float2 velvet_form;
    metal::float4 velvet;
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
constant float WASH_FBM_FINE = 2.0;
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

metal::float3 wash_vary_brightness(
    metal::float3 color,
    float ceiling,
    float draw,
    float amount
) {
    float peak = metal::max(color.x, metal::max(color.y, color.z));
    float headroom = metal::clamp((ceiling - peak) / metal::max(peak, 0.000001), 0.0, 1.0);
    return color * (1.0 + ((amount * draw) * headroom));
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
    float over = _e83 * b.w;
    metal::float4 painted = metal::mix(first, second, over);
    float _e94 = settings.randomness;
    metal::float3 _e95 = wash_vary_brightness(painted.xyz, painted.w, metal::mix(a.z, b.z, over), _e94);
    metal::float4 varied = metal::float4(_e95, painted.w);
    float _e100 = settings.depth;
    return fs_materialOutput { metal::mix(raw, varied, _e100) };
}
