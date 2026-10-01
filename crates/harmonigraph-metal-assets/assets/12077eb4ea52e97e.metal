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
constant int STAR_HASH_PERIOD = 65536;
constant uint STAR_LIFE_PERIOD = 4096u;
constant float STAR_FADE = 0.2;
constant float STAR_LIFT = 0.18;
constant uint STAR_FAR_LAYERS = 3u;
constant bool STAR_SPLIT = false;

metal::int2 atlas_texel(
    int index
) {
    return metal::int2(index & 2047, index >> STAR_ATLAS_SHIFT);
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

float star_profile(
    StarSlice s,
    float t
) {
    float v = {};
    if (t >= 1.0) {
        return 0.0;
    }
    float q = t / s.core;
    v = metal::exp((-2.0 * q) * q);
    if (s.glow > 0.0) {
        float _e15 = v;
        v = _e15 + (s.glow * metal::pow(1.0 - t, s.falloff));
    }
    float _e23 = v;
    return metal::min(_e23, 1.0) * (1.0 - metal::smoothstep(0.75, 1.0, t));
}

metal::float4 star_geometry(
    constant Settings& settings
) {
    metal::float4 _e3 = settings.stars.star_geometry;
    return _e3;
}

metal::float4 star_texel(
    StarSlice s_1,
    metal::float2 f,
    int index_1,
    bool halo,
    constant Settings& settings,
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
    metal::float4 _e61 = star_geometry(settings);
    float inner = _e41 * (1.0 - metal::smoothstep(_e61.z * reach, reach, dist));
    float cover = (halo ? metal::max(_e41 - inner, 0.0) : inner) * shape.y;
    return metal::float4(colour * cover, cover);
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
    metal::float2 r,
    metal::texture2d<uint, metal::access::sample> star_atlas
) {
    metal::float4 result_1 = metal::float4(0.0);
    metal::float2 o = metal::floor(r - metal::float2(0.5));
    metal::float2 f_2 = r - o;
    metal::int2 local_3 = as_type<metal::int2>(as_type<metal::uint2>(as_type<metal::int2>(as_type<metal::uint2>(naga_f2i32(o)) - as_type<metal::uint2>(naga_f2i32(metal::floor(s_3.offset))))) - as_type<metal::uint2>(s_3.origin));
    int index_3 = as_type<int>(as_type<uint>(as_type<int>(as_type<uint>(s_3.base) + as_type<uint>(as_type<int>(as_type<uint>(local_3.y) * as_type<uint>(s_3.grid.x))))) + as_type<uint>(local_3.x));
    metal::float4 _e25 = result_1;
    metal::float4 _e26 = star_far_texel(s_3, f_2, index_3, star_atlas);
    result_1 = _e25 + _e26;
    metal::float4 _e28 = result_1;
    metal::float4 _e35 = star_far_texel(s_3, f_2 - metal::float2(1.0, 0.0), as_type<int>(as_type<uint>(index_3) + as_type<uint>(1)), star_atlas);
    result_1 = _e28 + _e35;
    metal::float4 _e37 = result_1;
    metal::float4 _e45 = star_far_texel(s_3, f_2 - metal::float2(0.0, 1.0), as_type<int>(as_type<uint>(index_3) + as_type<uint>(s_3.grid.x)), star_atlas);
    result_1 = _e37 + _e45;
    metal::float4 _e47 = result_1;
    metal::float4 _e57 = star_far_texel(s_3, f_2 - metal::float2(1.0, 1.0), as_type<int>(as_type<uint>(as_type<int>(as_type<uint>(index_3) + as_type<uint>(s_3.grid.x))) + as_type<uint>(1)), star_atlas);
    result_1 = _e47 + _e57;
    metal::float4 _e59 = result_1;
    return _e59;
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
            uint _e145 = k_3;
            k_3 = _e145 + 1u;
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
            if (_e20.gather == 2u) {
                metal::float4 _e50 = star_far_gather(_e20, r_1, star_atlas);
                slice = _e50;
            } else {
                if (_e20.gather == 1u) {
                    metal::float4 _e54 = star_far_texel(_e20, f_3, index_4, star_atlas);
                    slice = _e54;
                } else {
                    if (_e20.gather == 3u) {
                        metal::float4 _e59 = star_texel(_e20, f_3, index_4, false, settings, star_atlas);
                        slice = _e59;
                        metal::float4 _e60 = slice;
                        uint _e61 = k_3;
                        metal::float4 _e62 = star_halo_at(pt_1, _e61, settings, cloud_sampler, star_halos, star_halos_b, star_halos_c);
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
            }
            metal::float4 _e90 = star_geometry(settings);
            if (_e90.w > 0.0) {
                uint _e96 = k_3;
                local_1 = _e96 < STAR_FAR_LAYERS;
            } else {
                local_1 = false;
            }
            bool _e100 = local_1;
            if (_e100) {
                float _e101 = far_gap;
                float _e103 = slice.w;
                far_gap = _e101 * (1.0 - metal::min(_e103, 1.0));
                uint _e109 = k_3;
                if ((_e109 + 1u) == STAR_FAR_LAYERS) {
                    local_2 = first == 0u;
                } else {
                    local_2 = false;
                }
                bool _e119 = local_2;
                if (_e119) {
                    metal::float4 _e120 = star_geometry(settings);
                    float amount = _e120.w * 2.0;
                    float gentle = metal::min(amount, 1.0);
                    float strong = metal::max(amount - 1.0, 0.0);
                    float _e130 = far_gap;
                    float _e134 = far_gap;
                    float _e136 = far_gap;
                    float gain = (1.0 + (gentle * _e130)) * (1.0 + ((strong * _e134) * _e136));
                    metal::float4 _e141 = out;
                    out = under + ((_e141 - under) * gain);
                }
            }
        }
    }
    metal::float4 _e148 = out;
    return _e148;
}

metal::float4 star_far(
    constant Settings& settings
) {
    metal::float4 _e3 = settings.stars.star_far;
    return _e3;
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
