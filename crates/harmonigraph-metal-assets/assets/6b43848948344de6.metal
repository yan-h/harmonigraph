// language: metal3.2
#include <metal_stdlib>
#include <simd/simd.h>

using metal::uint;

struct Wet {
    metal::float2 offset;
    float brightness;
    float gap;
};
struct WashField {
    Wet coarse;
    Wet fine;
    float cover;
    char _pad3[4];
};
struct StarHaloSample {
    metal::float2 size;
    uint group;
    uint layer;
};
struct StarSlice {
    metal::float2 offset;
    float cell;
    float radius;
    float core;
    float glow;
    float falloff;
    int base;
    metal::int2 origin;
    metal::int2 grid;
    float width;
    float inner;
    float gain;
    uint gather;
};
struct type_10 {
    StarSlice inner[5];
};
struct type_11 {
    StarHaloSample inner[5];
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
    uint star_pad0_;
    uint star_pad1_;
    metal::float4 star_far;
    metal::float4 star_near;
    metal::float4 star_geometry;
    type_10 star_slices;
    uint memory_enabled;
    uint memory_valid;
    float pickup_alpha;
    float release_alpha;
    metal::int2 memory_shift;
    metal::float2 memory_fraction;
    float previous_life;
    float wash_randomness;
    metal::float2 memory_extent;
    type_10 previous_slices;
    type_11 star_halo_samples;
    metal::float4 velvet;
    metal::float4 velvet_form;
    metal::float4 wash_pigment;
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
constant float WASH_FBM_FINE = 2.0;
constant int STAR_ATLAS_WIDTH = 2048;
constant uint STAR_ATLAS_SHIFT = 11u;
constant uint STAR_SLICES = 5u;
constant float STAR_PANE = 540.0;
constant int STAR_HASH_PERIOD = 65536;
constant uint STAR_LIFE_PERIOD = 4096u;
constant float STAR_FADE = 0.2;
constant float STAR_LIFT = 0.18;
constant uint STAR_FAR_LAYERS = 3u;
constant float CLOUD_UNITS = 10.0;
constant float WASH_POOL = 0.44;
constant float WASH_PIG_DEPTH = 0.35;
constant float WASH_BLOOM = 2.7;
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

metal::float2 star_size(
    constant Cloud& cloud
) {
    metal::float2 _e2 = cloud.size;
    return _e2;
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
    float v_1 = {};
    if (t >= 1.0) {
        return 0.0;
    }
    float q = t / s.core;
    v_1 = metal::exp((-2.0 * q) * q);
    if (s.glow > 0.0) {
        float _e15 = v_1;
        v_1 = _e15 + (s.glow * metal::pow(1.0 - t, s.falloff));
    }
    float _e23 = v_1;
    return metal::min(_e23, 1.0) * (1.0 - metal::smoothstep(0.75, 1.0, t));
}

metal::float4 star_geometry(
    constant Cloud& cloud
) {
    metal::float4 _e2 = cloud.star_geometry;
    return _e2;
}

metal::float4 star_texel(
    StarSlice s_1,
    metal::float2 f,
    int index_1,
    bool halo,
    constant Cloud& cloud,
    metal::texture2d<uint, metal::access::sample> star_atlas
) {
    bool local = {};
    uint clamped_lod_e11 = metal::min(uint(0), star_atlas.get_num_mip_levels() - 1);
    metal::uint4 t_1 = star_atlas.read(metal::min(metal::uint2(metal::int2(index_1 & 2047, index_1 >> STAR_ATLAS_SHIFT)), metal::uint2(star_atlas.get_width(clamped_lod_e11), star_atlas.get_height(clamped_lod_e11)) - 1), clamped_lod_e11);
    if (t_1.w == 0u) {
        return metal::float4(0.0);
    }
    float dist = metal::length(f - metal::float2(as_type<float>(t_1.x), as_type<float>(t_1.y))) * s_1.cell;
    float reach = s_1.inner * s_1.cell;
    if (!(halo)) {
        local = dist >= reach;
    } else {
        local = false;
    }
    bool _e34 = local;
    if (_e34) {
        return metal::float4(0.0);
    }
    metal::float2 shape = float2(as_type<half2>(t_1.w));
    float _e41 = star_profile(s_1, dist * shape.x);
    if (_e41 <= 0.0) {
        return metal::float4(0.0);
    }
    metal::float3 colour = static_cast<metal::float3>(metal::uint3(t_1.z >> 20u, t_1.z >> 10u, t_1.z) & metal::uint3(1023u)) / metal::float3(1023.0);
    metal::float4 _e61 = star_geometry(cloud);
    float inner = _e41 * (1.0 - metal::smoothstep(_e61.z * reach, reach, dist));
    float cover = (halo ? metal::max(_e41 - inner, 0.0) : inner) * shape.y;
    return metal::float4(colour * cover, cover);
}

StarHaloSample star_halo_sample(
    uint k_1,
    constant Cloud& cloud
) {
    StarHaloSample _e4 = cloud.star_halo_samples.inner[metal::min(unsigned(k_1), 4u)];
    return _e4;
}

metal::float4 star_halo_at(
    metal::float2 pt,
    uint k_2,
    constant Cloud& cloud,
    metal::sampler cloud_sampler,
    metal::texture2d_array<float, metal::access::sample> star_halos,
    metal::texture2d_array<float, metal::access::sample> star_halos_b,
    metal::texture2d_array<float, metal::access::sample> star_halos_c
) {
    StarHaloSample _e2 = star_halo_sample(k_2, cloud);
    metal::float2 _e3 = star_size(cloud);
    metal::float2 uv_1 = pt / _e3;
    switch(_e2.group) {
        case 0u: {
            metal::float4 _e11 = star_halos.sample(cloud_sampler, uv_1, static_cast<int>(_e2.layer), metal::level(0.0));
            return _e11;
        }
        case 1u: {
            metal::float4 _e17 = star_halos_b.sample(cloud_sampler, uv_1, static_cast<int>(_e2.layer), metal::level(0.0));
            return _e17;
        }
        default: {
            metal::float4 _e23 = star_halos_c.sample(cloud_sampler, uv_1, static_cast<int>(_e2.layer), metal::level(0.0));
            return _e23;
        }
    }
}

metal::float4 star_far_texel(
    StarSlice s_2,
    metal::float2 f_1,
    int index_2,
    metal::texture2d<uint, metal::access::sample> star_atlas
) {
    metal::int2 _e4 = atlas_texel(index_2);
    uint clamped_lod_e6 = metal::min(uint(0), star_atlas.get_num_mip_levels() - 1);
    metal::uint4 t_2 = star_atlas.read(metal::min(metal::uint2(_e4), metal::uint2(star_atlas.get_width(clamped_lod_e6), star_atlas.get_height(clamped_lod_e6)) - 1), clamped_lod_e6);
    if (t_2.w == 0u) {
        return metal::float4(0.0);
    }
    float dist_1 = metal::length(f_1 - metal::float2(as_type<float>(t_2.x), as_type<float>(t_2.y))) * s_2.cell;
    metal::float2 shape_1 = float2(as_type<half2>(t_2.w));
    float _e25 = star_profile(s_2, dist_1 * shape_1.x);
    float cover_1 = _e25 * shape_1.y;
    if (cover_1 <= 0.0) {
        return metal::float4(0.0);
    }
    metal::float3 colour_1 = static_cast<metal::float3>(metal::uint3(t_2.z >> 20u, t_2.z >> 10u, t_2.z) & metal::uint3(1023u)) / metal::float3(1023.0);
    return metal::float4(colour_1 * cover_1, cover_1);
}

metal::int2 naga_f2i32(metal::float2 value) {
    return static_cast<metal::int2>(metal::clamp(value, -2147483600.0, 2147483500.0));
}

metal::float4 star_far_gather(
    StarSlice s_3,
    metal::float2 r_1,
    metal::texture2d<uint, metal::access::sample> star_atlas
) {
    metal::float4 result = metal::float4(0.0);
    metal::float2 o = metal::floor(r_1 - metal::float2(0.5));
    metal::float2 f_2 = r_1 - o;
    metal::int2 local_8 = as_type<metal::int2>(as_type<metal::uint2>(as_type<metal::int2>(as_type<metal::uint2>(naga_f2i32(o)) - as_type<metal::uint2>(naga_f2i32(metal::floor(s_3.offset))))) - as_type<metal::uint2>(s_3.origin));
    int index_3 = as_type<int>(as_type<uint>(as_type<int>(as_type<uint>(s_3.base) + as_type<uint>(as_type<int>(as_type<uint>(local_8.y) * as_type<uint>(s_3.grid.x))))) + as_type<uint>(local_8.x));
    metal::float4 _e25 = result;
    metal::float4 _e26 = star_far_texel(s_3, f_2, index_3, star_atlas);
    result = _e25 + _e26;
    metal::float4 _e28 = result;
    metal::float4 _e35 = star_far_texel(s_3, f_2 - metal::float2(1.0, 0.0), as_type<int>(as_type<uint>(index_3) + as_type<uint>(1)), star_atlas);
    result = _e28 + _e35;
    metal::float4 _e37 = result;
    metal::float4 _e45 = star_far_texel(s_3, f_2 - metal::float2(0.0, 1.0), as_type<int>(as_type<uint>(index_3) + as_type<uint>(s_3.grid.x)), star_atlas);
    result = _e37 + _e45;
    metal::float4 _e47 = result;
    metal::float4 _e57 = star_far_texel(s_3, f_2 - metal::float2(1.0, 1.0), as_type<int>(as_type<uint>(as_type<int>(as_type<uint>(index_3) + as_type<uint>(s_3.grid.x))) + as_type<uint>(1)), star_atlas);
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
    uint k_3 = {};
    metal::float4 slice = {};
    bool local_1 = {};
    bool local_2 = {};
    bool local_3 = {};
    out = under;
    metal::float2 _e7 = star_size(cloud);
    metal::float2 _e12 = star_size(cloud);
    metal::float2 sp = (pt_1 - (_e7 * 0.5)) * (STAR_PANE / _e12.y);
    k_3 = first;
    uint2 loop_bound = uint2(4294967295u);
    bool loop_init = true;
    while(true) {
        if (metal::all(loop_bound == uint2(0u))) { break; }
        loop_bound -= uint2(loop_bound.y == 0u, 1u);
        if (!loop_init) {
            uint _e191 = k_3;
            k_3 = _e191 + 1u;
        }
        loop_init = false;
        uint _e17 = k_3;
        if (_e17 < last) {
        } else {
            break;
        }
        {
            uint _e19 = k_3;
            StarSlice _e20 = star_slice(_e19, cloud);
            metal::float2 r_4 = (sp / metal::float2(_e20.cell)) - metal::fract(_e20.offset);
            metal::float2 o_1 = metal::floor(r_4);
            metal::float2 f_3 = r_4 - o_1;
            metal::int2 local_9 = as_type<metal::int2>(as_type<metal::uint2>(as_type<metal::int2>(as_type<metal::uint2>(naga_f2i32(o_1)) - as_type<metal::uint2>(naga_f2i32(metal::floor(_e20.offset))))) - as_type<metal::uint2>(_e20.origin));
            int index_4 = as_type<int>(as_type<uint>(as_type<int>(as_type<uint>(_e20.base) + as_type<uint>(as_type<int>(as_type<uint>(local_9.y) * as_type<uint>(_e20.grid.x))))) + as_type<uint>(local_9.x));
            slice = metal::float4(0.0);
            if (_e20.gather == 2u) {
                metal::float4 _e50 = star_far_gather(_e20, r_4, star_atlas);
                slice = _e50;
            } else {
                if (_e20.gather == 1u) {
                    metal::float4 _e54 = star_far_texel(_e20, f_3, index_4, star_atlas);
                    slice = _e54;
                } else {
                    if (_e20.gather == 3u) {
                        metal::float4 _e59 = star_texel(_e20, f_3, index_4, false, cloud, star_atlas);
                        slice = _e59;
                        metal::float4 _e60 = slice;
                        uint _e61 = k_3;
                        metal::float4 _e62 = star_halo_at(pt_1, _e61, cloud, cloud_sampler, star_halos, star_halos_b, star_halos_c);
                        slice = _e60 + _e62;
                    }
                }
            }
            float _e65 = slice.w;
            if (_e65 > 0.0) {
                float _e69 = slice.w;
                float cover_2 = metal::min(_e69, 1.0);
                metal::float4 _e72 = out;
                metal::float4 _e74 = slice;
                float _e77 = slice.w;
                float _e82 = out.w;
                float _e84 = out.w;
                out = metal::float4(metal::mix(_e72.xyz, _e74.xyz / metal::float3(_e77), cover_2), _e82 + ((1.0 - _e84) * cover_2));
                metal::float4 _e90 = star_geometry(cloud);
                float light = _e90.x;
                float _e93 = slice.w;
                if (_e93 > 1.0) {
                    local_1 = light > 0.0;
                } else {
                    local_1 = false;
                }
                bool _e101 = local_1;
                if (_e101) {
                    metal::float4 _e102 = slice;
                    float _e105 = slice.w;
                    metal::float3 base = _e102.xyz / metal::float3(_e105);
                    metal::float3 room = metal::max(metal::float3(1.0) - base, metal::float3(0.0));
                    float _e115 = slice.w;
                    metal::float3 excess = (base * (_e115 - 1.0)) * light;
                    metal::float4 _e120 = out;
                    float _e134 = out.w;
                    out = metal::float4(_e120.xyz + (room * (metal::float3(1.0) - metal::exp(-(excess) / metal::max(room, metal::float3(0.0001))))), _e134);
                }
            }
            metal::float4 _e136 = star_geometry(cloud);
            if (_e136.w > 0.0) {
                uint _e142 = k_3;
                local_2 = _e142 < STAR_FAR_LAYERS;
            } else {
                local_2 = false;
            }
            bool _e146 = local_2;
            if (_e146) {
                float _e147 = far_gap;
                float _e149 = slice.w;
                far_gap = _e147 * (1.0 - metal::min(_e149, 1.0));
                uint _e155 = k_3;
                if ((_e155 + 1u) == STAR_FAR_LAYERS) {
                    local_3 = first == 0u;
                } else {
                    local_3 = false;
                }
                bool _e165 = local_3;
                if (_e165) {
                    metal::float4 _e166 = star_geometry(cloud);
                    float amount_1 = _e166.w * 2.0;
                    float gentle = metal::min(amount_1, 1.0);
                    float strong = metal::max(amount_1 - 1.0, 0.0);
                    float _e176 = far_gap;
                    float _e180 = far_gap;
                    float _e182 = far_gap;
                    float gain = (1.0 + (gentle * _e176)) * (1.0 + ((strong * _e180) * _e182));
                    metal::float4 _e187 = out;
                    out = under + ((_e187 - under) * gain);
                }
            }
        }
    }
    metal::float4 _e194 = out;
    return _e194;
}

metal::float4 star_far(
    constant Cloud& cloud
) {
    metal::float4 _e2 = cloud.star_far;
    return _e2;
}

float star_ppp(
    constant Cloud& cloud
) {
    float _e2 = cloud.ppp;
    return _e2;
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
    metal::float4 _e4 = star_far(cloud);
    if (_e4.z > 0.0) {
        metal::float2 _e10 = star_size(cloud);
        metal::float4 _e13 = cloud_tone.sample(cloud_sampler, pt_2 / _e10, metal::level(0.0));
        far = _e13;
    } else {
        float _e15 = star_ppp(cloud);
        uint clamped_lod_e19 = metal::min(uint(0), cloud_tone.get_num_mip_levels() - 1);
        metal::float4 _e19 = cloud_tone.read(metal::min(metal::uint2(naga_f2i32(pt_2 * _e15)), metal::uint2(cloud_tone.get_width(clamped_lod_e19), cloud_tone.get_height(clamped_lod_e19)) - 1), clamped_lod_e19);
        far = _e19;
    }
    metal::float4 _e22 = far;
    metal::float4 _e23 = star_layers(pt_2, STAR_FAR_LAYERS, STAR_SLICES, _e22, cloud, cloud_sampler, star_atlas, star_halos, star_halos_b, star_halos_c);
    return _e23;
}

metal::float4 star_near(
    constant Cloud& cloud
) {
    metal::float4 _e2 = cloud.star_near;
    return _e2;
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
        metal::float4 _e2 = star_near(cloud);
        if (_e2.x > 0.0) {
            metal::float2 _e8 = star_size(cloud);
            metal::float4 _e11 = cloud_tone.sample(cloud_sampler, pt_3 / _e8, metal::level(0.0));
            return _e11;
        }
        metal::float4 _e12 = star_near_color(pt_3, cloud, cloud_sampler, star_atlas, star_halos, star_halos_b, star_halos_c, cloud_tone);
        return _e12;
    }
    metal::float4 _e15 = star_floor(lut);
    metal::float4 _e16 = star_layers(pt_3, 0u, STAR_SLICES, _e15, cloud, cloud_sampler, star_atlas, star_halos, star_halos_b, star_halos_c);
    return _e16;
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
    metal::float2 v_2,
    constant Cloud& cloud
) {
    uint _e3 = cloud.pitch_vertical;
    metal::float2 _e4 = rotate_watercolor_tile_vector_for(v_2, _e3);
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

bool wash_pigmented(
    constant Cloud& cloud
) {
    float _e3 = cloud.wash_pigment.x;
    return _e3 != 0.0;
}

float wash_level(
    Wet wet,
    float pane_per_cell,
    metal::float2 pt_5,
    constant Cloud& cloud,
    metal::texture2d<float, metal::access::sample> close_light,
    metal::sampler cloud_sampler
) {
    float _e7 = cloud.wash_refract;
    float _e10 = cloud_light(pt_5 + ((wet.offset * pane_per_cell) * _e7), cloud, close_light, cloud_sampler);
    bool _e11 = wash_pigmented(cloud);
    if (!(_e11)) {
        return _e10;
    }
    float _e17 = cloud.wash_pigment.y;
    float _e27 = cloud.wash_pigment.z;
    float shape_2 = metal::pow(metal::clamp(1.0 - (wet.gap / _e17), 0.0, 1.0), _e27);
    float fuzz = cloud.wash_fuzz;
    float _e36 = cloud.wash_pigment.x;
    float pigment = ((WASH_POOL * _e36) * (1.0 - (0.75 * fuzz))) * shape_2;
    if (pigment < 0.0) {
        return metal::min(_e10 - (((pigment * WASH_BLOOM) * _e10) * (1.0 - _e10)), 1.0);
    }
    return metal::max(_e10 - (pigment * (WASH_PIG_DEPTH + (0.65 * _e10))), 0.0);
}

WashField wash_tile_field(
    metal::float2 r_3,
    constant Cloud& cloud,
    metal::texture2d<float, metal::access::sample> cloud_tile_a,
    metal::texture2d<float, metal::access::sample> cloud_tile_b,
    metal::texture2d<float, metal::access::sample> cloud_tile_c,
    metal::sampler tile_sampler
) {
    WashField out_1 = {};
    metal::float4 c = metal::float4(0.0);
    metal::float2 _e1 = watercolor_tile_uv(r_3, cloud);
    metal::float4 a_1 = cloud_tile_a.sample(tile_sampler, _e1, metal::level(0.0));
    bool _e10 = wash_pigmented(cloud);
    if (_e10) {
        metal::float4 _e14 = cloud_tile_c.sample(tile_sampler, _e1, metal::level(0.0));
        c = _e14;
    }
    metal::float2 _e17 = rotate_watercolor_tile_vector(a_1.xy, cloud);
    float _e20 = c.x;
    out_1.coarse = Wet {_e17, a_1.z, _e20};
    out_1.fine = Wet {metal::float2(0.0), 0.0, 0.0};
    out_1.cover = 0.0;
    float _e32 = cloud.wash_layers;
    if (_e32 > 0.0) {
        metal::float4 b_1 = cloud_tile_b.sample(tile_sampler, _e1, metal::level(0.0));
        metal::float2 _e41 = rotate_watercolor_tile_vector(b_1.xy, cloud);
        float _e44 = c.y;
        out_1.fine = Wet {_e41, b_1.z, _e44};
        out_1.cover = b_1.w;
    }
    WashField _e48 = out_1;
    return _e48;
}

metal::float2 wash_cell_at(
    metal::float2 pt_6,
    constant Cloud& cloud
) {
    metal::float2 _e3 = cloud.size;
    float _e10 = cloud.size.y;
    metal::float2 _e17 = cloud.drift;
    metal::float2 q_1 = (((pt_6 - (_e3 * 0.5)) / metal::float2(_e10)) * CLOUD_UNITS) + _e17;
    float _e22 = cloud.wash_size;
    return q_1 * (WASH_CELLS / _e22);
}

float wash_cloud_tone(
    metal::float2 pt_7,
    constant Cloud& cloud,
    metal::texture2d<float, metal::access::sample> close_light,
    metal::sampler cloud_sampler,
    metal::texture2d<float, metal::access::sample> cloud_tile_a,
    metal::texture2d<float, metal::access::sample> cloud_tile_b,
    metal::texture2d<float, metal::access::sample> cloud_tile_c,
    metal::sampler tile_sampler
) {
    float level_2 = {};
    float _e4 = cloud.wash_size;
    float cells = WASH_CELLS / _e4;
    float _e9 = cloud.size.y;
    float pane_per_cell_1 = (_e9 / CLOUD_UNITS) / cells;
    metal::float2 _e13 = wash_cell_at(pt_7, cloud);
    WashField _e14 = wash_tile_field(_e13, cloud, cloud_tile_a, cloud_tile_b, cloud_tile_c, tile_sampler);
    float _e16 = wash_level(_e14.coarse, pane_per_cell_1, pt_7, cloud, close_light, cloud_sampler);
    level_2 = _e16;
    float _e20 = cloud.wash_layers;
    if (_e20 > 0.0) {
        float _e26 = wash_level(_e14.fine, pane_per_cell_1 / WASH_LACUNARITY, pt_7, cloud, close_light, cloud_sampler);
        float _e29 = cloud.wash_layers;
        float over = _e29 * _e14.cover;
        float _e32 = level_2;
        level_2 = metal::mix(_e32, _e26, over);
    }
    float _e34 = level_2;
    return _e34;
}

float cloud_tone_at(
    metal::float2 pt_8,
    constant Cloud& cloud,
    metal::texture2d<float, metal::access::sample> close_light,
    metal::sampler cloud_sampler,
    metal::texture2d<float, metal::access::sample> cloud_tone,
    metal::texture2d<float, metal::access::sample> cloud_tile_a,
    metal::texture2d<float, metal::access::sample> cloud_tile_b,
    metal::texture2d<float, metal::access::sample> cloud_tile_c,
    metal::sampler tile_sampler
) {
    uint _e3 = cloud.cloud_style;
    if (_e3 == 3u) {
        metal::float2 _e10 = cloud.size;
        metal::float4 _e13 = cloud_tone.sample(cloud_sampler, pt_8 / _e10, metal::level(0.0));
        return _e13.x;
    }
    float _e15 = wash_cloud_tone(pt_8, cloud, close_light, cloud_sampler, cloud_tile_a, cloud_tile_b, cloud_tile_c, tile_sampler);
    return _e15;
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
    metal::float3 c_1 = _e49.xyz;
    uint clamped_lod_e63 = metal::min(uint(0), color_memory.get_num_mip_levels() - 1);
    metal::float4 _e63 = color_memory.read(metal::min(metal::uint2(metal::clamp(as_type<metal::int2>(as_type<metal::uint2>(lo) + as_type<metal::uint2>(metal::int2(1, 1))), metal::int2(0), as_type<metal::int2>(as_type<metal::uint2>(size) - as_type<metal::uint2>(metal::int2(1))))), metal::uint2(color_memory.get_width(clamped_lod_e63), color_memory.get_height(clamped_lod_e63)) - 1), clamped_lod_e63);
    metal::float3 d = _e63.xyz;
    return metal::mix(metal::mix(a_2, b_2, f_4.x), metal::mix(c_1, d, f_4.x), f_4.y);
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
    metal::texture2d<float, metal::access::sample> cloud_tile_c,
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
    metal::float2 pt_9 = (position_1 / metal::float2(_e10)) - _e15;
    uint _e19 = cloud.cloud_style;
    if (_e19 == 2u) {
        metal::float4 _e22 = density_color(level_3, cloud, lut);
        metal::float4 _e24 = star_color(pt_9, cloud, cloud_sampler, lut, star_atlas, star_halos, star_halos_b, star_halos_c, cloud_tone);
        float _e28 = cloud.cloud_depth;
        return metal::float4(metal::mix(_e22.xyz, _e24.xyz, _e28), 1.0);
    }
    uint _e34 = cloud.memory_enabled;
    if (_e34 != 0u) {
        metal::float2 dimensions = cloud.memory_extent;
        metal::float2 _e42 = cloud.size;
        metal::float2 _e53 = cloud.memory_fraction;
        metal::float2 uv_3 = ((((pt_9 / _e42) * (dimensions - metal::float2(2.0))) + metal::float2(1.0)) + _e53) / dimensions;
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
        metal::float4 _e88 = cloud_tone.sample(cloud_sampler, pt_9 / _e85, metal::level(0.0));
        tone = _e88.x;
    } else {
        float _e90 = cloud_tone_at(pt_9, cloud, close_light, cloud_sampler, cloud_tone, cloud_tile_a, cloud_tile_b, cloud_tile_c, tile_sampler);
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
    metal::texture2d<float, metal::access::sample> cloud_tile_c,
    metal::sampler tile_sampler
) {
    bool local_4 = {};
    bool local_5 = {};
    metal::float4 _e2 = clouded_base(level_4, position_2, cloud, close_light, cloud_sampler, color_memory, lut, star_atlas, star_halos, star_halos_b, star_halos_c, cloud_tone, cloud_tile_a, cloud_tile_b, cloud_tile_c, tile_sampler);
    uint _e5 = cloud.cloud_style;
    if (!((_e5 != 1u))) {
        float _e13 = cloud.wash_randomness;
        local_4 = _e13 <= 0.0;
    } else {
        local_4 = true;
    }
    bool _e17 = local_4;
    if (!(_e17)) {
        float _e23 = cloud.cloud_depth;
        local_5 = _e23 <= 0.0;
    } else {
        local_5 = true;
    }
    bool _e27 = local_5;
    if (_e27) {
        return _e2;
    }
    float _e30 = cloud.ppp;
    metal::float2 _e35 = cloud.origin;
    metal::float2 pt_10 = (position_2 / metal::float2(_e30)) - _e35;
    metal::float2 _e37 = wash_cell_at(pt_10, cloud);
    WashField _e38 = wash_tile_field(_e37, cloud, cloud_tile_a, cloud_tile_b, cloud_tile_c, tile_sampler);
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
    bool local_6 = {};
    bool local_7 = {};
    uint _e2 = cloud.memory_enabled;
    if (_e2 != 0u) {
        float _e9 = cloud.cloud_depth;
        local_6 = _e9 >= 1.0;
    } else {
        local_6 = false;
    }
    bool _e13 = local_6;
    if (_e13) {
        uint _e18 = cloud.cloud_style;
        local_7 = _e18 != 2u;
    } else {
        local_7 = false;
    }
    bool _e22 = local_7;
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
    metal::texture2d<float, metal::access::sample> cloud_tile_c,
    metal::sampler tile_sampler
) {
    float level_5 = 0.0;
    bool _e1 = full_material_memory(cloud);
    if (_e1) {
        metal::float4 _e3 = clouded(0.0, position_3, cloud, close_light, cloud_sampler, color_memory, lut, star_atlas, star_halos, star_halos_b, star_halos_c, cloud_tone, cloud_tile_a, cloud_tile_b, cloud_tile_c, tile_sampler);
        return _e3;
    }
    bool _e6 = softened(cloud);
    if (_e6) {
        float _e7 = baked_density(position_3, cloud, close_light, cloud_sampler);
        level_5 = _e7;
    }
    float _e8 = level_5;
    metal::float4 _e9 = clouded(_e8, position_3, cloud, close_light, cloud_sampler, color_memory, lut, star_atlas, star_halos, star_halos_b, star_halos_c, cloud_tone, cloud_tile_a, cloud_tile_b, cloud_tile_c, tile_sampler);
    return _e9;
}

struct fs_cloud_backdrop_gammaInput {
    float slab [[user(loc0), center_perspective]];
    float t [[user(loc1), center_perspective]];
};
struct fs_cloud_backdrop_gammaOutput {
    metal::float4 member [[color(0)]];
};
fragment fs_cloud_backdrop_gammaOutput fs_cloud_backdrop_gamma(
  fs_cloud_backdrop_gammaInput varyings [[stage_in]]
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
, metal::texture2d<float, metal::access::sample> cloud_tile_c [[texture(11)]]
, metal::sampler tile_sampler [[sampler(1)]]
) {
    const VertexOut in = { position_4, varyings.slab, varyings.t };
    metal::float4 _e3 = backdrop_color(in.position.xy, cloud, close_light, cloud_sampler, color_memory, lut, star_atlas, star_halos, star_halos_b, star_halos_c, cloud_tone, cloud_tile_a, cloud_tile_b, cloud_tile_c, tile_sampler);
    return fs_cloud_backdrop_gammaOutput { _e3 };
}
