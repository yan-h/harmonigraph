// language: metal3.2
#include <metal_stdlib>
#include <simd/simd.h>

using metal::uint;

struct Wet {
    metal::float2 offset;
};
struct WashField {
    Wet coarse;
    Wet fine;
    float cover;
    char _pad3[4];
};
struct Pile {
    metal::float2 face;
    metal::float2 to_centre;
};
struct VertexOut {
    metal::float4 position;
    float slab;
    float t;
    char _pad3[8];
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
    uint memory_enabled;
    uint memory_valid;
    float pickup_alpha;
    float release_alpha;
    metal::int2 memory_shift;
    metal::float2 memory_fraction;
    float previous_life;
    float memory_pad_a;
    metal::float2 memory_extent;
    type_10 previous_slices;
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
constant bool STAR_SPLIT = true;

metal::float2 rotate_watercolor_tile_vector_for(
    metal::float2 v,
    uint pitch_vertical
) {
    metal::float2 semantic = (pitch_vertical == 1u) ? v : metal::float2(v.y, v.x);
    metal::float2 turned = metal::float2((CLOUD_TILE_ROT_COS * semantic.x) - (CLOUD_TILE_ROT_SIN * semantic.y), (CLOUD_TILE_ROT_SIN * semantic.x) + (CLOUD_TILE_ROT_COS * semantic.y));
    return (pitch_vertical == 1u) ? turned : metal::float2(turned.y, turned.x);
}

metal::float2 watercolor_tile_uv_for(
    metal::float2 r,
    float period,
    uint pitch_vertical_1
) {
    metal::float2 semantic_1 = (pitch_vertical_1 == 1u) ? r : metal::float2(r.y, r.x);
    return metal::float2((CLOUD_TILE_ROT_COS * semantic_1.x) + (CLOUD_TILE_ROT_SIN * semantic_1.y), (-0.6 * semantic_1.x) + (CLOUD_TILE_ROT_COS * semantic_1.y)) / metal::float2(period);
}

metal::float3 linear_from_gamma_rgb(
    metal::float3 srgb
) {
    metal::bool3 cutoff = srgb < metal::float3(0.04045);
    metal::float3 lower = srgb / metal::float3(12.92);
    metal::float3 higher = metal::pow((srgb + metal::float3(0.055)) / metal::float3(1.055), metal::float3(2.4));
    return metal::select(higher, lower, cutoff);
}

float baked_density(
    metal::float2 position,
    metal::texture2d<float, metal::access::sample> close_light,
    metal::sampler cloud_sampler,
    constant Cloud& cloud
) {
    float _e3 = cloud.ppp;
    metal::float2 _e8 = cloud.origin;
    metal::float2 _e12 = cloud.size;
    metal::float2 uv_1 = ((position / metal::float2(_e3)) - _e8) / _e12;
    metal::float4 _e17 = close_light.sample(cloud_sampler, uv_1, metal::level(0.0));
    return _e17.x;
}

bool softened(
    constant Cloud& cloud
) {
    metal::float2 _e2 = cloud.step;
    return metal::any(_e2 != metal::float2(0.0));
}

float style_level(
    float level,
    constant Cloud& cloud
) {
    float _e3 = cloud.contour_strength;
    if (_e3 <= 0.0) {
        return level;
    }
    float _e11 = cloud.contours;
    float x = metal::clamp(level, 0.0, 1.0) * _e11;
    float _e15 = cloud.contour_softness;
    float _e16 = metal::fwidth(x);
    float edge = metal::min(0.5, metal::max(_e15, _e16 * 0.5));
    float _e32 = cloud.contours;
    float terraces = (metal::floor(x) + metal::smoothstep(0.5 - edge, 0.5 + edge, metal::fract(x))) / _e32;
    float _e36 = cloud.contour_strength;
    float _e43 = metal::fwidth(x);
    float strength = ((0.9 * _e36) * metal::smoothstep(0.0, 1.0, x)) * (1.0 - metal::smoothstep(0.5, 1.5, _e43));
    return metal::mix(level, terraces, strength);
}

uint naga_f2u32(float value) {
    return static_cast<uint>(metal::clamp(value, 0.0, 4294967000.0));
}

metal::float3 palette_color(
    float level_1,
    metal::texture2d<float, metal::access::sample> lut
) {
    uint levels = metal::uint2(lut.get_width(), lut.get_height()).x;
    float x_1 = metal::max((metal::clamp(level_1, 0.0, 1.0) * static_cast<float>(levels)) - 0.5, 0.0);
    uint i = naga_f2u32(metal::clamp(metal::floor(x_1), 0.0, static_cast<float>(levels - 1u)));
    uint clamped_lod_e24 = metal::min(uint(0), lut.get_num_mip_levels() - 1);
    metal::float4 _e24 = lut.read(metal::min(metal::uint2(metal::uint2(i, 0u)), metal::uint2(lut.get_width(clamped_lod_e24), lut.get_height(clamped_lod_e24)) - 1), clamped_lod_e24);
    metal::float3 a = _e24.xyz;
    uint clamped_lod_e35 = metal::min(uint(0), lut.get_num_mip_levels() - 1);
    metal::float4 _e35 = lut.read(metal::min(metal::uint2(metal::uint2(metal::min(i + 1u, levels - 1u), 0u)), metal::uint2(lut.get_width(clamped_lod_e35), lut.get_height(clamped_lod_e35)) - 1), clamped_lod_e35);
    metal::float3 b = _e35.xyz;
    return metal::mix(a, b, metal::fract(x_1));
}

metal::float4 density_color(
    float raw_level,
    metal::texture2d<float, metal::access::sample> lut,
    constant Cloud& cloud
) {
    float _e1 = style_level(raw_level, cloud);
    metal::float3 _e2 = palette_color(_e1, lut);
    return metal::float4(_e2, 1.0);
}

metal::float2 watercolor_tile_uv(
    metal::float2 r_1,
    constant Cloud& cloud
) {
    uint _e3 = cloud.tile_cells;
    uint _e7 = cloud.pitch_vertical;
    metal::float2 _e8 = watercolor_tile_uv_for(r_1, static_cast<float>(_e3), _e7);
    return _e8;
}

metal::float2 rotate_watercolor_tile_vector(
    metal::float2 v_1,
    constant Cloud& cloud
) {
    uint _e3 = cloud.pitch_vertical;
    metal::float2 _e4 = rotate_watercolor_tile_vector_for(v_1, _e3);
    return _e4;
}

float cloud_light(
    metal::float2 pt,
    metal::texture2d<float, metal::access::sample> close_light,
    metal::sampler cloud_sampler,
    constant Cloud& cloud
) {
    metal::float2 _e5 = cloud.size;
    metal::float4 _e8 = close_light.sample(cloud_sampler, pt / _e5, metal::level(0.0));
    return _e8.x;
}

float scale_tone(
    metal::float2 pt_1,
    metal::texture2d<float, metal::access::sample> close_light,
    metal::sampler cloud_sampler,
    constant Cloud& cloud,
    metal::texture2d<float, metal::access::sample> cloud_tile_a,
    metal::sampler tile_sampler
) {
    Pile pile = {};
    metal::float2 _e3 = cloud.size;
    float _e10 = cloud.size.y;
    metal::float2 _e17 = cloud.drift;
    metal::float2 q = (((pt_1 - (_e3 * 0.5)) / metal::float2(_e10)) * CLOUD_UNITS) + _e17;
    float _e22 = cloud.scale_size;
    float scale_units = SCALE_CELLS / _e22;
    float _e27 = cloud.size.y;
    float scale_points = (_e27 / CLOUD_UNITS) / scale_units;
    metal::float2 r_3 = q * scale_units;
    uint _e36 = cloud.tile_cells;
    metal::float4 tile = cloud_tile_a.sample(tile_sampler, r_3 / metal::float2(static_cast<float>(_e36)), metal::level(0.0));
    pile.face = tile.xy;
    pile.to_centre = tile.zw;
    metal::float2 face = pile.face;
    float _e51 = cloud.scale_refract;
    float bend = metal::max(_e51, 0.0) * scale_points;
    float _e57 = cloud.scale_refract;
    float gather = metal::max(-(_e57), 0.0) * scale_points;
    metal::float2 _e65 = pile.to_centre;
    metal::float2 lookup = (-(face) * bend) + (_e65 * gather);
    float _e69 = cloud_light(pt_1 + lookup, close_light, cloud_sampler, cloud);
    return _e69;
}

float wash_level(
    Wet wet,
    float pane_per_cell,
    metal::float2 pt_2,
    metal::texture2d<float, metal::access::sample> close_light,
    metal::sampler cloud_sampler,
    constant Cloud& cloud
) {
    float _e7 = cloud.wash_refract;
    float _e10 = cloud_light(pt_2 + ((wet.offset * pane_per_cell) * _e7), close_light, cloud_sampler, cloud);
    return _e10;
}

WashField wash_tile_field(
    metal::float2 r_2,
    constant Cloud& cloud,
    metal::texture2d<float, metal::access::sample> cloud_tile_a,
    metal::texture2d<float, metal::access::sample> cloud_tile_b,
    metal::sampler tile_sampler
) {
    WashField out = {};
    metal::float2 _e1 = watercolor_tile_uv(r_2, cloud);
    metal::float4 a_1 = cloud_tile_a.sample(tile_sampler, _e1, metal::level(0.0));
    metal::float2 _e9 = rotate_watercolor_tile_vector(a_1.xy, cloud);
    out.coarse = Wet {_e9};
    out.fine = Wet {metal::float2(0.0)};
    out.cover = 0.0;
    float _e19 = cloud.wash_layers;
    if (_e19 > 0.0) {
        metal::float4 b_1 = cloud_tile_b.sample(tile_sampler, _e1, metal::level(0.0));
        metal::float2 _e28 = rotate_watercolor_tile_vector(b_1.xy, cloud);
        out.fine = Wet {_e28};
        out.cover = b_1.w;
    }
    WashField _e32 = out;
    return _e32;
}

float wash_cloud_tone(
    metal::float2 pt_3,
    metal::texture2d<float, metal::access::sample> close_light,
    metal::sampler cloud_sampler,
    constant Cloud& cloud,
    metal::texture2d<float, metal::access::sample> cloud_tile_a,
    metal::texture2d<float, metal::access::sample> cloud_tile_b,
    metal::sampler tile_sampler
) {
    float level_2 = {};
    metal::float2 _e3 = cloud.size;
    float _e10 = cloud.size.y;
    metal::float2 _e17 = cloud.drift;
    metal::float2 q_1 = (((pt_3 - (_e3 * 0.5)) / metal::float2(_e10)) * CLOUD_UNITS) + _e17;
    float _e22 = cloud.wash_size;
    float cells = WASH_CELLS / _e22;
    float _e27 = cloud.size.y;
    float pane_per_cell_1 = (_e27 / CLOUD_UNITS) / cells;
    metal::float2 r_4 = q_1 * cells;
    WashField _e32 = wash_tile_field(r_4, cloud, cloud_tile_a, cloud_tile_b, tile_sampler);
    float _e34 = wash_level(_e32.coarse, pane_per_cell_1, pt_3, close_light, cloud_sampler, cloud);
    level_2 = _e34;
    float _e38 = cloud.wash_layers;
    if (_e38 > 0.0) {
        float _e44 = wash_level(_e32.fine, pane_per_cell_1 / WASH_LACUNARITY, pt_3, close_light, cloud_sampler, cloud);
        float _e47 = cloud.wash_layers;
        float over = _e47 * _e32.cover;
        float _e50 = level_2;
        level_2 = metal::mix(_e50, _e44, over);
    }
    float _e52 = level_2;
    return _e52;
}

float cloud_tone_at(
    metal::float2 pt_4,
    metal::texture2d<float, metal::access::sample> close_light,
    metal::sampler cloud_sampler,
    constant Cloud& cloud,
    metal::texture2d<float, metal::access::sample> cloud_tile_a,
    metal::texture2d<float, metal::access::sample> cloud_tile_b,
    metal::sampler tile_sampler
) {
    uint _e3 = cloud.cloud_style;
    if (_e3 == 1u) {
        float _e6 = wash_cloud_tone(pt_4, close_light, cloud_sampler, cloud, cloud_tile_a, cloud_tile_b, tile_sampler);
        return _e6;
    }
    float _e7 = scale_tone(pt_4, close_light, cloud_sampler, cloud, cloud_tile_a, tile_sampler);
    return _e7;
}

metal::float3 gamma_from_linear_rgb(
    metal::float3 linear
) {
    metal::float3 bounded = metal::clamp(linear, metal::float3(0.0), metal::float3(1.0));
    return metal::select((1.055 * metal::pow(bounded, metal::float3(0.41666666))) - metal::float3(0.055), 12.92 * bounded, bounded <= metal::float3(0.0031308));
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
    metal::float2 pt_5,
    uint first,
    uint last,
    metal::float3 under,
    constant Cloud& cloud,
    metal::texture2d<uint, metal::access::sample> star_atlas
) {
    metal::float3 out_1 = {};
    uint k = {};
    float cut_1 = {};
    int index_1 = {};
    metal::float4 slice = {};
    out_1 = under;
    metal::float2 _e7 = cloud.size;
    float _e15 = cloud.size.y;
    metal::float2 sp = (pt_5 - (_e7 * 0.5)) * (STAR_PANE / _e15);
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
            metal::float2 r_5 = (sp / metal::float2(s_1.cell)) - s_1.offset;
            metal::float2 o = metal::floor(r_5);
            metal::float2 f_1 = r_5 - o;
            metal::int2 local_2 = as_type<metal::int2>(as_type<metal::uint2>(as_type<metal::int2>(as_type<metal::uint2>(naga_f2i32(o)) - as_type<metal::uint2>(metal::int2(1)))) - as_type<metal::uint2>(s_1.origin));
            index_1 = as_type<int>(as_type<uint>(as_type<int>(as_type<uint>(s_1.base) + as_type<uint>(as_type<int>(as_type<uint>(local_2.y) * as_type<uint>(s_1.grid.x))))) + as_type<uint>(local_2.x));
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
                metal::float3 _e170 = out_1;
                metal::float4 _e171 = slice;
                float _e174 = slice.w;
                float _e178 = slice.w;
                out_1 = metal::mix(_e170, _e171.xyz / metal::float3(_e174), metal::min(_e178, 1.0));
            }
        }
    }
    metal::float3 _e185 = out_1;
    return _e185;
}

