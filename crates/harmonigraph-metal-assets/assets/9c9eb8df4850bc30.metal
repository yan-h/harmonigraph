// language: metal3.2
#include <metal_stdlib>
#include <simd/simd.h>

using metal::uint;

struct CellOut {
    metal::float4 position;
    metal::float4 bounds;
    float sigma;
    char _pad3[12];
};
constant float DISTANCE_KIND = 1.0;
constant float REACH = 3.0;
constant int MAX_RADIUS = 9;
constant float PEDESTAL = 0.011109;

int naga_f2i32(float value) {
    return static_cast<int>(metal::clamp(value, -2147483600.0, 2147483500.0));
}

metal::int2 naga_f2i32(metal::float2 value) {
    return static_cast<metal::int2>(metal::clamp(value, -2147483600.0, 2147483500.0));
}

int naga_neg(int val) {
    return as_type<int>(-as_type<uint>(val));
}

float blur(
    CellOut in_1,
    metal::int2 axis,
    metal::texture2d<float, metal::access::sample> src
) {
    float sum = 0.0;
    float weight = 0.0;
    int i = {};
    bool local = {};
    bool local_1 = {};
    bool local_2 = {};
    float sigma = metal::max(in_1.sigma, 0.001);
    int radius = metal::min(naga_f2i32(metal::ceil(REACH * sigma)), MAX_RADIUS);
    metal::int2 at = naga_f2i32(in_1.position.xy);
    i = naga_neg(radius);
    uint2 loop_bound = uint2(4294967295u);
    bool loop_init = true;
    while(true) {
        if (metal::all(loop_bound == uint2(0u))) { break; }
        loop_bound -= uint2(loop_bound.y == 0u, 1u);
        if (!loop_init) {
            int _e82 = i;
            i = as_type<int>(as_type<uint>(_e82) + as_type<uint>(1));
        }
        loop_init = false;
        int _e20 = i;
        if (_e20 <= radius) {
        } else {
            break;
        }
        {
            int _e22 = i;
            int _e23 = i;
            float w = metal::max(metal::exp((-0.5 * static_cast<float>(as_type<int>(as_type<uint>(_e22) * as_type<uint>(_e23)))) / (sigma * sigma)) - PEDESTAL, 0.0);
            float _e35 = weight;
            weight = _e35 + w;
            int _e37 = i;
            metal::int2 tap = as_type<metal::int2>(as_type<metal::uint2>(at) + as_type<metal::uint2>(as_type<metal::int2>(as_type<metal::uint2>(axis) * as_type<uint>(_e37))));
            metal::float2 centre = static_cast<metal::float2>(tap) + metal::float2(0.5);
            if (!((centre.x < in_1.bounds.x))) {
                local = centre.y < in_1.bounds.y;
            } else {
                local = true;
            }
            bool _e56 = local;
            if (!(_e56)) {
                local_1 = centre.x >= in_1.bounds.z;
            } else {
                local_1 = true;
            }
            bool _e65 = local_1;
            if (!(_e65)) {
                local_2 = centre.y >= in_1.bounds.w;
            } else {
                local_2 = true;
            }
            bool _e74 = local_2;
            if (_e74) {
                continue;
            }
            float _e75 = sum;
            uint clamped_lod_e78 = metal::min(uint(0), src.get_num_mip_levels() - 1);
            metal::float4 _e78 = src.read(metal::min(metal::uint2(tap), metal::uint2(src.get_width(clamped_lod_e78), src.get_height(clamped_lod_e78)) - 1), clamped_lod_e78);
            sum = _e75 + (w * _e78.x);
        }
    }
    float _e85 = sum;
    float _e86 = weight;
    return _e85 / _e86;
}

struct fs_blur_yInput {
    metal::float4 bounds [[user(loc0), flat]];
    float sigma [[user(loc1), flat]];
};
struct fs_blur_yOutput {
    metal::float4 member [[color(0)]];
};
fragment fs_blur_yOutput fs_blur_y(
  fs_blur_yInput varyings [[stage_in]]
, metal::float4 position [[position]]
, metal::texture2d<float, metal::access::sample> src [[texture(0)]]
) {
    const CellOut in = { position, varyings.bounds, varyings.sigma };
    float _e4 = blur(in, metal::int2(0, 1), src);
    return fs_blur_yOutput { metal::float4(_e4, 0.0, 0.0, 1.0) };
}
