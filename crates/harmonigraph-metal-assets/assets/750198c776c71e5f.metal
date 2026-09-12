// language: metal3.2
#include <metal_stdlib>
#include <simd/simd.h>

using metal::uint;

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
struct type_7 {
    metal::float4 inner[3];
};
struct OctaveParams {
    float span;
    float center;
    metal::float2 padding;
    type_7 bounds;
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
struct type_8 {
    metal::float4 inner[64];
};
struct type_10 {
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
    type_8 pitch_lut;
    type_8 spectral_lut;
    type_10 spectrum_color;
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

float aa_inside(
    float edge,
    float x,
    float w
) {
    return 1.0 - metal::smoothstep(edge - w, edge + w, x);
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

float plus_body_coverage(
    metal::float2 uv_3,
    float aa,
    constant Uniforms& u
) {
    float _e3 = plus_sd(uv_3, u);
    float _e4 = aa_inside(0.0, _e3, aa);
    return _e4;
}

float plus_coverage(
    metal::float2 uv_4,
    float aa_1,
    constant Uniforms& u
) {
    float _e2 = plus_body_coverage(uv_4, aa_1, u);
    float _e3 = plus_taper(uv_4, u);
    return _e2 * _e3;
}

struct fs_plus_cellInput {
    metal::float2 uv [[user(loc0), center_perspective]];
    metal::float4 color [[user(loc1), center_perspective]];
    metal::float4 shadow_box [[user(loc3), flat]];
    metal::float4 shadow_at [[user(loc4), center_no_perspective]];
};
struct fs_plus_cellOutput {
    metal::float4 member [[color(0)]];
};
fragment fs_plus_cellOutput fs_plus_cell(
  fs_plus_cellInput varyings [[stage_in]]
, metal::float4 clip_pos [[position]]
, constant Uniforms& u [[buffer(0)]]
) {
    const PlusVsOut in = { clip_pos, varyings.uv, {}, varyings.color, varyings.shadow_box, varyings.shadow_at };
    float _e3 = metal::fwidth(in.uv.x);
    float _e6 = aa_width(_e3, in.shadow_at.w, u);
    float aa_2 = metal::min(_e6, 0.6);
    float _e10 = plus_coverage(in.uv, aa_2, u);
    return fs_plus_cellOutput { metal::float4(_e10, 0.0, 0.0, 0.0) };
}
