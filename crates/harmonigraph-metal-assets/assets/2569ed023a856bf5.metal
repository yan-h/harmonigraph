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
struct type_7 {
    StarSlice inner[5];
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
    metal::float4 star_geometry;
    type_7 star_slices;
    uint memory_enabled;
    uint memory_valid;
    float pickup_alpha;
    float release_alpha;
    metal::int2 memory_shift;
    metal::float2 memory_fraction;
    float previous_life;
    float memory_pad_a;
    metal::float2 memory_extent;
    type_7 previous_slices;
    metal::float2 star_halo_size;
    metal::float2 star_halo_pad;
};
struct TileVertex {
    metal::float4 position;
    metal::float2 fraction;
    uint layer;
    char _pad3[4];
};
constant float CLOUD_UNITS = 10.0;
constant float CLOUD_TILE_ROT_COS = 0.8;
constant float CLOUD_TILE_ROT_SIN = 0.6;
constant float SCALE_CELLS = 2.7272727;
constant float DOME_RADIUS = 1.15;
constant float DOME_JITTER = 0.3;
constant float DOME_RADIUS_MIN = 0.95;
constant float DOME_RADIUS_MAX = 1.32;
constant float DOME_UNION = 9.0;
constant float DOME_VARIETY_GAIN = 5.0;
constant float DOME_FACE = 1.5122874;
constant float DOME_LACUNARITY = 2.1;
constant float DOME_FINE_GAIN = 0.22;
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

metal::float4 star_texel(
    StarSlice s,
    metal::float2 f,
    int index,
    bool halo_1,
    constant Cloud& cloud,
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
    float _e29 = cloud.star_geometry.y;
    float reach = _e29 * s.cell;
    if (dist >= (halo_1 ? (STAR_HALO_REACH * s.cell) : reach)) {
        return metal::float4(0.0);
    }
    metal::float3 colour = static_cast<metal::float3>(metal::uint3(t.z >> 20u, t.z >> 10u, t.z) & metal::uint3(1023u)) / metal::float3(1023.0);
    metal::float2 shape = float2(as_type<half2>(t.w));
    float d = dist * shape.x;
    float gaussian = metal::exp((-0.5 * d) * d);
    float _e65 = cloud.star_geometry.z;
    float core = gaussian * (1.0 - metal::smoothstep(_e65 * reach, reach, dist));
    cover = core;
    if (halo_1) {
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
    metal::float2 _e3 = cloud.size;
    metal::float2 _e6 = cloud.star_halo_size;
    metal::float2 step = _e3 / _e6;
    metal::float2 pt = in.position.xy * step;
    metal::float2 _e13 = cloud.size;
    float _e21 = cloud.size.y;
    metal::float2 sp = (pt - (_e13 * 0.5)) * (STAR_PANE / _e21);
    StarSlice s_1 = cloud.star_slices.inner[metal::min(unsigned(in.layer), 4u)];
    metal::float2 r = (sp / metal::float2(s_1.cell)) - metal::fract(s_1.offset);
    metal::float2 o = metal::floor(r);
    metal::float2 f_1 = r - o;
    metal::int2 local = as_type<metal::int2>(as_type<metal::uint2>(as_type<metal::int2>(as_type<metal::uint2>(naga_f2i32(o)) - as_type<metal::uint2>(naga_f2i32(metal::floor(s_1.offset))))) - as_type<metal::uint2>(s_1.origin));
    int index_1 = as_type<int>(as_type<uint>(as_type<int>(as_type<uint>(s_1.base) + as_type<uint>(as_type<int>(as_type<uint>(local.y) * as_type<uint>(s_1.grid.x))))) + as_type<uint>(local.x));
    uint2 loop_bound = uint2(4294967295u);
    bool loop_init = true;
    while(true) {
        if (metal::all(loop_bound == uint2(0u))) { break; }
        loop_bound -= uint2(loop_bound.y == 0u, 1u);
        if (!loop_init) {
            int _e95 = y;
            y = as_type<int>(as_type<uint>(_e95) + as_type<uint>(1));
        }
        loop_init = false;
        int _e57 = y;
        if (_e57 <= 1) {
        } else {
            break;
        }
        {
            int _e60 = y;
            int row = as_type<int>(as_type<uint>(index_1) + as_type<uint>(as_type<int>(as_type<uint>(_e60) * as_type<uint>(s_1.grid.x))));
            int _e66 = y;
            float fy = f_1.y - static_cast<float>(_e66);
            metal::float4 _e69 = halo;
            metal::float4 _e77 = star_texel(s_1, metal::float2(f_1.x + 1.0, fy), as_type<int>(as_type<uint>(row) - as_type<uint>(1)), true, cloud, star_atlas);
            halo = _e69 + _e77;
            metal::float4 _e79 = halo;
            metal::float4 _e83 = star_texel(s_1, metal::float2(f_1.x, fy), row, true, cloud, star_atlas);
            halo = _e79 + _e83;
            metal::float4 _e85 = halo;
            metal::float4 _e93 = star_texel(s_1, metal::float2(f_1.x - 1.0, fy), as_type<int>(as_type<uint>(row) + as_type<uint>(1)), true, cloud, star_atlas);
            halo = _e85 + _e93;
        }
    }
    metal::float4 _e98 = halo;
    return fs_star_haloOutput { _e98 };
}
