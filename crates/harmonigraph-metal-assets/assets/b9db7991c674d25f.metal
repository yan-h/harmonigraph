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
    metal::float4 star_geometry;
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
constant float STAR_HALO_FADE = 0.7;
constant int STAR_HASH_PERIOD = 65536;
constant uint STAR_LIFE_PERIOD = 4096u;
constant float STAR_FADE = 0.2;
constant float STAR_LIFT = 0.18;
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

metal::float4 star_geometry(
    constant Settings& settings
) {
    metal::float4 _e3 = settings.stars.star_geometry;
    return _e3;
}

metal::float4 star_texel(
    StarSlice s,
    metal::float2 f,
    int index,
    bool halo_1,
    constant Settings& settings,
    metal::texture2d<uint, metal::access::sample> star_atlas
) {
    float cover = {};
    float full = {};
    uint clamped_lod_e11 = metal::min(uint(0), star_atlas.get_num_mip_levels() - 1);
    metal::uint4 t = star_atlas.read(metal::min(metal::uint2(metal::int2(index & 2047, index >> STAR_ATLAS_SHIFT)), metal::uint2(star_atlas.get_width(clamped_lod_e11), star_atlas.get_height(clamped_lod_e11)) - 1), clamped_lod_e11);
    if (t.w == 0u) {
        return metal::float4(0.0);
    }
    float dist = metal::length(f - metal::float2(as_type<float>(t.x), as_type<float>(t.y))) * s.cell;
    float reach = s.core * s.cell;
    float outer = s.glow * s.cell;
    if (dist >= (halo_1 ? outer : reach)) {
        return metal::float4(0.0);
    }
    metal::float3 colour = static_cast<metal::float3>(metal::uint3(t.z >> 20u, t.z >> 10u, t.z) & metal::uint3(1023u)) / metal::float3(1023.0);
    metal::float2 shape = float2(as_type<half2>(t.w));
    float d = dist * shape.x;
    float gaussian = metal::exp((-0.5 * d) * d);
    metal::float4 _e59 = star_geometry(settings);
    float core = gaussian * (1.0 - metal::smoothstep(_e59.z * reach, reach, dist));
    cover = core;
    if (halo_1) {
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
    metal::int2 local = as_type<metal::int2>(as_type<metal::uint2>(as_type<metal::int2>(as_type<metal::uint2>(naga_f2i32(o)) - as_type<metal::uint2>(naga_f2i32(metal::floor(_e19.offset))))) - as_type<metal::uint2>(_e19.origin));
    int index_1 = as_type<int>(as_type<uint>(as_type<int>(as_type<uint>(_e19.base) + as_type<uint>(as_type<int>(as_type<uint>(local.y) * as_type<uint>(_e19.grid.x))))) + as_type<uint>(local.x));
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
            metal::float4 _e68 = star_texel(_e19, metal::float2(f_1.x + 1.0, fy), as_type<int>(as_type<uint>(row) - as_type<uint>(1)), true, settings, star_atlas);
            halo = _e60 + _e68;
            metal::float4 _e70 = halo;
            metal::float4 _e74 = star_texel(_e19, metal::float2(f_1.x, fy), row, true, settings, star_atlas);
            halo = _e70 + _e74;
            metal::float4 _e76 = halo;
            metal::float4 _e84 = star_texel(_e19, metal::float2(f_1.x - 1.0, fy), as_type<int>(as_type<uint>(row) + as_type<uint>(1)), true, settings, star_atlas);
            halo = _e76 + _e84;
        }
    }
    metal::float4 _e89 = halo;
    return fs_star_haloOutput { _e89 };
}
