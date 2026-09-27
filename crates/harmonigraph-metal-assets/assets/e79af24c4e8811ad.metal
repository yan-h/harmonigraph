// language: metal3.2
#include <metal_stdlib>
#include <simd/simd.h>

using metal::uint;

struct Vertex {
    metal::float4 position;
    metal::float2 uv;
    char _pad2[8];
};
constant float CLOUD_TILE_ROT_SIN = 0.6;
constant float CLOUD_TILE_ROT_COS = 0.8;
constant int WASH_RING = 2;
constant float WASH_JITTER = 0.4;
constant float WASH_RADIUS_MIN = 1.17;
constant float WASH_RADIUS_MAX = 1.91;
constant float WASH_CELLS = 5.25;
constant float WASH_LACUNARITY = 2.1;
constant float WASH_FINE_OCCUPANCY = 0.2;
constant float WASH_WARP = 0.45;
constant float WASH_WARP_SCALE = 0.9;
constant float WASH_FBM_FINE = 2.07;
constant float WASH_FBM_FINE_TILED = 2.0;
constant float PERIOD = 40.0;

struct vs_fullscreenInput {
};
struct vs_fullscreenOutput {
    metal::float4 position [[position]];
    metal::float2 uv [[user(loc0), center_perspective]];
};
vertex vs_fullscreenOutput vs_fullscreen(
  uint i [[vertex_id]]
) {
    metal::float2 uv = metal::float2(static_cast<float>(i & 1u), static_cast<float>(i >> 1u));
    const auto _tmp = Vertex {metal::float4((uv.x * 2.0) - 1.0, 1.0 - (uv.y * 2.0), 0.0, 1.0), uv};
    return vs_fullscreenOutput { _tmp.position, _tmp.uv };
}
