// language: metal3.2
#include <metal_stdlib>
#include <simd/simd.h>

using metal::uint;

struct AddUniforms {
    metal::float4 strength;
};
struct BlitOut {
    metal::float4 pos;
    metal::float2 uv;
    char _pad2[8];
};
constant float BLOOM_THRESHOLD = 0.35;
constant float BLOOM_KNEE = 0.25;
constant float BLUR_W0_ = 0.227027;
constant metal::float4 BLUR_W = metal::float4(0.1945946, 0.1216216, 0.054054, 0.016216);

struct fs_bloom_addInput {
    metal::float2 uv [[user(loc0), center_perspective]];
};
struct fs_bloom_addOutput {
    metal::float4 member [[color(0)]];
};
fragment fs_bloom_addOutput fs_bloom_add(
  fs_bloom_addInput varyings [[stage_in]]
, metal::float4 pos [[position]]
, metal::texture2d<float, metal::access::sample> scene_tex [[texture(0)]]
, metal::sampler scene_samp [[sampler(0)]]
, constant AddUniforms& add [[buffer(0)]]
) {
    const BlitOut in = { pos, varyings.uv };
    metal::float4 bloom = scene_tex.sample(scene_samp, in.uv);
    float _e9 = add.strength.x;
    return fs_bloom_addOutput { metal::float4(bloom.xyz * _e9, 0.0) };
}
