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
    float diffusion;
    float texture;
    float ppp;
    float _pad0_;
    float _pad1_;
    float _pad2_;
};

float cloud_hash(
    metal::int2 cell
) {
    uint n = {};
    n = (as_type<uint>(cell.x) * 2654435769u) ^ as_type<uint>(cell.y);
    uint _e9 = n;
    uint _e10 = n;
    n = (_e9 ^ (_e10 >> 16u)) * 2146121005u;
    uint _e16 = n;
    uint _e17 = n;
    n = (_e16 ^ (_e17 >> 15u)) * 2221713035u;
    uint _e23 = n;
    uint _e24 = n;
    n = _e23 ^ (_e24 >> 16u);
    uint _e28 = n;
    return static_cast<float>(_e28 >> 8u) / 16777216.0;
}

metal::int2 naga_f2i32(metal::float2 value) {
    return static_cast<metal::int2>(metal::clamp(value, -2147483600.0, 2147483500.0));
}

float cloud_noise(
    metal::float2 p
) {
    metal::int2 cell_1 = naga_f2i32(metal::floor(p));
    metal::float2 f = metal::fract(p);
    metal::float2 w = (f * f) * (metal::float2(3.0) - (2.0 * f));
    float _e11 = cloud_hash(cell_1);
    float _e16 = cloud_hash(as_type<metal::int2>(as_type<metal::uint2>(cell_1) + as_type<metal::uint2>(metal::int2(1, 0))));
    float _e23 = cloud_hash(as_type<metal::int2>(as_type<metal::uint2>(cell_1) + as_type<metal::uint2>(metal::int2(0, 1))));
    float _e28 = cloud_hash(as_type<metal::int2>(as_type<metal::uint2>(cell_1) + as_type<metal::uint2>(metal::int2(1, 1))));
    return metal::mix(metal::mix(_e11, _e16, w.x), metal::mix(_e23, _e28, w.x), w.y);
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
    metal::float2 _e12 = cloud.size;
    float _e17 = cloud.size.y;
    metal::float2 p_1 = (((uv - metal::float2(0.5)) * _e12) / metal::float2(_e17)) * 6.0;
    float _e22 = cloud_noise(p_1);
    float _e27 = cloud_noise(p_1 + metal::float2(8.3, 2.7));
    metal::float2 warp = metal::float2(_e22, _e27);
    float _e32 = cloud_noise(p_1 + (warp * 1.2));
    float _e39 = cloud_noise((p_1 * 2.3) + metal::float2(3.1, 7.4));
    float density = 0.25 + (0.75 * metal::smoothstep(0.25, 0.7, (_e32 * 0.75) + (_e39 * 0.25)));
    metal::float4 _e55 = close_light.sample(cloud_sampler, uv, metal::level(0.0));
    float close = _e55.x;
    metal::float4 _e60 = wide_light.sample(cloud_sampler, uv, metal::level(0.0));
    float wide = _e60.x;
    return fs_cloud_lightOutput { metal::float4((close * 0.75) + (wide * 0.25), density, 0.0, 1.0) };
}
