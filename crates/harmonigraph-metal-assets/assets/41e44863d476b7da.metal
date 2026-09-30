// language: metal3.2
#include <metal_stdlib>
#include <simd/simd.h>

using metal::uint;

struct StarHaloSample {
    metal::float2 size;
    uint group;
    uint layer;
};
struct StarSlice {
    metal::float2 offset;
    float cell;
    float sigma;
    float cap;
    float defocus;
    float fringe;
    int base;
    metal::int2 origin;
    metal::int2 grid;
};
struct type_9 {
    StarSlice inner[5];
};
struct type_10 {
    StarHaloSample inner[5];
};
struct StarUniforms {
    metal::float2 origin;
    metal::float2 size;
    float ppp;
    float star_randomness;
    float star_life;
    float star_size_variation;
    metal::float4 star_far;
    metal::float4 star_near;
    metal::float4 star_geometry;
    type_9 star_slices;
    type_10 star_halo_samples;
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
constant float STAR_HALO_REACH = 1.2;
constant float STAR_HALO_FADE = 0.7;
constant int STAR_HASH_PERIOD = 65536;
constant uint STAR_LIFE_PERIOD = 4096u;
constant float STAR_FADE = 0.2;
constant float STAR_LIFT = 0.18;
constant uint STAR_FAR_LAYERS = 3u;

metal::float4 star_hash(
    metal::int2 cell,
    uint salt
) {
    uint n = {};
    n = (as_type<uint>(cell.x) * 2654435769u) ^ (as_type<uint>(cell.y) * 2246822507u);
    uint _e12 = n;
    n = _e12 ^ (salt * 668265261u);
    uint _e16 = n;
    uint _e17 = n;
    n = (_e16 ^ (_e17 >> 16u)) * 2146121005u;
    uint _e23 = n;
    uint _e24 = n;
    n = (_e23 ^ (_e24 >> 15u)) * 2221713035u;
    uint _e30 = n;
    uint _e31 = n;
    n = _e30 ^ (_e31 >> 16u);
    uint _e35 = n;
    uint _e36 = n;
    uint _e39 = n;
    uint _e42 = n;
    metal::uint4 bytes = metal::uint4(_e35, _e36 >> 8u, _e39 >> 16u, _e42 >> 24u) & metal::uint4(255u);
    return (static_cast<metal::float4>(bytes) + metal::float4(0.5)) / metal::float4(256.0);
}

float star_life(
    constant Settings& settings
) {
    float _e3 = settings.stars.star_life;
    return _e3;
}

metal::float4 star_geometry(
    constant Settings& settings
) {
    metal::float4 _e3 = settings.stars.star_geometry;
    return _e3;
}

metal::float2 star_size(
    constant Settings& settings
) {
    metal::float2 _e3 = settings.stars.size;
    return _e3;
}

float star_randomness(
    constant Settings& settings
) {
    float _e3 = settings.stars.star_randomness;
    return _e3;
}

metal::float4 star_source(
    metal::float2 pt,
    float rank,
    int index,
    constant Settings& settings,
    metal::texture2d<float, metal::access::sample> source,
    metal::sampler cloud_sampler
) {
    metal::float2 _e8 = settings.stars.size;
    metal::float4 light = source.sample(cloud_sampler, pt / _e8, metal::level(0.0));
    if (light.w <= 0.0) {
        return metal::float4(0.0);
    }
    float randomness = settings.stars.star_randomness;
    float spread = (1.0 - randomness) + (randomness * (0.35 + (0.65 * rank)));
    float lift = (0.09 * rank) * metal::smoothstep(0.0, 0.15, light.w);
    float level = metal::clamp((light.w * spread) + lift, 0.0, 1.0);
    return metal::float4((light.xyz / metal::float3(light.w)) * level, 1.0);
}

float star_size_variation(
    constant Settings& settings
) {
    float _e3 = settings.stars.star_size_variation;
    return _e3;
}

uint naga_f2u32(float value) {
    return static_cast<uint>(metal::clamp(value, 0.0, 4294967000.0));
}

metal::uint3 naga_f2u32(metal::float3 value) {
    return static_cast<metal::uint3>(metal::clamp(value, 0.0, 4294967000.0));
}

metal::uint4 star_bake(
    StarSlice s,
    metal::int2 cell_1,
    uint salt_1,
    int index_1,
    constant Settings& settings,
    metal::texture2d<float, metal::access::sample> source,
    metal::sampler cloud_sampler
) {
    float fade = {};
    metal::uint3 tens = {};
    metal::int2 hashed = cell_1 & metal::int2(65535);
    float _e7 = star_life(settings);
    metal::float4 _e10 = star_hash(hashed, salt_1 + 2u);
    float age = _e7 + _e10.x;
    uint life = naga_f2u32(metal::floor(age)) & 4095u;
    uint key = salt_1 + ((life + 1u) << 16u);
    metal::float4 _e22 = star_hash(hashed, key);
    float through = metal::fract(age);
    metal::float4 _e24 = star_geometry(settings);
    metal::float2 centre = metal::float2(0.5) + (_e24.x * (_e22.xy - metal::float2(0.5)));
    metal::float2 _e40 = star_size(settings);
    metal::float2 _e45 = star_size(settings);
    metal::float2 at = ((((static_cast<metal::float2>(cell_1) + centre) + s.offset) * s.cell) * (_e40.y / STAR_PANE)) + (_e45 * 0.5);
    metal::float4 _e51 = star_hash(hashed, key + 1u);
    float _e52 = star_randomness(settings);
    metal::float4 _e64 = star_source(at, metal::pow(_e51.x, 1.0 + (6.0 * _e52)) * (2.0 + (6.0 * _e52)), index_1, settings, source, cloud_sampler);
    if (_e64.w <= 0.0) {
        return metal::uint4(0u);
    }
    metal::float3 colour = _e64.xyz;
    float _e71 = star_size_variation(settings);
    float size = metal::exp((-2.4 * _e71) * _e51.y);
    float sigma = metal::min(s.sigma * size, s.cap) * s.defocus;
    fade = metal::smoothstep(0.0, STAR_FADE, through) * metal::smoothstep(0.0, STAR_FADE, 1.0 - through);
    tens = naga_f2u32(metal::rint(metal::clamp(colour, metal::float3(0.0), metal::float3(1.0)) * 1023.0));
    if (_e64.w != 1.0) {
        float _e106 = fade;
        fade = _e106 * _e64.w;
    }
    uint _e114 = tens.x;
    uint _e118 = tens.y;
    uint _e123 = tens.z;
    float _e127 = fade;
    return metal::uint4(as_type<uint>(centre.x), as_type<uint>(centre.y), ((_e114 << 20u) | (_e118 << 10u)) | _e123, as_type<uint>(half2(metal::float2(1.0 / sigma, _e127))));
}

StarSlice star_slice(
    uint k_1,
    constant Settings& settings
) {
    StarSlice _e5 = settings.stars.star_slices.inner[metal::min(unsigned(k_1), 4u)];
    return _e5;
}
metal::int2 naga_f2i32(metal::float2 value) {
    return static_cast<metal::int2>(metal::clamp(value, -2147483600.0, 2147483500.0));
}

int naga_mod(int lhs, int rhs) {
    int divisor = metal::select(rhs, 1, (lhs == (-2147483647 - 1) & rhs == -1) | (rhs == 0));
    return lhs - (lhs / divisor) * divisor;
}

int naga_div(int lhs, int rhs) {
    return lhs / metal::select(rhs, 1, (lhs == (-2147483647 - 1) & rhs == -1) | (rhs == 0));
}


struct fs_star_bakeInput {
    uint layer [[user(loc0), flat]];
};
struct fs_star_bakeOutput {
    metal::uint4 member [[color(0)]];
};
fragment fs_star_bakeOutput fs_star_bake(
  fs_star_bakeInput varyings [[stage_in]]
, metal::float4 position [[position]]
, constant Settings& settings [[buffer(0)]]
, metal::texture2d<float, metal::access::sample> source [[texture(0)]]
, metal::sampler cloud_sampler [[sampler(0)]]
) {
    const TileVertex in = { position, varyings.layer };
    uint k = 0u;
    bool local = {};
    metal::int2 texel = naga_f2i32(metal::floor(in.position.xy));
    int index_2 = as_type<int>(as_type<uint>(as_type<int>(as_type<uint>(texel.y) * as_type<uint>(STAR_ATLAS_WIDTH))) + as_type<uint>(texel.x));
    uint2 loop_bound = uint2(4294967295u);
    bool loop_init = true;
    while(true) {
        if (metal::all(loop_bound == uint2(0u))) { break; }
        loop_bound -= uint2(loop_bound.y == 0u, 1u);
        if (!loop_init) {
            uint _e46 = k;
            k = _e46 + 1u;
        }
        loop_init = false;
        uint _e12 = k;
        if (_e12 < STAR_SLICES) {
        } else {
            break;
        }
        {
            uint _e15 = k;
            StarSlice _e16 = star_slice(_e15, settings);
            int at_1 = as_type<int>(as_type<uint>(index_2) - as_type<uint>(_e16.base));
            if (at_1 >= 0) {
                local = at_1 < as_type<int>(as_type<uint>(_e16.grid.x) * as_type<uint>(_e16.grid.y));
            } else {
                local = false;
            }
            bool _e30 = local;
            if (_e30) {
                metal::int2 local_1 = metal::int2(naga_mod(at_1, _e16.grid.x), naga_div(at_1, _e16.grid.x));
                uint _e42 = k;
                metal::uint4 _e45 = star_bake(_e16, as_type<metal::int2>(as_type<metal::uint2>(_e16.origin) + as_type<metal::uint2>(local_1)), 1000u + (3u * _e42), index_2, settings, source, cloud_sampler);
                return fs_star_bakeOutput { _e45 };
            }
        }
    }
    return fs_star_bakeOutput { metal::uint4(0u) };
}
