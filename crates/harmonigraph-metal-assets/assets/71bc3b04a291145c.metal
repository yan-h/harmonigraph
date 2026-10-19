// language: metal3.2
#include <metal_stdlib>
#include <simd/simd.h>

using metal::uint;

struct BlitOut {
    metal::float4 pos;
    metal::float2 uv;
    char _pad2[8];
};
constant float BLOOM_THRESHOLD = 0.35;
constant float BLOOM_KNEE = 0.25;
constant float BLUR_W0_ = 0.227027;
constant metal::float4 BLUR_W = metal::float4(0.1945946, 0.1216216, 0.054054, 0.016216);

struct fs_downsampleInput {
    metal::float2 uv [[user(loc0), center_perspective]];
};
struct fs_downsampleOutput {
    metal::float4 member [[color(0)]];
};
fragment fs_downsampleOutput fs_downsample(
  fs_downsampleInput varyings [[stage_in]]
, metal::float4 pos [[position]]
, metal::texture2d<float, metal::access::sample> scene_tex [[texture(0)]]
, metal::sampler scene_samp [[sampler(0)]]
) {
    const BlitOut in = { pos, varyings.uv };
    metal::float2 reach = metal::float2(0.75) / static_cast<metal::float2>(metal::uint2(scene_tex.get_width(), scene_tex.get_height()));
    metal::float4 _e16 = scene_tex.sample(scene_samp, in.uv + metal::float2(-(reach.x), -(reach.y)));
    metal::float4 _e25 = scene_tex.sample(scene_samp, in.uv + metal::float2(reach.x, -(reach.y)));
    metal::float4 _e35 = scene_tex.sample(scene_samp, in.uv + metal::float2(-(reach.x), reach.y));
    metal::float4 _e44 = scene_tex.sample(scene_samp, in.uv + metal::float2(reach.x, reach.y));
    return fs_downsampleOutput { 0.25 * (((_e16 + _e25) + _e35) + _e44) };
}
