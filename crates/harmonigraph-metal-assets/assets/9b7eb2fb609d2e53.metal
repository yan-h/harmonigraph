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
struct StarHaloSample {
    metal::float2 size;
    uint group;
    uint layer;
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
    float memory_pad_a;
    metal::float2 memory_extent;
    type_9 previous_slices;
    type_10 star_halo_samples;
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
constant float CLOUD_UNITS = 10.0;
constant float SCALE_CELLS = 2.7272727;
constant uint STAR_SLICES = 5u;
constant float STAR_PANE = 540.0;
constant float STAR_HALO_REACH = 1.2;
constant float STAR_HALO_FADE = 0.7;
constant int STAR_ATLAS_WIDTH = 2048;
constant uint STAR_ATLAS_SHIFT = 11u;
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
    float _e29 = cloud.star_geometry.y;
    float reach = _e29 * s.cell;
    if (dist >= (halo ? (STAR_HALO_REACH * s.cell) : reach)) {
        return metal::float4(0.0);
    }
    metal::float3 colour = static_cast<metal::float3>(metal::uint3(t.z >> 20u, t.z >> 10u, t.z) & metal::uint3(1023u)) / metal::float3(1023.0);
    metal::float2 shape = float2(as_type<half2>(t.w));
    float d = dist * shape.x;
    float gaussian = metal::exp((-0.5 * d) * d);
    float _e65 = cloud.star_geometry.z;
    float core = gaussian * (1.0 - metal::smoothstep(_e65 * reach, reach, dist));
    cover = core;
    if (halo) {
        full = gaussian;
        if (s.fringe > 0.0) {
            float _e76 = full;
            full = _e76 + (s.fringe * metal::exp(-0.4 * d));
        }
        float outer = STAR_HALO_REACH * s.cell;
        float _e86 = full;
        full = metal::min(_e86, 1.0) * (1.0 - metal::smoothstep(STAR_HALO_FADE * outer, outer, dist));
        float _e95 = full;
        cover = metal::max(_e95 - core, 0.0);
    }
    float _e99 = cover;
    cover = _e99 * shape.y;
    float _e102 = cover;
    float _e104 = cover;
    return metal::float4(colour * _e102, _e104);
}

