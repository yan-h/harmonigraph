// language: metal3.2
#include <metal_stdlib>
#include <simd/simd.h>

using metal::uint;

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
struct type_10 {
    StarSlice inner[5];
};
struct Cloud {
    metal::float2 origin;
    metal::float2 size;
    metal::float2 step;
    float ppp;
    float spread;
    float contours;
    float contour_softness;
    float contour_strength;
    uint tone_baked;
    metal::float2 drift;
    float cloud_depth;
    float scale_size;
    float scale_variety;
    float scale_refract;
    uint cloud_style;
    float wash_size;
    float wash_fuzz;
    float wash_lobe;
    float wash_refract;
    float wash_layers;
    uint tile_cells;
    uint pitch_vertical;
    float star_randomness;
    float star_life;
    type_10 star_slices;
};
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
constant float STAR_REACH = 1.2;
constant int STAR_ATLAS_WIDTH = 2048;
constant uint STAR_ATLAS_SHIFT = 11u;
constant int STAR_HASH_PERIOD = 65536;
constant uint STAR_LIFE_PERIOD = 4096u;
constant float STAR_FADE = 0.2;
constant float STAR_LIFT = 0.18;
constant float STAR_RING_FADE = 0.7;

uint naga_f2u32(float value) {
    return static_cast<uint>(metal::clamp(value, 0.0, 4294967000.0));
}

metal::float3 palette_color(
    float level,
    metal::texture2d<float, metal::access::sample> lut
) {
    uint levels = metal::uint2(lut.get_width(), lut.get_height()).x;
    float x = metal::max((metal::clamp(level, 0.0, 1.0) * static_cast<float>(levels)) - 0.5, 0.0);
    uint i = naga_f2u32(metal::clamp(metal::floor(x), 0.0, static_cast<float>(levels - 1u)));
    uint clamped_lod_e24 = metal::min(uint(0), lut.get_num_mip_levels() - 1);
    metal::float4 _e24 = lut.read(metal::min(metal::uint2(metal::uint2(i, 0u)), metal::uint2(lut.get_width(clamped_lod_e24), lut.get_height(clamped_lod_e24)) - 1), clamped_lod_e24);
    metal::float3 a = _e24.xyz;
    uint clamped_lod_e35 = metal::min(uint(0), lut.get_num_mip_levels() - 1);
    metal::float4 _e35 = lut.read(metal::min(metal::uint2(metal::uint2(metal::min(i + 1u, levels - 1u), 0u)), metal::uint2(lut.get_width(clamped_lod_e35), lut.get_height(clamped_lod_e35)) - 1), clamped_lod_e35);
    metal::float3 b = _e35.xyz;
    return metal::mix(a, b, metal::fract(x));
}

float star_level_at(
    metal::float2 pt,
    metal::texture2d<float, metal::access::sample> close_light,
    metal::sampler cloud_sampler,
    constant Cloud& cloud
) {
    metal::float2 _e3 = cloud.size;
    metal::float2 uv = pt / _e3;
    metal::float4 _e8 = close_light.sample(cloud_sampler, uv, metal::level(0.0));
    return metal::clamp(_e8.x, 0.0, 1.0);
}

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

metal::float3 star_paint(
    float level_1,
    float rank,
    metal::texture2d<float, metal::access::sample> lut,
    constant Cloud& cloud
) {
    float randomness = cloud.star_randomness;
    float spread = (1.0 - randomness) + (randomness * (0.35 + (0.65 * rank)));
    float lift = (0.09 * rank) * metal::smoothstep(0.0, 0.15, level_1);
    metal::float3 _e24 = palette_color(metal::clamp((level_1 * spread) + lift, 0.0, 1.0), lut);
    return _e24;
}

metal::uint3 naga_f2u32(metal::float3 value) {
    return static_cast<metal::uint3>(metal::clamp(value, 0.0, 4294967000.0));
}

