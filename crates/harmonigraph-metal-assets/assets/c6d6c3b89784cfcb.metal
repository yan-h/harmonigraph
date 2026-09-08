// language: metal3.2
#include <metal_stdlib>
#include <simd/simd.h>

using metal::uint;

struct BlitOut {
    metal::float4 pos;
    metal::float2 uv;
    char _pad2[8];
};
constant float BLOOM_THRESHOLD = 0.35;
constant float BLOOM_KNEE = 0.25;
constant float BLUR_W0_ = 0.227027;
constant metal::float4 BLUR_W = metal::float4(0.1945946, 0.1216216, 0.054054, 0.016216);

struct vs_blitInput {
};
struct vs_blitOutput {
    metal::float4 pos [[position]];
    metal::float2 uv [[user(loc0), center_perspective]];
};
vertex vs_blitOutput vs_blit(
  uint vi [[vertex_id]]
) {
    BlitOut out = {};
    metal::float2 corner = metal::float2(static_cast<float>(vi & 1u), static_cast<float>(vi >> 1u));
    out.pos = metal::float4((corner * 2.0) - metal::float2(1.0), 0.0, 1.0);
    out.uv = metal::float2(corner.x, 1.0 - corner.y);
    BlitOut _e24 = out;
    const auto _tmp = _e24;
    return vs_blitOutput { _tmp.pos, _tmp.uv };
}
