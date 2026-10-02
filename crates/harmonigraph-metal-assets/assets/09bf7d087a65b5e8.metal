// language: metal3.2
#include <metal_stdlib>
#include <simd/simd.h>

using metal::uint;

struct TileVertex {
    metal::float4 position;
    uint layer;
    char _pad2[12];
};
constant float DISTANCE_KIND = 1.0;
constant float DISTANCE_COVERAGE_KIND = 2.0;
constant uint OCCLUDER_HEADER = 5u;
constant float GAUSSIAN_GAIN = 2.5;
constant float SHADOW_STOP = 1.0;
constant float SHADOW_FALLOFF_MIN = -6.0;
constant float SHADOW_FALLOFF_MAX = 6.0;
constant float SHADOW_KEEP_FLOOR = 0.0009765625;
constant float GLOW_BASE = 0.8;
constant int STAR_ATLAS_WIDTH = 2048;
constant uint STAR_ATLAS_SHIFT = 11u;
constant uint STAR_SLICES = 5u;
constant float STAR_PANE = 540.0;
constant int STAR_HASH_PERIOD = 65536;
constant uint STAR_LIFE_PERIOD = 4096u;
constant float STAR_FADE = 0.2;
constant float STAR_LIFT = 0.18;
constant float STAR_INNER_FADE = 0.7;
constant uint STAR_FAR_LAYERS = 3u;

struct vs_starsInput {
};
struct vs_starsOutput {
    metal::float4 position [[position]];
    uint layer [[user(loc0), flat]];
};
vertex vs_starsOutput vs_stars(
  uint vertex_ [[vertex_id]]
, uint layer [[instance_id]]
) {
    metal::float2 uv = metal::float2(static_cast<float>((vertex_ << 1u) & 2u), static_cast<float>(vertex_ & 2u));
    const auto _tmp = TileVertex {metal::float4((uv * metal::float2(2.0, -2.0)) + metal::float2(-1.0, 1.0), 0.0, 1.0), layer};
    return vs_starsOutput { _tmp.position, _tmp.layer };
}
