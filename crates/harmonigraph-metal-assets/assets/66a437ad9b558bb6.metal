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

metal::float4 bloom_sample(
    metal::texture2d<float, metal::access::sample> tex,
    metal::float2 uv,
    metal::sampler scene_samp
) {
    metal::float2 size = static_cast<metal::float2>(metal::uint2(tex.get_width(), tex.get_height()));
    metal::float2 p = (uv * size) - metal::float2(0.5);
    metal::float2 i = metal::floor(p);
    metal::float2 f = p - i;
    metal::float2 w0_ = (((metal::float2(1.0) - f) * (metal::float2(1.0) - f)) * (metal::float2(1.0) - f)) / metal::float2(6.0);
    metal::float2 w1_ = ((metal::float2(4.0) - ((6.0 * f) * f)) + (((3.0 * f) * f) * f)) / metal::float2(6.0);
    metal::float2 w3_ = ((f * f) * f) / metal::float2(6.0);
    metal::float2 w2_ = ((metal::float2(1.0) - w0_) - w1_) - w3_;
    metal::float2 g0_ = w0_ + w1_;
    metal::float2 g1_ = w2_ + w3_;
    metal::float2 lo = ((i + (w1_ / g0_)) - metal::float2(0.5)) / size;
    metal::float2 hi = ((i + metal::float2(1.5)) + (w3_ / g1_)) / size;
    metal::float4 _e68 = tex.sample(scene_samp, metal::float2(lo.x, lo.y));
    metal::float4 _e75 = tex.sample(scene_samp, metal::float2(hi.x, lo.y));
    metal::float4 _e85 = tex.sample(scene_samp, metal::float2(lo.x, hi.y));
    metal::float4 _e92 = tex.sample(scene_samp, metal::float2(hi.x, hi.y));
    return (g0_.y * ((g0_.x * _e68) + (g1_.x * _e75))) + (g1_.y * ((g0_.x * _e85) + (g1_.x * _e92)));
}

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
    metal::float4 _e3 = bloom_sample(scene_tex, in.uv, scene_samp);
    float _e8 = add.strength.x;
    return fs_bloom_addOutput { metal::float4(_e3.xyz * _e8, 0.0) };
}