metal::float3 star_color(
    metal::float2 pt_6,
    metal::texture2d<float, metal::access::sample> lut,
    constant Cloud& cloud,
    metal::texture2d<float, metal::access::sample> cloud_tone,
    metal::texture2d<uint, metal::access::sample> star_atlas
) {
    if (STAR_SPLIT) {
        float _e5 = cloud.ppp;
        uint clamped_lod_e9 = metal::min(uint(0), cloud_tone.get_num_mip_levels() - 1);
        metal::float4 _e9 = cloud_tone.read(metal::min(metal::uint2(naga_f2i32(pt_6 * _e5)), metal::uint2(cloud_tone.get_width(clamped_lod_e9), cloud_tone.get_height(clamped_lod_e9)) - 1), clamped_lod_e9);
        metal::float3 far = _e9.xyz;
        metal::float3 _e13 = star_layers(pt_6, 2u, STAR_SLICES, far, cloud, star_atlas);
        return _e13;
    }
    metal::float3 _e17 = palette_color(0.0, lut);
    metal::float3 _e18 = star_layers(pt_6, 0u, STAR_SLICES, _e17, cloud, star_atlas);
    return _e18;
}

metal::float3 memory_color(
    metal::float2 uv,
    metal::texture2d<float, metal::access::sample> color_memory,
    constant Cloud& cloud
) {
    metal::float2 _e3 = cloud.memory_extent;
    metal::int2 size = naga_f2i32(_e3);
    metal::float2 p = (uv * static_cast<metal::float2>(size)) - metal::float2(0.5);
    metal::int2 lo = naga_f2i32(metal::floor(p));
    metal::float2 f_2 = metal::fract(p);
    uint clamped_lod_e21 = metal::min(uint(0), color_memory.get_num_mip_levels() - 1);
    metal::float4 _e21 = color_memory.read(metal::min(metal::uint2(metal::clamp(lo, metal::int2(0), as_type<metal::int2>(as_type<metal::uint2>(size) - as_type<metal::uint2>(metal::int2(1))))), metal::uint2(color_memory.get_width(clamped_lod_e21), color_memory.get_height(clamped_lod_e21)) - 1), clamped_lod_e21);
    metal::float3 a_2 = _e21.xyz;
    uint clamped_lod_e35 = metal::min(uint(0), color_memory.get_num_mip_levels() - 1);
    metal::float4 _e35 = color_memory.read(metal::min(metal::uint2(metal::clamp(as_type<metal::int2>(as_type<metal::uint2>(lo) + as_type<metal::uint2>(metal::int2(1, 0))), metal::int2(0), as_type<metal::int2>(as_type<metal::uint2>(size) - as_type<metal::uint2>(metal::int2(1))))), metal::uint2(color_memory.get_width(clamped_lod_e35), color_memory.get_height(clamped_lod_e35)) - 1), clamped_lod_e35);
    metal::float3 b_2 = _e35.xyz;
    uint clamped_lod_e49 = metal::min(uint(0), color_memory.get_num_mip_levels() - 1);
    metal::float4 _e49 = color_memory.read(metal::min(metal::uint2(metal::clamp(as_type<metal::int2>(as_type<metal::uint2>(lo) + as_type<metal::uint2>(metal::int2(0, 1))), metal::int2(0), as_type<metal::int2>(as_type<metal::uint2>(size) - as_type<metal::uint2>(metal::int2(1))))), metal::uint2(color_memory.get_width(clamped_lod_e49), color_memory.get_height(clamped_lod_e49)) - 1), clamped_lod_e49);
    metal::float3 c = _e49.xyz;
    uint clamped_lod_e63 = metal::min(uint(0), color_memory.get_num_mip_levels() - 1);
    metal::float4 _e63 = color_memory.read(metal::min(metal::uint2(metal::clamp(as_type<metal::int2>(as_type<metal::uint2>(lo) + as_type<metal::uint2>(metal::int2(1, 1))), metal::int2(0), as_type<metal::int2>(as_type<metal::uint2>(size) - as_type<metal::uint2>(metal::int2(1))))), metal::uint2(color_memory.get_width(clamped_lod_e63), color_memory.get_height(clamped_lod_e63)) - 1), clamped_lod_e63);
    metal::float3 d = _e63.xyz;
    return metal::mix(metal::mix(a_2, b_2, f_2.x), metal::mix(c, d, f_2.x), f_2.y);
}

