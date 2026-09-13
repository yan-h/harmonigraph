// language: metal3.2
#include <metal_stdlib>
#include <simd/simd.h>

using metal::uint;

struct Cloud {
    metal::float2 origin;
    metal::float2 size;
    metal::float2 step;
    float diffusion;
    float ppp;
};
struct Vertex {
    metal::float4 position;
    metal::float2 uv;
    char _pad2[8];
};
struct type_5 {
    float inner[9];
};

int naga_abs(int val) {
    return metal::select(as_type<int>(-as_type<uint>(val)), val, val >= 0);
}

metal::float4 filtered(
    metal::float2 uv,
    metal::float2 step,
    metal::texture2d<float, metal::access::sample> source,
    metal::sampler linear_sampler
) {
    float level = 0.0;
    float total = 0.0;
    int i = -8;
    bool local = {};
    type_5 weights = type_5 {{1.0, 0.9576695, 0.8411289, 0.677549, 0.5005531, 0.3391493, 0.2107477, 0.1201064, 0.062777}};
    uint2 loop_bound = uint2(4294967295u);
    bool loop_init = true;
    while(true) {
        if (metal::all(loop_bound == uint2(0u))) { break; }
        loop_bound -= uint2(loop_bound.y == 0u, 1u);
        if (!loop_init) {
            int _e57 = i;
            i = as_type<int>(as_type<uint>(_e57) + as_type<uint>(1));
        }
        loop_init = false;
        int _e18 = i;
        if (_e18 <= 8) {
        } else {
            break;
        }
        {
            int _e21 = i;
            float x = static_cast<float>(_e21) * 0.5;
            int _e25 = i;
            float weight = weights.inner[metal::min(unsigned(static_cast<uint>(naga_abs(_e25))), 8u)];
            metal::float2 tap = uv + (step * x);
            if (metal::all(tap >= metal::float2(0.0))) {
                local = metal::all(tap <= metal::float2(1.0));
            } else {
                local = false;
            }
            bool inside = local;
            float _e43 = level;
            metal::float4 _e47 = source.sample(linear_sampler, tap, metal::level(0.0));
            level = _e43 + ((_e47.x * weight) * (inside ? 1.0 : 0.0));
            float _e55 = total;
            total = _e55 + weight;
        }
    }
    float _e60 = level;
    float _e61 = total;
    return metal::float4(_e60 / _e61, 0.0, 0.0, 1.0);
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
