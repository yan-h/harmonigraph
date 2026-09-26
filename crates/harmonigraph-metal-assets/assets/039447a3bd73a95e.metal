// language: metal3.2
#include <metal_stdlib>
#include <simd/simd.h>

using metal::uint;

struct VertexOut {
    metal::float4 position;
    float slab;
    float t;
    char _pad3[8];
};
struct StarSlice {
    metal::float2 offset;
    float cell;
    float sigma;
    float cap;
    float defocus;
    float occupancy;
    float blur;
    float fringe;
    float fringe_reach;
    float reach;
    float unseen;
};
struct type_8 {
    StarSlice inner[8];
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
    float star_glow;
    float star_wander;
    float star_time;
    metal::float2 _star_pad;
    type_8 star_slices;
};
struct Pile {
    metal::float2 face;
    metal::float2 to_centre;
};
struct Wet {
    metal::float2 offset;
};
struct WashField {
    Wet coarse;
    Wet fine;
    float cover;
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
constant uint STAR_SLICES = 8u;
constant float STAR_PANE = 540.0;
constant float STAR_JITTER = 0.6;
constant int STAR_HASH_PERIOD = 4096;
constant float STAR_WANDER_PERIOD = 400.0;
constant float STAR_EXPOSURE = 1.5;
constant float STAR_LIFT = 0.18;
constant float STAR_OVER_GROUND = 6.0;
constant float STAR_RING_FADE = 0.7;
constant float STAR_TAU = 6.2831855;

float density_decode(
    float value
) {
    float y = metal::max(value, 0.0);
    return (2.0 * y) / (0.1 + metal::sqrt(0.01 + (3.6 * y)));
}

float baked_density(
    metal::float2 position,
    metal::texture2d<float, metal::access::sample> close_light,
    metal::sampler cloud_sampler,
    constant Cloud& cloud
) {
    float _e3 = cloud.ppp;
    metal::float2 _e8 = cloud.origin;
    metal::float2 _e12 = cloud.size;
    metal::float2 uv = ((position / metal::float2(_e3)) - _e8) / _e12;
    metal::float4 _e17 = close_light.sample(cloud_sampler, uv, metal::level(0.0));
    return _e17.x;
}

bool softened(
    constant Cloud& cloud
) {
    metal::float2 _e2 = cloud.step;
    return metal::any(_e2 != metal::float2(0.0));
}

float style_level(
    float level,
    constant Cloud& cloud
) {
    float _e3 = cloud.contour_strength;
    if (_e3 <= 0.0) {
        return level;
    }
    float _e11 = cloud.contours;
    float x = metal::clamp(level, 0.0, 1.0) * _e11;
    float _e15 = cloud.contour_softness;
    float _e16 = metal::fwidth(x);
    float edge = metal::min(0.5, metal::max(_e15, _e16 * 0.5));
    float _e32 = cloud.contours;
    float terraces = (metal::floor(x) + metal::smoothstep(0.5 - edge, 0.5 + edge, metal::fract(x))) / _e32;
    float _e36 = cloud.contour_strength;
    float _e43 = metal::fwidth(x);
    float strength = ((0.9 * _e36) * metal::smoothstep(0.0, 1.0, x)) * (1.0 - metal::smoothstep(0.5, 1.5, _e43));
    return metal::mix(level, terraces, strength);
}

uint naga_f2u32(float value) {
    return static_cast<uint>(metal::clamp(value, 0.0, 4294967000.0));
}

metal::float3 palette_color(
    float level_1,
    metal::texture2d<float, metal::access::sample> lut
) {
    uint levels = metal::uint2(lut.get_width(), lut.get_height()).x;
    float x_1 = metal::max((metal::clamp(level_1, 0.0, 1.0) * static_cast<float>(levels)) - 0.5, 0.0);
    uint i = naga_f2u32(metal::clamp(metal::floor(x_1), 0.0, static_cast<float>(levels - 1u)));
    uint clamped_lod_e24 = metal::min(uint(0), lut.get_num_mip_levels() - 1);
    metal::float4 _e24 = lut.read(metal::min(metal::uint2(metal::uint2(i, 0u)), metal::uint2(lut.get_width(clamped_lod_e24), lut.get_height(clamped_lod_e24)) - 1), clamped_lod_e24);
    metal::float3 a = _e24.xyz;
    uint clamped_lod_e35 = metal::min(uint(0), lut.get_num_mip_levels() - 1);
    metal::float4 _e35 = lut.read(metal::min(metal::uint2(metal::uint2(metal::min(i + 1u, levels - 1u), 0u)), metal::uint2(lut.get_width(clamped_lod_e35), lut.get_height(clamped_lod_e35)) - 1), clamped_lod_e35);
    metal::float3 b = _e35.xyz;
    return metal::mix(a, b, metal::fract(x_1));
}

metal::float4 density_color(
    float raw_level,
    metal::texture2d<float, metal::access::sample> lut,
    constant Cloud& cloud
) {
    float _e1 = style_level(raw_level, cloud);
    metal::float3 _e2 = palette_color(_e1, lut);
    return metal::float4(_e2, 1.0);
}

metal::float2 watercolor_tile_uv_for(
    metal::float2 r,
    float period,
    uint pitch_vertical
) {
    metal::float2 semantic = (pitch_vertical == 1u) ? r : metal::float2(r.y, r.x);
    return metal::float2((CLOUD_TILE_ROT_COS * semantic.x) + (CLOUD_TILE_ROT_SIN * semantic.y), (-0.6 * semantic.x) + (CLOUD_TILE_ROT_COS * semantic.y)) / metal::float2(period);
}

metal::float2 watercolor_tile_uv(
    metal::float2 r_1,
    constant Cloud& cloud
) {
    uint _e3 = cloud.tile_cells;
    uint _e7 = cloud.pitch_vertical;
    metal::float2 _e8 = watercolor_tile_uv_for(r_1, static_cast<float>(_e3), _e7);
    return _e8;
}

metal::float2 rotate_watercolor_tile_vector_for(
    metal::float2 v,
    uint pitch_vertical_1
) {
    metal::float2 semantic_1 = (pitch_vertical_1 == 1u) ? v : metal::float2(v.y, v.x);
    metal::float2 turned = metal::float2((CLOUD_TILE_ROT_COS * semantic_1.x) - (CLOUD_TILE_ROT_SIN * semantic_1.y), (CLOUD_TILE_ROT_SIN * semantic_1.x) + (CLOUD_TILE_ROT_COS * semantic_1.y));
    return (pitch_vertical_1 == 1u) ? turned : metal::float2(turned.y, turned.x);
}

metal::float2 rotate_watercolor_tile_vector(
    metal::float2 v_1,
    constant Cloud& cloud
) {
    uint _e3 = cloud.pitch_vertical;
    metal::float2 _e4 = rotate_watercolor_tile_vector_for(v_1, _e3);
    return _e4;
}

float cloud_light(
    metal::float2 pt,
    metal::texture2d<float, metal::access::sample> close_light,
    metal::sampler cloud_sampler,
    constant Cloud& cloud
) {
    metal::float2 _e5 = cloud.size;
    metal::float4 _e8 = close_light.sample(cloud_sampler, pt / _e5, metal::level(0.0));
    return _e8.x;
}

float scale_tone(
    metal::float2 pt_1,
    metal::texture2d<float, metal::access::sample> close_light,
    metal::sampler cloud_sampler,
    constant Cloud& cloud,
    metal::texture2d<float, metal::access::sample> cloud_tile_a,
    metal::sampler tile_sampler
) {
    Pile pile = {};
    metal::float2 _e3 = cloud.size;
    float _e10 = cloud.size.y;
    metal::float2 _e17 = cloud.drift;
    metal::float2 q = (((pt_1 - (_e3 * 0.5)) / metal::float2(_e10)) * CLOUD_UNITS) + _e17;
    float _e22 = cloud.scale_size;
    float scale_units = SCALE_CELLS / _e22;
    float _e27 = cloud.size.y;
    float scale_points = (_e27 / CLOUD_UNITS) / scale_units;
    metal::float2 r_4 = q * scale_units;
    uint _e36 = cloud.tile_cells;
    metal::float4 tile = cloud_tile_a.sample(tile_sampler, r_4 / metal::float2(static_cast<float>(_e36)), metal::level(0.0));
    pile.face = tile.xy;
    pile.to_centre = tile.zw;
    metal::float2 face = pile.face;
    float _e51 = cloud.scale_refract;
    float bend = metal::max(_e51, 0.0) * scale_points;
    float _e57 = cloud.scale_refract;
    float gather = metal::max(-(_e57), 0.0) * scale_points;
    metal::float2 _e65 = pile.to_centre;
    metal::float2 lookup = (-(face) * bend) + (_e65 * gather);
    float _e69 = cloud_light(pt_1 + lookup, close_light, cloud_sampler, cloud);
    return _e69;
}

metal::float3 wash_hash(
    metal::int2 cell,
    uint salt
) {
    uint n = {};
    n = (as_type<uint>(cell.x) * 2654435769u) ^ (as_type<uint>(cell.y) * 2246822507u);
    uint _e12 = n;
    n = _e12 ^ (salt * 668265261u);
    uint _e16 = n;
    uint _e17 = n;
    n = (_e16 ^ (_e17 >> 16u)) * 2146121005u;
    uint _e23 = n;
    uint _e24 = n;
    n = (_e23 ^ (_e24 >> 15u)) * 2221713035u;
    uint _e30 = n;
    uint _e31 = n;
    n = _e30 ^ (_e31 >> 16u);
    uint _e35 = n;
    uint _e41 = n;
    uint _e49 = n;
    return metal::float3(static_cast<float>(_e35 & 1023u) / 1023.0, static_cast<float>((_e41 >> 10u) & 1023u) / 1023.0, static_cast<float>((_e49 >> 20u) & 1023u) / 1023.0);
}

float wash_level(
    Wet wet,
    float pane_per_cell,
    metal::float2 pt_2,
    metal::texture2d<float, metal::access::sample> close_light,
    metal::sampler cloud_sampler,
    constant Cloud& cloud
) {
    float _e7 = cloud.wash_refract;
    float _e10 = cloud_light(pt_2 + ((wet.offset * pane_per_cell) * _e7), close_light, cloud_sampler, cloud);
    return _e10;
}

WashField wash_tile_field(
    metal::float2 r_2,
    constant Cloud& cloud,
    metal::texture2d<float, metal::access::sample> cloud_tile_a,
    metal::texture2d<float, metal::access::sample> cloud_tile_b,
    metal::sampler tile_sampler
) {
    WashField out = {};
    metal::float2 _e1 = watercolor_tile_uv(r_2, cloud);
    metal::float4 a_1 = cloud_tile_a.sample(tile_sampler, _e1, metal::level(0.0));
    metal::float2 _e9 = rotate_watercolor_tile_vector(a_1.xy, cloud);
    out.coarse = Wet {_e9};
    out.fine = Wet {metal::float2(0.0)};
    out.cover = 0.0;
    float _e19 = cloud.wash_layers;
    if (_e19 > 0.0) {
        metal::float4 b_1 = cloud_tile_b.sample(tile_sampler, _e1, metal::level(0.0));
        metal::float2 _e28 = rotate_watercolor_tile_vector(b_1.xy, cloud);
        out.fine = Wet {_e28};
        out.cover = b_1.w;
    }
    WashField _e32 = out;
    return _e32;
}

float wash_cloud_tone(
    metal::float2 pt_3,
    metal::texture2d<float, metal::access::sample> close_light,
    metal::sampler cloud_sampler,
    constant Cloud& cloud,
    metal::texture2d<float, metal::access::sample> cloud_tile_a,
    metal::texture2d<float, metal::access::sample> cloud_tile_b,
    metal::sampler tile_sampler
) {
    float level_2 = {};
    metal::float2 _e3 = cloud.size;
    float _e10 = cloud.size.y;
    metal::float2 _e17 = cloud.drift;
    metal::float2 q_1 = (((pt_3 - (_e3 * 0.5)) / metal::float2(_e10)) * CLOUD_UNITS) + _e17;
    float _e22 = cloud.wash_size;
    float cells = WASH_CELLS / _e22;
    float _e27 = cloud.size.y;
    float pane_per_cell_1 = (_e27 / CLOUD_UNITS) / cells;
    metal::float2 r_5 = q_1 * cells;
    WashField _e32 = wash_tile_field(r_5, cloud, cloud_tile_a, cloud_tile_b, tile_sampler);
    float _e34 = wash_level(_e32.coarse, pane_per_cell_1, pt_3, close_light, cloud_sampler, cloud);
    level_2 = _e34;
    float _e38 = cloud.wash_layers;
    if (_e38 > 0.0) {
        float _e44 = wash_level(_e32.fine, pane_per_cell_1 / WASH_LACUNARITY, pt_3, close_light, cloud_sampler, cloud);
        float _e47 = cloud.wash_layers;
        float over = _e47 * _e32.cover;
        float _e50 = level_2;
        level_2 = metal::mix(_e50, _e44, over);
    }
    float _e52 = level_2;
    return _e52;
}

float cloud_tone_at(
    metal::float2 pt_4,
    metal::texture2d<float, metal::access::sample> close_light,
    metal::sampler cloud_sampler,
    constant Cloud& cloud,
    metal::texture2d<float, metal::access::sample> cloud_tile_a,
    metal::texture2d<float, metal::access::sample> cloud_tile_b,
    metal::sampler tile_sampler
) {
    uint _e3 = cloud.cloud_style;
    if (_e3 == 1u) {
        float _e6 = wash_cloud_tone(pt_4, close_light, cloud_sampler, cloud, cloud_tile_a, cloud_tile_b, tile_sampler);
        return _e6;
    }
    float _e7 = scale_tone(pt_4, close_light, cloud_sampler, cloud, cloud_tile_a, tile_sampler);
    return _e7;
}

float star_brightest(
    metal::float3 c
) {
    return metal::max(c.x, metal::max(c.y, c.z));
}

float star_level_at(
    metal::float2 pt_5,
    float blur,
    metal::texture2d<float, metal::access::sample> close_light,
    metal::texture2d<float, metal::access::sample> wide_light,
    metal::sampler cloud_sampler,
    constant Cloud& cloud
) {
    float level_3 = {};
    metal::float2 _e4 = cloud.size;
    metal::float2 uv_1 = pt_5 / _e4;
    metal::float4 _e9 = close_light.sample(cloud_sampler, uv_1, metal::level(0.0));
    level_3 = _e9.x;
    if (blur > 0.0) {
        metal::float4 _e17 = wide_light.sample(cloud_sampler, uv_1, metal::level(0.0));
        float _e19 = density_decode(_e17.x);
        float _e20 = level_3;
        level_3 = metal::mix(_e20, _e19, blur);
    }
    float _e22 = level_3;
    return metal::clamp(_e22, 0.0, 1.0);
}

metal::float3 star_paint(
    float level_4,
    float rank,
    metal::texture2d<float, metal::access::sample> lut,
    constant Cloud& cloud
) {
    float randomness = cloud.star_randomness;
    float spread = (1.0 - randomness) + (randomness * (0.35 + (0.65 * rank)));
    float lift = (0.09 * rank) * metal::smoothstep(0.0, 0.15, level_4);
    metal::float3 _e24 = palette_color(metal::clamp((level_4 * spread) + lift, 0.0, 1.0), lut);
    return _e24;
}

float star_over_ground(
    metal::float3 colour,
    metal::float2 ground
) {
    float _e2 = star_brightest(colour);
    return metal::clamp(((_e2 - ground.x) * STAR_OVER_GROUND) + ground.y, 0.0, 1.0);
}

metal::float4 star_cell(
    StarSlice s,
    metal::float2 r_3,
    metal::int2 cell_1,
    uint salt_1,
    float cut,
    metal::float2 ground_1,
    metal::texture2d<float, metal::access::sample> lut,
    metal::texture2d<float, metal::access::sample> close_light,
    metal::texture2d<float, metal::access::sample> wide_light,
    metal::sampler cloud_sampler,
    constant Cloud& cloud
) {
    float cover = {};
    metal::int2 hashed = cell_1 & metal::int2(4095);
    metal::float3 _e9 = wash_hash(hashed, salt_1);
    if (_e9.z >= s.occupancy) {
        return metal::float4(0.0);
    }
    metal::float3 _e17 = wash_hash(hashed, salt_1 + 1u);
    metal::float3 _e20 = wash_hash(hashed, salt_1 + 2u);
    metal::float2 rate = (metal::float2(20.0) + metal::floor(metal::float2(_e17.x, _e17.y) * 60.999)) / metal::float2(400.0);
    float _e35 = cloud.star_time;
    metal::float2 phase = metal::fract(rate * _e35) + metal::float2(_e17.z, _e20.x);
    float _e44 = cloud.star_wander;
    metal::float2 wander = _e44 * metal::sin(STAR_TAU * phase);
    metal::float2 centre = ((static_cast<metal::float2>(cell_1) + metal::float2(0.5)) + (STAR_JITTER * (_e9.xy - metal::float2(0.5)))) + wander;
    float dist = metal::length(r_3 - centre) * s.cell;
    if (dist >= cut) {
        return metal::float4(0.0);
    }
    float _e75 = cloud.size.y;
    metal::float2 _e81 = cloud.size;
    metal::float2 at = (((centre + s.offset) * s.cell) * (_e75 / STAR_PANE)) + (_e81 * 0.5);
    float _e86 = star_level_at(at, s.blur, close_light, wide_light, cloud_sampler, cloud);
    if (_e86 <= 0.0) {
        return metal::float4(0.0);
    }
    float randomness_1 = cloud.star_randomness;
    metal::float3 _e105 = star_paint(_e86, metal::pow(_e20.y, 1.0 + (6.0 * randomness_1)) * (2.0 + (6.0 * randomness_1)), lut, cloud);
    float size = metal::exp(((0.3 + (0.9 * randomness_1)) * (_e20.z - 0.5)) * 2.0);
    float sigma = metal::min(s.sigma * size, s.cap) * s.defocus;
    cover = metal::exp((-(dist) * dist) / ((2.0 * sigma) * sigma));
    if (s.fringe > 0.0) {
        float window = metal::max(1.0 - (dist / s.fringe_reach), 0.0);
        float _e140 = cover;
        cover = _e140 + (((s.fringe * metal::exp(-(dist) / (2.5 * sigma))) * window) * window);
    }
    float _e151 = cover;
    cover = metal::min(_e151, 1.0) * (1.0 - metal::smoothstep(STAR_RING_FADE * s.reach, s.reach, dist));
    float _e162 = cover;
    float _e163 = star_over_ground(_e105, ground_1);
    cover = _e162 * _e163;
    float _e165 = cover;
    float _e167 = cover;
    return metal::float4(_e105 * _e165, _e167);
}

metal::int2 naga_f2i32(metal::float2 value) {
    return static_cast<metal::int2>(metal::clamp(value, -2147483600.0, 2147483500.0));
}

int naga_mod(int lhs, int rhs) {
    int divisor = metal::select(rhs, 1, (lhs == (-2147483647 - 1) & rhs == -1) | (rhs == 0));
    return lhs - (lhs / divisor) * divisor;
}

int naga_div(int lhs, int rhs) {
    return lhs / metal::select(rhs, 1, (lhs == (-2147483647 - 1) & rhs == -1) | (rhs == 0));
}

metal::float3 star_color(
    metal::float2 pt_6,
    metal::texture2d<float, metal::access::sample> lut,
    metal::texture2d<float, metal::access::sample> close_light,
    metal::texture2d<float, metal::access::sample> wide_light,
    metal::sampler cloud_sampler,
    constant Cloud& cloud
) {
    metal::float3 out_1 = {};
    metal::float2 ground_2 = metal::float2(0.0, 10000.0);
    uint k = 0u;
    metal::float4 slice = {};
    int n_1 = {};
    bool local = {};
    metal::float2 _e3 = cloud.size;
    metal::float2 uv_2 = pt_6 / _e3;
    metal::float4 _e8 = close_light.sample(cloud_sampler, uv_2, metal::level(0.0));
    float close = _e8.x;
    metal::float4 _e13 = wide_light.sample(cloud_sampler, uv_2, metal::level(0.0));
    float _e15 = density_decode(_e13.x);
    metal::float3 _e17 = palette_color(0.0, lut);
    out_1 = _e17;
    float _e25 = cloud.star_glow;
    if (_e25 > 0.0) {
        float under = metal::clamp(_e15, 0.0, 1.0);
        float _e33 = cloud.star_glow;
        metal::float3 _e34 = palette_color(under, lut);
        metal::float3 glow = (_e33 * _e34) * metal::sqrt(under);
        out_1 = _e17 + ((metal::float3(1.0) - _e17) * (metal::float3(1.0) - metal::exp(-1.5 * glow)));
        metal::float3 _e49 = out_1;
        float _e50 = star_brightest(_e49);
        float _e51 = star_brightest(_e17);
        ground_2 = metal::float2(_e50, 1.0 - metal::min((_e50 - _e51) * STAR_OVER_GROUND, 1.0));
    }
    metal::float2 _e62 = cloud.size;
    float _e70 = cloud.size.y;
    metal::float2 sp = (pt_6 - (_e62 * 0.5)) * (STAR_PANE / _e70);
    uint2 loop_bound = uint2(4294967295u);
    bool loop_init = true;
    while(true) {
        if (metal::all(loop_bound == uint2(0u))) { break; }
        loop_bound -= uint2(loop_bound.y == 0u, 1u);
        if (!loop_init) {
            uint _e171 = k;
            k = _e171 + 1u;
        }
        loop_init = false;
        uint _e75 = k;
        if (_e75 < STAR_SLICES) {
        } else {
            break;
        }
        {
            uint _e80 = k;
            StarSlice s_1 = cloud.star_slices.inner[metal::min(unsigned(_e80), 7u)];
            float cut_1 = metal::min(s_1.reach, metal::max((5.0 * s_1.cap) * s_1.defocus, s_1.fringe_reach));
            metal::float2 r_6 = (sp / metal::float2(s_1.cell)) - s_1.offset;
            metal::int2 o = naga_f2i32(metal::floor(r_6));
            uint _e101 = k;
            uint salt_2 = 1000u + (4u * _e101);
            slice = metal::float4(0.0);
            n_1 = 0;
            uint2 loop_bound_1 = uint2(4294967295u);
            bool loop_init_1 = true;
            while(true) {
                if (metal::all(loop_bound_1 == uint2(0u))) { break; }
                loop_bound_1 -= uint2(loop_bound_1.y == 0u, 1u);
                if (!loop_init_1) {
                    int _e128 = n_1;
                    n_1 = as_type<int>(as_type<uint>(_e128) + as_type<uint>(1));
                }
                loop_init_1 = false;
                int _e109 = n_1;
                if (_e109 < 9) {
                } else {
                    break;
                }
                {
                    metal::float4 _e112 = slice;
                    int _e113 = n_1;
                    int _e118 = n_1;
                    metal::float2 _e125 = ground_2;
                    metal::float4 _e126 = star_cell(s_1, r_6, as_type<metal::int2>(as_type<metal::uint2>(o) + as_type<metal::uint2>(metal::int2(as_type<int>(as_type<uint>(naga_mod(_e113, 3)) - as_type<uint>(1)), as_type<int>(as_type<uint>(naga_div(_e118, 3)) - as_type<uint>(1))))), salt_2, cut_1, _e125, lut, close_light, wide_light, cloud_sampler, cloud);
                    slice = _e112 + _e126;
                }
            }
            float level_7 = metal::clamp(metal::mix(close, _e15, s_1.blur), 0.0, 1.0);
            if (s_1.unseen > 0.0) {
                local = level_7 > 0.0;
            } else {
                local = false;
            }
            bool _e144 = local;
            if (_e144) {
                metal::float3 _e146 = star_paint(level_7, 1.0, lut, cloud);
                metal::float2 _e148 = ground_2;
                float _e149 = star_over_ground(_e146, _e148);
                float cover_1 = s_1.unseen * _e149;
                metal::float4 _e151 = slice;
                slice = _e151 + metal::float4(_e146 * cover_1, cover_1);
            }
            float _e156 = slice.w;
            if (_e156 > 0.0) {
                metal::float3 _e159 = out_1;
                metal::float4 _e160 = slice;
                float _e163 = slice.w;
                float _e167 = slice.w;
                out_1 = metal::mix(_e159, _e160.xyz / metal::float3(_e163), metal::min(_e167, 1.0));
            }
        }
    }
    metal::float3 _e174 = out_1;
    return _e174;
}

metal::float4 clouded(
    float level_5,
    metal::float2 position_1,
    metal::texture2d<float, metal::access::sample> lut,
    metal::texture2d<float, metal::access::sample> close_light,
    metal::texture2d<float, metal::access::sample> wide_light,
    metal::sampler cloud_sampler,
    constant Cloud& cloud,
    metal::texture2d<float, metal::access::sample> cloud_tone,
    metal::texture2d<float, metal::access::sample> cloud_tile_a,
    metal::texture2d<float, metal::access::sample> cloud_tile_b,
    metal::sampler tile_sampler
) {
    float tone = {};
    float _e4 = cloud.cloud_depth;
    if (_e4 <= 0.0) {
        metal::float4 _e7 = density_color(level_5, lut, cloud);
        return _e7;
    }
    float _e10 = cloud.ppp;
    metal::float2 _e15 = cloud.origin;
    metal::float2 pt_7 = (position_1 / metal::float2(_e10)) - _e15;
    uint _e19 = cloud.cloud_style;
    if (_e19 == 2u) {
        metal::float4 _e22 = density_color(level_5, lut, cloud);
        metal::float3 _e24 = star_color(pt_7, lut, close_light, wide_light, cloud_sampler, cloud);
        float _e27 = cloud.cloud_depth;
        return metal::float4(metal::mix(_e22.xyz, _e24, _e27), 1.0);
    }
    uint _e34 = cloud.tone_baked;
    if (_e34 == 1u) {
        metal::float2 _e41 = cloud.size;
        metal::float4 _e44 = cloud_tone.sample(cloud_sampler, pt_7 / _e41, metal::level(0.0));
        tone = _e44.x;
    } else {
        float _e46 = cloud_tone_at(pt_7, close_light, cloud_sampler, cloud, cloud_tile_a, cloud_tile_b, tile_sampler);
        tone = _e46;
    }
    float _e47 = tone;
    float _e50 = cloud.cloud_depth;
    metal::float4 _e52 = density_color(metal::mix(level_5, _e47, _e50), lut, cloud);
    return _e52;
}

metal::float4 backdrop_color(
    metal::float2 position_2,
    metal::texture2d<float, metal::access::sample> lut,
    metal::texture2d<float, metal::access::sample> close_light,
    metal::texture2d<float, metal::access::sample> wide_light,
    metal::sampler cloud_sampler,
    constant Cloud& cloud,
    metal::texture2d<float, metal::access::sample> cloud_tone,
    metal::texture2d<float, metal::access::sample> cloud_tile_a,
    metal::texture2d<float, metal::access::sample> cloud_tile_b,
    metal::sampler tile_sampler
) {
    float level_6 = 0.0;
    bool _e3 = softened(cloud);
    if (_e3) {
        float _e4 = baked_density(position_2, close_light, cloud_sampler, cloud);
        level_6 = _e4;
    }
    float _e5 = level_6;
    metal::float4 _e6 = clouded(_e5, position_2, lut, close_light, wide_light, cloud_sampler, cloud, cloud_tone, cloud_tile_a, cloud_tile_b, tile_sampler);
    return _e6;
}

struct fs_cloud_backdrop_gammaInput {
    float slab [[user(loc0), center_perspective]];
    float t [[user(loc1), center_perspective]];
};
struct fs_cloud_backdrop_gammaOutput {
    metal::float4 member [[color(0)]];
};
fragment fs_cloud_backdrop_gammaOutput fs_cloud_backdrop_gamma(
  fs_cloud_backdrop_gammaInput varyings [[stage_in]]
, metal::float4 position_3 [[position]]
, metal::texture2d<float, metal::access::sample> lut [[texture(0)]]
, metal::texture2d<float, metal::access::sample> close_light [[texture(1)]]
, metal::texture2d<float, metal::access::sample> wide_light [[texture(2)]]
, metal::sampler cloud_sampler [[sampler(0)]]
, constant Cloud& cloud [[buffer(2)]]
, metal::texture2d<float, metal::access::sample> cloud_tone [[texture(3)]]
, metal::texture2d<float, metal::access::sample> cloud_tile_a [[texture(4)]]
, metal::texture2d<float, metal::access::sample> cloud_tile_b [[texture(5)]]
, metal::sampler tile_sampler [[sampler(1)]]
) {
    const VertexOut in = { position_3, varyings.slab, varyings.t };
    metal::float4 _e3 = backdrop_color(in.position.xy, lut, close_light, wide_light, cloud_sampler, cloud, cloud_tone, cloud_tile_a, cloud_tile_b, tile_sampler);
    return fs_cloud_backdrop_gammaOutput { _e3 };
}
