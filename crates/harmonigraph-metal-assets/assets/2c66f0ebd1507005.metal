// language: metal3.2
#include <metal_stdlib>
#include <simd/simd.h>

using metal::uint;

struct Cloud {
    metal::float2 origin;
    metal::float2 size;
    metal::float2 step;
    float ppp;
    float spread;
    float contours;
    float contour_softness;
    float contour_strength;
    uint tone_baked;
};
struct Vertex {
    metal::float4 position;
    metal::float2 uv;
    char _pad2[8];
};

metal::float2 gaussian_tap(
    metal::float2 uv,
    metal::float2 direction,
    float offset,
    float sigma,
    metal::texture2d<float, metal::access::sample> source,
    metal::sampler linear_sampler
) {
    float x = offset / sigma;
    float weight = metal::exp((-0.5 * x) * x);
    metal::float4 _e14 = source.sample(linear_sampler, uv + (direction * offset), metal::level(0.0));
    float level = _e14.x;
    return metal::float2(level * weight, weight);
}

int naga_f2i32(float value) {
    return static_cast<int>(metal::clamp(value, -2147483600.0, 2147483500.0));
}

int naga_neg(int val) {
    return as_type<int>(-as_type<uint>(val));
}

metal::float4 filtered(
    metal::float2 uv_1,
    metal::float2 step,
    bool close,
    metal::texture2d<float, metal::access::sample> source,
    metal::sampler linear_sampler
) {
    metal::float2 sum = metal::float2(0.0);
    bool local = {};
    int i = {};
    bool local_1 = {};
    uint i_1 = 0u;
    metal::float2 dims = static_cast<metal::float2>(metal::uint2(source.get_width(), source.get_height()));
    bool horizontal = step.x > 0.0;
    float sigma_1 = horizontal ? step.x : step.y;
    float pixels = horizontal ? dims.x : dims.y;
    if ((sigma_1 * pixels) < 0.25) {
        metal::float4 _e21 = source.sample(linear_sampler, uv_1, metal::level(0.0));
        return _e21;
    }
    float coordinate = horizontal ? uv_1.x : uv_1.y;
    float low = metal::max(-3.0 * sigma_1, (0.5 / pixels) - coordinate);
    float high = metal::min(3.0 * sigma_1, (1.0 - (0.5 / pixels)) - coordinate);
    metal::float2 direction_1 = horizontal ? metal::float2(1.0, 0.0) : metal::float2(0.0, 1.0);
    if (!(close)) {
        local = ((6.0 * sigma_1) * pixels) < 17.0;
    } else {
        local = true;
    }
    bool _e58 = local;
    if (_e58) {
        int reach = naga_f2i32(metal::min(8.0, metal::ceil((3.0 * sigma_1) * pixels)));
        i = naga_neg(reach);
        uint2 loop_bound = uint2(4294967295u);
        bool loop_init = true;
        while(true) {
            if (metal::all(loop_bound == uint2(0u))) { break; }
            loop_bound -= uint2(loop_bound.y == 0u, 1u);
            if (!loop_init) {
                int _e83 = i;
                i = as_type<int>(as_type<uint>(_e83) + as_type<uint>(1));
            }
            loop_init = false;
            int _e68 = i;
            if (_e68 <= reach) {
            } else {
                break;
            }
            {
                int _e70 = i;
                float offset_1 = static_cast<float>(_e70) / pixels;
                if (!((offset_1 < low))) {
                    local_1 = offset_1 > high;
                } else {
                    local_1 = true;
                }
                bool _e79 = local_1;
                if (_e79) {
                    continue;
                }
                metal::float2 _e80 = sum;
                metal::float2 _e81 = gaussian_tap(uv_1, direction_1, offset_1, sigma_1, source, linear_sampler);
                sum = _e80 + _e81;
            }
        }
    } else {
        uint2 loop_bound_1 = uint2(4294967295u);
        bool loop_init_1 = true;
        while(true) {
            if (metal::all(loop_bound_1 == uint2(0u))) { break; }
            loop_bound_1 -= uint2(loop_bound_1.y == 0u, 1u);
            if (!loop_init_1) {
                uint _e101 = i_1;
                i_1 = _e101 + 1u;
            }
            loop_init_1 = false;
            uint _e88 = i_1;
            if (_e88 < 17u) {
            } else {
                break;
            }
            {
                metal::float2 _e91 = sum;
                uint _e92 = i_1;
                metal::float2 _e99 = gaussian_tap(uv_1, direction_1, metal::mix(low, high, (static_cast<float>(_e92) + 0.5) / 17.0), sigma_1, source, linear_sampler);
                sum = _e91 + _e99;
            }
        }
    }
    float _e105 = sum.x;
    float _e107 = sum.y;
    return metal::float4(_e105 / _e107, 0.0, 0.0, 1.0);
}

struct fs_wide_vInput {
    metal::float2 uv [[user(loc0), center_perspective]];
};
struct fs_wide_vOutput {
    metal::float4 member [[color(0)]];
};
fragment fs_wide_vOutput fs_wide_v(
  fs_wide_vInput varyings [[stage_in]]
, metal::float4 position [[position]]
, metal::texture2d<float, metal::access::sample> source [[texture(0)]]
, metal::sampler linear_sampler [[sampler(0)]]
, constant Cloud& cloud [[buffer(0)]]
) {
    const Vertex in = { position, varyings.uv };
    float _e5 = cloud.step.y;
    metal::float4 _e11 = filtered(in.uv, metal::float2(0.0, _e5 * 5.0), false, source, linear_sampler);
    return fs_wide_vOutput { _e11 };
}
