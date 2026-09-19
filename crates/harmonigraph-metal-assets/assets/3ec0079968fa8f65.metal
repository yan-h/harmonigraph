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
    uint _pad;
};
struct Vertex {
    metal::float4 position;
    metal::float2 uv;
    char _pad2[8];
};

metal::float4 filtered(
    metal::float2 uv,
    metal::float2 step,
    bool close,
    metal::texture2d<float, metal::access::sample> source,
    metal::sampler linear_sampler
) {
    float level = 0.0;
    float total = 0.0;
    uint i = 0u;
    float offset = {};
    bool local = {};
    metal::float2 dims = static_cast<metal::float2>(metal::uint2(source.get_width(), source.get_height()));
    bool horizontal = step.x > 0.0;
    float sigma = horizontal ? step.x : step.y;
    float pixels = horizontal ? dims.x : dims.y;
    if ((sigma * pixels) < 0.25) {
        metal::float4 _e21 = source.sample(linear_sampler, uv, metal::level(0.0));
        return _e21;
    }
    float coordinate = horizontal ? uv.x : uv.y;
    float low = metal::max(-3.0 * sigma, (0.5 / pixels) - coordinate);
    float high = metal::min(3.0 * sigma, (1.0 - (0.5 / pixels)) - coordinate);
    metal::float2 direction = horizontal ? metal::float2(1.0, 0.0) : metal::float2(0.0, 1.0);
    uint2 loop_bound = uint2(4294967295u);
    bool loop_init = true;
    while(true) {
        if (metal::all(loop_bound == uint2(0u))) { break; }
        loop_bound -= uint2(loop_bound.y == 0u, 1u);
        if (!loop_init) {
            uint _e96 = i;
            i = _e96 + 1u;
        }
        loop_init = false;
        uint _e52 = i;
        if (_e52 < 17u) {
        } else {
            break;
        }
        {
            uint _e55 = i;
            offset = metal::mix(low, high, (static_cast<float>(_e55) + 0.5) / 17.0);
            if (close) {
                uint _e63 = i;
                offset = (static_cast<float>(_e63) - 8.0) / pixels;
                float _e68 = offset;
                if (!((_e68 < low))) {
                    float _e73 = offset;
                    local = _e73 > high;
                } else {
                    local = true;
                }
                bool _e76 = local;
                if (_e76) {
                    continue;
                }
            }
            float _e77 = offset;
            float x = _e77 / sigma;
            float weight = metal::exp((-0.5 * x) * x);
            float _e83 = level;
            float _e86 = offset;
            metal::float4 _e90 = source.sample(linear_sampler, uv + (direction * _e86), metal::level(0.0));
            level = _e83 + (_e90.x * weight);
            float _e94 = total;
            total = _e94 + weight;
        }
    }
    float _e99 = level;
    float _e100 = total;
    return metal::float4(_e99 / _e100, 0.0, 0.0, 1.0);
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
