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
constant float STAR_HALO_FADE = 0.7;
constant int STAR_HASH_PERIOD = 65536;
constant uint STAR_LIFE_PERIOD = 4096u;
constant float STAR_FADE = 0.2;
constant float STAR_LIFT = 0.18;
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

metal::float4 star_geometry(
    constant Settings& settings
) {
    metal::float4 _e3 = settings.stars.star_geometry;
    return _e3;
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
    float reach = s.core * s.cell;
    float outer = s.glow * s.cell;
    if (dist >= (halo ? outer : reach)) {
        return metal::float4(0.0);
    }
    metal::float3 colour = static_cast<metal::float3>(metal::uint3(t.z >> 20u, t.z >> 10u, t.z) & metal::uint3(1023u)) / metal::float3(1023.0);
    metal::float2 shape = float2(as_type<half2>(t.w));
    float d = dist * shape.x;
    float gaussian = metal::exp((-0.5 * d) * d);
    metal::float4 _e59 = star_geometry(settings);
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
    float radius = s_1.glow;
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

metal::int2 naga_f2i32(metal::float2 value) {
    return static_cast<metal::int2>(metal::clamp(value, -2147483600.0, 2147483500.0));
}

metal::float4 star_far_gather(
    StarSlice s_2,
    metal::float2 r,
    metal::texture2d<uint, metal::access::sample> star_atlas
) {
    metal::float4 result = metal::float4(0.0);
    metal::float2 o = metal::floor(r - metal::float2(0.5));
    metal::float2 f_2 = r - o;
    metal::int2 local_2 = as_type<metal::int2>(as_type<metal::uint2>(as_type<metal::int2>(as_type<metal::uint2>(naga_f2i32(o)) - as_type<metal::uint2>(naga_f2i32(metal::floor(s_2.offset))))) - as_type<metal::uint2>(s_2.origin));
    int index_3 = as_type<int>(as_type<uint>(as_type<int>(as_type<uint>(s_2.base) + as_type<uint>(as_type<int>(as_type<uint>(local_2.y) * as_type<uint>(s_2.grid.x))))) + as_type<uint>(local_2.x));
    metal::float4 _e25 = result;
    metal::float4 _e26 = star_far_texel(s_2, f_2, index_3, star_atlas);
    result = _e25 + _e26;
    metal::float4 _e28 = result;
    metal::float4 _e35 = star_far_texel(s_2, f_2 - metal::float2(1.0, 0.0), as_type<int>(as_type<uint>(index_3) + as_type<uint>(1)), star_atlas);
    result = _e28 + _e35;
    metal::float4 _e37 = result;
    metal::float4 _e45 = star_far_texel(s_2, f_2 - metal::float2(0.0, 1.0), as_type<int>(as_type<uint>(index_3) + as_type<uint>(s_2.grid.x)), star_atlas);
    result = _e37 + _e45;
    metal::float4 _e47 = result;
    metal::float4 _e57 = star_far_texel(s_2, f_2 - metal::float2(1.0, 1.0), as_type<int>(as_type<uint>(as_type<int>(as_type<uint>(index_3) + as_type<uint>(s_2.grid.x))) + as_type<uint>(1)), star_atlas);
    result = _e47 + _e57;
    metal::float4 _e59 = result;
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
    bool local = {};
    bool local_1 = {};
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
            uint _e144 = k_3;
            k_3 = _e144 + 1u;
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
            metal::int2 local_3 = as_type<metal::int2>(as_type<metal::uint2>(as_type<metal::int2>(as_type<metal::uint2>(naga_f2i32(o_1)) - as_type<metal::uint2>(naga_f2i32(metal::floor(_e20.offset))))) - as_type<metal::uint2>(_e20.origin));
            int index_4 = as_type<int>(as_type<uint>(as_type<int>(as_type<uint>(_e20.base) + as_type<uint>(as_type<int>(as_type<uint>(local_3.y) * as_type<uint>(_e20.grid.x))))) + as_type<uint>(local_3.x));
            slice = metal::float4(0.0);
            if (_e20.gather == 2u) {
                metal::float4 _e50 = star_far_gather(_e20, r_1, star_atlas);
                slice = _e50;
            } else {
                if (_e20.gather != 0u) {
                    metal::float4 _e55 = star_texel(_e20, f_3, index_4, false, settings, star_atlas);
                    slice = _e55;
                    if (_e20.gather == 3u) {
                        metal::float4 _e59 = slice;
                        uint _e60 = k_3;
                        metal::float4 _e61 = star_halo_at(pt_1, _e60, settings, cloud_sampler, star_halos, star_halos_b, star_halos_c);
                        slice = _e59 + _e61;
                    }
                }
            }
            float _e64 = slice.w;
            if (_e64 > 0.0) {
                float _e68 = slice.w;
                float cover_2 = metal::min(_e68, 1.0);
                metal::float4 _e71 = out;
                metal::float4 _e73 = slice;
                float _e76 = slice.w;
                float _e81 = out.w;
                float _e83 = out.w;
                out = metal::float4(metal::mix(_e71.xyz, _e73.xyz / metal::float3(_e76), cover_2), _e81 + ((1.0 - _e83) * cover_2));
            }
            metal::float4 _e89 = star_geometry(settings);
            if (_e89.w > 0.0) {
                uint _e95 = k_3;
                local = _e95 < STAR_FAR_LAYERS;
            } else {
                local = false;
            }
            bool _e99 = local;
            if (_e99) {
                float _e100 = far_gap;
                float _e102 = slice.w;
                far_gap = _e100 * (1.0 - metal::min(_e102, 1.0));
                uint _e108 = k_3;
                if ((_e108 + 1u) == STAR_FAR_LAYERS) {
                    local_1 = first == 0u;
                } else {
                    local_1 = false;
                }
                bool _e118 = local_1;
                if (_e118) {
                    metal::float4 _e119 = star_geometry(settings);
                    float amount = _e119.w * 2.0;
                    float gentle = metal::min(amount, 1.0);
                    float strong = metal::max(amount - 1.0, 0.0);
                    float _e129 = far_gap;
                    float _e133 = far_gap;
                    float _e135 = far_gap;
                    float gain = (1.0 + (gentle * _e129)) * (1.0 + ((strong * _e133) * _e135));
                    metal::float4 _e140 = out;
                    out = under + ((_e140 - under) * gain);
                }
            }
        }
    }
    metal::float4 _e147 = out;
    return _e147;
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

