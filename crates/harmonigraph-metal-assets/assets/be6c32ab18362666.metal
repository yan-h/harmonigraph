// language: metal3.2
#include <metal_stdlib>
#include <simd/simd.h>

using metal::uint;

struct BlitOut {
    metal::float4 pos;
    metal::float2 uv;
    char _pad2[8];
};
struct type_6 {
    float inner[5];
};
constant float BLOOM_THRESHOLD = 0.35;
constant float BLOOM_KNEE = 0.25;
constant float BLUR_W0_ = 0.227027;
constant metal::float4 BLUR_W = metal::float4(0.1945946, 0.1216216, 0.054054, 0.016216);

uint naga_f2u32(float value) {
    return static_cast<uint>(metal::clamp(value, 0.0, 4294967000.0));
}

float bloom_weight(
    float distance
) {
    type_6 weights = type_6 {{0.227027, 0.1945946, 0.1216216, 0.054054, 0.016216}};
    if (distance > 4.0) {
        return 0.0;
    }
    uint low = naga_f2u32(metal::floor(distance));
    return metal::mix(weights.inner[metal::min(unsigned(low), 4u)], weights.inner[metal::min(unsigned(metal::min(low + 1u, 4u)), 4u)], metal::fract(distance));
}

int naga_f2i32(float value) {
    return static_cast<int>(metal::clamp(value, -2147483600.0, 2147483500.0));
}

metal::float4 blur(
    metal::float2 uv,
    metal::float2 dir,
    metal::texture2d<float, metal::access::sample> scene_tex,
    metal::sampler scene_samp,
    constant metal::float4& bloom_scale
) {
    metal::float4 acc = {};
    float total = BLUR_W0_;
    uint i = 1u;
    int i_1 = 1;
    metal::float2 size = static_cast<metal::float2>(metal::uint2(scene_tex.get_width(), scene_tex.get_height()));
    metal::float4 _e6 = bloom_scale;
    float scale = metal::dot(_e6.xy, dir);
    metal::float2 texel = dir / size;
    metal::float4 _e13 = scene_tex.sample(scene_samp, uv, metal::level(0.0));
    acc = _e13 * BLUR_W0_;
    if (scale <= 1.0) {
        uint2 loop_bound = uint2(4294967295u);
        bool loop_init = true;
        while(true) {
            if (metal::all(loop_bound == uint2(0u))) { break; }
            loop_bound -= uint2(loop_bound.y == 0u, 1u);
            if (!loop_init) {
                uint _e54 = i;
                i = _e54 + 1u;
            }
            loop_init = false;
            uint _e23 = i;
            if (_e23 <= 4u) {
            } else {
                break;
            }
            {
                uint _e27 = i;
                float weight = BLUR_W[metal::min(unsigned(_e27 - 1u), 3u)];
                uint _e31 = i;
                metal::float2 offset = (texel * static_cast<float>(_e31)) * scale;
                metal::float4 _e35 = acc;
                metal::float4 _e40 = scene_tex.sample(scene_samp, uv + offset, metal::level(0.0));
                metal::float4 _e45 = scene_tex.sample(scene_samp, uv - offset, metal::level(0.0));
                acc = _e35 + ((_e40 + _e45) * weight);
                float _e49 = total;
                total = _e49 + (2.0 * weight);
            }
        }
        metal::float4 _e56 = acc;
        float _e57 = total;
        return _e56 / metal::float4(_e57);
    }
    int radius = naga_f2i32(metal::ceil(4.0 * scale));
    uint2 loop_bound_1 = uint2(4294967295u);
    bool loop_init_1 = true;
    while(true) {
        if (metal::all(loop_bound_1 == uint2(0u))) { break; }
        loop_bound_1 -= uint2(loop_bound_1.y == 0u, 1u);
        if (!loop_init_1) {
            int _e94 = i_1;
            i_1 = as_type<int>(as_type<uint>(_e94) + as_type<uint>(1));
        }
        loop_init_1 = false;
        int _e66 = i_1;
        if (_e66 <= radius) {
        } else {
            break;
        }
        {
            int _e68 = i_1;
            float _e71 = bloom_weight(static_cast<float>(_e68) / scale);
            int _e72 = i_1;
            metal::float2 offset_1 = texel * static_cast<float>(_e72);
            metal::float4 _e75 = acc;
            metal::float4 _e80 = scene_tex.sample(scene_samp, uv + offset_1, metal::level(0.0));
            metal::float4 _e85 = scene_tex.sample(scene_samp, uv - offset_1, metal::level(0.0));
            acc = _e75 + ((_e80 + _e85) * _e71);
            float _e89 = total;
            total = _e89 + (2.0 * _e71);
        }
    }
    metal::float4 _e96 = acc;
    float _e97 = total;
    return _e96 / metal::float4(_e97);
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
, constant metal::float4& bloom_scale [[buffer(0)]]
) {
    const BlitOut in = { pos, varyings.uv };
    metal::float4 _e5 = blur(in.uv, metal::float2(1.0, 0.0), scene_tex, scene_samp, bloom_scale);
    return fs_blur_hOutput { _e5 };
}
