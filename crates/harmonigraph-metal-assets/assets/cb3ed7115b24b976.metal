// language: metal3.2
#include <metal_stdlib>
#include <simd/simd.h>

using metal::uint;

struct CompositeParams {
    float darkest_pitch;
    float brightest_pitch;
    float render_scale;
    float bloom_strength;
    metal::float4 background;
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
    float animation;
    metal::float4 pose;
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
    float padding;
    float curve;
    float wash;
    float row_capacity;
    float lit;
    float accumulation;
};
struct TextureParams {
    float depth;
    float scale;
    metal::float2 drift;
    metal::float2 target_size;
    uint style;
    float padding;
};
struct MaterialParams {
    float amount;
    float scale;
    metal::float2 drift;
    uint style;
    float fuzz;
    float lobe;
    float variety;
    float refract;
    float layers;
    metal::float2 padding;
};
struct PickupParams {
    float intensity;
    float width;
    float softness;
    float padding;
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
    float spread_points;
};
struct type_8 {
    metal::float4 inner[64];
};
struct type_10 {
    metal::uint4 inner[240];
};
struct type_11 {
    metal::float4 inner[16];
};
struct Uniforms {
    CompositeParams composite;
    CameraParams camera;
    NodeParams node;
    MarkerParams marker;
    OctaveParams octave;
    SpectralParams spectral;
    GlowParams glow;
    TextureParams texture;
    MaterialParams material;
    PickupParams pickup;
    ShadowParams geometry_shadow;
    ShadowParams marker_shadow;
    ShadowTargetParams shadow_target;
    MarkerCellParams marker_cell;
    metal::float4 lattice_ground;
    metal::float4 lut_spacing;
    type_8 pitch_lut;
    type_8 spectral_lut;
    type_10 spectrum_color;
    type_11 ink_kernel;
};
struct PickupOut {
    metal::float4 position;
    metal::float2 uv;
    float level;
    char _pad3[4];
};
constant float DISTANCE_KIND = 1.0;
constant float DISTANCE_COVERAGE_KIND = 2.0;
constant uint OCCLUDER_HEADER = 5u;
constant float GAUSSIAN_GAIN = 2.5;
constant float SHADOW_STOP = 1.0;
constant float SHADOW_FALLOFF_MIN = -6.0;
constant float SHADOW_FALLOFF_MAX = 6.0;
constant float SHADOW_KEEP_FLOOR = 0.0009765625;
constant float TAU = 6.2831855;
constant float QUAD_MARGIN = 1.6;
constant float GLYPH_FADE_LIMIT = 1.3;
constant float GLOW_BASE = 0.8;
constant float SHADOW_REACH_SIGMAS = 3.0;
constant uint INK_STRIP_N = 64u;
constant bool EARLY_OUT = true;
constant float INK_FLOOR = 0.01;
constant uint OCTAVE_SLOTS = 11u;
constant float THICKNESS_STEPS = 64.0;
constant float GAUSSIAN_SPREAD_KIND = 3.0;
constant uint MAX_SPAN = 11u;
constant uint PITCH_LUT_N = 64u;
constant float BEND_ROUNDING = 0.4;
constant uint SPECTRUM_BUCKETS = 3828u;
constant float BUCKETS_PER_SEMITONE = 32.0;
constant float SPECTRUM_MIN_MIDI = 15.48682;
constant float AA_SOFTNESS_PX = 2.0;
constant float OCT_UP = -4.712389;
constant float DISTANCE_LEVEL_FLOOR = 0.5;
constant float EMPTY_DISTANCE = 65504.0;
constant float PLUS_QUAD_MARGIN = 1.6;
constant metal::float3 GLOW_LUMINANCE = metal::float3(0.2126, 0.7152, 0.0722);

float node_rim(
    bool marked,
    constant Uniforms& u
) {
    float rim = {};
    bool local = {};
    float _e4 = u.node.rings_outer;
    rim = metal::max(_e4, 0.0);
    if (marked) {
        float _e13 = u.node.mark_thickness;
        local = _e13 > 0.0;
    } else {
        local = false;
    }
    bool _e17 = local;
    if (_e17) {
        float _e18 = rim;
        float _e22 = u.node.mark_inner;
        float _e26 = u.node.mark_thickness;
        rim = metal::max(_e18, _e22 + _e26);
    }
    float _e29 = rim;
    return _e29;
}

struct fs_source_shadowInput {
    metal::float2 uv [[user(loc0), center_perspective]];
    float level [[user(loc1), flat]];
};
struct fs_source_shadowOutput {
    metal::float4 member [[color(0)]];
};
fragment fs_source_shadowOutput fs_source_shadow(
  fs_source_shadowInput varyings [[stage_in]]
, metal::float4 position [[position]]
, constant Uniforms& u [[buffer(0)]]
) {
    const PickupOut in = { position, varyings.uv, varyings.level };
    float _e4 = node_rim(false, u);
    float d = metal::abs(metal::length(in.uv) - _e4);
    float _e10 = u.pickup.width;
    float half_width = (0.5 * _e10) / 1.8;
    float _e17 = metal::fwidth(in.uv.x);
    float aa = metal::max(_e17, 0.00001);
    float _e23 = u.pickup.softness;
    float feather = metal::max(_e23 / 1.8, aa);
    float coverage = 1.0 - metal::smoothstep(half_width - aa, half_width + feather, d);
    float _e37 = u.pickup.intensity;
    return fs_source_shadowOutput { metal::float4(0.0, 0.0, 0.0, (coverage * in.level) * _e37) };
}
