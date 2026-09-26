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
    metal::float2 spread;
    float life;
    float _pad;
};
struct type_5 {
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
    float star_glow;
    float star_wander;
    float star_time;
    metal::float2 _star_pad;
    type_5 star_slices;
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
constant uint STAR_SLICES = 5u;
constant float STAR_PANE = 540.0;
constant float STAR_JITTER = 0.6;
constant int STAR_HASH_PERIOD = 4096;
constant float STAR_WANDER_PERIOD = 400.0;
constant uint STAR_LIFE_PERIOD = 4096u;
constant float STAR_FADE = 0.2;
constant float STAR_EXPOSURE = 1.5;
constant float STAR_LIFT = 0.18;
constant float STAR_OVER_GROUND = 6.0;
constant float STAR_RING_FADE = 0.7;
constant float STAR_TAU = 6.2831855;

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
    metal::float2 r_3 = q * scale_units;
    uint _e36 = cloud.tile_cells;
    metal::float4 tile = cloud_tile_a.sample(tile_sampler, r_3 / metal::float2(static_cast<float>(_e36)), metal::level(0.0));
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
    metal::float4 a = cloud_tile_a.sample(tile_sampler, _e1, metal::level(0.0));
    metal::float2 _e9 = rotate_watercolor_tile_vector(a.xy, cloud);
    out.coarse = Wet {_e9};
    out.fine = Wet {metal::float2(0.0)};
    out.cover = 0.0;
    float _e19 = cloud.wash_layers;
    if (_e19 > 0.0) {
        metal::float4 b = cloud_tile_b.sample(tile_sampler, _e1, metal::level(0.0));
        metal::float2 _e28 = rotate_watercolor_tile_vector(b.xy, cloud);
        out.fine = Wet {_e28};
        out.cover = b.w;
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
    float level = {};
    metal::float2 _e3 = cloud.size;
    float _e10 = cloud.size.y;
    metal::float2 _e17 = cloud.drift;
    metal::float2 q_1 = (((pt_3 - (_e3 * 0.5)) / metal::float2(_e10)) * CLOUD_UNITS) + _e17;
    float _e22 = cloud.wash_size;
    float cells = WASH_CELLS / _e22;
    float _e27 = cloud.size.y;
    float pane_per_cell_1 = (_e27 / CLOUD_UNITS) / cells;
    metal::float2 r_4 = q_1 * cells;
    WashField _e32 = wash_tile_field(r_4, cloud, cloud_tile_a, cloud_tile_b, tile_sampler);
    float _e34 = wash_level(_e32.coarse, pane_per_cell_1, pt_3, close_light, cloud_sampler, cloud);
    level = _e34;
    float _e38 = cloud.wash_layers;
    if (_e38 > 0.0) {
        float _e44 = wash_level(_e32.fine, pane_per_cell_1 / WASH_LACUNARITY, pt_3, close_light, cloud_sampler, cloud);
        float _e47 = cloud.wash_layers;
        float over = _e47 * _e32.cover;
        float _e50 = level;
        level = metal::mix(_e50, _e44, over);
    }
    float _e52 = level;
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

struct fs_cloud_toneInput {
    float slab [[user(loc0), center_perspective]];
    float t [[user(loc1), center_perspective]];
};
struct fs_cloud_toneOutput {
    metal::float4 member [[color(0)]];
};
fragment fs_cloud_toneOutput fs_cloud_tone(
  fs_cloud_toneInput varyings [[stage_in]]
, metal::float4 position [[position]]
, metal::texture2d<float, metal::access::sample> close_light [[texture(1)]]
, metal::sampler cloud_sampler [[sampler(0)]]
, constant Cloud& cloud [[buffer(2)]]
, metal::texture2d<float, metal::access::sample> cloud_tile_a [[texture(4)]]
, metal::texture2d<float, metal::access::sample> cloud_tile_b [[texture(5)]]
, metal::sampler tile_sampler [[sampler(1)]]
) {
    const VertexOut in = { position, varyings.slab, varyings.t };
    metal::float2 _e6 = cloud.size;
    float _e8 = cloud_tone_at(metal::float2(in.slab, in.t) * _e6, close_light, cloud_sampler, cloud, cloud_tile_a, cloud_tile_b, tile_sampler);
    return fs_cloud_toneOutput { metal::float4(_e8, 0.0, 0.0, 1.0) };
}
