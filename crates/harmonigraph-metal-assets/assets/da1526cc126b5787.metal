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

struct fs_mosaicInput {
    metal::float2 uv [[user(loc0), center_perspective]];
};
struct fs_mosaicOutput {
    metal::float4 member [[color(0)]];
};
fragment fs_mosaicOutput fs_mosaic(
  fs_mosaicInput varyings [[stage_in]]
, metal::float4 position [[position]]
, constant Settings& settings [[buffer(0)]]
, metal::texture2d<float, metal::access::sample> source [[texture(0)]]
, metal::sampler source_sampler [[sampler(0)]]
, metal::texture2d<float, metal::access::sample> tile_a [[texture(1)]]
, metal::sampler tile_sampler [[sampler(1)]]
) {
    const Vertex in = { position, varyings.uv };
    metal::float2 _e4 = settings.size;
    metal::float2 _e8 = settings.size;
    float _e14 = settings.cell;
    metal::float2 p = ((in.uv * _e4) - (_e8 * 0.5)) / metal::float2(_e14);
    metal::float2 _e19 = settings.drift;
    metal::float2 uv = (p + _e19) / metal::float2(40.0);
    metal::float4 tile = tile_a.sample(tile_sampler, uv, metal::level(0.0));
    float _e32 = settings.refract;
    float _e39 = settings.refract;
    float _e47 = settings.cell;
    metal::float2 _e51 = settings.size;
    metal::float2 offset = (((-(tile.xy) * metal::max(_e32, 0.0)) + (tile.zw * metal::max(-(_e39), 0.0))) * _e47) / _e51;
    metal::float4 raw = source.sample(source_sampler, in.uv, metal::level(0.0));
    metal::float4 facet = source.sample(source_sampler, in.uv + offset, metal::level(0.0));
    float _e66 = settings.depth;
    return fs_mosaicOutput { metal::mix(raw, facet, _e66) };
}