metal::float4 clouded(
    float level_3,
    metal::float2 position_1,
    metal::texture2d<float, metal::access::sample> lut,
    metal::texture2d<float, metal::access::sample> color_memory,
    metal::texture2d<float, metal::access::sample> close_light,
    metal::sampler cloud_sampler,
    constant Cloud& cloud,
    metal::texture2d<float, metal::access::sample> cloud_tone,
    metal::texture2d<float, metal::access::sample> cloud_tile_a,
    metal::texture2d<float, metal::access::sample> cloud_tile_b,
    metal::sampler tile_sampler,
    metal::texture2d<uint, metal::access::sample> star_atlas
) {
    float tone = {};
    float _e4 = cloud.cloud_depth;
    if (_e4 <= 0.0) {
        metal::float4 _e7 = density_color(level_3, lut, cloud);
        return _e7;
    }
    float _e10 = cloud.ppp;
    metal::float2 _e15 = cloud.origin;
    metal::float2 pt_7 = (position_1 / metal::float2(_e10)) - _e15;
    uint _e19 = cloud.cloud_style;
    if (_e19 == 2u) {
        metal::float4 _e22 = density_color(level_3, lut, cloud);
        metal::float3 _e24 = star_color(pt_7, lut, cloud, cloud_tone, star_atlas);
        float _e27 = cloud.cloud_depth;
        return metal::float4(metal::mix(_e22.xyz, _e24, _e27), 1.0);
    }
    uint _e33 = cloud.memory_enabled;
    if (_e33 != 0u) {
        metal::float2 dimensions = cloud.memory_extent;
        metal::float2 _e41 = cloud.size;
        metal::float2 _e52 = cloud.memory_fraction;
        metal::float2 uv_2 = ((((pt_7 / _e41) * (dimensions - metal::float2(2.0))) + metal::float2(1.0)) + _e52) / dimensions;
        metal::float3 _e55 = memory_color(uv_2, color_memory, cloud);
        float _e58 = cloud.cloud_depth;
        if (_e58 >= 1.0) {
            metal::float3 _e61 = gamma_from_linear_rgb(_e55);
            return metal::float4(_e61, 1.0);
        }
        metal::float4 _e64 = density_color(level_3, lut, cloud);
        metal::float3 _e66 = linear_from_gamma_rgb(_e64.xyz);
        float _e69 = cloud.cloud_depth;
        metal::float3 _e71 = gamma_from_linear_rgb(metal::mix(_e66, _e55, _e69));
        return metal::float4(_e71, 1.0);
    }
    uint _e77 = cloud.tone_baked;
    if (_e77 == 1u) {
        metal::float2 _e84 = cloud.size;
        metal::float4 _e87 = cloud_tone.sample(cloud_sampler, pt_7 / _e84, metal::level(0.0));
        tone = _e87.x;
    } else {
        float _e89 = cloud_tone_at(pt_7, close_light, cloud_sampler, cloud, cloud_tile_a, cloud_tile_b, tile_sampler);
        tone = _e89;
    }
    float _e90 = tone;
    float _e93 = cloud.cloud_depth;
    metal::float4 _e95 = density_color(metal::mix(level_3, _e90, _e93), lut, cloud);
    return _e95;
}

