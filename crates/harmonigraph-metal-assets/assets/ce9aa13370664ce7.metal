// language: metal3.2
#include <metal_stdlib>
#include <simd/simd.h>

using metal::uint;

struct TileVertex {
    metal::float4 position;
    metal::float2 fraction;
    char _pad2[8];
};
constant float CLOUD_UNITS = 10.0;
constant float CLOUD_TILE_ROT_COS = 0.8;
constant float CLOUD_TILE_ROT_SIN = 0.6;
constant float SCALE_CELLS = 2.7272727;
constant float DOME_RADIUS = 1.15;
constant float DOME_JITTER = 0.3;
constant float DOME_RADIUS_MIN = 0.95;
constant float DOME_RADIUS_MAX = 1.32;
constant float DOME_UNION = 9.0;
constant float DOME_VARIETY_GAIN = 5.0;
constant float DOME_FACE = 1.5122874;
constant float DOME_LACUNARITY = 2.1;
constant float DOME_FINE_GAIN = 0.22;
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
constant uint STAR_SLICES = 5u;
constant float STAR_PANE = 540.0;
constant float STAR_JITTER = 0.6;
constant int STAR_ATLAS_WIDTH = 2048;
constant uint STAR_ATLAS_SHIFT = 11u;
constant int STAR_HASH_PERIOD = 65536;
constant uint STAR_LIFE_PERIOD = 4096u;
constant float STAR_FADE = 0.2;
constant float STAR_LIFT = 0.18;
constant float STAR_RING_FADE = 0.7;

struct vs_cloud_tileInput {
};
struct vs_cloud_tileOutput {
    metal::float4 position [[position]];
    metal::float2 fraction [[user(loc0), center_perspective]];
};
vertex vs_cloud_tileOutput vs_cloud_tile(
  uint vertex_ [[vertex_id]]
) {
    TileVertex out = {};
    metal::float2 uv = metal::float2(static_cast<float>((vertex_ << 1u) & 2u), static_cast<float>(vertex_ & 2u));
    out.position = metal::float4((uv * metal::float2(2.0, -2.0)) + metal::float2(-1.0, 1.0), 0.0, 1.0);
    out.fraction = uv;
    TileVertex _e24 = out;
    const auto _tmp = _e24;
    return vs_cloud_tileOutput { _tmp.position, _tmp.fraction };
}
