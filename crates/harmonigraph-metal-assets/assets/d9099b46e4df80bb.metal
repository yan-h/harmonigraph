// language: metal3.2
#include <metal_stdlib>
#include <simd/simd.h>

using metal::uint;

struct VertexOut {
    metal::float4 position;
    float slab;
    float t;
    char _pad3[8];
};
struct Cloud {
    metal::float2 origin;
    metal::float2 size;
    metal::float2 step;
    float ppp;
    float spread;
    float contours;
    float contour_softness;
    float contour_strength;
    uint _pad;
    metal::float2 drift;
    float time;
    float cloud_depth;
    float scale_size;
    float scale_variety;
    float scale_refract;
    float scale_relief;
    float scale_rock;
    uint cloud_style;
    float wash_size;
    float wash_fuzz;
    float wash_ragged;
    float wash_lobe;
    float wash_refract;
    float wash_pool;
    float wash_grain;
    float wash_layers;
};
constant float CLOUD_UNITS = 10.0;
constant float SCALE_CELLS = 2.7272727;
constant float DOME_RADIUS = 1.15;
constant float DOME_JITTER = 0.3;
constant float DOME_RADIUS_MIN = 0.95;
constant float DOME_RADIUS_MAX = 1.32;
constant float DOME_UNION = 9.0;
constant float DOME_VARIETY_GAIN = 5.0;
constant float DOME_FACE = 1.5122874;
constant float ROCK_TILT = 0.3;
constant float SUN_LEAN = 1.0;
constant float SUN_KNEE = 0.03;
constant float RELIEF_FLOOR_FALL = 3.22;
constant float DOME_LACUNARITY = 2.1;
constant float DOME_FINE_GAIN = 0.22;
constant float CLOUD_SHADE = 0.64;
constant int WASH_RING = 2;
constant float WASH_JITTER = 0.4;
constant float WASH_RAGGED = 0.3;
constant float WASH_RADIUS_MIN = 1.02;
constant float WASH_RADIUS_MAX = 1.66;
constant float WASH_CELLS = 5.25;
constant float WASH_LACUNARITY = 2.1;
constant float WASH_FINE_OCCUPANCY = 0.2;
constant float WASH_WARP = 0.45;
constant float WASH_WARP_SCALE = 0.9;
constant float WASH_RAGGED_SCALE = 2.8;
constant float WASH_POOL = 0.44;
constant float WASH_GRAIN = 0.1;
constant float WASH_POOL_WIDTH = 0.55;
constant float WASH_SURF = 0.07;
constant float WASH_PIG_DEPTH = 0.35;
constant float WASH_TONE_FLOOR = 0.05;
constant float WASH_PIVOT = 0.45;
constant float WASH_LIFT_A = 1.15;
constant float WASH_LIFT_B = 0.16;
constant float WASH_BLACK_KNEE = 0.175;

float density_decode(
    float value
) {
    float y = metal::max(value, 0.0);
    return (2.0 * y) / (0.1 + metal::sqrt(0.01 + (3.6 * y)));
}

struct fs_cloud_lightInput {
    float slab [[user(loc0), center_perspective]];
    float t [[user(loc1), center_perspective]];
};
struct fs_cloud_lightOutput {
    metal::float4 member [[color(0)]];
};
fragment fs_cloud_lightOutput fs_cloud_light(
  fs_cloud_lightInput varyings [[stage_in]]
, metal::float4 position [[position]]
, metal::texture2d<float, metal::access::sample> close_light [[texture(1)]]
, metal::texture2d<float, metal::access::sample> wide_light [[texture(2)]]
, metal::sampler cloud_sampler [[sampler(0)]]
, constant Cloud& cloud [[buffer(2)]]
) {
    const VertexOut in = { position, varyings.slab, varyings.t };
    metal::float2 uv = in.position.xy / static_cast<metal::float2>(metal::uint2(wide_light.get_width(), wide_light.get_height()));
    metal::float4 _e10 = close_light.sample(cloud_sampler, uv, metal::level(0.0));
    float close = _e10.x;
    metal::float4 _e15 = wide_light.sample(cloud_sampler, uv, metal::level(0.0));
    float wide = _e15.x;
    float _e19 = cloud.spread;
    float _e21 = density_decode(metal::mix(close, wide, _e19));
    return fs_cloud_lightOutput { metal::float4(_e21, 0.0, 0.0, 1.0) };
}
