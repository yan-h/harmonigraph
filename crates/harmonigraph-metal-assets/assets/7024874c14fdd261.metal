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

metal::float2 box_pair(
    float a,
    float lo,
    float hi
) {
    float w0_ = metal::max(metal::min(hi, a + 1.0) - metal::max(lo, a), 0.0);
    float w1_ = metal::max(metal::min(hi, a + 2.0) - metal::max(lo, a + 1.0), 0.0);
    float w = w0_ + w1_;
    return metal::float2((a + 0.5) + (w1_ / w), w / (hi - lo));
}

metal::float4 bright(
    metal::float3 rgb
) {
    float lum = metal::dot(rgb, metal::float3(0.2126, 0.7152, 0.0722));
    float keep = metal::smoothstep(0.099999994, 0.6, lum);
    return metal::float4(rgb * keep, 0.0);
}
metal::int2 naga_f2i32(metal::float2 value) {
    return static_cast<metal::int2>(metal::clamp(value, -2147483600.0, 2147483500.0));
}


struct fs_brightInput {
    metal::float2 uv [[user(loc0), center_perspective]];
};
struct fs_brightOutput {
    metal::float4 member [[color(0)]];
};
fragment fs_brightOutput fs_bright(
  fs_brightInput varyings [[stage_in]]
, metal::float4 pos [[position]]
, metal::texture2d<float, metal::access::sample> scene_tex [[texture(0)]]
, metal::sampler scene_samp [[sampler(0)]]
, constant metal::float4& bloom_scale [[buffer(0)]]
) {
    const BlitOut in = { pos, varyings.uv };
    metal::float4 c = metal::float4(0.0);
    int y = 0;
    int x = {};
    metal::float2 size = static_cast<metal::float2>(metal::uint2(scene_tex.get_width(), scene_tex.get_height()));
    metal::float4 _e5 = bloom_scale;
    metal::float2 ratio = size / _e5.zw;
    metal::float2 lo_1 = (in.pos.xy - metal::float2(0.5)) * ratio;
    metal::float2 hi_1 = (in.pos.xy + metal::float2(0.5)) * ratio;
    metal::float2 first = metal::floor(lo_1);
    metal::int2 texels = naga_f2i32(metal::ceil(hi_1) - first);
    uint2 loop_bound = uint2(4294967295u);
    bool loop_init = true;
    while(true) {
        if (metal::all(loop_bound == uint2(0u))) { break; }
        loop_bound -= uint2(loop_bound.y == 0u, 1u);
        if (!loop_init) {
            int _e68 = y;
            y = as_type<int>(as_type<uint>(_e68) + as_type<uint>(2));
        }
        loop_init = false;
        int _e29 = y;
        if (_e29 < texels.y) {
        } else {
            break;
        }
        {
            int _e33 = y;
            metal::float2 _e38 = box_pair(first.y + static_cast<float>(_e33), lo_1.y, hi_1.y);
            x = 0;
            uint2 loop_bound_1 = uint2(4294967295u);
            bool loop_init_1 = true;
            while(true) {
                if (metal::all(loop_bound_1 == uint2(0u))) { break; }
                loop_bound_1 -= uint2(loop_bound_1.y == 0u, 1u);
                if (!loop_init_1) {
                    int _e65 = x;
                    x = as_type<int>(as_type<uint>(_e65) + as_type<uint>(2));
                }
                loop_init_1 = false;
                int _e41 = x;
                if (_e41 < texels.x) {
                } else {
                    break;
                }
                {
                    int _e45 = x;
                    metal::float2 _e50 = box_pair(first.x + static_cast<float>(_e45), lo_1.x, hi_1.x);
                    metal::float2 uv = metal::float2(_e50.x, _e38.x) / size;
                    metal::float4 _e55 = c;
                    metal::float4 _e62 = scene_tex.sample(scene_samp, uv, metal::level(0.0));
                    c = _e55 + ((_e50.y * _e38.y) * _e62);
                }
            }
        }
    }
    metal::float4 _e71 = c;
    metal::float4 _e73 = bright(_e71.xyz);
    return fs_brightOutput { _e73 };
}
