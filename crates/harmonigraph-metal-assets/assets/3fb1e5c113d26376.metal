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
constant bool STAR_SPLIT = true;

metal::int2 atlas_texel(
    int index
) {
    return metal::int2(index & 2047, index >> STAR_ATLAS_SHIFT);
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

StarSlice star_slice(
    uint k,
    constant Settings& settings
) {
    StarSlice _e5 = settings.stars.star_slices.inner[metal::min(unsigned(k), 4u)];
    return _e5;
}

metal::float4 star_texel(
    StarSlice s,
    metal::float2 f,
    int index_1,
    bool halo,
    constant Settings& settings,
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
    metal::float4 _e26 = star_geometry(settings);
    float reach = _e26.y * s.cell;
    if (dist >= (halo ? (STAR_HALO_REACH * s.cell) : reach)) {
        return metal::float4(0.0);
    }
    metal::float3 colour = static_cast<metal::float3>(metal::uint3(t.z >> 20u, t.z >> 10u, t.z) & metal::uint3(1023u)) / metal::float3(1023.0);
    metal::float2 shape = float2(as_type<half2>(t.w));
    float d = dist * shape.x;
    float gaussian = metal::exp((-0.5 * d) * d);
    metal::float4 _e60 = star_geometry(settings);
    float core = gaussian * (1.0 - metal::smoothstep(_e60.z * reach, reach, dist));
    cover = core;
    if (halo) {
        full = gaussian;
        if (s.fringe > 0.0) {
            float _e72 = full;
            full = _e72 + (s.fringe * metal::exp(-0.4 * d));
        }
        float outer = STAR_HALO_REACH * s.cell;
        float _e82 = full;
        full = metal::min(_e82, 1.0) * (1.0 - metal::smoothstep(STAR_HALO_FADE * outer, outer, dist));
        float _e91 = full;
        cover = metal::max(_e91 - core, 0.0);
    }
    float _e95 = cover;
    cover = _e95 * shape.y;
    float _e98 = cover;
    float _e100 = cover;
    return metal::float4(colour * _e98, _e100);
}

StarHaloSample star_halo_sample(
    uint k_1,
    constant Settings& settings
) {
    StarHaloSample _e5 = settings.stars.star_halo_samples.inner[metal::min(unsigned(k_1), 4u)];
    return _e5;
}

metal::float4 star_halo_at(
    metal::float2 pt,
    uint k_2,
    constant Settings& settings,
    metal::sampler cloud_sampler,
    metal::texture2d_array<float, metal::access::sample> star_halos,
    metal::texture2d_array<float, metal::access::sample> star_halos_b,
    metal::texture2d_array<float, metal::access::sample> star_halos_c
) {
    StarHaloSample _e2 = star_halo_sample(k_2, settings);
    metal::float2 _e3 = star_size(settings);
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

metal::float4 star_far_texel(
    StarSlice s_1,
    metal::float2 f_1,
    int index_2,
    constant Settings& settings,
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
    metal::float4 _e21 = star_geometry(settings);
    float radius = 1.0 - (_e21.x * 0.5);
    float outer_1 = radius * s_1.cell;
    if (dist_1 >= outer_1) {
        return metal::float4(0.0);
    }
    metal::float3 colour_1 = static_cast<metal::float3>(metal::uint3(t_1.z >> 20u, t_1.z >> 10u, t_1.z) & metal::uint3(1023u)) / metal::float3(1023.0);
    metal::float2 shape_1 = float2(as_type<half2>(t_1.w));
    float d_1 = dist_1 * shape_1.x;
    full_1 = metal::exp((-0.5 * d_1) * d_1);
    if (s_1.fringe > 0.0) {
        float _e59 = full_1;
        full_1 = _e59 + (s_1.fringe * metal::exp(-0.4 * d_1));
    }
    float _e66 = full_1;
    float cover_1 = (metal::min(_e66, 1.0) * (1.0 - metal::smoothstep((radius - 0.15) * s_1.cell, outer_1, dist_1))) * shape_1.y;
    return metal::float4(colour_1 * cover_1, cover_1);
}

metal::int2 naga_f2i32(metal::float2 value) {
    return static_cast<metal::int2>(metal::clamp(value, -2147483600.0, 2147483500.0));
}

metal::float4 star_far_gather(
    StarSlice s_2,
    metal::float2 r,
    constant Settings& settings,
    metal::texture2d<uint, metal::access::sample> star_atlas
) {
    metal::float4 result_1 = metal::float4(0.0);
    metal::float2 o = metal::floor(r - metal::float2(0.5));
    metal::float2 f_2 = r - o;
    metal::int2 local_3 = as_type<metal::int2>(as_type<metal::uint2>(as_type<metal::int2>(as_type<metal::uint2>(naga_f2i32(o)) - as_type<metal::uint2>(naga_f2i32(metal::floor(s_2.offset))))) - as_type<metal::uint2>(s_2.origin));
    int index_3 = as_type<int>(as_type<uint>(as_type<int>(as_type<uint>(s_2.base) + as_type<uint>(as_type<int>(as_type<uint>(local_3.y) * as_type<uint>(s_2.grid.x))))) + as_type<uint>(local_3.x));
    metal::float4 _e25 = result_1;
    metal::float4 _e26 = star_far_texel(s_2, f_2, index_3, settings, star_atlas);
    result_1 = _e25 + _e26;
    metal::float4 _e28 = result_1;
    metal::float4 _e35 = star_far_texel(s_2, f_2 - metal::float2(1.0, 0.0), as_type<int>(as_type<uint>(index_3) + as_type<uint>(1)), settings, star_atlas);
    result_1 = _e28 + _e35;
    metal::float4 _e37 = result_1;
    metal::float4 _e45 = star_far_texel(s_2, f_2 - metal::float2(0.0, 1.0), as_type<int>(as_type<uint>(index_3) + as_type<uint>(s_2.grid.x)), settings, star_atlas);
    result_1 = _e37 + _e45;
    metal::float4 _e47 = result_1;
    metal::float4 _e57 = star_far_texel(s_2, f_2 - metal::float2(1.0, 1.0), as_type<int>(as_type<uint>(as_type<int>(as_type<uint>(index_3) + as_type<uint>(s_2.grid.x))) + as_type<uint>(1)), settings, star_atlas);
    result_1 = _e47 + _e57;
    metal::float4 _e59 = result_1;
    return _e59;
}

metal::float4 star_far(
    constant Settings& settings
) {
    metal::float4 _e3 = settings.stars.star_far;
    return _e3;
}

metal::float4 star_layers(
    metal::float2 pt_1,
    uint first,
    uint last,
    metal::float4 under,
    constant Settings& settings,
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
    bool local_2 = {};
    out = under;
    metal::float2 _e7 = star_size(settings);
    metal::float2 _e12 = star_size(settings);
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
            StarSlice _e20 = star_slice(_e19, settings);
            metal::float2 r_1 = (sp / metal::float2(_e20.cell)) - metal::fract(_e20.offset);
            metal::float2 o_1 = metal::floor(r_1);
            metal::float2 f_3 = r_1 - o_1;
            metal::int2 local_4 = as_type<metal::int2>(as_type<metal::uint2>(as_type<metal::int2>(as_type<metal::uint2>(naga_f2i32(o_1)) - as_type<metal::uint2>(naga_f2i32(metal::floor(_e20.offset))))) - as_type<metal::uint2>(_e20.origin));
            int index_4 = as_type<int>(as_type<uint>(as_type<int>(as_type<uint>(_e20.base) + as_type<uint>(as_type<int>(as_type<uint>(local_4.y) * as_type<uint>(_e20.grid.x))))) + as_type<uint>(local_4.x));
            slice = metal::float4(0.0);
            metal::float4 _e47 = star_far(settings);
            if (_e47.z > 0.0) {
                uint _e53 = k_3;
                local = _e53 < STAR_FAR_LAYERS;
            } else {
                local = false;
            }
            bool _e57 = local;
            if (_e57) {
                metal::float4 _e58 = star_far_gather(_e20, r_1, settings, star_atlas);
                slice = _e58;
            } else {
                metal::float4 _e60 = star_texel(_e20, f_3, index_4, false, settings, star_atlas);
                slice = _e60;
                metal::float4 _e61 = slice;
                uint _e62 = k_3;
                metal::float4 _e63 = star_halo_at(pt_1, _e62, settings, cloud_sampler, star_halos, star_halos_b, star_halos_c);
                slice = _e61 + _e63;
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
            metal::float4 _e91 = star_geometry(settings);
            if (_e91.w > 0.0) {
                uint _e97 = k_3;
                local_1 = _e97 < STAR_FAR_LAYERS;
            } else {
                local_1 = false;
            }
            bool _e101 = local_1;
            if (_e101) {
                float _e102 = far_gap;
                float _e104 = slice.w;
                far_gap = _e102 * (1.0 - metal::min(_e104, 1.0));
                uint _e110 = k_3;
                if ((_e110 + 1u) == STAR_FAR_LAYERS) {
                    local_2 = first == 0u;
                } else {
                    local_2 = false;
                }
                bool _e120 = local_2;
                if (_e120) {
                    metal::float4 _e121 = star_geometry(settings);
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

float star_ppp(
    constant Settings& settings
) {
    float _e3 = settings.stars.ppp;
    return _e3;
}

metal::float4 star_near_color(
    metal::float2 pt_2,
    constant Settings& settings,
    metal::sampler cloud_sampler,
    metal::texture2d<uint, metal::access::sample> star_atlas,
    metal::texture2d_array<float, metal::access::sample> star_halos,
    metal::texture2d_array<float, metal::access::sample> star_halos_b,
    metal::texture2d_array<float, metal::access::sample> star_halos_c,
    metal::texture2d<float, metal::access::sample> cloud_tone
) {
    metal::float4 far = metal::float4(0.0);
    metal::float4 _e4 = star_far(settings);
    if (_e4.z > 0.0) {
        metal::float2 _e10 = star_size(settings);
        metal::float4 _e13 = cloud_tone.sample(cloud_sampler, pt_2 / _e10, metal::level(0.0));
        far = _e13;
    } else {
        float _e15 = star_ppp(settings);
        uint clamped_lod_e19 = metal::min(uint(0), cloud_tone.get_num_mip_levels() - 1);
        metal::float4 _e19 = cloud_tone.read(metal::min(metal::uint2(naga_f2i32(pt_2 * _e15)), metal::uint2(cloud_tone.get_width(clamped_lod_e19), cloud_tone.get_height(clamped_lod_e19)) - 1), clamped_lod_e19);
        far = _e19;
    }
    metal::float4 _e22 = far;
    metal::float4 _e23 = star_layers(pt_2, STAR_FAR_LAYERS, STAR_SLICES, _e22, settings, cloud_sampler, star_atlas, star_halos, star_halos_b, star_halos_c);
    return _e23;
}

metal::float4 star_near(
    constant Settings& settings
) {
    metal::float4 _e3 = settings.stars.star_near;
    return _e3;
}

metal::float4 star_floor(
) {
    return metal::float4(0.0);
}

metal::float4 star_color(
    metal::float2 pt_3,
    constant Settings& settings,
    metal::sampler cloud_sampler,
    metal::texture2d<uint, metal::access::sample> star_atlas,
    metal::texture2d_array<float, metal::access::sample> star_halos,
    metal::texture2d_array<float, metal::access::sample> star_halos_b,
    metal::texture2d_array<float, metal::access::sample> star_halos_c,
    metal::texture2d<float, metal::access::sample> cloud_tone
) {
    if (STAR_SPLIT) {
        metal::float4 _e2 = star_near(settings);
        if (_e2.x > 0.0) {
            metal::float2 _e8 = star_size(settings);
            metal::float4 _e11 = cloud_tone.sample(cloud_sampler, pt_3 / _e8, metal::level(0.0));
            return _e11;
        }
        metal::float4 _e12 = star_near_color(pt_3, settings, cloud_sampler, star_atlas, star_halos, star_halos_b, star_halos_c, cloud_tone);
        return _e12;
    }
    metal::float4 _e15 = star_floor();
    metal::float4 _e16 = star_layers(pt_3, 0u, STAR_SLICES, _e15, settings, cloud_sampler, star_atlas, star_halos, star_halos_b, star_halos_c);
    return _e16;
}

struct fs_lattice_starsInput {
    uint layer [[user(loc0), flat]];
};
struct fs_lattice_starsOutput {
    metal::float4 member [[color(0)]];
};
fragment fs_lattice_starsOutput fs_lattice_stars(
  fs_lattice_starsInput varyings [[stage_in]]
, metal::float4 position [[position]]
, constant Settings& settings [[buffer(0)]]
, metal::texture2d<float, metal::access::sample> source [[texture(0)]]
, metal::sampler cloud_sampler [[sampler(0)]]
, metal::texture2d<uint, metal::access::sample> star_atlas [[texture(1)]]
, metal::texture2d_array<float, metal::access::sample> star_halos [[texture(2)]]
, metal::texture2d_array<float, metal::access::sample> star_halos_b [[texture(3)]]
, metal::texture2d_array<float, metal::access::sample> star_halos_c [[texture(4)]]
, metal::texture2d<float, metal::access::sample> cloud_tone [[texture(5)]]
) {
    const TileVertex in = { position, varyings.layer };
    metal::float4 result = {};
    metal::float2 _e8 = settings.stars.size;
    metal::float4 raw = source.sample(cloud_sampler, in.position.xy / _e8, metal::level(0.0));
    metal::float4 _e14 = star_color(in.position.xy, settings, cloud_sampler, star_atlas, star_halos, star_halos_b, star_halos_c, cloud_tone);
    result = _e14;
    float _e17 = result.x;
    float _e19 = result.y;
    float _e22 = result.z;
    float brightest = metal::max(metal::max(_e17, _e19), _e22);
    float _e26 = result.w;
    result.w = metal::max(brightest, metal::min(_e26, 1.0) * raw.w);
    metal::float4 _e32 = result;
    float _e34 = result.w;
    result = _e32 / metal::float4(metal::max(_e34, 1.0));
    metal::float4 _e39 = result;
    float _e42 = settings.depth;
    return fs_lattice_starsOutput { metal::mix(raw, _e39, _e42) };
}
