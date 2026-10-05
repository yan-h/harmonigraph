// language: metal3.2
#include <metal_stdlib>
#include <simd/simd.h>

using metal::uint;

struct StarSlice {
    metal::float2 offset;
    float cell;
    float radius;
    float solid;
    float ramp;
    float bend;
    int base;
    metal::int2 origin;
    metal::int2 grid;
    float width;
    float inverse_floor;
    uint gather;
    float twinkle;
};
struct type_8 {
    StarSlice inner[5];
};
struct StarUniforms {
    metal::float2 size;
    metal::float2 star_image;
    float star_randomness;
    float star_life;
    float star_size_variation;
    float pad;
    type_8 star_slices;
};
struct Settings {
    StarUniforms stars;
    float depth;
    float _pad0_;
    float _pad1_;
    float _pad2_;
};
struct TileVertex {
    metal::float4 position;
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

metal::float2 star_size(
    constant Settings& settings
) {
    metal::float2 _e3 = settings.stars.size;
    return _e3;
}

metal::float4 star_color(
    metal::float2 pt,
    constant Settings& settings,
    metal::sampler cloud_sampler,
    metal::texture2d<float, metal::access::sample> cloud_tone
) {
    metal::float2 _e3 = star_size(settings);
    metal::float4 _e6 = cloud_tone.sample(cloud_sampler, pt / _e3, metal::level(0.0));
    return _e6;
}

struct fs_lattice_starsInput {
};
struct fs_lattice_starsOutput {
    metal::float4 member [[color(0)]];
};
fragment fs_lattice_starsOutput fs_lattice_stars(
  metal::float4 position [[position]]
, constant Settings& settings [[buffer(0)]]
, metal::texture2d<float, metal::access::sample> source [[texture(0)]]
, metal::sampler cloud_sampler [[sampler(0)]]
, metal::texture2d<float, metal::access::sample> cloud_tone [[texture(2)]]
) {
    const TileVertex in = { position };
    metal::float4 result = {};
    metal::float2 _e8 = settings.stars.size;
    metal::float4 raw = source.sample(cloud_sampler, in.position.xy / _e8, metal::level(0.0));
    metal::float4 _e14 = star_color(in.position.xy, settings, cloud_sampler, cloud_tone);
    result = _e14;
    float _e17 = result.x;
    float _e19 = result.y;
    float _e22 = result.z;
    float brightest = metal::max(metal::max(_e17, _e19), _e22);
    float _e26 = result.w;
    result.w = metal::max(brightest, metal::min(_e26, 1.0) * raw.w);
    metal::float4 _e32 = result;
    float _e35 = settings.depth;
    return fs_lattice_starsOutput { metal::mix(raw, _e32, _e35) };
}