metal::uint4 star_bake(
    StarSlice s,
    metal::int2 cell_1,
    uint salt_1,
    metal::texture2d<float, metal::access::sample> lut,
    metal::texture2d<float, metal::access::sample> close_light,
    metal::sampler cloud_sampler,
    constant Cloud& cloud
) {
    metal::int2 hashed = cell_1 & metal::int2(65535);
    float _e8 = cloud.star_life;
    metal::float4 _e11 = star_hash(hashed, salt_1 + 2u);
    float age = _e8 + _e11.x;
    uint life = naga_f2u32(metal::floor(age)) & 4095u;
    uint key = salt_1 + ((life + 1u) << 16u);
    metal::float4 _e23 = star_hash(hashed, key);
    float through = metal::fract(age);
    metal::float2 centre = metal::float2(0.5) + (STAR_JITTER * (_e23.xy - metal::float2(0.5)));
    float _e43 = cloud.size.y;
    metal::float2 _e49 = cloud.size;
    metal::float2 at = ((((static_cast<metal::float2>(cell_1) + centre) + s.offset) * s.cell) * (_e43 / STAR_PANE)) + (_e49 * 0.5);
    float _e53 = star_level_at(at, close_light, cloud_sampler, cloud);
    if (_e53 <= 0.0) {
        return metal::uint4(0u);
    }
    metal::float4 _e60 = star_hash(hashed, key + 1u);
    float randomness_1 = cloud.star_randomness;
    metal::float3 _e75 = star_paint(_e53, metal::pow(_e60.x, 1.0 + (6.0 * randomness_1)) * (2.0 + (6.0 * randomness_1)), lut, cloud);
    float size = metal::exp(((0.3 + (0.9 * randomness_1)) * (_e60.y - 0.5)) * 2.0);
    float sigma = metal::min(s.sigma * size, s.cap) * s.defocus;
    float fade = metal::smoothstep(0.0, STAR_FADE, through) * metal::smoothstep(0.0, STAR_FADE, 1.0 - through);
    metal::uint3 tens = naga_f2u32(metal::rint(metal::clamp(_e75, metal::float3(0.0), metal::float3(1.0)) * 1023.0));
    return metal::uint4(as_type<uint>(centre.x), as_type<uint>(centre.y), ((tens.x << 20u) | (tens.y << 10u)) | tens.z, as_type<uint>(half2(metal::float2(sigma, fade))));
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
    metal::float2 fraction [[user(loc0), center_perspective]];
};
struct fs_star_bakeOutput {
    metal::uint4 member [[color(0)]];
};
fragment fs_star_bakeOutput fs_star_bake(
  fs_star_bakeInput varyings [[stage_in]]
, metal::float4 position [[position]]
, metal::texture2d<float, metal::access::sample> lut [[texture(0)]]
, metal::texture2d<float, metal::access::sample> close_light [[texture(1)]]
, metal::sampler cloud_sampler [[sampler(0)]]
, constant Cloud& cloud [[buffer(2)]]
) {
    const TileVertex in = { position, varyings.fraction };
    uint k = 0u;
    bool local = {};
    metal::int2 texel = naga_f2i32(metal::floor(in.position.xy));
    int index = as_type<int>(as_type<uint>(as_type<int>(as_type<uint>(texel.y) * as_type<uint>(STAR_ATLAS_WIDTH))) + as_type<uint>(texel.x));
    uint2 loop_bound = uint2(4294967295u);
    bool loop_init = true;
    while(true) {
        if (metal::all(loop_bound == uint2(0u))) { break; }
        loop_bound -= uint2(loop_bound.y == 0u, 1u);
        if (!loop_init) {
            uint _e49 = k;
            k = _e49 + 1u;
        }
        loop_init = false;
        uint _e12 = k;
        if (_e12 < STAR_SLICES) {
        } else {
            break;
        }
        {
            uint _e17 = k;
            StarSlice s_1 = cloud.star_slices.inner[metal::min(unsigned(_e17), 4u)];
            int at_1 = as_type<int>(as_type<uint>(index) - as_type<uint>(s_1.base));
            if (at_1 >= 0) {
                local = at_1 < as_type<int>(as_type<uint>(s_1.grid.x) * as_type<uint>(s_1.grid.y));
            } else {
                local = false;
            }
            bool _e33 = local;
            if (_e33) {
                metal::int2 local_1 = metal::int2(naga_mod(at_1, s_1.grid.x), naga_div(at_1, s_1.grid.x));
                uint _e45 = k;
                metal::uint4 _e48 = star_bake(s_1, as_type<metal::int2>(as_type<metal::uint2>(s_1.origin) + as_type<metal::uint2>(local_1)), 1000u + (3u * _e45), lut, close_light, cloud_sampler, cloud);
                return fs_star_bakeOutput { _e48 };
            }
        }
    }
    return fs_star_bakeOutput { metal::uint4(0u) };
}
