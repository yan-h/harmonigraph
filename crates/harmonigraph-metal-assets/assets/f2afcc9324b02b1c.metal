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
    uint gather;
    float twinkle;
};
struct type_7 {
    StarSlice inner[5];
};
struct type_8 {
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
    type_7 star_slices;
    uint memory_enabled;
    uint memory_valid;
    float pickup_alpha;
    float release_alpha;
    metal::int2 memory_shift;
    metal::float2 memory_fraction;
    float previous_life;
    float wash_randomness;
    metal::float2 memory_extent;
    type_7 previous_slices;
    type_8 star_halo_samples;
    metal::float4 velvet;
    metal::float4 velvet_form;
    metal::float4 wash_pigment;
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
constant float WASH_FBM_FINE = 2.0;
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
constant float CLOUD_UNITS = 10.0;
constant float WASH_POOL = 0.44;
constant float WASH_PIG_DEPTH = 0.35;
constant float WASH_BLOOM = 2.7;

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
    int index,
    bool halo_1,
    metal::texture2d<uint, metal::access::sample> star_atlas
) {
    bool local = {};
    uint clamped_lod_e11 = metal::min(uint(0), star_atlas.get_num_mip_levels() - 1);
    metal::uint4 t_1 = star_atlas.read(metal::min(metal::uint2(metal::int2(index & 2047, index >> STAR_ATLAS_SHIFT)), metal::uint2(star_atlas.get_width(clamped_lod_e11), star_atlas.get_height(clamped_lod_e11)) - 1), clamped_lod_e11);
    if (t_1.w == 0u) {
        return metal::float4(0.0);
    }
    float dist = metal::length(f - metal::float2(as_type<float>(t_1.x), as_type<float>(t_1.y))) * s_1.cell;
    float reach = s_1.inner * s_1.cell;
    if (!(halo_1)) {
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
    float cover = (halo_1 ? metal::max(_e41 - inner, 0.0) : inner) * shape.y;
    return metal::float4(colour * cover, cover);
}

StarHaloSample star_halo_sample(
    uint k_1,
    constant Cloud& cloud
) {
    StarHaloSample _e4 = cloud.star_halo_samples.inner[metal::min(unsigned(k_1), 4u)];
    return _e4;
}
metal::int2 naga_f2i32(metal::float2 value) {
    return static_cast<metal::int2>(metal::clamp(value, -2147483600.0, 2147483500.0));
}


struct fs_star_haloInput {
    metal::float2 fraction [[user(loc0), center_perspective]];
    uint layer [[user(loc1), flat]];
};
struct fs_star_haloOutput {
    metal::float4 member [[color(0)]];
};
fragment fs_star_haloOutput fs_star_halo(
  fs_star_haloInput varyings [[stage_in]]
, metal::float4 position [[position]]
, constant Cloud& cloud [[buffer(2)]]
, metal::texture2d<uint, metal::access::sample> star_atlas [[texture(6)]]
) {
    const TileVertex in = { position, varyings.fraction, varyings.layer };
    metal::float4 halo = metal::float4(0.0);
    int y = -1;
    metal::float2 _e1 = star_size(cloud);
    StarHaloSample _e3 = star_halo_sample(in.layer, cloud);
    metal::float2 step = _e1 / _e3.size;
    metal::float2 pt = in.position.xy * step;
    metal::float2 _e9 = star_size(cloud);
    metal::float2 _e14 = star_size(cloud);
    metal::float2 sp = (pt - (_e9 * 0.5)) * (STAR_PANE / _e14.y);
    StarSlice _e19 = star_slice(in.layer, cloud);
    metal::float2 r = (sp / metal::float2(_e19.cell)) - metal::fract(_e19.offset);
    metal::float2 o = metal::floor(r);
    metal::float2 f_1 = r - o;
    metal::int2 local_1 = as_type<metal::int2>(as_type<metal::uint2>(as_type<metal::int2>(as_type<metal::uint2>(naga_f2i32(o)) - as_type<metal::uint2>(naga_f2i32(metal::floor(_e19.offset))))) - as_type<metal::uint2>(_e19.origin));
    int index_1 = as_type<int>(as_type<uint>(as_type<int>(as_type<uint>(_e19.base) + as_type<uint>(as_type<int>(as_type<uint>(local_1.y) * as_type<uint>(_e19.grid.x))))) + as_type<uint>(local_1.x));
    uint2 loop_bound = uint2(4294967295u);
    bool loop_init = true;
    while(true) {
        if (metal::all(loop_bound == uint2(0u))) { break; }
        loop_bound -= uint2(loop_bound.y == 0u, 1u);
        if (!loop_init) {
            int _e86 = y;
            y = as_type<int>(as_type<uint>(_e86) + as_type<uint>(1));
        }
        loop_init = false;
        int _e48 = y;
        if (_e48 <= 1) {
        } else {
            break;
        }
        {
            int _e51 = y;
            int row = as_type<int>(as_type<uint>(index_1) + as_type<uint>(as_type<int>(as_type<uint>(_e51) * as_type<uint>(_e19.grid.x))));
            int _e57 = y;
            float fy = f_1.y - static_cast<float>(_e57);
            metal::float4 _e60 = halo;
            metal::float4 _e68 = star_texel(_e19, metal::float2(f_1.x + 1.0, fy), as_type<int>(as_type<uint>(row) - as_type<uint>(1)), true, star_atlas);
            halo = _e60 + _e68;
            metal::float4 _e70 = halo;
            metal::float4 _e74 = star_texel(_e19, metal::float2(f_1.x, fy), row, true, star_atlas);
            halo = _e70 + _e74;
            metal::float4 _e76 = halo;
            metal::float4 _e84 = star_texel(_e19, metal::float2(f_1.x - 1.0, fy), as_type<int>(as_type<uint>(row) + as_type<uint>(1)), true, star_atlas);
            halo = _e76 + _e84;
        }
    }
    metal::float4 _e89 = halo;
    return fs_star_haloOutput { _e89 };
}
