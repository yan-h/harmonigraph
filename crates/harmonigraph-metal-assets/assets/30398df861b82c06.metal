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
struct type_9 {
    metal::float4 inner[3];
};
struct OctaveParams {
    float span;
    float center;
    metal::float2 padding;
    type_9 bounds;
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
struct type_10 {
    metal::float4 inner[64];
};
struct type_12 {
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
    type_10 pitch_lut;
    type_10 spectral_lut;
    type_12 spectrum_color;
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

float glow_blend_kappa(
    constant Uniforms& u
) {
    float _e4 = u.glow.blend;
    return GLOW_LOBE_KAPPA * (1.0 - metal::clamp(_e4, 0.0, 1.0));
}
int naga_f2i32(float value) {
    return static_cast<int>(metal::clamp(value, -2147483600.0, 2147483500.0));
}


struct fs_ink_blurInput {
};
struct fs_ink_blurOutput {
    metal::float4 member [[color(0)]];
};
fragment fs_ink_blurOutput fs_ink_blur(
  metal::float4 pos [[position]]
, constant Uniforms& u [[buffer(0)]]
, metal::texture2d<float, metal::access::sample> ink_strip [[texture(0)]]
) {
    metal::float3 rgb = metal::float3(0.0);
    float wsum = 0.0;
    float lobes = 0.0;
    uint i = 0u;
    int col = naga_f2i32(pos.x);
    int row = naga_f2i32(pos.y);
    float _e5 = glow_blend_kappa(u);
    float kappa = (col >= 64) ? 0.0 : _e5;
    uint2 loop_bound = uint2(4294967295u);
    bool loop_init = true;
    while(true) {
        if (metal::all(loop_bound == uint2(0u))) { break; }
        loop_bound -= uint2(loop_bound.y == 0u, 1u);
        if (!loop_init) {
            uint _e49 = i;
            i = _e49 + 1u;
        }
        loop_init = false;
        uint _e19 = i;
        if (_e19 < INK_STRIP_N) {
        } else {
            break;
        }
        {
            uint _e22 = i;
            float off = (static_cast<float>(_e22) - static_cast<float>(col)) * 0.09817477;
            float lobe = metal::exp(kappa * (metal::cos(off) - 1.0));
            uint _e34 = i;
            uint clamped_lod_e38 = metal::min(uint(0), ink_strip.get_num_mip_levels() - 1);
            metal::float4 ink = ink_strip.read(metal::min(metal::uint2(metal::int2(static_cast<int>(_e34), row)), metal::uint2(ink_strip.get_width(clamped_lod_e38), ink_strip.get_height(clamped_lod_e38)) - 1), clamped_lod_e38);
            metal::float3 _e39 = rgb;
            rgb = _e39 + (ink.xyz * lobe);
            float _e43 = wsum;
            wsum = _e43 + (ink.w * lobe);
            float _e47 = lobes;
            lobes = _e47 + lobe;
        }
    }
    metal::float3 _e52 = rgb;
    float _e53 = wsum;
    float _e58 = wsum;
    float _e59 = lobes;
    return fs_ink_blurOutput { metal::float4(_e52 / metal::float3(metal::max(_e53, 0.00001)), _e58 / metal::max(_e59, 0.00001)) };
}
