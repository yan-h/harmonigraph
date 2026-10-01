// language: metal3.2
#include <metal_stdlib>
#include <simd/simd.h>

using metal::uint;

struct Cloud {
    metal::float2 origin;
    metal::float2 size;
    metal::float2 step;
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
    metal::texture2d<float, metal::access::sample> source,
    metal::sampler linear_sampler
) {
    metal::float2 sum = metal::float2(0.0);
    int i = {};
    bool local = {};
    metal::float2 dims = static_cast<metal::float2>(metal::uint2(source.get_width(), source.get_height()));
    bool horizontal = step.x > 0.0;
    float sigma_1 = horizontal ? step.x : step.y;
    float pixels = horizontal ? dims.x : dims.y;
    if ((sigma_1 * pixels) < 0.25) {
        metal::float4 _e20 = source.sample(linear_sampler, uv_1, metal::level(0.0));
        return _e20;
    }
    float coordinate = horizontal ? uv_1.x : uv_1.y;
    float low = metal::max(-3.0 * sigma_1, (0.5 / pixels) - coordinate);
    float high = metal::min(3.0 * sigma_1, (1.0 - (0.5 / pixels)) - coordinate);
    metal::float2 direction_1 = horizontal ? metal::float2(1.0, 0.0) : metal::float2(0.0, 1.0);
    int reach = naga_f2i32(metal::min(8.0, metal::ceil((3.0 * sigma_1) * pixels)));
    i = naga_neg(reach);
    uint2 loop_bound = uint2(4294967295u);
    bool loop_init = true;
    while(true) {
        if (metal::all(loop_bound == uint2(0u))) { break; }
        loop_bound -= uint2(loop_bound.y == 0u, 1u);
        if (!loop_init) {
            int _e72 = i;
            i = as_type<int>(as_type<uint>(_e72) + as_type<uint>(1));
        }
        loop_init = false;
        int _e57 = i;
        if (_e57 <= reach) {
        } else {
            break;
        }
        {
            int _e59 = i;
            float offset_1 = static_cast<float>(_e59) / pixels;
            if (!((offset_1 < low))) {
                local = offset_1 > high;
            } else {
                local = true;
            }
            bool _e68 = local;
            if (_e68) {
                continue;
            }
            metal::float2 _e69 = sum;
            metal::float2 _e70 = gaussian_tap(uv_1, direction_1, offset_1, sigma_1, source, linear_sampler);
            sum = _e69 + _e70;
        }
    }
    float _e76 = sum.x;
    float _e78 = sum.y;
    return metal::float4(_e76 / _e78, 0.0, 0.0, 1.0);
}

struct fs_close_hInput {
    metal::float2 uv [[user(loc0), center_perspective]];
};
struct fs_close_hOutput {
    metal::float4 member [[color(0)]];
};
fragment fs_close_hOutput fs_close_h(
  fs_close_hInput varyings [[stage_in]]
, metal::float4 position [[position]]
, metal::texture2d<float, metal::access::sample> source [[texture(0)]]
, metal::sampler linear_sampler [[sampler(0)]]
, constant Cloud& cloud [[buffer(0)]]
) {
    const Vertex in = { position, varyings.uv };
    float _e5 = cloud.step.x;
    metal::float4 _e8 = filtered(in.uv, metal::float2(_e5, 0.0), source, linear_sampler);
    return fs_close_hOutput { _e8 };
}
