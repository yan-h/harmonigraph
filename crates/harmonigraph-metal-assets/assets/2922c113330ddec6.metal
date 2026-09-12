// language: metal3.2
#include <metal_stdlib>
#include <simd/simd.h>

using metal::uint;

struct _mslBufferSizes {
    uint size3;
};

struct ShadowCaster {
    metal::float4 rect;
    metal::float4 cell;
    metal::float4 map;
    metal::float4 shade;
};
typedef ShadowCaster type_6[1];
struct SceneOut {
    metal::float4 other;
    metal::float4 ink;
    metal::float4 bloom_other;
    metal::float4 bloom_ink;
};
struct CompositeParams {
    float darkest_pitch;
    float brightest_pitch;
    float render_scale;
    float bloom_strength;
};
struct CameraParams {
    metal::float4x4 view_proj;
    metal::float4 right;
    metal::float4 up;
};
struct NodeParams {
    float radius;
    float band_inner;
    float band_outer;
    float rings_outer;
    float mark_inner;
    float angular_gap;
    float mark_thickness;
    float padding;
};
struct MarkerParams {
    float half_width;
    float taper_start;
    float world_unit;
    float padding;
};
struct type_11 {
    metal::float4 inner[3];
};
struct OctaveParams {
    float span;
    float center;
    metal::float2 padding;
    type_11 bounds;
};
struct SpectralParams {
    float inner;
    float outer;
    float range_cents;
    float folded;
};
struct GlowParams {
    float reach;
    float strength;
    float blend;
    float curve;
    float wash;
    float row_capacity;
    float lit;
    float accumulation;
};
struct ShadowParams {
    float width;
    float reach_sigmas;
    float depth;
    float occlusion;
};
struct ShadowTargetParams {
    metal::float2 pane_points;
    metal::float2 atlas_texels;
};
struct MarkerCellParams {
    metal::float4 rect;
    metal::float4 cell;
    float points_to_texels;
    float aa_scale;
    float arm_points;
    float padding;
};
struct type_12 {
    metal::float4 inner[64];
};
struct type_14 {
    metal::uint4 inner[240];
};
struct Uniforms {
    CompositeParams composite;
    CameraParams camera;
    NodeParams node;
    MarkerParams marker;
    OctaveParams octave;
    SpectralParams spectral;
    GlowParams glow;
    ShadowParams geometry_shadow;
    ShadowParams marker_shadow;
    ShadowTargetParams shadow_target;
    MarkerCellParams marker_cell;
    metal::float4 lattice_ground;
    type_12 pitch_lut;
    type_12 spectral_lut;
    type_14 spectrum_color;
};
struct ShadowThrough {
    float seen;
    float bloom;
};
struct Painted {
    metal::packed_float3 rgb;
    float seen;
    float bloom;
    float ink_alpha;
    char _pad4[8];
};
struct PlusVsOut {
    metal::float4 clip_pos;
    metal::float2 uv;
    char _pad2[8];
    metal::float4 color;
    metal::float4 shadow_box;
    metal::float4 shadow_at;
};
constant float DISTANCE_KIND = 1.0;
constant float GAUSSIAN_GAIN = 2.5;
constant float SHADOW_FALLOFF_FLOOR = 0.35;
constant float SHADOW_TAIL = 4.0;
constant float SHADOW_FALLOFF_FREE = 0.64025325;
constant float SHADOW_STOP = 2.0;
constant float SHADOW_INVISIBLE_FOLDS = 1.5586027;
constant float SHADOW_KEEP_FLOOR = 0.0009765625;
constant float TAU = 6.2831855;
constant float QUAD_MARGIN = 1.6;
constant float GLYPH_FADE_LIMIT = 1.3;
constant float GLOW_BASE = 0.8;
constant float SHADOW_REACH_SIGMAS = 3.0;
constant uint INK_STRIP_N = 64u;
constant uint GLOW_TILE_SIZE = 32u;
constant bool EARLY_OUT = true;
constant float INK_FLOOR = 0.01;
constant uint OCTAVE_SLOTS = 11u;
constant uint MAX_SPAN = 11u;
constant uint PITCH_LUT_N = 64u;
constant uint SPECTRUM_BUCKETS = 3828u;
constant float BUCKETS_PER_SEMITONE = 32.0;
constant float SPECTRUM_MIN_MIDI = 15.48682;
constant float AA_SOFTNESS_PX = 2.0;
constant float OCT_UP = -4.712389;
constant float DISTANCE_LEVEL_FLOOR = 0.5;
constant float EMPTY_DISTANCE = 65504.0;
constant float GLOW_LOBE_KAPPA = 4.0;
constant float PLUS_QUAD_MARGIN = 1.6;
constant metal::float3 GLOW_LUMINANCE = metal::float3(0.2126, 0.7152, 0.0722);

