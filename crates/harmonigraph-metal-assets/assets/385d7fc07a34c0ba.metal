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
struct Settings {
    StarUniforms stars;
    float depth;
    float strength;
    float accumulation;
    float padding;
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

metal::int2 atlas_texel(
    int index
) {
    return metal::int2(index & 2047, index >> STAR_ATLAS_SHIFT);
}

StarUniforms star_settings(
    constant Settings& settings
) {
    StarUniforms _e2 = settings.stars;
    return _e2;
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
    StarUniforms _e26 = star_settings(settings);
    float reach = _e26.star_geometry.y * s.cell;
    if (dist >= (halo ? (STAR_HALO_REACH * s.cell) : reach)) {
        return metal::float4(0.0);
    }
    metal::float3 colour = static_cast<metal::float3>(metal::uint3(t.z >> 20u, t.z >> 10u, t.z) & metal::uint3(1023u)) / metal::float3(1023.0);
    metal::float2 shape = float2(as_type<half2>(t.w));
    float d = dist * shape.x;
    float gaussian = metal::exp((-0.5 * d) * d);
    StarUniforms _e61 = star_settings(settings);
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
    metal::float2 pt_1,
    uint k,
    constant Settings& settings,
    metal::sampler cloud_sampler,
    metal::texture2d_array<float, metal::access::sample> star_halos,
    metal::texture2d_array<float, metal::access::sample> star_halos_b,
    metal::texture2d_array<float, metal::access::sample> star_halos_c
) {
    StarUniforms _e2 = star_settings(settings);
    StarHaloSample sample = _e2.star_halo_samples.inner[metal::min(unsigned(k), 4u)];
    StarUniforms _e5 = star_settings(settings);
    metal::float2 uv = pt_1 / _e5.size;
    switch(_e2.star_halo_samples.inner[metal::min(unsigned(k), 4u)].group) {
        case 0u: {
            metal::float4 _e14 = star_halos.sample(cloud_sampler, uv, static_cast<int>(_e2.star_halo_samples.inner[metal::min(unsigned(k), 4u)].layer), metal::level(0.0));
            return _e14;
        }
        case 1u: {
            metal::float4 _e20 = star_halos_b.sample(cloud_sampler, uv, static_cast<int>(_e2.star_halo_samples.inner[metal::min(unsigned(k), 4u)].layer), metal::level(0.0));
            return _e20;
        }
        default: {
            metal::float4 _e26 = star_halos_c.sample(cloud_sampler, uv, static_cast<int>(_e2.star_halo_samples.inner[metal::min(unsigned(k), 4u)].layer), metal::level(0.0));
            return _e26;
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
    StarUniforms _e21 = star_settings(settings);
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
    metal::float2 r,
    constant Settings& settings,
    metal::texture2d<uint, metal::access::sample> star_atlas
) {
    metal::float4 result = metal::float4(0.0);
    metal::float2 o = metal::floor(r - metal::float2(0.5));
    metal::float2 f_2 = r - o;
    metal::int2 local_3 = as_type<metal::int2>(as_type<metal::uint2>(as_type<metal::int2>(as_type<metal::uint2>(naga_f2i32(o)) - as_type<metal::uint2>(naga_f2i32(metal::floor(s_2.offset))))) - as_type<metal::uint2>(s_2.origin));
    int index_3 = as_type<int>(as_type<uint>(as_type<int>(as_type<uint>(s_2.base) + as_type<uint>(as_type<int>(as_type<uint>(local_3.y) * as_type<uint>(s_2.grid.x))))) + as_type<uint>(local_3.x));
    metal::float4 _e25 = result;
    metal::float4 _e26 = star_far_texel(s_2, f_2, index_3, settings, star_atlas);
    result = _e25 + _e26;
    metal::float4 _e28 = result;
    metal::float4 _e35 = star_far_texel(s_2, f_2 - metal::float2(1.0, 0.0), as_type<int>(as_type<uint>(index_3) + as_type<uint>(1)), settings, star_atlas);
    result = _e28 + _e35;
    metal::float4 _e37 = result;
    metal::float4 _e45 = star_far_texel(s_2, f_2 - metal::float2(0.0, 1.0), as_type<int>(as_type<uint>(index_3) + as_type<uint>(s_2.grid.x)), settings, star_atlas);
    result = _e37 + _e45;
    metal::float4 _e47 = result;
    metal::float4 _e57 = star_far_texel(s_2, f_2 - metal::float2(1.0, 1.0), as_type<int>(as_type<uint>(as_type<int>(as_type<uint>(index_3) + as_type<uint>(s_2.grid.x))) + as_type<uint>(1)), settings, star_atlas);
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
    float far_gap = 1.0;
    uint k_1 = {};
    metal::float4 slice = {};
    bool local = {};
    bool local_1 = {};
    bool local_2 = {};
    out = under;
    StarUniforms _e7 = star_settings(settings);
    StarUniforms _e13 = star_settings(settings);
    metal::float2 sp = (pt_2 - (_e7.size * 0.5)) * (STAR_PANE / _e13.size.y);
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
            StarUniforms _e21 = star_settings(settings);
            uint _e23 = k_1;
            StarSlice s_3 = _e21.star_slices.inner[metal::min(unsigned(_e23), 4u)];
            metal::float2 r_1 = (sp / metal::float2(_e21.star_slices.inner[metal::min(unsigned(_e23), 4u)].cell)) - metal::fract(_e21.star_slices.inner[metal::min(unsigned(_e23), 4u)].offset);
            metal::float2 o_1 = metal::floor(r_1);
            metal::float2 f_3 = r_1 - o_1;
            metal::int2 local_4 = as_type<metal::int2>(as_type<metal::uint2>(as_type<metal::int2>(as_type<metal::uint2>(naga_f2i32(o_1)) - as_type<metal::uint2>(naga_f2i32(metal::floor(_e21.star_slices.inner[metal::min(unsigned(_e23), 4u)].offset))))) - as_type<metal::uint2>(_e21.star_slices.inner[metal::min(unsigned(_e23), 4u)].origin));
            int index_4 = as_type<int>(as_type<uint>(as_type<int>(as_type<uint>(_e21.star_slices.inner[metal::min(unsigned(_e23), 4u)].base) + as_type<uint>(as_type<int>(as_type<uint>(local_4.y) * as_type<uint>(_e21.star_slices.inner[metal::min(unsigned(_e23), 4u)].grid.x))))) + as_type<uint>(local_4.x));
            slice = metal::float4(0.0);
            StarUniforms _e51 = star_settings(settings);
            if (_e51.star_far.z > 0.0) {
                uint _e58 = k_1;
                local = _e58 < STAR_FAR_LAYERS;
            } else {
                local = false;
            }
            bool _e62 = local;
            if (_e62) {
                metal::float4 _e63 = star_far_gather(s_3, r_1, settings, star_atlas);
                slice = _e63;
            } else {
                metal::float4 _e65 = star_texel(s_3, f_3, index_4, false, settings, star_atlas);
                slice = _e65;
                metal::float4 _e66 = slice;
                uint _e67 = k_1;
                metal::float4 _e68 = star_halo_at(pt_2, _e67, settings, cloud_sampler, star_halos, star_halos_b, star_halos_c);
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
            StarUniforms _e96 = star_settings(settings);
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
                    StarUniforms _e127 = star_settings(settings);
                    float amount = _e127.star_geometry.w * 2.0;
                    float gentle = metal::min(amount, 1.0);
                    float strong = metal::max(amount - 1.0, 0.0);
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

metal::float4 star_floor(
) {
    return metal::float4(0.0);
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
    StarUniforms _e3 = star_settings(settings);
    StarUniforms _e5 = star_settings(settings);
    metal::float2 position_1 = in.position.xy + metal::rint(_e3.origin * _e5.ppp);
    StarUniforms _e10 = star_settings(settings);
    StarUniforms _e14 = star_settings(settings);
    pt = (position_1 / metal::float2(_e10.ppp)) - _e14.origin;
    StarUniforms _e18 = star_settings(settings);
    if (_e18.star_far.z > 0.0) {
        StarUniforms _e25 = star_settings(settings);
        StarUniforms _e29 = star_settings(settings);
        pt = (in.position.xy / _e25.star_far.xy) * _e29.size;
    }
    metal::float2 _e32 = pt;
    metal::float4 _e35 = star_floor();
    metal::float4 _e36 = star_layers(_e32, 0u, STAR_FAR_LAYERS, _e35, settings, cloud_sampler, star_atlas, star_halos, star_halos_b, star_halos_c);
    return fs_star_farOutput { _e36 };
}
