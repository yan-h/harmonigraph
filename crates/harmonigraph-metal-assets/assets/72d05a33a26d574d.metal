// language: metal3.2
#include <metal_stdlib>
#include <simd/simd.h>

using metal::uint;

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
struct DotOut {
    metal::float4 position;
    metal::float2 local;
    float radius;
    char _pad3[4];
    metal::float4 aura;
};
struct SurroundOut {
    metal::float4 color;
    float depth;
    char _pad2[12];
};

float surround_ramp(
    float k,
    float x
) {
    float t = metal::clamp(x, 0.0, 1.0);
    return 1.0 - (t / ((k * (1.0 - t)) + 1.0));
}

float dot_cut(
    DotOut in_1,
    constant DotLocals& dot_locals
) {
    float reach = dot_locals.surround.cut.y;
    if (reach <= 0.0) {
        return 0.0;
    }
    float d = metal::length(in_1.local) - in_1.radius;
    float _e15 = dot_locals.feather;
    float f = metal::max(_e15, 0.000001);
    float fill = metal::clamp(0.5 - (d / f), 0.0, 1.0);
    float end = metal::clamp(((reach - d) / f) + 0.5, 0.0, 1.0);
    float _e35 = dot_locals.surround.cut.z;
    float _e39 = surround_ramp(_e35, metal::max(d, 0.0) / reach);
    return ((in_1.aura.w * _e39) * end) * (1.0 - fill);
}

struct fs_dot_surroundInput {
    metal::float2 local [[user(loc0), center_perspective]];
    float radius [[user(loc1), flat]];
    metal::float4 aura [[user(loc2), flat]];
};
struct fs_dot_surroundOutput {
    metal::float4 color [[color(0)]];
    float depth [[depth(any)]];
};
fragment fs_dot_surroundOutput fs_dot_surround(
  fs_dot_surroundInput varyings [[stage_in]]
, metal::float4 position [[position]]
, constant DotLocals& dot_locals [[buffer(0)]]
) {
    const DotOut in = { position, varyings.local, varyings.radius, {}, varyings.aura };
    float _e5 = dot_locals.surround.cut.x;
    float _e6 = dot_cut(in, dot_locals);
    float cut = _e5 * _e6;
    if (cut <= 0.0) {
        metal::discard_fragment();
    }
    const auto _tmp = SurroundOut {metal::float4(in.aura.xyz, cut), 1.0 - cut};
    return fs_dot_surroundOutput { _tmp.color, _tmp.depth };
}
