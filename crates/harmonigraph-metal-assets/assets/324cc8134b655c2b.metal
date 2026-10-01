// language: metal3.2
#include <metal_stdlib>
#include <simd/simd.h>

using metal::uint;

struct Wet {
    metal::float2 offset;
    float brightness;
    float gap;
};
struct WashField {
    Wet coarse;
    Wet fine;
    float cover;
    char _pad3[4];
};
struct Pile {
    metal::float2 face;
    metal::float2 to_centre;
};
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
    float star_size_variation;
    uint star_pad0_;
    uint star_pad1_;
    uint star_pad2_;
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
    float wash_randomness;
    metal::float2 memory_extent;
    type_9 previous_slices;
    type_10 star_halo_samples;
    metal::float4 velvet;
    metal::float4 velvet_size;
    metal::float4 wash_pigment;
};
struct VertexOut {
    metal::float4 position;
    float slab;
    float t;
    char _pad3[8];
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
constant float CLOUD_UNITS = 10.0;
constant float SCALE_CELLS = 2.7272727;
constant float WASH_POOL = 0.44;
constant float WASH_PIG_DEPTH = 0.35;
constant float WASH_BLOOM = 2.7;

metal::float2 rotate_watercolor_tile_vector_for(
    metal::float2 v,
    uint pitch_vertical
) {
    metal::float2 semantic = (pitch_vertical == 1u) ? v : metal::float2(v.y, v.x);
    metal::float2 turned = metal::float2((CLOUD_TILE_ROT_COS * semantic.x) - (CLOUD_TILE_ROT_SIN * semantic.y), (CLOUD_TILE_ROT_SIN * semantic.x) + (CLOUD_TILE_ROT_COS * semantic.y));
    return (pitch_vertical == 1u) ? turned : metal::float2(turned.y, turned.x);
}

metal::float2 watercolor_tile_uv_for(
    metal::float2 r,
    float period,
    uint pitch_vertical_1
) {
    metal::float2 semantic_1 = (pitch_vertical_1 == 1u) ? r : metal::float2(r.y, r.x);
    return metal::float2((CLOUD_TILE_ROT_COS * semantic_1.x) + (CLOUD_TILE_ROT_SIN * semantic_1.y), (-0.6 * semantic_1.x) + (CLOUD_TILE_ROT_COS * semantic_1.y)) / metal::float2(period);
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
    constant Cloud& cloud,
    metal::texture2d<float, metal::access::sample> close_light,
    metal::sampler cloud_sampler
) {
    metal::float2 _e5 = cloud.size;
    metal::float4 _e8 = close_light.sample(cloud_sampler, pt / _e5, metal::level(0.0));
    return _e8.x;
}

float scale_tone(
    metal::float2 pt_1,
    constant Cloud& cloud,
    metal::texture2d<float, metal::access::sample> close_light,
    metal::sampler cloud_sampler,
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
    float _e69 = cloud_light(pt_1 + lookup, cloud, close_light, cloud_sampler);
    return _e69;
}

bool wash_pigmented(
    constant Cloud& cloud
) {
    float _e3 = cloud.wash_pigment.x;
    return _e3 != 0.0;
}

float wash_level(
    Wet wet,
    float pane_per_cell,
    metal::float2 pt_2,
    constant Cloud& cloud,
    metal::texture2d<float, metal::access::sample> close_light,
    metal::sampler cloud_sampler
) {
    float _e7 = cloud.wash_refract;
    float _e10 = cloud_light(pt_2 + ((wet.offset * pane_per_cell) * _e7), cloud, close_light, cloud_sampler);
    bool _e11 = wash_pigmented(cloud);
    if (!(_e11)) {
        return _e10;
    }
    float _e17 = cloud.wash_pigment.y;
    float _e27 = cloud.wash_pigment.z;
    float shape = metal::pow(metal::clamp(1.0 - (wet.gap / _e17), 0.0, 1.0), _e27);
    float fuzz = cloud.wash_fuzz;
    float _e36 = cloud.wash_pigment.x;
    float pigment = ((WASH_POOL * _e36) * (1.0 - (0.75 * fuzz))) * shape;
    if (pigment < 0.0) {
        return metal::min(_e10 - (((pigment * WASH_BLOOM) * _e10) * (1.0 - _e10)), 1.0);
    }
    return metal::max(_e10 - (pigment * (WASH_PIG_DEPTH + (0.65 * _e10))), 0.0);
}

WashField wash_tile_field(
    metal::float2 r_2,
    constant Cloud& cloud,
    metal::texture2d<float, metal::access::sample> cloud_tile_a,
    metal::texture2d<float, metal::access::sample> cloud_tile_b,
    metal::texture2d<float, metal::access::sample> cloud_tile_c,
    metal::sampler tile_sampler
) {
    WashField out = {};
    metal::float4 c = metal::float4(0.0);
    metal::float2 _e1 = watercolor_tile_uv(r_2, cloud);
    metal::float4 a = cloud_tile_a.sample(tile_sampler, _e1, metal::level(0.0));
    bool _e10 = wash_pigmented(cloud);
    if (_e10) {
        metal::float4 _e14 = cloud_tile_c.sample(tile_sampler, _e1, metal::level(0.0));
        c = _e14;
    }
    metal::float2 _e17 = rotate_watercolor_tile_vector(a.xy, cloud);
    float _e20 = c.x;
    out.coarse = Wet {_e17, a.z, _e20};
    out.fine = Wet {metal::float2(0.0), 0.0, 0.0};
    out.cover = 0.0;
    float _e32 = cloud.wash_layers;
    if (_e32 > 0.0) {
        metal::float4 b = cloud_tile_b.sample(tile_sampler, _e1, metal::level(0.0));
        metal::float2 _e41 = rotate_watercolor_tile_vector(b.xy, cloud);
        float _e44 = c.y;
        out.fine = Wet {_e41, b.z, _e44};
        out.cover = b.w;
    }
    WashField _e48 = out;
    return _e48;
}

metal::float2 wash_cell_at(
    metal::float2 pt_3,
    constant Cloud& cloud
) {
    metal::float2 _e3 = cloud.size;
    float _e10 = cloud.size.y;
    metal::float2 _e17 = cloud.drift;
    metal::float2 q_1 = (((pt_3 - (_e3 * 0.5)) / metal::float2(_e10)) * CLOUD_UNITS) + _e17;
    float _e22 = cloud.wash_size;
    return q_1 * (WASH_CELLS / _e22);
}

float wash_cloud_tone(
    metal::float2 pt_4,
    constant Cloud& cloud,
    metal::texture2d<float, metal::access::sample> close_light,
    metal::sampler cloud_sampler,
    metal::texture2d<float, metal::access::sample> cloud_tile_a,
    metal::texture2d<float, metal::access::sample> cloud_tile_b,
    metal::texture2d<float, metal::access::sample> cloud_tile_c,
    metal::sampler tile_sampler
) {
    float level = {};
    float _e4 = cloud.wash_size;
    float cells = WASH_CELLS / _e4;
    float _e9 = cloud.size.y;
    float pane_per_cell_1 = (_e9 / CLOUD_UNITS) / cells;
    metal::float2 _e13 = wash_cell_at(pt_4, cloud);
    WashField _e14 = wash_tile_field(_e13, cloud, cloud_tile_a, cloud_tile_b, cloud_tile_c, tile_sampler);
    float _e16 = wash_level(_e14.coarse, pane_per_cell_1, pt_4, cloud, close_light, cloud_sampler);
    level = _e16;
    float _e20 = cloud.wash_layers;
    if (_e20 > 0.0) {
        float _e26 = wash_level(_e14.fine, pane_per_cell_1 / WASH_LACUNARITY, pt_4, cloud, close_light, cloud_sampler);
        float _e29 = cloud.wash_layers;
        float over = _e29 * _e14.cover;
        float _e32 = level;
        level = metal::mix(_e32, _e26, over);
    }
    float _e34 = level;
    return _e34;
}

float cloud_tone_at(
    metal::float2 pt_5,
    constant Cloud& cloud,
    metal::texture2d<float, metal::access::sample> close_light,
    metal::sampler cloud_sampler,
    metal::texture2d<float, metal::access::sample> cloud_tone,
    metal::texture2d<float, metal::access::sample> cloud_tile_a,
    metal::texture2d<float, metal::access::sample> cloud_tile_b,
    metal::texture2d<float, metal::access::sample> cloud_tile_c,
    metal::sampler tile_sampler
) {
    uint _e3 = cloud.cloud_style;
    if (_e3 == 3u) {
        metal::float2 _e10 = cloud.size;
        metal::float4 _e13 = cloud_tone.sample(cloud_sampler, pt_5 / _e10, metal::level(0.0));
        return _e13.x;
    }
    uint _e17 = cloud.cloud_style;
    if (_e17 == 1u) {
        float _e20 = wash_cloud_tone(pt_5, cloud, close_light, cloud_sampler, cloud_tile_a, cloud_tile_b, cloud_tile_c, tile_sampler);
        return _e20;
    }
    float _e21 = scale_tone(pt_5, cloud, close_light, cloud_sampler, cloud_tile_a, tile_sampler);
    return _e21;
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
, constant Cloud& cloud [[buffer(2)]]
, metal::texture2d<float, metal::access::sample> close_light [[texture(1)]]
, metal::sampler cloud_sampler [[sampler(0)]]
, metal::texture2d<float, metal::access::sample> cloud_tone [[texture(3)]]
, metal::texture2d<float, metal::access::sample> cloud_tile_a [[texture(4)]]
, metal::texture2d<float, metal::access::sample> cloud_tile_b [[texture(5)]]
, metal::texture2d<float, metal::access::sample> cloud_tile_c [[texture(11)]]
, metal::sampler tile_sampler [[sampler(1)]]
) {
    const VertexOut in = { position, varyings.slab, varyings.t };
    metal::float2 _e6 = cloud.size;
    float _e8 = cloud_tone_at(metal::float2(in.slab, in.t) * _e6, cloud, close_light, cloud_sampler, cloud_tone, cloud_tile_a, cloud_tile_b, cloud_tile_c, tile_sampler);
    return fs_cloud_toneOutput { metal::float4(_e8, 0.0, 0.0, 1.0) };
}
