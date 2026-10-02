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
    float solid;
    float ramp;
    float bend;
    int base;
    metal::int2 origin;
    metal::int2 grid;
    float width;
    float inner;
    float gain;
    uint gather;
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
    float star_size_variation;
    metal::float4 star_far;
    metal::float4 star_near;
    type_8 star_slices;
    type_9 star_halo_samples;
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
constant float STAR_INNER_FADE = 0.7;
constant uint STAR_FAR_LAYERS = 3u;

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
    if (t >= 1.0) {
        return 0.0;
    }
    float u = metal::saturate((t - s.solid) * s.ramp);
    float x = (u * u) * (3.0 - (2.0 * u));
    return (1.0 - x) / (1.0 + (s.bend * x));
}

metal::float4 star_texel(
    StarSlice s_1,
    metal::float2 f,
    int index_1,
    bool halo,
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
    float inner = _e41 * (1.0 - metal::smoothstep(STAR_INNER_FADE * reach, reach, dist));
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
    metal::float2 pt_1,
    uint k_2,
    constant Settings& settings,
    metal::sampler cloud_sampler,
    metal::texture2d_array<float, metal::access::sample> star_halos,
    metal::texture2d_array<float, metal::access::sample> star_halos_b,
    metal::texture2d_array<float, metal::access::sample> star_halos_c
) {
    StarHaloSample _e2 = star_halo_sample(k_2, settings);
    metal::float2 _e3 = star_size(settings);
    metal::float2 uv = pt_1 / _e3;
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
    metal::float4 result = metal::float4(0.0);
    metal::float2 o = metal::floor(r - metal::float2(0.5));
    metal::float2 f_2 = r - o;
    metal::int2 local_1 = as_type<metal::int2>(as_type<metal::uint2>(as_type<metal::int2>(as_type<metal::uint2>(naga_f2i32(o)) - as_type<metal::uint2>(naga_f2i32(metal::floor(s_3.offset))))) - as_type<metal::uint2>(s_3.origin));
    int index_3 = as_type<int>(as_type<uint>(as_type<int>(as_type<uint>(s_3.base) + as_type<uint>(as_type<int>(as_type<uint>(local_1.y) * as_type<uint>(s_3.grid.x))))) + as_type<uint>(local_1.x));
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
    metal::float2 pt_2,
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
    uint k_3 = {};
    metal::float4 slice = {};
    out = under;
    metal::float2 _e5 = star_size(settings);
    metal::float2 _e10 = star_size(settings);
    metal::float2 sp = (pt_2 - (_e5 * 0.5)) * (STAR_PANE / _e10.y);
    k_3 = first;
    uint2 loop_bound = uint2(4294967295u);
    bool loop_init = true;
    while(true) {
        if (metal::all(loop_bound == uint2(0u))) { break; }
        loop_bound -= uint2(loop_bound.y == 0u, 1u);
        if (!loop_init) {
            uint _e88 = k_3;
            k_3 = _e88 + 1u;
        }
        loop_init = false;
        uint _e15 = k_3;
        if (_e15 < last) {
        } else {
            break;
        }
        {
            uint _e17 = k_3;
            StarSlice _e18 = star_slice(_e17, settings);
            metal::float2 r_1 = (sp / metal::float2(_e18.cell)) - metal::fract(_e18.offset);
            metal::float2 o_1 = metal::floor(r_1);
            metal::float2 f_3 = r_1 - o_1;
            metal::int2 local_2 = as_type<metal::int2>(as_type<metal::uint2>(as_type<metal::int2>(as_type<metal::uint2>(naga_f2i32(o_1)) - as_type<metal::uint2>(naga_f2i32(metal::floor(_e18.offset))))) - as_type<metal::uint2>(_e18.origin));
            int index_4 = as_type<int>(as_type<uint>(as_type<int>(as_type<uint>(_e18.base) + as_type<uint>(as_type<int>(as_type<uint>(local_2.y) * as_type<uint>(_e18.grid.x))))) + as_type<uint>(local_2.x));
            slice = metal::float4(0.0);
            if (_e18.gather == 2u) {
                metal::float4 _e48 = star_far_gather(_e18, r_1, star_atlas);
                slice = _e48;
            } else {
                if (_e18.gather == 1u) {
                    metal::float4 _e52 = star_far_texel(_e18, f_3, index_4, star_atlas);
                    slice = _e52;
                } else {
                    if (_e18.gather == 3u) {
                        metal::float4 _e57 = star_texel(_e18, f_3, index_4, false, star_atlas);
                        slice = _e57;
                        metal::float4 _e58 = slice;
                        uint _e59 = k_3;
                        metal::float4 _e60 = star_halo_at(pt_2, _e59, settings, cloud_sampler, star_halos, star_halos_b, star_halos_c);
                        slice = _e58 + _e60;
                    }
                }
            }
            float _e63 = slice.w;
            if (_e63 > 0.0) {
                float _e67 = slice.w;
                float cover_2 = metal::min(_e67, 1.0);
                metal::float4 _e70 = out;
                metal::float4 _e72 = slice;
                float _e75 = slice.w;
                float _e80 = out.w;
                float _e82 = out.w;
                out = metal::float4(metal::mix(_e70.xyz, _e72.xyz / metal::float3(_e75), cover_2), _e80 + ((1.0 - _e82) * cover_2));
            }
        }
    }
    metal::float4 _e91 = out;
    return _e91;
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

metal::float4 star_floor(
) {
    return metal::float4(0.0);
}

metal::float2 star_origin(
    constant Settings& settings
) {
    metal::float2 _e3 = settings.stars.origin;
    return _e3;
}

struct fs_star_farInput {
    uint layer [[user(loc0), flat]];
};
struct fs_star_farOutput {
    metal::float4 member [[color(0)]];
};
fragment fs_star_farOutput fs_star_far(
  fs_star_farInput varyings [[stage_in]]
, metal::float4 position [[position]]
, constant Settings& settings [[buffer(0)]]
, metal::sampler cloud_sampler [[sampler(0)]]
, metal::texture2d<uint, metal::access::sample> star_atlas [[texture(1)]]
, metal::texture2d_array<float, metal::access::sample> star_halos [[texture(2)]]
, metal::texture2d_array<float, metal::access::sample> star_halos_b [[texture(3)]]
, metal::texture2d_array<float, metal::access::sample> star_halos_c [[texture(4)]]
) {
    const TileVertex in = { position, varyings.layer };
    metal::float2 pt = {};
    metal::float2 _e3 = star_origin(settings);
    float _e4 = star_ppp(settings);
    metal::float2 position_1 = in.position.xy + metal::rint(_e3 * _e4);
    float _e8 = star_ppp(settings);
    metal::float2 _e11 = star_origin(settings);
    pt = (position_1 / metal::float2(_e8)) - _e11;
    metal::float4 _e14 = star_far(settings);
    if (_e14.z > 0.0) {
        metal::float4 _e20 = star_far(settings);
        metal::float2 _e23 = star_size(settings);
        pt = (in.position.xy / _e20.xy) * _e23;
    }
    metal::float2 _e25 = pt;
    metal::float4 _e28 = star_floor();
    metal::float4 _e29 = star_layers(_e25, 0u, STAR_FAR_LAYERS, _e28, settings, cloud_sampler, star_atlas, star_halos, star_halos_b, star_halos_c);
    return fs_star_farOutput { _e29 };
}
