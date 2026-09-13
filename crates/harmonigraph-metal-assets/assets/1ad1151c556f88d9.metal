// language: metal3.2
#include <metal_stdlib>
#include <simd/simd.h>

using metal::uint;

struct Cloud {
    metal::float2 origin;
    metal::float2 size;
    metal::float2 step;
    float glow;
    float texture;
    metal::float2 time;
    float ppp;
    float time_scale;
    metal::float2 time_direction;
    metal::float2 pitch_direction;
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
    metal::float3 color = metal::float3(0.0);
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
            int _e58 = i;
            i = as_type<int>(as_type<uint>(_e58) + as_type<uint>(1));
        }
        loop_init = false;
        int _e19 = i;
        if (_e19 <= 8) {
        } else {
            break;
        }
        {
            int _e22 = i;
            float x = static_cast<float>(_e22) * 0.5;
            int _e26 = i;
            float weight = weights.inner[metal::min(unsigned(static_cast<uint>(naga_abs(_e26))), 8u)];
            metal::float2 tap = uv + (step * x);
            if (metal::all(tap >= metal::float2(0.0))) {
                local = metal::all(tap <= metal::float2(1.0));
            } else {
                local = false;
            }
            bool inside = local;
            metal::float3 _e44 = color;
            metal::float4 _e48 = source.sample(linear_sampler, tap, metal::level(0.0));
            color = _e44 + ((_e48.xyz * weight) * (inside ? 1.0 : 0.0));
            float _e56 = total;
            total = _e56 + weight;
        }
    }
    metal::float3 _e61 = color;
    float _e62 = total;
    return metal::float4(_e61 / metal::float3(_e62), 1.0);
}

struct fs_close_vInput {
    metal::float2 uv [[user(loc0), center_perspective]];
};
struct fs_close_vOutput {
    metal::float4 member [[color(0)]];
};
fragment fs_close_vOutput fs_close_v(
  fs_close_vInput varyings [[stage_in]]
, metal::float4 position [[position]]
, metal::texture2d<float, metal::access::sample> source [[texture(0)]]
, metal::sampler linear_sampler [[sampler(0)]]
, constant Cloud& cloud [[buffer(0)]]
) {
    const Vertex in = { position, varyings.uv };
    float _e5 = cloud.step.y;
    metal::float4 _e8 = filtered(in.uv, metal::float2(0.0, _e5), source, linear_sampler);
    return fs_close_vOutput { _e8 };
}
