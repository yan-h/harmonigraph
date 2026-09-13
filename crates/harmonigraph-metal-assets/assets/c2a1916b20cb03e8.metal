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
) {
    const VertexOut in = { position, varyings.slab, varyings.t };
    metal::float2 uv = in.position.xy / static_cast<metal::float2>(metal::uint2(wide_light.get_width(), wide_light.get_height()));
    metal::float4 _e10 = close_light.sample(cloud_sampler, uv, metal::level(0.0));
    float close = _e10.x;
    metal::float4 _e15 = wide_light.sample(cloud_sampler, uv, metal::level(0.0));
    float wide = _e15.x;
    return fs_cloud_lightOutput { metal::float4((close * 0.75) + (wide * 0.25), 0.0, 0.0, 1.0) };
}
