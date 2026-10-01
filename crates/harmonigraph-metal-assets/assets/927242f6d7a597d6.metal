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
    float width;
    float core;
    float glow;
    float gain;
    uint gather;
    uint pad0_;
    uint pad1_;
    uint pad2_;
};
struct type_9 {
    StarSlice inner[5];
};
struct type_10 {
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
    float star_size_variation;
    uint star_pad0_;
    uint star_pad1_;
    uint star_pad2_;
    metal::float4 star_far;
    metal::float4 star_near;
    metal::float4 star_geometry;
    type_9 star_slices;
    uint memory_enabled;
    uint memory_valid;
    float pickup_alpha;
    float release_alpha;
    metal::int2 memory_shift;
    metal::float2 memory_fraction;
    float previous_life;
    float wash_randomness;
    metal::float2 memory_extent;
    type_9 previous_slices;
    type_10 star_halo_samples;
    metal::float4 velvet;
    metal::float4 velvet_size;
};
struct TileVertex {
    metal::float4 position;
    metal::float2 fraction;
    uint layer;
    char _pad3[4];
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
constant float STAR_HALO_FADE = 0.7;
constant int STAR_HASH_PERIOD = 65536;
constant uint STAR_LIFE_PERIOD = 4096u;
constant float STAR_FADE = 0.2;
constant float STAR_LIFT = 0.18;
constant uint STAR_FAR_LAYERS = 3u;
constant float CLOUD_UNITS = 10.0;
constant float SCALE_CELLS = 2.7272727;

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

StarSlice star_slice(
    uint k,
    constant Cloud& cloud
) {
    StarSlice _e4 = cloud.star_slices.inner[metal::min(unsigned(k), 4u)];
    return _e4;
}

metal::float4 star_geometry(
    constant Cloud& cloud
) {
    metal::float4 _e2 = cloud.star_geometry;
    return _e2;
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
    float reach = s.core * s.cell;
    float outer = s.glow * s.cell;
    if (dist >= (halo ? outer : reach)) {
        return metal::float4(0.0);
    }
    metal::float3 colour = static_cast<metal::float3>(metal::uint3(t.z >> 20u, t.z >> 10u, t.z) & metal::uint3(1023u)) / metal::float3(1023.0);
    metal::float2 shape = float2(as_type<half2>(t.w));
    float d = dist * shape.x;
    float gaussian = metal::exp((-0.5 * d) * d);
    metal::float4 _e59 = star_geometry(cloud);
    float core = gaussian * (1.0 - metal::smoothstep(_e59.z * reach, reach, dist));
    cover = core;
    if (halo) {
        full = gaussian;
        if (s.fringe > 0.0) {
            float _e71 = full;
            full = _e71 + (s.fringe * metal::exp(-0.4 * d));
        }
        float _e78 = full;
        full = metal::min(_e78, 1.0) * (1.0 - metal::smoothstep(STAR_HALO_FADE * outer, outer, dist));
        float _e87 = full;
        cover = metal::max(_e87 - core, 0.0);
    }
    float _e91 = cover;
    cover = _e91 * shape.y;
    float _e94 = cover;
    float _e96 = cover;
    return metal::float4(colour * _e94, _e96);
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
    metal::float2 uv = pt / _e3;
    switch(_e2.group) {
        case 0u: {
            metal::float4 _e11 = star_halos.sample(cloud_sampler, uv, static_cast<int>(_e2.layer), metal::level(0.0));
            return _e11;
        }
        case 1u: {
            metal::float4 _e17 = star_halos_b.sample(cloud_sampler, uv, static_cast<int>(_e2.layer), metal::level(0.0));
            return _e17;
        }
        default: {
            metal::float4 _e23 = star_halos_c.sample(cloud_sampler, uv, static_cast<int>(_e2.layer), metal::level(0.0));
            return _e23;
        }
    }
}

metal::float4 star_whole_texel(
    StarSlice s_1,
    metal::float2 f_1,
    int index_2,
    float radius,
    metal::texture2d<uint, metal::access::sample> star_atlas
) {
    float full_1 = {};
    metal::int2 _e5 = atlas_texel(index_2);
    uint clamped_lod_e7 = metal::min(uint(0), star_atlas.get_num_mip_levels() - 1);
    metal::uint4 t_1 = star_atlas.read(metal::min(metal::uint2(_e5), metal::uint2(star_atlas.get_width(clamped_lod_e7), star_atlas.get_height(clamped_lod_e7)) - 1), clamped_lod_e7);
    if (t_1.w == 0u) {
        return metal::float4(0.0);
    }
    float dist_1 = metal::length(f_1 - metal::float2(as_type<float>(t_1.x), as_type<float>(t_1.y))) * s_1.cell;
    float outer_1 = radius * s_1.cell;
    if (dist_1 >= outer_1) {
        return metal::float4(0.0);
    }
    metal::float3 colour_1 = static_cast<metal::float3>(metal::uint3(t_1.z >> 20u, t_1.z >> 10u, t_1.z) & metal::uint3(1023u)) / metal::float3(1023.0);
    metal::float2 shape_1 = float2(as_type<half2>(t_1.w));
    float d_1 = dist_1 * shape_1.x;
    full_1 = metal::exp((-0.5 * d_1) * d_1);
    if (s_1.fringe > 0.0) {
        float _e54 = full_1;
        full_1 = _e54 + (s_1.fringe * metal::exp(-0.4 * d_1));
    }
    float _e61 = full_1;
    float cover_1 = (metal::min(_e61, 1.0) * (1.0 - metal::smoothstep((radius - 0.15) * s_1.cell, outer_1, dist_1))) * shape_1.y;
    return metal::float4(colour_1 * cover_1, cover_1);
}

metal::float4 star_far_texel(
    StarSlice s_2,
    metal::float2 f_2,
    int index_3,
    metal::texture2d<uint, metal::access::sample> star_atlas
) {
    metal::float4 _e4 = star_whole_texel(s_2, f_2, index_3, s_2.glow, star_atlas);
    return _e4;
}

metal::int2 naga_f2i32(metal::float2 value) {
    return static_cast<metal::int2>(metal::clamp(value, -2147483600.0, 2147483500.0));
}

metal::float4 star_far_gather(
    StarSlice s_3,
    metal::float2 r,
    metal::texture2d<uint, metal::access::sample> star_atlas
) {
    metal::float4 result = metal::float4(0.0);
    metal::float2 o = metal::floor(r - metal::float2(0.5));
    metal::float2 f_3 = r - o;
    metal::int2 local_2 = as_type<metal::int2>(as_type<metal::uint2>(as_type<metal::int2>(as_type<metal::uint2>(naga_f2i32(o)) - as_type<metal::uint2>(naga_f2i32(metal::floor(s_3.offset))))) - as_type<metal::uint2>(s_3.origin));
    int index_4 = as_type<int>(as_type<uint>(as_type<int>(as_type<uint>(s_3.base) + as_type<uint>(as_type<int>(as_type<uint>(local_2.y) * as_type<uint>(s_3.grid.x))))) + as_type<uint>(local_2.x));
    metal::float4 _e25 = result;
    metal::float4 _e26 = star_far_texel(s_3, f_3, index_4, star_atlas);
    result = _e25 + _e26;
    metal::float4 _e28 = result;
    metal::float4 _e35 = star_far_texel(s_3, f_3 - metal::float2(1.0, 0.0), as_type<int>(as_type<uint>(index_4) + as_type<uint>(1)), star_atlas);
    result = _e28 + _e35;
    metal::float4 _e37 = result;
    metal::float4 _e45 = star_far_texel(s_3, f_3 - metal::float2(0.0, 1.0), as_type<int>(as_type<uint>(index_4) + as_type<uint>(s_3.grid.x)), star_atlas);
    result = _e37 + _e45;
    metal::float4 _e47 = result;
    metal::float4 _e57 = star_far_texel(s_3, f_3 - metal::float2(1.0, 1.0), as_type<int>(as_type<uint>(as_type<int>(as_type<uint>(index_4) + as_type<uint>(s_3.grid.x))) + as_type<uint>(1)), star_atlas);
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
    bool local = {};
    bool local_1 = {};
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
            uint _e146 = k_3;
            k_3 = _e146 + 1u;
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
            metal::float2 r_1 = (sp / metal::float2(_e20.cell)) - metal::fract(_e20.offset);
            metal::float2 o_1 = metal::floor(r_1);
            metal::float2 f_4 = r_1 - o_1;
            metal::int2 local_3 = as_type<metal::int2>(as_type<metal::uint2>(as_type<metal::int2>(as_type<metal::uint2>(naga_f2i32(o_1)) - as_type<metal::uint2>(naga_f2i32(metal::floor(_e20.offset))))) - as_type<metal::uint2>(_e20.origin));
            int index_5 = as_type<int>(as_type<uint>(as_type<int>(as_type<uint>(_e20.base) + as_type<uint>(as_type<int>(as_type<uint>(local_3.y) * as_type<uint>(_e20.grid.x))))) + as_type<uint>(local_3.x));
            slice = metal::float4(0.0);
            if (_e20.gather == 2u) {
                metal::float4 _e50 = star_far_gather(_e20, r_1, star_atlas);
                slice = _e50;
            } else {
                if (_e20.gather == 1u) {
                    metal::float4 _e55 = star_whole_texel(_e20, f_4, index_5, _e20.core, star_atlas);
                    slice = _e55;
                } else {
                    if (_e20.gather == 3u) {
                        metal::float4 _e60 = star_texel(_e20, f_4, index_5, false, cloud, star_atlas);
                        slice = _e60;
                        metal::float4 _e61 = slice;
                        uint _e62 = k_3;
                        metal::float4 _e63 = star_halo_at(pt_1, _e62, cloud, cloud_sampler, star_halos, star_halos_b, star_halos_c);
                        slice = _e61 + _e63;
                    }
                }
            }
            float _e66 = slice.w;
            if (_e66 > 0.0) {
                float _e70 = slice.w;
                float cover_2 = metal::min(_e70, 1.0);
                metal::float4 _e73 = out;
                metal::float4 _e75 = slice;
                float _e78 = slice.w;
                float _e83 = out.w;
                float _e85 = out.w;
                out = metal::float4(metal::mix(_e73.xyz, _e75.xyz / metal::float3(_e78), cover_2), _e83 + ((1.0 - _e85) * cover_2));
            }
            metal::float4 _e91 = star_geometry(cloud);
            if (_e91.w > 0.0) {
                uint _e97 = k_3;
                local = _e97 < STAR_FAR_LAYERS;
            } else {
                local = false;
            }
            bool _e101 = local;
            if (_e101) {
                float _e102 = far_gap;
                float _e104 = slice.w;
                far_gap = _e102 * (1.0 - metal::min(_e104, 1.0));
                uint _e110 = k_3;
                if ((_e110 + 1u) == STAR_FAR_LAYERS) {
                    local_1 = first == 0u;
                } else {
                    local_1 = false;
                }
                bool _e120 = local_1;
                if (_e120) {
                    metal::float4 _e121 = star_geometry(cloud);
                    float amount = _e121.w * 2.0;
                    float gentle = metal::min(amount, 1.0);
                    float strong = metal::max(amount - 1.0, 0.0);
                    float _e131 = far_gap;
                    float _e135 = far_gap;
                    float _e137 = far_gap;
                    float gain = (1.0 + (gentle * _e131)) * (1.0 + ((strong * _e135) * _e137));
                    metal::float4 _e142 = out;
                    out = under + ((_e142 - under) * gain);
                }
            }
        }
    }
    metal::float4 _e149 = out;
    return _e149;
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

