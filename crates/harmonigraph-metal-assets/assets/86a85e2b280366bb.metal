// language: metal3.2
#include <metal_stdlib>
#include <simd/simd.h>

using metal::uint;

struct Locals {
    metal::float2 origin_points;
    metal::float2 viewport_points;
    float feather;
    float _pad;
    metal::float2 pitch_dir;
    metal::float2 depth_dir;
    metal::float2 _axis_pad;
    metal::float4 shadow;
    metal::float2 shadow_atlas_size;
    float shadow_falloff;
    float _shadow_pad;
};
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
    char _pad13[4];
};
constant float DISTANCE_KIND = 1.0;
constant float GAUSSIAN_GAIN = 2.5;
constant float SHADOW_FALLOFF_FLOOR = 0.35;
constant float SHADOW_TAIL = 4.0;
constant float SHADOW_FALLOFF_FREE = 0.64025325;
constant float SHADOW_STOP = 2.0;
constant float SHADOW_INVISIBLE_FOLDS = 1.5586027;
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
    float d,
    float edge,
    constant Locals& locals
) {
    float _e4 = locals.feather;
    float f = metal::max(_e4, 0.000001);
    return metal::clamp(((edge - d) / f) + 0.5, 0.0, 1.0);
}

float lead_coverage(
    VertexOut in_3,
    constant Locals& locals
) {
    float led = {};
    if (in_3.lead <= 0.0) {
        return 1.0;
    }
    float _e7 = locals.feather;
    float f_1 = metal::max(_e7, 0.000001);
    float u = in_3.local.y + in_3.half_extent.y;
    led = in_3.lead_alpha;
    if (in_3.lead_fade > 0.0) {
        led = in_3.lead_alpha * metal::clamp(u / metal::max(metal::min(in_3.lead_fade, in_3.lead), f_1), 0.0, 1.0);
    }
    float note = metal::clamp(((u - in_3.lead) / f_1) + 0.5, 0.0, 1.0);
    float _e38 = led;
    return metal::mix(_e38, 1.0, note);
}

metal::float4 core_color(
    VertexOut in_4,
    constant Locals& locals
) {
    float across_1 = metal::abs(in_4.local.x - (in_4.shear * in_4.local.y)) / metal::max(in_4.half_extent.x, 0.0001);
    float width = (2.0 * in_4.half_extent.x) / metal::sqrt(1.0 + (in_4.shear * in_4.shear));
    float shoulder = metal::smoothstep(0.3, 1.0, across_1) * metal::smoothstep(1.5, 3.0, width);
    metal::float4 color = metal::float4(in_4.core.xyz * (1.0 - (0.3 * shoulder)), in_4.core.w);
    float _e42 = box_distance(in_4);
    float _e44 = inside(_e42, 0.0, locals);
    float _e46 = lead_coverage(in_4, locals);
    return (color * _e44) * _e46;
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
};
struct fs_core_linearOutput {
    metal::float4 member [[color(0)]];
};
fragment fs_core_linearOutput fs_core_linear(
  fs_core_linearInput varyings [[stage_in]]
, metal::float4 position [[position]]
, constant Locals& locals [[buffer(0)]]
) {
    const VertexOut in = { position, varyings.local, varyings.half_extent, varyings.shear, varyings.outline_reach, varyings.lead, varyings.lead_fade, varyings.lead_alpha, varyings.cap_reach, {}, varyings.core, varyings.outline, varyings.at, varyings.who };
    metal::float4 _e1 = core_color(in, locals);
    metal::float3 _e3 = linear_from_gamma_rgb(_e1.xyz);
    return fs_core_linearOutput { metal::float4(_e3, _e1.w) };
}
