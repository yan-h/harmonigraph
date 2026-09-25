// language: metal3.2
#include <metal_stdlib>
#include <simd/simd.h>

using metal::uint;

struct VertexOut {
    metal::float4 position;
    metal::float2 local;
    metal::float2 half_extent;
    float shear;
    float outline_reach;
    float lead;
    float lead_fade;
    float lead_alpha;
    float cap_reach;
    char _pad9[8];
    metal::float4 core;
    metal::float4 outline;
    metal::float2 at;
    uint who;
    float feather;
    metal::float4 fade;
};
constant float DISTANCE_KIND = 1.0;
constant float DISTANCE_COVERAGE_KIND = 2.0;
constant uint OCCLUDER_HEADER = 5u;
constant float GAUSSIAN_GAIN = 2.5;
constant float SHADOW_STOP = 1.0;
constant float SHADOW_FALLOFF_MIN = -6.0;
constant float SHADOW_FALLOFF_MAX = 6.0;
constant float SHADOW_KEEP_FLOOR = 0.0009765625;

float box_distance_trimmed(
    VertexOut in_1,
    float trim
) {
    metal::float2 p = {};
    metal::float2 w = {};
    float near = {};
    float within = {};
    metal::float2 v = {};
    float slope = in_1.shear;
    float half_along = in_1.half_extent.y - (0.5 * trim);
    float half_pitch = in_1.half_extent.x;
    metal::float2 end = metal::float2(slope * half_along, half_along);
    p = in_1.local - ((0.5 * trim) * metal::float2(slope, 1.0));
    metal::float2 _e20 = p;
    metal::float2 _e21 = p;
    float _e24 = p.y;
    p = (_e24 < 0.0) ? -(_e21) : _e20;
    metal::float2 _e28 = p;
    w = _e28 - end;
    float _e32 = w.x;
    float _e34 = w.x;
    w.x = _e32 - metal::clamp(_e34, -(half_pitch), half_pitch);
    metal::float2 _e38 = w;
    metal::float2 _e39 = w;
    near = metal::dot(_e38, _e39);
    float _e43 = w.y;
    within = -(_e43);
    float _e47 = p.x;
    float _e51 = p.y;
    float side = (_e47 * end.y) - (_e51 * end.x);
    metal::float2 _e55 = p;
    metal::float2 _e56 = p;
    p = (side < 0.0) ? -(_e56) : _e55;
    metal::float2 _e61 = p;
    v = _e61 - metal::float2(half_pitch, 0.0);
    metal::float2 _e66 = v;
    metal::float2 _e67 = v;
    v = _e66 - (end * metal::clamp(metal::dot(_e67, end) / metal::max(metal::dot(end, end), 0.000000000001), -1.0, 1.0));
    float _e78 = near;
    metal::float2 _e79 = v;
    metal::float2 _e80 = v;
    near = metal::min(_e78, metal::dot(_e79, _e80));
    float _e83 = within;
    within = metal::min(_e83, (half_pitch * half_along) - metal::abs(side));
    float _e88 = near;
    float _e90 = near;
    float _e93 = within;
    return (_e93 > 0.0) ? -(metal::sqrt(_e90)) : metal::sqrt(_e88);
}

float box_distance(
    VertexOut in_2
) {
    float _e2 = box_distance_trimmed(in_2, 0.0);
    return _e2;
}

float inside(
    VertexOut in_3,
    float d,
    float edge
) {
    float f = metal::max(in_3.feather, 0.000001);
    return metal::clamp(((edge - d) / f) + 0.5, 0.0, 1.0);
}

float lead_coverage(
    VertexOut in_4
) {
    float led = {};
    if (in_4.lead <= 0.0) {
        return 1.0;
    }
    float f_1 = metal::max(in_4.feather, 0.000001);
    float u = in_4.local.y + in_4.half_extent.y;
    led = in_4.lead_alpha;
    if (in_4.lead_fade > 0.0) {
        led = in_4.lead_alpha * metal::clamp(u / metal::max(metal::min(in_4.lead_fade, in_4.lead), f_1), 0.0, 1.0);
    }
    float note = metal::clamp(((u - in_4.lead) / f_1) + 0.5, 0.0, 1.0);
    float _e36 = led;
    return metal::mix(_e36, 1.0, note);
}

struct fs_shadow_coverageInput {
    metal::float2 local [[user(loc0), center_perspective]];
    metal::float2 half_extent [[user(loc1), flat]];
    float shear [[user(loc2), flat]];
    float outline_reach [[user(loc3), flat]];
    float lead [[user(loc4), flat]];
    float lead_fade [[user(loc5), flat]];
    float lead_alpha [[user(loc6), flat]];
    float cap_reach [[user(loc7), flat]];
    metal::float4 core [[user(loc8), flat]];
    metal::float4 outline [[user(loc9), flat]];
    metal::float2 at [[user(loc10), center_perspective]];
    uint who [[user(loc11), flat]];
    float feather [[user(loc12), flat]];
    metal::float4 fade [[user(loc13), flat]];
};
struct fs_shadow_coverageOutput {
    metal::float4 member [[color(0)]];
};
fragment fs_shadow_coverageOutput fs_shadow_coverage(
  fs_shadow_coverageInput varyings [[stage_in]]
, metal::float4 position [[position]]
) {
    const VertexOut in = { position, varyings.local, varyings.half_extent, varyings.shear, varyings.outline_reach, varyings.lead, varyings.lead_fade, varyings.lead_alpha, varyings.cap_reach, {}, varyings.core, varyings.outline, varyings.at, varyings.who, varyings.feather, varyings.fade };
    float _e1 = box_distance(in);
    float _e3 = inside(in, _e1, 0.0);
    float _e4 = lead_coverage(in);
    float coverage = _e3 * _e4;
    return fs_shadow_coverageOutput { metal::float4(coverage, 0.0, 0.0, 1.0) };
}
