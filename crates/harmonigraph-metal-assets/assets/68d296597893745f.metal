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
};
constant float DISTANCE_KIND = 1.0;
constant float DISTANCE_COVERAGE_KIND = 2.0;
constant float GAUSSIAN_GAIN = 2.5;
constant float SHADOW_STOP = 1.0;
constant float SHADOW_FALLOFF_MIN = -6.0;
constant float SHADOW_FALLOFF_MAX = 6.0;
constant float SHADOW_KEEP_FLOOR = 0.0009765625;

float box_distance_trimmed(
    VertexOut in_1,
    float trim
) {
    float slope = in_1.shear;
    float skew = metal::sqrt(1.0 + (slope * slope));
    float across = (in_1.local.x - (slope * in_1.local.y)) / skew;
    float half_across = in_1.half_extent.x / skew;
    float along = in_1.local.y - (0.5 * trim);
    float half_along = in_1.half_extent.y - (0.5 * trim);
    metal::float2 q = metal::float2(metal::abs(across) - half_across, metal::abs(along) - half_along);
    return metal::min(metal::max(q.x, q.y), 0.0) + metal::length(metal::max(q, metal::float2(0.0)));
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

metal::float4 core_color(
    VertexOut in_5
) {
    float _e2 = box_distance(in_5);
    float _e4 = inside(in_5, _e2, 0.0);
    float _e6 = lead_coverage(in_5);
    return (in_5.core * _e4) * _e6;
}

metal::float3 linear_from_gamma_rgb(
    metal::float3 srgb
) {
    metal::bool3 cutoff = srgb < metal::float3(0.04045);
    metal::float3 lower = srgb / metal::float3(12.92);
    metal::float3 higher = metal::pow((srgb + metal::float3(0.055)) / metal::float3(1.055), metal::float3(2.4));
    return metal::select(higher, lower, cutoff);
}

struct fs_core_linearInput {
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
};
struct fs_core_linearOutput {
    metal::float4 member [[color(0)]];
};
fragment fs_core_linearOutput fs_core_linear(
  fs_core_linearInput varyings [[stage_in]]
, metal::float4 position [[position]]
) {
    const VertexOut in = { position, varyings.local, varyings.half_extent, varyings.shear, varyings.outline_reach, varyings.lead, varyings.lead_fade, varyings.lead_alpha, varyings.cap_reach, {}, varyings.core, varyings.outline, varyings.at, varyings.who, varyings.feather };
    metal::float4 _e1 = core_color(in);
    metal::float3 _e3 = linear_from_gamma_rgb(_e1.xyz);
    return fs_core_linearOutput { metal::float4(_e3, _e1.w) };
}