metal::float4 star_halo_at(
    metal::float2 pt,
    uint k,
    metal::sampler cloud_sampler,
    constant Cloud& cloud,
    metal::texture2d_array<float, metal::access::sample> star_halos,
    metal::texture2d_array<float, metal::access::sample> star_halos_b,
    metal::texture2d_array<float, metal::access::sample> star_halos_c
) {
    StarHaloSample sample = cloud.star_halo_samples.inner[metal::min(unsigned(k), 4u)];
    metal::float2 _e8 = cloud.size;
    metal::float2 uv = pt / _e8;
    switch(sample.group) {
        case 0u: {
            metal::float4 _e16 = star_halos.sample(cloud_sampler, uv, static_cast<int>(sample.layer), metal::level(0.0));
            return _e16;
        }
        case 1u: {
            metal::float4 _e22 = star_halos_b.sample(cloud_sampler, uv, static_cast<int>(sample.layer), metal::level(0.0));
            return _e22;
        }
        default: {
            metal::float4 _e28 = star_halos_c.sample(cloud_sampler, uv, static_cast<int>(sample.layer), metal::level(0.0));
            return _e28;
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
    float _e24 = cloud.star_geometry.x;
    float radius = 1.0 - (_e24 * 0.5);
    float outer_1 = radius * s_1.cell;
    if (dist_1 >= outer_1) {
        return metal::float4(0.0);
    }
    metal::float3 colour_1 = static_cast<metal::float3>(metal::uint3(t_1.z >> 20u, t_1.z >> 10u, t_1.z) & metal::uint3(1023u)) / metal::float3(1023.0);
    metal::float2 shape_1 = float2(as_type<half2>(t_1.w));
    float d_1 = dist_1 * shape_1.x;
    full_1 = metal::exp((-0.5 * d_1) * d_1);
    if (s_1.fringe > 0.0) {
        float _e61 = full_1;
        full_1 = _e61 + (s_1.fringe * metal::exp(-0.4 * d_1));
    }
    float _e68 = full_1;
    float cover_1 = (metal::min(_e68, 1.0) * (1.0 - metal::smoothstep((radius - 0.15) * s_1.cell, outer_1, dist_1))) * shape_1.y;
    return metal::float4(colour_1 * cover_1, cover_1);
}

metal::int2 naga_f2i32(metal::float2 value) {
    return static_cast<metal::int2>(metal::clamp(value, -2147483600.0, 2147483500.0));
}

metal::float4 star_far_gather(
    StarSlice s_2,
    metal::float2 r,
    constant Cloud& cloud,
    metal::texture2d<uint, metal::access::sample> star_atlas
) {
    metal::float4 result = metal::float4(0.0);
    metal::float2 o = metal::floor(r - metal::float2(0.5));
    metal::float2 f_2 = r - o;
    metal::int2 local_3 = as_type<metal::int2>(as_type<metal::uint2>(as_type<metal::int2>(as_type<metal::uint2>(naga_f2i32(o)) - as_type<metal::uint2>(naga_f2i32(metal::floor(s_2.offset))))) - as_type<metal::uint2>(s_2.origin));
    int index_3 = as_type<int>(as_type<uint>(as_type<int>(as_type<uint>(s_2.base) + as_type<uint>(as_type<int>(as_type<uint>(local_3.y) * as_type<uint>(s_2.grid.x))))) + as_type<uint>(local_3.x));
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

metal::float3 star_layers(
    metal::float2 pt_1,
    uint first,
    uint last,
    metal::float3 under,
    metal::sampler cloud_sampler,
    constant Cloud& cloud,
    metal::texture2d<uint, metal::access::sample> star_atlas,
    metal::texture2d_array<float, metal::access::sample> star_halos,
    metal::texture2d_array<float, metal::access::sample> star_halos_b,
    metal::texture2d_array<float, metal::access::sample> star_halos_c
) {
    metal::float3 out = {};
    float far_gap = 1.0;
    uint k_1 = {};
    metal::float4 slice = {};
    bool local = {};
    bool local_1 = {};
    bool local_2 = {};
    out = under;
    metal::float2 _e9 = cloud.size;
    float _e17 = cloud.size.y;
    metal::float2 sp = (pt_1 - (_e9 * 0.5)) * (STAR_PANE / _e17);
    k_1 = first;
    uint2 loop_bound = uint2(4294967295u);
    bool loop_init = true;
    while(true) {
        if (metal::all(loop_bound == uint2(0u))) { break; }
        loop_bound -= uint2(loop_bound.y == 0u, 1u);
        if (!loop_init) {
            uint _e149 = k_1;
            k_1 = _e149 + 1u;
        }
        loop_init = false;
        uint _e21 = k_1;
        if (_e21 < last) {
        } else {
            break;
        }
        {
            uint _e25 = k_1;
            StarSlice s_3 = cloud.star_slices.inner[metal::min(unsigned(_e25), 4u)];
            metal::float2 r_1 = (sp / metal::float2(s_3.cell)) - metal::fract(s_3.offset);
            metal::float2 o_1 = metal::floor(r_1);
            metal::float2 f_3 = r_1 - o_1;
            metal::int2 local_4 = as_type<metal::int2>(as_type<metal::uint2>(as_type<metal::int2>(as_type<metal::uint2>(naga_f2i32(o_1)) - as_type<metal::uint2>(naga_f2i32(metal::floor(s_3.offset))))) - as_type<metal::uint2>(s_3.origin));
            int index_4 = as_type<int>(as_type<uint>(as_type<int>(as_type<uint>(s_3.base) + as_type<uint>(as_type<int>(as_type<uint>(local_4.y) * as_type<uint>(s_3.grid.x))))) + as_type<uint>(local_4.x));
            slice = metal::float4(0.0);
            float _e57 = cloud.star_far.z;
            if (_e57 > 0.0) {
                uint _e62 = k_1;
                local = _e62 < STAR_FAR_LAYERS;
            } else {
                local = false;
            }
            bool _e66 = local;
            if (_e66) {
                metal::float4 _e67 = star_far_gather(s_3, r_1, cloud, star_atlas);
                slice = _e67;
            } else {
                metal::float4 _e69 = star_texel(s_3, f_3, index_4, false, cloud, star_atlas);
                slice = _e69;
                metal::float4 _e70 = slice;
                uint _e71 = k_1;
                metal::float4 _e72 = star_halo_at(pt_1, _e71, cloud_sampler, cloud, star_halos, star_halos_b, star_halos_c);
                slice = _e70 + _e72;
            }
            float _e75 = slice.w;
            if (_e75 > 0.0) {
                metal::float3 _e78 = out;
                metal::float4 _e79 = slice;
                float _e82 = slice.w;
                float _e86 = slice.w;
                out = metal::mix(_e78, _e79.xyz / metal::float3(_e82), metal::min(_e86, 1.0));
            }
            float _e93 = cloud.star_geometry.w;
            if (_e93 > 0.0) {
                uint _e98 = k_1;
                local_1 = _e98 < STAR_FAR_LAYERS;
            } else {
                local_1 = false;
            }
            bool _e102 = local_1;
            if (_e102) {
                float _e103 = far_gap;
                float _e105 = slice.w;
                far_gap = _e103 * (1.0 - metal::min(_e105, 1.0));
                uint _e111 = k_1;
                if ((_e111 + 1u) == STAR_FAR_LAYERS) {
                    local_2 = first == 0u;
                } else {
                    local_2 = false;
                }
                bool _e121 = local_2;
                if (_e121) {
                    float _e125 = cloud.star_geometry.w;
                    float amount = _e125 * 2.0;
                    float gentle = metal::min(amount, 1.0);
                    float strong = metal::max(amount - 1.0, 0.0);
                    float _e134 = far_gap;
                    float _e138 = far_gap;
                    float _e140 = far_gap;
                    float gain = (1.0 + (gentle * _e134)) * (1.0 + ((strong * _e138) * _e140));
                    metal::float3 _e145 = out;
                    out = under + ((_e145 - under) * gain);
                }
            }
        }
    }
    metal::float3 _e152 = out;
    return _e152;
}

metal::float3 star_near_color(
    metal::float2 pt_2,
    metal::sampler cloud_sampler,
    constant Cloud& cloud,
    metal::texture2d<float, metal::access::sample> cloud_tone,
    metal::texture2d<uint, metal::access::sample> star_atlas,
    metal::texture2d_array<float, metal::access::sample> star_halos,
    metal::texture2d_array<float, metal::access::sample> star_halos_b,
    metal::texture2d_array<float, metal::access::sample> star_halos_c
) {
    metal::float3 far = metal::float3(0.0);
    float _e7 = cloud.star_far.z;
    if (_e7 > 0.0) {
        metal::float2 _e14 = cloud.size;
        metal::float4 _e17 = cloud_tone.sample(cloud_sampler, pt_2 / _e14, metal::level(0.0));
        far = _e17.xyz;
    } else {
        float _e22 = cloud.ppp;
        uint clamped_lod_e26 = metal::min(uint(0), cloud_tone.get_num_mip_levels() - 1);
        metal::float4 _e26 = cloud_tone.read(metal::min(metal::uint2(naga_f2i32(pt_2 * _e22)), metal::uint2(cloud_tone.get_width(clamped_lod_e26), cloud_tone.get_height(clamped_lod_e26)) - 1), clamped_lod_e26);
        far = _e26.xyz;
    }
    metal::float3 _e30 = far;
    metal::float3 _e31 = star_layers(pt_2, STAR_FAR_LAYERS, STAR_SLICES, _e30, cloud_sampler, cloud, star_atlas, star_halos, star_halos_b, star_halos_c);
    return _e31;
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
, metal::sampler cloud_sampler [[sampler(0)]]
, constant Cloud& cloud [[buffer(2)]]
, metal::texture2d<float, metal::access::sample> cloud_tone [[texture(3)]]
, metal::texture2d<uint, metal::access::sample> star_atlas [[texture(6)]]
, metal::texture2d_array<float, metal::access::sample> star_halos [[texture(8)]]
, metal::texture2d_array<float, metal::access::sample> star_halos_b [[texture(9)]]
, metal::texture2d_array<float, metal::access::sample> star_halos_c [[texture(10)]]
) {
    const TileVertex in = { position, varyings.fraction, varyings.layer };
    metal::float4 _e5 = cloud.star_near;
    metal::float2 _e10 = cloud.size;
    metal::float2 pt_3 = (in.position.xy / _e5.xy) * _e10;
    metal::float3 _e12 = star_near_color(pt_3, cloud_sampler, cloud, cloud_tone, star_atlas, star_halos, star_halos_b, star_halos_c);
    return fs_star_nearOutput { metal::float4(_e12, 1.0) };
}