struct fs_star_nearInput {
    uint layer [[user(loc0), flat]];
};
struct fs_star_nearOutput {
    metal::float4 member [[color(0)]];
};
fragment fs_star_nearOutput fs_star_near(
  fs_star_nearInput varyings [[stage_in]]
, metal::float4 position [[position]]
, constant Settings& settings [[buffer(0)]]
, metal::sampler cloud_sampler [[sampler(0)]]
, metal::texture2d<uint, metal::access::sample> star_atlas [[texture(1)]]
, metal::texture2d_array<float, metal::access::sample> star_halos [[texture(2)]]
, metal::texture2d_array<float, metal::access::sample> star_halos_b [[texture(3)]]
, metal::texture2d_array<float, metal::access::sample> star_halos_c [[texture(4)]]
, metal::texture2d<float, metal::access::sample> cloud_tone [[texture(5)]]
) {
    const TileVertex in = { position, varyings.layer };
    metal::float4 _e3 = star_near(settings);
    metal::float2 _e6 = star_size(settings);
    metal::float2 pt_3 = (in.position.xy / _e3.xy) * _e6;
    metal::float4 _e8 = star_near_color(pt_3, settings, cloud_sampler, star_atlas, star_halos, star_halos_b, star_halos_c, cloud_tone);
    return fs_star_nearOutput { _e8 };
}
