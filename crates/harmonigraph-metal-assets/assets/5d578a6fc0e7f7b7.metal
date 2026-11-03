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
struct Cloud {
    metal::float2 origin;
    metal::float2 size;
    metal::float2 step;
    float ppp;
    uint tone_baked;
    metal::float2 drift;
    float cloud_depth;
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
    float star_size_variation;
    float star_cover;
    uint star_pad1_;
    metal::float4 star_image_size;
    type_8 star_slices;
    uint memory_enabled;
    uint memory_valid;
    float pickup_alpha;
    float release_alpha;
    metal::int2 memory_shift;
    metal::float2 memory_fraction;
    float previous_life;
    float wash_randomness;
    metal::float2 memory_extent;
    type_8 previous_slices;
    metal::float4 velvet;
    metal::float4 velvet_form;
    metal::float4 wash_pigment;
};
struct TileVertex {
    metal::float4 position;
    metal::float2 fraction;
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
constant float WASH_FBM_FINE = 2.0;
constant int STAR_ATLAS_WIDTH = 2048;
constant uint STAR_ATLAS_SHIFT = 11u;
constant uint STAR_SLICES = 5u;
constant float STAR_PANE = 540.0;
constant int STAR_HASH_PERIOD = 65536;
constant uint STAR_LIFE_PERIOD = 4096u;
constant float STAR_FADE = 0.2;
constant float STAR_LIFT = 0.18;
constant float CLOUD_UNITS = 10.0;
constant float WASH_POOL = 0.44;
constant float WASH_PIG_DEPTH = 0.35;
constant float WASH_BLOOM = 2.7;

metal::int2 atlas_texel(
    int index
) {
    return metal::int2(index & 2047, index >> STAR_ATLAS_SHIFT);
}

metal::float2 star_size(
    constant Cloud& cloud
) {
    metal::float2 _e2 = cloud.size;
    return _e2;
}

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

StarSlice star_slice(
    uint k,
    constant Cloud& cloud
) {
    StarSlice _e4 = cloud.star_slices.inner[metal::min(unsigned(k), 4u)];
    return _e4;
}

float star_profile(
    StarSlice s,
    float t
) {
    if (t >= 1.0) {
        return 0.0;
    }
    float u = metal::saturate((t - s.solid) * s.ramp);
    float x_1 = (u * u) * (3.0 - (2.0 * u));
    return (1.0 - x_1) / (1.0 + (s.bend * x_1));
}

metal::float4 star_texel(
    StarSlice s_1,
    metal::float2 f,
    int index_1,
    metal::texture2d<uint, metal::access::sample> star_atlas
) {
    metal::int2 _e4 = atlas_texel(index_1);
    uint clamped_lod_e6 = metal::min(uint(0), star_atlas.get_num_mip_levels() - 1);
    metal::uint4 t_1 = star_atlas.read(metal::min(metal::uint2(_e4), metal::uint2(star_atlas.get_width(clamped_lod_e6), star_atlas.get_height(clamped_lod_e6)) - 1), clamped_lod_e6);
    if (t_1.w == 0u) {
        return metal::float4(0.0);
    }
    float dist = metal::length(f - metal::float2(as_type<float>(t_1.x), as_type<float>(t_1.y))) * s_1.cell;
    metal::float2 shape = float2(as_type<half2>(t_1.w));
    float _e25 = star_profile(s_1, dist * shape.x);
    float cover = _e25 * shape.y;
    if (cover <= 0.0) {
        return metal::float4(0.0);
    }
    metal::float3 colour = static_cast<metal::float3>(metal::uint3(t_1.z >> 20u, t_1.z >> 10u, t_1.z) & metal::uint3(1023u)) / metal::float3(1023.0);
    return metal::float4(colour * cover, cover);
}

metal::int2 naga_f2i32(metal::float2 value) {
    return static_cast<metal::int2>(metal::clamp(value, -2147483600.0, 2147483500.0));
}

metal::float4 star_gather2_(
    StarSlice s_2,
    metal::float2 r,
    metal::texture2d<uint, metal::access::sample> star_atlas
) {
    metal::float4 result = metal::float4(0.0);
    metal::float2 o = metal::floor(r - metal::float2(0.5));
    metal::float2 f_2 = r - o;
    metal::int2 local = as_type<metal::int2>(as_type<metal::uint2>(as_type<metal::int2>(as_type<metal::uint2>(naga_f2i32(o)) - as_type<metal::uint2>(naga_f2i32(metal::floor(s_2.offset))))) - as_type<metal::uint2>(s_2.origin));
    int index_3 = as_type<int>(as_type<uint>(as_type<int>(as_type<uint>(s_2.base) + as_type<uint>(as_type<int>(as_type<uint>(local.y) * as_type<uint>(s_2.grid.x))))) + as_type<uint>(local.x));
    metal::float4 _e25 = result;
    metal::float4 _e26 = star_texel(s_2, f_2, index_3, star_atlas);
    result = _e25 + _e26;
    metal::float4 _e28 = result;
    metal::float4 _e35 = star_texel(s_2, f_2 - metal::float2(1.0, 0.0), as_type<int>(as_type<uint>(index_3) + as_type<uint>(1)), star_atlas);
    result = _e28 + _e35;
    metal::float4 _e37 = result;
    metal::float4 _e45 = star_texel(s_2, f_2 - metal::float2(0.0, 1.0), as_type<int>(as_type<uint>(index_3) + as_type<uint>(s_2.grid.x)), star_atlas);
    result = _e37 + _e45;
    metal::float4 _e47 = result;
    metal::float4 _e57 = star_texel(s_2, f_2 - metal::float2(1.0, 1.0), as_type<int>(as_type<uint>(as_type<int>(as_type<uint>(index_3) + as_type<uint>(s_2.grid.x))) + as_type<uint>(1)), star_atlas);
    result = _e47 + _e57;
    metal::float4 _e59 = result;
    return _e59;
}

metal::float4 star_gather3_(
    StarSlice s_3,
    metal::float2 f_1,
    int index_2,
    metal::texture2d<uint, metal::access::sample> star_atlas
) {
    metal::float4 result_1 = metal::float4(0.0);
    int y = -1;
    uint2 loop_bound = uint2(4294967295u);
    bool loop_init = true;
    while(true) {
        if (metal::all(loop_bound == uint2(0u))) { break; }
        loop_bound -= uint2(loop_bound.y == 0u, 1u);
        if (!loop_init) {
            int _e43 = y;
            y = as_type<int>(as_type<uint>(_e43) + as_type<uint>(1));
        }
        loop_init = false;
        int _e8 = y;
        if (_e8 <= 1) {
        } else {
            break;
        }
        {
            int _e11 = y;
            int row = as_type<int>(as_type<uint>(index_2) + as_type<uint>(as_type<int>(as_type<uint>(_e11) * as_type<uint>(s_3.grid.x))));
            int _e17 = y;
            float fy = f_1.y - static_cast<float>(_e17);
            metal::float4 _e20 = result_1;
            metal::float4 _e27 = star_texel(s_3, metal::float2(f_1.x + 1.0, fy), as_type<int>(as_type<uint>(row) - as_type<uint>(1)), star_atlas);
            result_1 = _e20 + _e27;
            metal::float4 _e29 = result_1;
            metal::float4 _e32 = star_texel(s_3, metal::float2(f_1.x, fy), row, star_atlas);
            result_1 = _e29 + _e32;
            metal::float4 _e34 = result_1;
            metal::float4 _e41 = star_texel(s_3, metal::float2(f_1.x - 1.0, fy), as_type<int>(as_type<uint>(row) + as_type<uint>(1)), star_atlas);
            result_1 = _e34 + _e41;
        }
    }
    metal::float4 _e46 = result_1;
    return _e46;
}

float star_cover(
    constant Cloud& cloud
) {
    float _e2 = cloud.star_cover;
    return _e2;
}

metal::float4 star_floor(
    metal::texture2d<float, metal::access::sample> lut
) {
    metal::float3 _e1 = palette_color(0.0, lut);
    return metal::float4(_e1, 1.0);
}

metal::float4 star_layers(
    metal::float2 pt,
    constant Cloud& cloud,
    metal::texture2d<float, metal::access::sample> lut,
    metal::texture2d<uint, metal::access::sample> star_atlas
) {
    metal::float4 stars = metal::float4(0.0);
    uint k_1 = 0u;
    metal::float4 slice = {};
    metal::float2 _e4 = star_size(cloud);
    metal::float2 _e9 = star_size(cloud);
    metal::float2 sp = (pt - (_e4 * 0.5)) * (STAR_PANE / _e9.y);
    uint2 loop_bound_1 = uint2(4294967295u);
    bool loop_init_1 = true;
    while(true) {
        if (metal::all(loop_bound_1 == uint2(0u))) { break; }
        loop_bound_1 -= uint2(loop_bound_1.y == 0u, 1u);
        if (!loop_init_1) {
            uint _e126 = k_1;
            k_1 = _e126 + 1u;
        }
        loop_init_1 = false;
        uint _e15 = k_1;
        if (_e15 < STAR_SLICES) {
        } else {
            break;
        }
        {
            uint _e18 = k_1;
            StarSlice _e19 = star_slice(_e18, cloud);
            metal::float2 r_1 = (sp / metal::float2(_e19.cell)) - metal::fract(_e19.offset);
            metal::float2 o_1 = metal::floor(r_1);
            metal::float2 f_3 = r_1 - o_1;
            metal::int2 local_1 = as_type<metal::int2>(as_type<metal::uint2>(as_type<metal::int2>(as_type<metal::uint2>(naga_f2i32(o_1)) - as_type<metal::uint2>(naga_f2i32(metal::floor(_e19.offset))))) - as_type<metal::uint2>(_e19.origin));
            int index_4 = as_type<int>(as_type<uint>(as_type<int>(as_type<uint>(_e19.base) + as_type<uint>(as_type<int>(as_type<uint>(local_1.y) * as_type<uint>(_e19.grid.x))))) + as_type<uint>(local_1.x));
            slice = metal::float4(0.0);
            if (_e19.gather == 1u) {
                metal::float4 _e49 = star_texel(_e19, f_3, index_4, star_atlas);
                slice = _e49;
            } else {
                if (_e19.gather == 2u) {
                    metal::float4 _e53 = star_gather2_(_e19, r_1, star_atlas);
                    slice = _e53;
                } else {
                    if (_e19.gather == 3u) {
                        metal::float4 _e57 = star_gather3_(_e19, f_3, index_4, star_atlas);
                        slice = _e57;
                    }
                }
            }
            float _e59 = slice.w;
            if (_e59 > 0.0) {
                float _e63 = slice.w;
                float cover_1 = metal::min(_e63, 1.0);
                metal::float4 _e66 = slice;
                float _e69 = slice.w;
                metal::float3 colour_1 = _e66.xyz / metal::float3(_e69);
                metal::float3 luminance = metal::float3(0.2126, 0.7152, 0.0722);
                metal::float4 _e76 = stars;
                metal::float4 _e78 = stars;
                float behind = metal::dot(_e76.xyz * _e78.xyz, luminance);
                float _e85 = stars.w;
                float _e88 = stars.w;
                float own = (metal::dot(colour_1 * colour_1, luminance) * _e85) * _e88;
                float ratio = (behind > 0.0) ? metal::saturate(own / behind) : 1.0;
                float _e97 = star_cover(cloud);
                float hold = cover_1 * metal::mix(ratio * ratio, 1.0, _e97);
                metal::float4 _e101 = stars;
                float _e107 = stars.w;
                float _e112 = stars.w;
                float _e118 = stars.w;
                float _e120 = stars.w;
                stars = metal::float4((_e101.xyz * (1.0 - hold)) + (colour_1 * ((cover_1 * (1.0 - _e107)) + (hold * _e112))), _e118 + ((1.0 - _e120) * cover_1));
            }
        }
    }
    metal::float4 _e129 = star_floor(lut);
    float _e132 = stars.w;
    metal::float4 _e136 = stars;
    float _e144 = stars.w;
    return metal::float4((_e129.xyz * (1.0 - _e132)) + _e136.xyz, _e129.w + ((1.0 - _e129.w) * _e144));
}

metal::float2 star_image_size(
    constant Cloud& cloud
) {
    metal::float4 _e2 = cloud.star_image_size;
    return _e2.xy;
}

struct fs_starsInput {
    metal::float2 fraction [[user(loc0), center_perspective]];
};
struct fs_starsOutput {
    metal::float4 member [[color(0)]];
};
fragment fs_starsOutput fs_stars(
  fs_starsInput varyings [[stage_in]]
, metal::float4 position [[position]]
, constant Cloud& cloud [[buffer(2)]]
, metal::texture2d<float, metal::access::sample> lut [[texture(0)]]
, metal::texture2d<uint, metal::access::sample> star_atlas [[texture(5)]]
) {
    const TileVertex in = { position, varyings.fraction };
    metal::float2 _e3 = star_image_size(cloud);
    metal::float2 _e5 = star_size(cloud);
    metal::float4 _e7 = star_layers((in.position.xy / _e3) * _e5, cloud, lut, star_atlas);
    return fs_starsOutput { _e7 };
}