metal::float4 glow_light(
    metal::int2 coord,
    metal::texture2d<float, metal::access::sample> glow_tex
) {
    uint clamped_lod_e3 = metal::min(uint(0), glow_tex.get_num_mip_levels() - 1);
    metal::float4 _e3 = glow_tex.read(metal::min(metal::uint2(coord), metal::uint2(glow_tex.get_width(clamped_lod_e3), glow_tex.get_height(clamped_lod_e3)) - 1), clamped_lod_e3);
    return _e3;
}

metal::float3 wash_over(
    metal::float3 ink,
    float alpha,
    metal::float3 light,
    float share
) {
    metal::float3 w_2 = light * share;
    return (w_2 * alpha) + (ink * (metal::float3(1.0) - w_2));
}

bool cell_packed(
    metal::float4 cell
) {
    bool local = {};
    if (cell.z > 0.0) {
        local = cell.w > 0.0;
    } else {
        local = false;
    }
    bool _e10 = local;
    return _e10;
}

float shadow_stop(
    float falloff
) {
    if (falloff >= SHADOW_FALLOFF_FREE) {
        return SHADOW_STOP;
    }
    return metal::max(SHADOW_STOP, metal::pow(SHADOW_INVISIBLE_FOLDS, 1.0 / falloff));
}

float standoff_coverage(
    float d,
    float w,
    float falloff_1
) {
    float t = {};
    float u_1 = metal::max(d, 0.0) / metal::max(w, 0.000001);
    float f = metal::max(falloff_1, SHADOW_FALLOFF_FLOOR);
    t = u_1;
    if (f != 1.0) {
        t = metal::pow(u_1, f);
    }
    float _e15 = t;
    float _e18 = shadow_stop(f);
    return metal::exp(-4.0 * _e15) * (1.0 - metal::smoothstep(1.0, _e18, u_1));
}

float shadow_kernel(
    uint who,
    metal::float2 points,
    metal::texture2d<float, metal::access::sample> shadow_atlas,
    metal::sampler shadow_sampler,
    device type_6 const& shadow_casters,
    constant _mslBufferSizes& _buffer_sizes
) {
    if (who >= (1 + (_buffer_sizes.size3 - 0 - 64) / 64)) {
        return 0.0;
    }
    metal::float4 cell_1 = shadow_casters[metal::min(unsigned(who), (_buffer_sizes.size3 - 0 - 64) / 64)].cell;
    bool _e10 = cell_packed(cell_1);
    if (!(_e10)) {
        return 0.0;
    }
    metal::float2 atlas = static_cast<metal::float2>(metal::uint2(shadow_atlas.get_width(), shadow_atlas.get_height()));
    metal::float4 map = shadow_casters[metal::min(unsigned(who), (_buffer_sizes.size3 - 0 - 64) / 64)].map;
    metal::float2 texel = metal::clamp(map.xy + (points * map.z), cell_1.xy + metal::float2(0.5), (cell_1.xy + cell_1.zw) - metal::float2(0.5));
    metal::float4 _e39 = shadow_atlas.sample(shadow_sampler, texel / atlas, metal::level(0.0));
    float held = _e39.x;
    float _e45 = shadow_casters[metal::min(unsigned(who), (_buffer_sizes.size3 - 0 - 64) / 64)].shade.y;
    if (_e45 >= 0.5) {
        float _e52 = shadow_casters[metal::min(unsigned(who), (_buffer_sizes.size3 - 0 - 64) / 64)].shade.z;
        float _e59 = shadow_casters[metal::min(unsigned(who), (_buffer_sizes.size3 - 0 - 64) / 64)].shade.w;
        float _e60 = standoff_coverage(held, 2.0 * _e52, _e59);
        return metal::clamp(_e60, 0.0, 1.0);
    }
    return metal::min(GAUSSIAN_GAIN * metal::clamp(held, 0.0, 1.0), 1.0);
}

