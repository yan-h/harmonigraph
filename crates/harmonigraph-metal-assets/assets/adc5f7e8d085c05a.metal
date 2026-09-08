// language: metal3.2
#include <metal_stdlib>
#include <simd/simd.h>

using metal::uint;

struct PadOut {
    metal::float4 position;
    float pad;
    char _pad2[12];
};
constant float DISTANCE_KIND = 1.0;
constant float GAUSSIAN_GAIN = 2.5;
constant float SHADOW_FALLOFF_FLOOR = 0.35;
constant float SHADOW_TAIL = 4.0;
constant float SHADOW_FALLOFF_FREE = 0.64025325;
constant float SHADOW_STOP = 2.0;
constant float SHADOW_INVISIBLE_FOLDS = 1.5586027;
constant float SHADOW_KEEP_FLOOR = 0.0009765625;
constant uint SHEET_MARK = 1u;
constant float PATCH_MARGIN = 0.5;
constant float FILTER_TAP = 0.25;
constant float SDF_NEAR_PAD = 32.0;
constant float SDF_COARSE_PAD = 48.0;
constant float SDF_NEAR_BLEND = 8.0;

struct fs_distance_padInput {
    float pad [[user(loc0), flat]];
};
struct fs_distance_padOutput {
    metal::float4 member [[color(0)]];
};
fragment fs_distance_padOutput fs_distance_pad(
  fs_distance_padInput varyings [[stage_in]]
, metal::float4 position [[position]]
) {
    const PadOut in = { position, varyings.pad };
    return fs_distance_padOutput { metal::float4(in.pad, 0.0, 0.0, 1.0) };
}
