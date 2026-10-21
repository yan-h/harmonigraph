// language: metal3.2
#include <metal_stdlib>
#include <simd/simd.h>

using metal::uint;

struct _mslBufferSizes {
    uint size1;
    uint size2;
};

struct SurroundCut {
    metal::float4 cut;
};
struct DotLocals {
    SurroundCut surround;
    metal::float2 origin_points;
    metal::float2 viewport_points;
    float feather;
    float _feather_pad;
    metal::float2 _tail_pad;
};
struct Dot {
    metal::float2 center;
    float radius;
    uint aura;
    uint first;
    uint candidates;
};
typedef Dot type_5[1];
typedef uint type_6[1];
struct DotOut {
    metal::float4 position;
    uint dot;
    char _pad2[12];
};

float surround_ramp(
    float k,
    float x
) {
    float t = metal::clamp(x, 0.0, 1.0);
    return 1.0 - (t / ((k * (1.0 - t)) + 1.0));
}

metal::float3 linear_from_gamma_rgb(
    metal::float3 srgb
) {
    metal::bool3 cutoff = srgb < metal::float3(0.04045);
    metal::float3 lower = srgb / metal::float3(12.92);
    metal::float3 higher = metal::pow((srgb + metal::float3(0.055)) / metal::float3(1.055), metal::float3(2.4));
    return metal::select(higher, lower, cutoff);
}

metal::float4 surround_lands_linear(
    metal::float3 color,
    float cut
) {
    metal::float3 _e2 = linear_from_gamma_rgb(color);
    return metal::float4(_e2 * cut, cut);
}

float cut_reach(
    constant DotLocals& dot_locals
) {
    float _e4 = dot_locals.surround.cut.y;
    float _e9 = dot_locals.surround.cut.x;
    return (_e9 > 0.0) ? _e4 : 0.0;
}

float dot_cut(
    Dot at,
    metal::float2 p,
    constant DotLocals& dot_locals
) {
    float _e2 = cut_reach(dot_locals);
    if (_e2 <= 0.0) {
        return 0.0;
    }
    float _e8 = dot_locals.feather;
    float f = metal::max(_e8, 0.000001);
    float d = metal::length(p - at.center) - at.radius;
    float fill = metal::clamp(0.5 - (d / f), 0.0, 1.0);
    float end = metal::clamp(((_e2 - d) / f) + 0.5, 0.0, 1.0);
    float _e33 = dot_locals.surround.cut.z;
    float _e37 = surround_ramp(_e33, metal::max(d, 0.0) / _e2);
    return ((metal::unpack_unorm4x8_to_float(at.aura).w * _e37) * end) * (1.0 - fill);
}

float landed(
    DotOut in_1,
    constant DotLocals& dot_locals,
    device type_5 const& dots,
    device type_6 const& candidates,
    constant _mslBufferSizes& _buffer_sizes
) {
    bool local = {};
    float strongest = 0.0;
    uint winner = {};
    uint n = 0u;
    bool local_1 = {};
    float _e5 = dot_locals.feather;
    metal::float2 p_1 = in_1.position.xy * _e5;
    Dot mine = dots[metal::min(unsigned(in_1.dot), (_buffer_sizes.size1 - 0 - 24) / 24)];
    float _e13 = dot_locals.feather;
    float f_1 = metal::max(_e13, 0.000001);
    float own = metal::length(p_1 - mine.center) - mine.radius;
    if (!((own <= (-0.51 * f_1)))) {
        float _e27 = cut_reach(dot_locals);
        local = own >= (_e27 + (0.51 * f_1));
    } else {
        local = true;
    }
    bool _e33 = local;
    if (_e33) {
        metal::discard_fragment();
    }
    float _e34 = cut_reach(dot_locals);
    float grow = _e34 + f_1;
    winner = in_1.dot;
    uint2 loop_bound = uint2(4294967295u);
    bool loop_init = true;
    while(true) {
        if (metal::all(loop_bound == uint2(0u))) { break; }
        loop_bound -= uint2(loop_bound.y == 0u, 1u);
        if (!loop_init) {
            uint _e66 = n;
            n = _e66 + 1u;
        }
        loop_init = false;
        uint _e42 = n;
        if (_e42 < mine.candidates) {
        } else {
            break;
        }
        {
            uint _e47 = n;
            uint i = candidates[metal::min(unsigned(mine.first + _e47), (_buffer_sizes.size2 - 0 - 4) / 4)];
            Dot at_1 = dots[metal::min(unsigned(i), (_buffer_sizes.size1 - 0 - 24) / 24)];
            if (metal::any(metal::abs(p_1 - at_1.center) > metal::float2(at_1.radius + grow))) {
                continue;
            }
            float _e62 = dot_cut(at_1, p_1, dot_locals);
            float _e63 = strongest;
            if (_e62 > _e63) {
                strongest = _e62;
                winner = i;
            }
        }
    }
    float _e68 = strongest;
    if (!((_e68 <= 0.0))) {
        uint _e74 = winner;
        local_1 = _e74 != in_1.dot;
    } else {
        local_1 = true;
    }
    bool _e78 = local_1;
    if (_e78) {
        metal::discard_fragment();
    }
    float _e83 = dot_locals.surround.cut.x;
    float _e84 = strongest;
    return _e83 * _e84;
}

struct fs_dot_surround_linearInput {
    uint dot [[user(loc0), flat]];
};
struct fs_dot_surround_linearOutput {
    metal::float4 member [[color(0)]];
};
fragment fs_dot_surround_linearOutput fs_dot_surround_linear(
  fs_dot_surround_linearInput varyings [[stage_in]]
, metal::float4 position [[position]]
, constant DotLocals& dot_locals [[buffer(0)]]
, device type_5 const& dots [[buffer(1)]]
, device type_6 const& candidates [[buffer(2)]]
, constant _mslBufferSizes& _buffer_sizes [[buffer(3)]]
) {
    const DotOut in = { position, varyings.dot };
    float _e1 = landed(in, dot_locals, dots, candidates, _buffer_sizes);
    uint _e6 = dots[metal::min(unsigned(in.dot), (_buffer_sizes.size1 - 0 - 24) / 24)].aura;
    metal::float4 _e9 = surround_lands_linear(metal::unpack_unorm4x8_to_float(_e6).xyz, _e1);
    return fs_dot_surround_linearOutput { _e9 };
}