float shadow_transmittance(
    float full,
    float depth,
    float level
) {
    float keep = metal::max(1.0 - metal::clamp(depth, 0.0, 1.0), SHADOW_KEEP_FLOOR);
    float through = metal::pow(keep, metal::clamp(full, 0.0, 1.0));
    return 1.0 - (metal::clamp(level, 0.0, 1.0) * (1.0 - through));
}

float plus_shadow_depth(
    constant Uniforms& u
) {
    float _e3 = u.marker_shadow.depth;
    return metal::clamp(_e3, 0.0, 1.0);
}

uint naga_f2u32(float value) {
    return static_cast<uint>(metal::clamp(value, 0.0, 4294967000.0));
}

ShadowThrough shadow_through(
    float who_1,
    metal::float2 points_1,
    float level_1,
    float depth_1,
    metal::texture2d<float, metal::access::sample> shadow_atlas,
    metal::sampler shadow_sampler,
    device type_6 const& shadow_casters,
    constant _mslBufferSizes& _buffer_sizes
) {
    if (level_1 <= 0.0) {
        return ShadowThrough {1.0, 1.0};
    }
    float _e12 = shadow_kernel(naga_f2u32(metal::max(who_1, 0.0)), points_1, shadow_atlas, shadow_sampler, shadow_casters, _buffer_sizes);
    float _e13 = shadow_transmittance(_e12, depth_1, level_1);
    float _e15 = shadow_transmittance(_e12, 1.0, level_1);
    return ShadowThrough {_e13, _e15};
}

bool shadow_is_distance(
    float who_2,
    device type_6 const& shadow_casters,
    constant _mslBufferSizes& _buffer_sizes
) {
    uint caster = naga_f2u32(metal::max(who_2, 0.0));
    if (caster >= (1 + (_buffer_sizes.size3 - 0 - 64) / 64)) {
        return false;
    }
    float _e12 = shadow_casters[metal::min(unsigned(caster), (_buffer_sizes.size3 - 0 - 64) / 64)].shade.y;
    return _e12 >= 0.5;
}

ShadowThrough plus_shadow_through(
    float who_3,
    float d_points,
    metal::float2 points_2,
    float level_2,
    float distance_level,
    metal::texture2d<float, metal::access::sample> shadow_atlas,
    metal::sampler shadow_sampler,
    device type_6 const& shadow_casters,
    constant Uniforms& u,
    constant _mslBufferSizes& _buffer_sizes
) {
    if (level_2 <= 0.0) {
        return ShadowThrough {1.0, 1.0};
    }
    uint caster_1 = naga_f2u32(metal::max(who_3, 0.0));
    bool _e13 = shadow_is_distance(who_3, shadow_casters, _buffer_sizes);
    if (!(_e13)) {
        float _e15 = plus_shadow_depth(u);
        ShadowThrough _e16 = shadow_through(who_3, points_2, level_2, _e15, shadow_atlas, shadow_sampler, shadow_casters, _buffer_sizes);
        return _e16;
    }
    float _e21 = shadow_casters[metal::min(unsigned(caster_1), (_buffer_sizes.size3 - 0 - 64) / 64)].shade.z;
    float _e28 = shadow_casters[metal::min(unsigned(caster_1), (_buffer_sizes.size3 - 0 - 64) / 64)].shade.w;
    float _e29 = standoff_coverage(d_points, 2.0 * _e21, _e28);
    float _e30 = plus_shadow_depth(u);
    float _e31 = shadow_transmittance(_e29, _e30, distance_level);
    float _e33 = shadow_transmittance(_e29, 1.0, distance_level);
    return ShadowThrough {_e31, _e33};
}