bool full_material_memory(
    constant Cloud& cloud
) {
    bool local = {};
    bool local_1 = {};
    uint _e2 = cloud.memory_enabled;
    if (_e2 != 0u) {
        float _e9 = cloud.cloud_depth;
        local = _e9 >= 1.0;
    } else {
        local = false;
    }
    bool _e13 = local;
    if (_e13) {
        uint _e18 = cloud.cloud_style;
        local_1 = _e18 != 2u;
    } else {
        local_1 = false;
    }
    bool _e22 = local_1;
    return _e22;
}

metal::float4 backdrop_color(
    metal::float2 position_2,
    metal::texture2d<float, metal::access::sample> lut,
    metal::texture2d<float, metal::access::sample> color_memory,
    metal::texture2d<float, metal::access::sample> close_light,
    metal::sampler cloud_sampler,
    constant Cloud& cloud,
    metal::texture2d<float, metal::access::sample> cloud_tone,
    metal::texture2d<float, metal::access::sample> cloud_tile_a,
    metal::texture2d<float, metal::access::sample> cloud_tile_b,
    metal::sampler tile_sampler,
    metal::texture2d<uint, metal::access::sample> star_atlas
) {
    float level_4 = 0.0;
    bool _e1 = full_material_memory(cloud);
    if (_e1) {
        metal::float4 _e3 = clouded(0.0, position_2, lut, color_memory, close_light, cloud_sampler, cloud, cloud_tone, cloud_tile_a, cloud_tile_b, tile_sampler, star_atlas);
        return _e3;
    }
    bool _e6 = softened(cloud);
    if (_e6) {
        float _e7 = baked_density(position_2, close_light, cloud_sampler, cloud);
        level_4 = _e7;
    }
    float _e8 = level_4;
    metal::float4 _e9 = clouded(_e8, position_2, lut, color_memory, close_light, cloud_sampler, cloud, cloud_tone, cloud_tile_a, cloud_tile_b, tile_sampler, star_atlas);
    return _e9;
}

