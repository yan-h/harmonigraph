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
    float padding;
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

metal::float4 no_quad(
) {
    return metal::float4(2.0, 2.0, 0.0, 1.0);
}

metal::float4 cell_clip(
    metal::float2 texel,
    metal::float2 size,
    float w
) {
    metal::float2 extent = metal::max(size, metal::float2(1.0));
    return metal::float4((((2.0 * texel.x) / extent.x) - 1.0) * w, (1.0 - ((2.0 * texel.y) / extent.y)) * w, 0.0, w);
}

struct vs_plus_cellInput {
};
struct vs_plus_cellOutput {
    metal::float4 clip_pos [[position]];
    metal::float2 uv [[user(loc0), center_perspective]];
    metal::float4 color [[user(loc1), center_perspective]];
    metal::float4 shadow_box [[user(loc3), flat]];
    metal::float4 shadow_at [[user(loc4), center_no_perspective]];
};
vertex vs_plus_cellOutput vs_plus_cell(
  uint vertex_index [[vertex_id]]
, constant Uniforms& u [[buffer(0)]]
) {
    PlusVsOut out = {};
    metal::float2 corner = metal::float2(((vertex_index & 1u) == 1u) ? 1.0 : 0.0, ((vertex_index & 2u) == 2u) ? 1.0 : 0.0);
    metal::float4 rect = u.marker_cell.rect;
    metal::float4 cell_1 = u.marker_cell.cell;
    float _e30 = u.marker_cell.points_to_texels;
    metal::float2 texel_1 = cell_1.xy + ((corner * rect.zw) * _e30);
    metal::float4 _e35 = no_quad();
    metal::float2 _e39 = u.shadow_target.atlas_texels;
    metal::float4 _e41 = cell_clip(texel_1, _e39, 1.0);
    bool _e42 = cell_packed(cell_1);
    out.clip_pos = _e42 ? _e41 : _e35;
    float _e47 = u.marker_cell.arm_points;
    float arm_points = metal::max(_e47, 0.000001);
    out.uv = ((corner * 2.0) - metal::float2(1.0)) * ((rect.z * 0.5) / arm_points);
    out.color = metal::float4(1.0);
    out.shadow_box = metal::float4(0.0);
    float _e71 = u.marker_cell.aa_scale;
    out.shadow_at = metal::float4(0.0, 0.0, 0.0, _e71);
    PlusVsOut _e76 = out;
    const auto _tmp = _e76;
    return vs_plus_cellOutput { _tmp.clip_pos, _tmp.uv, _tmp.color, _tmp.shadow_box, _tmp.shadow_at };
}
