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
struct type_7 {
    StarSlice inner[5];
};
struct type_8 {
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
    type_7 star_slices;
    type_8 star_halo_samples;
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
    constant Settings& settings
) {
    StarHaloSample _e5 = settings.stars.star_halo_samples.inner[metal::min(unsigned(k_1), 4u)];
    return _e5;
}
metal::int2 naga_f2i32(metal::float2 value) {
    return static_cast<metal::int2>(metal::clamp(value, -2147483600.0, 2147483500.0));
}


struct fs_star_haloInput {
    uint layer [[user(loc0), flat]];
};
struct fs_star_haloOutput {
    metal::float4 member [[color(0)]];
};
fragment fs_star_haloOutput fs_star_halo(
  fs_star_haloInput varyings [[stage_in]]
, metal::float4 position [[position]]
, constant Settings& settings [[buffer(0)]]
, metal::texture2d<uint, metal::access::sample> star_atlas [[texture(1)]]
) {
    const TileVertex in = { position, varyings.layer };
    metal::float4 halo = metal::float4(0.0);
    int y = -1;
    metal::float2 _e1 = star_size(settings);
    StarHaloSample _e3 = star_halo_sample(in.layer, settings);
    metal::float2 step = _e1 / _e3.size;
    metal::float2 pt = in.position.xy * step;
    metal::float2 _e9 = star_size(settings);
    metal::float2 _e14 = star_size(settings);
    metal::float2 sp = (pt - (_e9 * 0.5)) * (STAR_PANE / _e14.y);
    StarSlice _e19 = star_slice(in.layer, settings);
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
