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

metal::int2 naga_f2i32(metal::float2 value) {
    return static_cast<metal::int2>(metal::clamp(value, -2147483600.0, 2147483500.0));
}

metal::float4 shadow_mask_texel(
    BlitOut in_1,
    metal::texture2d<float, metal::access::sample> scene_tex
) {
    metal::int2 texel = naga_f2i32(in_1.uv * static_cast<metal::float2>(metal::uint2(scene_tex.get_width(), scene_tex.get_height())));
    uint clamped_lod_e9 = metal::min(uint(0), scene_tex.get_num_mip_levels() - 1);
    metal::float4 _e9 = scene_tex.read(metal::min(metal::uint2(texel), metal::uint2(scene_tex.get_width(clamped_lod_e9), scene_tex.get_height(clamped_lod_e9)) - 1), clamped_lod_e9);
    return _e9;
}

struct fs_shadow_maskInput {
    metal::float2 uv [[user(loc0), center_perspective]];
};
struct fs_shadow_maskOutput {
    metal::float4 member [[color(0)]];
};
fragment fs_shadow_maskOutput fs_shadow_mask(
  fs_shadow_maskInput varyings [[stage_in]]
, metal::float4 pos [[position]]
, metal::texture2d<float, metal::access::sample> scene_tex [[texture(0)]]
) {
    const BlitOut in = { pos, varyings.uv };
    metal::float4 _e1 = shadow_mask_texel(in, scene_tex);
    return fs_shadow_maskOutput { metal::float4(_e1.xyz * _e1.w, _e1.w) };
}
