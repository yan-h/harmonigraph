// language: metal3.2
#include <metal_stdlib>
#include <simd/simd.h>

using metal::uint;

struct Wet {
    metal::float2 offset;
    float brightness;
    char _pad2[4];
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
struct type_8 {
    StarSlice inner[5];
};
struct type_9 {
    StarHaloSample inner[5];
};
struct StarUniforms {
    metal::float2 origin;
    metal::float2 size;
    float ppp;
    float star_randomness;
    float star_life;
    float padding;
    metal::float4 star_far;
    metal::float4 star_near;
    metal::float4 star_geometry;
    type_8 star_slices;
    type_9 star_halo_samples;
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
    metal::float4 star_far;
    metal::float4 star_near;
    metal::float4 star_geometry;
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
    type_9 star_halo_samples;
};
struct VertexOut {
    metal::float4 position;
    float slab;
    float t;
    char _pad3[8];
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
constant float CLOUD_UNITS = 10.0;
constant float SCALE_CELLS = 2.7272727;
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

metal::float3 wash_vary_brightness(
    metal::float3 color,
    float ceiling,
    float draw,
    float amount
) {
    float peak = metal::max(color.x, metal::max(color.y, color.z));
    float headroom = metal::clamp((ceiling - peak) / metal::max(peak, 0.000001), 0.0, 1.0);
    return color * (1.0 + ((amount * draw) * headroom));
}

metal::int2 atlas_texel(
    int index
) {
    return metal::int2(index & 2047, index >> STAR_ATLAS_SHIFT);
}

StarUniforms star_settings(
    constant Cloud& cloud
) {
    metal::float2 _e2 = cloud.origin;
    metal::float2 _e5 = cloud.size;
    float _e8 = cloud.ppp;
    float _e11 = cloud.star_randomness;
    float _e14 = cloud.star_life;
    metal::float4 _e17 = cloud.star_far;
    metal::float4 _e20 = cloud.star_near;
    metal::float4 _e23 = cloud.star_geometry;
    type_8 _e26 = cloud.star_slices;
    type_9 _e29 = cloud.star_halo_samples;
    return StarUniforms {_e2, _e5, _e8, _e11, _e14, 0.0, _e17, _e20, _e23, _e26, _e29};
}

metal::float3 gamma_from_linear_rgb(
    metal::float3 linear
) {
    metal::float3 bounded = metal::clamp(linear, metal::float3(0.0), metal::float3(1.0));
    return metal::select((1.055 * metal::pow(bounded, metal::float3(0.41666666))) - metal::float3(0.055), 12.92 * bounded, bounded <= metal::float3(0.0031308));
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

metal::float4 star_texel(
    StarSlice s,
    metal::float2 f,
    int index_1,
    bool halo,
    constant Cloud& cloud,
    metal::texture2d<uint, metal::access::sample> star_atlas
) {
    float cover = {};
    float full = {};
    uint clamped_lod_e11 = metal::min(uint(0), star_atlas.get_num_mip_levels() - 1);
    metal::uint4 t = star_atlas.read(metal::min(metal::uint2(metal::int2(index_1 & 2047, index_1 >> STAR_ATLAS_SHIFT)), metal::uint2(star_atlas.get_width(clamped_lod_e11), star_atlas.get_height(clamped_lod_e11)) - 1), clamped_lod_e11);
    if (t.w == 0u) {
        return metal::float4(0.0);
    }
    float dist = metal::length(f - metal::float2(as_type<float>(t.x), as_type<float>(t.y))) * s.cell;
    StarUniforms _e26 = star_settings(cloud);
    float reach = _e26.star_geometry.y * s.cell;
    if (dist >= (halo ? (STAR_HALO_REACH * s.cell) : reach)) {
        return metal::float4(0.0);
    }
    metal::float3 colour = static_cast<metal::float3>(metal::uint3(t.z >> 20u, t.z >> 10u, t.z) & metal::uint3(1023u)) / metal::float3(1023.0);
    metal::float2 shape = float2(as_type<half2>(t.w));
    float d = dist * shape.x;
    float gaussian = metal::exp((-0.5 * d) * d);
    StarUniforms _e61 = star_settings(cloud);
    float core = gaussian * (1.0 - metal::smoothstep(_e61.star_geometry.z * reach, reach, dist));
    cover = core;
    if (halo) {
        full = gaussian;
        if (s.fringe > 0.0) {
            float _e74 = full;
            full = _e74 + (s.fringe * metal::exp(-0.4 * d));
        }
        float outer = STAR_HALO_REACH * s.cell;
        float _e84 = full;
        full = metal::min(_e84, 1.0) * (1.0 - metal::smoothstep(STAR_HALO_FADE * outer, outer, dist));
        float _e93 = full;
        cover = metal::max(_e93 - core, 0.0);
    }
    float _e97 = cover;
    cover = _e97 * shape.y;
    float _e100 = cover;
    float _e102 = cover;
    return metal::float4(colour * _e100, _e102);
}

metal::float4 star_halo_at(
    metal::float2 pt,
    uint k,
    constant Cloud& cloud,
    metal::sampler cloud_sampler,
    metal::texture2d_array<float, metal::access::sample> star_halos,
    metal::texture2d_array<float, metal::access::sample> star_halos_b,
    metal::texture2d_array<float, metal::access::sample> star_halos_c
) {
    StarUniforms _e2 = star_settings(cloud);
    StarHaloSample sample = _e2.star_halo_samples.inner[metal::min(unsigned(k), 4u)];
    StarUniforms _e5 = star_settings(cloud);
    metal::float2 uv_1 = pt / _e5.size;
    switch(_e2.star_halo_samples.inner[metal::min(unsigned(k), 4u)].group) {
        case 0u: {
            metal::float4 _e14 = star_halos.sample(cloud_sampler, uv_1, static_cast<int>(_e2.star_halo_samples.inner[metal::min(unsigned(k), 4u)].layer), metal::level(0.0));
            return _e14;
        }
        case 1u: {
            metal::float4 _e20 = star_halos_b.sample(cloud_sampler, uv_1, static_cast<int>(_e2.star_halo_samples.inner[metal::min(unsigned(k), 4u)].layer), metal::level(0.0));
            return _e20;
        }
        default: {
            metal::float4 _e26 = star_halos_c.sample(cloud_sampler, uv_1, static_cast<int>(_e2.star_halo_samples.inner[metal::min(unsigned(k), 4u)].layer), metal::level(0.0));
            return _e26;
        }
    }
}

metal::float4 star_far_texel(
    StarSlice s_1,
    metal::float2 f_1,
    int index_2,
    constant Cloud& cloud,
    metal::texture2d<uint, metal::access::sample> star_atlas
) {
    float full_1 = {};
    metal::int2 _e4 = atlas_texel(index_2);
    uint clamped_lod_e6 = metal::min(uint(0), star_atlas.get_num_mip_levels() - 1);
    metal::uint4 t_1 = star_atlas.read(metal::min(metal::uint2(_e4), metal::uint2(star_atlas.get_width(clamped_lod_e6), star_atlas.get_height(clamped_lod_e6)) - 1), clamped_lod_e6);
    if (t_1.w == 0u) {
        return metal::float4(0.0);
    }
    float dist_1 = metal::length(f_1 - metal::float2(as_type<float>(t_1.x), as_type<float>(t_1.y))) * s_1.cell;
    StarUniforms _e21 = star_settings(cloud);
    float radius = 1.0 - (_e21.star_geometry.x * 0.5);
    float outer_1 = radius * s_1.cell;
    if (dist_1 >= outer_1) {
        return metal::float4(0.0);
    }
    metal::float3 colour_1 = static_cast<metal::float3>(metal::uint3(t_1.z >> 20u, t_1.z >> 10u, t_1.z) & metal::uint3(1023u)) / metal::float3(1023.0);
    metal::float2 shape_1 = float2(as_type<half2>(t_1.w));
    float d_1 = dist_1 * shape_1.x;
    full_1 = metal::exp((-0.5 * d_1) * d_1);
    if (s_1.fringe > 0.0) {
        float _e60 = full_1;
        full_1 = _e60 + (s_1.fringe * metal::exp(-0.4 * d_1));
    }
    float _e67 = full_1;
    float cover_1 = (metal::min(_e67, 1.0) * (1.0 - metal::smoothstep((radius - 0.15) * s_1.cell, outer_1, dist_1))) * shape_1.y;
    return metal::float4(colour_1 * cover_1, cover_1);
}

metal::int2 naga_f2i32(metal::float2 value) {
    return static_cast<metal::int2>(metal::clamp(value, -2147483600.0, 2147483500.0));
}

metal::float4 star_far_gather(
    StarSlice s_2,
    metal::float2 r_1,
    constant Cloud& cloud,
    metal::texture2d<uint, metal::access::sample> star_atlas
) {
    metal::float4 result = metal::float4(0.0);
    metal::float2 o = metal::floor(r_1 - metal::float2(0.5));
    metal::float2 f_2 = r_1 - o;
    metal::int2 local_7 = as_type<metal::int2>(as_type<metal::uint2>(as_type<metal::int2>(as_type<metal::uint2>(naga_f2i32(o)) - as_type<metal::uint2>(naga_f2i32(metal::floor(s_2.offset))))) - as_type<metal::uint2>(s_2.origin));
    int index_3 = as_type<int>(as_type<uint>(as_type<int>(as_type<uint>(s_2.base) + as_type<uint>(as_type<int>(as_type<uint>(local_7.y) * as_type<uint>(s_2.grid.x))))) + as_type<uint>(local_7.x));
    metal::float4 _e25 = result;
    metal::float4 _e26 = star_far_texel(s_2, f_2, index_3, cloud, star_atlas);
    result = _e25 + _e26;
    metal::float4 _e28 = result;
    metal::float4 _e35 = star_far_texel(s_2, f_2 - metal::float2(1.0, 0.0), as_type<int>(as_type<uint>(index_3) + as_type<uint>(1)), cloud, star_atlas);
    result = _e28 + _e35;
    metal::float4 _e37 = result;
    metal::float4 _e45 = star_far_texel(s_2, f_2 - metal::float2(0.0, 1.0), as_type<int>(as_type<uint>(index_3) + as_type<uint>(s_2.grid.x)), cloud, star_atlas);
    result = _e37 + _e45;
    metal::float4 _e47 = result;
    metal::float4 _e57 = star_far_texel(s_2, f_2 - metal::float2(1.0, 1.0), as_type<int>(as_type<uint>(as_type<int>(as_type<uint>(index_3) + as_type<uint>(s_2.grid.x))) + as_type<uint>(1)), cloud, star_atlas);
    result = _e47 + _e57;
    metal::float4 _e59 = result;
    return _e59;
}

metal::float4 star_layers(
    metal::float2 pt_1,
    uint first,
    uint last,
    metal::float4 under,
    constant Cloud& cloud,
    metal::sampler cloud_sampler,
    metal::texture2d<uint, metal::access::sample> star_atlas,
    metal::texture2d_array<float, metal::access::sample> star_halos,
    metal::texture2d_array<float, metal::access::sample> star_halos_b,
    metal::texture2d_array<float, metal::access::sample> star_halos_c
) {
    metal::float4 out = {};
    float far_gap = 1.0;
    uint k_1 = {};
    metal::float4 slice = {};
    bool local = {};
    bool local_1 = {};
    bool local_2 = {};
    out = under;
    StarUniforms _e7 = star_settings(cloud);
    StarUniforms _e13 = star_settings(cloud);
    metal::float2 sp = (pt_1 - (_e7.size * 0.5)) * (STAR_PANE / _e13.size.y);
    k_1 = first;
    uint2 loop_bound = uint2(4294967295u);
    bool loop_init = true;
    while(true) {
        if (metal::all(loop_bound == uint2(0u))) { break; }
        loop_bound -= uint2(loop_bound.y == 0u, 1u);
        if (!loop_init) {
            uint _e153 = k_1;
            k_1 = _e153 + 1u;
        }
        loop_init = false;
        uint _e19 = k_1;
        if (_e19 < last) {
        } else {
            break;
        }
        {
            StarUniforms _e21 = star_settings(cloud);
            uint _e23 = k_1;
            StarSlice s_3 = _e21.star_slices.inner[metal::min(unsigned(_e23), 4u)];
            metal::float2 r_4 = (sp / metal::float2(_e21.star_slices.inner[metal::min(unsigned(_e23), 4u)].cell)) - metal::fract(_e21.star_slices.inner[metal::min(unsigned(_e23), 4u)].offset);
            metal::float2 o_1 = metal::floor(r_4);
            metal::float2 f_3 = r_4 - o_1;
            metal::int2 local_8 = as_type<metal::int2>(as_type<metal::uint2>(as_type<metal::int2>(as_type<metal::uint2>(naga_f2i32(o_1)) - as_type<metal::uint2>(naga_f2i32(metal::floor(_e21.star_slices.inner[metal::min(unsigned(_e23), 4u)].offset))))) - as_type<metal::uint2>(_e21.star_slices.inner[metal::min(unsigned(_e23), 4u)].origin));
            int index_4 = as_type<int>(as_type<uint>(as_type<int>(as_type<uint>(_e21.star_slices.inner[metal::min(unsigned(_e23), 4u)].base) + as_type<uint>(as_type<int>(as_type<uint>(local_8.y) * as_type<uint>(_e21.star_slices.inner[metal::min(unsigned(_e23), 4u)].grid.x))))) + as_type<uint>(local_8.x));
            slice = metal::float4(0.0);
            StarUniforms _e51 = star_settings(cloud);
            if (_e51.star_far.z > 0.0) {
                uint _e58 = k_1;
                local = _e58 < STAR_FAR_LAYERS;
            } else {
                local = false;
            }
            bool _e62 = local;
            if (_e62) {
                metal::float4 _e63 = star_far_gather(s_3, r_4, cloud, star_atlas);
                slice = _e63;
            } else {
                metal::float4 _e65 = star_texel(s_3, f_3, index_4, false, cloud, star_atlas);
                slice = _e65;
                metal::float4 _e66 = slice;
                uint _e67 = k_1;
                metal::float4 _e68 = star_halo_at(pt_1, _e67, cloud, cloud_sampler, star_halos, star_halos_b, star_halos_c);
                slice = _e66 + _e68;
            }
            float _e71 = slice.w;
            if (_e71 > 0.0) {
                float _e75 = slice.w;
                float cover_2 = metal::min(_e75, 1.0);
                metal::float4 _e78 = out;
                metal::float4 _e80 = slice;
                float _e83 = slice.w;
                float _e88 = out.w;
                float _e90 = out.w;
                out = metal::float4(metal::mix(_e78.xyz, _e80.xyz / metal::float3(_e83), cover_2), _e88 + ((1.0 - _e90) * cover_2));
            }
            StarUniforms _e96 = star_settings(cloud);
            if (_e96.star_geometry.w > 0.0) {
                uint _e103 = k_1;
                local_1 = _e103 < STAR_FAR_LAYERS;
            } else {
                local_1 = false;
            }
            bool _e107 = local_1;
            if (_e107) {
                float _e108 = far_gap;
                float _e110 = slice.w;
                far_gap = _e108 * (1.0 - metal::min(_e110, 1.0));
                uint _e116 = k_1;
                if ((_e116 + 1u) == STAR_FAR_LAYERS) {
                    local_2 = first == 0u;
                } else {
                    local_2 = false;
                }
                bool _e126 = local_2;
                if (_e126) {
                    StarUniforms _e127 = star_settings(cloud);
                    float amount_1 = _e127.star_geometry.w * 2.0;
                    float gentle = metal::min(amount_1, 1.0);
                    float strong = metal::max(amount_1 - 1.0, 0.0);
                    float _e138 = far_gap;
                    float _e142 = far_gap;
                    float _e144 = far_gap;
                    float gain = (1.0 + (gentle * _e138)) * (1.0 + ((strong * _e142) * _e144));
                    metal::float4 _e149 = out;
                    out = under + ((_e149 - under) * gain);
                }
            }
        }
    }
    metal::float4 _e156 = out;
    return _e156;
}

metal::float4 star_near_color(
    metal::float2 pt_2,
    constant Cloud& cloud,
    metal::sampler cloud_sampler,
    metal::texture2d<uint, metal::access::sample> star_atlas,
    metal::texture2d_array<float, metal::access::sample> star_halos,
    metal::texture2d_array<float, metal::access::sample> star_halos_b,
    metal::texture2d_array<float, metal::access::sample> star_halos_c,
    metal::texture2d<float, metal::access::sample> cloud_tone
) {
    metal::float4 far = metal::float4(0.0);
    StarUniforms _e4 = star_settings(cloud);
    if (_e4.star_far.z > 0.0) {
        StarUniforms _e11 = star_settings(cloud);
        metal::float4 _e15 = cloud_tone.sample(cloud_sampler, pt_2 / _e11.size, metal::level(0.0));
        far = _e15;
    } else {
        StarUniforms _e17 = star_settings(cloud);
        uint clamped_lod_e22 = metal::min(uint(0), cloud_tone.get_num_mip_levels() - 1);
        metal::float4 _e22 = cloud_tone.read(metal::min(metal::uint2(naga_f2i32(pt_2 * _e17.ppp)), metal::uint2(cloud_tone.get_width(clamped_lod_e22), cloud_tone.get_height(clamped_lod_e22)) - 1), clamped_lod_e22);
        far = _e22;
    }
    metal::float4 _e25 = far;
    metal::float4 _e26 = star_layers(pt_2, STAR_FAR_LAYERS, STAR_SLICES, _e25, cloud, cloud_sampler, star_atlas, star_halos, star_halos_b, star_halos_c);
    return _e26;
}

metal::float4 star_floor(
    metal::texture2d<float, metal::access::sample> lut
) {
    metal::float3 _e1 = palette_color(0.0, lut);
    return metal::float4(_e1, 1.0);
}

metal::float4 star_color(
    metal::float2 pt_3,
    constant Cloud& cloud,
    metal::sampler cloud_sampler,
    metal::texture2d<float, metal::access::sample> lut,
    metal::texture2d<uint, metal::access::sample> star_atlas,
    metal::texture2d_array<float, metal::access::sample> star_halos,
    metal::texture2d_array<float, metal::access::sample> star_halos_b,
    metal::texture2d_array<float, metal::access::sample> star_halos_c,
    metal::texture2d<float, metal::access::sample> cloud_tone
) {
    if (STAR_SPLIT) {
        StarUniforms _e2 = star_settings(cloud);
        if (_e2.star_near.x > 0.0) {
            StarUniforms _e9 = star_settings(cloud);
            metal::float4 _e13 = cloud_tone.sample(cloud_sampler, pt_3 / _e9.size, metal::level(0.0));
            return _e13;
        }
        metal::float4 _e14 = star_near_color(pt_3, cloud, cloud_sampler, star_atlas, star_halos, star_halos_b, star_halos_c, cloud_tone);
        return _e14;
    }
    metal::float4 _e17 = star_floor(lut);
    metal::float4 _e18 = star_layers(pt_3, 0u, STAR_SLICES, _e17, cloud, cloud_sampler, star_atlas, star_halos, star_halos_b, star_halos_c);
    return _e18;
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
    constant Cloud& cloud,
    metal::texture2d<float, metal::access::sample> close_light,
    metal::sampler cloud_sampler
) {
    float _e3 = cloud.ppp;
    metal::float2 _e8 = cloud.origin;
    metal::float2 _e12 = cloud.size;
    metal::float2 uv_2 = ((position / metal::float2(_e3)) - _e8) / _e12;
    metal::float4 _e17 = close_light.sample(cloud_sampler, uv_2, metal::level(0.0));
    return _e17.x;
}

bool softened(
    constant Cloud& cloud
) {
    metal::float2 _e2 = cloud.step;
    return metal::any(_e2 != metal::float2(0.0));
}

float style_level(
    float level_1,
    constant Cloud& cloud
) {
    float _e3 = cloud.contour_strength;
    if (_e3 <= 0.0) {
        return level_1;
    }
    float _e11 = cloud.contours;
    float x_1 = metal::clamp(level_1, 0.0, 1.0) * _e11;
    float _e15 = cloud.contour_softness;
    float _e16 = metal::fwidth(x_1);
    float edge = metal::min(0.5, metal::max(_e15, _e16 * 0.5));
    float _e32 = cloud.contours;
    float terraces = (metal::floor(x_1) + metal::smoothstep(0.5 - edge, 0.5 + edge, metal::fract(x_1))) / _e32;
    float _e36 = cloud.contour_strength;
    float _e43 = metal::fwidth(x_1);
    float strength = ((0.9 * _e36) * metal::smoothstep(0.0, 1.0, x_1)) * (1.0 - metal::smoothstep(0.5, 1.5, _e43));
    return metal::mix(level_1, terraces, strength);
}

metal::float4 density_color(
    float raw_level,
    constant Cloud& cloud,
    metal::texture2d<float, metal::access::sample> lut
) {
    float _e1 = style_level(raw_level, cloud);
    metal::float3 _e2 = palette_color(_e1, lut);
    return metal::float4(_e2, 1.0);
}

metal::float2 watercolor_tile_uv(
    metal::float2 r_2,
    constant Cloud& cloud
) {
    uint _e3 = cloud.tile_cells;
    uint _e7 = cloud.pitch_vertical;
    metal::float2 _e8 = watercolor_tile_uv_for(r_2, static_cast<float>(_e3), _e7);
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
    metal::float2 pt_4,
    constant Cloud& cloud,
    metal::texture2d<float, metal::access::sample> close_light,
    metal::sampler cloud_sampler
) {
    metal::float2 _e5 = cloud.size;
    metal::float4 _e8 = close_light.sample(cloud_sampler, pt_4 / _e5, metal::level(0.0));
    return _e8.x;
}

float scale_tone(
    metal::float2 pt_5,
    constant Cloud& cloud,
    metal::texture2d<float, metal::access::sample> close_light,
    metal::sampler cloud_sampler,
    metal::texture2d<float, metal::access::sample> cloud_tile_a,
    metal::sampler tile_sampler
) {
    Pile pile = {};
    metal::float2 _e3 = cloud.size;
    float _e10 = cloud.size.y;
    metal::float2 _e17 = cloud.drift;
    metal::float2 q = (((pt_5 - (_e3 * 0.5)) / metal::float2(_e10)) * CLOUD_UNITS) + _e17;
    float _e22 = cloud.scale_size;
    float scale_units = SCALE_CELLS / _e22;
    float _e27 = cloud.size.y;
    float scale_points = (_e27 / CLOUD_UNITS) / scale_units;
    metal::float2 r_5 = q * scale_units;
    uint _e36 = cloud.tile_cells;
    metal::float4 tile = cloud_tile_a.sample(tile_sampler, r_5 / metal::float2(static_cast<float>(_e36)), metal::level(0.0));
    pile.face = tile.xy;
    pile.to_centre = tile.zw;
    metal::float2 face = pile.face;
    float _e51 = cloud.scale_refract;
    float bend = metal::max(_e51, 0.0) * scale_points;
    float _e57 = cloud.scale_refract;
    float gather = metal::max(-(_e57), 0.0) * scale_points;
    metal::float2 _e65 = pile.to_centre;
    metal::float2 lookup = (-(face) * bend) + (_e65 * gather);
    float _e69 = cloud_light(pt_5 + lookup, cloud, close_light, cloud_sampler);
    return _e69;
}

float wash_level(
    Wet wet,
    float pane_per_cell,
    metal::float2 pt_6,
    constant Cloud& cloud,
    metal::texture2d<float, metal::access::sample> close_light,
    metal::sampler cloud_sampler
) {
    float _e7 = cloud.wash_refract;
    float _e10 = cloud_light(pt_6 + ((wet.offset * pane_per_cell) * _e7), cloud, close_light, cloud_sampler);
    return _e10;
}

WashField wash_tile_field(
    metal::float2 r_3,
    constant Cloud& cloud,
    metal::texture2d<float, metal::access::sample> cloud_tile_a,
    metal::texture2d<float, metal::access::sample> cloud_tile_b,
    metal::sampler tile_sampler
) {
    WashField out_1 = {};
    metal::float2 _e1 = watercolor_tile_uv(r_3, cloud);
    metal::float4 a_1 = cloud_tile_a.sample(tile_sampler, _e1, metal::level(0.0));
    metal::float2 _e9 = rotate_watercolor_tile_vector(a_1.xy, cloud);
    out_1.coarse = Wet {_e9, a_1.z};
    out_1.fine = Wet {metal::float2(0.0), 0.0};
    out_1.cover = 0.0;
    float _e21 = cloud.wash_layers;
    if (_e21 > 0.0) {
        metal::float4 b_1 = cloud_tile_b.sample(tile_sampler, _e1, metal::level(0.0));
        metal::float2 _e30 = rotate_watercolor_tile_vector(b_1.xy, cloud);
        out_1.fine = Wet {_e30, b_1.z};
        out_1.cover = b_1.w;
    }
    WashField _e35 = out_1;
    return _e35;
}

metal::float2 wash_cell_at(
    metal::float2 pt_7,
    constant Cloud& cloud
) {
    metal::float2 _e3 = cloud.size;
    float _e10 = cloud.size.y;
    metal::float2 _e17 = cloud.drift;
    metal::float2 q_1 = (((pt_7 - (_e3 * 0.5)) / metal::float2(_e10)) * CLOUD_UNITS) + _e17;
    float _e22 = cloud.wash_size;
    return q_1 * (WASH_CELLS / _e22);
}

float wash_cloud_tone(
    metal::float2 pt_8,
    constant Cloud& cloud,
    metal::texture2d<float, metal::access::sample> close_light,
    metal::sampler cloud_sampler,
    metal::texture2d<float, metal::access::sample> cloud_tile_a,
    metal::texture2d<float, metal::access::sample> cloud_tile_b,
    metal::sampler tile_sampler
) {
    float level_2 = {};
    float _e4 = cloud.wash_size;
    float cells = WASH_CELLS / _e4;
    float _e9 = cloud.size.y;
    float pane_per_cell_1 = (_e9 / CLOUD_UNITS) / cells;
    metal::float2 _e13 = wash_cell_at(pt_8, cloud);
    WashField _e14 = wash_tile_field(_e13, cloud, cloud_tile_a, cloud_tile_b, tile_sampler);
    float _e16 = wash_level(_e14.coarse, pane_per_cell_1, pt_8, cloud, close_light, cloud_sampler);
    level_2 = _e16;
    float _e20 = cloud.wash_layers;
    if (_e20 > 0.0) {
        float _e26 = wash_level(_e14.fine, pane_per_cell_1 / WASH_LACUNARITY, pt_8, cloud, close_light, cloud_sampler);
        float _e29 = cloud.wash_layers;
        float over = _e29 * _e14.cover;
        float _e32 = level_2;
        level_2 = metal::mix(_e32, _e26, over);
    }
    float _e34 = level_2;
    return _e34;
}

float cloud_tone_at(
    metal::float2 pt_9,
    constant Cloud& cloud,
    metal::texture2d<float, metal::access::sample> close_light,
    metal::sampler cloud_sampler,
    metal::texture2d<float, metal::access::sample> cloud_tile_a,
    metal::texture2d<float, metal::access::sample> cloud_tile_b,
    metal::sampler tile_sampler
) {
    uint _e3 = cloud.cloud_style;
    if (_e3 == 1u) {
        float _e6 = wash_cloud_tone(pt_9, cloud, close_light, cloud_sampler, cloud_tile_a, cloud_tile_b, tile_sampler);
        return _e6;
    }
    float _e7 = scale_tone(pt_9, cloud, close_light, cloud_sampler, cloud_tile_a, tile_sampler);
    return _e7;
}

metal::float3 memory_color(
    metal::float2 uv,
    constant Cloud& cloud,
    metal::texture2d<float, metal::access::sample> color_memory
) {
    metal::float2 _e3 = cloud.memory_extent;
    metal::int2 size = naga_f2i32(_e3);
    metal::float2 p = (uv * static_cast<metal::float2>(size)) - metal::float2(0.5);
    metal::int2 lo = naga_f2i32(metal::floor(p));
    metal::float2 f_4 = metal::fract(p);
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
    metal::float3 d_2 = _e63.xyz;
    return metal::mix(metal::mix(a_2, b_2, f_4.x), metal::mix(c, d_2, f_4.x), f_4.y);
}

metal::float4 clouded_base(
    float level_3,
    metal::float2 position_1,
    constant Cloud& cloud,
    metal::texture2d<float, metal::access::sample> close_light,
    metal::sampler cloud_sampler,
    metal::texture2d<float, metal::access::sample> color_memory,
    metal::texture2d<float, metal::access::sample> lut,
    metal::texture2d<uint, metal::access::sample> star_atlas,
    metal::texture2d_array<float, metal::access::sample> star_halos,
    metal::texture2d_array<float, metal::access::sample> star_halos_b,
    metal::texture2d_array<float, metal::access::sample> star_halos_c,
    metal::texture2d<float, metal::access::sample> cloud_tone,
    metal::texture2d<float, metal::access::sample> cloud_tile_a,
    metal::texture2d<float, metal::access::sample> cloud_tile_b,
    metal::sampler tile_sampler
) {
    float tone = {};
    float _e4 = cloud.cloud_depth;
    if (_e4 <= 0.0) {
        metal::float4 _e7 = density_color(level_3, cloud, lut);
        return _e7;
    }
    float _e10 = cloud.ppp;
    metal::float2 _e15 = cloud.origin;
    metal::float2 pt_10 = (position_1 / metal::float2(_e10)) - _e15;
    uint _e19 = cloud.cloud_style;
    if (_e19 == 2u) {
        metal::float4 _e22 = density_color(level_3, cloud, lut);
        metal::float4 _e24 = star_color(pt_10, cloud, cloud_sampler, lut, star_atlas, star_halos, star_halos_b, star_halos_c, cloud_tone);
        float _e28 = cloud.cloud_depth;
        return metal::float4(metal::mix(_e22.xyz, _e24.xyz, _e28), 1.0);
    }
    uint _e34 = cloud.memory_enabled;
    if (_e34 != 0u) {
        metal::float2 dimensions = cloud.memory_extent;
        metal::float2 _e42 = cloud.size;
        metal::float2 _e53 = cloud.memory_fraction;
        metal::float2 uv_3 = ((((pt_10 / _e42) * (dimensions - metal::float2(2.0))) + metal::float2(1.0)) + _e53) / dimensions;
        metal::float3 _e56 = memory_color(uv_3, cloud, color_memory);
        float _e59 = cloud.cloud_depth;
        if (_e59 >= 1.0) {
            metal::float3 _e62 = gamma_from_linear_rgb(_e56);
            return metal::float4(_e62, 1.0);
        }
        metal::float4 _e65 = density_color(level_3, cloud, lut);
        metal::float3 _e67 = linear_from_gamma_rgb(_e65.xyz);
        float _e70 = cloud.cloud_depth;
        metal::float3 _e72 = gamma_from_linear_rgb(metal::mix(_e67, _e56, _e70));
        return metal::float4(_e72, 1.0);
    }
    uint _e78 = cloud.tone_baked;
    if (_e78 == 1u) {
        metal::float2 _e85 = cloud.size;
        metal::float4 _e88 = cloud_tone.sample(cloud_sampler, pt_10 / _e85, metal::level(0.0));
        tone = _e88.x;
    } else {
        float _e90 = cloud_tone_at(pt_10, cloud, close_light, cloud_sampler, cloud_tile_a, cloud_tile_b, tile_sampler);
        tone = _e90;
    }
    float _e91 = tone;
    float _e94 = cloud.cloud_depth;
    metal::float4 _e96 = density_color(metal::mix(level_3, _e91, _e94), cloud, lut);
    return _e96;
}

metal::float4 clouded(
    float level_4,
    metal::float2 position_2,
    constant Cloud& cloud,
    metal::texture2d<float, metal::access::sample> close_light,
    metal::sampler cloud_sampler,
    metal::texture2d<float, metal::access::sample> color_memory,
    metal::texture2d<float, metal::access::sample> lut,
    metal::texture2d<uint, metal::access::sample> star_atlas,
    metal::texture2d_array<float, metal::access::sample> star_halos,
    metal::texture2d_array<float, metal::access::sample> star_halos_b,
    metal::texture2d_array<float, metal::access::sample> star_halos_c,
    metal::texture2d<float, metal::access::sample> cloud_tone,
    metal::texture2d<float, metal::access::sample> cloud_tile_a,
    metal::texture2d<float, metal::access::sample> cloud_tile_b,
    metal::sampler tile_sampler
) {
    bool local_3 = {};
    bool local_4 = {};
    metal::float4 _e2 = clouded_base(level_4, position_2, cloud, close_light, cloud_sampler, color_memory, lut, star_atlas, star_halos, star_halos_b, star_halos_c, cloud_tone, cloud_tile_a, cloud_tile_b, tile_sampler);
    uint _e5 = cloud.cloud_style;
    if (!((_e5 != 1u))) {
        float _e13 = cloud.wash_randomness;
        local_3 = _e13 <= 0.0;
    } else {
        local_3 = true;
    }
    bool _e17 = local_3;
    if (!(_e17)) {
        float _e23 = cloud.cloud_depth;
        local_4 = _e23 <= 0.0;
    } else {
        local_4 = true;
    }
    bool _e27 = local_4;
    if (_e27) {
        return _e2;
    }
    float _e30 = cloud.ppp;
    metal::float2 _e35 = cloud.origin;
    metal::float2 pt_11 = (position_2 / metal::float2(_e30)) - _e35;
    metal::float2 _e37 = wash_cell_at(pt_11, cloud);
    WashField _e38 = wash_tile_field(_e37, cloud, cloud_tile_a, cloud_tile_b, tile_sampler);
    float _e45 = cloud.wash_layers;
    float draw_1 = metal::mix(_e38.coarse.brightness, _e38.fine.brightness, _e45 * _e38.cover);
    metal::float3 _e50 = linear_from_gamma_rgb(_e2.xyz);
    float _e54 = cloud.wash_randomness;
    float _e57 = cloud.cloud_depth;
    metal::float3 _e59 = wash_vary_brightness(_e50, 1.0, draw_1, _e54 * _e57);
    metal::float3 _e60 = gamma_from_linear_rgb(_e59);
    return metal::float4(_e60, _e2.w);
}

bool full_material_memory(
    constant Cloud& cloud
) {
    bool local_5 = {};
    bool local_6 = {};
    uint _e2 = cloud.memory_enabled;
    if (_e2 != 0u) {
        float _e9 = cloud.cloud_depth;
        local_5 = _e9 >= 1.0;
    } else {
        local_5 = false;
    }
    bool _e13 = local_5;
    if (_e13) {
        uint _e18 = cloud.cloud_style;
        local_6 = _e18 != 2u;
    } else {
        local_6 = false;
    }
    bool _e22 = local_6;
    return _e22;
}

metal::float4 backdrop_color(
    metal::float2 position_3,
    constant Cloud& cloud,
    metal::texture2d<float, metal::access::sample> close_light,
    metal::sampler cloud_sampler,
    metal::texture2d<float, metal::access::sample> color_memory,
    metal::texture2d<float, metal::access::sample> lut,
    metal::texture2d<uint, metal::access::sample> star_atlas,
    metal::texture2d_array<float, metal::access::sample> star_halos,
    metal::texture2d_array<float, metal::access::sample> star_halos_b,
    metal::texture2d_array<float, metal::access::sample> star_halos_c,
    metal::texture2d<float, metal::access::sample> cloud_tone,
    metal::texture2d<float, metal::access::sample> cloud_tile_a,
    metal::texture2d<float, metal::access::sample> cloud_tile_b,
    metal::sampler tile_sampler
) {
    float level_5 = 0.0;
    bool _e1 = full_material_memory(cloud);
    if (_e1) {
        metal::float4 _e3 = clouded(0.0, position_3, cloud, close_light, cloud_sampler, color_memory, lut, star_atlas, star_halos, star_halos_b, star_halos_c, cloud_tone, cloud_tile_a, cloud_tile_b, tile_sampler);
        return _e3;
    }
    bool _e6 = softened(cloud);
    if (_e6) {
        float _e7 = baked_density(position_3, cloud, close_light, cloud_sampler);
        level_5 = _e7;
    }
    float _e8 = level_5;
    metal::float4 _e9 = clouded(_e8, position_3, cloud, close_light, cloud_sampler, color_memory, lut, star_atlas, star_halos, star_halos_b, star_halos_c, cloud_tone, cloud_tile_a, cloud_tile_b, tile_sampler);
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
, metal::float4 position_4 [[position]]
, constant Cloud& cloud [[buffer(2)]]
, metal::texture2d<float, metal::access::sample> close_light [[texture(1)]]
, metal::sampler cloud_sampler [[sampler(0)]]
, metal::texture2d<float, metal::access::sample> color_memory [[texture(7)]]
, metal::texture2d<float, metal::access::sample> lut [[texture(0)]]
, metal::texture2d<uint, metal::access::sample> star_atlas [[texture(6)]]
, metal::texture2d_array<float, metal::access::sample> star_halos [[texture(8)]]
, metal::texture2d_array<float, metal::access::sample> star_halos_b [[texture(9)]]
, metal::texture2d_array<float, metal::access::sample> star_halos_c [[texture(10)]]
, metal::texture2d<float, metal::access::sample> cloud_tone [[texture(3)]]
, metal::texture2d<float, metal::access::sample> cloud_tile_a [[texture(4)]]
, metal::texture2d<float, metal::access::sample> cloud_tile_b [[texture(5)]]
, metal::sampler tile_sampler [[sampler(1)]]
) {
    const VertexOut in = { position_4, varyings.slab, varyings.t };
    metal::float4 _e3 = backdrop_color(in.position.xy, cloud, close_light, cloud_sampler, color_memory, lut, star_atlas, star_halos, star_halos_b, star_halos_c, cloud_tone, cloud_tile_a, cloud_tile_b, tile_sampler);
    metal::float3 _e5 = linear_from_gamma_rgb(_e3.xyz);
    return fs_cloud_backdrop_linearOutput { metal::float4(_e5, 1.0) };
}