struct fs_star_nearInput {
    metal::float2 fraction [[user(loc0), center_perspective]];
    uint layer [[user(loc1), flat]];
};
struct fs_star_nearOutput {
    metal::float4 member [[color(0)]];
};
fragment fs_star_nearOutput fs_star_near(
  fs_star_nearInput varyings [[stage_in]]
, metal::float4 position [[position]]
, constant Cloud& cloud [[buffer(2)]]
, metal::sampler cloud_sampler [[sampler(0)]]
, metal::texture2d<uint, metal::access::sample> star_atlas [[texture(6)]]
, metal::texture2d_array<float, metal::access::sample> star_halos [[texture(8)]]
, metal::texture2d_array<float, metal::access::sample> star_halos_b [[texture(9)]]
, metal::texture2d_array<float, metal::access::sample> star_halos_c [[texture(10)]]
, metal::texture2d<float, metal::access::sample> cloud_tone [[texture(3)]]
) {
    const TileVertex in = { position, varyings.fraction, varyings.layer };
    metal::float4 _e3 = star_near(cloud);
    metal::float2 _e6 = star_size(cloud);
    metal::float2 pt_3 = (in.position.xy / _e3.xy) * _e6;
    metal::float4 _e8 = star_near_color(pt_3, cloud, cloud_sampler, star_atlas, star_halos, star_halos_b, star_halos_c, cloud_tone);
    return fs_star_nearOutput { _e8 };
}
