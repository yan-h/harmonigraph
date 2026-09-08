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

metal::float4 blur(
    metal::float2 uv,
    metal::float2 dir,
    metal::texture2d<float, metal::access::sample> scene_tex,
    metal::sampler scene_samp
) {
    metal::float4 acc = {};
    int i = 1;
    metal::float2 texel = dir / static_cast<metal::float2>(metal::uint2(scene_tex.get_width(), scene_tex.get_height()));
    metal::float4 _e8 = scene_tex.sample(scene_samp, uv);
    acc = _e8 * BLUR_W0_;
    uint2 loop_bound = uint2(4294967295u);
    bool loop_init = true;
    while(true) {
        if (metal::all(loop_bound == uint2(0u))) { break; }
        loop_bound -= uint2(loop_bound.y == 0u, 1u);
        if (!loop_init) {
            int _e45 = i;
            i = as_type<int>(as_type<uint>(_e45) + as_type<uint>(1));
        }
        loop_init = false;
        int _e14 = i;
        if (_e14 <= 4) {
        } else {
            break;
        }
        {
            int _e17 = i;
            metal::float2 offset = texel * static_cast<float>(_e17);
            metal::float4 _e20 = acc;
            metal::float4 _e24 = scene_tex.sample(scene_samp, uv + offset);
            int _e26 = i;
            acc = _e20 + (_e24 * BLUR_W[metal::min(unsigned(as_type<int>(as_type<uint>(_e26) - as_type<uint>(1))), 3u)]);
            metal::float4 _e32 = acc;
            metal::float4 _e36 = scene_tex.sample(scene_samp, uv - offset);
            int _e38 = i;
            acc = _e32 + (_e36 * BLUR_W[metal::min(unsigned(as_type<int>(as_type<uint>(_e38) - as_type<uint>(1))), 3u)]);
        }
    }
    metal::float4 _e47 = acc;
    return _e47;
}

struct fs_blur_hInput {
    metal::float2 uv [[user(loc0), center_perspective]];
};
struct fs_blur_hOutput {
    metal::float4 member [[color(0)]];
};
fragment fs_blur_hOutput fs_blur_h(
  fs_blur_hInput varyings [[stage_in]]
, metal::float4 pos [[position]]
, metal::texture2d<float, metal::access::sample> scene_tex [[texture(0)]]
, metal::sampler scene_samp [[sampler(0)]]
) {
    const BlitOut in = { pos, varyings.uv };
    metal::float4 _e5 = blur(in.uv, metal::float2(1.0, 0.0), scene_tex, scene_samp);
    return fs_blur_hOutput { _e5 };
}
