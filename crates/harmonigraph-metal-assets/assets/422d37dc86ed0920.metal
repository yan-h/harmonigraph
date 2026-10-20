// language: metal3.2
#include <metal_stdlib>
#include <simd/simd.h>

using metal::uint;

struct CoverOut {
    metal::float4 position;
    metal::float2 uv;
    char _pad2[8];
};
constant uint OCCLUDER_HEADER = 5u;
constant float GAUSSIAN_GAIN = 2.5;
constant float SHADOW_KEEP_FLOOR = 0.0009765625;
constant float GLOW_BASE = 0.8;

struct vs_coverInput {
};
struct vs_coverOutput {
    metal::float4 position [[position]];
    metal::float2 uv [[user(loc0), center_perspective]];
};
vertex vs_coverOutput vs_cover(
  uint vi [[vertex_id]]
) {
    CoverOut out = {};
    metal::float2 corner = metal::float2(static_cast<float>(vi & 1u), static_cast<float>(vi >> 1u));
    out.position = metal::float4((corner * 2.0) - metal::float2(1.0), 0.0, 1.0);
    out.uv = metal::float2(corner.x, 1.0 - corner.y);
    CoverOut _e24 = out;
    const auto _tmp = _e24;
    return vs_coverOutput { _tmp.position, _tmp.uv };
}