struct fs_cloud_backdrop_linearInput {
    float slab [[user(loc0), center_perspective]];
    float t [[user(loc1), center_perspective]];
};
struct fs_cloud_backdrop_linearOutput {
    metal::float4 member [[color(0)]];
};
fragment fs_cloud_backdrop_linearOutput fs_cloud_backdrop_linear(
  fs_cloud_backdrop_linearInput varyings [[stage_in]]
, metal::float4 position_3 [[position]]
, metal::texture2d<float, metal::access::sample> lut [[texture(0)]]
, metal::texture2d<float, metal::access::sample> color_memory [[texture(7)]]
, metal::texture2d<float, metal::access::sample> close_light [[texture(1)]]
, metal::sampler cloud_sampler [[sampler(0)]]
, constant Cloud& cloud [[buffer(2)]]
, metal::texture2d<float, metal::access::sample> cloud_tone [[texture(3)]]
, metal::texture2d<float, metal::access::sample> cloud_tile_a [[texture(4)]]
, metal::texture2d<float, metal::access::sample> cloud_tile_b [[texture(5)]]
, metal::sampler tile_sampler [[sampler(1)]]
, metal::texture2d<uint, metal::access::sample> star_atlas [[texture(6)]]
) {
    const VertexOut in = { position_3, varyings.slab, varyings.t };
    metal::float4 _e3 = backdrop_color(in.position.xy, lut, color_memory, close_light, cloud_sampler, cloud, cloud_tone, cloud_tile_a, cloud_tile_b, tile_sampler, star_atlas);
    metal::float3 _e5 = linear_from_gamma_rgb(_e3.xyz);
    return fs_cloud_backdrop_linearOutput { metal::float4(_e5, 1.0) };
}
