// language: metal3.2
#include <metal_stdlib>
#include <simd/simd.h>

using metal::uint;

struct DotShadowOut {
    metal::float4 position;
    metal::float2 local;
    float radius;
    char _pad3[4];
    metal::float2 at;
    uint who;
    char _pad5[4];
};
constant float DISTANCE_KIND = 1.0;
constant float GAUSSIAN_GAIN = 2.5;
constant float SHADOW_FALLOFF_FLOOR = 0.35;
constant float SHADOW_TAIL = 4.0;
constant float SHADOW_FALLOFF_FREE = 0.64025325;
constant float SHADOW_STOP = 2.0;
constant float SHADOW_INVISIBLE_FOLDS = 1.5586027;
constant float SHADOW_KEEP_FLOOR = 0.0009765625;

float dot_distance(
    DotShadowOut in_1
) {
    return metal::length(in_1.local) - in_1.radius;
}

struct fs_dot_shadow_coverageInput {
    metal::float2 local [[user(loc0), center_perspective]];
    float radius [[user(loc1), flat]];
    metal::float2 at [[user(loc2), center_perspective]];
    uint who [[user(loc3), flat]];
};
struct fs_dot_shadow_coverageOutput {
    metal::float4 member [[color(0)]];
};
fragment fs_dot_shadow_coverageOutput fs_dot_shadow_coverage(
  fs_dot_shadow_coverageInput varyings [[stage_in]]
, metal::float4 position [[position]]
) {
    const DotShadowOut in = { position, varyings.local, varyings.radius, {}, varyings.at, varyings.who };
    float _e1 = dot_distance(in);
    float _e2 = metal::fwidth(_e1);
    float aa = metal::max(_e2, 0.000001);
    float coverage = metal::clamp(0.5 - (_e1 / aa), 0.0, 1.0);
    return fs_dot_shadow_coverageOutput { metal::float4(coverage, 0.0, 0.0, 1.0) };
}