float aa_inside(
    float edge,
    float x,
    float w_1
) {
    return 1.0 - metal::smoothstep(edge - w_1, edge + w_1, x);
}

float aa_width(
    float coord_fwidth,
    float surface_scale,
    constant Uniforms& u
) {
    float _e6 = u.composite.render_scale;
    float knob = (AA_SOFTNESS_PX * metal::max(_e6, 0.01)) * metal::clamp(surface_scale, 0.0, 1.0);
    return metal::max(coord_fwidth, 0.0001) * metal::max(knob, 1.0);
}

metal::float4 seen_of(
    Painted paint
) {
    return metal::float4(paint.rgb, paint.seen);
}

metal::int2 naga_f2i32(metal::float2 value) {
    return static_cast<metal::int2>(metal::clamp(value, -2147483600.0, 2147483500.0));
}

metal::int2 light_coord(
    metal::float2 frag_pos,
    metal::texture2d<float, metal::access::sample> glow_tex
) {
    metal::int2 edge_1 = as_type<metal::int2>(as_type<metal::uint2>(static_cast<metal::int2>(metal::uint2(glow_tex.get_width(), glow_tex.get_height()))) - as_type<metal::uint2>(metal::int2(1, 1)));
    return metal::min(naga_f2i32(frag_pos), edge_1);
}

float plus_box_sd(
    metal::float2 uv,
    float end,
    constant Uniforms& u
) {
    metal::float2 p = metal::abs(uv);
    metal::float2 q = metal::float2(metal::max(p.x, p.y), metal::min(p.x, p.y));
    float _e16 = u.marker.half_width;
    metal::float2 corner = metal::float2(q.x - end, q.y - _e16);
    return metal::length(metal::max(corner, metal::float2(0.0))) + metal::min(metal::max(corner.x, corner.y), 0.0);
}

float plus_sd(
    metal::float2 uv_1,
    constant Uniforms& u
) {
    float _e2 = plus_box_sd(uv_1, 1.0, u);
    return _e2;
}

float plus_taper(
    metal::float2 uv_2,
    constant Uniforms& u
) {
    metal::float2 p_1 = metal::abs(uv_2);
    metal::float2 q_1 = metal::float2(metal::max(p_1.x, p_1.y), metal::min(p_1.x, p_1.y));
    float _e12 = u.marker.taper_start;
    float start = metal::min(_e12, 0.999);
    float fade = metal::smoothstep(start, 1.0, metal::clamp(q_1.x, 0.0, 1.0));
    return 1.0 - fade;
}

float plus_shadow_taper(
    metal::float2 uv_3,
    constant Uniforms& u
) {
    float _e1 = plus_taper(uv_3, u);
    float _e5 = u.marker.taper_start;
    return (_e5 >= 0.999) ? 1.0 : _e1;
}

float plus_body_coverage(
    metal::float2 uv_4,
    float aa,
    constant Uniforms& u
) {
    float _e3 = plus_sd(uv_4, u);
    float _e4 = aa_inside(0.0, _e3, aa);
    return _e4;
}

float plus_coverage(
    metal::float2 uv_5,
    float aa_1,
    constant Uniforms& u
) {
    float _e2 = plus_body_coverage(uv_5, aa_1, u);
    float _e3 = plus_taper(uv_5, u);
    return _e2 * _e3;
}

