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

metal::float4 bright(
    metal::float3 rgb
) {
    float lum = metal::dot(rgb, metal::float3(0.2126, 0.7152, 0.0722));
    float keep = metal::smoothstep(0.099999994, 0.6, lum);
    return metal::float4(rgb * keep, 0.0);
}

struct fs_bright_splitInput {
    metal::float2 uv [[user(loc0), center_perspective]];
};
struct fs_bright_splitOutput {
    metal::float4 member [[color(0)]];
};
fragment fs_bright_splitOutput fs_bright_split(
  fs_bright_splitInput varyings [[stage_in]]
, metal::float4 pos [[position]]
, metal::texture2d<float, metal::access::sample> scene_tex [[texture(0)]]
, metal::sampler scene_samp [[sampler(0)]]
, metal::texture2d<float, metal::access::sample> ink_tex [[texture(1)]]
) {
    const BlitOut in = { pos, varyings.uv };
    metal::float4 other = scene_tex.sample(scene_samp, in.uv);
    metal::float4 ink = ink_tex.sample(scene_samp, in.uv);
    metal::float4 _e12 = bright(other.xyz + ink.xyz);
    return fs_bright_splitOutput { _e12 };
}
