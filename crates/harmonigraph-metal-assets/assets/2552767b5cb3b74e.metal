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
struct type_9 {
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
    type_9 star_slices;
    uint memory_enabled;
    uint memory_valid;
    float pickup_alpha;
    float release_alpha;
    metal::int2 memory_shift;
    metal::float2 memory_fraction;
    float previous_life;
    float memory_pad_a;
    metal::float2 memory_extent;
    type_9 previous_slices;
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
constant float WASH_FBM_FINE = 2.07;
constant float WASH_FBM_FINE_TILED = 2.0;
constant float DOME_RADIUS = 1.15;
constant float DOME_JITTER = 0.3;
constant float DOME_RADIUS_MIN = 0.95;
constant float DOME_RADIUS_MAX = 1.32;
constant float DOME_UNION = 9.0;
constant float DOME_VARIETY_GAIN = 5.0;
constant float DOME_FACE = 1.5122874;
constant float DOME_LACUNARITY = 2.1;
constant float DOME_FINE_GAIN = 0.22;
constant float CLOUD_UNITS = 10.0;
constant float SCALE_CELLS = 2.7272727;
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

metal::float4 star_texel(
    StarSlice s,
    metal::float2 f,
    int index,
    float cut,
    metal::texture2d<uint, metal::access::sample> star_atlas
) {
    float cover = {};
    uint clamped_lod_e11 = metal::min(uint(0), star_atlas.get_num_mip_levels() - 1);
    metal::uint4 t = star_atlas.read(metal::min(metal::uint2(metal::int2(index & 2047, index >> STAR_ATLAS_SHIFT)), metal::uint2(star_atlas.get_width(clamped_lod_e11), star_atlas.get_height(clamped_lod_e11)) - 1), clamped_lod_e11);
    if (t.w == 0u) {
        return metal::float4(0.0);
    }
    float dist = metal::length(f - metal::float2(as_type<float>(t.x), as_type<float>(t.y))) * s.cell;
    if (dist >= cut) {
        return metal::float4(0.0);
    }
    metal::float3 colour = static_cast<metal::float3>(metal::uint3(t.z >> 20u, t.z >> 10u, t.z) & metal::uint3(1023u)) / metal::float3(1023.0);
    metal::float2 shape = float2(as_type<half2>(t.w));
    float inverse_sigma = shape.x;
    cover = metal::exp((-0.5 * (dist * inverse_sigma)) * (dist * inverse_sigma));
    if (s.fringe > 0.0) {
        float _e57 = cover;
        cover = _e57 + (s.fringe * metal::exp((-0.4 * dist) * inverse_sigma));
    }
    float reach = STAR_REACH * s.cell;
    float _e68 = cover;
    cover = metal::min(_e68, 1.0) * (1.0 - metal::smoothstep(STAR_RING_FADE * reach, reach, dist));
    float _e77 = cover;
    cover = _e77 * shape.y;
    float _e80 = cover;
    float _e82 = cover;
    return metal::float4(colour * _e80, _e82);
}

metal::int2 naga_f2i32(metal::float2 value) {
    return static_cast<metal::int2>(metal::clamp(value, -2147483600.0, 2147483500.0));
}

metal::float3 star_layers(
    metal::float2 pt,
    uint first,
    uint last,
    metal::float3 under,
    constant Cloud& cloud,
    metal::texture2d<uint, metal::access::sample> star_atlas
) {
    metal::float3 out = {};
    uint k = {};
    float cut_1 = {};
    int index_1 = {};
    metal::float4 slice = {};
    out = under;
    metal::float2 _e7 = cloud.size;
    float _e15 = cloud.size.y;
    metal::float2 sp = (pt - (_e7 * 0.5)) * (STAR_PANE / _e15);
    k = first;
    uint2 loop_bound = uint2(4294967295u);
    bool loop_init = true;
    while(true) {
        if (metal::all(loop_bound == uint2(0u))) { break; }
        loop_bound -= uint2(loop_bound.y == 0u, 1u);
        if (!loop_init) {
            uint _e182 = k;
            k = _e182 + 1u;
        }
        loop_init = false;
        uint _e19 = k;
        if (_e19 < last) {
        } else {
            break;
        }
        {
            uint _e23 = k;
            StarSlice s_1 = cloud.star_slices.inner[metal::min(unsigned(_e23), 4u)];
            cut_1 = STAR_REACH * s_1.cell;
            if (s_1.fringe <= 0.0) {
                float _e33 = cut_1;
                cut_1 = metal::min(_e33, (5.0 * s_1.cap) * s_1.defocus);
            }
            metal::float2 r = (sp / metal::float2(s_1.cell)) - s_1.offset;
            metal::float2 o = metal::floor(r);
            metal::float2 f_1 = r - o;
            metal::int2 local = as_type<metal::int2>(as_type<metal::uint2>(as_type<metal::int2>(as_type<metal::uint2>(naga_f2i32(o)) - as_type<metal::uint2>(metal::int2(1)))) - as_type<metal::uint2>(s_1.origin));
            index_1 = as_type<int>(as_type<uint>(as_type<int>(as_type<uint>(s_1.base) + as_type<uint>(as_type<int>(as_type<uint>(local.y) * as_type<uint>(s_1.grid.x))))) + as_type<uint>(local.x));
            slice = metal::float4(0.0);
            {
                metal::float2 g = f_1 - metal::float2(0.0, -1.0);
                metal::float4 _e69 = slice;
                int _e74 = index_1;
                float _e75 = cut_1;
                metal::float4 _e76 = star_texel(s_1, g + metal::float2(1.0, 0.0), _e74, _e75, star_atlas);
                slice = _e69 + _e76;
                metal::float4 _e78 = slice;
                int _e79 = index_1;
                float _e82 = cut_1;
                metal::float4 _e83 = star_texel(s_1, g, as_type<int>(as_type<uint>(_e79) + as_type<uint>(1)), _e82, star_atlas);
                slice = _e78 + _e83;
                metal::float4 _e85 = slice;
                int _e90 = index_1;
                float _e93 = cut_1;
                metal::float4 _e94 = star_texel(s_1, g - metal::float2(1.0, 0.0), as_type<int>(as_type<uint>(_e90) + as_type<uint>(2)), _e93, star_atlas);
                slice = _e85 + _e94;
                int _e96 = index_1;
                index_1 = as_type<int>(as_type<uint>(_e96) + as_type<uint>(s_1.grid.x));
            }
            {
                metal::float2 g_1 = f_1 - metal::float2(0.0, 0.0);
                metal::float4 _e104 = slice;
                int _e109 = index_1;
                float _e110 = cut_1;
                metal::float4 _e111 = star_texel(s_1, g_1 + metal::float2(1.0, 0.0), _e109, _e110, star_atlas);
                slice = _e104 + _e111;
                metal::float4 _e113 = slice;
                int _e114 = index_1;
                float _e117 = cut_1;
                metal::float4 _e118 = star_texel(s_1, g_1, as_type<int>(as_type<uint>(_e114) + as_type<uint>(1)), _e117, star_atlas);
                slice = _e113 + _e118;
                metal::float4 _e120 = slice;
                int _e125 = index_1;
                float _e128 = cut_1;
                metal::float4 _e129 = star_texel(s_1, g_1 - metal::float2(1.0, 0.0), as_type<int>(as_type<uint>(_e125) + as_type<uint>(2)), _e128, star_atlas);
                slice = _e120 + _e129;
                int _e131 = index_1;
                index_1 = as_type<int>(as_type<uint>(_e131) + as_type<uint>(s_1.grid.x));
            }
            {
                metal::float2 g_2 = f_1 - metal::float2(0.0, 1.0);
                metal::float4 _e139 = slice;
                int _e144 = index_1;
                float _e145 = cut_1;
                metal::float4 _e146 = star_texel(s_1, g_2 + metal::float2(1.0, 0.0), _e144, _e145, star_atlas);
                slice = _e139 + _e146;
                metal::float4 _e148 = slice;
                int _e149 = index_1;
                float _e152 = cut_1;
                metal::float4 _e153 = star_texel(s_1, g_2, as_type<int>(as_type<uint>(_e149) + as_type<uint>(1)), _e152, star_atlas);
                slice = _e148 + _e153;
                metal::float4 _e155 = slice;
                int _e160 = index_1;
                float _e163 = cut_1;
                metal::float4 _e164 = star_texel(s_1, g_2 - metal::float2(1.0, 0.0), as_type<int>(as_type<uint>(_e160) + as_type<uint>(2)), _e163, star_atlas);
                slice = _e155 + _e164;
            }
            float _e167 = slice.w;
            if (_e167 > 0.0) {
                metal::float3 _e170 = out;
                metal::float4 _e171 = slice;
                float _e174 = slice.w;
                float _e178 = slice.w;
                out = metal::mix(_e170, _e171.xyz / metal::float3(_e174), metal::min(_e178, 1.0));
            }
        }
    }
    metal::float3 _e185 = out;
    return _e185;
}

struct fs_star_farInput {
    metal::float2 fraction [[user(loc0), center_perspective]];
};
struct fs_star_farOutput {
    metal::float4 member [[color(0)]];
};
fragment fs_star_farOutput fs_star_far(
  fs_star_farInput varyings [[stage_in]]
, metal::float4 position [[position]]
, metal::texture2d<float, metal::access::sample> lut [[texture(0)]]
, constant Cloud& cloud [[buffer(2)]]
, metal::texture2d<uint, metal::access::sample> star_atlas [[texture(6)]]
) {
    const TileVertex in = { position, varyings.fraction };
    metal::float2 _e5 = cloud.origin;
    float _e8 = cloud.ppp;
    metal::float2 position_1 = in.position.xy + metal::rint(_e5 * _e8);
    float _e14 = cloud.ppp;
    metal::float2 _e19 = cloud.origin;
    metal::float2 pt_1 = (position_1 / metal::float2(_e14)) - _e19;
    metal::float3 _e24 = palette_color(0.0, lut);
    metal::float3 _e25 = star_layers(pt_1, 0u, 2u, _e24, cloud, star_atlas);
    return fs_star_farOutput { metal::float4(_e25, 1.0) };
}