Painted plus_paint(
    PlusVsOut in_1,
    metal::texture2d<float, metal::access::sample> glow_tex,
    metal::texture2d<float, metal::access::sample> shadow_atlas,
    metal::sampler shadow_sampler,
    device type_6 const& shadow_casters,
    constant Uniforms& u,
    constant _mslBufferSizes& _buffer_sizes
) {
    float _e3 = metal::fwidth(in_1.uv.x);
    float _e6 = aa_width(_e3, in_1.shadow_at.w, u);
    float aa_2 = metal::min(_e6, 0.6);
    float _e10 = plus_body_coverage(in_1.uv, aa_2, u);
    float _e14 = plus_coverage(in_1.uv, aa_2, u);
    float alpha_1 = in_1.color.w * _e14;
    float _e17 = plus_sd(in_1.uv, u);
    float d_points_1 = _e17 * in_1.shadow_box.y;
    float distance_level_1 = in_1.shadow_at.z;
    ShadowThrough _e29 = plus_shadow_through(in_1.shadow_box.x, d_points_1, in_1.shadow_at.xy, in_1.shadow_at.z, distance_level_1, shadow_atlas, shadow_sampler, shadow_casters, u, _buffer_sizes);
    float _e33 = plus_shadow_taper(in_1.uv, u);
    float shadow_exposure = (1.0 - _e10) * _e33;
    float seen_through = 1.0 - ((1.0 - _e29.seen) * shadow_exposure);
    float bloom_through = 1.0 - ((1.0 - _e29.bloom) * shadow_exposure);
    float final_alpha = 1.0 - ((1.0 - alpha_1) * seen_through);
    float bloom_alpha = 1.0 - ((1.0 - alpha_1) * bloom_through);
    if (bloom_alpha <= 0.0) {
        metal::discard_fragment();
    }
    metal::float3 ink_1 = in_1.color.xyz * alpha_1;
    metal::int2 _e64 = light_coord(in_1.clip_pos.xy, glow_tex);
    metal::float4 _e65 = glow_light(_e64, glow_tex);
    metal::float3 _e68 = wash_over(ink_1, alpha_1, _e65.xyz, 1.0);
    return Painted {_e68, final_alpha, bloom_alpha, alpha_1};
}

struct fs_plus_sceneInput {
    metal::float2 uv [[user(loc0), center_perspective]];
    metal::float4 color [[user(loc1), center_perspective]];
    metal::float4 shadow_box [[user(loc3), flat]];
    metal::float4 shadow_at [[user(loc4), center_no_perspective]];
};
struct fs_plus_sceneOutput {
    metal::float4 other [[color(0)]];
    metal::float4 ink [[color(1)]];
    metal::float4 bloom_other [[color(2)]];
    metal::float4 bloom_ink [[color(3)]];
};
fragment fs_plus_sceneOutput fs_plus_scene(
  fs_plus_sceneInput varyings [[stage_in]]
, metal::float4 clip_pos [[position]]
, metal::texture2d<float, metal::access::sample> glow_tex [[texture(0)]]
, metal::texture2d<float, metal::access::sample> shadow_atlas [[texture(1)]]
, metal::sampler shadow_sampler [[sampler(1)]]
, device type_6 const& shadow_casters [[buffer(1)]]
, constant Uniforms& u [[buffer(0)]]
, constant _mslBufferSizes& _buffer_sizes [[buffer(2)]]
) {
    const PlusVsOut in = { clip_pos, varyings.uv, {}, varyings.color, varyings.shadow_box, varyings.shadow_at };
    Painted _e1 = plus_paint(in, glow_tex, shadow_atlas, shadow_sampler, shadow_casters, u, _buffer_sizes);
    metal::float4 _e2 = seen_of(_e1);
    const auto _tmp = SceneOut {_e2, metal::float4(0.0, 0.0, 0.0, _e1.seen), metal::float4(_e1.rgb, _e1.bloom), metal::float4(0.0, 0.0, 0.0, _e1.bloom)};
    return fs_plus_sceneOutput { _tmp.other, _tmp.ink, _tmp.bloom_other, _tmp.bloom_ink };
}
