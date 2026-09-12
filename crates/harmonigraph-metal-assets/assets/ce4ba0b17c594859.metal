// language: metal3.2
#include <metal_stdlib>
#include <simd/simd.h>

using metal::uint;

struct BlitOut {
    metal::float4 pos;
    metal::float2 uv;
    char _pad2[8];
};
struct GlowOverOut {
    metal::float4 other;
    metal::float4 ink;
    metal::float4 bloom_other;
    metal::float4 bloom_ink;
};
constant float BLOOM_THRESHOLD = 0.35;
constant float BLOOM_KNEE = 0.25;
constant float BLUR_W0_ = 0.227027;
constant metal::float4 BLUR_W = metal::float4(0.1945946, 0.1216216, 0.054054, 0.016216);

struct fs_glow_overInput {
    metal::float2 uv [[user(loc0), center_perspective]];
};
struct fs_glow_overOutput {
    metal::float4 other [[color(0)]];
    metal::float4 ink [[color(1)]];
    metal::float4 bloom_other [[color(2)]];
    metal::float4 bloom_ink [[color(3)]];
};
fragment fs_glow_overOutput fs_glow_over(
  fs_glow_overInput varyings [[stage_in]]
, metal::float4 pos [[position]]
, metal::texture2d<float, metal::access::sample> scene_tex [[texture(0)]]
, metal::sampler scene_samp [[sampler(0)]]
) {
    const BlitOut in = { pos, varyings.uv };
    metal::float4 light = scene_tex.sample(scene_samp, in.uv);
    const auto _tmp = GlowOverOut {light, metal::float4(0.0), light, metal::float4(0.0)};
    return fs_glow_overOutput { _tmp.other, _tmp.ink, _tmp.bloom_other, _tmp.bloom_ink };
}
