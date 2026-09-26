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
struct StarSlice {
    metal::float2 offset;
    metal::float2 spread;
    float cell;
    float sigma;
    float cap;
    float defocus;
    float occupancy;
    float fringe;
    float fringe_reach;
    float reach;
    float life;
    float _pad0_;
    float _pad1_;
    float _pad2_;
};
struct type_5 {
    StarSlice inner[5];
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
    uint tone_baked;
    metal::float2 drift;
    float cloud_depth;
    float scale_size;
    float scale_variety;
    float scale_refract;
    uint cloud_style;
    float wash_size;
    float wash_fuzz;
    float wash_lobe;
    float wash_refract;
    float wash_layers;
    uint tile_cells;
    uint pitch_vertical;
    float star_randomness;
    float star_glow;
    float star_wander;
    float star_time;
    metal::float2 _star_pad;
    type_5 star_slices;
};
constant float CLOUD_UNITS = 10.0;
constant float CLOUD_TILE_ROT_COS = 0.8;
constant float CLOUD_TILE_ROT_SIN = 0.6;
constant float SCALE_CELLS = 2.7272727;
constant float DOME_RADIUS = 1.15;
constant float DOME_JITTER = 0.3;
constant float DOME_RADIUS_MIN = 0.95;
constant float DOME_RADIUS_MAX = 1.32;
constant float DOME_UNION = 9.0;
constant float DOME_VARIETY_GAIN = 5.0;
constant float DOME_FACE = 1.5122874;
constant float DOME_LACUNARITY = 2.1;
constant float DOME_FINE_GAIN = 0.22;
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
constant uint STAR_SLICES = 5u;
constant float STAR_PANE = 540.0;
constant float STAR_JITTER = 0.6;
constant int STAR_HASH_PERIOD = 4096;
constant float STAR_WANDER_PERIOD = 400.0;
constant uint STAR_LIFE_PERIOD = 4096u;
constant float STAR_FADE = 0.2;
constant float STAR_EXPOSURE = 1.5;
constant float STAR_LIFT = 0.18;
constant float STAR_OVER_GROUND = 6.0;
constant float STAR_RING_FADE = 0.7;
constant float STAR_TAU = 6.2831855;

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
